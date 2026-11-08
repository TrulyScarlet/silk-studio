//! Dynamic theme-reactive Windows taskbar and system tray icon generator.
//!
//! Renders the Silk pixel art circular replay arrows with a crisp black outline
//! and theme-reactive arrow fill:
//! - Studio: neutral white on charcoal, matching the default app theme
//! - Classic: Butter Yellow (`#ffffb3`)
//! - Autumn Ember: Amber (`#ff9a3c`)
//! - Vamp: Crimson Red (`#fc7171`)

use configuration::Theme;
use tauri::image::Image;
use tauri::{AppHandle, Manager, Runtime};

#[cfg(windows)]
#[derive(Default)]
struct TaskbarIcon(std::sync::Mutex<TaskbarIconState>);

#[cfg(windows)]
#[derive(Default)]
struct TaskbarIconState {
    handle: isize,
    theme: Option<Theme>,
}

#[cfg(windows)]
impl Drop for TaskbarIcon {
    fn drop(&mut self) {
        if let Ok(state) = self.0.get_mut() {
            if state.handle != 0 {
                // SAFETY: this state owns icons loaded without LR_SHARED.
                unsafe {
                    windows_sys::Win32::UI::WindowsAndMessaging::DestroyIcon(state.handle as _);
                }
            }
        }
    }
}

const N: usize = 32;

#[cfg(windows)]
fn taskbar_theme_id(base: &str, theme: Theme) -> String {
    let name = match theme {
        Theme::Studio => "studio",
        Theme::Classic => "classic",
        Theme::Ember => "ember",
        Theme::Vamp => "vamp",
    };
    // Explorer caches the group image by AppUserModelID. Each of these four
    // stable identities always refers to the same immutable icon resource.
    format!("{base}.theme.{name}")
}

/// Set the stable process identity before UI; the main window adds its theme suffix.
pub fn initialize_taskbar_identity(identifier: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        let id: Vec<u16> = identifier.encode_utf16().chain(Some(0)).collect();
        // SAFETY: valid NUL-terminated string; called before any window exists.
        unsafe {
            windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID(
                windows::core::PCWSTR(id.as_ptr()),
            )
            .map_err(|error| error.to_string())?;
        }
    }
    #[cfg(not(windows))]
    let _ = identifier;
    Ok(())
}

