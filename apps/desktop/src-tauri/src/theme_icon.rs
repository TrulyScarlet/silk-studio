//! Dynamic theme-reactive Windows taskbar and system tray icon generator.
//!
//! Renders the Silk pixel art circular replay arrows with a crisp black outline
//! and theme-reactive arrow fill:
//! - Classic: Butter Yellow (`#ffffb3`)
//! - Autumn Ember: Harvest Seafoam (`#daffcf`)
//! - Vamp: Crimson Red (`#fc7171`)

use configuration::Theme;
use tauri::image::Image;
use tauri::{AppHandle, Manager, Runtime};

const N: usize = 32;

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

    // 6. Theme Fill Colors
    let (fill_r, fill_g, fill_b) = match theme {
        Theme::Studio => (168, 85, 247),   // Electric Violet (#a855f7)
        Theme::Classic => (255, 255, 179), // Butter Yellow (#ffffb3)
        Theme::Ember => (255, 154, 60),    // Warm Spiced Autumn Orange (#ff9a3c)
        Theme::Vamp => (252, 113, 113),    // Crimson Coral (#fc7171)
    };

    // Render 2x scaled (64x64) raw RGBA buffer
    let scale = 2_usize;
    let size = N * scale;
    let mut rgba = vec![0_u8; size * size * 4];

    for y in 0..size {
        let gy = y / scale;
        for x in 0..size {
            let gx = x / scale;
            let offset = (y * size + x) * 4;

            if fill[gy][gx] {
                rgba[offset] = fill_r;
                rgba[offset + 1] = fill_g;
                rgba[offset + 2] = fill_b;
                rgba[offset + 3] = 255;
            } else if outline[gy][gx] {
                rgba[offset] = 0;
                rgba[offset + 1] = 0;
                rgba[offset + 2] = 0;
                rgba[offset + 3] = 255;
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

/// Dynamically update both the window taskbar icon and system tray icon.
pub fn apply_theme_icon<R: Runtime>(app: &AppHandle<R>, theme: Theme) {
    let icon = create_theme_icon(theme);

    // 1. Update active Window Taskbar Icon
    if let Some(window) = app.get_webview_window("main") {
        if let Err(error) = window.set_icon(icon.clone()) {
            diagnostics::warn("icon", &format!("failed setting window icon: {error}"));
        }
    }

    // 2. Update System Tray Icon
    if let Some(tray) = app.tray_by_id("main") {
        if let Err(error) = tray.set_icon(Some(icon)) {
            diagnostics::warn("icon", &format!("failed setting tray icon: {error}"));
        }
    }
}
