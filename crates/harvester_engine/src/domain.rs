//! Registrable-domain (eTLD+1) extraction, shared by the fetch engine and the
//! core blacklist reducer so both agree on what counts as "the same site".

use url::Url;

/// Returns the lowercased registrable domain (eTLD+1) of `url`, collapsing
/// subdomains (`www.bloomberg.com` -> `bloomberg.com`) and respecting
/// multi-label public suffixes (`bbc.co.uk`). Returns `None` for unparseable
/// URLs, hosts that are IP literals, or hosts without a registrable domain.
pub fn registrable_domain(url: &str) -> Option<String> {
    let parsed = Url::parse(url).ok()?;
    let host = parsed.host()?;

    // Reject IP literal hosts
    match host {
        url::Host::Ipv4(_) | url::Host::Ipv6(_) => return None,
        url::Host::Domain(_) => {}
    }

    let host_str = parsed.host_str()?;
    let suffix = psl::domain_str(host_str)?;
    Some(suffix.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collapses_subdomains() {
        assert_eq!(
            registrable_domain("https://www.bloomberg.com/news/articles/x"),
            Some("bloomberg.com".to_string())
        );
        assert_eq!(
            registrable_domain("https://finance.yahoo.com/markets/stocks"),
            Some("yahoo.com".to_string())
        );
    }

    #[test]
    fn respects_multi_label_suffix() {
        assert_eq!(
            registrable_domain("https://news.bbc.co.uk/story"),
            Some("bbc.co.uk".to_string())
        );
    }

    #[test]
    fn lowercases_host() {
        assert_eq!(
            registrable_domain("https://WWW.WSJ.COM/tech"),
            Some("wsj.com".to_string())
        );
    }

    #[test]
    fn rejects_invalid_and_ip_hosts() {
        assert_eq!(registrable_domain("not a url"), None);
        assert_eq!(registrable_domain("https://127.0.0.1/x"), None);
    }
}
