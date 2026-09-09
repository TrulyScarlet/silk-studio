use std::collections::VecDeque;
use std::fmt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};

use thiserror::Error;

use crate::planner::ExportPlan;

/// Exactly one job may occupy the active slot.
pub const ACTIVE_JOB_LIMIT: usize = 1;
/// At most two jobs wait behind the active slot.
pub const PENDING_JOB_LIMIT: usize = 2;
/// Default number of terminal records retained for status/history inspection.
pub const TERMINAL_HISTORY_LIMIT: usize = 32;

pub const EXPORT_ACTIVE_JOB_LIMIT: usize = ACTIVE_JOB_LIMIT;
pub const EXPORT_PENDING_JOB_LIMIT: usize = PENDING_JOB_LIMIT;
pub const EXPORT_TERMINAL_HISTORY_LIMIT: usize = TERMINAL_HISTORY_LIMIT;

/// A stable identifier assigned when an export is accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ExportJobId(pub u64);

impl ExportJobId {
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for ExportJobId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Backend-neutral work description for one post-save export.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportJob {
    pub source_path: PathBuf,
    pub destination_path: PathBuf,
    pub plan: ExportPlan,
}

impl ExportJob {
    pub fn new(
        source_path: impl Into<PathBuf>,
        destination_path: impl Into<PathBuf>,
        plan: ExportPlan,
    ) -> Self {
        Self {
            source_path: source_path.into(),
            destination_path: destination_path.into(),
            plan,
        }
    }
}

/// Coarse stages exposed while an export worker is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportStage {
    Preparing,
    Encoding,
    Probing,
    Decoding,
    Publishing,
}

impl ExportStage {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Preparing => "preparing",
            Self::Encoding => "encoding",
            Self::Probing => "probing",
            Self::Decoding => "decoding",
            Self::Publishing => "publishing",
        }
    }
}

/// Bounded, best-effort progress for one active export.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExportProgress {
    pub stage: ExportStage,
    pub percent: u8,
}

impl Default for ExportProgress {
    fn default() -> Self {
        Self {
            stage: ExportStage::Preparing,
            percent: 0,
        }
    }
}

impl ExportProgress {
    pub const fn new(stage: ExportStage, percent: u8) -> Self {
        Self {
            stage,
            percent: if percent > 100 { 100 } else { percent },
        }
    }
}

/// Cooperative cancellation signal for a future backend executor.
#[derive(Clone, Debug)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    pub fn new() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

/// Observable lifecycle state. Active cancellation remains cooperative until
/// the eventual backend reports that the job stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportStatus {
    Queued,
    Active,
    Completed,
    Failed { reason: String },
    Cancelled,
}

impl ExportStatus {
    pub const fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed { .. } | Self::Cancelled
        )
    }
}

/// Immutable status snapshot for an accepted job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportJobState {
    pub id: ExportJobId,
    pub job: ExportJob,
    pub status: ExportStatus,
    pub progress: ExportProgress,
    pub cancellation_requested: bool,
}

/// Result of requesting cancellation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelOutcome {
    /// The pending job was removed and is already terminal.
    QueuedCancelled,
    /// The active job received a cooperative cancellation signal.
    ActiveCancellationRequested,
}

/// Errors from the bounded export queue.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ExportQueueError {
    #[error("export queue is full: {active_capacity} active and {pending_capacity} pending")]
    QueueFull {
        active_capacity: usize,
        pending_capacity: usize,
    },

    #[error("export queue is busy; retry without waiting")]
    Busy,

    #[error("export queue state is unavailable")]
    StateUnavailable,

    #[error("unknown export job {id}")]
    UnknownJob { id: ExportJobId },

    #[error("export job {id} is not active")]
    NotActive { id: ExportJobId },

    #[error("export job {id} has a cancellation request and must finish as cancelled")]
    CancellationRequested { id: ExportJobId },

    #[error("export job {id} has no active cancellation request")]
    CancellationNotRequested { id: ExportJobId },

    #[error("export job ID space is exhausted")]
    JobIdExhausted,
}

