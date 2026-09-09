//! Structured JSON Lines logging for the Silk recorder (spec §22.2).
//!
//! Properties:
//! - Every line is a self-contained JSON object: timestamp, level,
//!   component, message, optional structured fields.
//! - Size-based rotation keeps a bounded number of bounded files.
//! - A redaction pass runs over the fully serialized line so paths and
//!   configured secrets never reach disk.
//! - Logging must never panic the media pipeline; I/O errors are swallowed
//!   after first report.

pub mod redaction;

use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde_json::{Map, Value};

use redaction::Redactor;

pub const DEFAULT_MAX_ROTATED_FILES: usize = 5;
pub const DEFAULT_EXPORT_LOG_LINES: usize = 200;
const MAX_EXPORT_RECORD_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Trace = 0,
    Debug = 1,
    Info = 2,
    Warn = 3,
    Error = 4,
}

impl Level {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Trace => "TRACE",
            Self::Debug => "DEBUG",
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_uppercase().as_str() {
            "TRACE" => Some(Self::Trace),
            "DEBUG" => Some(Self::Debug),
            "INFO" => Some(Self::Info),
            "WARN" | "WARNING" => Some(Self::Warn),
            "ERROR" => Some(Self::Error),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct LoggerConfig {
    pub level: Level,
    pub file_path: Option<PathBuf>,
    pub max_file_bytes: u64,
    pub max_rotated_files: usize,
    pub also_stderr: bool,
    pub redactions: Vec<String>,
}

impl Default for LoggerConfig {
    fn default() -> Self {
        Self {
            level: Level::Info,
            file_path: None,
            max_file_bytes: 5 * 1024 * 1024,
            max_rotated_files: DEFAULT_MAX_ROTATED_FILES,
            also_stderr: false,
            redactions: Vec::new(),
        }
    }
}

struct Inner {
    config: LoggerConfig,
    file: Option<File>,
    written_bytes: u64,
    io_failed_reported: bool,
}

/// Thread-safe JSONL logger. All writes serialize through one mutex; the
/// critical section is short (format + write) and never blocks callers on
/// anything but local disk buffering, which is acceptable for diagnostics
/// (media threads use in-memory metrics, spec §14).
pub struct Logger {
    inner: Mutex<Inner>,
    redactor: Redactor,
}

impl Logger {
    pub fn new(config: LoggerConfig) -> std::io::Result<Self> {
        let mut effective = config;
        if effective.redactions.is_empty() {
            if let Some(profile) = std::env::var_os("USERPROFILE") {
                let profile = profile.to_string_lossy().into_owned();
                if profile.len() > 4 {
                    effective.redactions.push(profile);
                }
            }
        }
        let redactor = Redactor::new(effective.redactions.clone());

        let (file, written_bytes) = match &effective.file_path {
            Some(path) => {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let existing = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
                let file = OpenOptions::new().create(true).append(true).open(path)?;
                (Some(file), existing)
            }
            None => (None, 0),
        };

        Ok(Self {
            inner: Mutex::new(Inner {
                config: effective,
                file,
                written_bytes,
                io_failed_reported: false,
            }),
            redactor,
        })
    }

    /// Logger that discards everything (used when global init was skipped).
    pub fn off() -> Self {
        Self::new(LoggerConfig {
            level: Level::Error,
            file_path: None,
            also_stderr: false,
            ..LoggerConfig::default()
        })
        .expect("off logger cannot fail to open")
    }

    /// Log with structured fields (`fields` must be a JSON object).
    ///
    /// Redaction runs on raw strings *before* serialization so that JSON
    /// escaping cannot hide Windows paths from substring matching.
    pub fn log_kv(&self, level: Level, component: &str, message: &str, fields: Map<String, Value>) {
        let Ok(mut inner) = self.inner.lock() else {
            return;
        };
        if level < inner.config.level {
            return;
        }

        let safe_message = self.redactor.redact(message);
        let sanitized_fields: Map<String, Value> = fields
            .into_iter()
            .map(|(key, value)| (key, self.redactor.redact_value(&value)))
            .collect();
        let record = build_record(level, component, &safe_message, sanitized_fields);
        let line = record.to_string();

        if inner.config.also_stderr {
            eprintln!("{line}");
        }

        if inner.config.file_path.is_some()
            && inner.written_bytes + line.len() as u64 > inner.config.max_file_bytes
        {
            rotate(&mut inner);
        }

        if let Some(file) = inner.file.as_mut() {
            match writeln!(file, "{line}") {
                Ok(()) => {
                    inner.written_bytes += line.len() as u64 + 1;
                    inner.io_failed_reported = false;
                }
                Err(err) => {
                    if !inner.io_failed_reported {
                        inner.io_failed_reported = true;
                        eprintln!("silk-diagnostics: log write failed: {err}");
                    }
                }
            }
        }
    }

    pub fn log(&self, level: Level, component: &str, message: &str) {
        self.log_kv(level, component, message, Map::new());
    }

    /// Change the threshold without replacing the logger or its file handle.
    pub fn set_level(&self, level: Level) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.config.level = level;
        }
    }
}

