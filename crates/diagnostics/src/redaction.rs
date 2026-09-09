//! Substring redaction applied to serialized log lines before they reach
//! any sink (spec §22.2/§22.3: logs must not expose user paths or secrets).

/// Replaces every occurrence of each pattern with `[REDACTED]`.
#[derive(Debug, Clone)]
pub struct Redactor {
    patterns: Vec<String>,
}

impl Redactor {
    pub fn new(patterns: Vec<String>) -> Self {
        // Skip trivially short patterns; replacing them would corrupt lines
        // without protecting anything meaningful.
        let patterns = patterns
            .into_iter()
            .filter(|p| p.chars().count() > 4)
            .collect();
        Self { patterns }
    }

    pub fn patterns(&self) -> &[String] {
        &self.patterns
    }

    pub fn redact(&self, input: &str) -> String {
        let mut current = input.to_string();
        for pattern in &self.patterns {
            if current.contains(pattern.as_str()) {
                current = current.replace(pattern.as_str(), "[REDACTED]");
            }
        }
        current
    }

    /// Recursively redact every string inside a JSON value.
    ///
    /// Redaction must happen on raw strings *before* serialization:
    /// JSON escaping doubles backslashes, which would otherwise let
    /// Windows paths slip past substring matching.
    pub fn redact_value(&self, value: &serde_json::Value) -> serde_json::Value {
        use serde_json::Value;
        match value {
            Value::String(s) => Value::String(self.redact(s)),
            Value::Array(items) => {
                Value::Array(items.iter().map(|v| self.redact_value(v)).collect())
            }
            Value::Object(map) => Value::Object(
                map.iter()
                    .map(|(k, v)| (k.clone(), self.redact_value(v)))
                    .collect(),
            ),
            other => other.clone(),
        }
    }
}

/// Compile-time smoke check used by tests to prove the module is wired.
pub fn self_check() {
    let r = Redactor::new(vec!["C:\\Users\\someone".to_string()]);
    debug_assert_eq!(
        r.redact("path C:\\Users\\someone\\file"),
        "path [REDACTED]\\file"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn replaces_all_occurrences() {
        let r = Redactor::new(vec![r"C:\Users\bob".to_string()]);
        let out = r.redact(r"see C:\Users\bob\a and C:\Users\bob\b");
        assert_eq!(out, "see [REDACTED]\\a and [REDACTED]\\b");
    }

    #[test]
    fn ignores_short_patterns() {
        let r = Redactor::new(vec!["ab".to_string()]);
        assert_eq!(r.patterns().len(), 0);
        assert_eq!(r.redact("abc ab ab"), "abc ab ab");
    }

    #[test]
    fn redacts_nested_json_strings() {
        let r = Redactor::new(vec![r"C:\Users\bob".to_string()]);
        let input = json!({
            "path": r"C:\Users\bob\clip.mp4",
            "nested": { "list": [r"C:\Users\bob\a", 7, null] },
            "count": 3
        });
        let out = r.redact_value(&input);
        assert_eq!(out["path"], "[REDACTED]\\clip.mp4");
        assert_eq!(out["nested"]["list"][0], "[REDACTED]\\a");
        assert_eq!(out["nested"]["list"][1], 7);
        assert_eq!(out["count"], 3);
    }
}
