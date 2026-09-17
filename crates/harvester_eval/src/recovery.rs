//! Recovery of the exact document bytes embedded in a recorded user prompt.

use thiserror::Error;

use crate::prompt_identity::sha256;

/// The evidence recovered from the nonce-delimited document wrapper.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveredText {
    pub text: String,
    pub nonce: String,
    pub nonce_verified: bool,
}

/// A malformed recorded document wrapper.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum RecoveryError {
    #[error("missing document opening tag")]
    MissingOpenTag,
    #[error("missing document closing tag")]
    MissingCloseTag,
    #[error("opening and closing document nonces differ")]
    TagNonceMismatch,
}

/// Extracts the first nonce-delimited document from a rendered user message.
pub fn recover_document_text(rendered_user_message: &str) -> Result<RecoveredText, RecoveryError> {
    let start = rendered_user_message
        .find("<document-")
        .ok_or(RecoveryError::MissingOpenTag)?;
    let after_prefix = start + "<document-".len();
    let tag_end = rendered_user_message[after_prefix..]
        .find('>')
        .map(|offset| after_prefix + offset)
        .ok_or(RecoveryError::MissingOpenTag)?;
    let nonce = &rendered_user_message[after_prefix..tag_end];
    if nonce.len() != 12 || !nonce.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(RecoveryError::MissingOpenTag);
    }
    let body_start = tag_end + 1;
    let exact_closing_tag = format!("</document-{nonce}>");
    let closing_start =
        if let Some(offset) = rendered_user_message[body_start..].find(&exact_closing_tag) {
            body_start + offset
        } else if rendered_user_message[body_start..].contains("</document-") {
            return Err(RecoveryError::TagNonceMismatch);
        } else {
            return Err(RecoveryError::MissingCloseTag);
        };
    let mut text = &rendered_user_message[body_start..closing_start];
    if let Some(without_leading_newline) = text.strip_prefix('\n') {
        text = without_leading_newline;
    }
    if let Some(without_trailing_newline) = text.strip_suffix('\n') {
        text = without_trailing_newline;
    }
    Ok(RecoveredText {
        text: text.to_string(),
        nonce: nonce.to_string(),
        // TemplateVars hashes the original content and then removes literal
        // nonce occurrences. In that vanishingly unlikely case this is false,
        // which is deliberately reported instead of rejecting recovered text.
        nonce_verified: &sha256(text)[..12] == nonce,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use harvester_engine::llm::TemplateVars;

    #[test]
    fn round_trips_normal_document_bytes() {
        let mut vars = TemplateVars::new();
        vars.set_document("content", "article\ntext");
        let wrapped = vars.to_map().remove("content").unwrap();
        let recovered = recover_document_text(&wrapped).unwrap();
        assert_eq!(recovered.text, "article\ntext");
        assert!(recovered.nonce_verified);
    }

    #[test]
    fn ignores_close_like_article_content_and_requires_a_twelve_hex_nonce() {
        let mut vars = TemplateVars::new();
        vars.set_document("content", "article </document-abc> text");
        let wrapped = vars.to_map().remove("content").unwrap();
        assert_eq!(
            recover_document_text(&wrapped).unwrap().text,
            "article </document-abc> text"
        );
        assert_eq!(
            recover_document_text("<document-abc>\ntext\n</document-abc>").unwrap_err(),
            RecoveryError::MissingOpenTag
        );
    }
}
