//! Cross-process verification of the single-instance guard (APP-005).
//!
//! The in-process unit test proves mutex semantics within one process;
//! this test proves them across real OS processes by launching the
//! `instance-probe` helper binary as children.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use app_controller::{SingleInstanceError, SingleInstanceGuard};

fn unique_name(tag: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("silk-xproc-{tag}-{}-{nanos}", std::process::id())
}

fn spawn_probe(name: &str, hold_ms: u64) -> Child {
    Command::new(env!("CARGO_BIN_EXE_instance-probe"))
        .arg(name)
        .arg(hold_ms.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn instance-probe")
}

fn wait_for_line(child: &mut Child, expected: &str) -> bool {
    let Some(stdout) = child.stdout.take() else {
        return false;
    };
    let mut reader = BufReader::new(stdout);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut line = String::new();
    while std::time::Instant::now() < deadline {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => return false,
            Ok(_) if line.trim() == expected => return true,
            Ok(_) => continue,
            Err(_) => return false,
        }
    }
    false
}

#[test]
fn guard_blocks_a_second_process_until_released() {
    let name = unique_name("block");

    // First process acquires and holds.
    let mut holder = spawn_probe(&name, 1500);
    assert!(
        wait_for_line(&mut holder, "ACQUIRED"),
        "holder must report acquisition"
    );

    // Parent (this process) must be rejected while the holder lives.
    assert!(matches!(
        SingleInstanceGuard::try_acquire(&name),
        Err(SingleInstanceError)
    ));

    // A second child process must be rejected as well, exiting BUSY.
    let mut second = spawn_probe(&name, 0);
    let status = second.wait().expect("wait second probe");
    assert!(!status.success(), "busy probe must exit non-zero");

    // After the holder exits, the slot frees up for everyone.
    let _ = holder.wait().expect("wait holder");
    let late = SingleInstanceGuard::try_acquire(&name);
    assert!(late.is_ok(), "slot must be free after holder exit");
    drop(late);
}
