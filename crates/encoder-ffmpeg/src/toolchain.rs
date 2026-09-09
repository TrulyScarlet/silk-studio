use std::ffi::OsString;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use clip_export::CancellationToken;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::process::{ProcessOutput, ProcessRunner, ProcessRunnerError, SystemProcessRunner};
use crate::{AAC_AUDIO_ENCODER, H264_VIDEO_ENCODER};

const FORBIDDEN_BUILD_FLAGS: [&str; 1] = ["--enable-nonfree"];
const APPROVED_FFMPEG_SHA256: &str =
    "57C56E369D5B4873B4D93FC1A1D833CB7CD8BC9325C14B05C34CE60B22842D8A";
const APPROVED_FFPROBE_SHA256: &str =
    "AFE05347CAAABE479B3C4EAE71992B6EC1E11C57266A1D665DEB0F9FE9847208";

/// Paths for one externally supplied FFmpeg tool pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FfmpegToolchain {
    pub ffmpeg_path: PathBuf,
    pub ffprobe_path: PathBuf,
}

impl FfmpegToolchain {
    pub fn new(ffmpeg_path: impl Into<PathBuf>, ffprobe_path: impl Into<PathBuf>) -> Self {
        Self {
            ffmpeg_path: ffmpeg_path.into(),
            ffprobe_path: ffprobe_path.into(),
        }
    }

    pub fn verify(&self) -> Result<VerifiedFfmpegToolchain, ToolchainError> {
        if self.ffmpeg_path.as_os_str().is_empty() {
            return Err(ToolchainError::EmptyPath { tool: "ffmpeg" });
        }
        if self.ffprobe_path.as_os_str().is_empty() {
            return Err(ToolchainError::EmptyPath { tool: "ffprobe" });
        }
        let ffmpeg_sha256 =
            verify_binary_hash("ffmpeg", &self.ffmpeg_path, APPROVED_FFMPEG_SHA256)?;
        let ffprobe_sha256 =
            verify_binary_hash("ffprobe", &self.ffprobe_path, APPROVED_FFPROBE_SHA256)?;
        let mut verified = self.verify_with_runner(&SystemProcessRunner)?;
        verified.ffmpeg_sha256 = ffmpeg_sha256;
        verified.ffprobe_sha256 = ffprobe_sha256;
        Ok(verified)
    }

    #[cfg(test)]
    pub(crate) fn verify_with<R: ProcessRunner>(
        &self,
        runner: &R,
    ) -> Result<VerifiedFfmpegToolchain, ToolchainError> {
        self.verify_with_runner(runner)
    }

    fn verify_with_runner<R: ProcessRunner>(
        &self,
        runner: &R,
    ) -> Result<VerifiedFfmpegToolchain, ToolchainError> {
        if self.ffmpeg_path.as_os_str().is_empty() {
            return Err(ToolchainError::EmptyPath { tool: "ffmpeg" });
        }
        if self.ffprobe_path.as_os_str().is_empty() {
            return Err(ToolchainError::EmptyPath { tool: "ffprobe" });
        }

        let ffmpeg_version = checked_command(runner, "ffmpeg", &self.ffmpeg_path, &["-version"])?;
        let ffmpeg_version_line = version_line("ffmpeg", &ffmpeg_version)?;
        let ffprobe_version =
            checked_command(runner, "ffprobe", &self.ffprobe_path, &["-version"])?;
        let ffprobe_version_line = version_line("ffprobe", &ffprobe_version)?;
        let ffprobe_build_configuration = checked_command(
            runner,
            "ffprobe",
            &self.ffprobe_path,
            &["-hide_banner", "-buildconf"],
        )?;
        reject_forbidden_build_flags(
            &ffprobe_build_configuration.stdout,
            &ffprobe_build_configuration.stderr,
        )?;
        let build_configuration = checked_command(
            runner,
            "ffmpeg",
            &self.ffmpeg_path,
            &["-hide_banner", "-buildconf"],
        )?;
        reject_forbidden_build_flags(&build_configuration.stdout, &build_configuration.stderr)?;

        let formats = checked_command(
            runner,
            "ffmpeg",
            &self.ffmpeg_path,
            &["-hide_banner", "-formats"],
        )?;
        require_entry("format", "mp4", 'E', &formats)?;

        let decoders = checked_command(
            runner,
            "ffmpeg",
            &self.ffmpeg_path,
            &["-hide_banner", "-decoders"],
        )?;
        require_entry("decoder", "h264", 'V', &decoders)?;
        require_entry("decoder", "aac", 'A', &decoders)?;

        let encoders = checked_command(
            runner,
            "ffmpeg",
            &self.ffmpeg_path,
            &["-hide_banner", "-encoders"],
        )?;
        require_entry("encoder", H264_VIDEO_ENCODER, 'V', &encoders)?;
        require_entry("encoder", AAC_AUDIO_ENCODER, 'A', &encoders)?;

        let muxers = checked_command(
            runner,
            "ffmpeg",
            &self.ffmpeg_path,
            &["-hide_banner", "-muxers"],
        )?;
        require_entry("muxer", "mp4", 'E', &muxers)?;
        require_entry("muxer", "null", 'E', &muxers)?;

        Ok(VerifiedFfmpegToolchain {
            ffmpeg_path: self.ffmpeg_path.clone(),
            ffprobe_path: self.ffprobe_path.clone(),
            ffmpeg_version: ffmpeg_version_line,
            ffprobe_version: ffprobe_version_line,
            build_configuration: combine_output(&build_configuration),
            ffprobe_build_configuration: combine_output(&ffprobe_build_configuration),
            ffmpeg_sha256: String::new(),
            ffprobe_sha256: String::new(),
        })
    }
}

