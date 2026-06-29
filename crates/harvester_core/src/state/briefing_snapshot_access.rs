use super::AppState;
use crate::briefing_snapshot::{
    build_briefing_snapshot, BriefingSnapshot, SnapshotArticle, BRIEFING_SNAPSHOT_BUDGET_BYTES,
};

impl AppState {
    /// Assemble the frozen briefing snapshot from the triaged base corpus
    /// (duplicates included), pulling each article's cached summary and applying
    /// the active coverage window.
    #[allow(dead_code)]
    pub(crate) fn build_briefing_snapshot_now(&self) -> BriefingSnapshot {
        let coverage_window_label =
            crate::briefing::format_briefing_time_window_label(self.briefing_since_utc());
        let ordered = self.archive_corpus().ordered_urls().to_vec();

        let summaries: Vec<Option<crate::briefing::ArticleSummaryResult>> = ordered
            .iter()
            .map(|url| self.summary_result_for_url(url).cloned())
            .collect();

        let articles: Vec<SnapshotArticle<'_>> = ordered
            .iter()
            .zip(summaries.iter())
            .map(|(url, summary)| SnapshotArticle {
                url: url.as_str(),
                fetched_utc: self.triage().fetched_utc_for_url(url),
                summary: summary.as_ref(),
            })
            .collect();

        build_briefing_snapshot(
            &articles,
            self.briefing_since_utc(),
            BRIEFING_SNAPSHOT_BUDGET_BYTES,
            coverage_window_label,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::briefing::{ArticleSummaryResult, LoadedArticle};
    use crate::summary_cache::{context_hash, SummaryCacheKey};
    use crate::triage::{ArticleTriageResult, TriageSession};
    use harvester_engine::llm::prompt::PromptId;

    fn article(url: &str, content_hash: &str) -> LoadedArticle {
        LoadedArticle {
            url: url.to_string(),
            source_title: None,
            prepared_text: "body".to_string(),
            content_hash: content_hash.to_string(),
            fetched_utc: None,
        }
    }

    fn completed_triage(articles: Vec<LoadedArticle>) -> TriageSession {
        let mut triage = TriageSession::new_loading(None);
        triage.set_articles(articles);
        triage.transition_to_triaging();
        for idx in 0..triage.articles().len() {
            triage.complete_article(
                idx,
                ArticleTriageResult {
                    category: "tech".to_string(),
                    priority: 3,
                    tags: vec![],
                    rationale: "r".to_string(),
                    input_tokens: 0,
                    output_tokens: 0,
                },
            );
        }
        triage.complete();
        triage
    }

    fn summary(title: &str) -> ArticleSummaryResult {
        ArticleSummaryResult {
            title: title.to_string(),
            summary: "Summary".to_string(),
            key_points: vec![],
            input_tokens: 1,
            output_tokens: 1,
            entities: Default::default(),
        }
    }

    fn cache_key(content_hash_value: &str) -> SummaryCacheKey {
        SummaryCacheKey {
            content_hash: content_hash_value.to_string(),
            prompt_id: PromptId::ArticleSummary,
            prompt_version: 1,
            model_id: "test-model".to_string(),
            context_hash: context_hash(&[]),
        }
    }

    #[test]
    fn snapshot_uses_full_base_corpus_including_duplicates() {
        let mut state = AppState::new();
        state.set_triage(completed_triage(vec![
            article("https://example.com/a", "same-hash"),
            article("https://example.com/b", "same-hash"),
        ]));
        state.store_summary_result(
            cache_key("same-hash"),
            summary("Duplicate summary"),
            "2026-06-01T00:00:00Z".to_string(),
        );

        let snap = state.build_briefing_snapshot_now();

        assert_eq!(
            snap.included_count,
            state.archive_corpus().ordered_urls().len(),
            "snapshot must walk the full base corpus when all summaries are settled"
        );
        assert!(snap.text.contains("[A1] Duplicate summary"));
        assert!(snap.text.contains("[A2] Duplicate summary"));
    }
}
