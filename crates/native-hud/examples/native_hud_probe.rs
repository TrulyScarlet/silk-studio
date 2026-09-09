//! Manual visual probe for testing Native HUD state transitions and rendering.
//!
//! Cycles through `Queued` -> `Saved` -> `Failed` -> `Rejected` on the Windows desktop.
//! Accepts an optional CLI argument `compact` or `--compact` to run in Compact mode.

use native_hud::{HudConfig, HudCue, HudMode, NativeHud};
use std::time::Duration;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let is_compact = args
        .iter()
        .any(|arg| arg == "compact" || arg == "--compact" || arg == "-c");

    let mode = if is_compact {
        HudMode::Compact
    } else {
        HudMode::Full
    };

    println!("Initializing Silk Native HUD probe in {mode:?} mode...");
    let config = HudConfig {
        mode,
        ..Default::default()
    };

    let hud = match NativeHud::try_new(config) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("Failed to initialize Native HUD: {e}");
            return;
        }
    };

    println!("Capabilities: {:?}", hud.capabilities());
    println!("1/4: Showing Queued state (saving replay)...");
    let _ = hud.try_show(HudCue::queued());
    std::thread::sleep(Duration::from_millis(1500));

    println!("2/4: Transitioning in-place to Saved state (Silk Captured)...");
    let _ = hud.try_show(HudCue::saved(60_000, 94_200_000));
    std::thread::sleep(Duration::from_millis(2600));

    println!("3/4: Showing Failed state (Save failed)...");
    let _ = hud.try_show(HudCue::failed("Buffer not ready yet"));
    std::thread::sleep(Duration::from_millis(3600));

    println!("4/4: Showing Rejected state (Save rejected)...");
    let _ = hud.try_show(HudCue::rejected("Insufficient disk space"));
    std::thread::sleep(Duration::from_millis(3600));

    println!("Probe complete. Shutting down.");
}