/// A toolchain that passed all configured license and capability checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedFfmpegToolchain {
    ffmpeg_path: PathBuf,
    ffprobe_path: PathBuf,
    ffmpeg_version: String,
    ffprobe_version: String,
    build_configuration: String,
    ffprobe_build_configuration: String,
    ffmpeg_sha256: String,
    ffprobe_sha256: String,
}

impl VerifiedFfmpegToolchain {
    pub fn ffmpeg_path(&self) -> &Path {
        &self.ffmpeg_path
    }

    pub fn ffprobe_path(&self) -> &Path {
        &self.ffprobe_path
    }

    pub fn ffmpeg_version(&self) -> &str {
        &self.ffmpeg_version
    }

    pub fn ffprobe_version(&self) -> &str {
        &self.ffprobe_version
    }

    pub fn build_configuration(&self) -> &str {
        &self.build_configuration
    }

    pub fn ffprobe_build_configuration(&self) -> &str {
        &self.ffprobe_build_configuration
    }

    pub fn ffmpeg_sha256(&self) -> &str {
        &self.ffmpeg_sha256
    }

    pub fn ffprobe_sha256(&self) -> &str {
        &self.ffprobe_sha256
    }

    #[cfg(test)]
    pub(crate) fn for_tests(
        ffmpeg_path: PathBuf,
        ffprobe_path: PathBuf,
        ffmpeg_version: String,
        ffprobe_version: String,
        build_configuration: String,
    ) -> Self {
        Self {
            ffmpeg_path,
            ffprobe_path,
            ffmpeg_version,
            ffprobe_version,
            build_configuration,
            ffprobe_build_configuration: "test".to_string(),
            ffmpeg_sha256: "test".to_string(),
            ffprobe_sha256: "test".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ToolchainError {
    #[error("{tool} executable path must not be empty")]
    EmptyPath { tool: &'static str },

    #[error("{tool} could not be executed: {reason}")]
    Process { tool: &'static str, reason: String },

    #[error("{tool} exited unsuccessfully (code {exit_code:?}): {details}")]
    CommandFailed {
        tool: &'static str,
        exit_code: Option<i32>,
        details: String,
    },

    #[error("{tool} did not report a recognizable version")]
    MissingVersion { tool: &'static str },

    #[error("{tool} binary could not be hashed: {reason}")]
    Hash { tool: &'static str, reason: String },

    #[error("{tool} binary hash is not approved: expected {expected}, found {actual}")]
    UnapprovedBinary {
        tool: &'static str,
        expected: &'static str,
        actual: String,
    },

    #[error("FFmpeg build contains forbidden flag {flag}")]
    ForbiddenBuildFlag { flag: String },

    #[error("required FFmpeg {kind} is unavailable: {name}")]
    MissingCapability {
        kind: &'static str,
        name: &'static str,
    },
}

impl ToolchainError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::EmptyPath { .. }
            | Self::Process { .. }
            | Self::CommandFailed { .. }
            | Self::MissingVersion { .. }
            | Self::Hash { .. } => "FFMPEG_TOOLCHAIN_UNAVAILABLE",
            Self::UnapprovedBinary { .. } => "FFMPEG_UNAPPROVED_BUILD",
            Self::ForbiddenBuildFlag { .. } => "FFMPEG_FORBIDDEN_BUILD",
            Self::MissingCapability { .. } => "FFMPEG_CAPABILITY_MISSING",
        }
    }
}

fn verify_binary_hash(
    tool: &'static str,
    path: &Path,
    expected: &'static str,
) -> Result<String, ToolchainError> {
    let mut file = File::open(path).map_err(|error| ToolchainError::Hash {
        tool,
        reason: error.to_string(),
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| ToolchainError::Hash {
                tool,
                reason: error.to_string(),
            })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let actual = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<String>();
    if actual != expected {
        return Err(ToolchainError::UnapprovedBinary {
            tool,
            expected,
            actual,
        });
    }
    Ok(actual)
}

fn checked_command<R: ProcessRunner>(
    runner: &R,
    tool: &'static str,
    executable: &Path,
    arguments: &[&str],
) -> Result<ProcessOutput, ToolchainError> {
    let args: Vec<OsString> = arguments.iter().map(OsString::from).collect();
    let cancellation = CancellationToken::new();
    let output = runner
        .run(executable, &args, &cancellation)
        .map_err(|error| process_error(tool, error))?;
    if output.cancelled || !output.success || output.truncated {
        return Err(ToolchainError::CommandFailed {
            tool,
            exit_code: output.exit_code,
            details: process_details(&output),
        });
    }
    Ok(output)
}

fn process_error(tool: &'static str, error: ProcessRunnerError) -> ToolchainError {
    ToolchainError::Process {
        tool,
        reason: error.to_string(),
    }
}

fn version_line(tool: &'static str, output: &ProcessOutput) -> Result<String, ToolchainError> {
    combine_output(output)
        .lines()
        .find(|line| {
            line.to_ascii_lowercase()
                .contains(&format!("{tool} version"))
        })
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .ok_or(ToolchainError::MissingVersion { tool })
}

fn reject_forbidden_build_flags(stdout: &str, stderr: &str) -> Result<(), ToolchainError> {
    for token in stdout.split_whitespace().chain(stderr.split_whitespace()) {
        if FORBIDDEN_BUILD_FLAGS.iter().any(|flag| {
            token == *flag
                || token
                    .strip_prefix(flag)
                    .is_some_and(|suffix| suffix.starts_with('='))
        }) {
            return Err(ToolchainError::ForbiddenBuildFlag {
                flag: token.to_string(),
            });
        }
    }
    Ok(())
}

fn require_entry(
    kind: &'static str,
    name: &'static str,
    required_flag: char,
    output: &ProcessOutput,
) -> Result<(), ToolchainError> {
    let found = combine_output(output).lines().any(|line| {
        let mut fields = line.split_whitespace();
        let Some(flags) = fields.next() else {
            return false;
        };
        flags.contains(required_flag)
            && fields
                .flat_map(|field| field.split(','))
                .any(|field| field == name)
    });
    if found {
        Ok(())
    } else {
        Err(ToolchainError::MissingCapability { kind, name })
    }
}

fn combine_output(output: &ProcessOutput) -> String {
    match (output.stdout.is_empty(), output.stderr.is_empty()) {
        (true, true) => String::new(),
        (false, true) => output.stdout.clone(),
        (true, false) => output.stderr.clone(),
        (false, false) => format!("{}\n{}", output.stdout, output.stderr),
    }
}

fn process_details(output: &ProcessOutput) -> String {
    let details = combine_output(output).trim().to_string();
    let details = if details.is_empty() {
        "no diagnostic output".to_string()
    } else {
        details
    };
    if output.truncated {
        format!("{details} (output truncated)")
    } else {
        details
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::ffi::OsString;
    use std::path::Path;
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    struct FakeRunner {
        outputs: HashMap<String, ProcessOutput>,
        calls: Mutex<Vec<Vec<OsString>>>,
    }

    impl FakeRunner {
        fn with_standard_outputs() -> Self {
            let mut outputs = HashMap::new();
            outputs.insert(
                "-version".to_string(),
                ProcessOutput {
                    success: true,
                    cancelled: false,
                    truncated: false,
                    exit_code: Some(0),
                    stdout: "ffmpeg version 9.0.1 Silk\n".to_string(),
                    stderr: String::new(),
                },
            );
            outputs.insert(
                "-version:ffprobe".to_string(),
                ProcessOutput {
                    success: true,
                    cancelled: false,
                    truncated: false,
                    exit_code: Some(0),
                    stdout: "ffprobe version 9.0.1 Silk\n".to_string(),
                    stderr: String::new(),
                },
            );
            outputs.insert(
                "-buildconf".to_string(),
                success("    --disable-gpl --disable-nonfree\n"),
            );
            outputs.insert(
                "-buildconf:ffprobe".to_string(),
                success("    --disable-gpl --disable-nonfree\n"),
            );
            outputs.insert("-formats".to_string(), success(" E  mp4\n"));
            outputs.insert(
                "-decoders".to_string(),
                success(" V..... h264\n A..... aac\n"),
            );
            outputs.insert(
                "-encoders".to_string(),
                success(" V..... h264_mf\n A..... aac\n"),
            );
            outputs.insert("-muxers".to_string(), success(" E  mp4\n E  null\n"));
            Self {
                outputs,
                calls: Mutex::new(Vec::new()),
            }
        }
    }

    impl ProcessRunner for FakeRunner {
        fn run(
            &self,
            executable: &Path,
            args: &[OsString],
            _cancellation: &CancellationToken,
        ) -> Result<ProcessOutput, ProcessRunnerError> {
            let mut call = vec![executable.as_os_str().to_os_string()];
            call.extend_from_slice(args);
            self.calls.lock().expect("calls").push(call);
            let command = args
                .iter()
                .find(|argument| {
                    matches!(
                        argument.to_string_lossy().as_ref(),
                        "-version"
                            | "-buildconf"
                            | "-formats"
                            | "-decoders"
                            | "-encoders"
                            | "-muxers"
                    )
                })
                .expect("fake command switch");
            let key = if executable.to_string_lossy().contains("ffprobe") {
                format!("{}:ffprobe", command.to_string_lossy())
            } else {
                command.to_string_lossy().into_owned()
            };
            self.outputs
                .get(&key)
                .cloned()
                .ok_or_else(|| ProcessRunnerError::Spawn {
                    executable: executable.to_path_buf(),
                    reason: format!("unexpected fake command: {key}"),
                })
        }
    }

    fn success(stdout: &str) -> ProcessOutput {
        ProcessOutput {
            success: true,
            cancelled: false,
            truncated: false,
            exit_code: Some(0),
            stdout: stdout.to_string(),
            stderr: String::new(),
        }
    }

    #[test]
    fn verifies_required_capabilities_and_records_provenance() {
        let runner = FakeRunner::with_standard_outputs();
        let toolchain = FfmpegToolchain::new("ffmpeg.exe", "ffprobe.exe");

        let verified = toolchain.verify_with(&runner).expect("verified toolchain");

        assert_eq!(verified.ffmpeg_version(), "ffmpeg version 9.0.1 Silk");
        assert_eq!(verified.ffprobe_version(), "ffprobe version 9.0.1 Silk");
        assert!(verified.build_configuration().contains("--disable-gpl"));
        assert!(verified
            .ffprobe_build_configuration()
            .contains("--disable-gpl"));
        assert_eq!(runner.calls.lock().expect("calls").len(), 8);
    }

    #[test]
    fn rejects_forbidden_build_flags_before_capability_checks() {
        let mut runner = FakeRunner::with_standard_outputs();
        runner.outputs.insert(
            "-buildconf".to_string(),
            success("    --enable-nonfree --disable-gpl\n"),
        );

        let error = FfmpegToolchain::new("ffmpeg.exe", "ffprobe.exe")
            .verify_with(&runner)
            .expect_err("nonfree build must be rejected");

        assert_eq!(
            error,
            ToolchainError::ForbiddenBuildFlag {
                flag: "--enable-nonfree".to_string()
            }
        );
    }

    #[test]
    fn accepts_explicitly_approved_gpl_build_flags() {
        let mut runner = FakeRunner::with_standard_outputs();
        runner.outputs.insert(
            "-buildconf".to_string(),
            success("    --enable-gpl --enable-libx264 --disable-nonfree\n"),
        );

        FfmpegToolchain::new("ffmpeg.exe", "ffprobe.exe")
            .verify_with(&runner)
            .expect("approved GPL configuration should pass policy checks");
    }

    #[test]
    fn rejects_a_binary_with_an_unapproved_hash() {
        let path =
            std::env::temp_dir().join(format!("silk-ffmpeg-hash-test-{}.bin", std::process::id()));
        std::fs::write(&path, b"not an approved executable").expect("test binary");

        let error = verify_binary_hash("ffmpeg", &path, APPROVED_FFMPEG_SHA256)
            .expect_err("unapproved bytes must be rejected");
        let _ = std::fs::remove_file(&path);

        assert!(matches!(error, ToolchainError::UnapprovedBinary { .. }));
    }

    #[test]
    fn rejects_missing_capability() {
        let mut runner = FakeRunner::with_standard_outputs();
        runner
            .outputs
            .insert("-muxers".to_string(), success(" E  avi\n"));

        let error = FfmpegToolchain::new("ffmpeg.exe", "ffprobe.exe")
            .verify_with(&runner)
            .expect_err("missing MP4 muxer must be rejected");

        assert_eq!(
            error,
            ToolchainError::MissingCapability {
                kind: "muxer",
                name: "mp4"
            }
        );
    }
}
