//! FFmpeg-based encoder/muxer boundaries for post-save export.
//!
//! This crate provides a deterministic argument boundary, a verified external
//! process adapter, and a staged exporter. It does not link FFmpeg libraries.
//! Release distribution remains subject to ADR 0017.

mod executor;
mod process;
mod toolchain;
mod worker;

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use clip_export::ExportJob;
use thiserror::Error;

pub use executor::{ExportError, ExportResult, FfmpegExporter};
pub use process::{
    ProcessOutput, ProcessRunner, ProcessRunnerError, SystemProcessRunner,
    MAX_CAPTURED_PROCESS_OUTPUT_BYTES,
};
pub use toolchain::{FfmpegToolchain, ToolchainError, VerifiedFfmpegToolchain};
pub use worker::{ExportWorker, ExportWorkerError};

/// Encoder selected for the first Windows export backend.
pub const H264_VIDEO_ENCODER: &str = "h264_mf";
/// Audio encoder selected for the first export backend.
pub const AAC_AUDIO_ENCODER: &str = "aac";
/// Container produced by the first share-oriented export preset.
pub const MP4_CONTAINER: &str = "mp4";

/// A direct FFmpeg invocation plus its atomic-publication target.
///
/// The arguments are kept as an `OsString` vector so a future worker can pass
/// them directly to `std::process::Command` without shell interpolation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FfmpegInvocation {
    pub executable: PathBuf,
    pub args: Vec<OsString>,
    pub staged_output: PathBuf,
    pub destination_path: PathBuf,
}

impl FfmpegInvocation {
    pub fn final_output(&self) -> &Path {
        &self.destination_path
    }
}

/// Errors found while constructing an invocation, before any backend work.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum InvocationError {
    #[error("FFmpeg executable path must not be empty")]
    EmptyExecutable,

    #[error("export source path must not be empty")]
    EmptySource,

    #[error("export destination path must not be empty")]
    EmptyDestination,

    #[error("export source and destination paths must differ")]
    SameSourceAndDestination,

    #[error("export source aliases the staged output path")]
    SourceAliasesStagedOutput,

    #[error("export destination must use the .mp4 extension: {path}")]
    DestinationMustBeMp4 { path: PathBuf },

    #[error("export plan must contain a positive video bitrate")]
    InvalidVideoBitrate,

    #[error("export plan must contain positive output dimensions and frame rate")]
    InvalidVideoRung,

    #[error("audio tracks require a positive per-track audio bitrate")]
    InvalidAudioBitrate,
}

/// Build the first share-oriented export command without starting a process.
///
/// The output is always MP4 and is written to a sibling `.part` path. The
/// eventual executor must validate the staged file, enforce the byte target,
/// and rename it to `destination_path` only after successful validation.
pub fn build_invocation(
    executable: impl Into<PathBuf>,
    job: &ExportJob,
) -> Result<FfmpegInvocation, InvocationError> {
    let executable = executable.into();
    if executable.as_os_str().is_empty() {
        return Err(InvocationError::EmptyExecutable);
    }
    if job.source_path.as_os_str().is_empty() {
        return Err(InvocationError::EmptySource);
    }
    if job.destination_path.as_os_str().is_empty() {
        return Err(InvocationError::EmptyDestination);
    }
    if job.source_path == job.destination_path {
        return Err(InvocationError::SameSourceAndDestination);
    }
    if !has_mp4_extension(&job.destination_path) {
        return Err(InvocationError::DestinationMustBeMp4 {
            path: job.destination_path.clone(),
        });
    }
    let staged_output = staged_output_path(&job.destination_path);
    if job.source_path == staged_output {
        return Err(InvocationError::SourceAliasesStagedOutput);
    }
    if job.plan.video_bitrate_bps == 0 {
        return Err(InvocationError::InvalidVideoBitrate);
    }
    if job.plan.rung.width == 0 || job.plan.rung.height == 0 || job.plan.rung.fps == 0 {
        return Err(InvocationError::InvalidVideoRung);
    }
    if job.plan.audio_track_count > 0 && job.plan.audio_bitrate_bps == 0 {
        return Err(InvocationError::InvalidAudioBitrate);
    }

    let mut args = Vec::new();
    push_arg(&mut args, "-hide_banner");
    push_arg(&mut args, "-nostdin");
    push_arg(&mut args, "-y");
    push_arg(&mut args, "-i");
    args.push(job.source_path.as_os_str().to_os_string());
    push_arg(&mut args, "-map");
    push_arg(&mut args, "0:v:0");
    for track in 0..job.plan.audio_track_count {
        push_arg(&mut args, "-map");
        push_arg(&mut args, format!("0:a:{track}?"));
    }
    push_arg(&mut args, "-c:v");
    push_arg(&mut args, H264_VIDEO_ENCODER);
    push_arg(&mut args, "-b:v");
    push_arg(&mut args, job.plan.video_bitrate_bps.to_string());
    push_arg(&mut args, "-s");
    push_arg(
        &mut args,
        format!("{}x{}", job.plan.rung.width, job.plan.rung.height),
    );
    push_arg(&mut args, "-r");
    push_arg(&mut args, job.plan.rung.fps.to_string());
    push_arg(&mut args, "-pix_fmt");
    push_arg(&mut args, "yuv420p");
    if job.plan.audio_track_count > 0 {
        push_arg(&mut args, "-c:a");
        push_arg(&mut args, AAC_AUDIO_ENCODER);
        for track in 0..job.plan.audio_track_count {
            push_arg(&mut args, format!("-b:a:{track}"));
            push_arg(&mut args, job.plan.audio_bitrate_bps.to_string());
        }
    }
    push_arg(&mut args, "-movflags");
    push_arg(&mut args, "+faststart");
    push_arg(&mut args, "-f");
    push_arg(&mut args, MP4_CONTAINER);
    args.push(staged_output.as_os_str().to_os_string());

    Ok(FfmpegInvocation {
        executable,
        args,
        staged_output,
        destination_path: job.destination_path.clone(),
    })
}

