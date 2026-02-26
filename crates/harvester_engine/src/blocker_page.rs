use url::Url;

pub(crate) fn detect_blocked_page(
    final_url: &str,
    title: Option<&str>,
    markdown: &str,
) -> Option<String> {
    if let Ok(parsed) = Url::parse(final_url) {
        let host = parsed.host_str().unwrap_or_default().to_ascii_lowercase();
        let path = parsed.path().to_ascii_lowercase();

        if host == "consent.yahoo.com" && path.contains("collectconsent") {
            return Some("yahoo consent interstitial".to_string());
        }

        if host.starts_with("consent.") && path.contains("consent") {
            return Some(format!("consent interstitial host={host}"));
        }

        if path.contains("client_captcha/challenge") || path.contains("captcha/challenge") {
            return Some("captcha challenge interstitial".to_string());
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
        return Some("browser verification interstitial".to_string());
    }

    if shortish
        && lower.contains("captcha")
        && (lower.contains("verify you are human")
            || lower.contains("security check")
            || lower.contains("request id")
            || lower.contains("ray id")
            || lower.contains("enable javascript and cookies"))
    {
        return Some("captcha interstitial content".to_string());
    }

    None
}

#[cfg(test)]
mod tests {
    use super::detect_blocked_page;

    #[test]
    fn detects_yahoo_consent_url() {
        let result = detect_blocked_page(
            "https://consent.yahoo.com/v2/collectConsent?sessionId=abc",
            Some("Yahoo"),
            "consent form",
        );
        assert!(matches!(
            result.as_deref(),
            Some("yahoo consent interstitial")
        ));
    }

    #[test]
    fn detects_captcha_challenge_url() {
        let result = detect_blocked_page(
            "https://www.telegraphherald.com/_services/v1/client_captcha/challenge?request=abc",
            Some("Captcha"),
            "challenge",
        );
        assert!(matches!(
            result.as_deref(),
            Some("captcha challenge interstitial")
        ));
    }

    #[test]
    fn detects_verification_interstitial_from_content() {
        let result = detect_blocked_page(
            "https://example.com/article",
            Some("Just a moment..."),
            "Please verify you are human before continuing.",
        );
        assert!(matches!(
            result.as_deref(),
            Some("browser verification interstitial")
        ));
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
