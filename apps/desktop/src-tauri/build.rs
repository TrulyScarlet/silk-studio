use std::env;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=SILK_RELEASE_BUILD");
    assert!(
        !(env::var("SILK_RELEASE_BUILD").as_deref() == Ok("1")
            && env::var_os("CARGO_FEATURE_UPDATER_TEST_SERVER").is_some()),
        "The loopback updater test feature cannot be included in a public release"
    );
    tauri_build::build();
    // The interactive taskbar example also needs Tauri's Common Controls v6
    // activation manifest; without it Windows 10 fails at process startup.
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let resources =
            std::path::PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("resource.lib");
        println!("cargo:rustc-link-arg-examples={}", resources.display());
    }
    // Windows executable icon resources are emitted by tauri_build; explicitly
    // rerun it when the ICO changes (Cargo otherwise leaves the old purple icon
    // embedded even after rebuilding the installer).
    println!("cargo:rerun-if-changed=icons/icon.ico");

    println!("cargo:rerun-if-env-changed=SILK_BUILD_COMMIT");
    println!("cargo:rerun-if-env-changed=SILK_BUILD_DATE");
    println!("cargo:rerun-if-env-changed=SILK_RUST_VERSION");

    let target = env::var("TARGET").unwrap_or_else(|_| "unknown".to_string());
    println!("cargo:rustc-env=SILK_BUILD_TARGET={target}");
    emit_optional_env("SILK_BUILD_COMMIT");
    emit_optional_env("SILK_BUILD_DATE");

    let rust_version = env::var("SILK_RUST_VERSION").ok().or_else(|| {
        Command::new("rustc")
            .arg("--version")
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
    });
    if let Some(version) = rust_version.filter(|version| !version.is_empty()) {
        println!("cargo:rustc-env=SILK_RUST_VERSION={version}");
    }
}

fn emit_optional_env(name: &str) {
    if let Ok(value) = env::var(name) {
        if !value.is_empty() {
            println!("cargo:rustc-env={name}={value}");
        }
    }
}
