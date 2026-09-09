use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use clip_export::{
    CancelOutcome, ExportJob, ExportJobId, ExportJobState, ExportProgress, ExportQueue,
    ExportQueueError, ExportStatus,
};
use thiserror::Error;

use crate::executor::{ExportError, FfmpegExporter};
use crate::process::{ProcessRunner, SystemProcessRunner};

const WORKER_WAKE_INTERVAL: Duration = Duration::from_millis(50);

/// Errors from bounded worker submission and queue operations.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ExportWorkerError {
    #[error(transparent)]
    Queue(#[from] ExportQueueError),

    #[error("export worker is stopped")]
    WorkerStopped,

    #[error("export queue already has an executor worker")]
    WorkerAlreadyRunning,

    #[error("export worker could not start: {reason}")]
    WorkerStartFailed { reason: String },
}

impl ExportWorkerError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Queue(error) => error.code(),
            Self::WorkerStopped => "CLIP_EXPORT_WORKER_STOPPED",
            Self::WorkerAlreadyRunning => "CLIP_EXPORT_WORKER_ALREADY_RUNNING",
            Self::WorkerStartFailed { .. } => "CLIP_EXPORT_WORKER_START_FAILED",
        }
    }
}

/// One bounded post-save export worker.
///
/// The queue accepts at most one active and two pending jobs. The worker owns
/// all process and staged-file I/O; callers only submit jobs and inspect state.
pub struct ExportWorker<R: ProcessRunner + 'static = SystemProcessRunner> {
    queue: Arc<ExportQueue>,
    wake_sender: Option<SyncSender<()>>,
    shutdown: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    _runner: std::marker::PhantomData<R>,
}

impl ExportWorker<SystemProcessRunner> {
    pub fn new(
        queue: Arc<ExportQueue>,
        exporter: FfmpegExporter<SystemProcessRunner>,
    ) -> Result<Self, ExportWorkerError> {
        Self::with_exporter(queue, exporter)
    }
}

impl<R: ProcessRunner + 'static> ExportWorker<R> {
    pub fn with_exporter(
        queue: Arc<ExportQueue>,
        exporter: FfmpegExporter<R>,
    ) -> Result<Self, ExportWorkerError> {
        if !queue.try_claim_worker() {
            return Err(ExportWorkerError::WorkerAlreadyRunning);
        }
        let (wake_sender, wake_receiver) = mpsc::sync_channel(1);
        let shutdown = Arc::new(AtomicBool::new(false));
        let worker_shutdown = Arc::clone(&shutdown);
        let worker_queue = Arc::clone(&queue);
        let worker = match thread::Builder::new()
            .name("silk-export-worker".to_string())
            .spawn(move || {
                enter_background_processing_mode();
                run_worker(worker_queue, exporter, wake_receiver, worker_shutdown);
            }) {
            Ok(worker) => worker,
            Err(error) => {
                queue.release_worker();
                return Err(ExportWorkerError::WorkerStartFailed {
                    reason: error.to_string(),
                });
            }
        };

        Ok(Self {
            queue,
            wake_sender: Some(wake_sender),
            shutdown,
            worker: Some(worker),
            _runner: std::marker::PhantomData,
        })
    }

    /// Submit without waiting for an active export or worker I/O.
    pub fn submit(&self, job: ExportJob) -> Result<ExportJobId, ExportWorkerError> {
        if self.shutdown.load(Ordering::Acquire) {
            return Err(ExportWorkerError::WorkerStopped);
        }
        let id = self.queue.submit(job)?;
        match self.wake_sender.as_ref().map(|sender| sender.try_send(())) {
            Some(Ok(())) | Some(Err(TrySendError::Full(()))) => Ok(id),
            Some(Err(TrySendError::Disconnected(()))) | None => {
                if matches!(
                    self.queue.cancel(id),
                    Ok(CancelOutcome::ActiveCancellationRequested)
                ) {
                    let _ = self.queue.finish_cancelled(id);
                }
                Err(ExportWorkerError::WorkerStopped)
            }
        }
    }

    pub fn cancel(&self, id: ExportJobId) -> Result<CancelOutcome, ExportWorkerError> {
        let outcome = self.queue.cancel(id)?;
        let _ = self
            .wake_sender
            .as_ref()
            .and_then(|sender| sender.try_send(()).ok());
        Ok(outcome)
    }

    pub fn queue(&self) -> &Arc<ExportQueue> {
        &self.queue
    }

    pub fn status(&self, id: ExportJobId) -> Option<ExportStatus> {
        self.queue.status(id)
    }

    pub fn state(&self, id: ExportJobId) -> Option<ExportJobState> {
        self.queue.state(id)
    }

    pub fn states(&self) -> Vec<ExportJobState> {
        self.queue.states()
    }

    pub fn shutdown(mut self) {
        self.stop_and_join();
    }

    fn stop_and_join(&mut self) {
        if !self.shutdown.swap(true, Ordering::AcqRel) {
            cancel_accepted_jobs(&self.queue);
        }
        self.wake_sender.take();
        if let Some(worker) = self.worker.take() {
            if worker.join().is_err() {
                finish_orphaned_jobs(&self.queue);
            }
        }
        finish_orphaned_jobs(&self.queue);
        self.queue.release_worker();
    }
}

