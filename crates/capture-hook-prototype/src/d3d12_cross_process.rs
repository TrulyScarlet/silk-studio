//! Exact owned-child transport. No arbitrary PID, injection or production source.
use crate::d3d12_gpu_bridge::consumer_device;
use crate::d3d12_present_fixture::D3d12PresentFixture;
use crate::d3d12_protocol::{GpuHello, Wire};
use crate::local_gpu_bridge::{GpuPoolBudget, LocalGpuBridge};
use crate::publication_pool::Publication;
use crate::transport::{Message, PACKET_SIZE};
use crate::{FrameHeader, SessionGate, SessionIdentity};
use encoder_api::GpuFrameContext;
use gpu_windows::{GpuDeviceContext, GpuTextureSlot};
use std::io::{Read, Write};
use std::os::windows::io::{AsRawHandle, BorrowedHandle, FromRawHandle, OwnedHandle};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError, SyncSender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use windows::core::Interface;
use windows::Win32::Foundation::{
    DuplicateHandle, DUPLICATE_SAME_ACCESS, FILETIME, HANDLE, HWND, LUID, WAIT_OBJECT_0,
    WAIT_TIMEOUT,
};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::System::Performance::QueryPerformanceFrequency;
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetProcessId, GetProcessTimes, WaitForSingleObject,
};
use windows::Win32::UI::WindowsAndMessaging::{GetWindowThreadProcessId, IsWindow};

fn error(e: windows::core::Error) -> String {
    e.to_string()
}
pub fn read_wire(reader: &mut impl Read) -> Result<Wire, String> {
    let mut bytes = [0; PACKET_SIZE];
    reader
        .read_exact(&mut bytes)
        .map_err(|e| format!("D3D12 peer closed/truncated: {e}"))?;
    Wire::decode(&bytes).map_err(|e| format!("D3D12 packet rejected: {e:?}"))
}
pub fn write_wire(writer: &mut impl Write, wire: Wire) -> Result<(), String> {
    writer
        .write_all(&wire.encode())
        .and_then(|()| writer.flush())
        .map_err(|e| e.to_string())
}
fn running(process: HANDLE) -> bool {
    (unsafe { WaitForSingleObject(process, 0) }) == WAIT_TIMEOUT
}
fn validate_child(process: HANDLE, pid: u32, session: SessionIdentity) -> Result<(), String> {
    if !running(process) || SessionGate::new(session).is_none() {
        return Err("owned producer exited/invalid session".into());
    }
    let hwnd = HWND(session.target.window as usize as *mut _);
    let mut window_pid = 0;
    let mut created = FILETIME::default();
    let mut exited = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    unsafe {
        if !IsWindow(Some(hwnd)).as_bool() {
            return Err("owned producer HWND closed".into());
        }
        GetWindowThreadProcessId(hwnd, Some(&mut window_pid));
        GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user)
            .map_err(error)?;
        if GetProcessId(process) != pid
            || session.target.pid != pid
            || window_pid != pid
            || session.target.creation_time
                != ((u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime))
        {
            return Err("owned producer PID/lifetime/HWND mismatch".into());
        }
    }
    Ok(())
}

