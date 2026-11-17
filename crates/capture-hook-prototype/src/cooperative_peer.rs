//! Owned fixture process and bounded protocol reader. Development sources only.

use crate::transport::{Message, PACKET_SIZE};
use std::io::{Read, Write};
use std::os::windows::io::AsRawHandle;
use std::path::Path;
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{HANDLE, WAIT_OBJECT_0};
use windows::Win32::System::Threading::WaitForSingleObject;

pub fn write_message(writer: &mut impl Write, message: Message) -> Result<(), String> {
    writer
        .write_all(&message.encode())
        .and_then(|()| writer.flush())
        .map_err(|error| error.to_string())
}
pub fn read_message(reader: &mut impl Read) -> Result<Message, String> {
    let mut packet = [0; PACKET_SIZE];
    reader
        .read_exact(&mut packet)
        .map_err(|error| format!("peer pipe closed or truncated: {error}"))?;
    Message::decode(&packet).map_err(|error| format!("peer protocol rejected: {error:?}"))
}

pub struct FixturePeer {
    child: Child,
    input: Option<ChildStdin>,
    messages: Option<Receiver<Result<Message, String>>>,
    reader: Option<JoinHandle<()>>,
    #[cfg(feature = "resident-hooks")]
    passive: bool,
    #[cfg(feature = "resident-hooks")]
    executable: std::path::PathBuf,
    #[cfg(feature = "resident-hooks")]
    attachment_reserved: AtomicBool,
}
impl FixturePeer {
    pub fn launch(warp: bool, fault: Option<&str>) -> Result<Self, String> {
        Self::launch_fixture(
            &std::env::current_exe().map_err(|error| error.to_string())?,
            warp,
            fault,
        )
    }

    /// A development path supplied by the test harness, never a UI/process ID.
    pub fn launch_fixture(
        executable: &Path,
        warp: bool,
        fault: Option<&str>,
    ) -> Result<Self, String> {
        Self::launch_mode(executable, warp, fault, false, false)
    }