impl<R: ProcessRunner + 'static> Drop for ExportWorker<R> {
    fn drop(&mut self) {
        self.stop_and_join();
    }
}

fn run_worker<R: ProcessRunner + 'static>(
    queue: Arc<ExportQueue>,
    exporter: FfmpegExporter<R>,
    wake_receiver: Receiver<()>,
    shutdown: Arc<AtomicBool>,
) {
    loop {
        if shutdown.load(Ordering::Acquire) {
            cancel_accepted_jobs(&queue);
        }

        match queue.active_snapshot() {
            Ok(Some((id, job, cancellation))) => {
                run_active_job(&queue, &exporter, id, job, cancellation);
                continue;
            }
            Ok(None) => {}
            Err(ExportQueueError::Busy) => {
                std::thread::sleep(Duration::from_millis(1));
                continue;
            }
            Err(_) => return,
        }
        if shutdown.load(Ordering::Acquire) {
            return;
        }

        match wake_receiver.recv_timeout(WORKER_WAKE_INTERVAL) {
            Ok(()) | Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// Keep post-save probing and export bookkeeping below the capture/game
/// scheduling path. The spawned FFmpeg child receives the same policy in the
/// system process adapter.
#[cfg(windows)]
fn enter_background_processing_mode() {
    use windows_sys::Win32::System::Threading::{
        GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN,
    };

    unsafe {
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN);
    }
}

#[cfg(not(windows))]
fn enter_background_processing_mode() {}

fn run_active_job<R: ProcessRunner + 'static>(
    queue: &ExportQueue,
    exporter: &FfmpegExporter<R>,
    id: ExportJobId,
    job: ExportJob,
    cancellation: clip_export::CancellationToken,
) {
    match exporter.export_with_progress(&job, &cancellation, |progress: ExportProgress| {
        let _ = queue.update_progress(id, progress);
    }) {
        Ok(result) => match complete_with_retry(queue, id) {
            Ok(()) => {}
            Err(ExportQueueError::CancellationRequested { .. }) => {
                let _ = fs::remove_file(result.destination_path);
                let _ = finish_cancelled_with_retry(queue, id);
            }
            Err(error) => {
                let _ = fail_with_retry(queue, id, &error.to_string());
            }
        },
        Err(ExportError::Cancelled) => finish_cancelled(queue, id),
        Err(error) => fail_or_cancel(queue, id, error),
    }
}

fn finish_cancelled(queue: &ExportQueue, id: ExportJobId) {
    match finish_cancelled_with_retry(queue, id) {
        Ok(()) => {}
        Err(ExportQueueError::CancellationNotRequested { .. }) => {
            let _ = fail_with_retry(queue, id, "backend reported cancellation without a request");
        }
        Err(_) => {}
    }
}

fn fail_or_cancel(queue: &ExportQueue, id: ExportJobId, error: ExportError) {
    let reason = format!("[{}] {}", error.code(), error);
    match fail_with_retry(queue, id, &reason) {
        Ok(()) => {}
        Err(ExportQueueError::CancellationRequested { .. }) => finish_cancelled(queue, id),
        Err(_) => {}
    }
}

fn cancel_accepted_jobs(queue: &ExportQueue) {
    let states = loop {
        match queue.states_result() {
            Ok(states) => break states,
            Err(ExportQueueError::Busy) => std::thread::sleep(Duration::from_millis(1)),
            Err(_) => return,
        }
    };
    for state in states {
        if matches!(state.status, ExportStatus::Active | ExportStatus::Queued) {
            let _ = cancel_with_retry(queue, state.id);
        }
    }
}

fn complete_with_retry(queue: &ExportQueue, id: ExportJobId) -> Result<(), ExportQueueError> {
    retry_queue_operation(|| queue.complete(id))
}

fn finish_cancelled_with_retry(
    queue: &ExportQueue,
    id: ExportJobId,
) -> Result<(), ExportQueueError> {
    retry_queue_operation(|| queue.finish_cancelled(id))
}

fn fail_with_retry(
    queue: &ExportQueue,
    id: ExportJobId,
    reason: &str,
) -> Result<(), ExportQueueError> {
    retry_queue_operation(|| queue.fail(id, reason))
}

fn cancel_with_retry(
    queue: &ExportQueue,
    id: ExportJobId,
) -> Result<CancelOutcome, ExportQueueError> {
    retry_queue_operation(|| queue.cancel(id))
}

fn retry_queue_operation<T>(
    mut operation: impl FnMut() -> Result<T, ExportQueueError>,
) -> Result<T, ExportQueueError> {
    loop {
        match operation() {
            Err(ExportQueueError::Busy) => std::thread::sleep(Duration::from_millis(1)),
            result => return result,
        }
    }
}

fn finish_orphaned_jobs(queue: &ExportQueue) {
    let states = loop {
        match queue.states_result() {
            Ok(states) => break states,
            Err(ExportQueueError::Busy) => std::thread::sleep(Duration::from_millis(1)),
            Err(_) => return,
        }
    };
    for state in states {
        match state.status {
            ExportStatus::Active if state.cancellation_requested => {
                let _ = finish_cancelled_with_retry(queue, state.id);
            }
            ExportStatus::Active => {
                let _ = fail_with_retry(queue, state.id, "export worker stopped unexpectedly");
            }
            ExportStatus::Queued => {
                let _ = cancel_with_retry(queue, state.id);
            }
            ExportStatus::Completed | ExportStatus::Failed { .. } | ExportStatus::Cancelled => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    use clip_export::{
        plan, CancellationToken, ExportJob, ExportQueue, PlannerRequest, TargetSize, VideoSource,
    };

    use super::*;
    use crate::process::{ProcessOutput, ProcessRunner, ProcessRunnerError};
    use crate::VerifiedFfmpegToolchain;

    struct FakeRunner {}

    impl ProcessRunner for FakeRunner {
        fn run(
            &self,
            executable: &Path,
            args: &[std::ffi::OsString],
            cancellation: &CancellationToken,
        ) -> Result<ProcessOutput, ProcessRunnerError> {
            if cancellation.is_cancelled() {
                return Ok(ProcessOutput {
                    success: false,
                    cancelled: true,
                    truncated: false,
                    exit_code: None,
                    stdout: String::new(),
                    stderr: String::new(),
                });
            }
            if executable.to_string_lossy().contains("ffprobe") {
                let stdout = if args.iter().any(|arg| arg == "-show_packets") {
                    valid_packet_probe()
                } else {
                    valid_probe()
                };
                return Ok(ProcessOutput {
                    success: true,
                    cancelled: false,
                    truncated: false,
                    exit_code: Some(0),
                    stdout,
                    stderr: String::new(),
                });
            }
            if args
                .iter()
                .any(|argument| argument.to_string_lossy() == "null")
            {
                return Ok(success_output());
            }
            let output_path = args.last().expect("output path");
            fs::write(PathBuf::from(output_path), b"fake-mp4").expect("fake output");
            Ok(success_output())
        }
    }

    struct BlockingRunner {
        started: Arc<AtomicBool>,
    }

    impl ProcessRunner for BlockingRunner {
        fn run(
            &self,
            executable: &Path,
            args: &[std::ffi::OsString],
            cancellation: &CancellationToken,
        ) -> Result<ProcessOutput, ProcessRunnerError> {
            if executable.to_string_lossy().contains("ffprobe") {
                return Ok(ProcessOutput {
                    success: true,
                    cancelled: false,
                    truncated: false,
                    exit_code: Some(0),
                    stdout: valid_packet_probe(),
                    stderr: String::new(),
                });
            }
            if args
                .iter()
                .any(|argument| argument.to_string_lossy() == "null")
            {
                return Ok(success_output());
            }
            self.started.store(true, Ordering::Release);
            while !cancellation.is_cancelled() {
                std::thread::sleep(Duration::from_millis(1));
            }
            Ok(ProcessOutput {
                success: false,
                cancelled: true,
                truncated: false,
                exit_code: None,
                stdout: String::new(),
                stderr: String::new(),
            })
        }
    }

    fn success_output() -> ProcessOutput {
        ProcessOutput {
            success: true,
            cancelled: false,
            truncated: false,
            exit_code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
        }
    }

    fn valid_probe() -> String {
        r#"{
            "streams": [
                {"codec_type":"video","codec_name":"h264","width":1920,"height":1080,"r_frame_rate":"60/1"},
                {"codec_type":"audio","codec_name":"aac","sample_rate":"48000","channels":2}
            ],
            "format": {"format_name":"mov,mp4,m4a,3gp,3g2,mj2","duration":"60.000000"}
        }"#
        .to_string()
    }

    fn valid_packet_probe() -> String {
        "0,0.000000\n1,0.000000\n0,0.016667\n1,0.021333\n".to_string()
    }

    fn toolchain() -> VerifiedFfmpegToolchain {
        VerifiedFfmpegToolchain::for_tests(
            PathBuf::from("ffmpeg.exe"),
            PathBuf::from("ffprobe.exe"),
            "ffmpeg version test".to_string(),
            "ffprobe version test".to_string(),
            "test".to_string(),
        )
    }

    fn temp_directory() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("silk-ffmpeg-worker-{nonce}"));
        fs::create_dir_all(&path).expect("temp directory");
        path
    }

    fn job(directory: &Path) -> ExportJob {
        let request = PlannerRequest::new(
            TargetSize::Medium,
            60_000_000,
            192_000,
            VideoSource::new(1_920, 1_080, 60),
        );
        ExportJob::new(
            directory.join("source.mp4"),
            directory.join("export.mp4"),
            plan(&request).expect("plan"),
        )
    }

    #[test]
    fn worker_runs_export_off_submitter_and_publishes_terminal_state() {
        let directory = temp_directory();
        let job = job(&directory);
        fs::write(&job.source_path, b"source").expect("source");
        let queue = Arc::new(ExportQueue::new());
        let exporter = FfmpegExporter::with_runner(toolchain(), FakeRunner {});
        let worker = ExportWorker::with_exporter(Arc::clone(&queue), exporter).expect("worker");
        let id = worker.submit(job.clone()).expect("submit");
        let deadline = Instant::now() + Duration::from_secs(2);
        while queue.status(id) != Some(ExportStatus::Completed) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
        }

        assert_eq!(queue.status(id), Some(ExportStatus::Completed));
        assert_eq!(
            queue.state(id).expect("terminal state").progress.percent,
            100
        );
        assert!(job.destination_path.is_file());
        assert!(!PathBuf::from(format!("{}.part", job.destination_path.display())).exists());
        worker.shutdown();
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn worker_cancels_an_active_process_and_records_terminal_state() {
        let directory = temp_directory();
        let job = job(&directory);
        fs::write(&job.source_path, b"source").expect("source");
        let queue = Arc::new(ExportQueue::new());
        let started = Arc::new(AtomicBool::new(false));
        let exporter = FfmpegExporter::with_runner(
            toolchain(),
            BlockingRunner {
                started: Arc::clone(&started),
            },
        );
        let worker = ExportWorker::with_exporter(Arc::clone(&queue), exporter).expect("worker");
        assert!(matches!(
            ExportWorker::with_exporter(
                Arc::clone(&queue),
                FfmpegExporter::with_runner(toolchain(), FakeRunner {})
            ),
            Err(ExportWorkerError::WorkerAlreadyRunning)
        ));
        let id = worker.submit(job.clone()).expect("submit");
        let deadline = Instant::now() + Duration::from_secs(2);
        while !started.load(Ordering::Acquire) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
        }

        assert!(started.load(Ordering::Acquire));
        assert_eq!(
            worker.cancel(id).expect("cancel"),
            CancelOutcome::ActiveCancellationRequested
        );
        let deadline = Instant::now() + Duration::from_secs(2);
        while queue.status(id) != Some(ExportStatus::Cancelled) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
        }

        assert_eq!(queue.status(id), Some(ExportStatus::Cancelled));
        assert!(!job.destination_path.exists());
        worker.shutdown();
        let _ = fs::remove_dir_all(directory);
    }
}
