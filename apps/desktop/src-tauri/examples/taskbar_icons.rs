//! Opt-in interactive Windows taskbar check; no recorder or capture is started.
//! Run with `cargo run -p silk --example taskbar_icons`.
//! Add `-- --stress` for 24 transitions including rapid bursts and revisits.
#[allow(dead_code)]
#[path = "../src/theme_icon.rs"]
mod theme_icon;

fn main() {
    use tauri::Manager;
    let mut context = tauri::generate_context!();
    // Do not share the installed app's group, cached shortcut, or WebView profile.
    context.config_mut().identifier = "dev.silk.project.taskbar-color-test".into();
    context.config_mut().product_name = Some("Silk icon test".into());
    theme_icon::initialize_taskbar_identity(&context.config().identifier).unwrap();
    let window = &mut context.config_mut().app.windows[0];
    window.url = tauri::WebviewUrl::External("about:blank".parse().unwrap());
    window.title = "Silk taskbar check (no capture)".into();
    window.maximized = false;
    window.width = 960.0;
    window.height = 640.0;
    diagnostics::init(diagnostics::LoggerConfig {
        also_stderr: true,
        ..Default::default()
    });
    let stress = std::env::args().any(|arg| arg == "--stress");
    tauri::Builder::default()
        .setup(move |app| {
            theme_icon::apply_theme_icon(app.handle(), configuration::Theme::Studio);
            app.get_webview_window("main").unwrap().show()?;
            let app = app.handle().clone();
            std::thread::spawn(move || {
                let sequence = if stress {
                    [
                        configuration::Theme::Classic,
                        configuration::Theme::Studio,
                        configuration::Theme::Vamp,
                        configuration::Theme::Studio,
                        configuration::Theme::Ember,
                        configuration::Theme::Classic,
                        configuration::Theme::Studio,
                        configuration::Theme::Ember,
                    ]
                    .repeat(3)
                } else {
                    vec![
                        configuration::Theme::Classic,
                        configuration::Theme::Ember,
                        configuration::Theme::Vamp,
                        configuration::Theme::Studio,
                    ]
                };
                for (index, theme) in sequence.into_iter().enumerate() {
                    std::thread::sleep(std::time::Duration::from_secs(if stress { 3 } else { 8 }));
                    if stress && index % 4 == 3 {
                        for burst in [
                            configuration::Theme::Vamp,
                            configuration::Theme::Studio,
                            configuration::Theme::Classic,
                            configuration::Theme::Ember,
                        ] {
                            theme_icon::apply_theme_icon(&app, burst);
                            std::thread::sleep(std::time::Duration::from_millis(30));
                        }
                    }
                    theme_icon::apply_theme_icon(&app, theme);
                    app.get_webview_window("main")
                        .unwrap()
                        .set_title(&format!("Silk taskbar check: {theme:?} (no capture)"))
                        .unwrap();
                    println!("CHECK {index} {theme:?}");
                }
                std::thread::sleep(std::time::Duration::from_secs(8));
                app.exit(0);
            });
            Ok(())
        })
        .build(context)
        .expect("taskbar check failed")
        .run(|app, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                theme_icon::cleanup_theme_icon(app);
            }
        });
}