impl ExportQueueError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::QueueFull { .. } => "CLIP_EXPORT_QUEUE_FULL",
            Self::Busy => "CLIP_EXPORT_QUEUE_BUSY",
            Self::StateUnavailable => "CLIP_EXPORT_QUEUE_UNAVAILABLE",
            Self::UnknownJob { .. } => "CLIP_EXPORT_UNKNOWN_JOB",
            Self::NotActive { .. } => "CLIP_EXPORT_JOB_NOT_ACTIVE",
            Self::CancellationRequested { .. } => "CLIP_EXPORT_CANCELLATION_REQUESTED",
            Self::CancellationNotRequested { .. } => "CLIP_EXPORT_CANCELLATION_NOT_REQUESTED",
            Self::JobIdExhausted => "CLIP_EXPORT_JOB_ID_EXHAUSTED",
        }
    }
}

pub type QueueError = ExportQueueError;

#[derive(Debug)]
struct QueueEntry {
    id: ExportJobId,
    job: ExportJob,
    cancellation: CancellationToken,
    progress: ExportProgress,
}

#[derive(Debug)]
struct QueueState {
    next_id: u64,
    active: Option<QueueEntry>,
    pending: VecDeque<QueueEntry>,
    terminal_history: VecDeque<ExportJobState>,
}

impl Default for QueueState {
    fn default() -> Self {
        Self {
            next_id: 1,
            active: None,
            pending: VecDeque::new(),
            terminal_history: VecDeque::new(),
        }
    }
}

/// In-memory bounded export state machine.
///
/// Submission is a `try_lock` operation over a short state-only critical
/// section. It never waits for an active export and never performs backend or
/// filesystem work. The first accepted job occupies the active slot so a
/// caller can hand it to a future executor through [`Self::active_job`].
#[derive(Debug)]
pub struct ExportQueue {
    state: Mutex<QueueState>,
    terminal_history_limit: usize,
    worker_claimed: AtomicBool,
}

impl Default for ExportQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl ExportQueue {
    pub fn new() -> Self {
        Self::with_terminal_history_limit(TERMINAL_HISTORY_LIMIT)
    }

    pub fn with_terminal_history_limit(terminal_history_limit: usize) -> Self {
        Self {
            state: Mutex::new(QueueState::default()),
            terminal_history_limit,
            worker_claimed: AtomicBool::new(false),
        }
    }

