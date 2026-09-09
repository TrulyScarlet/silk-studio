#![cfg(windows)]

use hotkeys::{HotkeyAction, HotkeyBinding, HotkeyEvent, HotkeyManager};
use std::time::{Duration, Instant};

#[test]
#[ignore = "requires interactive desktop / Windows UI session"]
fn native_hotkey_manager_detects_injected_alt_s() {
    let binding = HotkeyBinding::save_replay("Alt+S").expect("valid Alt+S binding");
    let manager = HotkeyManager::new(vec![binding]).expect("manager starts with hook");

    // Give hook loop a moment to establish
    std::thread::sleep(Duration::from_millis(50));

    // Inject Alt+S using SendInput
    inject_key_event(0x12, false); // Alt down (VK_MENU)
    std::thread::sleep(Duration::from_millis(20));
    inject_key_event(0x53, false); // S down ('S')
    std::thread::sleep(Duration::from_millis(20));
    inject_key_event(0x53, true); // S up
    std::thread::sleep(Duration::from_millis(20));
    inject_key_event(0x12, true); // Alt up

    // Poll events from manager
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut found = false;
    while Instant::now() < deadline {
        let events = manager.drain();
        if events.contains(&HotkeyEvent::Activated(HotkeyAction::SaveReplay)) {
            found = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    assert!(
        found,
        "expected HotkeyEvent::Activated(SaveReplay) from injected Alt+S"
    );
}

fn inject_key_event(vk_code: u16, key_up: bool) {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
    };

    let flags = if key_up { KEYEVENTF_KEYUP } else { 0 };
    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk_code,
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };

    unsafe {
        SendInput(1, &input, std::mem::size_of::<INPUT>() as i32);
    }
}