fn has_mp4_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case(MP4_CONTAINER))
}

fn staged_output_path(destination: &Path) -> PathBuf {
    let mut staged = destination.as_os_str().to_os_string();
    staged.push(".part");
    PathBuf::from(staged)
}

fn push_arg(args: &mut Vec<OsString>, value: impl Into<OsString>) {
    args.push(value.into());
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use clip_export::{plan, ExportJob, PlannerRequest, TargetSize, VideoSource};

    use super::*;

    fn job(audio_track_count: u32) -> ExportJob {
        let request = PlannerRequest::new(
            TargetSize::Medium,
            60_000_000,
            192_000,
            VideoSource::new(1_920, 1_080, 60),
        )
        .with_audio_track_count(audio_track_count);
        ExportJob::new(
            PathBuf::from("captures\\source.mp4"),
            PathBuf::from("exports\\share.mp4"),
            plan(&request).expect("plan"),
        )
    }

    fn strings(args: &[OsString]) -> Vec<String> {
        args.iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn builds_a_direct_staged_mp4_invocation() {
        let invocation = build_invocation("tools\\ffmpeg.exe", &job(1)).expect("invocation");

        assert_eq!(invocation.executable, PathBuf::from("tools\\ffmpeg.exe"));
        assert_eq!(
            invocation.staged_output,
            PathBuf::from("exports\\share.mp4.part")
        );
        assert_eq!(
            invocation.final_output(),
            PathBuf::from("exports\\share.mp4")
        );
        assert_eq!(
            strings(&invocation.args),
            vec![
                "-hide_banner",
                "-nostdin",
                "-y",
                "-i",
                "captures\\source.mp4",
                "-map",
                "0:v:0",
                "-map",
                "0:a:0?",
                "-c:v",
                "h264_mf",
                "-b:v",
                "6474666",
                "-s",
                "1920x1080",
                "-r",
                "60",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac",
                "-b:a:0",
                "192000",
                "-movflags",
                "+faststart",
                "-f",
                "mp4",
                "exports\\share.mp4.part",
            ]
        );
    }

    #[test]
    fn allocates_audio_arguments_for_each_selected_track() {
        let invocation = build_invocation("ffmpeg.exe", &job(2)).expect("invocation");
        let args = strings(&invocation.args);

        assert_eq!(args.iter().filter(|arg| arg.as_str() == "-map").count(), 3);
        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "-map" && pair[1] == "0:a:0?"));
        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "-map" && pair[1] == "0:a:1?"));
        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "-b:a:0" && pair[1] == "192000"));
        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "-b:a:1" && pair[1] == "192000"));
    }

    #[test]
    fn omits_audio_when_the_plan_has_no_selected_tracks() {
        let invocation = build_invocation("ffmpeg.exe", &job(0)).expect("invocation");
        let args = strings(&invocation.args);

        assert!(!args.iter().any(|arg| arg == "-c:a"));
        assert_eq!(args.iter().filter(|arg| arg.as_str() == "-map").count(), 1);
        assert!(!args.iter().any(|arg| arg.starts_with("0:a:")));
    }

    #[test]
    fn rejects_non_mp4_destination_and_same_path() {
        let mut non_mp4 = job(1);
        non_mp4.destination_path = PathBuf::from("exports\\share.webm");
        assert!(matches!(
            build_invocation("ffmpeg.exe", &non_mp4),
            Err(InvocationError::DestinationMustBeMp4 { .. })
        ));

        let mut same_path = job(1);
        same_path.destination_path = same_path.source_path.clone();
        assert!(matches!(
            build_invocation("ffmpeg.exe", &same_path),
            Err(InvocationError::SameSourceAndDestination)
        ));

        let mut aliases_stage = job(1);
        aliases_stage.source_path = PathBuf::from("exports\\share.mp4.part");
        assert!(matches!(
            build_invocation("ffmpeg.exe", &aliases_stage),
            Err(InvocationError::SourceAliasesStagedOutput)
        ));
    }

    #[test]
    fn rejects_empty_executable_path() {
        assert!(matches!(
            build_invocation(PathBuf::new(), &job(1)),
            Err(InvocationError::EmptyExecutable)
        ));
    }
}
