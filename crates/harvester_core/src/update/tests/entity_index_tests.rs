use super::*;

fn make_entity_index_with_company(
    url: &str,
    company: &str,
    fetched_utc: &str,
) -> crate::entity_index::EntityIndex {
    use crate::entity_index::{EntityIndex, EntityIndexEntry};
    use std::collections::BTreeMap;

    let mut entries = BTreeMap::new();
    entries.insert(
        url.to_string(),
        EntityIndexEntry {
            fetched_utc: Some(fetched_utc.to_string()),
            content_hash: Some("abc123".to_string()),
            companies: vec![company.to_string()],
            ..EntityIndexEntry::default()
        },
    );
    EntityIndex {
        schema_version: 1,
        entries,
    }
}

#[test]
fn entity_index_loaded_populates_trend_data() {
    init_logging();
    let state = AppState::default();
    let index =
        make_entity_index_with_company("https://example.com/a", "Nvidia", "2026-02-01T00:00:00Z");
    let (state, effects) = update(state, Msg::EntityIndexLoaded { index });
    assert!(effects.is_empty());
    let trend_data = state
        .entity_trend_data()
        .expect("entity_trend_data should be set");
    assert_eq!(
        trend_data.companies.weeks.len(),
        13,
        "should have 13 week buckets"
    );
}

#[test]
fn entity_index_load_failed_triggers_rebuild() {
    init_logging();
    let state = AppState::default();
    let (_, effects) = update(
        state,
        Msg::EntityIndexLoadFailed {
            reason: "file not found".to_string(),
        },
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::RebuildEntityIndex)),
        "EntityIndexLoadFailed should emit Effect::RebuildEntityIndex; got: {effects:?}"
    );
}

#[test]
fn trend_category_selected_updates_active_category_no_effects() {
    init_logging();
    let state = AppState::default();
    assert_eq!(
        state.active_trend_category(),
        crate::tabs::TrendCategory::Companies
    );
    let (state, effects) = update(
        state,
        Msg::TrendCategorySelected {
            category: crate::tabs::TrendCategory::Technologies,
        },
    );
    assert!(
        effects.is_empty(),
        "TrendCategorySelected should emit no effects"
    );
    assert_eq!(
        state.active_trend_category(),
        crate::tabs::TrendCategory::Technologies
    );
}
