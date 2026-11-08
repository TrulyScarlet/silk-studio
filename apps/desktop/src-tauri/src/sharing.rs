//! Post-save sharing. File I/O and exports run away from the window/capture threads.
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use clip_export::{
    ExportJob, ExportJobId, ExportPlan, ExportQueue, ExportStatus, PlannerRequest, Ratio,
    TargetBytePlanner, TargetSize, VideoRung, VideoSource,
};
use encoder_ffmpeg::{ExportWorker, FfmpegExporter, FfmpegToolchain};
use serde::Serialize;
use tauri::{AppHandle, Manager, State};

#[derive(Default)]
pub struct ShareRuntime(Mutex<Option<ExportWorker>>);

impl ShareRuntime {
    pub(crate) fn ensure_idle(&self) -> Result<(), String> {
        let guard = self.0.lock().map_err(|e| e.to_string())?;
        if guard
            .as_ref()
            .is_some_and(|worker| worker.states().iter().any(|job| !job.status.is_terminal()))
        {
            return Err("Wait for the clip share to finish, then install the update.".into());
        }
        Ok(())
    }
}
static NEXT_FILE: AtomicU64 = AtomicU64::new(0);
const CACHE_LIMIT_BYTES: u64 = 2_000_000_000;
const CACHE_LIMIT_FILES: usize = 64;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareState {
    job_id: Option<u64>,
    status: &'static str,
    stage: String,
    size_bytes: Option<u64>,
    error: Option<String>,
}

