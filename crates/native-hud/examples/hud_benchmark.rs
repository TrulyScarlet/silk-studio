//! Phase 3/4 Native-HUD benchmark affordance for capture confirmation performance measurement.
//!
//! Emits an invariant CSV recording high-resolution QPC timestamps before cue submissions
//! across configurable HUD states, layout modes, anchors, and run cycles.

use native_hud::{HudAnchor, HudConfig, HudCue, HudMode, NativeHud};

/// Benchmark states supported by the CLI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BenchmarkState {
    /// Do not instantiate or display the Native HUD.
    NoHud,
    /// Instantiate and prewarm the HUD surface, but submit no cues.
    Idle,
    /// Submit Saved cues.
    Saved,
    /// Submit Queued cue, sleep transition delay, then submit Saved cue.
    QueuedToSaved,
    /// Submit Failed cue.
    Failed,
    /// Submit Rejected cue.
    Rejected,
}

impl BenchmarkState {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "no-hud" | "no_hud" => Ok(Self::NoHud),
            "idle" => Ok(Self::Idle),
            "saved" => Ok(Self::Saved),
            "queued-to-saved" | "queued_to_saved" => Ok(Self::QueuedToSaved),
            "failed" => Ok(Self::Failed),
            "rejected" => Ok(Self::Rejected),
            other => Err(format!(
                "unknown state: '{other}' (expected: no-hud, idle, saved, queued-to-saved, failed, rejected)"
            )),
        }
    }

    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::NoHud => "no-hud",
            Self::Idle => "idle",
            Self::Saved => "saved",
            Self::QueuedToSaved => "queued-to-saved",
            Self::Failed => "failed",
            Self::Rejected => "rejected",
        }
    }
}

pub fn parse_mode(s: &str) -> Result<HudMode, String> {
    match s {
        "full" => Ok(HudMode::Full),
        "compact" => Ok(HudMode::Compact),
        other => Err(format!("unknown mode: '{other}' (expected: full, compact)")),
    }
}

pub fn mode_to_str(mode: HudMode) -> &'static str {
    match mode {
        HudMode::Full => "full",
        HudMode::Compact => "compact",
    }
}

pub fn parse_anchor(s: &str) -> Result<HudAnchor, String> {
    match s {
        "top_left" | "top-left" => Ok(HudAnchor::TopLeft),
        "top_center" | "top-center" => Ok(HudAnchor::TopCenter),
        "top_right" | "top-right" => Ok(HudAnchor::TopRight),
        "center_left" | "center-left" => Ok(HudAnchor::CenterLeft),
        "center_right" | "center-right" => Ok(HudAnchor::CenterRight),
        "bottom_left" | "bottom-left" => Ok(HudAnchor::BottomLeft),
        "bottom_center" | "bottom-center" => Ok(HudAnchor::BottomCenter),
        "bottom_right" | "bottom-right" => Ok(HudAnchor::BottomRight),
        other => Err(format!(
            "unknown anchor: '{other}' (expected: top_left, top_center, top_right, center_left, center_right, bottom_left, bottom_center, bottom_right)"
        )),
    }
}

pub fn parse_capture_exclusion(s: &str) -> Result<bool, String> {
    match s {
        "on" => Ok(true),
        "off" => Ok(false),
        other => Err(format!(
            "unknown capture-exclusion setting: '{other}' (expected: on, off)"
        )),
    }
}

/// Lifecycle policies for Native HUD benchmark stimulation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BenchmarkLifecyclePolicy {
    AttachedShown,
    DetachedShown,
    AttachedHidden,
}

impl BenchmarkLifecyclePolicy {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "attached-shown" | "attached_shown" => Ok(Self::AttachedShown),
            "detached-shown" | "detached_shown" => Ok(Self::DetachedShown),
            "attached-hidden" | "attached_hidden" => Ok(Self::AttachedHidden),
            other => Err(format!(
                "unknown lifecycle-policy: '{other}' (expected: attached-shown, detached-shown, attached-hidden)"
            )),
        }
    }

    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::AttachedShown => "attached-shown",
            Self::DetachedShown => "detached-shown",
            Self::AttachedHidden => "attached-hidden",
        }
    }
}

/// Minimum allowed trigger interval in milliseconds.
pub const MIN_INTERVAL_MS: u64 = 5000;

