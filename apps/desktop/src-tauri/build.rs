use std::env;
use std::process::Command;

fn main() {
    tauri_build::build();

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
