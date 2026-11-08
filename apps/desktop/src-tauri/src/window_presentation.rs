//! Keep maximized work-area sizing out of borderless fullscreen transitions.
use std::sync::Mutex;
use tauri::{Manager, WebviewWindow};

#[derive(Default)]
pub struct FullscreenState(Mutex<Option<bool>>);

/// Called on the window thread so unmaximize, monitor selection, and fullscreen
/// run in order before the IPC reply tells the player the transition is finished.
pub fn set_fullscreen(window: &WebviewWindow, fullscreen: bool) -> Result<(), String> {
    let state = window.state::<FullscreenState>();
    let mut restore_maximized = state.0.lock().map_err(|error| error.to_string())?;
    if window.is_fullscreen().map_err(|error| error.to_string())? == fullscreen {
        return Ok(());
    }
    if fullscreen {
        let was_maximized = window.is_maximized().map_err(|error| error.to_string())?;
        let monitor = window
            .current_monitor()
            .map_err(|error| error.to_string())?;
        if was_maximized {
            window.unmaximize().map_err(|error| error.to_string())?;
            // Restoring a maximized window can return its normal bounds to a
            // different monitor. Enter fullscreen on the monitor the user chose.
            if let Some(monitor) = monitor {
                let restored = window
                    .current_monitor()
                    .map_err(|error| error.to_string())?;
                if restored.as_ref().map(|value| value.position()) != Some(monitor.position()) {
                    window
                        .set_position(*monitor.position())
                        .map_err(|error| error.to_string())?;
                }
            }
        }
        if let Err(error) = window.set_fullscreen(true) {
            if was_maximized {
                let _ = window.maximize();
            }
            return Err(error.to_string());
        }
        *restore_maximized = Some(was_maximized);
    } else {
        window
            .set_fullscreen(false)
            .map_err(|error| error.to_string())?;
        if restore_maximized.take() == Some(true) {
            window.maximize().map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}
