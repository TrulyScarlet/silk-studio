#![cfg(windows)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use clip_export::{
    plan, ExportJob, ExportQueue, ExportStatus, PlannerRequest, TargetSize, VideoSource,
};
use encoder_ffmpeg::{ExportWorker, FfmpegExporter, FfmpegToolchain};

const SOURCE_DURATION_US: u64 = 2_000_000;
const SOURCE_WIDTH: u32 = 640;
const SOURCE_HEIGHT: u32 = 360;
const SOURCE_FPS: u32 = 30;
const AUDIO_BITRATE_BPS: u64 = 128_000;

struct TempDirectory(PathBuf);

impl TempDirectory {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("silk-real-export-{nonce}"));
        fs::create_dir_all(&path).expect("create temporary export directory");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn configured_toolchain() -> FfmpegToolchain {
    let ffmpeg = std::env::var_os("SILK_FFMPEG_PATH")
        .expect("SILK_FFMPEG_PATH must point to the approved ffmpeg.exe");
    let ffprobe = std::env::var_os("SILK_FFPROBE_PATH")
        .expect("SILK_FFPROBE_PATH must point to the approved ffprobe.exe");
    FfmpegToolchain::new(ffmpeg, ffprobe)
}

fn generate_source(toolchain: &FfmpegToolchain, source_path: &Path) {
    let status = Command::new(&toolchain.ffmpeg_path)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=640x360:rate=30",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=1000:sample_rate=48000",
            "-t",
            "2",
            "-map",
            "0:v:0",
            "-map",
            "1:a:0",
            "-c:v",
            "h264_mf",
            "-b:v",
            "2M",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-b:a",
            "128k",
            "-movflags",
            "+faststart",
            "-f",
            "mp4",
        ])
        .arg(source_path)
        .status()
        .expect("start source generation");
    assert!(status.success(), "source generation failed: {status}");
}

#[test]
#[ignore = "requires the approved Gyan FFmpeg pair and Windows Media Foundation"]
fn approved_toolchain_exports_a_generated_h264_aac_mp4() {
    let toolchain = configured_toolchain();
    let verified = toolchain.verify().expect("approved toolchain verification");
    let directory = TempDirectory::new();
    let source_path = directory.path().join("source.mp4");
    let destination_path = directory.path().join("export.mp4");
    generate_source(&toolchain, &source_path);

    let request = PlannerRequest::new(
        TargetSize::Medium,
        SOURCE_DURATION_US,
        AUDIO_BITRATE_BPS,
        VideoSource::new(SOURCE_WIDTH, SOURCE_HEIGHT, SOURCE_FPS),
    )
    .with_audio_track_count(1);
    let plan = plan(&request).expect("export plan");
    let job = ExportJob::new(&source_path, &destination_path, plan);
    let queue = std::sync::Arc::new(ExportQueue::new());
    let worker = ExportWorker::new(std::sync::Arc::clone(&queue), FfmpegExporter::new(verified))
        .expect("start export worker");
    let id = worker.submit(job).expect("submit export");

    let deadline = Instant::now() + Duration::from_secs(60);
    let final_state = loop {
        let state = worker.state(id).expect("export state");
        if state.status.is_terminal() {
            break state;
        }
        assert!(Instant::now() < deadline, "export did not finish in time");
        std::thread::sleep(Duration::from_millis(25));
    };

    assert_eq!(final_state.status, ExportStatus::Completed);
    assert!(source_path.is_file());
    assert!(destination_path.is_file());
    assert!(destination_path.metadata().expect("export metadata").len() > 0);
    assert!(!PathBuf::from(format!("{}.part", destination_path.display())).exists());
    assert!(!PathBuf::from(format!("{}.part.lock", destination_path.display())).exists());
}
