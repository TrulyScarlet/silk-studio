//! Global hotkey registration and conflict handling (Segment S6).
//!
//! Windows registration and message pumping stay on a dedicated thread. The
//! rest of the application receives only bounded, plain-data activations and
//! routes them through the normal controller command path.

use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::thread::JoinHandle;
use std::time::Duration;

use configuration::{normalize_chord, ChordError};
use thiserror::Error;

pub const SAVE_REPLAY_HOTKEY_ID: i32 = 1;
pub const START_CAPTURE_HOTKEY_ID: i32 = 2;
pub const STOP_CAPTURE_HOTKEY_ID: i32 = 3;
pub const HOTKEY_EVENT_QUEUE_BOUND: usize = 64;
const HOTKEY_CONTROL_QUEUE_BOUND: usize = 8;
const HOTKEY_DEBOUNCE: Duration = Duration::from_millis(50);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyAction {
    SaveReplay,
    StartCapture,
    StopCapture,
}

impl HotkeyAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SaveReplay => "save_replay",
            Self::StartCapture => "start_capture",
            Self::StopCapture => "stop_capture",
        }
    }
}

impl std::fmt::Display for HotkeyAction {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotkeyBinding {
    pub id: i32,
    pub action: HotkeyAction,
    pub chord: String,
}

impl HotkeyBinding {
    pub fn new(id: i32, action: HotkeyAction, chord: &str) -> Result<Self, HotkeyError> {
        let chord = normalize_chord(chord).map_err(|source| HotkeyError::InvalidChord {
            chord: chord.to_string(),
            reason: source.to_user_message(),
        })?;
        Ok(Self { id, action, chord })
    }