fn build_record(level: Level, component: &str, message: &str, fields: Map<String, Value>) -> Value {
    let mut map = Map::new();
    map.insert(
        "ts".to_string(),
        Value::String(chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)),
    );
    map.insert(
        "level".to_string(),
        Value::String(level.as_str().to_string()),
    );
    map.insert(
        "component".to_string(),
        Value::String(component.to_string()),
    );
    map.insert("message".to_string(), Value::String(message.to_string()));
    if !fields.is_empty() {
        map.insert("fields".to_string(), Value::Object(fields));
    }
    Value::Object(map)
}

/// Shift `log.jsonl` → `log.1.jsonl` → … keeping at most
/// `max_rotated_files` rotated files plus the active one.
fn rotate(inner: &mut Inner) {
    let Some(path) = inner.config.file_path.clone() else {
        return;
    };
    inner.file = None;

    if inner.config.max_rotated_files == 0 {
        let _ = std::fs::remove_file(&path);
    } else {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let parent = path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
        let max = inner.config.max_rotated_files;

        let oldest = parent.join(format!("{name}.{max}.jsonl"));
        let _ = std::fs::remove_file(oldest);
        for i in (1..max).rev() {
            let from = parent.join(format!("{name}.{i}.jsonl"));
            let to = parent.join(format!("{name}.{}.jsonl", i + 1));
            let _ = std::fs::rename(from, to);
        }
        let first = parent.join(format!("{name}.1.jsonl"));
        let _ = std::fs::rename(&path, first);
    }

    inner.file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .ok();
    inner.written_bytes = 0;
}

static GLOBAL: OnceLock<Logger> = OnceLock::new();

/// Install the process-global logger. Later calls keep the first logger.
pub fn init(config: LoggerConfig) -> &'static Logger {
    GLOBAL.get_or_init(|| Logger::new(config).unwrap_or_else(|_| Logger::off()))
}

/// Access the global logger, initializing an inert one if needed.
pub fn global() -> &'static Logger {
    GLOBAL.get_or_init(Logger::off)
}

pub fn set_level(level: Level) {
    global().set_level(level);
}

pub fn error(component: &str, message: &str) {
    global().log(Level::Error, component, message);
}

pub fn warn(component: &str, message: &str) {
    global().log(Level::Warn, component, message);
}

pub fn info(component: &str, message: &str) {
    global().log(Level::Info, component, message);
}

pub fn debug(component: &str, message: &str) {
    global().log(Level::Debug, component, message);
}

/// Read a bounded, redacted tail from the active log and its rotated files.
/// Export code uses this instead of copying raw log files so a package cannot
/// bypass the logger's path and secret redaction policy.
pub fn recent_records(
    path: &Path,
    max_lines: usize,
    max_rotated_files: usize,
    redactions: &[String],
) -> Vec<Value> {
    if max_lines == 0 {
        return Vec::new();
    }

    let redactor = Redactor::new(redactions.to_vec());
    let mut records = VecDeque::with_capacity(max_lines);
    let mut paths = Vec::with_capacity(max_rotated_files.saturating_add(1));
    for index in (1..=max_rotated_files).rev() {
        paths.push(rotated_path(path, index));
    }
    paths.push(path.to_path_buf());

    for log_path in paths {
        let Ok(contents) = std::fs::read_to_string(log_path) else {
            continue;
        };
        for line in contents.lines() {
            let record = if line.len() > MAX_EXPORT_RECORD_BYTES {
                let truncated: String = line.chars().take(MAX_EXPORT_RECORD_BYTES).collect();
                Value::String(format!("[TRUNCATED] {}", redactor.redact(&truncated)))
            } else {
                serde_json::from_str::<Value>(line)
                    .map(|value| redactor.redact_value(&value))
                    .unwrap_or_else(|_| Value::String(redactor.redact(line)))
            };
            if records.len() == max_lines {
                records.pop_front();
            }
            records.push_back(record);
        }
    }

    records.into_iter().collect()
}

