use harvester_core::{
    AutoVerdict, FilterReason, LoadedArticle, ManualDecision, PreTriagePhase, PreTriagePolicy,
    PreTriageSession,
};

fn article(url: &str, title: Option<&str>, body: &str) -> LoadedArticle {
    LoadedArticle {
        url: url.to_string(),
        source_title: title.map(str::to_string),
        prepared_text: body.to_string(),
        content_hash: format!("hash-{url}"),
        fetched_utc: None,
    }
}

#[test]
fn youtube_host_is_hard_excluded() {
    let policy = PreTriagePolicy::default();
    let (verdict, reasons) = policy.evaluate(&article(
        "https://youtube.com/watch?v=1",
        None,
        "long enough body text",
    ));
    assert_eq!(verdict, AutoVerdict::HardExclude);
    assert!(reasons.contains(&FilterReason::BlockedHost));
}

#[test]
fn very_small_content_is_hard_excluded() {
    let policy = PreTriagePolicy::default();
    let (verdict, reasons) = policy.evaluate(&article("https://example.com", None, "small"));
    assert_eq!(verdict, AutoVerdict::HardExclude);
    assert!(reasons.contains(&FilterReason::VerySmallContent));
}

#[test]
fn medium_small_content_requires_review() {
    let policy = PreTriagePolicy::default();
    let body = std::iter::repeat_n("contentword", 100)
        .collect::<Vec<_>>()
        .join(" ");
    let (verdict, reasons) = policy.evaluate(&article("https://example.com", None, &body));
    assert_eq!(verdict, AutoVerdict::Review);
    assert!(reasons.contains(&FilterReason::SmallMediumContent));
}

#[test]
fn boilerplate_density_requires_review() {
    let policy = PreTriagePolicy::default();
    let body = "continue reading enable cookies advertisement ".repeat(100);
    let (verdict, reasons) = policy.evaluate(&article("https://example.com", None, &body));
    assert_eq!(verdict, AutoVerdict::Review);
    assert!(reasons.contains(&FilterReason::BoilerplateDensity));
}

#[test]
fn link_density_requires_review() {
    let policy = PreTriagePolicy::default();
    let body = std::iter::repeat_n("[x](https://example.com) contentword", 40)
        .collect::<Vec<_>>()
        .join(" ");
    let (verdict, reasons) = policy.evaluate(&article("https://example.com", None, &body));
    assert_eq!(verdict, AutoVerdict::Review);
    assert!(reasons.contains(&FilterReason::HighLinkDensity));
}

#[test]
fn manual_include_overrides_hard_exclude() {
    let policy = PreTriagePolicy::default();
    let normal_body = std::iter::repeat_n("word", 300)
        .collect::<Vec<_>>()
        .join(" ");
    let mut session = PreTriageSession::load_articles(
        vec![
            article("https://youtube.com/watch?v=1", None, "x"),
            article("https://example.com/normal", None, &normal_body),
        ],
        &policy,
    );
    assert_eq!(session.phase(), &PreTriagePhase::ReadyToTriage);
    let youtube_key = session
        .entries()
        .iter()
        .find(|entry| entry.key.url.contains("youtube"))
        .expect("youtube entry should exist")
        .key
        .clone();
    assert!(session
        .set_manual_decision(&youtube_key, ManualDecision::Include)
        .is_ok());
    assert_eq!(session.phase(), &PreTriagePhase::ReadyToTriage);
    assert!(
        session
            .resolved_included_urls()
            .contains(&"https://youtube.com/watch?v=1".to_string()),
        "manual Include must override the hard-exclude verdict"
    );
}

#[test]
fn manual_exclude_overrides_auto_include() {
    let policy = PreTriagePolicy::default();
    let body = std::iter::repeat_n("word", 300)
        .collect::<Vec<_>>()
        .join(" ");
    let mut session =
        PreTriageSession::load_articles(vec![article("https://example.com", None, &body)], &policy);
    assert_eq!(session.phase(), &PreTriagePhase::ReadyToTriage);
    let key = session.entries()[0].key.clone();
    assert!(session
        .set_manual_decision(&key, ManualDecision::Exclude)
        .is_ok());
    assert!(matches!(session.phase(), PreTriagePhase::Failed { .. }));
}

#[test]
fn deterministic_reason_order() {
    let policy = PreTriagePolicy::default();
    let article = article(
        "https://youtube.com/watch?v=1",
        Some("Subscribe to read"),
        "small",
    );
    let (_verdict, reasons) = policy.evaluate(&article);
    assert_eq!(
        reasons,
        vec![
            FilterReason::BlockedHost,
            FilterReason::VerySmallContent,
            FilterReason::PaywallShellTitle
        ]
    );
}

#[test]
fn policy_with_no_review_entries_fast_paths_to_ready() {
    let policy = PreTriagePolicy::default();
    let body = std::iter::repeat_n("word", 300)
        .collect::<Vec<_>>()
        .join(" ");
    let session =
        PreTriageSession::load_articles(vec![article("https://example.com", None, &body)], &policy);
    assert_eq!(session.phase(), &PreTriagePhase::ReadyToTriage);
}

#[test]
fn zero_included_articles_produces_failed_phase() {
    let policy = PreTriagePolicy::default();
    let session = PreTriageSession::load_articles(
        vec![article("https://youtube.com/watch?v=1", None, "x")],
        &policy,
    );
    assert!(matches!(session.phase(), PreTriagePhase::Failed { .. }));
}

#[test]
fn corpus_fingerprint_changes_when_decisions_change() {
    let policy = PreTriagePolicy::default();
    let body = std::iter::repeat_n("contentword", 100)
        .collect::<Vec<_>>()
        .join(" ");
    let mut session =
        PreTriageSession::load_articles(vec![article("https://example.com", None, &body)], &policy);
    assert_eq!(session.phase(), &PreTriagePhase::ReadyToTriage);
    let before = session.corpus_fingerprint();
    let key = session.entries()[0].key.clone();
    session
        .set_manual_decision(&key, ManualDecision::Exclude)
        .expect("manual exclude");
    let after = session.corpus_fingerprint();
    assert_ne!(before, after);
}