/// Computes the deterministic nominal cue lifecycle duration in milliseconds from trigger to completion.
///
/// Lifecycle timing breakdown:
/// - `NoHud` / `Idle`: 0 ms (no cue displayed)
/// - `Saved`: entrance (180ms or 60ms reduced) + hold (2200ms) + exit (200ms or 60ms reduced)
/// - `Failed` / `Rejected`: entrance (180ms or 60ms reduced) + hold (3200ms) + exit (200ms or 60ms reduced)
/// - `QueuedToSaved`: transition_ms + hold (2200ms) + exit (200ms or 60ms reduced)
///   (the Saved update replaces the active Queued cue without a second entrance)
pub const fn nominal_cue_duration_ms(
    state: BenchmarkState,
    transition_ms: u64,
    reduced_motion: bool,
) -> u64 {
    let (entrance, exit) = if reduced_motion {
        (
            native_hud::REDUCED_MOTION_FADE_DURATION_MS,
            native_hud::REDUCED_MOTION_FADE_DURATION_MS,
        )
    } else {
        (
            native_hud::ENTRANCE_DURATION_MS,
            native_hud::EXIT_DURATION_MS,
        )
    };

    match state {
        BenchmarkState::NoHud | BenchmarkState::Idle => 0,
        BenchmarkState::Saved => entrance + native_hud::HOLD_SAVED_DURATION_MS + exit,
        BenchmarkState::Failed | BenchmarkState::Rejected => {
            entrance + native_hud::HOLD_FAILED_DURATION_MS + exit
        }
        BenchmarkState::QueuedToSaved => transition_ms + native_hud::HOLD_SAVED_DURATION_MS + exit,
    }
}

/// Parsed benchmark CLI configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BenchmarkConfig {
    pub state: BenchmarkState,
    pub mode: HudMode,
    pub anchor: HudAnchor,
    pub capture_exclusion: bool,
    pub lifecycle_policy: BenchmarkLifecyclePolicy,
    pub runs: usize,
    pub warmup: usize,
    pub interval_ms: u64,
    pub transition_ms: u64,
    pub markers_path: String,
}

impl Default for BenchmarkConfig {
    fn default() -> Self {
        Self {
            state: BenchmarkState::Saved,
            mode: HudMode::Full,
            anchor: HudAnchor::BottomCenter,
            capture_exclusion: true,
            lifecycle_policy: BenchmarkLifecyclePolicy::AttachedShown,
            runs: 10,
            warmup: 2,
            interval_ms: 5000,
            transition_ms: 250,
            markers_path: String::new(),
        }
    }
}

pub fn print_help() {
    println!(
        r#"Silk Native HUD Benchmark Affordance

USAGE:
    cargo run -p native-hud --example hud_benchmark -- [OPTIONS] --markers <PATH>

OPTIONS:
    --state <STATE>             Benchmark state: no-hud, idle, saved, queued-to-saved, failed, rejected [default: saved]
    --mode <MODE>               HUD layout mode: full, compact [default: full]
    --anchor <ANCHOR>           HUD screen anchor: top_left, top_center, top_right, center_left,
                                center_right, bottom_left, bottom_center, bottom_right [default: bottom_center]
    --capture-exclusion <on|off> HUD capture exclusion (WDA_EXCLUDEFROMCAPTURE): on, off [default: on]
    --lifecycle-policy <POLICY> HUD diagnostic lifecycle policy: attached-shown, detached-shown, attached-hidden [default: attached-shown]
    --runs <N>                  Number of measured benchmark runs (must be > 0) [default: 10]
    --warmup <N>                Number of warmup runs before measured runs [default: 2]
    --interval-ms <MS>          Interval between triggers in milliseconds (minimum 5000) [default: 5000]
    --transition-ms <MS>        Transition delay for queued-to-saved in milliseconds [default: 250]
    --markers <PATH>            File path to write invariant CSV markers [required]
    -h, --help                  Print this help text and exit"#
    );
}

#[derive(Debug, PartialEq, Eq)]
pub enum ParseOutcome {
    Help,
    Config(BenchmarkConfig),
}

