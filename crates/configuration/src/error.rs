use std::fmt;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("configuration I/O error for {path}: {source}")]
    Io {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("invalid configuration in {path}: {message}")]
    InvalidJson {
        path: std::path::PathBuf,
        message: String,
    },

    #[error("unsupported configuration version {found} (supported up to {supported})")]
    UnsupportedVersion { found: u32, supported: u32 },

    #[error("configuration validation failed: {}", format_issues(.0))]
    Validation(Vec<FieldIssue>),

    #[error("configuration serialization failed: {0}")]
    Serialization(String),
}

/// One invalid setting, identified by its dotted/camelCase location in the
/// configuration document. The path is owned because collection members use
/// dynamic indices, for example `audio.tracks[1].deviceId`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldIssue {
    pub field: String,
    pub reason: String,
}

impl fmt::Display for FieldIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.field, self.reason)
    }
}

fn format_issues(issues: &[FieldIssue]) -> String {
    issues
        .iter()
        .map(|i| i.to_string())
        .collect::<Vec<_>>()
        .join("; ")
}
