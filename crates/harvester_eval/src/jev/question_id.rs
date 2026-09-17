//! Stable Jev question identifier construction shared by request and response code.

/// Builds a stable question identifier from its kind and configured value.
pub fn question_id(kind: &str, value: &str) -> String {
    format!("{kind}__{}", sanitize_identifier(value))
}

/// Sanitises human vocabulary into a stable identifier component.
pub fn sanitize_identifier(value: &str) -> String {
    let mut result = String::new();
    let mut separator = false;
    for character in value.chars() {
        if character.is_ascii_alphanumeric() {
            if separator && !result.is_empty() {
                result.push('_');
            }
            result.push(character.to_ascii_lowercase());
            separator = false;
        } else if !result.is_empty() {
            separator = true;
        }
    }
    result
}
