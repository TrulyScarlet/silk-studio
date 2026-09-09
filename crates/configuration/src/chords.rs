/// Parse and normalize a hotkey chord such as `"Ctrl+Shift+F10"`.
///
/// Canonical form: modifiers uppercased in CTRL, ALT, SHIFT, WIN order,
/// followed by the key token (`A-Z`, `0-9`, `F1-F24`). This validator lives
/// in the configuration crate so settings can be validated before being
/// applied (spec §8.11); full registration happens in the `hotkeys`
/// crate during Segment S6.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChordError {
    Empty,
    MissingKey,
    DuplicateModifier(&'static str),
    UnknownToken(String),
}

impl ChordError {
    pub fn to_user_message(&self) -> String {
        match self {
            Self::Empty => "hotkey chord must not be empty".to_string(),
            Self::MissingKey => "hotkey chord must include a key".to_string(),
            Self::DuplicateModifier(m) => format!("modifier '{m}' appears more than once"),
            Self::UnknownToken(t) => {
                format!("'{t}' is not a recognized modifier or key")
            }
        }
    }
}

const MODIFIER_ORDER: [&str; 4] = ["CTRL", "ALT", "SHIFT", "WIN"];

fn modifier_rank(token: &str) -> Option<usize> {
    MODIFIER_ORDER.iter().position(|m| *m == token)
}

fn is_valid_key(token: &str) -> bool {
    if token.len() == 1
        && token
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric())
    {
        return true;
    }
    if let Some(rest) = token.strip_prefix('F') {
        if let Ok(n) = rest.parse::<u32>() {
            return (1..=24).contains(&n);
        }
    }
    matches!(
        token,
        "UP" | "DOWN" | "LEFT" | "RIGHT" | "SPACE" | "TAB" | "ENTER" | "ESC"
    )
}

pub fn normalize_chord(input: &str) -> Result<String, ChordError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(ChordError::Empty);
    }

    let mut modifiers = [false; MODIFIER_ORDER.len()];
    let mut key: Option<String> = None;

    for raw_token in trimmed.split('+') {
        let token = raw_token.trim().to_ascii_uppercase();
        if token.is_empty() {
            return Err(ChordError::UnknownToken(String::new()));
        }
        if let Some(rank) = modifier_rank(&token) {
            if modifiers[rank] {
                return Err(ChordError::DuplicateModifier(MODIFIER_ORDER[rank]));
            }
            modifiers[rank] = true;
        } else if key.is_some() {
            return Err(ChordError::UnknownToken(token));
        } else {
            key = Some(token);
        }
    }

    let key = key.ok_or(ChordError::MissingKey)?;
    if !is_valid_key(&key) {
        return Err(ChordError::UnknownToken(key));
    }

    let mut parts: Vec<String> = MODIFIER_ORDER
        .iter()
        .enumerate()
        .filter(|(rank, _)| modifiers[*rank])
        .map(|(_, name)| name.to_string())
        .collect();
    parts.push(key);
    Ok(parts.join("+"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_spec_example() {
        assert_eq!(normalize_chord("Ctrl+Shift+F10").unwrap(), "CTRL+SHIFT+F10");
        assert_eq!(normalize_chord(" shift + f9 ").unwrap(), "SHIFT+F9");
        assert_eq!(normalize_chord("Alt+Ctrl+A").unwrap(), "CTRL+ALT+A");
    }

    #[test]
    fn rejects_bad_chords() {
        assert_eq!(normalize_chord(""), Err(ChordError::Empty));
        assert_eq!(normalize_chord("Ctrl"), Err(ChordError::MissingKey));
        assert_eq!(
            normalize_chord("Ctrl+Ctrl+F5"),
            Err(ChordError::DuplicateModifier("CTRL"))
        );
        assert_eq!(
            normalize_chord("Ctrl++"),
            Err(ChordError::UnknownToken(String::new()))
        );
        assert_eq!(
            normalize_chord("Ctrl+Hyper+F5"),
            Err(ChordError::UnknownToken("F5".to_string()))
        );
        assert_eq!(
            normalize_chord("F25"),
            Err(ChordError::UnknownToken("F25".to_string()))
        );
    }

    #[test]
    fn accepts_function_and_special_keys() {
        assert!(normalize_chord("F12").is_ok());
        assert!(normalize_chord("Ctrl+DOWN").is_ok());
        assert!(normalize_chord("Win+NumpadX").is_err());
    }
}