fn rotated_path(path: &Path, index: usize) -> PathBuf {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    parent.join(format!("{name}.{index}.jsonl"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::redaction;

    fn temp_log_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join("silk-diag-tests")
            .join(format!("{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn recent_records_are_bounded_and_redacted() {
        let dir = temp_log_dir("export");
        let path = dir.join("log.jsonl");
        let logger = Logger::new(LoggerConfig {
            file_path: Some(path.clone()),
            ..LoggerConfig::default()
        })
        .expect("logger");
        let secret = r"C:\Users\bob\Videos";
        logger.log(Level::Info, "test", &format!("saved {secret}\\clip.mp4"));
        logger.log(Level::Warn, "test", "second");

        let records = recent_records(&path, 1, DEFAULT_MAX_ROTATED_FILES, &[secret.to_string()]);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0]["message"], "second");

        let all = recent_records(
            &path,
            DEFAULT_EXPORT_LOG_LINES,
            DEFAULT_MAX_ROTATED_FILES,
            &[secret.to_string()],
        );
        assert_eq!(all.len(), 2);
        assert!(!all[0].to_string().contains(secret));
        assert!(all[0].to_string().contains("[REDACTED]"));
    }

    #[test]
    fn writes_parseable_json_lines() {
        let dir = temp_log_dir("parse");
        let path = dir.join("log.jsonl");
        let logger = Logger::new(LoggerConfig {
            file_path: Some(path.clone()),
            ..LoggerConfig::default()
        })
        .expect("logger");

        logger.log(Level::Info, "engine", "hello");
        let mut fields = Map::new();
        fields.insert("frames".to_string(), Value::from(30));
        logger.log_kv(Level::Warn, "engine", "dropped", fields);

        let contents = std::fs::read_to_string(&path).expect("read");
        let lines: Vec<&str> = contents.lines().collect();
        assert_eq!(lines.len(), 2);
        let parsed: Value = serde_json::from_str(lines[0]).expect("valid json");
        assert_eq!(parsed["level"], "INFO");
        assert_eq!(parsed["component"], "engine");
        assert_eq!(parsed["message"], "hello");
        let second: Value = serde_json::from_str(lines[1]).expect("valid json");
        assert_eq!(second["fields"]["frames"], 30);
    }

    #[test]
    fn level_filter_blocks_lower_levels() {
        let dir = temp_log_dir("filter");
        let path = dir.join("log.jsonl");
        let logger = Logger::new(LoggerConfig {
            level: Level::Warn,
            file_path: Some(path.clone()),
            ..LoggerConfig::default()
        })
        .expect("logger");

        logger.log(Level::Info, "x", "ignored");
        logger.log(Level::Error, "x", "kept");

        let contents = std::fs::read_to_string(&path).expect("read");
        assert_eq!(contents.lines().count(), 1);
        assert!(contents.contains("kept"));
    }

    #[test]
    fn rotation_keeps_bounded_files() {
        let dir = temp_log_dir("rotate");
        let path = dir.join("log.jsonl");
        let logger = Logger::new(LoggerConfig {
            file_path: Some(path.clone()),
            max_file_bytes: 200,
            max_rotated_files: 2,
            ..LoggerConfig::default()
        })
        .expect("logger");

        let big_line = "x".repeat(80);
        for _ in 0..12 {
            logger.log(Level::Error, "rotate-test", &big_line);
        }

        let rotated: Vec<_> = std::fs::read_dir(&dir)
            .expect("dir")
            .filter_map(std::result::Result::ok)
            .collect();
        assert!(
            rotated.len() <= 3,
            "expected at most active+2 rotated files, found {}",
            rotated.len()
        );
        assert!(path.exists());
        redaction::self_check();
    }

    #[test]
    fn redaction_applies_before_write() {
        let dir = temp_log_dir("redact");
        let path = dir.join("log.jsonl");
        let secret = r"C:\Users\alice42";
        let logger = Logger::new(LoggerConfig {
            file_path: Some(path.clone()),
            redactions: vec![secret.to_string()],
            ..LoggerConfig::default()
        })
        .expect("logger");

        logger.log(
            Level::Info,
            "cfg",
            &format!("loaded from {secret}\\config.json"),
        );

        let contents = std::fs::read_to_string(&path).expect("read");
        assert!(!contents.contains("alice42"), "leaked path: {contents}");
        assert!(contents.contains("[REDACTED]"));
    }
}