pub fn parse_args<I, T>(args: I) -> Result<ParseOutcome, String>
where
    I: IntoIterator<Item = T>,
    T: Into<String>,
{
    let args_vec: Vec<String> = args.into_iter().map(Into::into).collect();
    let mut config = BenchmarkConfig::default();
    let mut markers_set = false;

    let mut i = 0;
    while i < args_vec.len() {
        let arg = &args_vec[i];
        if arg == "-h" || arg == "--help" {
            return Ok(ParseOutcome::Help);
        } else if arg == "--state" {
            i += 1;
            if i >= args_vec.len() {
                return Err("missing value for --state".to_string());
            }
            config.state = BenchmarkState::parse(&args_vec[i])?;
        } else if let Some(val) = arg.strip_prefix("--state=") {
            config.state = BenchmarkState::parse(val)?;
        } else if arg == "--mode" {
            i += 1;
            if i >= args_vec.len() {
                return Err("missing value for --mode".to_string());
            }
            config.mode = parse_mode(&args_vec[i])?;
        } else if let Some(val) = arg.strip_prefix("--mode=") {
            config.mode = parse_mode(val)?;
        } else if arg == "--anchor" {
            i += 1;
            if i >= args_vec.len() {
                return Err("missing value for --anchor".to_string());
            }
            config.anchor = parse_anchor(&args_vec[i])?;
        } else if let Some(val) = arg.strip_prefix("--anchor=") {
            config.anchor = parse_anchor(val)?;
        } else if arg == "--capture-exclusion" {
            i += 1;
            if i >= args_vec.len() {
                return Err("missing value for --capture-exclusion".to_string());
            }
            config.capture_exclusion = parse_capture_exclusion(&args_vec[i])?;
        } else if let Some(val) = arg.strip_prefix("--capture-exclusion=") {
            config.capture_exclusion = parse_capture_exclusion(val)?;
        } else if arg == "--lifecycle-policy" {
            i += 1;
            if i >= args_vec.len() {
                return Err("missing value for --lifecycle-policy".to_string());
            }
            config.lifecycle_policy = BenchmarkLifecyclePolicy::parse(&args_vec[i])?;
        } else if let Some(val) = arg.strip_prefix("--lifecycle-policy=") {
            config.lifecycle_policy = BenchmarkLifecyclePolicy::parse(val)?;
        } else if arg == "--runs" {
            i += 1;
            if i >= args_vec.len() {
                return Err("missing value for --runs".to_string());
            }
            let n: usize = args_vec[i]
                .parse()
                .map_err(|_| format!("invalid integer for --runs: '{}'", args_vec[i]))?;
            if n == 0 {
                return Err("--runs must be greater than 0".to_string());
            }
            config.runs = n;
        } else if let Some(val) = arg.strip_prefix("--runs=") {
            let n: usize = val
                .parse()
                .map_err(|_| format!("invalid integer for --runs: '{val}'"))?;
            if n == 0 {
                return Err("--runs must be greater than 0".to_string());
            }
            config.runs = n;
        } else if arg == "--warmup" {
            i += 1;
            if i >= args_vec.len() {
                return Err("missing value for --warmup".to_string());
            }
            config.warmup = args_vec[i]
                .parse()
                .map_err(|_| format!("invalid integer for --warmup: '{}'", args_vec[i]))?;
        } else if let Some(val) = arg.strip_prefix("--warmup=") {
            config.warmup = val
                .parse()
                .map_err(|_| format!("invalid integer for --warmup: '{val}'"))?;
        } else if arg == "--interval-ms" {
            i += 1;
            if i >= args_vec.len() {
                return Err("missing value for --interval-ms".to_string());
            }
            let ms: u64 = args_vec[i]
                .parse()
                .map_err(|_| format!("invalid integer for --interval-ms: '{}'", args_vec[i]))?;
            if ms < MIN_INTERVAL_MS {
                return Err(format!(
                    "--interval-ms must be at least {MIN_INTERVAL_MS} ms (got {ms})"
                ));
            }
            config.interval_ms = ms;
        } else if let Some(val) = arg.strip_prefix("--interval-ms=") {
            let ms: u64 = val
                .parse()
                .map_err(|_| format!("invalid integer for --interval-ms: '{val}'"))?;
            if ms < MIN_INTERVAL_MS {
                return Err(format!(
                    "--interval-ms must be at least {MIN_INTERVAL_MS} ms (got {ms})"
                ));
            }
            config.interval_ms = ms;
        } else if arg == "--transition-ms" {
            i += 1;
            if i >= args_vec.len() {
                return Err("missing value for --transition-ms".to_string());
            }
            config.transition_ms = args_vec[i]
                .parse()
                .map_err(|_| format!("invalid integer for --transition-ms: '{}'", args_vec[i]))?;
        } else if let Some(val) = arg.strip_prefix("--transition-ms=") {
            config.transition_ms = val
                .parse()
                .map_err(|_| format!("invalid integer for --transition-ms: '{val}'"))?;
        } else if arg == "--markers" {
            i += 1;
            if i >= args_vec.len() {
                return Err("missing value for --markers".to_string());
            }
            if args_vec[i].trim().is_empty() {
                return Err("--markers path cannot be empty".to_string());
            }
            config.markers_path = args_vec[i].clone();
            markers_set = true;
        } else if let Some(val) = arg.strip_prefix("--markers=") {
            if val.trim().is_empty() {
                return Err("--markers path cannot be empty".to_string());
            }
            config.markers_path = val.to_string();
            markers_set = true;
        } else {
            return Err(format!("unknown argument: '{arg}'"));
        }
        i += 1;
    }

    if !markers_set {
        return Err("missing required argument: --markers <path>".to_string());
    }

    Ok(ParseOutcome::Config(config))
}

/// Invariant schema header string with nominal duration and interval metadata appended.
pub const CSV_HEADER: &str =
    "run_index,warmup,state,mode,anchor,capture_exclusion,lifecycle_policy,trigger_qpc,qpc_frequency,submit_result,nominal_duration_ms,interval_ms";

/// Invariant single run marker record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkerRecord {
    pub run_index: usize,
    pub warmup: bool,
    pub state: &'static str,
    pub mode: &'static str,
    pub anchor: &'static str,
    pub capture_exclusion: &'static str,
    pub lifecycle_policy: &'static str,
    pub trigger_qpc: i64,
    pub qpc_frequency: i64,
    pub submit_result: String,
    pub nominal_duration_ms: u64,
    pub interval_ms: u64,
}