/// Generate 64x64 RGBA raw pixel buffer for the specified theme.
#[allow(clippy::needless_range_loop, clippy::manual_range_contains)]
pub fn generate_theme_icon_rgba(theme: Theme) -> (Vec<u8>, u32, u32) {
    let mut fill = [[false; N]; N];
    let cx = 15.5_f64;
    let cy = 15.5_f64;

    // 1. Symmetrical circular arcs
    for y in 0..N {
        for x in 0..N {
            let dx = x as f64 - cx;
            let dy = y as f64 - cy;
            let dist = (dx * dx + dy * dy).sqrt();
            let mut angle = dy.atan2(dx).to_degrees();
            if angle < 0.0 {
                angle += 360.0;
            }

            if dist >= 9.6 && dist <= 13.2 {
                // Top-right arc: 216 deg to 325 deg
                if (216.0..=325.0).contains(&angle) {
                    fill[y][x] = true;
                }
                // Bottom-left arc: 36 deg to 145 deg
                if (36.0..=145.0).contains(&angle) {
                    fill[y][x] = true;
                }
            }
        }
    }

    // 2. Top-right arrowhead with symmetrical flared shoulders
    const HEAD: [(usize, usize, usize); 6] = [
        (10, 19, 29),
        (11, 19, 29),
        (12, 20, 28),
        (13, 21, 27),
        (14, 22, 26),
        (15, 23, 24),
    ];

    for &(y, x_start, x_end) in &HEAD {
        for x in x_start..=x_end {
            fill[y][x] = true;
        }
    }

    // 3. Bottom-left arrowhead (180° rotational symmetry: 31 - x, 31 - y)
    for &(y, x_start, x_end) in &HEAD {
        let sy = 31 - y;
        let sx0 = 31 - x_end;
        let sx1 = 31 - x_start;
        for x in sx0..=sx1 {
            fill[sy][x] = true;
        }
    }

    // 4. Cut back tails for generous breathing room
    fill[22][26] = false;
    fill[23][25] = false;
    fill[23][26] = false;
    fill[24][24] = false;
    fill[24][25] = false;

    fill[31 - 22][31 - 26] = false;
    fill[31 - 23][31 - 25] = false;
    fill[31 - 23][31 - 26] = false;
    fill[31 - 24][31 - 24] = false;
    fill[31 - 24][31 - 25] = false;

    // 5. Compute solid 1-block black outline
    let mut outline = [[false; N]; N];
    for y in 0..N {
        for x in 0..N {
            if fill[y][x] {
                continue;
            }
            let mut neighbor = false;
            for dy in -1_i32..=1 {
                for dx in -1_i32..=1 {
                    if dx == 0 && dy == 0 {
                        continue;
                    }
                    let ny = y as i32 + dy;
                    let nx = x as i32 + dx;
                    if ny >= 0
                        && ny < N as i32
                        && nx >= 0
                        && nx < N as i32
                        && fill[ny as usize][nx as usize]
                    {
                        neighbor = true;
                        break;
                    }
                }
                if neighbor {
                    break;
                }
            }
            if neighbor {
                outline[y][x] = true;
            }
        }
    }

    // 6. Theme Palette
    let (fill_rgb, outline_rgb, border_rgb, bg_top, bg_bottom) = match theme {
        Theme::Studio => (
            (239, 239, 237), // #efefed default theme accent
            (8, 8, 8),
            (85, 85, 85),
            (36, 36, 36),
            (16, 16, 16),
        ),
        Theme::Classic => (
            (255, 255, 179), // #ffffb3 butter yellow
            (18, 19, 14),
            (212, 212, 122),
            (32, 34, 22),
            (16, 17, 12),
        ),
        Theme::Ember => (
            (255, 154, 60), // #ff9a3c amber
            (26, 14, 8),
            (249, 115, 22),
            (46, 26, 14),
            (24, 14, 8),
        ),
        Theme::Vamp => (
            (252, 113, 113), // #fc7171 crimson
            (22, 10, 14),
            (236, 72, 153),
            (42, 16, 22),
            (20, 10, 14),
        ),
    };

    let size = 64_usize;
    let mut rgba = vec![0_u8; size * size * 4];

    let center = (size as f64 - 1.0) / 2.0;
    let half = size as f64 * 0.485;
    let radius = size as f64 * 0.22;
    let border_thickness = 1.8_f64;

    let glyph_scale = (size as f64 * 0.68) / (N as f64);
    let glyph_origin = (size as f64 - (N as f64) * glyph_scale) / 2.0;

    for y in 0..size {
        for x in 0..size {
            let offset = (y * size + x) * 4;

            let dx = (x as f64 - center).abs() - (half - radius);
            let dy = (y as f64 - center).abs() - (half - radius);
            let dist = if dx > 0.0 && dy > 0.0 {
                (dx * dx + dy * dy).sqrt() - radius
            } else {
                dx.max(dy) - radius
            };

            if dist > 0.5 {
                continue;
            }

            let outer_alpha = (0.5 - dist).clamp(0.0, 1.0);

            let gx = ((x as f64 - glyph_origin) / glyph_scale).floor() as isize;
            let gy = ((y as f64 - glyph_origin) / glyph_scale).floor() as isize;
            let mut is_fill = false;
            let mut is_outline = false;
            if gx >= 0 && gx < N as isize && gy >= 0 && gy < N as isize {
                let ux = gx as usize;
                let uy = gy as usize;
                if fill[uy][ux] {
                    is_fill = true;
                } else if outline[uy][ux] {
                    is_outline = true;
                }
            }

            if is_fill {
                rgba[offset] = fill_rgb.0;
                rgba[offset + 1] = fill_rgb.1;
                rgba[offset + 2] = fill_rgb.2;
                rgba[offset + 3] = (255.0 * outer_alpha).round() as u8;
            } else if is_outline {
                rgba[offset] = outline_rgb.0;
                rgba[offset + 1] = outline_rgb.1;
                rgba[offset + 2] = outline_rgb.2;
                rgba[offset + 3] = (255.0 * outer_alpha).round() as u8;
            } else if dist >= -border_thickness {
                rgba[offset] = border_rgb.0;
                rgba[offset + 1] = border_rgb.1;
                rgba[offset + 2] = border_rgb.2;
                rgba[offset + 3] = (255.0 * outer_alpha).round() as u8;
            } else {
                let grad_t = ((x + y) as f64 / ((size * 2) as f64)).clamp(0.0, 1.0);
                let r =
                    (bg_top.0 as f64 * (1.0 - grad_t) + bg_bottom.0 as f64 * grad_t).round() as u8;
                let g =
                    (bg_top.1 as f64 * (1.0 - grad_t) + bg_bottom.1 as f64 * grad_t).round() as u8;
                let b =
                    (bg_top.2 as f64 * (1.0 - grad_t) + bg_bottom.2 as f64 * grad_t).round() as u8;
                rgba[offset] = r;
                rgba[offset + 1] = g;
                rgba[offset + 2] = b;
                rgba[offset + 3] = (255.0 * outer_alpha).round() as u8;
            }
        }
    }

    (rgba, size as u32, size as u32)
}

