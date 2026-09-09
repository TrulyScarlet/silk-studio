use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};

use clip_export::{CancellationToken, ExportJob, ExportProgress, ExportStage};
use serde::Deserialize;
use thiserror::Error;

use crate::process::{ProcessOutput, ProcessRunner, ProcessRunnerError, SystemProcessRunner};
use crate::toolchain::VerifiedFfmpegToolchain;
use crate::{build_invocation, InvocationError};

const DURATION_TOLERANCE_US: u64 = 500_000;

/// Result of one successfully validated and published export.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportResult {
    pub destination_path: PathBuf,
    pub size_bytes: u64,
    pub duration_us: u64,
    pub audio_track_count: u32,
}

/// Errors from planning, process execution, validation, or publication.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ExportError {
    #[error(transparent)]
    Invocation(#[from] InvocationError),

    #[error("export was cancelled")]
    Cancelled,

    #[error("source clip is unavailable at {path}: {reason}")]
    SourceUnavailable { path: PathBuf, reason: String },

    #[error("export destination already exists: {path}")]
    DestinationExists { path: PathBuf },

    #[error("could not prepare staged output {path}: {reason}")]
    StageUnavailable { path: PathBuf, reason: String },

    #[error("FFmpeg process adapter failed during {stage}: {reason}")]
    ProcessAdapter { stage: &'static str, reason: String },

    #[error("FFmpeg failed during {stage} (exit code {exit_code:?}): {details}")]
    ProcessFailed {
        stage: &'static str,
        exit_code: Option<i32>,
        details: String,
    },

    #[error("staged output is unavailable at {path}: {reason}")]
    OutputUnavailable { path: PathBuf, reason: String },

    #[error("staged output is empty: {path}")]
    EmptyOutput { path: PathBuf },

    #[error("export output is {actual_bytes} bytes, over the {target_bytes}-byte target")]
    OutputTooLarge {
        actual_bytes: u64,
        target_bytes: u64,
    },

    #[error("FFprobe returned invalid media metadata: {reason}")]
    InvalidProbeOutput { reason: String },

    #[error("export media validation failed: {reason}")]
    MediaValidation { reason: String },

    #[error("could not publish export at {path}: {reason}")]
    PublishFailed { path: PathBuf, reason: String },
}

impl ExportError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Invocation(_) => "CLIP_EXPORT_INVALID_INVOCATION",
            Self::Cancelled => "CLIP_EXPORT_CANCELLED",
            Self::SourceUnavailable { .. } => "CLIP_EXPORT_SOURCE_UNAVAILABLE",
            Self::DestinationExists { .. } => "CLIP_EXPORT_DESTINATION_EXISTS",
            Self::StageUnavailable { .. } => "CLIP_EXPORT_STAGE_UNAVAILABLE",
            Self::ProcessAdapter { .. } => "CLIP_EXPORT_PROCESS_UNAVAILABLE",
            Self::ProcessFailed { .. } => "CLIP_EXPORT_PROCESS_FAILED",
            Self::OutputUnavailable { .. } | Self::EmptyOutput { .. } => {
                "CLIP_EXPORT_OUTPUT_UNAVAILABLE"
            }
            Self::OutputTooLarge { .. } => "CLIP_EXPORT_OUTPUT_TOO_LARGE",
            Self::InvalidProbeOutput { .. } | Self::MediaValidation { .. } => {
                "CLIP_EXPORT_MEDIA_INVALID"
            }
            Self::PublishFailed { .. } => "CLIP_EXPORT_PUBLISH_FAILED",
        }
    }
}

/// FFmpeg exporter that requires a previously verified toolchain.
pub struct FfmpegExporter<R = SystemProcessRunner> {
    toolchain: VerifiedFfmpegToolchain,
    runner: R,
}

impl FfmpegExporter<SystemProcessRunner> {
    pub fn new(toolchain: VerifiedFfmpegToolchain) -> Self {
        Self {
            toolchain,
            runner: SystemProcessRunner,
        }
    }
}

impl<R: ProcessRunner> FfmpegExporter<R> {
    pub fn with_runner(toolchain: VerifiedFfmpegToolchain, runner: R) -> Self {
        Self { toolchain, runner }
    }

    pub fn toolchain(&self) -> &VerifiedFfmpegToolchain {
        &self.toolchain
    }

    /// Execute one export on the caller's worker thread.
    ///
    /// This method never runs on capture or UI threads by itself. The caller
    /// must provide a bounded worker and may cancel through the supplied token.
    pub fn export(
        &self,
        job: &ExportJob,
        cancellation: &CancellationToken,
    ) -> Result<ExportResult, ExportError> {
        self.export_with_progress(job, cancellation, |_| {})
    }