impl MarkerRecord {
    pub fn to_csv_row(&self) -> String {
        format!(
            "{},{},{},{},{},{},{},{},{},{},{},{}",
            self.run_index,
            if self.warmup { "true" } else { "false" },
            escape_csv_field(self.state),
            escape_csv_field(self.mode),
            escape_csv_field(self.anchor),
            escape_csv_field(self.capture_exclusion),
            escape_csv_field(self.lifecycle_policy),
            self.trigger_qpc,
            self.qpc_frequency,
            escape_csv_field(&self.submit_result),
            self.nominal_duration_ms,
            self.interval_ms,
        )
    }
}

pub fn escape_csv_field(field: &str) -> String {
    if field.contains(',') || field.contains('"') || field.contains('\n') || field.contains('\r') {
        let escaped = field.replace('"', "\"\"");
        format!("\"{escaped}\"")
    } else {
        field.to_string()
    }
}

pub fn generate_csv(records: &[MarkerRecord]) -> String {
    let mut csv = String::new();
    csv.push_str(CSV_HEADER);
    csv.push('\n');
    for rec in records {
        csv.push_str(&rec.to_csv_row());
        csv.push('\n');
    }
    csv
}

#[cfg(windows)]
fn run_benchmark(config: BenchmarkConfig) -> Result<(), String> {
    use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};

    let mut qpc_freq = 0i64;
    unsafe {
        QueryPerformanceFrequency(&mut qpc_freq)
            .map_err(|e| format!("QueryPerformanceFrequency failed: {e}"))?;
    }
    if qpc_freq <= 0 {
        return Err(format!(
            "QueryPerformanceFrequency returned non-positive frequency: {qpc_freq}"
        ));
    }

    let hud = if config.state != BenchmarkState::NoHud {
        let hud_config = HudConfig {
            mode: config.mode,
            anchor: config.anchor,
            exclude_from_capture: config.capture_exclusion,
        };
        let hud_instance = match config.lifecycle_policy {
            #[cfg(feature = "diagnostic-lifecycle")]
            BenchmarkLifecyclePolicy::AttachedShown => NativeHud::try_new_diagnostic(
                hud_config,
                native_hud::DiagnosticLifecyclePolicy::AttachedShown,
            )
            .map_err(|e| format!("failed to initialize NativeHud with AttachedShown: {e}"))?,
            #[cfg(not(feature = "diagnostic-lifecycle"))]
            BenchmarkLifecyclePolicy::AttachedShown => NativeHud::try_new(hud_config)
                .map_err(|e| format!("failed to initialize NativeHud: {e}"))?,
            #[cfg(feature = "diagnostic-lifecycle")]
            BenchmarkLifecyclePolicy::DetachedShown => NativeHud::try_new_diagnostic(
                hud_config,
                native_hud::DiagnosticLifecyclePolicy::DetachedShown,
            )
            .map_err(|e| format!("failed to initialize NativeHud with DetachedShown: {e}"))?,
            #[cfg(feature = "diagnostic-lifecycle")]
            BenchmarkLifecyclePolicy::AttachedHidden => NativeHud::try_new_diagnostic(
                hud_config,
                native_hud::DiagnosticLifecyclePolicy::AttachedHidden,
            )
            .map_err(|e| format!("failed to initialize NativeHud with AttachedHidden: {e}"))?,
            #[cfg(not(feature = "diagnostic-lifecycle"))]
            BenchmarkLifecyclePolicy::DetachedShown | BenchmarkLifecyclePolicy::AttachedHidden => {
                return Err(format!(
                    "diagnostic lifecycle policy '{}' requires building with --features diagnostic-lifecycle. Rebuild command: cargo build --release -p native-hud --example hud_benchmark --features diagnostic-lifecycle",
                    config.lifecycle_policy.as_str()
                ));
            }
        };
        Some(hud_instance)
    } else {
        None
    };

    // Pre-marker steady state: at least 1 second
    std::thread::sleep(std::time::Duration::from_millis(1000));

    let total_cycles = config.warmup + config.runs;
    let mut records = Vec::with_capacity(total_cycles);
    let exclusion_str = if config.capture_exclusion {
        "on"
    } else {
        "off"
    };
    let policy_str = config.lifecycle_policy.as_str();
    let reduced_motion = hud
        .as_ref()
        .map(|h| h.capabilities().reduced_motion)
        .unwrap_or(false);
    let nominal_duration_ms =
        nominal_cue_duration_ms(config.state, config.transition_ms, reduced_motion);

    for idx in 0..total_cycles {
        let is_warmup = idx < config.warmup;
        let cycle_start = std::time::Instant::now();

        // Record marker QPC and frequency immediately before no-op or first try_show
        let mut trigger_qpc = 0i64;
        unsafe {
            QueryPerformanceCounter(&mut trigger_qpc)
                .map_err(|e| format!("QueryPerformanceCounter failed: {e}"))?;
        }

        let submit_result = match (config.state, &hud) {
            (BenchmarkState::NoHud, _) | (BenchmarkState::Idle, _) => "not_submitted".to_string(),
            (BenchmarkState::Saved, Some(hud)) => {
                match hud.try_show(HudCue::saved(60_000, 10_000_000)) {
                    Ok(()) => "accepted".to_string(),
                    Err(e) => e.to_string(),
                }
            }
            (BenchmarkState::Failed, Some(hud)) => {
                match hud.try_show(HudCue::failed("benchmark error")) {
                    Ok(()) => "accepted".to_string(),
                    Err(e) => e.to_string(),
                }
            }
            (BenchmarkState::Rejected, Some(hud)) => {
                match hud.try_show(HudCue::rejected("benchmark rejection")) {
                    Ok(()) => "accepted".to_string(),
                    Err(e) => e.to_string(),
                }
            }
            (BenchmarkState::QueuedToSaved, Some(hud)) => {
                let queued_res = hud.try_show(HudCue::queued());
                std::thread::sleep(std::time::Duration::from_millis(config.transition_ms));
                let saved_res = hud.try_show(HudCue::saved(60_000, 10_000_000));

                match (queued_res, saved_res) {
                    (Ok(()), Ok(())) => "accepted".to_string(),
                    (Err(e1), Ok(())) => format!("queued_error: {e1}"),
                    (Ok(()), Err(e2)) => format!("saved_error: {e2}"),
                    (Err(e1), Err(e2)) => format!("queued_error: {e1}; saved_error: {e2}"),
                }
            }
            (_, None) => "not_submitted".to_string(),
        };

        records.push(MarkerRecord {
            run_index: idx,
            warmup: is_warmup,
            state: config.state.as_str(),
            mode: mode_to_str(config.mode),
            anchor: config.anchor.name(),
            capture_exclusion: exclusion_str,
            lifecycle_policy: policy_str,
            trigger_qpc,
            qpc_frequency: qpc_freq,
            submit_result,
            nominal_duration_ms,
            interval_ms: config.interval_ms,
        });

        // Maintain interval spacing (minimum 5000 ms between triggers)
        if idx + 1 < total_cycles {
            let elapsed = cycle_start.elapsed();
            let target_interval = std::time::Duration::from_millis(config.interval_ms);
            if elapsed < target_interval {
                std::thread::sleep(target_interval - elapsed);
            }
        }
    }

    // Post-marker steady state: at least 5 seconds
    std::thread::sleep(std::time::Duration::from_millis(5000));

    // Write invariant CSV
    let csv = generate_csv(&records);
    if let Some(parent) = std::path::Path::new(&config.markers_path).parent() {
        if !parent.as_os_str().is_empty() {
            let _ = std::fs::create_dir_all(parent);
        }
    }
    std::fs::write(&config.markers_path, csv).map_err(|e| {
        format!(
            "failed to write markers CSV to '{}': {e}",
            config.markers_path
        )
    })?;

    println!(
        "HUD benchmark complete: {} cycles recorded to '{}'.",
        total_cycles, config.markers_path
    );
    Ok(())
}