/// Create a `tauri::image::Image` for the specified theme.
pub fn create_theme_icon(theme: Theme) -> Image<'static> {
    let (rgba, width, height) = generate_theme_icon_rgba(theme);
    Image::new_owned(rgba, width, height)
}

/// Single-image Windows ICO (32-bit DIB plus transparency mask). No codec dependency.
pub fn theme_icon_ico(theme: Theme) -> Vec<u8> {
    let (rgba, width, height) = generate_theme_icon_rgba(theme);
    let mask_stride = width.div_ceil(32) * 4;
    let image_size = 40 + width * height * 4 + mask_stride * height;
    let mut bytes = vec![
        0,
        0,
        1,
        0,
        1,
        0,
        width as u8,
        height as u8,
        0,
        0,
        1,
        0,
        32,
        0,
    ];
    bytes.extend_from_slice(&image_size.to_le_bytes());
    bytes.extend_from_slice(&22_u32.to_le_bytes());
    bytes.extend_from_slice(&40_u32.to_le_bytes());
    bytes.extend_from_slice(&width.to_le_bytes());
    bytes.extend_from_slice(&(height * 2).to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&32_u16.to_le_bytes());
    bytes.extend_from_slice(&[0; 24]);
    for row in rgba.chunks_exact(width as usize * 4).rev() {
        for pixel in row.as_chunks::<4>().0 {
            bytes.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
        }
    }
    for row in rgba.chunks_exact(width as usize * 4).rev() {
        let mut mask = vec![0_u8; mask_stride as usize];
        for (x, pixel) in row.as_chunks::<4>().0.iter().enumerate() {
            if pixel[3] == 0 {
                mask[x / 8] |= 0x80 >> (x % 8);
            }
        }
        bytes.extend_from_slice(&mask);
    }
    bytes
}