    /// Execute one export and report coarse stage transitions to the owning
    /// worker. The callback must remain nonblocking and must not perform I/O.
    pub fn export_with_progress(
        &self,
        job: &ExportJob,
        cancellation: &CancellationToken,
        mut progress: impl FnMut(ExportProgress),
    ) -> Result<ExportResult, ExportError> {
        progress(ExportProgress::new(ExportStage::Preparing, 0));
        let invocation = build_invocation(self.toolchain.ffmpeg_path(), job)?;
        if cancellation.is_cancelled() {
            return Err(ExportError::Cancelled);
        }
        ensure_source_file(&job.source_path)?;
        if path_exists(&job.destination_path)? {
            return Err(ExportError::DestinationExists {
                path: job.destination_path.clone(),
            });
        }
        reject_source_stage_alias(&job.source_path, &invocation.staged_output)?;
        let stage_lock = prepare_stage(&invocation.staged_output)?;
        let stage_guard = StagedOutputGuard::new(invocation.staged_output.clone(), stage_lock);

        progress(ExportProgress::new(ExportStage::Encoding, 10));
        let ffmpeg_output = self
            .runner
            .run(&invocation.executable, &invocation.args, cancellation)
            .map_err(|error| process_adapter_error("encode", error))?;
        if ffmpeg_output.cancelled || cancellation.is_cancelled() {
            return Err(ExportError::Cancelled);
        }
        if !ffmpeg_output.success {
            return Err(process_failed("encode", &ffmpeg_output));
        }

        let encoded_bytes = output_size(&invocation.staged_output)?;
        if encoded_bytes == 0 {
            return Err(ExportError::EmptyOutput {
                path: invocation.staged_output.clone(),
            });
        }
        if encoded_bytes > job.plan.target_bytes {
            return Err(ExportError::OutputTooLarge {
                actual_bytes: encoded_bytes,
                target_bytes: job.plan.target_bytes,
            });
        }

        progress(ExportProgress::new(ExportStage::Probing, 55));
        let probe_args = build_probe_args(&invocation.staged_output);
        let probe_output = self
            .runner
            .run(self.toolchain.ffprobe_path(), &probe_args, cancellation)
            .map_err(|error| process_adapter_error("probe", error))?;
        if probe_output.cancelled || cancellation.is_cancelled() {
            return Err(ExportError::Cancelled);
        }
        if !probe_output.success || probe_output.truncated {
            return Err(process_failed("probe", &probe_output));
        }
        let report = parse_probe_report(&probe_output.stdout)?;
        validate_probe_report(job, &report)?;
        let duration_us = report.duration_us()?;
        let audio_track_count = report.audio_track_count();

        progress(ExportProgress::new(ExportStage::Probing, 65));
        let packet_probe_args = build_packet_probe_args(&invocation.staged_output);
        let packet_probe_output = self
            .runner
            .run(
                self.toolchain.ffprobe_path(),
                &packet_probe_args,
                cancellation,
            )
            .map_err(|error| process_adapter_error("packet probe", error))?;
        if packet_probe_output.cancelled || cancellation.is_cancelled() {
            return Err(ExportError::Cancelled);
        }
        if !packet_probe_output.success || packet_probe_output.truncated {
            return Err(process_failed("packet probe", &packet_probe_output));
        }
        let packet_report = parse_packet_report(&packet_probe_output.stdout)?;
        validate_packet_report(&packet_report)?;

        progress(ExportProgress::new(ExportStage::Decoding, 75));
        let decode_args = build_decode_args(&invocation.staged_output);
        let decode_output = self
            .runner
            .run(&invocation.executable, &decode_args, cancellation)
            .map_err(|error| process_adapter_error("decode", error))?;
        if decode_output.cancelled || cancellation.is_cancelled() {
            return Err(ExportError::Cancelled);
        }
        if !decode_output.success {
            return Err(process_failed("decode", &decode_output));
        }

        let final_bytes = output_size(&invocation.staged_output)?;
        if final_bytes > job.plan.target_bytes {
            return Err(ExportError::OutputTooLarge {
                actual_bytes: final_bytes,
                target_bytes: job.plan.target_bytes,
            });
        }
        if cancellation.is_cancelled() {
            return Err(ExportError::Cancelled);
        }
        progress(ExportProgress::new(ExportStage::Publishing, 98));
        publish_without_replacement(&invocation.staged_output, &invocation.destination_path)
            .map_err(|error| ExportError::PublishFailed {
                path: invocation.destination_path.clone(),
                reason: error.to_string(),
            })?;
        stage_guard.published();
        progress(ExportProgress::new(ExportStage::Publishing, 100));

        Ok(ExportResult {
            destination_path: invocation.destination_path,
            size_bytes: final_bytes,
            duration_us,
            audio_track_count,
        })
    }
}