#[cfg(not(windows))]
fn run_benchmark(_config: BenchmarkConfig) -> Result<(), String> {
    Err("native HUD benchmark is only supported on Windows".to_string())
}

fn main() {
    let raw_args: Vec<String> = std::env::args().skip(1).collect();

    let outcome = match parse_args(raw_args) {
        Ok(o) => o,
        Err(err) => {
            eprintln!("Error: {err}\n");
            print_help();
            std::process::exit(1);
        }
    };

    let config = match outcome {
        ParseOutcome::Help => {
            print_help();
            std::process::exit(0);
        }
        ParseOutcome::Config(cfg) => cfg,
    };

    if let Err(err) = run_benchmark(config) {
        eprintln!("Benchmark failed: {err}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_valid_all_states() {
        let states = [
            ("no-hud", BenchmarkState::NoHud),
            ("no_hud", BenchmarkState::NoHud),
            ("idle", BenchmarkState::Idle),
            ("saved", BenchmarkState::Saved),
            ("queued-to-saved", BenchmarkState::QueuedToSaved),
            ("queued_to_saved", BenchmarkState::QueuedToSaved),
            ("failed", BenchmarkState::Failed),
            ("rejected", BenchmarkState::Rejected),
        ];

        for (input, expected) in states {
            let parsed = BenchmarkState::parse(input).expect("valid state should parse");
            assert_eq!(parsed, expected);

            let args = vec!["--state", input, "--markers", "out.csv"];
            match parse_args(args).expect("valid args") {
                ParseOutcome::Config(cfg) => assert_eq!(cfg.state, expected),
                ParseOutcome::Help => panic!("unexpected help"),
            }
        }
    }

    #[test]
    fn test_parse_valid_modes() {
        let modes = [("full", HudMode::Full), ("compact", HudMode::Compact)];

        for (input, expected) in modes {
            let parsed = parse_mode(input).expect("valid mode should parse");
            assert_eq!(parsed, expected);

            let args = vec!["--mode", input, "--markers", "out.csv"];
            match parse_args(args).expect("valid args") {
                ParseOutcome::Config(cfg) => assert_eq!(cfg.mode, expected),
                ParseOutcome::Help => panic!("unexpected help"),
            }
        }
    }

    #[test]
    fn test_parse_valid_all_8_anchors() {
        let anchors = [
            ("top_left", HudAnchor::TopLeft),
            ("top_center", HudAnchor::TopCenter),
            ("top_right", HudAnchor::TopRight),
            ("center_left", HudAnchor::CenterLeft),
            ("center_right", HudAnchor::CenterRight),
            ("bottom_left", HudAnchor::BottomLeft),
            ("bottom_center", HudAnchor::BottomCenter),
            ("bottom_right", HudAnchor::BottomRight),
            ("top-left", HudAnchor::TopLeft),
            ("bottom-center", HudAnchor::BottomCenter),
        ];

        for (input, expected) in anchors {
            let parsed = parse_anchor(input).expect("valid anchor should parse");
            assert_eq!(parsed, expected);

            let args = vec!["--anchor", input, "--markers", "out.csv"];
            match parse_args(args).expect("valid args") {
                ParseOutcome::Config(cfg) => assert_eq!(cfg.anchor, expected),
                ParseOutcome::Help => panic!("unexpected help"),
            }
        }
    }

    #[test]
    fn test_parse_capture_exclusion_options() {
        let cases = [("on", true), ("off", false)];

        for (input, expected) in cases {
            let parsed = parse_capture_exclusion(input).expect("valid capture exclusion");
            assert_eq!(parsed, expected);

            let args = vec!["--capture-exclusion", input, "--markers", "out.csv"];
            match parse_args(args).expect("valid args") {
                ParseOutcome::Config(cfg) => assert_eq!(cfg.capture_exclusion, expected),
                ParseOutcome::Help => panic!("unexpected help"),
            }

            let eq_arg = format!("--capture-exclusion={input}");
            match parse_args(vec![eq_arg.as_str(), "--markers", "out.csv"]).expect("valid args") {
                ParseOutcome::Config(cfg) => assert_eq!(cfg.capture_exclusion, expected),
                ParseOutcome::Help => panic!("unexpected help"),
            }
        }
    }

    #[test]
    fn test_parse_lifecycle_policy_options() {
        let cases = [
            ("attached-shown", BenchmarkLifecyclePolicy::AttachedShown),
            ("attached_shown", BenchmarkLifecyclePolicy::AttachedShown),
            ("detached-shown", BenchmarkLifecyclePolicy::DetachedShown),
            ("detached_shown", BenchmarkLifecyclePolicy::DetachedShown),
            ("attached-hidden", BenchmarkLifecyclePolicy::AttachedHidden),
            ("attached_hidden", BenchmarkLifecyclePolicy::AttachedHidden),
        ];

        for (input, expected) in cases {
            let parsed = BenchmarkLifecyclePolicy::parse(input).expect("valid lifecycle policy");
            assert_eq!(parsed, expected);

            let args = vec!["--lifecycle-policy", input, "--markers", "out.csv"];
            match parse_args(args).expect("valid args") {
                ParseOutcome::Config(cfg) => assert_eq!(cfg.lifecycle_policy, expected),
                ParseOutcome::Help => panic!("unexpected help"),
            }

            let eq_arg = format!("--lifecycle-policy={input}");
            match parse_args(vec![eq_arg.as_str(), "--markers", "out.csv"]).expect("valid args") {
                ParseOutcome::Config(cfg) => assert_eq!(cfg.lifecycle_policy, expected),
                ParseOutcome::Help => panic!("unexpected help"),
            }
        }
    }

    #[test]
    fn test_parse_invalid_lifecycle_policy() {
        let err = BenchmarkLifecyclePolicy::parse("invalid-policy").unwrap_err();
        assert!(err.contains("unknown lifecycle-policy"));

        let res = parse_args(vec![
            "--lifecycle-policy",
            "invalid",
            "--markers",
            "out.csv",
        ]);
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("unknown lifecycle-policy"));

        let res_missing = parse_args(vec!["--lifecycle-policy"]);
        assert!(res_missing.is_err());
        assert!(res_missing.unwrap_err().contains("missing value"));
    }

    #[test]
    fn test_parse_defaults() {
        let args = vec!["--markers", "benchmark.csv"];
        match parse_args(args).expect("valid args") {
            ParseOutcome::Config(cfg) => {
                assert_eq!(cfg.state, BenchmarkState::Saved);
                assert_eq!(cfg.mode, HudMode::Full);
                assert_eq!(cfg.anchor, HudAnchor::BottomCenter);
                assert!(cfg.capture_exclusion);
                assert_eq!(
                    cfg.lifecycle_policy,
                    BenchmarkLifecyclePolicy::AttachedShown
                );
                assert_eq!(cfg.runs, 10);
                assert_eq!(cfg.warmup, 2);
                assert_eq!(cfg.interval_ms, 5000);
                assert_eq!(cfg.transition_ms, 250);
                assert_eq!(cfg.markers_path, "benchmark.csv");
            }
            ParseOutcome::Help => panic!("unexpected help"),
        }
    }

    #[test]
    fn test_parse_equals_syntax() {
        let args = vec![
            "--state=idle",
            "--mode=compact",
            "--anchor=top_right",
            "--capture-exclusion=off",
            "--lifecycle-policy=detached-shown",
            "--runs=5",
            "--warmup=1",
            "--interval-ms=6000",
            "--transition-ms=300",
            "--markers=test_out.csv",
        ];
        match parse_args(args).expect("valid args") {
            ParseOutcome::Config(cfg) => {
                assert_eq!(cfg.state, BenchmarkState::Idle);
                assert_eq!(cfg.mode, HudMode::Compact);
                assert_eq!(cfg.anchor, HudAnchor::TopRight);
                assert!(!cfg.capture_exclusion);
                assert_eq!(
                    cfg.lifecycle_policy,
                    BenchmarkLifecyclePolicy::DetachedShown
                );
                assert_eq!(cfg.runs, 5);
                assert_eq!(cfg.warmup, 1);
                assert_eq!(cfg.interval_ms, 6000);
                assert_eq!(cfg.transition_ms, 300);
                assert_eq!(cfg.markers_path, "test_out.csv");
            }
            ParseOutcome::Help => panic!("unexpected help"),
        }
    }

    #[test]
    fn test_parse_help() {
        assert!(matches!(
            parse_args(vec!["--help"]).unwrap(),
            ParseOutcome::Help
        ));
        assert!(matches!(
            parse_args(vec!["-h"]).unwrap(),
            ParseOutcome::Help
        ));
        assert!(matches!(
            parse_args(vec!["--state", "saved", "-h"]).unwrap(),
            ParseOutcome::Help
        ));
    }

    #[test]
    fn test_parse_invalid_capture_exclusion() {
        let err = parse_capture_exclusion("invalid").unwrap_err();
        assert!(err.contains("unknown capture-exclusion"));

        let res = parse_args(vec!["--capture-exclusion", "yes", "--markers", "out.csv"]);
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("unknown capture-exclusion"));

        let res_missing = parse_args(vec!["--capture-exclusion"]);
        assert!(res_missing.is_err());
        assert!(res_missing.unwrap_err().contains("missing value"));
    }

    #[test]
    fn test_parse_invalid_state() {
        let err = BenchmarkState::parse("invalid-state").unwrap_err();
        assert!(err.contains("unknown state"));

        let res = parse_args(vec!["--state", "invalid", "--markers", "out.csv"]);
        assert!(res.is_err());
    }

    #[test]
    fn test_parse_invalid_mode() {
        let err = parse_mode("ultra").unwrap_err();
        assert!(err.contains("unknown mode"));

        let res = parse_args(vec!["--mode", "ultra", "--markers", "out.csv"]);
        assert!(res.is_err());
    }

    #[test]
    fn test_parse_invalid_anchor() {
        let err = parse_anchor("middle_nowhere").unwrap_err();
        assert!(err.contains("unknown anchor"));

        let res = parse_args(vec!["--anchor", "middle_nowhere", "--markers", "out.csv"]);
        assert!(res.is_err());
    }

    #[test]
    fn test_parse_zero_runs() {
        let res = parse_args(vec!["--runs", "0", "--markers", "out.csv"]);
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("greater than 0"));

        let res_eq = parse_args(vec!["--runs=0", "--markers", "out.csv"]);
        assert!(res_eq.is_err());
        assert!(res_eq.unwrap_err().contains("greater than 0"));
    }

    #[test]
    fn test_parse_interval_floor() {
        // Below 5000 ms floor should error
        let res = parse_args(vec!["--interval-ms", "4999", "--markers", "out.csv"]);
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("at least 5000 ms"));

        let res_eq = parse_args(vec!["--interval-ms=1000", "--markers", "out.csv"]);
        assert!(res_eq.is_err());
        assert!(res_eq.unwrap_err().contains("at least 5000 ms"));

        // 5000 ms exact floor should succeed
        let res_exact = parse_args(vec!["--interval-ms", "5000", "--markers", "out.csv"]);
        assert!(res_exact.is_ok());
    }

    #[test]
    fn test_parse_missing_markers() {
        let res = parse_args(vec!["--state", "saved"]);
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("missing required argument"));

        let res_empty = parse_args(vec!["--markers", "  "]);
        assert!(res_empty.is_err());
        assert!(res_empty.unwrap_err().contains("cannot be empty"));
    }

    #[test]
    fn test_parse_unknown_argument() {
        let res = parse_args(vec!["--unknown-flag", "--markers", "out.csv"]);
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("unknown argument"));
    }

    #[test]
    fn test_csv_escaping() {
        assert_eq!(escape_csv_field("accepted"), "accepted");
        assert_eq!(escape_csv_field("not_submitted"), "not_submitted");
        assert_eq!(
            escape_csv_field("error: failed, code=1"),
            "\"error: failed, code=1\""
        );
        assert_eq!(
            escape_csv_field("error with \"quotes\""),
            "\"error with \"\"quotes\"\"\""
        );
        assert_eq!(
            escape_csv_field("multi\nline\rerror"),
            "\"multi\nline\rerror\""
        );
    }

    #[test]
    fn test_nominal_cue_duration_standard_motion() {
        assert_eq!(
            nominal_cue_duration_ms(BenchmarkState::NoHud, 250, false),
            0
        );
        assert_eq!(nominal_cue_duration_ms(BenchmarkState::Idle, 250, false), 0);
        // Saved: 180 + 2200 + 200 = 2580
        assert_eq!(
            nominal_cue_duration_ms(BenchmarkState::Saved, 250, false),
            2580
        );
        // Failed: 180 + 3200 + 200 = 3580
        assert_eq!(
            nominal_cue_duration_ms(BenchmarkState::Failed, 250, false),
            3580
        );
        // Rejected: 180 + 3200 + 200 = 3580
        assert_eq!(
            nominal_cue_duration_ms(BenchmarkState::Rejected, 250, false),
            3580
        );
        // QueuedToSaved: transition_ms (250) + 2200 + 200 = 2650
        assert_eq!(
            nominal_cue_duration_ms(BenchmarkState::QueuedToSaved, 250, false),
            2650
        );
        // QueuedToSaved with 500ms transition: 500 + 2200 + 200 = 2900
        assert_eq!(
            nominal_cue_duration_ms(BenchmarkState::QueuedToSaved, 500, false),
            2900
        );
    }

    #[test]
    fn test_nominal_cue_duration_reduced_motion() {
        assert_eq!(nominal_cue_duration_ms(BenchmarkState::NoHud, 250, true), 0);
        assert_eq!(nominal_cue_duration_ms(BenchmarkState::Idle, 250, true), 0);
        // Saved: 60 + 2200 + 60 = 2320
        assert_eq!(
            nominal_cue_duration_ms(BenchmarkState::Saved, 250, true),
            2320
        );
        // Failed: 60 + 3200 + 60 = 3320
        assert_eq!(
            nominal_cue_duration_ms(BenchmarkState::Failed, 250, true),
            3320
        );
        // Rejected: 60 + 3200 + 60 = 3320
        assert_eq!(
            nominal_cue_duration_ms(BenchmarkState::Rejected, 250, true),
            3320
        );
        // QueuedToSaved: transition_ms (250) + 2200 + 60 = 2510
        assert_eq!(
            nominal_cue_duration_ms(BenchmarkState::QueuedToSaved, 250, true),
            2510
        );
    }

    #[test]
    fn test_csv_schema_and_generation() {
        let records = vec![
            MarkerRecord {
                run_index: 0,
                warmup: true,
                state: "saved",
                mode: "full",
                anchor: "bottom_center",
                capture_exclusion: "on",
                lifecycle_policy: "attached-shown",
                trigger_qpc: 100000,
                qpc_frequency: 10000000,
                submit_result: "accepted".to_string(),
                nominal_duration_ms: 2580,
                interval_ms: 5000,
            },
            MarkerRecord {
                run_index: 1,
                warmup: false,
                state: "queued-to-saved",
                mode: "compact",
                anchor: "top_left",
                capture_exclusion: "off",
                lifecycle_policy: "detached-shown",
                trigger_qpc: 150000,
                qpc_frequency: 10000000,
                submit_result: "queued_error: queue full, code 5".to_string(),
                nominal_duration_ms: 2650,
                interval_ms: 6000,
            },
        ];

        let csv = generate_csv(&records);
        let lines: Vec<&str> = csv.lines().collect();

        assert_eq!(lines.len(), 3);
        assert_eq!(
            lines[0],
            "run_index,warmup,state,mode,anchor,capture_exclusion,lifecycle_policy,trigger_qpc,qpc_frequency,submit_result,nominal_duration_ms,interval_ms"
        );
        assert_eq!(
            lines[1],
            "0,true,saved,full,bottom_center,on,attached-shown,100000,10000000,accepted,2580,5000"
        );
        assert_eq!(
            lines[2],
            "1,false,queued-to-saved,compact,top_left,off,detached-shown,150000,10000000,\"queued_error: queue full, code 5\",2650,6000"
        );
    }
}