pub struct Peer {
    child: Child,
    commands: Option<SyncSender<Wire>>,
    messages: Option<Receiver<Result<Wire, String>>>,
    reader: Option<JoinHandle<()>>,
    writer: Option<JoinHandle<()>>,
    writer_error: Arc<Mutex<Option<String>>>,
    #[cfg(feature = "d3d12-remote-helper-fixture")]
    passive_target: Option<std::path::PathBuf>,
    #[cfg(feature = "d3d12-remote-helper-fixture")]
    attachment_reserved: AtomicBool,
}
#[derive(Default)]
struct LaunchMode {
    observed_loss: bool,
    automatic: bool,
    late_scenario: Option<LateResizeScenario>,
    late: bool,
    passive: bool,
    legacy_factory: bool,
    present_hook: bool,
    factory_hook: bool,
    bootstrap: bool,
    rearm: bool,
    replace_device: bool,
    lifetime_seconds: Option<u32>,
}
pub(crate) fn is_owned_loss_probe(fault: Option<&str>) -> bool {
    matches!(
        fault,
        Some("remove-on-command" | "remove-and-recreate-on-command")
    )
}
fn clear_owned_pending_ack(
    pending: &mut [Option<u64>; 3],
    expected: SessionIdentity,
    session: SessionIdentity,
    slot: u32,
    sequence: u64,
) -> Result<(), String> {
    if session != expected || pending.get(slot as usize) != Some(&Some(sequence)) {
        return Err("stale/mismatched owned producer ACK".into());
    }
    pending[slot as usize] = None;
    Ok(())
}
#[cfg(test)]
mod loss_control_tests {
    use super::*;
    #[test]
    fn loss_profile_requires_exact_explicit_names() {
        for value in [
            None,
            Some(""),
            Some("replace-remove-pending"),
            Some("remove-on-command-extra"),
            Some("remove-and-recreate-on-command-extra"),
        ] {
            assert!(!is_owned_loss_probe(value));
        }
        assert!(is_owned_loss_probe(Some("remove-on-command")));
        assert!(is_owned_loss_probe(Some("remove-and-recreate-on-command")));
    }
    #[test]
    fn terminal_loss_acks_still_require_exact_session_slot_and_sequence_once() {
        let expected = SessionIdentity {
            target: crate::TargetIdentity {
                pid: 1,
                creation_time: 2,
                window: 3,
            },
            generation: 1,
            epoch: 1,
        };
        let original = [Some(1), Some(2), None];
        let mut pending = original;
        assert!(clear_owned_pending_ack(&mut pending, expected, expected, 3, 1).is_err());
        assert!(clear_owned_pending_ack(&mut pending, expected, expected, 0, 2).is_err());
        assert!(clear_owned_pending_ack(
            &mut pending,
            expected,
            SessionIdentity {
                epoch: 2,
                ..expected
            },
            0,
            1
        )
        .is_err());
        assert_eq!(pending, original);
        clear_owned_pending_ack(&mut pending, expected, expected, 0, 1).unwrap();
        assert!(clear_owned_pending_ack(&mut pending, expected, expected, 0, 1).is_err());
        assert_eq!(pending, [None, Some(2), None]);
    }
}
#[derive(Clone, Copy)]
pub enum LateResizeScenario {
    Observe,
    NoResize,
    DistinctQueues,
}
impl Peer {
    #[cfg(feature = "d3d12-observed-loss-fixture")]
    pub fn launch_observed_loss(executable: &Path) -> Result<Self, String> {
        Self::launch_mode(
            executable,
            false,
            None,
            LaunchMode {
                observed_loss: true,
                passive: true,
                present_hook: true,
                factory_hook: true,
                ..Default::default()
            },
        )
    }
    #[cfg(feature = "d3d12-auto-resize-fixture")]
    pub fn launch_automatic_resize(executable: &Path, warp: bool) -> Result<Self, String> {
        Self::launch_mode(
            executable,
            warp,
            None,
            LaunchMode {
                automatic: true,
                late: true,
                passive: true,
                present_hook: true,
                factory_hook: true,
                ..Default::default()
            },
        )
    }
    #[cfg(feature = "d3d12-late-resize-fixture")]
    pub fn launch_late_resize(executable: &Path, warp: bool) -> Result<Self, String> {
        Self::launch_late_resize_scenario(executable, warp, LateResizeScenario::Observe)
    }
    #[cfg(feature = "d3d12-late-resize-fixture")]
    pub fn launch_late_resize_scenario(
        executable: &Path,
        warp: bool,
        scenario: LateResizeScenario,
    ) -> Result<Self, String> {
        Self::launch_mode(
            executable,
            warp,
            None,
            LaunchMode {
                present_hook: true,
                factory_hook: true,
                passive: true,
                late: true,
                late_scenario: Some(scenario),
                ..Default::default()
            },
        )
    }
    #[cfg(feature = "d3d12-remote-helper-fixture")]
    pub fn launch_passive(executable: &Path, warp: bool) -> Result<Self, String> {
        Self::launch_passive_factory(executable, warp, false)
    }
    #[cfg(feature = "d3d12-remote-helper-fixture")]
    pub fn launch_passive_factory(
        executable: &Path,
        warp: bool,
        legacy: bool,
    ) -> Result<Self, String> {
        Self::launch_mode(
            executable,
            warp,
            None,
            LaunchMode {
                present_hook: true,
                factory_hook: true,
                passive: true,
                legacy_factory: legacy,
                ..Default::default()
            },
        )
    }
    #[cfg(feature = "d3d12-remote-helper-fixture")]
    pub(crate) fn passive_target(&self) -> Option<&Path> {
        self.passive_target.as_deref()
    }
    #[cfg(feature = "d3d12-remote-helper-fixture")]
    pub(crate) fn reserve_attachment(&self) -> Result<(), String> {
        self.attachment_reserved
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| ())
            .map_err(|_| "one D3D12 attachment attempt per owned child".into())
    }
    pub fn launch(executable: &Path, warp: bool, fault: Option<&str>) -> Result<Self, String> {
        Self::launch_mode(executable, warp, fault, LaunchMode::default())
    }
    #[cfg(feature = "d3d12-present-hook-fixture")]
    pub fn launch_hook(executable: &Path, warp: bool, fault: Option<&str>) -> Result<Self, String> {
        Self::launch_mode(
            executable,
            warp,
            fault,
            LaunchMode {
                present_hook: true,
                ..Default::default()
            },
        )
    }
    #[cfg(feature = "d3d12-factory-hook-fixture")]
    pub fn launch_factory_hook(
        executable: &Path,
        warp: bool,
        fault: Option<&str>,
    ) -> Result<Self, String> {
        Self::launch_mode(
            executable,
            warp,
            fault,
            LaunchMode {
                present_hook: true,
                factory_hook: true,
                ..Default::default()
            },
        )
    }
    #[cfg(feature = "d3d12-bootstrap-fixture")]
    pub fn launch_bootstrap(
        executable: &Path,
        warp: bool,
        fault: Option<&str>,
    ) -> Result<Self, String> {
        Self::launch_mode(
            executable,
            warp,
            fault,
            LaunchMode {
                present_hook: true,
                factory_hook: true,
                bootstrap: true,
                ..Default::default()
            },
        )
    }
    #[cfg(feature = "d3d12-bootstrap-fixture")]
    pub fn launch_legacy_factory(executable: &Path, warp: bool) -> Result<Self, String> {
        Self::launch_mode(
            executable,
            warp,
            None,
            LaunchMode {
                present_hook: true,
                factory_hook: true,
                bootstrap: true,
                legacy_factory: true,
                ..Default::default()
            },
        )
    }
    #[cfg(feature = "d3d12-bootstrap-fixture")]
    pub fn launch_endurance(executable: &Path, warp: bool, seconds: u32) -> Result<Self, String> {
        if !(60..=28_800).contains(&seconds) {
            return Err("owned endurance requires 60..28800 seconds".into());
        }
        Self::launch_mode(
            executable,
            warp,
            None,
            LaunchMode {
                present_hook: true,
                factory_hook: true,
                bootstrap: true,
                lifetime_seconds: Some(seconds + 30),
                ..Default::default()
            },
        )
    }
    #[cfg(feature = "d3d12-rearm-fixture")]
    pub fn launch_rearm(
        executable: &Path,
        warp: bool,
        fault: Option<&str>,
    ) -> Result<Self, String> {
        Self::launch_mode(
            executable,
            warp,
            fault,
            LaunchMode {
                present_hook: true,
                factory_hook: true,
                bootstrap: true,
                rearm: true,
                ..Default::default()
            },
        )
    }
    #[cfg(feature = "d3d12-device-replace-fixture")]
    pub fn launch_device_replace(
        executable: &Path,
        warp: bool,
        fault: Option<&str>,
    ) -> Result<Self, String> {
        Self::launch_mode(
            executable,
            warp,
            fault,
            LaunchMode {
                present_hook: true,
                factory_hook: true,
                bootstrap: true,
                rearm: true,
                replace_device: true,
                lifetime_seconds: None,
                legacy_factory: false,
                passive: false,
                late: false,
                late_scenario: None,
                automatic: false,
                observed_loss: false,
            },
        )
    }
    fn launch_mode(
        executable: &Path,
        warp: bool,
        fault: Option<&str>,
        mode: LaunchMode,
    ) -> Result<Self, String> {
        let executable = executable.canonicalize().map_err(|e| e.to_string())?;
        if !executable.file_name().is_some_and(|n| {
            n.to_string_lossy()
                .eq_ignore_ascii_case("silk-d3d12-owned-fixture.exe")
        }) {
            return Err("only the owned D3D12 fixture executable is supported".into());
        }
        if fault.is_some_and(|f| {
            !matches!(
                f,
                "truncate"
                    | "malformed"
                    | "bad-frame"
                    | "exit-after-six"
                    | "bootstrap-stale-target"
                    | "replace-after-release"
                    | "replace-before-hello"
                    | "replace-bad-hello"
                    | "replace-remove-quiescent"
                    | "replace-remove-pending"
                    | "remove-on-command"
                    | "remove-and-recreate-on-command"
            )
        }) {
            return Err("unknown fixture fault".into());
        }
        if fault.is_some_and(|f| f.starts_with("replace-")) && !mode.replace_device {
            return Err("replacement fault requires replacement route".into());
        }
        if fault == Some("bootstrap-stale-target") && !mode.bootstrap {
            return Err("bootstrap fault requires bootstrap route".into());
        }
        if is_owned_loss_probe(fault)
            && (mode.present_hook
                || mode.factory_hook
                || mode.bootstrap
                || mode.passive
                || mode.late)
        {
            return Err(
                "unannounced loss probe requires the manual exact-owned source route".into(),
            );
        }
        let mut command = Command::new(&executable);
        command.args(["--allow-owned-d3d12", "--producer"]);
        if std::env::var_os("SILK_OWNED_FAULT_DIAGNOSTICS").is_some() {
            command.arg("--diagnose-owned-faults");
        }
        if mode.present_hook {
            command.arg("--present-hook");
        }
        if mode.factory_hook {
            command.arg("--factory-hook");
        }
        if mode.bootstrap {
            command.arg("--bootstrap");
        }
        if mode.passive {
            command.args(["--passive-d3d12", "--allow-external-helper"]);
        }
        if mode.late {
            command.arg("--late-d3d12");
        }
        if mode.automatic {
            command.arg("--auto-resize-owned");
        }
        if mode.observed_loss {
            command.arg("--observe-owned-loss");
        }
        match mode.late_scenario {
            Some(LateResizeScenario::NoResize) => {
                command.arg("--late-no-resize");
            }
            Some(LateResizeScenario::DistinctQueues) => {
                command.arg("--late-distinct-queues");
            }
            _ => {}
        }
        if mode.legacy_factory {
            command.arg("--legacy-factory");
        }
        if mode.rearm {
            command.arg("--rearm");
        }
        if mode.replace_device {
            command.arg("--replace-device");
        }
        if let Some(seconds) = mode.lifetime_seconds {
            command.arg(format!("--fixture-lifetime-seconds={seconds}"));
        }
        if warp {
            command.arg("--warp");
        }
        if let Some(fault) = fault {
            command.arg(format!("--fault={fault}"));
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| e.to_string())?;
        let input = child.stdin.take();
        let output = child.stdout.take();
        let (commands, command_rx) = sync_channel(8);
        let (send, messages) = sync_channel(4);
        let writer_error = Arc::new(Mutex::new(None));
        let errors = writer_error.clone();
        let mut peer = Self {
            child,
            commands: Some(commands),
            messages: Some(messages),
            reader: None,
            writer: None,
            writer_error,
            #[cfg(feature = "d3d12-remote-helper-fixture")]
            passive_target: mode.passive.then_some(executable),
            #[cfg(feature = "d3d12-remote-helper-fixture")]
            attachment_reserved: AtomicBool::new(false),
        };
        let mut input = input.ok_or("missing child stdin")?;
        let mut output = output.ok_or("missing child stdout")?;
        peer.writer = Some(
            std::thread::Builder::new()
                .name("silk-d3d12-command-writer".into())
                .spawn(move || {
                    for message in command_rx {
                        if let Err(e) = write_wire(&mut input, message) {
                            if let Ok(mut stored) = errors.lock() {
                                *stored = Some(e);
                            }
                            break;
                        }
                    }
                })
                .map_err(|e| e.to_string())?,
        );
        peer.reader = Some(
            std::thread::Builder::new()
                .name("silk-d3d12-packet-reader".into())
                .spawn(move || loop {
                    let message = read_wire(&mut output);
                    let failed = message.is_err();
                    if send.send(message).is_err() || failed {
                        break;
                    }
                })
                .map_err(|e| e.to_string())?,
        );
        Ok(peer)
    }
    pub fn pid(&self) -> u32 {
        self.child.id()
    }
    pub fn handle(&self) -> HANDLE {
        HANDLE(self.child.as_raw_handle())
    }
    pub fn is_running(&self) -> bool {
        running(self.handle())
    }
    pub fn process_handle(&self) -> Result<OwnedHandle, String> {
        unsafe { BorrowedHandle::borrow_raw(self.child.as_raw_handle()) }
            .try_clone_to_owned()
            .map_err(|e| e.to_string())
    }
    pub fn send(&self, message: Message) -> Result<(), String> {
        if let Some(e) = self
            .writer_error
            .lock()
            .map_err(|_| "peer writer state poisoned")?
            .as_ref()
        {
            return Err(e.clone());
        }
        self.commands
            .as_ref()
            .ok_or("peer input closed")?
            .try_send(Wire::Control(message))
            .map_err(|e| format!("bounded D3D12 control queue unavailable: {e}"))
    }
    pub fn close_input(&mut self) {
        self.commands.take();
    }
    pub fn window_action(&self, action: crate::d3d12_protocol::WindowAction) -> Result<(), String> {
        self.commands
            .as_ref()
            .ok_or("peer input closed")?
            .try_send(Wire::Window(action))
            .map_err(|e| format!("bounded owned-window command queue unavailable: {e}"))
    }
    pub fn resize(&self, request: crate::d3d12_protocol::ResizeRequest) -> Result<(), String> {
        request
            .validate()
            .map_err(|e| format!("invalid resize request: {e:?}"))?;
        self.queue_wire(Wire::Resize(request))
    }
    pub fn retired_epoch(&self, session: crate::SessionIdentity) -> Result<(), String> {
        self.queue_wire(Wire::Retired(session))
    }
    #[cfg(feature = "d3d12-device-replace-fixture")]
    pub fn trigger_owned_loss(&self, session: crate::SessionIdentity) -> Result<(), String> {
        validate_child(self.handle(), self.pid(), session)?;
        self.queue_wire(Wire::TriggerOwnedLoss(session))
    }
    #[cfg(feature = "d3d12-device-replace-fixture")]
    pub fn replace_device(
        &self,
        request: crate::d3d12_protocol::ResizeRequest,
    ) -> Result<(), String> {
        request
            .validate()
            .map_err(|e| format!("invalid replacement request: {e:?}"))?;
        self.queue_wire(Wire::ReplaceDevice(request))
    }
    fn queue_wire(&self, wire: Wire) -> Result<(), String> {
        self.commands
            .as_ref()
            .ok_or("peer input closed")?
            .try_send(wire)
            .map_err(|e| format!("bounded owned control queue unavailable: {e}"))
    }
    pub fn receive(&self, timeout: Duration) -> Result<Wire, String> {
        self.receive_cancellable(timeout, &AtomicBool::new(false))
    }
    pub fn receive_cancellable(
        &self,
        timeout: Duration,
        stop: &AtomicBool,
    ) -> Result<Wire, String> {
        let deadline = Instant::now() + timeout;
        loop {
            if stop.load(Ordering::Acquire) {
                return Err("D3D12 capture stopped".into());
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err("D3D12 peer receive deadline".into());
            }
            match self
                .messages
                .as_ref()
                .ok_or("peer reader closed")?
                .recv_timeout(remaining.min(Duration::from_millis(25)))
            {
                Ok(message) => return message,
                Err(RecvTimeoutError::Disconnected) => {
                    return Err("D3D12 peer reader disconnected".into())
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
        }
    }
    pub fn approve(&self, hello: GpuHello) -> Result<(), String> {
        hello
            .validate()
            .map_err(|e| format!("invalid hello: {e:?}"))?;
        validate_child(self.handle(), self.pid(), hello.session)?;
        self.send(Message::Select(hello.session))
    }
    pub fn kill(&mut self) -> Result<(), String> {
        self.child.kill().map_err(|e| e.to_string())
    }
    pub fn wait_exit(&mut self, timeout: Duration) -> Result<bool, String> {
        let status = self.wait_exit_status(timeout)?;
        if !status.success() {
            eprintln!("owned D3D12 child exited unsuccessfully: {status}");
        }
        Ok(status.success())
    }
    pub fn wait_exit_status(
        &mut self,
        timeout: Duration,
    ) -> Result<std::process::ExitStatus, String> {
        let millis =
            u32::try_from(timeout.as_millis()).map_err(|_| "invalid child wait duration")?;
        if unsafe { WaitForSingleObject(self.handle(), millis) } != WAIT_OBJECT_0 {
            return Err("owned D3D12 child exit deadline".into());
        }
        self.child.wait().map_err(|e| e.to_string())
    }
}
impl Drop for Peer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.commands.take();
        self.messages.take();
        if let Some(w) = self.writer.take() {
            let _ = w.join();
        }
        if let Some(r) = self.reader.take() {
            let _ = r.join();
        }
    }
}