struct StagedOutputGuard {
    path: PathBuf,
    lock_path: PathBuf,
    published: bool,
}

impl StagedOutputGuard {
    fn new(path: PathBuf, lock_path: PathBuf) -> Self {
        Self {
            path,
            lock_path,
            published: false,
        }
    }

    fn published(mut self) {
        self.published = true;
    }
}

impl Drop for StagedOutputGuard {
    fn drop(&mut self) {
        if !self.published {
            let _ = fs::remove_file(&self.path);
        }
        let _ = fs::remove_file(&self.lock_path);
    }
}

fn ensure_source_file(path: &Path) -> Result<(), ExportError> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(()),
        Ok(_) => Err(ExportError::SourceUnavailable {
            path: path.to_path_buf(),
            reason: "path is not a regular file".to_string(),
        }),
        Err(error) => Err(ExportError::SourceUnavailable {
            path: path.to_path_buf(),
            reason: error.to_string(),
        }),
    }
}

fn path_exists(path: &Path) -> Result<bool, ExportError> {
    match fs::metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(ExportError::StageUnavailable {
            path: path.to_path_buf(),
            reason: error.to_string(),
        }),
    }
}

fn reject_source_stage_alias(source: &Path, stage: &Path) -> Result<(), ExportError> {
    let source = fs::canonicalize(source).map_err(|error| ExportError::SourceUnavailable {
        path: source.to_path_buf(),
        reason: error.to_string(),
    })?;
    let stage =
        canonicalize_without_file(stage).map_err(|error| ExportError::StageUnavailable {
            path: stage.to_path_buf(),
            reason: error.to_string(),
        })?;
    if equivalent_paths(&source, &stage) {
        return Err(ExportError::Invocation(
            InvocationError::SourceAliasesStagedOutput,
        ));
    }
    Ok(())
}

fn canonicalize_without_file(path: &Path) -> std::io::Result<PathBuf> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path.file_name().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "staged path has no file name",
        )
    })?;
    Ok(fs::canonicalize(parent)?.join(file_name))
}