    /// Claim the single executor lease associated with this queue.
    pub fn try_claim_worker(&self) -> bool {
        self.worker_claimed
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    /// Release the executor lease after its worker has stopped.
    pub fn release_worker(&self) {
        self.worker_claimed.store(false, Ordering::Release);
    }

    /// Accept a job without waiting for the active slot or a backend.
    pub fn submit(&self, job: ExportJob) -> Result<ExportJobId, ExportQueueError> {
        let mut state = self.try_state()?;
        if state.active.is_some() && state.pending.len() >= PENDING_JOB_LIMIT {
            return Err(ExportQueueError::QueueFull {
                active_capacity: ACTIVE_JOB_LIMIT,
                pending_capacity: PENDING_JOB_LIMIT,
            });
        }

        let id = ExportJobId(state.next_id);
        state.next_id = state
            .next_id
            .checked_add(1)
            .ok_or(ExportQueueError::JobIdExhausted)?;
        let entry = QueueEntry {
            id,
            job,
            cancellation: CancellationToken::new(),
            progress: ExportProgress::default(),
        };
        if state.active.is_none() {
            state.active = Some(entry);
        } else {
            state.pending.push_back(entry);
        }
        Ok(id)
    }

    /// Explicit spelling for callers that want to emphasize nonblocking
    /// submission. [`Self::submit`] is already nonblocking.
    pub fn try_submit(&self, job: ExportJob) -> Result<ExportJobId, ExportQueueError> {
        self.submit(job)
    }

    pub fn active_id(&self) -> Option<ExportJobId> {
        self.try_state()
            .ok()
            .and_then(|state| state.active.as_ref().map(|entry| entry.id))
    }

    pub fn active_job(&self) -> Option<ExportJob> {
        self.try_state()
            .ok()
            .and_then(|state| state.active.as_ref().map(|entry| entry.job.clone()))
    }

    pub fn active_cancellation_token(&self) -> Option<CancellationToken> {
        self.try_state().ok().and_then(|state| {
            state
                .active
                .as_ref()
                .map(|entry| entry.cancellation.clone())
        })
    }

    pub fn cancellation_token(&self, id: ExportJobId) -> Option<CancellationToken> {
        let state = self.try_state().ok()?;
        state
            .active
            .iter()
            .chain(state.pending.iter())
            .find(|entry| entry.id == id)
            .map(|entry| entry.cancellation.clone())
    }

    pub fn pending_count(&self) -> usize {
        self.try_state()
            .map(|state| state.pending.len())
            .unwrap_or_default()
    }

    pub fn pending_jobs(&self) -> Vec<ExportJob> {
        self.try_state()
            .map(|state| {
                state
                    .pending
                    .iter()
                    .map(|entry| entry.job.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn len(&self) -> usize {
        self.try_state()
            .map(|state| usize::from(state.active.is_some()) + state.pending.len())
            .unwrap_or_default()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn is_full(&self) -> bool {
        self.try_state()
            .map(|state| state.active.is_some() && state.pending.len() >= PENDING_JOB_LIMIT)
            .unwrap_or(false)
    }

    pub fn status(&self, id: ExportJobId) -> Option<ExportStatus> {
        self.state(id).map(|state| state.status)
    }

    pub fn state(&self, id: ExportJobId) -> Option<ExportJobState> {
        let state = self.try_state().ok()?;
        state_snapshot(&state, id)
    }

    /// Update active progress without waiting for a worker or filesystem lock.
    pub fn update_progress(
        &self,
        id: ExportJobId,
        progress: ExportProgress,
    ) -> Result<(), ExportQueueError> {
        let mut state = self.try_state()?;
        let Some(active) = state.active.as_mut().filter(|entry| entry.id == id) else {
            return Err(ExportQueueError::NotActive { id });
        };
        active.progress = ExportProgress::new(progress.stage, progress.percent);
        Ok(())
    }

    /// Return active, pending, and retained terminal states in deterministic
    /// queue/history order.
    pub fn states(&self) -> Vec<ExportJobState> {
        self.states_result().unwrap_or_default()
    }

    pub fn states_result(&self) -> Result<Vec<ExportJobState>, ExportQueueError> {
        let state = self.try_state()?;
        let mut states = Vec::with_capacity(
            usize::from(state.active.is_some())
                + state.pending.len()
                + state.terminal_history.len(),
        );
        if let Some(active) = &state.active {
            states.push(snapshot_entry(
                active,
                ExportStatus::Active,
                active.progress,
                active.cancellation.is_cancelled(),
            ));
        }
        states.extend(
            state
                .pending
                .iter()
                .map(|entry| snapshot_entry(entry, ExportStatus::Queued, entry.progress, false)),
        );
        states.extend(state.terminal_history.iter().cloned());
        Ok(states)
    }

    /// Read the active job and its cancellation signal under one short lock.
    /// Workers use this snapshot to avoid observing mismatched queue fields or
    /// treating transient queue contention as backend work.
    pub fn active_snapshot(
        &self,
    ) -> Result<Option<(ExportJobId, ExportJob, CancellationToken)>, ExportQueueError> {
        let state = self.try_state()?;
        Ok(state
            .active
            .as_ref()
            .map(|entry| (entry.id, entry.job.clone(), entry.cancellation.clone())))
    }

    /// Terminal history is oldest-first and bounded by the constructor limit.
    pub fn terminal_history(&self) -> Vec<ExportJobState> {
        self.try_state()
            .map(|state| state.terminal_history.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Cancel a queued job immediately, or signal cancellation for the active
    /// job. Active work remains active until [`Self::finish_cancelled`] is
    /// called by the eventual backend executor.
    pub fn cancel(&self, id: ExportJobId) -> Result<CancelOutcome, ExportQueueError> {
        let mut state = self.try_state()?;
        if let Some(active) = state.active.as_ref() {
            if active.id == id {
                active.cancellation.cancel();
                return Ok(CancelOutcome::ActiveCancellationRequested);
            }
        }

        if let Some(position) = state.pending.iter().position(|entry| entry.id == id) {
            let entry = state
                .pending
                .remove(position)
                .expect("pending position was found");
            entry.cancellation.cancel();
            push_terminal(
                &mut state,
                entry,
                ExportStatus::Cancelled,
                true,
                self.terminal_history_limit,
            );
            return Ok(CancelOutcome::QueuedCancelled);
        }

        if state.terminal_history.iter().any(|entry| entry.id == id) {
            return Err(ExportQueueError::NotActive { id });
        }
        Err(ExportQueueError::UnknownJob { id })
    }

    /// Mark the active job complete and promote the oldest pending job.
    pub fn complete(&self, id: ExportJobId) -> Result<(), ExportQueueError> {
        let mut state = self.try_state()?;
        let entry = take_active(&mut state, id)?;
        if entry.cancellation.is_cancelled() {
            state.active = Some(entry);
            return Err(ExportQueueError::CancellationRequested { id });
        }
        push_terminal(
            &mut state,
            entry,
            ExportStatus::Completed,
            false,
            self.terminal_history_limit,
        );
        promote_next(&mut state);
        Ok(())
    }

    /// Mark the active job failed without invoking any backend itself.
    pub fn fail(&self, id: ExportJobId, reason: impl Into<String>) -> Result<(), ExportQueueError> {
        let mut state = self.try_state()?;
        let entry = take_active(&mut state, id)?;
        if entry.cancellation.is_cancelled() {
            state.active = Some(entry);
            return Err(ExportQueueError::CancellationRequested { id });
        }
        push_terminal(
            &mut state,
            entry,
            ExportStatus::Failed {
                reason: reason.into(),
            },
            false,
            self.terminal_history_limit,
        );
        promote_next(&mut state);
        Ok(())
    }

    /// Complete an active cancellation after the backend has stopped work.
    pub fn finish_cancelled(&self, id: ExportJobId) -> Result<(), ExportQueueError> {
        let mut state = self.try_state()?;
        let entry = take_active(&mut state, id)?;
        if !entry.cancellation.is_cancelled() {
            state.active = Some(entry);
            return Err(ExportQueueError::CancellationNotRequested { id });
        }
        push_terminal(
            &mut state,
            entry,
            ExportStatus::Cancelled,
            true,
            self.terminal_history_limit,
        );
        promote_next(&mut state);
        Ok(())
    }

    fn try_state(&self) -> Result<MutexGuard<'_, QueueState>, ExportQueueError> {
        match self.state.try_lock() {
            Ok(state) => Ok(state),
            Err(TryLockError::WouldBlock) => Err(ExportQueueError::Busy),
            Err(TryLockError::Poisoned(_)) => Err(ExportQueueError::StateUnavailable),
        }
    }
}

fn take_active(state: &mut QueueState, id: ExportJobId) -> Result<QueueEntry, ExportQueueError> {
    let Some(active) = state.active.take() else {
        return Err(ExportQueueError::NotActive { id });
    };
    if active.id != id {
        state.active = Some(active);
        return Err(ExportQueueError::NotActive { id });
    }
    Ok(active)
}

fn promote_next(state: &mut QueueState) {
    if state.active.is_none() {
        state.active = state.pending.pop_front();
    }
}

fn push_terminal(
    state: &mut QueueState,
    entry: QueueEntry,
    status: ExportStatus,
    cancellation_requested: bool,
    history_limit: usize,
) {
    if history_limit == 0 {
        return;
    }
    if state.terminal_history.len() >= history_limit {
        state.terminal_history.pop_front();
    }
    state.terminal_history.push_back(ExportJobState {
        id: entry.id,
        job: entry.job,
        status,
        progress: entry.progress,
        cancellation_requested,
    });
}

fn snapshot_entry(
    entry: &QueueEntry,
    status: ExportStatus,
    progress: ExportProgress,
    cancellation_requested: bool,
) -> ExportJobState {
    ExportJobState {
        id: entry.id,
        job: entry.job.clone(),
        status,
        progress,
        cancellation_requested,
    }
}

fn state_snapshot(state: &QueueState, id: ExportJobId) -> Option<ExportJobState> {
    if let Some(active) = state.active.as_ref().filter(|entry| entry.id == id) {
        return Some(snapshot_entry(
            active,
            ExportStatus::Active,
            active.progress,
            active.cancellation.is_cancelled(),
        ));
    }
    if let Some(pending) = state.pending.iter().find(|entry| entry.id == id) {
        return Some(snapshot_entry(
            pending,
            ExportStatus::Queued,
            pending.progress,
            false,
        ));
    }
    state
        .terminal_history
        .iter()
        .find(|entry| entry.id == id)
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::planner::{PlannerRequest, TargetBytePlanner};

    fn plan() -> ExportPlan {
        TargetBytePlanner::new()
            .plan(&PlannerRequest::default())
            .expect("default plan")
    }

    fn job(number: u8) -> ExportJob {
        ExportJob::new(
            format!("source-{number}.mp4"),
            format!("destination-{number}.mp4"),
            plan(),
        )
    }

    #[test]
    fn accepts_one_active_and_two_pending_jobs_only() {
        let queue = ExportQueue::new();
        let first = queue.submit(job(1)).expect("first");
        let second = queue.submit(job(2)).expect("second");
        let third = queue.submit(job(3)).expect("third");

        assert_eq!(queue.active_id(), Some(first));
        assert_eq!(queue.status(first), Some(ExportStatus::Active));
        assert_eq!(queue.status(second), Some(ExportStatus::Queued));
        assert_eq!(queue.status(third), Some(ExportStatus::Queued));
        assert_eq!(queue.pending_count(), PENDING_JOB_LIMIT);
        assert!(queue.is_full());
        assert!(matches!(
            queue.try_submit(job(4)),
            Err(ExportQueueError::QueueFull {
                active_capacity: ACTIVE_JOB_LIMIT,
                pending_capacity: PENDING_JOB_LIMIT,
            })
        ));
    }

    #[test]
    fn queued_cancellation_is_terminal_and_frees_a_pending_slot() {
        let queue = ExportQueue::new();
        let active = queue.submit(job(1)).expect("active");
        let queued = queue.submit(job(2)).expect("queued");
        let last = queue.submit(job(3)).expect("queued");
        let token = queue.cancellation_token(queued).expect("token");

        assert_eq!(
            queue.cancel(queued).expect("cancel"),
            CancelOutcome::QueuedCancelled
        );
        assert!(token.is_cancelled());
        assert_eq!(queue.status(queued), Some(ExportStatus::Cancelled));
        assert_eq!(queue.active_id(), Some(active));
        assert_eq!(queue.pending_jobs().len(), 1);

        let replacement = queue.submit(job(4)).expect("replacement");
        assert_eq!(
            queue
                .pending_jobs()
                .iter()
                .map(|item| item.source_path.to_string_lossy().into_owned())
                .collect::<Vec<_>>(),
            vec![
                format!("source-{}.mp4", last.get()),
                "source-4.mp4".to_string()
            ]
        );
        assert_eq!(queue.status(replacement), Some(ExportStatus::Queued));
    }

    #[test]
    fn active_cancellation_is_cooperative_and_promotes_after_finish() {
        let queue = ExportQueue::new();
        let active = queue.submit(job(1)).expect("active");
        let next = queue.submit(job(2)).expect("queued");
        let token = queue.active_cancellation_token().expect("active token");

        assert_eq!(
            queue.cancel(active).expect("request cancellation"),
            CancelOutcome::ActiveCancellationRequested
        );
        assert!(token.is_cancelled());
        assert_eq!(queue.status(active), Some(ExportStatus::Active));
        assert!(matches!(
            queue.complete(active),
            Err(ExportQueueError::CancellationRequested { .. })
        ));

        queue.finish_cancelled(active).expect("finish cancellation");
        assert_eq!(queue.status(active), Some(ExportStatus::Cancelled));
        assert_eq!(queue.active_id(), Some(next));
        assert_eq!(queue.status(next), Some(ExportStatus::Active));
    }

    #[test]
    fn active_progress_is_bounded_and_visible_in_state_snapshots() {
        let queue = ExportQueue::new();
        let id = queue.submit(job(1)).expect("active");

        queue
            .update_progress(id, ExportProgress::new(ExportStage::Decoding, 150))
            .expect("progress update");

        let state = queue.state(id).expect("state");
        assert_eq!(state.progress.stage, ExportStage::Decoding);
        assert_eq!(state.progress.percent, 100);
    }

    #[test]
    fn completion_failure_and_history_limit_are_deterministic() {
        let queue = ExportQueue::with_terminal_history_limit(2);
        let first = queue.submit(job(1)).expect("first");
        let second = queue.submit(job(2)).expect("second");
        let third = queue.submit(job(3)).expect("third");

        queue.complete(first).expect("complete first");
        queue
            .fail(second, "backend not installed")
            .expect("fail second");
        queue.cancel(third).expect("cancel third");
        queue.finish_cancelled(third).expect("finish third");

        let history = queue.terminal_history();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].id, second);
        assert_eq!(history[1].id, third);
        assert_eq!(queue.status(first), None);
        assert!(matches!(
            history[0].status,
            ExportStatus::Failed { ref reason } if reason == "backend not installed"
        ));
        assert_eq!(queue.status(third), Some(ExportStatus::Cancelled));
    }

    #[test]
    fn zero_history_limit_retains_no_terminal_records() {
        let queue = ExportQueue::with_terminal_history_limit(0);
        let id = queue.submit(job(1)).expect("job");
        queue.complete(id).expect("complete");

        assert!(queue.terminal_history().is_empty());
        assert_eq!(queue.status(id), None);
        assert!(queue.is_empty());
    }

    #[test]
    fn state_snapshot_exposes_job_and_cancellation_request() {
        let queue = ExportQueue::new();
        let id = queue.submit(job(1)).expect("job");
        queue.cancel(id).expect("request cancellation");

        let state = queue.state(id).expect("state");
        assert_eq!(state.id, id);
        assert_eq!(state.job.source_path, PathBuf::from("source-1.mp4"));
        assert_eq!(state.status, ExportStatus::Active);
        assert!(state.cancellation_requested);
    }

    #[test]
    fn queue_allows_only_one_executor_lease() {
        let queue = ExportQueue::new();

        assert!(queue.try_claim_worker());
        assert!(!queue.try_claim_worker());
        queue.release_worker();
        assert!(queue.try_claim_worker());
        queue.release_worker();
    }

    #[test]
    fn active_snapshot_reads_job_and_token_consistently() {
        let queue = ExportQueue::new();
        let id = queue.submit(job(1)).expect("job");

        let snapshot = queue
            .active_snapshot()
            .expect("snapshot")
            .expect("active job");

        assert_eq!(snapshot.0, id);
        assert_eq!(snapshot.1.source_path, PathBuf::from("source-1.mp4"));
        assert!(!snapshot.2.is_cancelled());
    }
}
