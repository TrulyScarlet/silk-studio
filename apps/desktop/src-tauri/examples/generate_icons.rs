//! Regenerate the default ICO assets from the runtime renderer.
#[allow(dead_code)]
#[path = "../src/theme_icon.rs"]
mod theme_icon;

fn main() -> std::io::Result<()> {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("icons");
    let bytes = theme_icon::theme_icon_ico(configuration::Theme::Studio);
    for name in ["icon-studio.ico", "icon.ico"] {
        std::fs::write(directory.join(name), &bytes)?;
    }
    Ok(())
}