#[tauri::command]
pub async fn prepare_clip_share(
    app: AppHandle,
    path: PathBuf,
    target_mb: Option<u32>,
) -> Result<ShareState, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let source =
            crate::shell::library_action_path(&app.state::<crate::shell::ShellState>(), path)?;
        if target_mb.is_none() {
            return Ok(ShareState {
                job_id: None,
                status: "ready",
                stage: "ready".into(),
                size_bytes: Some(source.metadata().map_err(|e| e.to_string())?.len()),
                error: None,
            });
        }
        let target = target_bytes(target_mb.unwrap())?;
        let runtime = app.state::<ShareRuntime>();
        let mut guard = runtime
            .0
            .try_lock()
            .map_err(|_| "Another clip is being prepared. Please try again.".to_string())?;
        if app.state::<crate::shell::ShellState>().is_updating() {
            return Err("Silk is restarting to install an update.".into());
        }
        if guard.is_none() {
            let verified = discover_toolchain()?
                .verify()
                .map_err(|e| format!("Sharing tools could not be verified: {e}"))?;
            *guard = Some(
                ExportWorker::new(Arc::new(ExportQueue::new()), FfmpegExporter::new(verified))
                    .map_err(|e| e.to_string())?,
            );
        }
        let worker = guard.as_ref().unwrap();
        // Only one share preparation at a time; the underlying worker is bounded too.
        if worker.states().iter().any(|job| !job.status.is_terminal()) {
            return Err(
                "A clip is still being prepared or cancelled. Please try again in a moment.".into(),
            );
        }
        let cache = app
            .path()
            .app_cache_dir()
            .map_err(|e| e.to_string())?
            .join("shares");
        prepare_cache(&cache, target)?;
        let unique = NEXT_FILE.fetch_add(1, Ordering::Relaxed);
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_millis();
        let destination = cache.join(format!(
            "silk-share-{timestamp}-{unique}-{}MB.mp4",
            target_mb.unwrap()
        ));
        let job = make_job(&source, destination, target)?;
        let id = worker.submit(job).map_err(|e| e.to_string())?;
        Ok(ShareState {
            job_id: Some(id.get()),
            status: "preparing",
            stage: "queued".into(),
            size_bytes: None,
            error: None,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn clip_share_status(app: AppHandle, job_id: u64) -> Result<ShareState, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let runtime = app.state::<ShareRuntime>();
        let guard = runtime.0.lock().map_err(|e| e.to_string())?;
        let state = guard
            .as_ref()
            .and_then(|worker| worker.state(ExportJobId(job_id)))
            .ok_or("This share has expired. Prepare it again.")?;
        let (status, error) = match state.status {
            ExportStatus::Completed => ("ready", None),
            ExportStatus::Failed { reason } => ("failed", Some(reason)),
            ExportStatus::Cancelled => ("cancelled", None),
            _ => ("preparing", None),
        };
        let size = if status == "ready" {
            Some(
                state
                    .job
                    .destination_path
                    .metadata()
                    .map_err(|e| e.to_string())?
                    .len(),
            )
        } else {
            None
        };
        Ok(ShareState {
            job_id: Some(job_id),
            status,
            stage: state.progress.stage.as_str().into(),
            size_bytes: size,
            error,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn cancel_clip_share(runtime: State<'_, ShareRuntime>, job_id: u64) -> Result<(), String> {
    let guard = runtime.0.lock().map_err(|e| e.to_string())?;
    if let Some(worker) = guard.as_ref() {
        worker
            .cancel(ExportJobId(job_id))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

// A caller may drag a library original or a completed export issued by this process.
// Arbitrary frontend paths are never handed to the Shell.
fn share_path(app: &AppHandle, source: PathBuf, job_id: Option<u64>) -> Result<PathBuf, String> {
    if let Some(id) = job_id {
        let runtime = app.state::<ShareRuntime>();
        let guard = runtime.0.lock().map_err(|e| e.to_string())?;
        let state = guard
            .as_ref()
            .and_then(|worker| worker.state(ExportJobId(id)))
            .ok_or("This share has expired. Prepare it again.")?;
        if state.status != ExportStatus::Completed {
            return Err("Wait until the clip is ready to drag.".into());
        }
        if !state.job.destination_path.is_file() {
            return Err("The prepared file is missing. Prepare it again.".into());
        }
        Ok(state.job.destination_path)
    } else {
        crate::shell::library_action_path(&app.state::<crate::shell::ShellState>(), source)
    }
}

#[tauri::command]
pub async fn reveal_clip_share(
    app: AppHandle,
    path: PathBuf,
    job_id: Option<u64>,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let path = share_path(&app, path, job_id)?;
        tauri_plugin_opener::reveal_item_in_dir(path).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn drag_clip_share(
    app: AppHandle,
    path: PathBuf,
    job_id: Option<u64>,
) -> Result<(), String> {
    let validator = app.clone();
    let path = tauri::async_runtime::spawn_blocking(move || share_path(&validator, path, job_id))
        .await
        .map_err(|e| e.to_string())??;
    let (send, mut receive) = tauri::async_runtime::channel(1);
    app.run_on_main_thread(move || {
        let _ = send.try_send(crate::file_drag::drag_file(&path));
    })
    .map_err(|e| e.to_string())?;
    receive
        .recv()
        .await
        .ok_or_else(|| "The drag operation could not start.".to_string())?
}

fn target_bytes(mb: u32) -> Result<u64, String> {
    match mb {
        10 | 50 | 100 => Ok(u64::from(mb) * 1_000_000),
        _ => Err("Choose 10 MB, 50 MB, or 100 MB.".into()),
    }
}

fn make_job(source: &Path, destination: PathBuf, target: u64) -> Result<ExportJob, String> {
    let file = std::fs::File::open(source).map_err(|e| e.to_string())?;
    let size = file.metadata().map_err(|e| e.to_string())?.len();
    let reader = mp4::Mp4Reader::read_header(std::io::BufReader::new(file), size)
        .map_err(|e| format!("Could not read this clip for sharing: {e}"))?;
    let video = reader
        .tracks()
        .values()
        .find(|track| matches!(track.track_type(), Ok(mp4::TrackType::Video)))
        .ok_or("This clip has no video track.")?;
    let audio_count = reader
        .tracks()
        .values()
        .filter(|track| matches!(track.track_type(), Ok(mp4::TrackType::Audio)))
        .count() as u32;
    if audio_count > 8 {
        return Err("Sharing supports up to eight recorded audio tracks.".into());
    }
    let duration = video
        .duration()
        .as_micros()
        .try_into()
        .map_err(|_| "Clip duration is too long.")?;
    let source_video = VideoSource::new(
        u32::from(video.width()),
        u32::from(video.height()),
        video.frame_rate().round().clamp(1.0, 240.0) as u32,
    );
    let plan = share_plan(target, duration, source_video, audio_count > 0)?;
    let job = ExportJob::new(source, destination, plan);
    Ok(if audio_count > 0 {
        job.with_audio_mix(audio_count)
    } else {
        job
    })
}

fn share_plan(
    target: u64,
    duration_us: u64,
    source: VideoSource,
    audio: bool,
) -> Result<ExportPlan, String> {
    if source.width < 2 || source.height < 2 || source.fps == 0 || duration_us == 0 {
        return Err("This clip has invalid video dimensions or duration.".into());
    }
    // Decimal MB, 12% headroom for encoder variance/container overhead, one stereo mix.
    let request = PlannerRequest::new(TargetSize::Custom(target), duration_us, 128_000, source)
        .with_audio_track_count(u32::from(audio))
        .with_reserve_ratio(Ratio::percent(12))
        .with_video_bitrate_floor_bps(150_000);
    let initial =
        TargetBytePlanner::with_ladder([VideoRung::new(source.width, source.height, source.fps)])
            .plan(&request)
            .map_err(|e| format!("This clip needs a larger file size or a shorter trim: {e}"))?;
    let (height_cap, fps_cap) = match initial.video_bitrate_bps {
        0..=799_999 => (360, 30),
        800_000..=1_999_999 => (480, 30),
        2_000_000..=3_999_999 => (720, 30),
        4_000_000..=7_999_999 => (1080, 30),
        _ => (1080, 60),
    };
    let scale = (1920.0 / f64::from(source.width))
        .min(f64::from(height_cap) / f64::from(source.height))
        .min(1.0);
    let width = ((f64::from(source.width) * scale) as u32 / 2 * 2).max(2);
    let height = ((f64::from(source.height) * scale) as u32 / 2 * 2).max(2);
    TargetBytePlanner::with_ladder([VideoRung::new(width, height, source.fps.min(fps_cap))])
        .plan(&request)
        .map_err(|e| e.to_string())
}

fn prepare_cache(cache: &Path, reservation: u64) -> Result<(), String> {
    std::fs::create_dir_all(cache).map_err(|e| e.to_string())?;
    let mut count = 0;
    let mut bytes = 0u64;
    for entry in std::fs::read_dir(cache).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if !entry.file_type().map_err(|e| e.to_string())?.is_file()
            || !entry
                .file_name()
                .to_string_lossy()
                .starts_with("silk-share-")
        {
            continue;
        }
        let metadata = entry.metadata().map_err(|e| e.to_string())?;
        // Keep recently dragged files available while another app uploads them.
        if metadata
            .modified()
            .ok()
            .and_then(|time| time.elapsed().ok())
            .is_some_and(|age| age > Duration::from_secs(7 * 24 * 3600))
            && std::fs::remove_file(entry.path()).is_ok()
        {
            continue;
        }
        count += 1;
        bytes = bytes.saturating_add(metadata.len());
    }
    if count >= CACHE_LIMIT_FILES || bytes.saturating_add(reservation) > CACHE_LIMIT_BYTES {
        return Err(format!(
            "The share cache is full. Remove older shared copies in {} and try again.",
            cache.display()
        ));
    }
    Ok(())
}

fn discover_toolchain() -> Result<FfmpegToolchain, String> {
    if let (Some(ffmpeg), Some(ffprobe)) = (
        std::env::var_os("SILK_FFMPEG_PATH"),
        std::env::var_os("SILK_FFPROBE_PATH"),
    ) {
        return Ok(FfmpegToolchain::new(ffmpeg, ffprobe));
    }
    let mut folders = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            folders.push(parent.join("tools/ffmpeg/bin"));
            folders.push(parent.to_path_buf());
        }
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        let local = PathBuf::from(local);
        folders.push(local.join("Silk/tools/ffmpeg/bin"));
        if let Ok(packages) = std::fs::read_dir(local.join("Microsoft/WinGet/Packages")) {
            for package in packages.flatten().filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("Gyan.FFmpeg_")
            }) {
                if let Ok(builds) = std::fs::read_dir(package.path()) {
                    for build in builds.flatten().take(16) {
                        folders.push(build.path().join("bin"));
                    }
                }
            }
        }
    }
    if let Some(path) = std::env::var_os("PATH") {
        folders.extend(std::env::split_paths(&path));
    }
    folders.into_iter().find(|folder| folder.join("ffmpeg.exe").is_file() && folder.join("ffprobe.exe").is_file())
        .map(|folder| FfmpegToolchain::new(folder.join("ffmpeg.exe"), folder.join("ffprobe.exe")))
        .ok_or_else(|| "Size presets need Silk’s approved FFmpeg 9.0.1 tools. You can still drag the original clip.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_keeps_recent_drops_and_enforces_limits() {
        let cache = test_support::TempDir::new("share-cache").unwrap();
        for index in 0..CACHE_LIMIT_FILES {
            std::fs::write(
                cache.path().join(format!("silk-share-{index}.mp4")),
                b"clip",
            )
            .unwrap();
        }
        assert!(prepare_cache(cache.path(), 10_000_000).is_err());
        assert!(cache.path().join("silk-share-0.mp4").is_file());
        let expired = std::fs::File::options()
            .write(true)
            .open(cache.path().join("silk-share-0.mp4"))
            .unwrap();
        expired
            .set_times(
                std::fs::FileTimes::new()
                    .set_modified(SystemTime::now() - Duration::from_secs(8 * 24 * 3600)),
            )
            .unwrap();
        drop(expired);
        prepare_cache(cache.path(), 10_000_000).unwrap();
        assert!(!cache.path().join("silk-share-0.mp4").exists());
        assert!(cache.path().join("silk-share-1.mp4").is_file());
        assert!(prepare_cache(cache.path(), CACHE_LIMIT_BYTES).is_err());
    }

    #[test]
    fn share_budget_accounts_for_mixed_audio_and_preserves_aspect() {
        for mb in [10, 50, 100] {
            let target = target_bytes(mb).unwrap();
            let plan =
                share_plan(target, 90_000_000, VideoSource::new(3440, 1440, 120), true).unwrap();
            assert_eq!(plan.audio_track_count, 1);
            assert!(plan.rung.width <= 1920 && plan.rung.height <= 1080 && plan.rung.fps <= 60);
            assert_eq!(plan.rung.width % 2, 0);
            assert_eq!(plan.rung.height % 2, 0);
            assert!(
                (f64::from(plan.rung.width) / f64::from(plan.rung.height) - 3440.0 / 1440.0).abs()
                    < 0.02
            );
            assert!(plan.audio_bytes + plan.video_budget_bytes + plan.reserve_bytes <= target);
        }
    }

    #[test]
    fn share_presets_reject_invalid_requests_and_do_not_upscale() {
        assert!(target_bytes(11).is_err());
        assert!(share_plan(10_000_000, 0, VideoSource::new(1920, 1080, 60), true).is_err());
        assert!(share_plan(
            10_000_000,
            3_600_000_000,
            VideoSource::new(1920, 1080, 60),
            true
        )
        .is_err());
        let plan = share_plan(
            100_000_000,
            10_000_000,
            VideoSource::new(640, 360, 24),
            false,
        )
        .unwrap();
        assert_eq!(plan.rung, VideoRung::new(640, 360, 24));
        assert_eq!(plan.audio_bytes, 0);
    }

    #[test]
    #[ignore = "requires approved FFmpeg binaries and SILK_SHARE_TEST_CLIP"]
    fn native_share_exports_all_presets() {
        let source = PathBuf::from(std::env::var_os("SILK_SHARE_TEST_CLIP").expect("source clip"));
        let cache = std::env::temp_dir().join(format!("silk-share-test-{}", std::process::id()));
        std::fs::create_dir_all(&cache).unwrap();
        let exporter = FfmpegExporter::new(discover_toolchain().unwrap().verify().unwrap());
        for mb in [10, 50, 100] {
            let target = target_bytes(mb).unwrap();
            let destination = cache.join(format!("test-{mb}.mp4"));
            let job = make_job(&source, destination, target).unwrap();
            let result = exporter
                .export(&job, &clip_export::CancellationToken::new())
                .unwrap();
            assert!(result.size_bytes <= target);
            assert_eq!(result.audio_track_count, job.plan.audio_track_count);
            eprintln!(
                "{mb} MB: {} bytes, {}",
                result.size_bytes,
                result.destination_path.display()
            );
        }
    }
}
