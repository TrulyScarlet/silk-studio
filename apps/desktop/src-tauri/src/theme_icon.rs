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

    // 6. Theme Palette
    let (fill_rgb, outline_rgb, border_rgb, bg_top, bg_bottom) = match theme {
        Theme::Studio => (
            (192, 132, 252), // #c084fc bright purple arrows
            (20, 12, 32),    // dark outline
            (168, 85, 247),  // #a855f7 border
            (38, 18, 58),    // bg top-left
            (18, 12, 28),    // bg bottom-right
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
                let r = (bg_top.0 as f64 * (1.0 - grad_t) + bg_bottom.0 as f64 * grad_t).round()
                    as u8;
                let g = (bg_top.1 as f64 * (1.0 - grad_t) + bg_bottom.1 as f64 * grad_t).round()
                    as u8;
                let b = (bg_top.2 as f64 * (1.0 - grad_t) + bg_bottom.2 as f64 * grad_t).round()
                    as u8;
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