    pub fn launch_helper(warp: bool) -> Result<Self, String> {
        Self::launch_mode(
            &std::env::current_exe().map_err(|error| error.to_string())?,
            warp,
            None,
            true,
            false,
        )
    }
    pub fn launch_helper_fixture(executable: &Path, warp: bool) -> Result<Self, String> {
        Self::launch_mode(executable, warp, None, true, false)
    }
    #[cfg(feature = "resident-hooks")]
    pub fn launch_passive_fixture(executable: &Path, warp: bool) -> Result<Self, String> {
        Self::launch_mode(executable, warp, None, false, true)
    }
    #[cfg(feature = "resident-hooks")]
    pub(crate) fn passive_target(&self) -> Option<&Path> {
        self.passive.then_some(self.executable.as_path())
    }
    #[cfg(feature = "resident-hooks")]
    pub(crate) fn reserve_attachment(&self) -> Result<(), String> {
        self.attachment_reserved
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| ())
            .map_err(|_| {
                "one resident attachment attempt per owned process; relaunch fixture".into()
            })
    }
    pub fn is_running(&self) -> bool {
        (unsafe { WaitForSingleObject(self.handle(), 0) })
            == windows::Win32::Foundation::WAIT_TIMEOUT
    }
    pub fn owned_process_handle(&self) -> Result<std::os::windows::io::OwnedHandle, String> {
        unsafe { std::os::windows::io::BorrowedHandle::borrow_raw(self.child.as_raw_handle()) }
            .try_clone_to_owned()
            .map_err(|error| error.to_string())
    }

    fn launch_mode(
        executable: &Path,
        warp: bool,
        fault: Option<&str>,
        helper: bool,
        passive: bool,
    ) -> Result<Self, String> {
        let executable = executable
            .canonicalize()
            .map_err(|error| error.to_string())?;
        if !executable.file_name().is_some_and(|name| {
            name.to_string_lossy()
                .eq_ignore_ascii_case("silk-d3d11-hook-fixture.exe")
        }) {
            return Err(
                "only the explicit controlled D3D11 fixture executable is supported".into(),
            );
        }
        let mut command = Command::new(&executable);
        command.arg("--allow-self-hook");
        if warp {
            command.arg("--warp");
        }
        if passive {
            command.args(["--passive-renderer", "--allow-external-helper"]);
        } else if helper {
            command.args(["--helper-producer", "--allow-helper-load"]);
        } else if let Some(fault) = fault.filter(|fault| *fault != "bad-ack") {
            command.arg(format!("--transport-fault={fault}"));
        } else {
            command.arg("--transport-producer");
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(if fault.is_some() {
                Stdio::null()
            } else {
                Stdio::inherit()
            })
            .spawn()
            .map_err(|error| error.to_string())?;
        let input = child.stdin.take();
        let mut output = match child.stdout.take() {
            Some(output) if input.is_some() => output,
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("missing producer standard pipes".into());
            }
        };
        let (sender, messages) = sync_channel(4);
        let reader = match std::thread::Builder::new()
            .name("silk-fixture-peer-reader".into())
            .spawn(move || loop {
                let message = read_message(&mut output);
                let failed = message.is_err();
                if sender.send(message).is_err() || failed {
                    break;
                }
            }) {
            Ok(reader) => reader,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error.to_string());
            }
        };
        Ok(Self {
            child,
            input,
            messages: Some(messages),
            reader: Some(reader),
            #[cfg(feature = "resident-hooks")]
            passive,
            #[cfg(feature = "resident-hooks")]
            executable,
            #[cfg(feature = "resident-hooks")]
            attachment_reserved: AtomicBool::new(false),
        })
    }

    pub fn receive(&self, timeout: Duration) -> Result<Message, String> {
        self.messages
            .as_ref()
            .ok_or("peer reader closed")?
            .recv_timeout(timeout)
            .map_err(|error| format!("peer receive deadline/connection failed: {error}"))?
    }

    pub fn receive_cancellable(
        &self,
        timeout: Duration,
        stop: &AtomicBool,
    ) -> Result<Message, String> {
        let deadline = Instant::now() + timeout;
        let messages = self.messages.as_ref().ok_or("peer reader closed")?;
        loop {
            if stop.load(Ordering::Acquire) {
                return Err("capture session stopped".into());
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err("peer receive deadline expired".into());
            }
            match messages.recv_timeout(remaining.min(Duration::from_millis(25))) {
                Ok(message) => return message,
                Err(RecvTimeoutError::Disconnected) => {
                    return Err("peer reader disconnected".into())
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
        }
    }

    pub fn send(&mut self, message: Message) -> Result<(), String> {
        write_message(self.input.as_mut().ok_or("peer input closed")?, message)
    }
    pub fn close_input(&mut self) {
        self.input.take();
    }
    pub fn handle(&self) -> HANDLE {
        HANDLE(self.child.as_raw_handle())
    }
    pub fn pid(&self) -> u32 {
        self.child.id()
    }
    pub fn kill(&mut self) -> Result<(), String> {
        self.child.kill().map_err(|error| error.to_string())
    }
    pub fn wait_status(&mut self, timeout: Duration) -> Result<ExitStatus, String> {
        let milliseconds =
            u32::try_from(timeout.as_millis()).map_err(|_| "invalid child exit timeout")?;
        if unsafe { WaitForSingleObject(self.handle(), milliseconds) } != WAIT_OBJECT_0 {
            return Err("producer did not exit by deadline".into());
        }
        self.child.wait().map_err(|error| error.to_string())
    }
    pub fn expect_exit(&mut self) -> Result<(), String> {
        let status = self.wait_status(Duration::from_secs(5))?;
        if !status.success() {
            return Err(format!("producer failed: {status}"));
        }
        Ok(())
    }
}
impl Drop for FixturePeer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.input.take();
        self.messages.take();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}
