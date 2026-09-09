//! Cross-process single-instance probe used only by tests
//! (`tests/single_instance_cross_process.rs`). Not part of the product UI;
//! prints one status line so the parent can synchronize deterministically.

use app_controller::SingleInstanceGuard;

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(name) = args.next() else {
        eprintln!("usage: instance-probe <name> [hold_ms]");
        std::process::exit(64);
    };
    let hold_ms: u64 = args.next().and_then(|v| v.parse().ok()).unwrap_or(1500);

    match SingleInstanceGuard::try_acquire(&name) {
        Ok(guard) => {
            println!("ACQUIRED");
            use std::io::Write;
            std::io::stdout().flush().expect("flush stdout");
            std::thread::sleep(std::time::Duration::from_millis(hold_ms));
            drop(guard);
            println!("RELEASED");
        }
        Err(_) => {
            println!("BUSY");
            std::process::exit(2);
        }
    }
}
