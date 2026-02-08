use harvester_engine::{UrlPolicy, UrlPolicyViolation};
use url::Url;

fn expect_violation(url: &str, check: Result<(), UrlPolicyViolation>) -> UrlPolicyViolation {
    check.expect_err(&format!("policy should block {}", url))
}

#[test]
fn allows_http_and_https_by_default() {
    let policy = UrlPolicy::default();
    assert!(policy
        .check(&Url::parse("http://example.com").unwrap())
        .is_ok());
    assert!(policy
        .check(&Url::parse("https://example.com").unwrap())
        .is_ok());
}

#[test]
fn disallows_unlisted_scheme() {
    let policy = UrlPolicy::default();
    let violation = expect_violation(
        "ftp://example.com",
        policy.check(&Url::parse("ftp://example.com").unwrap()),
    );
    assert!(matches!(
        violation,
        UrlPolicyViolation::DisallowedScheme { .. }
    ));
}

#[test]
fn blocks_private_ipv4() {
    let policy = UrlPolicy::default();
    let violation = expect_violation(
        "http://127.0.0.1/",
        policy.check(&Url::parse("http://127.0.0.1/").unwrap()),
    );
    assert!(matches!(violation, UrlPolicyViolation::PrivateIp { .. }));
}

#[test]
fn blocks_cgnat_range() {
    let policy = UrlPolicy::default();
    let violation = expect_violation(
        "http://100.64.0.1/",
        policy.check(&Url::parse("http://100.64.0.1/").unwrap()),
    );
    assert!(matches!(violation, UrlPolicyViolation::PrivateIp { .. }));
}

#[test]
fn blocks_ipv6_loopback() {
    let policy = UrlPolicy::default();
    let violation = expect_violation(
        "http://[::1]/",
        policy.check(&Url::parse("http://[::1]/").unwrap()),
    );
    assert!(matches!(violation, UrlPolicyViolation::PrivateIp { .. }));
}

#[test]
fn dns_resolution_of_localhost_is_blocked() {
    let policy = UrlPolicy::default();
    let violation = expect_violation(
        "http://localhost:8080/test",
        policy.check(&Url::parse("http://localhost:8080/test").unwrap()),
    );
    assert!(matches!(violation, UrlPolicyViolation::PrivateIp { .. }));
}

#[test]
fn allows_private_when_disabled() {
    let mut policy = UrlPolicy::default();
    policy.block_private_ips = false;
    assert!(policy
        .check(&Url::parse("http://127.0.0.1").unwrap())
        .is_ok());
}

#[test]
fn enforces_allowed_hosts() {
    let mut policy = UrlPolicy::default();
    policy.allowed_hosts = Some(vec!["example.com".to_string()]);
    assert!(policy
        .check(&Url::parse("http://example.com").unwrap())
        .is_ok());
    let violation = expect_violation(
        "http://other.com/",
        policy.check(&Url::parse("http://other.com/").unwrap()),
    );
    assert!(matches!(violation, UrlPolicyViolation::BlockedHost { .. }));
}

#[test]
fn honors_blocked_hosts_list() {
    let mut policy = UrlPolicy::default();
    policy.blocked_hosts = vec!["evil.com".to_string()];
    let violation = expect_violation(
        "http://evil.com/",
        policy.check(&Url::parse("http://evil.com/").unwrap()),
    );
    assert!(matches!(violation, UrlPolicyViolation::BlockedHost { .. }));
}
