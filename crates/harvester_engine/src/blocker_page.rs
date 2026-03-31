use std::fmt;

use url::Url;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BlockedPageKind {
    YahooConsentInterstitial,
    ConsentInterstitial,
    CaptchaChallengeInterstitial,
    BrowserVerificationInterstitial,
    CaptchaInterstitialContent,
}

impl fmt::Display for BlockedPageKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BlockedPageKind::YahooConsentInterstitial => write!(f, "yahoo consent interstitial"),
            BlockedPageKind::ConsentInterstitial => write!(f, "consent interstitial"),
            BlockedPageKind::CaptchaChallengeInterstitial => {
                write!(f, "captcha challenge interstitial")
            }
            BlockedPageKind::BrowserVerificationInterstitial => {
                write!(f, "browser verification interstitial")
            }
            BlockedPageKind::CaptchaInterstitialContent => {
                write!(f, "captcha interstitial content")
            }
        }
    }
}

pub(crate) fn detect_blocked_page(
    final_url: &str,
    title: Option<&str>,
    markdown: &str,
) -> Option<BlockedPageKind> {
    if let Ok(parsed) = Url::parse(final_url) {
        let host = parsed.host_str().unwrap_or_default().to_ascii_lowercase();
        let path = parsed.path().to_ascii_lowercase();

        if host == "consent.yahoo.com" && path.contains("collectconsent") {
            return Some(BlockedPageKind::YahooConsentInterstitial);
        }

        if host.starts_with("consent.") && path.contains("consent") {
            return Some(BlockedPageKind::ConsentInterstitial);
        }

        if path.contains("client_captcha/challenge") || path.contains("captcha/challenge") {
            return Some(BlockedPageKind::CaptchaChallengeInterstitial);
        }
    }

    let mut combined = String::new();
    if let Some(title) = title {
        combined.push_str(title);
        combined.push('\n');
    }
    combined.push_str(markdown);
    let lower = combined.to_ascii_lowercase();
    let shortish = lower.chars().take(5000).count() == lower.chars().count();

    if shortish && lower.contains("just a moment") && lower.contains("verify you are human") {
        return Some(BlockedPageKind::BrowserVerificationInterstitial);
    }

    if shortish
        && lower.contains("captcha")
        && (lower.contains("verify you are human")
            || lower.contains("security check")
            || lower.contains("request id")
            || lower.contains("ray id")
            || lower.contains("enable javascript and cookies"))
    {
        return Some(BlockedPageKind::CaptchaInterstitialContent);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::{detect_blocked_page, BlockedPageKind};

    #[test]
    fn detects_yahoo_consent_url() {
        let result = detect_blocked_page(
            "https://consent.yahoo.com/v2/collectConsent?sessionId=abc",
            Some("Yahoo"),
            "consent form",
        );
        assert_eq!(result, Some(BlockedPageKind::YahooConsentInterstitial));
    }

    #[test]
    fn detects_generic_consent_url() {
        let result = detect_blocked_page(
            "https://consent.example.com/consent?sessionId=abc",
            Some("Consent"),
            "consent form",
        );
        assert_eq!(result, Some(BlockedPageKind::ConsentInterstitial));
    }

    #[test]
    fn detects_captcha_challenge_url() {
        let result = detect_blocked_page(
            "https://www.telegraphherald.com/_services/v1/client_captcha/challenge?request=abc",
            Some("Captcha"),
            "challenge",
        );
        assert_eq!(result, Some(BlockedPageKind::CaptchaChallengeInterstitial));
    }

    #[test]
    fn detects_verification_interstitial_from_content() {
        let result = detect_blocked_page(
            "https://example.com/article",
            Some("Just a moment..."),
            "Please verify you are human before continuing.",
        );
        assert_eq!(result, Some(BlockedPageKind::BrowserVerificationInterstitial));
    }

    #[test]
    fn detects_captcha_interstitial_from_content() {
        let result = detect_blocked_page(
            "https://example.com/article",
            Some("Security Check"),
            "Captcha required. Verify you are human and note the Ray ID before continuing.",
        );
        assert_eq!(result, Some(BlockedPageKind::CaptchaInterstitialContent));
    }

    #[test]
    fn ignores_normal_article_content() {
        let result = detect_blocked_page(
            "https://example.com/news/article",
            Some("How CAPTCHA startups are changing fraud prevention"),
            "This article discusses the history of captcha systems and challenge-response tests in security.",
        );
        assert!(result.is_none());
    }
}
