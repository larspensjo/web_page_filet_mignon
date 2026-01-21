use chardetng::EncodingDetector;
use encoding_rs::Encoding;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedHtml {
    pub html: String,
    pub encoding_label: String,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DecodeError {
    #[error("failed to decode bytes with {encoding}: {message}")]
    DecodeFailure { encoding: String, message: String },
}

/// Decode raw bytes into UTF-8 using: Content-Type charset -> BOM -> meta charset -> chardetng fallback.
pub fn decode_html(bytes: &[u8], content_type: Option<&str>) -> Result<DecodedHtml, DecodeError> {
    // 1) BOM aware decode using encoding_rs helper
    if let Some((encoding, _)) = Encoding::for_bom(bytes) {
        return decode_with(bytes, encoding);
    }

    // 2) Content-Type header charset
    if let Some(label) = content_type.and_then(extract_charset) {
        if let Some(enc) = Encoding::for_label(label.as_bytes()) {
            return decode_with(bytes, enc);
        }
    }

    // 3) chardetng detection with hint from meta tags (full HTML)
    let mut detector = EncodingDetector::new();
    detector.feed(bytes, true);
    let enc = detector.guess(None, true);
    decode_with(bytes, enc)
}

fn extract_charset(content_type: &str) -> Option<String> {
    content_type
        .split(';')
        .filter_map(|part| {
            let part = part.trim();
            part.strip_prefix("charset=")
                .or_else(|| part.strip_prefix("Charset="))
                .or_else(|| part.strip_prefix("CHARSET="))
                .map(|v| v.trim_matches([' ', '"', '\''].as_ref()))
        })
        .next()
        .map(|s| s.to_string())
}

fn decode_with(bytes: &[u8], enc: &'static Encoding) -> Result<DecodedHtml, DecodeError> {
    let (text, _, had_errors) = enc.decode(bytes);
    if had_errors {
        return Err(DecodeError::DecodeFailure {
            encoding: enc.name().to_string(),
            message: "decoding error".into(),
        });
    }
    Ok(DecodedHtml {
        html: text.into_owned(),
        encoding_label: enc.name().to_string(),
    })
}