/// Explorer uses relaunch metadata for grouped/pinned buttons, not just WM_SETICON.
#[cfg(windows)]
fn apply_taskbar_icon<R: Runtime>(app: &AppHandle<R>, theme: Theme) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
    use windows::Win32::UI::Shell::PropertiesSystem::{
        IPropertyStore, SHGetPropertyStoreForWindow,
    };

    let directory = app
        .path()
        .app_local_data_dir()
        .map_err(|e| e.to_string())?
        .join("theme-icons");
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    // Versioned paths avoid Explorer reusing stale icon-cache entries.
    let path = directory.join(format!("{theme:?}-v4.ico"));
    let bytes = theme_icon_ico(theme);
    if std::fs::read(&path).ok().as_deref() != Some(bytes.as_slice()) {
        std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    }
    let window = app
        .get_webview_window("main")
        .ok_or("main window unavailable")?;
    let hwnd = window.hwnd().map_err(|e| e.to_string())?;
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let command = format!("\"{}\"", executable.display());
    let resource = format!("{},0", path.display());
    let app_id = taskbar_theme_id(&app.config().identifier, theme);
    if let Err(error) = update_start_menu_shortcut(&path, &app_id) {
        diagnostics::warn("icon", &format!("failed updating shortcut icon: {error}"));
    }
    // Tauri/tao set_icon updates ICON_SMALL only; the taskbar uses ICON_BIG.
    app.manage(TaskbarIcon::default());
    let native_window = hwnd.0 as isize;
    let icon_path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let handle = app.clone();
    window
        .run_on_main_thread(move || {
            use windows_sys::Win32::UI::WindowsAndMessaging::{
                DestroyIcon, LoadImageW, SendMessageW, IMAGE_ICON, LR_LOADFROMFILE, WM_SETICON,
            };
            let state = handle.state::<TaskbarIcon>();
            let Ok(mut previous) = state.0.lock() else {
                return;
            };
            if previous.theme == Some(theme) {
                return;
            }
            // SAFETY: NUL-terminated filename and live HWND. Retain the owned HICON until replacement.
            unsafe {
                let icon = LoadImageW(
                    std::ptr::null_mut(),
                    icon_path.as_ptr(),
                    IMAGE_ICON,
                    64,
                    64,
                    LR_LOADFROMFILE,
                );
                if icon.is_null() {
                    diagnostics::warn(
                        "icon",
                        &format!(
                            "taskbar icon load failed: {}",
                            std::io::Error::last_os_error()
                        ),
                    );
                    return;
                }
                SendMessageW(native_window as _, WM_SETICON, 1, icon as isize);
                if previous.handle != 0 {
                    DestroyIcon(previous.handle as _);
                }
                previous.handle = icon as isize;

                // Keep both native icon updates and Shell notifications on the
                // window thread, in order. Window stores apply values immediately.
                let result = (|| -> windows::core::Result<()> {
                    let store: IPropertyStore = SHGetPropertyStoreForWindow(
                        windows::Win32::Foundation::HWND(native_window as _),
                    )?;
                    for (key, value) in taskbar_properties(
                        &command,
                        &resource,
                        &app_id,
                        handle.config().product_name.as_deref().unwrap_or("Silk"),
                    ) {
                        store.SetValue(&key, &PROPVARIANT::from(value))?;
                    }
                    previous.theme = Some(theme);
                    diagnostics::info(
                        "icon",
                        &format!("applied {theme:?} taskbar identity {app_id}"),
                    );
                    Ok(())
                })();
                if let Err(error) = result {
                    diagnostics::warn("icon", &format!("taskbar metadata update failed: {error}"));
                }
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Release Shell properties before the main window closes.
/// Call on the UI thread at RunEvent::Exit, while the HWND is still available.
pub fn cleanup_theme_icon<R: Runtime>(app: &AppHandle<R>) {
    #[cfg(not(windows))]
    let _ = app;
    #[cfg(windows)]
    if let Some(window) = app.get_webview_window("main") {
        if let Ok(hwnd) = window.hwnd() {
            use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
            use windows::Win32::UI::Shell::PropertiesSystem::{
                IPropertyStore, SHGetPropertyStoreForWindow,
            };
            // SAFETY: on the UI thread before Tauri destroys the window.
            unsafe {
                let hwnd = windows::Win32::Foundation::HWND(hwnd.0);
                if let Ok(store) = SHGetPropertyStoreForWindow::<IPropertyStore>(hwnd) {
                    for (key, _) in taskbar_properties("", "", "", "") {
                        let _ = store.SetValue(&key, &PROPVARIANT::default());
                    }
                }
            }
        }
    }
}

/// Setting AppUserModel.ID notifies Explorer. Publish the new metadata BEFORE
/// that notification so the new group receives the matching icon.
#[cfg(windows)]
fn taskbar_properties<'a>(
    command: &'a str,
    resource: &'a str,
    app_id: &'a str,
    display_name: &'a str,
) -> [(windows::Win32::Foundation::PROPERTYKEY, &'a str); 4] {
    use windows::Win32::Storage::EnhancedStorage::{
        PKEY_AppUserModel_ID, PKEY_AppUserModel_RelaunchCommand,
        PKEY_AppUserModel_RelaunchDisplayNameResource, PKEY_AppUserModel_RelaunchIconResource,
    };
    [
        (PKEY_AppUserModel_RelaunchCommand, command),
        (PKEY_AppUserModel_RelaunchDisplayNameResource, display_name),
        (PKEY_AppUserModel_RelaunchIconResource, resource),
        (PKEY_AppUserModel_ID, app_id),
    ]
}

/// Update Silk's Start Menu link and any normal taskbar pin shortcut. Explorer
/// owns the pin database itself, so only its documented .lnk files are edited.
#[cfg(windows)]
fn update_start_menu_shortcut(icon_path: &std::path::Path, app_id: &str) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::{Interface, PCWSTR};
    use windows::Win32::Storage::EnhancedStorage::PKEY_AppUserModel_ID;
    use windows::Win32::Storage::FileSystem::WIN32_FIND_DATAW;
    use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, IPersistFile, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED, STGM_READWRITE,
    };
    use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};

    let app_data = std::env::var_os("APPDATA").ok_or("APPDATA is unavailable")?;
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let icon_wide: Vec<u16> = icon_path.as_os_str().encode_wide().chain(Some(0)).collect();
    let start_menu =
        std::path::PathBuf::from(&app_data).join("Microsoft/Windows/Start Menu/Programs/Silk.lnk");
    let pinned_dir = std::path::PathBuf::from(app_data)
        .join("Microsoft/Internet Explorer/Quick Launch/User Pinned/TaskBar");
    let mut shortcuts = Vec::new();
    if start_menu.is_file() {
        shortcuts.push(start_menu);
    }
    if let Ok(entries) = std::fs::read_dir(pinned_dir) {
        for entry in entries.flatten() {
            if entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "lnk")
            {
                shortcuts.push(entry.path());
            }
        }
    }
    if shortcuts.is_empty() {
        return Ok(());
    }

    std::thread::scope(|scope| {
        scope
            .spawn(|| unsafe {
                CoInitializeEx(None, COINIT_APARTMENTTHREADED)
                    .ok()
                    .map_err(|e| e.to_string())?;
                struct Apartment;
                impl Drop for Apartment {
                    fn drop(&mut self) {
                        unsafe { windows::Win32::System::Com::CoUninitialize() };
                    }
                }
                let _apartment = Apartment;
                let mut updated = 0;
                for shortcut in shortcuts {
                    let shortcut_wide: Vec<u16> =
                        shortcut.as_os_str().encode_wide().chain(Some(0)).collect();
                    let shell_link: IShellLinkW =
                        CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)
                            .map_err(|e| e.to_string())?;
                    let persist: IPersistFile = shell_link.cast().map_err(|e| e.to_string())?;
                    persist
                        .Load(PCWSTR(shortcut_wide.as_ptr()), STGM_READWRITE)
                        .map_err(|e| e.to_string())?;
                    let mut target = [0_u16; 32768];
                    let mut find_data = WIN32_FIND_DATAW::default();
                    shell_link
                        .GetPath(&mut target, &mut find_data, 0)
                        .map_err(|e| e.to_string())?;
                    let target = String::from_utf16_lossy(
                        &target[..target
                            .iter()
                            .position(|value| *value == 0)
                            .unwrap_or(target.len())],
                    );
                    if shortcut_targets_executable(&target, &executable) {
                        shell_link
                            .SetIconLocation(PCWSTR(icon_wide.as_ptr()), 0)
                            .map_err(|e| e.to_string())?;
                        let properties: IPropertyStore =
                            shell_link.cast().map_err(|e| e.to_string())?;
                        properties
                            .SetValue(&PKEY_AppUserModel_ID, &PROPVARIANT::from(app_id))
                            .map_err(|e| e.to_string())?;
                        properties.Commit().map_err(|e| e.to_string())?;
                        persist
                            .Save(PCWSTR(shortcut_wide.as_ptr()), true)
                            .map_err(|e| e.to_string())?;
                        windows::Win32::UI::Shell::SHChangeNotify(
                            windows::Win32::UI::Shell::SHCNE_UPDATEITEM,
                            windows::Win32::UI::Shell::SHCNF_PATHW,
                            Some(shortcut_wide.as_ptr().cast()),
                            None,
                        );
                        updated += 1;
                    }
                }
                diagnostics::info("icon", &format!("updated {updated} Silk shortcut icon(s)"));
                Ok::<(), String>(())
            })
            .join()
            .map_err(|_| "shortcut update thread panicked".to_string())?
    })
}