fn equivalent_paths(left: &Path, right: &Path) -> bool {
    #[cfg(windows)]
    {
        left.to_string_lossy()
            .eq_ignore_ascii_case(&right.to_string_lossy())
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

fn publish_without_replacement(stage: &Path, destination: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;

        use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH};

        let stage: Vec<u16> = stage
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let destination: Vec<u16> = destination
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let moved =
            unsafe { MoveFileExW(stage.as_ptr(), destination.as_ptr(), MOVEFILE_WRITE_THROUGH) };
        if moved == 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
    #[cfg(not(windows))]
    {
        // A hard link is atomic and fails if the destination was created after
        // the initial existence check. Roll back if removing the stage fails.
        fs::hard_link(stage, destination)?;
        if let Err(error) = fs::remove_file(stage) {
            let _ = fs::remove_file(destination);
            return Err(error);
        }
        Ok(())
    }
}

fn prepare_stage(path: &Path) -> Result<PathBuf, ExportError> {
    let lock_path = stage_lock_path(path);
    if let Err(error) = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock_path)
    {
        return Err(ExportError::StageUnavailable {
            path: path.to_path_buf(),
            reason: if error.kind() == std::io::ErrorKind::AlreadyExists {
                "staged output is already in use".to_string()
            } else {
                error.to_string()
            },
        });
    }
    match fs::remove_file(path) {
        Ok(()) => Ok(lock_path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(lock_path),
        Err(error) => {
            let _ = fs::remove_file(&lock_path);
            Err(ExportError::StageUnavailable {
                path: path.to_path_buf(),
                reason: error.to_string(),
            })
        }
    }
}

fn stage_lock_path(path: &Path) -> PathBuf {
    let mut lock = path.as_os_str().to_os_string();
    lock.push(".lock");
    PathBuf::from(lock)
}

fn output_size(path: &Path) -> Result<u64, ExportError> {
    let metadata = fs::metadata(path).map_err(|error| ExportError::OutputUnavailable {
        path: path.to_path_buf(),
        reason: error.to_string(),
    })?;
    if !metadata.is_file() {
        return Err(ExportError::OutputUnavailable {
            path: path.to_path_buf(),
            reason: "path is not a regular file".to_string(),
        });
    }
    Ok(metadata.len())
}

fn build_probe_args(path: &Path) -> Vec<OsString> {
    let mut args = vec![
        OsString::from("-hide_banner"),
        OsString::from("-v"),
        OsString::from("error"),
        OsString::from("-show_streams"),
        OsString::from("-show_format"),
        OsString::from("-of"),
        OsString::from("json"),
    ];
    args.push(path.as_os_str().to_os_string());
    args
}

fn build_packet_probe_args(path: &Path) -> Vec<OsString> {
    let mut args = vec![
        OsString::from("-hide_banner"),
        OsString::from("-v"),
        OsString::from("error"),
        OsString::from("-show_packets"),
        OsString::from("-show_entries"),
        OsString::from("packet=stream_index,dts_time"),
        OsString::from("-of"),
        OsString::from("csv=p=0"),
    ];
    args.push(path.as_os_str().to_os_string());
    args
}

fn build_decode_args(path: &Path) -> Vec<OsString> {
    vec![
        OsString::from("-hide_banner"),
        OsString::from("-nostdin"),
        OsString::from("-v"),
        OsString::from("error"),
        OsString::from("-xerror"),
        OsString::from("-i"),
        path.as_os_str().to_os_string(),
        OsString::from("-map"),
        OsString::from("0"),
        OsString::from("-f"),
        OsString::from("null"),
        OsString::from("-"),
    ]
}

fn process_adapter_error(stage: &'static str, error: ProcessRunnerError) -> ExportError {
    ExportError::ProcessAdapter {
        stage,
        reason: error.to_string(),
    }
}

fn process_failed(stage: &'static str, output: &ProcessOutput) -> ExportError {
    let details = match (output.stderr.trim(), output.stdout.trim()) {
        ("", "") => "no diagnostic output".to_string(),
        (stderr, "") => stderr.to_string(),
        ("", stdout) => stdout.to_string(),
        (stderr, stdout) => format!("{stderr}\n{stdout}"),
    };
    let details = if output.truncated {
        format!("{details} (output truncated)")
    } else {
        details
    };
    ExportError::ProcessFailed {
        stage,
        exit_code: output.exit_code,
        details,
    }
}

#[derive(Debug, Deserialize)]
struct ProbeReport {
    #[serde(default)]
    streams: Vec<ProbeStream>,
    format: Option<ProbeFormat>,
}

#[derive(Debug, Deserialize)]
struct ProbeStream {
    codec_type: Option<String>,
    codec_name: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    r_frame_rate: Option<String>,
    sample_rate: Option<String>,
    channels: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct ProbeFormat {
    format_name: Option<String>,
    duration: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PacketReport {
    packets: Vec<ProbePacket>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProbePacket {
    stream_index: u32,
    dts_us: i128,
}

impl ProbeReport {
    fn audio_track_count(&self) -> u32 {
        self.streams
            .iter()
            .filter(|stream| stream.codec_type.as_deref() == Some("audio"))
            .count() as u32
    }

    fn duration_us(&self) -> Result<u64, ExportError> {
        let duration = self
            .format
            .as_ref()
            .and_then(|format| format.duration.as_deref())
            .ok_or_else(|| ExportError::InvalidProbeOutput {
                reason: "format duration is missing".to_string(),
            })?;
        parse_seconds_to_us(duration).ok_or_else(|| ExportError::InvalidProbeOutput {
            reason: format!("format duration is invalid: {duration}"),
        })
    }
}

fn parse_probe_report(output: &str) -> Result<ProbeReport, ExportError> {
    serde_json::from_str(output).map_err(|error| ExportError::InvalidProbeOutput {
        reason: error.to_string(),
    })
}

fn validate_probe_report(job: &ExportJob, report: &ProbeReport) -> Result<(), ExportError> {
    let format_name = report
        .format
        .as_ref()
        .and_then(|format| format.format_name.as_deref())
        .ok_or_else(|| ExportError::InvalidProbeOutput {
            reason: "format name is missing".to_string(),
        })?;
    if !format_name.split(',').any(|name| name == "mp4") {
        return Err(ExportError::MediaValidation {
            reason: format!("output format is not MP4: {format_name}"),
        });
    }

    let videos: Vec<&ProbeStream> = report
        .streams
        .iter()
        .filter(|stream| stream.codec_type.as_deref() == Some("video"))
        .collect();
    if videos.len() != 1 {
        return Err(ExportError::MediaValidation {
            reason: format!("expected one video stream, found {}", videos.len()),
        });
    }
    let video = videos[0];
    if video.codec_name.as_deref() != Some("h264") {
        return Err(ExportError::MediaValidation {
            reason: format!(
                "expected H.264 video, found {}",
                video.codec_name.as_deref().unwrap_or("missing")
            ),
        });
    }
    if video.width != Some(job.plan.rung.width) || video.height != Some(job.plan.rung.height) {
        return Err(ExportError::MediaValidation {
            reason: format!(
                "expected video dimensions {}x{}, found {}x{}",
                job.plan.rung.width,
                job.plan.rung.height,
                video.width.unwrap_or_default(),
                video.height.unwrap_or_default()
            ),
        });
    }
    let frame_rate = video
        .r_frame_rate
        .as_deref()
        .and_then(parse_frame_rate)
        .ok_or_else(|| ExportError::InvalidProbeOutput {
            reason: "video frame rate is missing or invalid".to_string(),
        })?;
    if frame_rate != job.plan.rung.fps {
        return Err(ExportError::MediaValidation {
            reason: format!(
                "expected {} FPS video, found {frame_rate} FPS",
                job.plan.rung.fps
            ),
        });
    }

    let audios: Vec<&ProbeStream> = report
        .streams
        .iter()
        .filter(|stream| stream.codec_type.as_deref() == Some("audio"))
        .collect();
    if audios.len() != job.plan.audio_track_count as usize {
        return Err(ExportError::MediaValidation {
            reason: format!(
                "expected {} audio tracks, found {}",
                job.plan.audio_track_count,
                audios.len()
            ),
        });
    }
    if let Some(invalid) = audios
        .iter()
        .find(|stream| stream.codec_name.as_deref() != Some("aac"))
    {
        return Err(ExportError::MediaValidation {
            reason: format!(
                "expected AAC audio, found {}",
                invalid.codec_name.as_deref().unwrap_or("missing")
            ),
        });
    }
    if audios.iter().any(|stream| {
        stream
            .sample_rate
            .as_deref()
            .and_then(|rate| rate.parse::<u32>().ok())
            .is_none_or(|rate| rate == 0)
            || stream.channels.is_none_or(|channels| channels == 0)
    }) {
        return Err(ExportError::MediaValidation {
            reason: "audio stream has missing or invalid sample-rate/channel metadata".to_string(),
        });
    }

    let duration_us = report.duration_us()?;
    let difference = duration_us.abs_diff(job.plan.duration_us);
    if duration_us == 0 || difference > DURATION_TOLERANCE_US {
        return Err(ExportError::MediaValidation {
            reason: format!(
                "expected duration near {} us, found {duration_us} us",
                job.plan.duration_us
            ),
        });
    }
    Ok(())
}

fn parse_packet_report(output: &str) -> Result<PacketReport, ExportError> {
    let mut packets = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split(',');
        let stream_index_str = parts
            .next()
            .ok_or_else(|| ExportError::InvalidProbeOutput {
                reason: format!("malformed packet line: {line}"),
            })?;
        let dts_time_str = parts
            .next()
            .ok_or_else(|| ExportError::InvalidProbeOutput {
                reason: format!("malformed packet line: {line}"),
            })?;
        let stream_index =
            stream_index_str
                .parse::<u32>()
                .map_err(|_| ExportError::InvalidProbeOutput {
                    reason: format!("packet stream index is invalid: {stream_index_str}"),
                })?;
        let dts_us = parse_signed_seconds_to_us(dts_time_str).ok_or_else(|| {
            ExportError::InvalidProbeOutput {
                reason: format!("packet DTS is invalid: {dts_time_str}"),
            }
        })?;
        packets.push(ProbePacket {
            stream_index,
            dts_us,
        });
    }
    Ok(PacketReport { packets })
}

fn validate_packet_report(report: &PacketReport) -> Result<(), ExportError> {
    let mut previous_by_stream = BTreeMap::<u32, i128>::new();
    let mut timestamped_packets = 0_usize;
    for packet in &report.packets {
        let stream_index = packet.stream_index;
        let dts_us = packet.dts_us;
        // Codec priming can legally place the first audio DTS before zero;
        // ordering, not zero-origin, is the invariant for encoded output.
        if previous_by_stream
            .insert(stream_index, dts_us)
            .is_some_and(|previous| dts_us < previous)
        {
            return Err(ExportError::MediaValidation {
                reason: format!("packet DTS is not monotonic on stream {stream_index}"),
            });
        }
        timestamped_packets += 1;
    }
    if timestamped_packets == 0 {
        return Err(ExportError::MediaValidation {
            reason: "packet probe returned no decode timestamps".to_string(),
        });
    }
    Ok(())
}

fn parse_frame_rate(value: &str) -> Option<u32> {
    let (numerator, denominator) = value.split_once('/')?;
    let numerator = numerator.parse::<u64>().ok()?;
    let denominator = denominator.parse::<u64>().ok()?;
    if denominator == 0 || numerator % denominator != 0 {
        return None;
    }
    u32::try_from(numerator / denominator).ok()
}

fn parse_seconds_to_us(value: &str) -> Option<u64> {
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    let whole = whole.parse::<u64>().ok()?;
    let mut micros = fraction
        .as_bytes()
        .iter()
        .copied()
        .take(6)
        .collect::<Vec<_>>();
    while micros.len() < 6 {
        micros.push(b'0');
    }
    if micros.iter().any(|byte| !byte.is_ascii_digit()) {
        return None;
    }
    let fractional = std::str::from_utf8(&micros).ok()?.parse::<u64>().ok()?;
    whole.checked_mul(1_000_000)?.checked_add(fractional)
}

fn parse_signed_seconds_to_us(value: &str) -> Option<i128> {
    let (negative, unsigned) = match value.strip_prefix('-') {
        Some(unsigned) => (true, unsigned),
        None => (false, value.strip_prefix('+').unwrap_or(value)),
    };
    let micros = i128::from(parse_seconds_to_us(unsigned)?);
    Some(if negative { -micros } else { micros })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};
    use std::time::{SystemTime, UNIX_EPOCH};

    use clip_export::{plan, ExportJob, PlannerRequest, TargetSize, VideoSource};

    use super::*;

    #[derive(Debug)]
    struct FakeRunner {
        probe_output: String,
        encoded_bytes: Vec<u8>,
        calls: Arc<Mutex<Vec<Vec<OsString>>>>,
    }

    impl FakeRunner {
        fn valid(probe_output: String) -> Self {
            Self {
                probe_output,
                encoded_bytes: b"fake-mp4".to_vec(),
                calls: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    impl ProcessRunner for FakeRunner {
        fn run(
            &self,
            executable: &Path,
            args: &[OsString],
            cancellation: &CancellationToken,
        ) -> Result<ProcessOutput, ProcessRunnerError> {
            self.calls.lock().expect("calls").push(args.to_vec());
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
                    self.probe_output.clone()
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
            if args.iter().any(|arg| arg == "null") {
                return Ok(success_output());
            }
            let output_path = args.last().expect("output path");
            let path = PathBuf::from(output_path);
            if let Some(parent) = path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            fs::write(&path, &self.encoded_bytes).expect("fake output");
            Ok(success_output())
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

    fn verified_toolchain() -> VerifiedFfmpegToolchain {
        VerifiedFfmpegToolchain::for_tests(
            PathBuf::from("ffmpeg.exe"),
            PathBuf::from("ffprobe.exe"),
            "ffmpeg version test".to_string(),
            "ffprobe version test".to_string(),
            "test".to_string(),
        )
    }

    fn temp_directory() -> TempDirectory {
        TempDirectory::new()
    }

    struct TempDirectory(PathBuf);

    impl TempDirectory {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos();
            let path = std::env::temp_dir().join(format!("silk-ffmpeg-test-{nonce}"));
            fs::create_dir_all(&path).expect("temp directory");
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

    #[test]
    fn exports_validates_then_publishes_and_removes_stage() {
        let directory = temp_directory();
        let job = job(directory.path());
        fs::write(&job.source_path, b"source").expect("source");
        let runner = FakeRunner::valid(valid_probe());
        let exporter = FfmpegExporter::with_runner(verified_toolchain(), runner);

        let result = exporter
            .export(&job, &CancellationToken::new())
            .expect("export");

        assert_eq!(result.destination_path, job.destination_path);
        assert_eq!(result.size_bytes, b"fake-mp4".len() as u64);
        assert_eq!(result.duration_us, job.plan.duration_us);
        assert_eq!(result.audio_track_count, 1);
        assert!(job.destination_path.is_file());
        assert!(!PathBuf::from(format!("{}.part", job.destination_path.display())).exists());
        assert!(!PathBuf::from(format!("{}.part.lock", job.destination_path.display())).exists());
        assert_eq!(
            fs::read(&job.source_path).expect("source remains"),
            b"source"
        );
    }

    #[test]
    fn cancellation_happens_before_process_start() {
        let directory = temp_directory();
        let job = job(directory.path());
        fs::write(&job.source_path, b"source").expect("source");
        let runner = FakeRunner::valid(valid_probe());
        let calls = runner.calls.clone();
        let exporter = FfmpegExporter::with_runner(verified_toolchain(), runner);
        let cancellation = CancellationToken::new();
        cancellation.cancel();

        assert_eq!(
            exporter.export(&job, &cancellation),
            Err(ExportError::Cancelled)
        );
        assert!(calls.lock().expect("calls").is_empty());
        assert!(!job.destination_path.exists());
    }

    #[test]
    fn probe_invocations_use_ffprobe_compatible_arguments() {
        let path = Path::new("export.mp4");
        for args in [build_probe_args(path), build_packet_probe_args(path)] {
            assert!(args
                .iter()
                .any(|arg| arg == "-show_streams" || arg == "-show_packets"));
            assert!(!args.iter().any(|arg| arg == "-nostdin"));
        }
    }

    #[test]
    fn packet_probe_invocation_uses_compact_csv_arguments() {
        let path = Path::new("export.mp4");
        let args = build_packet_probe_args(path);
        let expected = vec![
            OsString::from("-hide_banner"),
            OsString::from("-v"),
            OsString::from("error"),
            OsString::from("-show_packets"),
            OsString::from("-show_entries"),
            OsString::from("packet=stream_index,dts_time"),
            OsString::from("-of"),
            OsString::from("csv=p=0"),
            OsString::from("export.mp4"),
        ];
        assert_eq!(args, expected);
    }

    #[test]
    fn parses_valid_multi_stream_compact_packet_report() {
        let compact = "0,0.000000\n1,-0.021333,Skip Samples,1024,0,0,0\n0,0.016667\n1,0.000000\n2,0.000000\n3,0.000000\n";
        let report = parse_packet_report(compact).expect("valid compact probe report");
        assert_eq!(report.packets.len(), 6);
        assert_eq!(
            report.packets[0],
            ProbePacket {
                stream_index: 0,
                dts_us: 0
            }
        );
        assert_eq!(
            report.packets[1],
            ProbePacket {
                stream_index: 1,
                dts_us: -21_333
            }
        );
        assert_eq!(
            report.packets[2],
            ProbePacket {
                stream_index: 0,
                dts_us: 16_667
            }
        );
        assert_eq!(
            report.packets[3],
            ProbePacket {
                stream_index: 1,
                dts_us: 0
            }
        );
        assert_eq!(
            report.packets[4],
            ProbePacket {
                stream_index: 2,
                dts_us: 0
            }
        );
        assert_eq!(
            report.packets[5],
            ProbePacket {
                stream_index: 3,
                dts_us: 0
            }
        );
        validate_packet_report(&report).expect("valid report passes validation");
    }

    #[test]
    fn rejects_malformed_and_missing_packet_data() {
        let malformed_cases = [
            ("missing dts", "0\n"),
            ("missing stream index", ",0.000000\n"),
            ("invalid stream index", "video,0.000000\n"),
            ("negative stream index", "-1,0.000000\n"),
            ("missing dts value", "0,\n"),
            ("non-numeric dts", "0,invalid\n"),
            ("not-a-number dts", "0,NaN\n"),
            ("infinity dts", "0,inf\n"),
            ("negative infinity dts", "0,-inf\n"),
            (
                "json output",
                r#"{"packets":[{"stream_index":0,"dts_time":"0.000000"}]}"#,
            ),
        ];

        for (desc, input) in malformed_cases {
            assert!(
                matches!(
                    parse_packet_report(input),
                    Err(ExportError::InvalidProbeOutput { .. })
                ),
                "case '{desc}' should be rejected as InvalidProbeOutput"
            );
        }
    }

    #[test]
    fn packet_validation_rejects_non_monotonic_timestamps() {
        let report = PacketReport {
            packets: vec![
                ProbePacket {
                    stream_index: 0,
                    dts_us: 100_000,
                },
                ProbePacket {
                    stream_index: 0,
                    dts_us: 50_000,
                },
            ],
        };
        assert!(matches!(
            validate_packet_report(&report),
            Err(ExportError::MediaValidation { reason }) if reason.contains("not monotonic")
        ));
    }

    #[test]
    fn invalid_probe_output_does_not_publish() {
        let directory = temp_directory();
        let job = job(directory.path());
        fs::write(&job.source_path, b"source").expect("source");
        let runner = FakeRunner::valid(
            r#"{"streams":[],"format":{"format_name":"mp4","duration":"60"}}"#.to_string(),
        );
        let exporter = FfmpegExporter::with_runner(verified_toolchain(), runner);

        assert!(matches!(
            exporter.export(&job, &CancellationToken::new()),
            Err(ExportError::MediaValidation { .. })
        ));
        assert!(!job.destination_path.exists());
        assert!(!PathBuf::from(format!("{}.part", job.destination_path.display())).exists());
    }

    #[test]
    fn packet_validation_allows_negative_codec_priming_dts() {
        let report = PacketReport {
            packets: vec![
                ProbePacket {
                    stream_index: 1,
                    dts_us: -21_333,
                },
                ProbePacket {
                    stream_index: 1,
                    dts_us: 0,
                },
            ],
        };

        validate_packet_report(&report).expect("codec priming DTS is valid");
    }

    #[test]
    fn packet_validation_still_rejects_non_monotonic_negative_dts() {
        let report = PacketReport {
            packets: vec![
                ProbePacket {
                    stream_index: 1,
                    dts_us: -10_000,
                },
                ProbePacket {
                    stream_index: 1,
                    dts_us: -20_000,
                },
            ],
        };

        assert!(matches!(
            validate_packet_report(&report),
            Err(ExportError::MediaValidation { reason })
                if reason.contains("not monotonic")
        ));
    }

    #[test]
    fn packet_probe_worst_case_size_budget_is_well_under_process_cap() {
        // Worst-case supported bounds: 120-second replay with 1 video track + 4 audio tracks.
        // 120s @ 60fps = 7,200 video packets.
        // 4 audio tracks @ 48kHz AAC (1024 samples/packet = 46.875 pkts/s) = 4 * 120 * ~47 = 22,560 audio packets.
        // Total packets ≈ 29,760 packets.
        let mut report_str = String::with_capacity(512 * 1024);
        let video_packets = 120 * 60; // 7,200
        let audio_tracks = 4;
        let audio_packets_per_track = 120 * 47; // 5,640 each
        let total_packets = video_packets + (audio_tracks * audio_packets_per_track);

        for i in 0..video_packets {
            let seconds = i as f64 / 60.0;
            use std::fmt::Write;
            let _ = writeln!(report_str, "0,{seconds:.6}");
        }
        for track in 1..=audio_tracks {
            for i in 0..audio_packets_per_track {
                let seconds = i as f64 * (1024.0 / 48000.0);
                use std::fmt::Write;
                let _ = writeln!(report_str, "{track},{seconds:.6}");
            }
        }

        assert!(
            report_str.len() < crate::process::MAX_CAPTURED_PROCESS_OUTPUT_BYTES,
            "realistic 120s 4-track output ({} bytes) must fit within {} byte process limit",
            report_str.len(),
            crate::process::MAX_CAPTURED_PROCESS_OUTPUT_BYTES
        );

        // Expect size to be comfortably under 512 KiB (half the 1 MiB cap)
        assert!(
            report_str.len() < 512 * 1024,
            "compact probe output ({} bytes) should be < 512 KiB",
            report_str.len()
        );

        let report = parse_packet_report(&report_str).expect("worst case compact parse");
        assert_eq!(report.packets.len(), total_packets);
        validate_packet_report(&report).expect("worst case packet validation");
    }

    #[test]
    fn output_over_target_is_rejected_and_cleaned() {
        let directory = temp_directory();
        let mut job = job(directory.path());
        job.plan.target_bytes = 4;
        fs::write(&job.source_path, b"source").expect("source");
        let mut runner = FakeRunner::valid(valid_probe());
        runner.encoded_bytes = b"too-large".to_vec();
        let exporter = FfmpegExporter::with_runner(verified_toolchain(), runner);

        assert_eq!(
            exporter.export(&job, &CancellationToken::new()),
            Err(ExportError::OutputTooLarge {
                actual_bytes: 9,
                target_bytes: 4,
            })
        );
        assert!(!job.destination_path.exists());
    }

    #[test]
    fn rejects_source_aliasing_stage_through_a_normalized_path() {
        let directory = temp_directory();
        let mut job = job(directory.path());
        job.source_path = directory.path().join(".").join("export.mp4.part");
        fs::write(directory.path().join("export.mp4.part"), b"source").expect("source");
        let exporter =
            FfmpegExporter::with_runner(verified_toolchain(), FakeRunner::valid(valid_probe()));

        assert_eq!(
            exporter.export(&job, &CancellationToken::new()),
            Err(ExportError::Invocation(
                InvocationError::SourceAliasesStagedOutput
            ))
        );
        assert!(directory.path().join("export.mp4.part").exists());
    }

    #[test]
    fn publication_does_not_replace_a_destination_created_after_the_check() {
        let directory = temp_directory();
        let stage = directory.path().join("export.mp4.part");
        let destination = directory.path().join("export.mp4");
        fs::write(&stage, b"new").expect("stage");
        fs::write(&destination, b"old").expect("destination");

        assert!(publish_without_replacement(&stage, &destination).is_err());
        assert_eq!(fs::read(&destination).expect("destination remains"), b"old");
        assert!(stage.exists());
    }

    #[test]
    fn staged_lock_prevents_concurrent_writers_and_is_released() {
        let directory = temp_directory();
        let stage = directory.path().join("export.mp4.part");
        let lock = prepare_stage(&stage).expect("first stage lock");

        assert!(lock.exists());
        assert!(matches!(
            prepare_stage(&stage),
            Err(ExportError::StageUnavailable { .. })
        ));

        drop(StagedOutputGuard::new(stage.clone(), lock.clone()));
        assert!(!lock.exists());
        assert!(prepare_stage(&stage).is_ok());
        let second_lock = stage_lock_path(&stage);
        drop(StagedOutputGuard::new(stage, second_lock.clone()));
        assert!(!second_lock.exists());
    }
}