struct ConsumerResources {
    device: Arc<GpuDeviceContext>,
    context4: ID3D11DeviceContext4,
    produced: ID3D11Fence,
    consumed: ID3D11Fence,
    retired: ID3D11Fence,
    textures: [GpuTextureSlot; 3],
    output: Arc<LocalGpuBridge>,
}
pub struct RemoteConsumer {
    resources: Option<ConsumerResources>,
    process: OwnedHandle,
    hello: GpuHello,
    gate: SessionGate,
    slot_sequences: [u64; 3],
    last_consumed: u64,
    highest_seen: u64,
    loss_pending: bool,
    closed: bool,
    uncertain: bool,
    pub gpu_copies: u64,
    pub dropped: u64,
}
pub enum RemoteCopyOutcome {
    Copied(Option<Publication<GpuTextureSlot>>, Message),
    ProducerLost,
}
impl RemoteConsumer {
    pub fn open(
        peer: &Peer,
        hello: GpuHello,
        warp: bool,
        budget: Arc<GpuPoolBudget>,
    ) -> Result<Self, String> {
        hello
            .validate()
            .map_err(|e| format!("invalid D3D12 greeting: {e:?}"))?;
        validate_child(peer.handle(), peer.pid(), hello.session)?;
        let mut frequency = 0;
        unsafe { QueryPerformanceFrequency(&mut frequency) }.map_err(error)?;
        if frequency as u64 != hello.qpc_frequency {
            return Err("D3D12 QPC frequency mismatch".into());
        }
        let luid = LUID {
            LowPart: hello.adapter_luid as u32,
            HighPart: (hello.adapter_luid >> 32) as i32,
        };
        let device = consumer_device(luid, warp)?;
        let device1: ID3D11Device1 = device.device().cast().map_err(error)?;
        let device5: ID3D11Device5 = device.device().cast().map_err(error)?;
        let context4: ID3D11DeviceContext4 = device.context().cast().map_err(error)?;
        let mut handles = Vec::with_capacity(5);
        for remote in hello.handles {
            let mut local = HANDLE::default();
            unsafe {
                DuplicateHandle(
                    peer.handle(),
                    HANDLE(remote as usize as *mut _),
                    GetCurrentProcess(),
                    &mut local,
                    0,
                    false,
                    DUPLICATE_SAME_ACCESS,
                )
            }
            .map_err(error)?;
            handles.push(unsafe { OwnedHandle::from_raw_handle(local.0) });
        }
        let mut textures = Vec::with_capacity(3);
        for handle in &handles[..3] {
            let texture: ID3D11Texture2D =
                unsafe { device1.OpenSharedResource1(HANDLE(handle.as_raw_handle())) }
                    .map_err(error)?;
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            unsafe { texture.GetDesc(&mut desc) };
            if (desc.Width, desc.Height) != (hello.width, hello.height)
                || desc.Format != DXGI_FORMAT_B8G8R8A8_UNORM
                || desc.SampleDesc.Count != 1
                || desc.MipLevels != 1
                || desc.ArraySize != 1
                || desc.Usage != D3D11_USAGE_DEFAULT
                || desc.CPUAccessFlags != 0
            {
                return Err("D3D12 imported texture descriptor mismatch".into());
            }
            textures.push(GpuTextureSlot::new(texture, device.clone()));
        }
        let mut produced = None;
        let mut consumed = None;
        let mut retired = None;
        unsafe { device5.OpenSharedFence(HANDLE(handles[3].as_raw_handle()), &mut produced) }
            .map_err(error)?;
        unsafe { device5.OpenSharedFence(HANDLE(handles[4].as_raw_handle()), &mut consumed) }
            .map_err(error)?;
        unsafe { device5.CreateFence(0, D3D11_FENCE_FLAG_NONE, &mut retired) }.map_err(error)?;
        let output = LocalGpuBridge::new(device.clone(), hello.width, hello.height, budget)?;
        validate_child(peer.handle(), peer.pid(), hello.session)?;
        Ok(Self {
            resources: Some(ConsumerResources {
                device,
                context4,
                produced: produced.ok_or("null produced fence")?,
                consumed: consumed.ok_or("null consumed fence")?,
                retired: retired.ok_or("null local retirement fence")?,
                textures: textures
                    .try_into()
                    .map_err(|_| "invalid shared texture count")?,
                output,
            }),
            process: peer.process_handle()?,
            hello,
            gate: SessionGate::new(hello.session).ok_or("invalid remote session")?,
            slot_sequences: [0; 3],
            last_consumed: 0,
            highest_seen: 0,
            loss_pending: false,
            closed: false,
            uncertain: false,
            gpu_copies: 0,
            dropped: 0,
        })
    }
    fn process(&self) -> HANDLE {
        HANDLE(self.process.as_raw_handle())
    }
    pub fn context(&self) -> GpuFrameContext {
        self.resources.as_ref().unwrap().output.context()
    }
    pub(crate) fn output(&self) -> Arc<LocalGpuBridge> {
        self.resources.as_ref().unwrap().output.clone()
    }
    pub fn is_running(&self) -> bool {
        running(self.process())
    }
    pub fn copy(
        &mut self,
        header: FrameHeader,
    ) -> Result<(Option<Publication<GpuTextureSlot>>, Message), String> {
        self.copy_if(header, true)
    }
    pub fn copy_if(
        &mut self,
        header: FrameHeader,
        admit: bool,
    ) -> Result<(Option<Publication<GpuTextureSlot>>, Message), String> {
        if self.closed || self.loss_pending {
            return Err("remote GPU consumer closed".into());
        }
        let result = self.copy_inner(header, admit);
        if result.is_err() {
            self.close();
        }
        result
    }
    fn copy_inner(
        &mut self,
        header: FrameHeader,
        admit: bool,
    ) -> Result<(Option<Publication<GpuTextureSlot>>, Message), String> {
        self.validate_header(header)?;
        self.copy_validated(header, admit)
    }
    fn validate_header(&mut self, header: FrameHeader) -> Result<(), String> {
        validate_child(
            self.process(),
            self.hello.session.target.pid,
            self.hello.session,
        )?;
        self.gate
            .accept(header)
            .map_err(|e| format!("remote GPU frame rejected: {e:?}"))?;
        if (header.width, header.height) != (self.hello.width, self.hello.height) {
            return Err("remote GPU dimensions changed".into());
        }
        self.highest_seen = header.sequence;
        Ok(())
    }
    fn copy_validated(
        &mut self,
        header: FrameHeader,
        admit: bool,
    ) -> Result<(Option<Publication<GpuTextureSlot>>, Message), String> {
        let r = self.resources.as_ref().unwrap();
        let produced = unsafe { r.produced.GetCompletedValue() };
        let consumed = unsafe { r.consumed.GetCompletedValue() };
        if produced == u64::MAX
            || consumed == u64::MAX
            || produced < header.sequence
            || consumed < self.slot_sequences[header.slot as usize]
        {
            return Err("remote GPU frame is incomplete/reused/device removed".into());
        }
        let uncertain = &mut self.uncertain;
        let publication = if admit {
            r.output
                .copy_imported_after(&r.textures[header.slot as usize], || {
                    *uncertain = true;
                    unsafe { r.context4.Wait(&r.produced, header.sequence) }.map_err(error)
                })?
        } else {
            None
        };
        if publication.is_none() {
            self.uncertain = true;
            unsafe { r.context4.Wait(&r.produced, header.sequence) }.map_err(error)?;
            self.dropped += 1;
        } else {
            self.gpu_copies += 1;
        }
        unsafe { r.context4.Signal(&r.consumed, header.sequence) }.map_err(error)?;
        unsafe { r.context4.Signal(&r.retired, header.sequence) }.map_err(error)?;
        unsafe { r.device.context().Flush() };
        self.last_consumed = header.sequence;
        self.slot_sequences[header.slot as usize] = header.sequence;
        self.uncertain = false;
        if !self.is_running() {
            return Err("owned producer exited before frame delivery".into());
        }
        Ok((
            publication,
            Message::Ack {
                session: header.session,
                slot: header.slot,
                sequence: header.sequence,
            },
        ))
    }
    pub fn close(&mut self) {
        self.closed = true;
        self.gate.close();
        self.resources.as_ref().unwrap().output.retire();
    }
    pub fn producer_removed(&self) -> bool {
        let resources = self.resources.as_ref().unwrap();
        unsafe {
            resources.produced.GetCompletedValue() == u64::MAX
                || resources.consumed.GetCompletedValue() == u64::MAX
        }
    }
    /// Opt-in loss handling. A loss result admits no copy/ACK and deliberately
    /// leaves the output resolver live until the caller crosses its delivery
    /// boundary and invokes retire_epoch_after_producer_loss().
    pub fn copy_if_preserving_loss(
        &mut self,
        header: FrameHeader,
        admit: bool,
    ) -> Result<RemoteCopyOutcome, String> {
        if self.closed {
            return Err("remote GPU consumer closed".into());
        }
        if let Err(error) = self.validate_header(header) {
            self.close();
            return Err(error);
        }
        if self.loss_pending || self.producer_removed() {
            self.loss_pending = true;
            return Ok(RemoteCopyOutcome::ProducerLost);
        }
        match self.copy_validated(header, admit) {
            Ok((publication, ack)) => Ok(RemoteCopyOutcome::Copied(publication, ack)),
            Err(_) if self.producer_removed() => {
                self.loss_pending = true;
                Ok(RemoteCopyOutcome::ProducerLost)
            }
            Err(error) => {
                self.close();
                Err(error)
            }
        }
    }
    /// Control-thread retirement only. A new independent D3D11 tail signal
    /// covers even a partially scheduled copy; a shared removal sentinel does
    /// not prove consumer completion. Failure remains terminal/retained.
    pub fn retire_epoch_after_producer_loss(&mut self) -> Result<(), String> {
        if self.closed && !self.loss_pending {
            return Err("terminal consumer cannot enter deferred loss retirement".into());
        }
        if !self.producer_removed() {
            return Err("loss retirement requires actual producer removal".into());
        }
        self.close();
        self.uncertain = true;
        let resources = self.resources.as_ref().unwrap();
        unsafe { resources.device.device().GetDeviceRemovedReason() }.map_err(error)?;
        let ticket = crate::device_loss_epoch::next_consumer_retirement_ticket(
            self.highest_seen,
            self.last_consumed,
        )
        .ok_or("local loss-retirement fence exhausted")?;
        unsafe { resources.context4.Signal(&resources.retired, ticket) }.map_err(error)?;
        unsafe { resources.device.context().Flush() };
        self.last_consumed = ticket;
        self.uncertain = false;
        if !self.drain() {
            return Err("independent consumer tail did not retire after producer loss".into());
        }
        Ok(())
    }
    pub fn retire_epoch(&mut self) -> Result<(), String> {
        self.close();
        if !self.drain() {
            return Err("old consumer GPU epoch did not drain; replacement denied".into());
        }
        Ok(())
    }
    fn drain(&self) -> bool {
        if self.uncertain {
            return false;
        }
        if self.last_consumed == 0 {
            return true;
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let value = unsafe { self.resources.as_ref().unwrap().retired.GetCompletedValue() };
            if value != u64::MAX && value >= self.last_consumed {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}
impl Drop for RemoteConsumer {
    fn drop(&mut self) {
        self.close();
        let drained = self.drain();
        let resources = self.resources.take().unwrap();
        if !drained {
            eprintln!(
                "remote D3D12 consumer completion uncertain; retaining GPU resources until exit"
            );
            std::mem::forget(resources);
        }
    }
}

/// Called only by the fixed owned-fixture binary with explicit opt-in arguments.
pub fn run_producer(
    warp: bool,
    fault: Option<&str>,
    present_hook: bool,
    factory_hook: bool,
) -> Result<(), String> {
    if !std::env::args().any(|a| a == "--allow-owned-d3d12")
        || !std::env::current_exe().ok().is_some_and(|p| {
            p.file_name().is_some_and(|n| {
                n.to_string_lossy()
                    .eq_ignore_ascii_case("silk-d3d12-owned-fixture.exe")
            })
        })
    {
        return Err("D3D12 producer requires exact owned-fixture opt-in".into());
    }
    if matches!(fault, Some("truncate" | "malformed")) {
        let mut out = std::io::stdout().lock();
        return out
            .write_all(if fault == Some("truncate") {
                b"SK12"
            } else {
                &[255; 128]
            })
            .map_err(|e| e.to_string());
    }
    let loss_probe = is_owned_loss_probe(fault);
    #[cfg(feature = "d3d12-device-replace-fixture")]
    let recover_loss = fault == Some("remove-and-recreate-on-command");
    if loss_probe
        && (warp || present_hook || factory_hook || !cfg!(feature = "d3d12-device-replace-fixture"))
    {
        return Err("unannounced loss probe requires hardware, no hooks, and the enabled owned loss fixture".into());
    }
    #[cfg(not(feature = "d3d12-factory-hook-fixture"))]
    if factory_hook {
        return Err("owned factory hook feature not enabled".into());
    }
    if factory_hook && !present_hook {
        return Err("factory route requires owned Present hook".into());
    }
    let budget = Arc::new(GpuPoolBudget::default());
    #[cfg(feature = "d3d12-present-hook-fixture")]
    let mut fixture = if factory_hook {
        #[cfg(feature = "d3d12-factory-hook-fixture")]
        {
            D3d12PresentFixture::new_gpu_factory_hook(warp, 318, 178, budget.clone())?
        }
        #[cfg(not(feature = "d3d12-factory-hook-fixture"))]
        {
            unreachable!()
        }
    } else if present_hook {
        D3d12PresentFixture::new_gpu_present_hook(warp, 318, 178, budget.clone())?
    } else {
        D3d12PresentFixture::new_gpu(warp, 318, 178, budget.clone())?
    };
    #[cfg(not(feature = "d3d12-present-hook-fixture"))]
    let mut fixture = {
        if present_hook {
            return Err("owned Present hook feature not enabled".into());
        }
        D3d12PresentFixture::new_gpu(warp, 318, 178, budget.clone())?
    };
    fixture.cover_selected()?;
    let hello = fixture.export_remote()?;
    let (out, messages) = sync_channel(8);
    let writer = std::thread::spawn(move || {
        let mut stdout = std::io::stdout().lock();
        for message in messages {
            write_wire(&mut stdout, message)?;
        }
        Ok::<_, String>(())
    });
    let (commands, input) = sync_channel(4);
    let _reader = std::thread::spawn(move || {
        let mut stdin = std::io::stdin().lock();
        loop {
            let message = read_wire(&mut stdin);
            let failed = message.is_err();
            if commands.send(message).is_err() || failed {
                break;
            }
        }
    });
    let emit = |wire| {
        out.try_send(wire)
            .map_err(|e| format!("bounded producer output unavailable: {e}"))
    };
    emit(Wire::Hello(hello))?;
    match input
        .recv_timeout(Duration::from_secs(10))
        .map_err(|e| e.to_string())??
    {
        Wire::Control(Message::Select(session)) if session == hello.session => {}
        _ => return Err("D3D12 source approval mismatch".into()),
    }
    let deadline = Instant::now()
        + Duration::from_secs(if fault == Some("remove-and-recreate-on-command") {
            175
        } else {
            30
        });
    let mut next = Instant::now();
    let mut published = 0;
    let mut pending = [None; 3];
    #[cfg(feature = "d3d12-device-replace-fixture")]
    let mut producer_lost = false;
    #[cfg(not(feature = "d3d12-device-replace-fixture"))]
    let producer_lost = false;
    let result = (move || {
        let mut fixture = fixture;
        #[cfg(feature = "d3d12-device-replace-fixture")]
        let mut hello = hello;
        #[cfg(not(feature = "d3d12-device-replace-fixture"))]
        let hello = hello;
        loop {
            if Instant::now() >= deadline {
                return Err("owned D3D12 producer session deadline".into());
            }
            match input.try_recv() {
                Ok(Ok(Wire::Control(Message::Ack {
                    session,
                    slot,
                    sequence,
                }))) => {
                    clear_owned_pending_ack(&mut pending, hello.session, session, slot, sequence)?;
                    if !producer_lost {
                        fixture.acknowledge_remote(session, slot, sequence)?;
                    }
                }
                Ok(Ok(Wire::TriggerOwnedLoss(session))) => {
                    if !loss_probe || producer_lost || session != hello.session {
                        return Err(
                            "unannounced loss trigger requires one exact fixture opt-in/session"
                                .into(),
                        );
                    }
                    #[cfg(feature = "d3d12-device-replace-fixture")]
                    {
                        if recover_loss && !fixture.remote_epoch_primed()? {
                            return Err("automatic owned loss reconstruction requires a prior exact remote ACK; no removal attempted".into());
                        }
                        let reason = fixture.trigger_owned_device_loss()?;
                        producer_lost = true;
                        let notice = crate::d3d12_protocol::DeviceLossNotice {
                            session: hello.session,
                            reason,
                        };
                        emit(Wire::DeviceLost(notice))?;
                        if recover_loss {
                            let mut loss = crate::device_loss_epoch::DeviceLossEpoch::new(hello)
                                .map_err(|error| {
                                    format!("owned loss epoch initialization: {error:?}")
                                })?;
                            let next_epoch = loss
                                .begin(notice)
                                .map_err(|error| format!("owned loss begin: {error:?}"))?;
                            let until = Instant::now() + Duration::from_secs(5);
                            loop {
                                if Instant::now() >= until {
                                    return Err("owned loss retirement proof deadline".into());
                                }
                                fixture.pump_paused_windows()?;
                                match input.recv_timeout(Duration::from_millis(25)) {
                                    Ok(Ok(Wire::Retired(session))) if session == next_epoch => {
                                        break
                                    }
                                    Ok(Ok(Wire::Control(Message::Ack {
                                        session,
                                        slot,
                                        sequence,
                                    }))) => clear_owned_pending_ack(
                                        &mut pending,
                                        hello.session,
                                        session,
                                        slot,
                                        sequence,
                                    )?,
                                    Ok(Ok(Wire::Control(Message::Stop))) => {
                                        emit(Wire::Control(Message::Bye))?;
                                        return Ok(());
                                    }
                                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                                    other => {
                                        return Err(format!(
                                            "unexpected owned loss retirement packet: {other:?}"
                                        ))
                                    }
                                }
                            }
                            loss.retired(next_epoch)
                                .map_err(|error| format!("owned loss retirement: {error:?}"))?;
                            let (width, height) = if next_epoch.epoch.is_multiple_of(2) {
                                (640, 360)
                            } else {
                                (800, 450)
                            };
                            fixture = fixture.reconstruct_remote_loss(
                                hello.session,
                                width,
                                height,
                                budget.clone(),
                            )?;
                            let replacement = fixture.export_remote_epoch(next_epoch.epoch)?;
                            loss.offer(replacement)
                                .map_err(|error| format!("owned loss offer: {error:?}"))?;
                            emit(Wire::Hello(replacement))?;
                            let until = Instant::now() + Duration::from_secs(5);
                            loop {
                                if Instant::now() >= until {
                                    return Err("owned loss approval deadline".into());
                                }
                                fixture.pump_paused_windows()?;
                                match input.recv_timeout(Duration::from_millis(25)) {
                                    Ok(Ok(Wire::Control(Message::Select(session))))
                                        if session == next_epoch =>
                                    {
                                        break
                                    }
                                    Ok(Ok(Wire::Control(Message::Stop))) => {
                                        emit(Wire::Control(Message::Bye))?;
                                        return Ok(());
                                    }
                                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                                    other => {
                                        return Err(format!(
                                            "unexpected owned loss approval packet: {other:?}"
                                        ))
                                    }
                                }
                            }
                            hello = loss
                                .approve(next_epoch)
                                .map_err(|error| format!("owned loss approval: {error:?}"))?;
                            producer_lost = false;
                            pending.fill(None);
                        }
                        continue;
                    }
                    #[cfg(not(feature = "d3d12-device-replace-fixture"))]
                    return Err("unannounced loss fixture disabled".into());
                }
                Ok(Ok(Wire::Control(Message::Render { color, .. }))) => {
                    if color == 2 {
                        #[cfg(feature = "d3d12-factory-hook-fixture")]
                        if let Some((forwarded, observed, claimed, dropped, errors)) =
                            fixture.factory_counts()
                        {
                            if dropped != 0 || errors != 0 {
                                return Err("factory interception diagnostic failed".into());
                            }
                            emit(Wire::Control(Message::Report {
                                presented: forwarded,
                                published: observed,
                                dropped: claimed,
                            }))?;
                            continue;
                        }
                        return Err("factory counters requested from non-factory source".into());
                    }
                    if color == 3 {
                        #[cfg(feature = "d3d12-present-hook-fixture")]
                        if let Some((selected, unrelated, captured, _)) = fixture.hook_counts() {
                            emit(Wire::Control(Message::Report {
                                presented: selected,
                                published: captured,
                                dropped: unrelated,
                            }))?;
                            continue;
                        }
                        return Err("hook counters requested from non-hook source".into());
                    }
                    let stats = fixture.stats();
                    emit(Wire::Control(Message::Report {
                        presented: stats.selected,
                        published,
                        dropped: stats.dropped_capture,
                    }))?;
                }
                Ok(Ok(Wire::Control(Message::Stop))) => {
                    fixture.cancel_capture();
                    let target = fixture.stats().selected + 12;
                    let stop_deadline = Instant::now() + Duration::from_secs(3);
                    while !producer_lost && fixture.stats().selected < target {
                        fixture.step()?;
                        if Instant::now() >= stop_deadline {
                            return Err("renderer stopped after capture cancellation".into());
                        }
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    let stats = fixture.stats();
                    emit(Wire::Control(Message::Report {
                        presented: stats.selected,
                        published,
                        dropped: stats.dropped_capture,
                    }))?;
                    emit(Wire::Control(Message::Bye))?;
                    #[cfg(feature = "d3d12-present-hook-fixture")]
                    if let Some(counts) = fixture.hook_counts() {
                        eprintln!("owned D3D12 Present detour selected/unrelated forwards, captured, dropped: {counts:?}; executable forwarder stays resident until process exit");
                    }
                    return Ok(());
                }
                Ok(Ok(Wire::Window(action))) => {
                    let flags = fixture.window_action(action)?;
                    let mut marker = 0;
                    unsafe {
                        windows::Win32::System::Performance::QueryPerformanceCounter(&mut marker)
                    }
                    .map_err(error)?;
                    emit(Wire::Control(Message::Report {
                        presented: action as u64,
                        published: marker as u64,
                        dropped: flags,
                    }))?;
                }
                Ok(Err(error)) => return Err(error),
                Err(TryRecvError::Disconnected) => {
                    return Err("owned D3D12 input disconnected".into())
                }
                Err(TryRecvError::Empty) => {}
                _ => return Err("unexpected owned D3D12 command".into()),
            }
            if producer_lost {
                #[cfg(feature = "d3d12-device-replace-fixture")]
                fixture.pump_paused_windows()?;
                std::thread::sleep(Duration::from_millis(1));
                continue;
            }
            if Instant::now() >= next {
                fixture.step()?;
                next = Instant::now() + Duration::from_millis(8);
            }
            if let Some(mut header) = fixture.poll_remote()? {
                if fault == Some("bad-frame") {
                    header.session.epoch += 1;
                }
                emit(Wire::Control(Message::Frame(header)))?;
                if pending[header.slot as usize]
                    .replace(header.sequence)
                    .is_some()
                {
                    return Err("producer reused an unacknowledged remote slot".into());
                }
                published += 1;
                if fault == Some("exit-after-six") && published >= 6 {
                    return Err("intentional owned-producer exit".into());
                }
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    })();
    drop(out);
    if result.is_ok() {
        writer
            .join()
            .map_err(|_| "D3D12 producer writer panicked")??;
    }
    result
}