#[cfg(windows)]
fn shortcut_targets_executable(target: &str, executable: &std::path::Path) -> bool {
    // A development build must not rewrite an installed copy's shortcuts.
    target.eq_ignore_ascii_case(&executable.to_string_lossy())
}

/// Dynamically update both the window taskbar icon and system tray icon.
pub fn apply_theme_icon<R: Runtime>(app: &AppHandle<R>, theme: Theme) {
    #[cfg(windows)]
    if app
        .try_state::<TaskbarIcon>()
        .is_some_and(|state| state.0.lock().is_ok_and(|state| state.theme == Some(theme)))
    {
        return;
    }
    let icon = create_theme_icon(theme);

    // 1. Update active Window Taskbar Icon
    if let Some(window) = app.get_webview_window("main") {
        if let Err(error) = window.set_icon(icon.clone()) {
            diagnostics::warn("icon", &format!("failed setting window icon: {error}"));
        }
    }

    #[cfg(windows)]
    if let Err(error) = apply_taskbar_icon(app, theme) {
        diagnostics::warn(
            "icon",
            &format!("failed setting taskbar relaunch icon: {error}"),
        );
    }

    // 2. Update System Tray Icon
    if let Some(tray) = app.tray_by_id("main") {
        if let Err(error) = tray.set_icon(Some(icon)) {
            diagnostics::warn("icon", &format!("failed setting tray icon: {error}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn cached_taskbar_identity_never_refers_to_two_different_theme_icons() {
        let mut resources = std::collections::BTreeMap::new();
        for theme in [
            Theme::Studio,
            Theme::Classic,
            Theme::Studio,
            Theme::Vamp,
            Theme::Ember,
            Theme::Classic,
            Theme::Vamp,
            Theme::Studio,
        ]
        .repeat(100)
        {
            let id = taskbar_theme_id("dev.silk.project", theme);
            assert!(id.len() <= 128 && !id.contains(' '));
            let pixels = theme_icon_ico(theme);
            if let Some(previous) = resources.insert(id, pixels.clone()) {
                assert_eq!(
                    previous, pixels,
                    "a cached identity must retain its original pixels"
                );
            }
        }
        assert_eq!(resources.len(), 4);
    }

    #[cfg(windows)]
    #[test]
    fn taskbar_refresh_publishes_the_new_icon_before_notifying_explorer() {
        use windows::Win32::Storage::EnhancedStorage::{
            PKEY_AppUserModel_ID, PKEY_AppUserModel_RelaunchIconResource,
        };
        for resource in [
            "studio.ico,0",
            "classic.ico,0",
            "ember.ico,0",
            "vamp.ico,0",
            "studio.ico,0",
        ] {
            let properties = taskbar_properties("silk.exe", resource, "dev.silk.project", "Silk");
            let notification = properties
                .iter()
                .position(|(key, _)| *key == PKEY_AppUserModel_ID)
                .unwrap();
            let icon = properties
                .iter()
                .position(|(key, _)| *key == PKEY_AppUserModel_RelaunchIconResource)
                .unwrap();
            assert!(icon < notification);
            assert_eq!(notification, properties.len() - 1);
            assert_eq!(properties[icon].1, resource);
            assert_eq!(properties[notification].1, "dev.silk.project");
        }
    }

    #[cfg(windows)]
    #[test]
    fn shortcut_updates_only_target_this_executable() {
        let executable = std::path::Path::new(r"C:\Users\Home\AppData\Local\Silk\silk.exe");
        assert!(shortcut_targets_executable(
            r"c:\users\home\appdata\local\silk\SILK.EXE",
            executable
        ));
        assert!(!shortcut_targets_executable(
            r"C:\repo\target\release\silk.exe",
            executable
        ));
        assert!(!shortcut_targets_executable("", executable));
    }

    #[test]
    fn default_icon_matches_studio_accent() {
        let (pixels, width, height) = generate_theme_icon_rgba(Theme::Studio);
        assert_eq!(pixels.len(), (width * height * 4) as usize);
        assert!(pixels.as_chunks::<4>().0.contains(&[239, 239, 237, 255]));
        assert!(pixels
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| p[0].abs_diff(p[2]) <= 2));
    }

    #[test]
    fn bundled_default_ico_matches_runtime_icon() {
        let expected = theme_icon_ico(Theme::Studio);
        assert_eq!(include_bytes!("../icons/icon.ico").as_slice(), expected);
        assert_eq!(
            include_bytes!("../icons/icon-studio.ico").as_slice(),
            expected
        );
    }

    #[test]
    fn ico_header_and_payload_are_consistent_for_every_theme() {
        for theme in [Theme::Studio, Theme::Classic, Theme::Ember, Theme::Vamp] {
            let bytes = theme_icon_ico(theme);
            assert_eq!(&bytes[..6], &[0, 0, 1, 0, 1, 0]);
            assert_eq!(&bytes[6..8], &[64, 64]);
            let size = u32::from_le_bytes(bytes[14..18].try_into().unwrap());
            assert_eq!(bytes.len(), size as usize + 22);
            assert_eq!(&bytes[22..26], &40_u32.to_le_bytes());
        }
    }
}
