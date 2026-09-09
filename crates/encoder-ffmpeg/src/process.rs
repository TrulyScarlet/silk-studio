use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use clip_export::CancellationToken;
use thiserror::Error;

/// Keep diagnostic output bounded even when a tool reports a very large log.
/// This also accommodates normal capability listings and bounded packet
/// metadata used by export validation.
pub const MAX_CAPTURED_PROCESS_OUTPUT_BYTES: usize = 1024 * 1024;
const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(20);
const OUTPUT_READ_CHUNK_BYTES: usize = 4096;

/// Result of one bounded child-process invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessOutput {
    pub success: bool,
    pub cancelled: bool,
    pub truncated: bool,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// Errors raised by the system process adapter itself.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProcessRunnerError {
    #[error("could not start {executable}: {reason}")]
    Spawn { executable: PathBuf, reason: String },

    #[error("could not wait for {executable}: {reason}")]
    Wait { executable: PathBuf, reason: String },

    #[error("could not terminate {executable}: {reason}")]
    Kill { executable: PathBuf, reason: String },

    #[error("could not collect {stream} output from {executable}: {reason}")]
    Output {
        executable: PathBuf,
        stream: &'static str,
        reason: String,
    },
}

/// Process adapter used by the exporter. Tests can provide a fake adapter
/// without starting external programs or requiring FFmpeg on the machine.
pub trait ProcessRunner: Send + Sync {
    fn run(
        &self,
        executable: &Path,
        args: &[OsString],
        cancellation: &CancellationToken,
    ) -> Result<ProcessOutput, ProcessRunnerError>;
}

/// Direct child-process runner. It never invokes a shell and polls the child
/// so cooperative cancellation can terminate it promptly.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemProcessRunner;

impl ProcessRunner for SystemProcessRunner {
    fn run(
        &self,
        executable: &Path,
        args: &[OsString],
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

        let mut command = Command::new(executable);
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            use windows_sys::Win32::System::Threading::{
                BELOW_NORMAL_PRIORITY_CLASS, CREATE_NO_WINDOW,
            };

            // The child performs export encoding and validation. Keep its CPU
            // scheduling below the capture/game path and prevent any console window popup.
            command.creation_flags(BELOW_NORMAL_PRIORITY_CLASS | CREATE_NO_WINDOW);
        }
        let mut child = command.spawn().map_err(|error| ProcessRunnerError::Spawn {
            executable: executable.to_path_buf(),
            reason: error.to_string(),
        })?;
        #[cfg(windows)]
        set_background_process_priority(&child);

        let stdout_reader = spawn_reader(child.stdout.take());
        let stderr_reader = spawn_reader(child.stderr.take());
        let mut cancelled = false;
        let status = wait_for_child(&mut child, executable, cancellation, &mut cancelled)?;
        let stdout = join_reader(stdout_reader, executable, "stdout")?;
        let stderr = join_reader(stderr_reader, executable, "stderr")?;

        Ok(ProcessOutput {
            success: status.success(),
            cancelled,
            truncated: stdout.1 || stderr.1,
            exit_code: status.code(),
            stdout: stdout.0,
            stderr: stderr.0,
        })
    }
}

#[cfg(windows)]
fn set_background_process_priority(child: &Child) {
    use std::os::windows::io::AsRawHandle;

    use windows_sys::Win32::System::Threading::{SetPriorityClass, PROCESS_MODE_BACKGROUND_BEGIN};

    unsafe {
        let _ = SetPriorityClass(child.as_raw_handle() as _, PROCESS_MODE_BACKGROUND_BEGIN);
    }
}

fn wait_for_child(
    child: &mut Child,
    executable: &Path,
    cancellation: &CancellationToken,
    cancelled: &mut bool,
) -> Result<ExitStatus, ProcessRunnerError> {
    loop {
        if cancellation.is_cancelled() {
            *cancelled = true;
            child.kill().map_err(|error| ProcessRunnerError::Kill {
                executable: executable.to_path_buf(),
                reason: error.to_string(),
            })?;
            return child.wait().map_err(|error| ProcessRunnerError::Wait {
                executable: executable.to_path_buf(),
                reason: error.to_string(),
            });
        }

        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => thread::sleep(PROCESS_POLL_INTERVAL),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(ProcessRunnerError::Wait {
                    executable: executable.to_path_buf(),
                    reason: error.to_string(),
                });
            }
        }
    }
}

fn spawn_reader<R>(reader: Option<R>) -> JoinHandle<Result<(String, bool), String>>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let Some(reader) = reader else {
            return Ok((String::new(), false));
        };
        let (bytes, truncated) = read_capped(reader)?;
        Ok((String::from_utf8_lossy(&bytes).into_owned(), truncated))
    })
}

fn join_reader(
    reader: JoinHandle<Result<(String, bool), String>>,
    executable: &Path,
    stream: &'static str,
) -> Result<(String, bool), ProcessRunnerError> {
    match reader.join() {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(reason)) => Err(ProcessRunnerError::Output {
            executable: executable.to_path_buf(),
            stream,
            reason,
        }),
        Err(_) => Err(ProcessRunnerError::Output {
            executable: executable.to_path_buf(),
            stream,
            reason: "output reader panicked".to_string(),
        }),
    }
}

fn read_capped<R: Read>(mut reader: R) -> Result<(Vec<u8>, bool), String> {
    let mut output = Vec::with_capacity(MAX_CAPTURED_PROCESS_OUTPUT_BYTES);
    let mut buffer = [0_u8; OUTPUT_READ_CHUNK_BYTES];
    let mut truncated = false;
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|error| error.to_string())?;
        if read == 0 {
            return Ok((output, truncated));
        }
        let remaining = MAX_CAPTURED_PROCESS_OUTPUT_BYTES.saturating_sub(output.len());
        if remaining > 0 {
            output.extend_from_slice(&buffer[..read.min(remaining)]);
        }
        if read > remaining {
            truncated = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::path::Path;

    use super::*;

    #[test]
    fn capped_reader_reports_truncation_without_growing_storage() {
        let input = vec![b'x'; MAX_CAPTURED_PROCESS_OUTPUT_BYTES + 1];

        let (output, truncated) = read_capped(Cursor::new(input)).expect("read");

        assert_eq!(output.len(), MAX_CAPTURED_PROCESS_OUTPUT_BYTES);
        assert!(truncated);
    }

    #[test]
    fn cancelled_system_process_does_not_start_an_executable() {
        let cancellation = CancellationToken::new();
        cancellation.cancel();

        let output = SystemProcessRunner
            .run(
                Path::new("path-that-does-not-exist.exe"),
                &[],
                &cancellation,
            )
            .expect("cancelled result");

        assert!(output.cancelled);
        assert!(!output.success);
    }
}