    pub fn save_replay(chord: &str) -> Result<Self, HotkeyError> {
        Self::new(SAVE_REPLAY_HOTKEY_ID, HotkeyAction::SaveReplay, chord)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeBinding {
    pub id: i32,
    pub action: HotkeyAction,
    pub modifiers: u32,
    pub virtual_key: u16,
}

pub fn to_native_bindings(bindings: &[HotkeyBinding]) -> Vec<NativeBinding> {
    bindings
        .iter()
        .filter_map(|binding| {
            parse_native_chord(&binding.chord)
                .ok()
                .map(|(modifiers, virtual_key)| NativeBinding {
                    id: binding.id,
                    action: binding.action,
                    modifiers,
                    virtual_key,
                })
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotkeyEvent {
    Activated(HotkeyAction),
    RegistrationFailed {
        action: HotkeyAction,
        chord: String,
        os_error: u32,
    },
    EventQueueFull,
    Stopped,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum HotkeyError {
    #[error("invalid hotkey '{chord}': {reason}")]
    InvalidChord { chord: String, reason: String },

    #[error("hotkey id {id} is assigned more than once")]
    DuplicateId { id: i32 },

    #[error("hotkey chord '{chord}' is assigned more than once")]
    DuplicateChord { chord: String },

    #[error("global hotkeys are only supported on Windows")]
    UnsupportedPlatform,

    #[error("hotkey control thread is unavailable")]
    ControlThreadUnavailable,

    #[error(
        "could not register {action} hotkey '{chord}' (Windows error {os_error}); another application may own the chord"
    )]
    RegistrationFailed {
        action: HotkeyAction,
        chord: String,
        os_error: u32,
    },
}

/// Dedicated-thread owner for the process global hotkeys.
pub struct HotkeyManager {
    #[cfg(windows)]
    control: SyncSender<ControlMessage>,
    #[cfg(windows)]
    events: Receiver<HotkeyEvent>,
    #[cfg(windows)]
    thread_id: u32,
    #[cfg(windows)]
    worker: Option<JoinHandle<()>>,
}

impl HotkeyManager {
    pub fn new(bindings: Vec<HotkeyBinding>) -> Result<Self, HotkeyError> {
        validate_bindings(&bindings)?;

        #[cfg(windows)]
        {
            let (control, control_rx) = mpsc::sync_channel(HOTKEY_CONTROL_QUEUE_BOUND);
            let (events, event_rx) = mpsc::sync_channel(HOTKEY_EVENT_QUEUE_BOUND);
            let (ready_tx, ready_rx) = mpsc::sync_channel(1);
            let worker = std::thread::Builder::new()
                .name("silk-hotkey-loop".to_string())
                .spawn(move || run_hotkey_loop(bindings, control_rx, events, ready_tx))
                .map_err(|_| HotkeyError::ControlThreadUnavailable)?;

            match ready_rx.recv() {
                Ok(Ok(thread_id)) => Ok(Self {
                    control,
                    events: event_rx,
                    thread_id,
                    worker: Some(worker),
                }),
                Ok(Err(error)) => {
                    let _ = worker.join();
                    Err(error)
                }
                Err(_) => {
                    let _ = worker.join();
                    Err(HotkeyError::ControlThreadUnavailable)
                }
            }
        }

        #[cfg(not(windows))]
        {
            let _ = bindings;
            Err(HotkeyError::UnsupportedPlatform)
        }
    }

    /// Replace all bindings and wait for the dedicated thread to confirm the
    /// registration. On a conflict, the previous set remains active when the
    /// operating system permits it.
    pub fn set_bindings(&self, bindings: Vec<HotkeyBinding>) -> Result<(), HotkeyError> {
        validate_bindings(&bindings)?;

        #[cfg(windows)]
        {
            const WM_HOTKEY_CONTROL: u32 = windows_sys::Win32::UI::WindowsAndMessaging::WM_APP + 1;
            let (result_tx, result_rx) = mpsc::sync_channel(1);
            self.control
                .try_send(ControlMessage::Replace {
                    bindings,
                    result: result_tx,
                })
                .map_err(|error| match error {
                    TrySendError::Full(_) | TrySendError::Disconnected(_) => {
                        HotkeyError::ControlThreadUnavailable
                    }
                })?;
            unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::PostThreadMessageW(
                    self.thread_id,
                    WM_HOTKEY_CONTROL,
                    0,
                    0,
                );
            }
            result_rx
                .recv()
                .map_err(|_| HotkeyError::ControlThreadUnavailable)?
        }

        #[cfg(not(windows))]
        {
            let _ = bindings;
            Err(HotkeyError::UnsupportedPlatform)
        }
    }

    pub fn drain(&self) -> Vec<HotkeyEvent> {
        #[cfg(windows)]
        {
            self.events.try_iter().collect()
        }

        #[cfg(not(windows))]
        {
            Vec::new()
        }
    }
}

#[cfg(windows)]
enum ControlMessage {
    Replace {
        bindings: Vec<HotkeyBinding>,
        result: SyncSender<Result<(), HotkeyError>>,
    },
    Stop,
}

impl Drop for HotkeyManager {
    fn drop(&mut self) {
        #[cfg(windows)]
        {
            if let Some(worker) = self.worker.take() {
                const WM_HOTKEY_CONTROL: u32 =
                    windows_sys::Win32::UI::WindowsAndMessaging::WM_APP + 1;
                let _ = self.control.send(ControlMessage::Stop);
                unsafe {
                    windows_sys::Win32::UI::WindowsAndMessaging::PostThreadMessageW(
                        self.thread_id,
                        WM_HOTKEY_CONTROL,
                        0,
                        0,
                    );
                }
                let _ = worker.join();
            }
        }
    }
}

/// Validate a complete binding set before it reaches the native registration
/// loop. This is also used by settings code so duplicate chords fail before
/// any existing registration is touched.
pub fn validate_bindings(bindings: &[HotkeyBinding]) -> Result<(), HotkeyError> {
    let mut ids = std::collections::BTreeSet::new();
    let mut chords = std::collections::BTreeSet::new();
    for binding in bindings {
        if !ids.insert(binding.id) {
            return Err(HotkeyError::DuplicateId { id: binding.id });
        }
        if !chords.insert(binding.chord.clone()) {
            return Err(HotkeyError::DuplicateChord {
                chord: binding.chord.clone(),
            });
        }
        parse_native_chord(&binding.chord)?;
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReplacementPlan {
    retained: Vec<HotkeyBinding>,
    removed: Vec<HotkeyBinding>,
    added: Vec<HotkeyBinding>,
}

fn plan_replacement(current: &[HotkeyBinding], requested: &[HotkeyBinding]) -> ReplacementPlan {
    let retained: Vec<_> = current
        .iter()
        .filter(|binding| requested.contains(binding))
        .cloned()
        .collect();
    let removed: Vec<_> = current
        .iter()
        .filter(|binding| !retained.contains(binding))
        .cloned()
        .collect();
    let added: Vec<_> = requested
        .iter()
        .filter(|binding| !retained.contains(binding))
        .cloned()
        .collect();

    ReplacementPlan {
        retained,
        removed,
        added,
    }
}

/// Apply a replacement transaction against an abstract registration surface.
/// The Windows worker uses this to keep the rollback behavior deterministic and
/// testable without requiring a second process to own a conflicting chord.
#[cfg(test)]
fn apply_replacement<Register, Unregister>(
    current: &[HotkeyBinding],
    requested: &[HotkeyBinding],
    mut unregister: Unregister,
    mut register: Register,
) -> (Vec<HotkeyBinding>, Result<(), HotkeyError>)
where
    Register: FnMut(&[HotkeyBinding]) -> Result<(), HotkeyError>,
    Unregister: FnMut(&[HotkeyBinding]),
{
    let plan = plan_replacement(current, requested);
    unregister(&plan.removed);

    match register(&plan.added) {
        Ok(()) => (requested.to_vec(), Ok(())),
        Err(error) => {
            if register(&plan.removed).is_ok() {
                (current.to_vec(), Err(error))
            } else {
                // The unchanged registrations were never removed. Do not
                // claim that bindings which could not be restored are active.
                (plan.retained, Err(error))
            }
        }
    }
}

pub fn parse_native_chord(chord: &str) -> Result<(u32, u16), HotkeyError> {
    let normalized = normalize_chord(chord).map_err(|source| HotkeyError::InvalidChord {
        chord: chord.to_string(),
        reason: source.to_user_message(),
    })?;
    let mut modifiers = 0_u32;
    let mut key = None;
    for token in normalized.split('+') {
        match token {
            "CTRL" => modifiers |= 0x0002,
            "ALT" => modifiers |= 0x0001,
            "SHIFT" => modifiers |= 0x0004,
            "WIN" => modifiers |= 0x0008,
            _ => key = Some(token),
        }
    }
    let key = key.ok_or_else(|| HotkeyError::InvalidChord {
        chord: chord.to_string(),
        reason: ChordError::MissingKey.to_user_message(),
    })?;
    let virtual_key = virtual_key_code(key).ok_or_else(|| HotkeyError::InvalidChord {
        chord: chord.to_string(),
        reason: format!("'{key}' is not supported by the Windows hotkey bridge"),
    })?;
    Ok((modifiers, virtual_key))
}

fn virtual_key_code(key: &str) -> Option<u16> {
    if key.len() == 1 {
        let byte = key.as_bytes()[0];
        if byte.is_ascii_uppercase() || byte.is_ascii_digit() {
            return Some(u16::from(byte));
        }
    }
    if let Some(number) = key
        .strip_prefix('F')
        .and_then(|value| value.parse::<u16>().ok())
    {
        if (1..=24).contains(&number) {
            return Some(0x6F + number);
        }
    }
    match key {
        "UP" => Some(0x26),
        "DOWN" => Some(0x28),
        "LEFT" => Some(0x25),
        "RIGHT" => Some(0x27),
        "SPACE" => Some(0x20),
        "TAB" => Some(0x09),
        "ENTER" => Some(0x0D),
        "ESC" => Some(0x1B),
        _ => None,
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ModifierState {
    pub lctrl: bool,
    pub rctrl: bool,
    pub lshift: bool,
    pub rshift: bool,
    pub lalt: bool,
    pub ralt: bool,
    pub lwin: bool,
    pub rwin: bool,
}

impl ModifierState {
    pub fn ctrl(&self) -> bool {
        self.lctrl || self.rctrl
    }

    pub fn shift(&self) -> bool {
        self.lshift || self.rshift
    }

    pub fn alt(&self) -> bool {
        self.lalt || self.ralt
    }

    pub fn win(&self) -> bool {
        self.lwin || self.rwin
    }

    pub fn mask(&self) -> u32 {
        let mut mask = 0;
        if self.alt() {
            mask |= 0x0001;
        }
        if self.ctrl() {
            mask |= 0x0002;
        }
        if self.shift() {
            mask |= 0x0004;
        }
        if self.win() {
            mask |= 0x0008;
        }
        mask
    }

    /// Updates modifier state and returns `true` if `vk_code` is a modifier key.
    pub fn update(&mut self, vk_code: u16, is_down: bool, alt_flag: bool) -> bool {
        match vk_code {
            0x11 => {
                self.lctrl = is_down;
                self.rctrl = is_down;
                true
            }
            0xA2 => {
                self.lctrl = is_down;
                true
            }
            0xA3 => {
                self.rctrl = is_down;
                true
            }
            0x10 => {
                self.lshift = is_down;
                self.rshift = is_down;
                true
            }
            0xA0 => {
                self.lshift = is_down;
                true
            }
            0xA1 => {
                self.rshift = is_down;
                true
            }
            0x12 => {
                self.lalt = is_down;
                self.ralt = is_down;
                true
            }
            0xA4 => {
                self.lalt = is_down;
                true
            }
            0xA5 => {
                self.ralt = is_down;
                true
            }
            0x5B => {
                self.lwin = is_down;
                true
            }
            0x5C => {
                self.rwin = is_down;
                true
            }
            _ => {
                if alt_flag {
                    if !self.alt() {
                        self.lalt = true;
                    }
                } else {
                    self.lalt = false;
                    self.ralt = false;
                }
                false
            }
        }
    }
}

pub struct HookDispatchState {
    pub bindings: Vec<NativeBinding>,
    pub events: SyncSender<HotkeyEvent>,
    pub modifiers: ModifierState,
    pub last_activation: std::collections::BTreeMap<i32, std::time::Instant>,
}

impl HookDispatchState {
    pub fn handle_event(&mut self, vk_code: u16, is_down: bool, alt_flag: bool) {
        let is_mod = self.modifiers.update(vk_code, is_down, alt_flag);
        if !is_down || is_mod {
            return;
        }

        let current_mods = self.modifiers.mask();
        for binding in &self.bindings {
            if binding.virtual_key == vk_code && binding.modifiers == current_mods {
                let now = std::time::Instant::now();
                if let Some(previous) = self.last_activation.get(&binding.id) {
                    if now.duration_since(*previous) < HOTKEY_DEBOUNCE {
                        continue;
                    }
                }
                self.last_activation.insert(binding.id, now);
                if self
                    .events
                    .try_send(HotkeyEvent::Activated(binding.action))
                    .is_err()
                {
                    let _ = self.events.try_send(HotkeyEvent::EventQueueFull);
                }
            }
        }
    }
}

#[cfg(windows)]
static HOOK_DISPATCH: std::sync::Mutex<Option<HookDispatchState>> = std::sync::Mutex::new(None);

#[cfg(windows)]
unsafe extern "system" fn low_level_keyboard_proc(
    n_code: i32,
    w_param: windows_sys::Win32::Foundation::WPARAM,
    l_param: windows_sys::Win32::Foundation::LPARAM,
) -> windows_sys::Win32::Foundation::LRESULT {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, KBDLLHOOKSTRUCT, LLKHF_ALTDOWN, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN,
        WM_SYSKEYUP,
    };

    if n_code >= 0 {
        let msg = w_param as u32;
        let is_down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
        let is_up = msg == WM_KEYUP || msg == WM_SYSKEYUP;

        if is_down || is_up {
            let kbd = &*(l_param as *const KBDLLHOOKSTRUCT);
            let vk_code = kbd.vkCode as u16;
            let alt_flag = (kbd.flags & LLKHF_ALTDOWN) != 0;

            if let Ok(mut guard) = HOOK_DISPATCH.lock() {
                if let Some(dispatch) = guard.as_mut() {
                    dispatch.handle_event(vk_code, is_down, alt_flag);
                }
            }
        }
    }
    CallNextHookEx(std::ptr::null_mut(), n_code, w_param, l_param)
}

#[cfg(windows)]
fn run_hotkey_loop(
    initial: Vec<HotkeyBinding>,
    control_rx: Receiver<ControlMessage>,
    events: SyncSender<HotkeyEvent>,
    ready: SyncSender<Result<u32, HotkeyError>>,
) {
    use std::collections::{BTreeMap, BTreeSet};
    use std::ptr::null_mut;
    use std::time::Instant;
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        RegisterHotKey, UnregisterHotKey, MOD_NOREPEAT,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetMessageW, PeekMessageW, SetWindowsHookExW, UnhookWindowsHookEx, MSG, PM_NOREMOVE,
        WH_KEYBOARD_LL, WM_APP, WM_HOTKEY,
    };

    let mut message = MSG {
        hwnd: null_mut(),
        message: 0,
        wParam: 0,
        lParam: 0,
        time: 0,
        pt: windows_sys::Win32::Foundation::POINT { x: 0, y: 0 },
    };

    // 1. Force the Win32 subsystem to create the thread message queue before readiness.
    unsafe {
        PeekMessageW(&mut message, null_mut(), 0, 0, PM_NOREMOVE);
    }
    let thread_id = unsafe { GetCurrentThreadId() };

    // 2. Obtain the executable module handle.
    let hinstance = unsafe { GetModuleHandleW(std::ptr::null()) };

    // 3. Register complementary hotkeys.
    let mut succeeded_hotkey_ids = BTreeSet::new();
    for binding in &initial {
        if let Ok((modifiers, virtual_key)) = parse_native_chord(&binding.chord) {
            let success = unsafe {
                RegisterHotKey(
                    null_mut(),
                    binding.id,
                    modifiers | MOD_NOREPEAT,
                    u32::from(virtual_key),
                )
            };
            if success != 0 {
                succeeded_hotkey_ids.insert(binding.id);
            }
        }
    }

    let mut current_bindings = initial;
    let native_bindings = to_native_bindings(&current_bindings);

    // 4. Populate the global hook dispatch state.
    if let Ok(mut guard) = HOOK_DISPATCH.lock() {
        *guard = Some(HookDispatchState {
            bindings: native_bindings,
            events: events.clone(),
            modifiers: ModifierState::default(),
            last_activation: BTreeMap::new(),
        });
    }

    // 5. Install WH_KEYBOARD_LL hook.
    let hook =
        unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(low_level_keyboard_proc), hinstance, 0) };

    if hook.is_null() {
        let os_error = unsafe { windows_sys::Win32::Foundation::GetLastError() };
        if let Ok(mut guard) = HOOK_DISPATCH.lock() {
            *guard = None;
        }
        for id in succeeded_hotkey_ids {
            unsafe {
                UnregisterHotKey(null_mut(), id);
            }
        }
        let _ = ready.send(Err(HotkeyError::RegistrationFailed {
            action: current_bindings
                .first()
                .map(|b| b.action)
                .unwrap_or(HotkeyAction::SaveReplay),
            chord: current_bindings
                .first()
                .map(|b| b.chord.clone())
                .unwrap_or_default(),
            os_error,
        }));
        return;
    }

    // 6. Signal manager ready with operational thread ID.
    if ready.send(Ok(thread_id)).is_err() {
        unsafe {
            UnhookWindowsHookEx(hook);
        }
        if let Ok(mut guard) = HOOK_DISPATCH.lock() {
            *guard = None;
        }
        for id in succeeded_hotkey_ids {
            unsafe {
                UnregisterHotKey(null_mut(), id);
            }
        }
        return;
    }

    const WM_HOTKEY_CONTROL: u32 = WM_APP + 1;

    'running: loop {
        let ret = unsafe { GetMessageW(&mut message, null_mut(), 0, 0) };
        if ret <= 0 {
            break 'running;
        }

        if message.message == WM_HOTKEY_CONTROL {
            while let Ok(command) = control_rx.try_recv() {
                match command {
                    ControlMessage::Replace { bindings, result } => {
                        let plan = plan_replacement(&current_bindings, &bindings);
                        for b in &plan.removed {
                            if succeeded_hotkey_ids.remove(&b.id) {
                                unsafe {
                                    UnregisterHotKey(null_mut(), b.id);
                                }
                            }
                        }
                        for b in &plan.added {
                            if let Ok((modifiers, virtual_key)) = parse_native_chord(&b.chord) {
                                let success = unsafe {
                                    RegisterHotKey(
                                        null_mut(),
                                        b.id,
                                        modifiers | MOD_NOREPEAT,
                                        u32::from(virtual_key),
                                    )
                                };
                                if success != 0 {
                                    succeeded_hotkey_ids.insert(b.id);
                                }
                            }
                        }
                        current_bindings = bindings;
                        if let Ok(mut guard) = HOOK_DISPATCH.lock() {
                            if let Some(dispatch) = guard.as_mut() {
                                dispatch.bindings = to_native_bindings(&current_bindings);
                            }
                        }
                        let _ = result.send(Ok(()));
                    }
                    ControlMessage::Stop => break 'running,
                }
            }
        } else if message.message == WM_HOTKEY {
            let id = message.wParam as i32;
            let now = Instant::now();
            let mut activated_action = None;
            if let Ok(mut guard) = HOOK_DISPATCH.lock() {
                if let Some(dispatch) = guard.as_mut() {
                    if let Some(binding) = dispatch.bindings.iter().find(|b| b.id == id) {
                        if let Some(previous) = dispatch.last_activation.get(&id) {
                            if now.duration_since(*previous) < HOTKEY_DEBOUNCE {
                                continue;
                            }
                        }
                        dispatch.last_activation.insert(id, now);
                        activated_action = Some(binding.action);
                    }
                }
            }
            if let Some(action) = activated_action {
                if events.try_send(HotkeyEvent::Activated(action)).is_err() {
                    let _ = events.try_send(HotkeyEvent::EventQueueFull);
                }
            }
        }
    }

    unsafe {
        UnhookWindowsHookEx(hook);
    }
    if let Ok(mut guard) = HOOK_DISPATCH.lock() {
        *guard = None;
    }
    for id in succeeded_hotkey_ids {
        unsafe {
            UnregisterHotKey(null_mut(), id);
        }
    }
    let _ = events.try_send(HotkeyEvent::Stopped);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_and_maps_supported_chords() {
        let binding = HotkeyBinding::save_replay("shift+ctrl+f10").expect("valid chord");
        assert_eq!(binding.chord, "CTRL+SHIFT+F10");
        assert_eq!(parse_native_chord(&binding.chord), Ok((0x0006, 0x79)));
    }

    #[test]
    fn rejects_duplicate_ids_and_bad_chords() {
        let first = HotkeyBinding::save_replay("Ctrl+F10").expect("valid chord");
        let duplicate =
            HotkeyBinding::new(1, HotkeyAction::StartCapture, "Alt+F9").expect("valid chord");
        assert_eq!(
            validate_bindings(&[first, duplicate]),
            Err(HotkeyError::DuplicateId { id: 1 })
        );
        assert!(matches!(
            HotkeyBinding::save_replay("Ctrl+Hyper+F10"),
            Err(HotkeyError::InvalidChord { .. })
        ));
    }

    #[test]
    fn rejects_duplicate_chords_before_registration() {
        let first = HotkeyBinding::save_replay("Ctrl+Shift+F10").expect("valid chord");
        let duplicate = HotkeyBinding::new(2, HotkeyAction::StartCapture, "shift+ctrl+f10")
            .expect("valid chord");
        assert_eq!(
            validate_bindings(&[first, duplicate]),
            Err(HotkeyError::DuplicateChord {
                chord: "CTRL+SHIFT+F10".to_string(),
            })
        );
    }

    #[test]
    fn failed_replacement_restores_previous_bindings() {
        let current = vec![
            HotkeyBinding::save_replay("Ctrl+F10").expect("valid chord"),
            HotkeyBinding::new(2, HotkeyAction::StartCapture, "Ctrl+F9").expect("valid chord"),
        ];
        let requested = vec![
            current[0].clone(),
            HotkeyBinding::new(2, HotkeyAction::StartCapture, "Ctrl+F8").expect("valid chord"),
        ];
        let mut unregistered = Vec::new();
        let mut registered = Vec::new();
        let mut attempts = 0_u8;

        let (active, result) = apply_replacement(
            &current,
            &requested,
            |bindings| unregistered.extend_from_slice(bindings),
            |bindings| {
                attempts += 1;
                registered.push(bindings.to_vec());
                if attempts == 1 {
                    Err(HotkeyError::RegistrationFailed {
                        action: HotkeyAction::StartCapture,
                        chord: "CTRL+F8".to_string(),
                        os_error: 1409,
                    })
                } else {
                    Ok(())
                }
            },
        );

        assert!(result.is_err());
        assert_eq!(active, current);
        assert_eq!(unregistered, vec![current[1].clone()]);
        assert_eq!(registered[0], vec![requested[1].clone()]);
        assert_eq!(registered[1], vec![current[1].clone()]);
    }

    #[test]
    fn sanitizes_key_mapping_boundaries() {
        assert_eq!(virtual_key_code("A"), Some(0x41));
        assert_eq!(virtual_key_code("0"), Some(0x30));
        assert_eq!(virtual_key_code("F1"), Some(0x70));
        assert_eq!(virtual_key_code("F24"), Some(0x87));
        assert_eq!(virtual_key_code("F25"), None);
        assert_eq!(virtual_key_code("PAGEUP"), None);
    }

    #[test]
    fn modifier_state_tracks_transitions_and_masks() {
        let mut state = ModifierState::default();
        assert_eq!(state.mask(), 0);

        // Alt down via VK_MENU
        assert!(state.update(0x12, true, false));
        assert_eq!(state.mask(), 0x0001); // ALT

        // Non-modifier 'S' down with alt_flag
        assert!(!state.update(0x53, true, true));
        assert_eq!(state.mask(), 0x0001);

        // Non-modifier 'S' up with alt_flag
        assert!(!state.update(0x53, false, true));
        assert_eq!(state.mask(), 0x0001);

        // Alt up via VK_MENU
        assert!(state.update(0x12, false, false));
        assert_eq!(state.mask(), 0);

        // Ctrl + Shift
        assert!(state.update(0xA2, true, false)); // LCtrl
        assert_eq!(state.mask(), 0x0002);
        assert!(state.update(0xA0, true, false)); // LShift
        assert_eq!(state.mask(), 0x0006); // Ctrl + Shift

        // Release LCtrl
        assert!(state.update(0xA2, false, false));
        assert_eq!(state.mask(), 0x0004); // Shift

        // Win down
        assert!(state.update(0x5B, true, false)); // LWin
        assert_eq!(state.mask(), 0x000C); // Shift + Win
    }

    #[test]
    fn hook_dispatch_state_matches_exact_chord_and_debounces() {
        let (tx, rx) = mpsc::sync_channel(16);
        let binding = HotkeyBinding::save_replay("Alt+S").expect("valid");
        let native = to_native_bindings(&[binding]);

        let mut dispatch = HookDispatchState {
            bindings: native,
            events: tx,
            modifiers: ModifierState::default(),
            last_activation: std::collections::BTreeMap::new(),
        };

        // Alt down
        dispatch.handle_event(0x12, true, false);
        assert!(rx.try_recv().is_err());

        // 'S' down with Alt flag
        dispatch.handle_event(0x53, true, true);
        assert_eq!(
            rx.try_recv(),
            Ok(HotkeyEvent::Activated(HotkeyAction::SaveReplay))
        );

        // Immediate duplicate 'S' down (within debounce)
        dispatch.handle_event(0x53, true, true);
        assert!(rx.try_recv().is_err());

        // 'S' up
        dispatch.handle_event(0x53, false, true);
        assert!(rx.try_recv().is_err());

        // Alt up
        dispatch.handle_event(0x12, false, false);
        assert!(rx.try_recv().is_err());

        // 'S' down without Alt
        dispatch.handle_event(0x53, true, false);
        assert!(rx.try_recv().is_err());
    }
}
