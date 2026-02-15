use std::collections::HashMap;

/// Validation errors produced when parsing a context draft text block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContextValidationError {
    MissingDelimiter {
        line_number: usize,
        raw: String,
    },
    EmptyKey {
        line_number: usize,
    },
    DuplicateKey {
        key: String,
        first_line: usize,
        second_line: usize,
    },
    KeyTooLong {
        line_number: usize,
        len: usize,
    },
    ValueTooLong {
        key: String,
        len: usize,
    },
}

impl std::fmt::Display for ContextValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContextValidationError::MissingDelimiter { line_number, raw } => {
                write!(f, "line {line_number}: missing '=' delimiter (`{raw}`)")
            }
            ContextValidationError::EmptyKey { line_number } => {
                write!(f, "line {line_number}: key is empty")
            }
            ContextValidationError::DuplicateKey {
                key,
                first_line,
                second_line,
            } => write!(
                f,
                "duplicate key `{key}` (first on line {first_line}, repeated on line {second_line})"
            ),
            ContextValidationError::KeyTooLong { line_number, len } => {
                write!(f, "line {line_number}: key length {len} exceeds 128 bytes")
            }
            ContextValidationError::ValueTooLong { key, len } => {
                write!(f, "value `{key}` is too long ({len} bytes, limit 32768)")
            }
        }
    }
}

/// Serialize context pairs (`key=value`) with keys sorted deterministically.
pub fn serialize_pairs(pairs: &[(String, String)]) -> String {
    let mut ordered: Vec<_> = pairs.iter().collect();
    ordered.sort_by(|(a, _), (b, _)| a.cmp(b));
    if ordered.is_empty() {
        return String::new();
    }
    let mut lines = Vec::with_capacity(ordered.len());
    for (key, value) in ordered {
        lines.push(format!("{key}={value}"));
    }
    format!("{}\n", lines.join("\n"))
}

const MAX_KEY_LEN: usize = 128;
const MAX_VALUE_LEN: usize = 32 * 1024; // 32,768

/// Parse user-edited draft text into validated context pairs.
pub fn parse_draft_text(text: &str) -> Result<Vec<(String, String)>, Vec<ContextValidationError>> {
    let mut errors = Vec::new();
    let mut pairs = Vec::new();
    let mut seen_keys: HashMap<String, usize> = HashMap::new();

    for (index, line) in text.lines().enumerate() {
        let line_number = index + 1;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with('#') {
            continue;
        }
        let delimiter_pos = match line.find('=') {
            Some(pos) => pos,
            None => {
                errors.push(ContextValidationError::MissingDelimiter {
                    line_number,
                    raw: line.to_string(),
                });
                continue;
            }
        };

        let raw_key = &line[..delimiter_pos];
        let key = raw_key.trim();
        let key_len = key.len();
        let mut line_has_error = false;
        if key.is_empty() {
            errors.push(ContextValidationError::EmptyKey { line_number });
            line_has_error = true;
        }

        let value = &line[delimiter_pos + 1..];
        if value.len() > MAX_VALUE_LEN {
            errors.push(ContextValidationError::ValueTooLong {
                key: key.to_string(),
                len: value.len(),
            });
            line_has_error = true;
        }

        if key_len > MAX_KEY_LEN {
            errors.push(ContextValidationError::KeyTooLong {
                line_number,
                len: key_len,
            });
            line_has_error = true;
        }

        if line_has_error {
            continue;
        }

        let key_owned = key.to_string();
        if let Some(first_line) = seen_keys.get(&key_owned) {
            errors.push(ContextValidationError::DuplicateKey {
                key: key_owned.clone(),
                first_line: *first_line,
                second_line: line_number,
            });
            continue;
        }
        seen_keys.insert(key_owned.clone(), line_number);

        pairs.push((key_owned, value.to_string()));
    }

    if errors.is_empty() {
        Ok(pairs)
    } else {
        Err(errors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Vec<(String, String)> {
        parse_draft_text(text).unwrap()
    }

    #[test]
    fn serialize_round_trip() {
        let input = "b=2\na=1\n";
        let pairs = parse(input);
        assert_eq!(serialize_pairs(&pairs), "a=1\nb=2\n".to_string());
    }

    #[test]
    fn ignores_blank_and_comments() {
        let input = "\n# comment\nkey=val\n";
        assert_eq!(parse(input), vec![("key".into(), "val".into())]);
    }

    #[test]
    fn missing_delimiter_reported() {
        let err = parse_draft_text("no_delimiter").unwrap_err();
        assert!(matches!(
            err.as_slice(),
            [ContextValidationError::MissingDelimiter { .. }]
        ));
    }

    #[test]
    fn empty_key_reported() {
        let err = parse_draft_text(" =value").unwrap_err();
        assert!(matches!(
            err.as_slice(),
            [ContextValidationError::EmptyKey { line_number: 1 }]
        ));
    }

    #[test]
    fn duplicate_key_reports_lines() {
        let err = parse_draft_text("foo=1\nfoo=2").unwrap_err();
        assert!(matches!(
            err.as_slice(),
            [ContextValidationError::DuplicateKey { key, first_line: 1, second_line: 2 }] if key == "foo"
        ));
    }

    #[test]
    fn key_too_long_detected() {
        let long_key = "k".repeat(MAX_KEY_LEN + 1);
        let err = parse_draft_text(&format!("{long_key}=value")).unwrap_err();
        assert!(matches!(
            err.as_slice(),
            [ContextValidationError::KeyTooLong { line_number: 1, len }] if *len == MAX_KEY_LEN + 1
        ));
    }

    #[test]
    fn value_too_long_detected() {
        let long_value = "v".repeat(MAX_VALUE_LEN + 1);
        let err = parse_draft_text(&format!("key={long_value}")).unwrap_err();
        assert!(matches!(
            err.as_slice(),
            [ContextValidationError::ValueTooLong { key, len }] if key == "key" && *len == MAX_VALUE_LEN + 1
        ));
    }

    #[test]
    fn collects_multiple_errors() {
        let input = "bad\n=empty\nkey=val\nkey=value";
        let err = parse_draft_text(input).unwrap_err();
        assert_eq!(err.len(), 3);
    }
}
