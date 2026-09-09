//! Tauri system-tray controls and window lifecycle policy.

use app_controller::{Command, ControllerEvent};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{App, AppHandle, Manager, Runtime, Window, WindowEvent};

use crate::shell::{self, CloseAction};
use crate::TauriEventSink;

const TRAY_ID: &str = "main";
const MENU_STATUS: &str = "status";
const MENU_SHOW: &str = "show";
const MENU_START: &str = "start";
const MENU_STOP: &str = "stop";
const MENU_SAVE: &str = "save";
const MENU_QUIT: &str = "quit";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MenuState {
    start_enabled: bool,
    stop_enabled: bool,
    save_enabled: bool,
}

pub(crate) fn setup<R: Runtime>(app: &mut App<R>) -> tauri::Result<()> {
    let menu = build_menu(app, "Stopped")?;
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .tooltip("Silk - Stopped")
        .show_menu_on_left_click(false)
        .on_menu_event(handle_menu_event)
        .on_tray_icon_event(handle_tray_event);

    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

pub(crate) fn update_from_event<R: Runtime>(app: &AppHandle<R>, event: &ControllerEvent) {
    let ControllerEvent::StatusChanged { state } = event else {
        return;
    };
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };

    if let Err(error) = tray.set_tooltip(Some(format!("Silk - {state}"))) {
        diagnostics::warn("tray", &format!("could not update tray tooltip: {error}"));
    }
    match build_menu(app, state).and_then(|menu| tray.set_menu(Some(menu))) {
        Ok(()) => {}
        Err(error) => diagnostics::warn("tray", &format!("could not update tray menu: {error}")),
    }
}

pub(crate) fn handle_window_event<R: Runtime>(window: &Window<R>, event: &WindowEvent) {
    let WindowEvent::CloseRequested { api, .. } = event else {
        return;
    };
    if window.label() != "main" {
        api.prevent_close();
        if let Err(error) = window.hide() {
            diagnostics::warn("tray", &format!("could not hide auxiliary window: {error}"));
        }
        return;
    }
    let state = window.app_handle().state::<shell::ShellState>();
    if state.close_action() != CloseAction::Hide {
        return;
    }

    api.prevent_close();
    if let Err(error) = window.hide() {
        diagnostics::warn("tray", &format!("could not hide window on close: {error}"));
    }
}

fn build_menu<R: Runtime, M: Manager<R>>(manager: &M, state: &str) -> tauri::Result<Menu<R>> {
    let controls = menu_state(state);
    let status = MenuItem::with_id(
        manager,
        MENU_STATUS,
        format!("Status: {state}"),
        false,
        None::<&str>,
    )?;
    let show = MenuItem::with_id(manager, MENU_SHOW, "Open Silk", true, None::<&str>)?;
    let start = MenuItem::with_id(
        manager,
        MENU_START,
        "Start Capture",
        controls.start_enabled,
        None::<&str>,
    )?;
    let stop = MenuItem::with_id(
        manager,
        MENU_STOP,
        "Stop Capture",
        controls.stop_enabled,
        None::<&str>,
    )?;
    let save = MenuItem::with_id(
        manager,
        MENU_SAVE,
        "Save Replay",
        controls.save_enabled,
        None::<&str>,
    )?;
    let separator = PredefinedMenuItem::separator(manager)?;
    let quit = MenuItem::with_id(manager, MENU_QUIT, "Quit Silk", true, None::<&str>)?;
    Menu::with_items(
        manager,
        &[&status, &show, &start, &stop, &save, &separator, &quit],
    )
}

fn menu_state(state: &str) -> MenuState {
    MenuState {
        start_enabled: state == "Stopped",
        stop_enabled: state != "Stopped",
        save_enabled: state == "Ready",
    }
}

fn handle_menu_event<R: Runtime>(app: &AppHandle<R>, event: tauri::menu::MenuEvent) {
    match event.id().as_ref() {
        MENU_SHOW => show_main_window(app),
        MENU_START => dispatch_command(app, Command::StartCapture),
        MENU_STOP => dispatch_command(app, Command::StopCapture),
        MENU_SAVE => dispatch_command(app, Command::SaveReplay),
        MENU_QUIT => quit(app),
        _ => {}
    }
}

fn handle_tray_event<R: Runtime>(tray: &tauri::tray::TrayIcon<R>, event: TrayIconEvent) {
    if matches!(
        event,
        TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        }
    ) {
        show_main_window(tray.app_handle());
    }
}

fn dispatch_command<R: Runtime>(app: &AppHandle<R>, command: Command) {
    let state = app.state::<shell::ShellState>();
    let sink = TauriEventSink(app.clone());
    if let Err(error) = shell::execute_command(&state, &sink, &command) {
        diagnostics::error("tray", &format!("tray command failed: {error}"));
    }
}

fn show_main_window<R: Runtime>(app: &AppHandle<R>) {
    let Some(window) = app.get_webview_window("main") else {
        diagnostics::warn("tray", "main window is unavailable");
        return;
    };
    for result in [window.unminimize(), window.show(), window.set_focus()] {
        if let Err(error) = result {
            diagnostics::warn("tray", &format!("could not restore main window: {error}"));
        }
    }
}

fn quit<R: Runtime>(app: &AppHandle<R>) {
    let state = app.state::<shell::ShellState>();
    state.request_exit();
    dispatch_command(app, Command::StopCapture);
    app.exit(0);
}

#[cfg(test)]
mod tests {
    use super::{menu_state, MenuState};

    #[test]
    fn tray_menu_follows_recorder_state() {
        assert_eq!(
            menu_state("Stopped"),
            MenuState {
                start_enabled: true,
                stop_enabled: false,
                save_enabled: false,
            }
        );
        assert_eq!(
            menu_state("Buffering"),
            MenuState {
                start_enabled: false,
                stop_enabled: true,
                save_enabled: false,
            }
        );
        assert_eq!(
            menu_state("Ready"),
            MenuState {
                start_enabled: false,
                stop_enabled: true,
                save_enabled: true,
            }
        );
        assert_eq!(
            menu_state("Error"),
            MenuState {
                start_enabled: false,
                stop_enabled: true,
                save_enabled: false,
            }
        );
    }
}
