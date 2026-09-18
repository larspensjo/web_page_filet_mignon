use super::*;

#[cfg(test)]
mod app_state_tests {
    use super::*;
    use crate::{
        update, BatchNextAction, BatchStatus, JobListMode, ManualDecision, Msg,
        PreTriageActionability, PreTriagePolicy, PreTriageSession, SelectedJobVisibility,
        SignalCandidateOutcome, SignalCandidateRow, DESKTOP_JOB_LIST_MAX_ROWS,
    };
    use harvester_engine::{ExtractedLink, LinkKind};

    #[test]
    fn job_done_success_stores_preview() {
        let mut state = AppState::new();
        state.jobs.insert(
            1,
            JobState {
                url: "https://example.com".to_string(),
                stage: Stage::Queued,
                ..Default::default()
            },
        );
        state.apply_done(
            1,
            JobResultKind::Success,
            Some("preview content".to_string()),
            Vec::new(),
            None,
        );
        let job = state.jobs.get(&1).expect("job exists");
        assert_eq!(job.content_preview(), Some("preview content"));
    }

    #[test]
    fn batch_observation_poll_in_progress_tracks_source_poll_state() {
        let mut state = AppState::new();
        assert!(!state.batch_observation().poll_in_progress);

        assert!(state.start_poll());
        assert!(state.batch_observation().poll_in_progress);

        state.end_poll();
        assert!(!state.batch_observation().poll_in_progress);
    }

    #[test]
    fn batch_observation_counts_queued_jobs_as_in_flight() {
        let mut state = AppState::new();
        state.jobs.insert(
            1,
            JobState {
                url: "https://queued.example".to_string(),
                stage: Stage::Queued,
                outcome: None,
                ..Default::default()
            },
        );
        state.jobs.insert(
            2,
            JobState {
                url: "https://done.example".to_string(),
                stage: Stage::Done,
                outcome: Some(JobResultKind::Success),
                ..Default::default()
            },
        );
        state.jobs.insert(
            3,
            JobState {
                url: "https://failed.example".to_string(),
                stage: Stage::Done,
                outcome: Some(JobResultKind::Failed {
                    reason: "boom".to_string(),
                }),
                ..Default::default()
            },
        );

        let obs = state.batch_observation();
        assert_eq!(obs.jobs_total, 3);
        assert_eq!(obs.jobs_in_flight, 1);
        assert_eq!(obs.jobs_done, 1);
        assert_eq!(obs.jobs_failed, 1);
    }

    #[test]
    fn settled_poll_article_jobs_clear_pipeline_tracker() {
        let mut state = AppState::new();
        assert!(state.start_poll());
        for job_id in [1, 2] {
            state.jobs.insert(
                job_id,
                JobState {
                    url: format!("https://example.com/{job_id}"),
                    stage: Stage::Queued,
                    outcome: None,
                    ..Default::default()
                },
            );
        }
        state.record_poll_pipeline_jobs(&[1, 2]);
        state.end_poll();
        assert_eq!(state.poll_pipeline_job_total(), Some(2));

        state.apply_done(1, JobResultKind::Success, None, Vec::new(), None);
        assert_eq!(state.poll_pipeline_job_total(), Some(2));

        state.apply_done(2, JobResultKind::Success, None, Vec::new(), None);
        assert_eq!(state.poll_pipeline_job_total(), None);
    }

    fn article_with_words(url: &str, word_count: usize) -> crate::LoadedArticle {
        crate::LoadedArticle {
            url: url.to_string(),
            source_title: None,
            prepared_text: std::iter::repeat_n("contentword", word_count)
                .collect::<Vec<_>>()
                .join(" "),
            content_hash: format!("hash-{url}"),
            fetched_utc: None,
        }
    }

    #[test]
    fn pre_triage_actionability_is_ready_when_pre_triage_is_ready() {
        let mut state = AppState::new();
        let pre_triage = PreTriageSession::load_articles(
            vec![article_with_words("https://example.com/ready", 220)],
            &PreTriagePolicy::default(),
        );
        state.set_pre_triage(pre_triage);

        assert_eq!(
            state.pre_triage_actionability(),
            PreTriageActionability::Ready
        );
        assert!(state.can_start_triage_from_pre_triage());
    }

    #[test]
    fn pre_triage_actionability_is_ready_with_pending_review_when_reviewing() {
        let mut state = AppState::new();
        let mut pre_triage = PreTriageSession::load_articles(
            vec![
                article_with_words("https://example.com/review-a", 100),
                article_with_words("https://example.com/review-b", 100),
            ],
            &PreTriagePolicy::default(),
        );
        let key = pre_triage
            .entry_for_url("https://example.com/review-a")
            .expect("review entry exists")
            .key
            .clone();
        pre_triage
            .set_manual_decision(&key, ManualDecision::Exclude)
            .expect("interactive reviewing should accept manual decisions");
        state.set_pre_triage(pre_triage);

        assert_eq!(
            state.pre_triage_actionability(),
            PreTriageActionability::ReadyWithPendingReview
        );
        assert!(state.can_start_triage_from_pre_triage());
    }

    #[test]
    fn batch_next_action_dispatches_triage_from_reviewing() {
        let mut state = AppState::new();
        let mut pre_triage = PreTriageSession::load_articles(
            vec![
                article_with_words("https://example.com/review-a", 100),
                article_with_words("https://example.com/review-b", 100),
            ],
            &PreTriagePolicy::default(),
        );
        let key = pre_triage
            .entry_for_url("https://example.com/review-a")
            .expect("review entry exists")
            .key
            .clone();
        pre_triage
            .set_manual_decision(&key, ManualDecision::Exclude)
            .expect("interactive reviewing should accept manual decisions");
        state.set_pre_triage(pre_triage);

        assert_eq!(state.batch_next_action(), BatchNextAction::DispatchTriage);
        assert_eq!(state.batch_status(), BatchStatus::Settled);
    }

    #[test]
    fn batch_status_is_running_when_pre_triage_load_is_in_flight() {
        let mut state = AppState::new();
        state.set_pre_triage(PreTriageSession::new_loading());

        assert_eq!(state.batch_status(), BatchStatus::Running);
    }

    #[test]
    fn batch_status_is_running_when_summary_article_load_is_in_flight() {
        let mut state = AppState::new();
        state.set_briefing(crate::briefing::BriefingSession::new_loading(None));

        assert_eq!(state.batch_status(), BatchStatus::Running);
    }

    #[test]
    fn job_done_failure_clears_preview() {
        let mut state = AppState::new();
        state.jobs.insert(
            2,
            JobState {
                url: "https://example.com".to_string(),
                stage: Stage::Queued,
                content_preview: Some("old preview".to_string()),
                ..Default::default()
            },
        );
        state.apply_done(
            2,
            JobResultKind::Failed {
                reason: "ignored".to_string(),
            },
            Some("ignored".to_string()),
            Vec::new(),
            None,
        );
        let job = state.jobs.get(&2).expect("job exists");
        assert_eq!(job.content_preview(), None);
    }

    #[test]
    fn selecting_job_with_preview_updates_view_model() {
        let mut state = AppState::new();
        state.jobs.insert(
            3,
            JobState {
                url: "https://example.com/path".to_string(),
                stage: Stage::Done,
                content_preview: Some("preview content".to_string()),
                ..Default::default()
            },
        );
        let (state, _) = update(state, Msg::JobSelected { job_id: 3 });
        let view = state.view();
        assert!(view
            .preview_text
            .as_deref()
            .unwrap_or("")
            .contains("No Analysis Available Yet"));
        assert_eq!(view.preview_header.as_ref().unwrap().domain, "example.com");
    }

    #[test]
    fn selecting_job_without_preview_only_sets_header() {
        let mut state = AppState::new();
        state.jobs.insert(
            4,
            JobState {
                url: "http://sub.example.net/a".to_string(),
                stage: Stage::Downloading,
                ..Default::default()
            },
        );
        let (state, _) = update(state, Msg::JobSelected { job_id: 4 });
        let view = state.view();
        assert!(view
            .preview_text
            .as_deref()
            .unwrap_or("")
            .contains("No Analysis Available Yet"));
        let header = view.preview_header.expect("header should exist");
        assert_eq!(header.domain, "sub.example.net");
        assert_eq!(header.stage, Stage::Downloading);
    }

    #[test]
    fn selecting_same_job_twice_only_sets_dirty_once() {
        let mut state = AppState::new();
        state.jobs.insert(
            5,
            JobState {
                url: "https://repeat.example".to_string(),
                stage: Stage::Done,
                content_preview: Some("d".to_string()),
                ..Default::default()
            },
        );
        let (state, _) = update(state, Msg::JobSelected { job_id: 5 });
        let mut state = state;
        assert!(state.consume_dirty());
        let (state, _) = update(state, Msg::JobSelected { job_id: 5 });
        let mut state = state;
        assert!(!state.consume_dirty());
    }

    #[test]
    fn domain_from_url_handles_various_inputs() {
        assert_eq!(domain_from_url("https://example.com/"), "example.com");
        assert_eq!(domain_from_url("http://foo.bar/baz?qux"), "foo.bar");
        assert_eq!(domain_from_url("example.org/path"), "example.org");
        assert_eq!(domain_from_url(""), "");
    }

    #[test]
    fn job_progress_with_preview_updates_selected_preview() {
        let mut state = AppState::new();
        state.jobs.insert(
            6,
            JobState {
                url: "https://partial.example".to_string(),
                stage: Stage::Downloading,
                ..Default::default()
            },
        );

        let (state, _) = update(state, Msg::JobSelected { job_id: 6 });
        let (state, _) = update(
            state,
            Msg::JobProgress {
                job_id: 6,
                stage: Stage::Converting,
                tokens: None,
                bytes: None,
                content_preview: Some("live content".to_string()),
            },
        );

        let view = state.view();
        assert_eq!(view.preview_text, Some("live content".to_string()));
        let job = state.jobs.get(&6).expect("job exists");
        assert_eq!(job.content_preview(), Some("live content"));
    }

    #[test]
    fn job_progress_with_preview_stores_content_when_not_selected() {
        let mut state = AppState::new();
        state.jobs.insert(
            7,
            JobState {
                url: "https://unselected.example".to_string(),
                stage: Stage::Downloading,
                ..Default::default()
            },
        );

        let (state, _) = update(
            state,
            Msg::JobProgress {
                job_id: 7,
                stage: Stage::Converting,
                tokens: None,
                bytes: None,
                content_preview: Some("background content".to_string()),
            },
        );

        let view = state.view();
        assert_eq!(view.preview_text, None);
        let job = state.jobs.get(&7).expect("job exists");
        assert_eq!(job.content_preview(), Some("background content"));
    }

    #[test]
    fn job_done_after_inprogress_promotes_preview_to_available() {
        let mut state = AppState::new();
        state.jobs.insert(
            8,
            JobState {
                url: "https://final.example".to_string(),
                stage: Stage::Downloading,
                ..Default::default()
            },
        );

        let (state, _) = update(state, Msg::JobSelected { job_id: 8 });
        let (state, _) = update(
            state,
            Msg::JobProgress {
                job_id: 8,
                stage: Stage::Converting,
                tokens: None,
                bytes: None,
                content_preview: Some("partial".to_string()),
            },
        );
        let (state, _) = update(
            state,
            Msg::JobDone {
                job_id: 8,
                result: JobResultKind::Success,
                content_preview: Some("final".to_string()),
                extracted_links: Vec::new(),
                fetched_utc: None,
            },
        );

        let view = state.view();
        assert!(view
            .preview_text
            .unwrap()
            .contains("No Analysis Available Yet"));
        let header = view.preview_header.expect("header present");
        assert_eq!(header.stage, Stage::Done);
    }

    #[test]
    fn preview_quality_counts_headings_and_skips_nav_indicator_when_low_density() {
        let content =
            "# Title\n## Section\nBody text with a [link](http://example.com).\nMore words here.";
        let quality = PreviewQuality::from_markdown(content);
        assert_eq!(quality.heading_count, 2);
        assert!(!quality.nav_heavy());
    }

    #[test]
    fn preview_quality_marks_nav_heavy_when_link_density_high() {
        let content = "[a](x) [b](x) [c](x) [d](x) [e](x)";
        let quality = PreviewQuality::from_markdown(content);
        assert!(quality.nav_heavy());
    }

    #[test]
    fn job_done_success_stores_normalized_links() {
        let mut state = AppState::new();
        state.jobs.insert(
            9,
            JobState {
                url: "https://link.example".to_string(),
                stage: Stage::Downloading,
                ..Default::default()
            },
        );

        let links = vec![
            ExtractedLink {
                url: "HTTP://EXAMPLE.com".to_string(),
                text: None,
                kind: LinkKind::Hyperlink,
            },
            ExtractedLink {
                url: "http://example.com/".to_string(),
                text: None,
                kind: LinkKind::Hyperlink,
            },
            ExtractedLink {
                url: "https://other.example:443/path".to_string(),
                text: None,
                kind: LinkKind::Hyperlink,
            },
        ];
        let (state, _) = update(
            state,
            Msg::JobDone {
                job_id: 9,
                result: JobResultKind::Success,
                content_preview: None,
                extracted_links: links,
                fetched_utc: None,
            },
        );

        let job = state.jobs.get(&9).expect("job exists");
        let stored_links = job.links();
        assert_eq!(stored_links.len(), 2);
        assert_eq!(stored_links[0].url, "http://example.com/".to_string());
        assert_eq!(stored_links[0].index, 0);
        assert_eq!(
            stored_links[1].url,
            "https://other.example/path".to_string()
        );
        assert_eq!(stored_links[1].index, 2);
    }

    #[test]
    fn collect_indirect_links_filters_navigation_and_share_noise() {
        let mut state = AppState::new();
        state.jobs.insert(
            9,
            JobState {
                url: "https://mashable.com/article/april-5-microsoft-windows-11-pro".to_string(),
                stage: Stage::Done,
                origin: JobOrigin::Direct,
                links: vec![
                    LinkRecord {
                        index: 0,
                        url: "https://mashable.com/tech".to_string(),
                        anchor_text: None,
                        kind: LinkKind::Hyperlink,
                        download_state: LinkDownloadState::NotDownloaded,
                        age_estimate: None,
                    },
                    LinkRecord {
                        index: 1,
                        url: "https://twitter.com/intent/tweet?url=https://mashable.com/article/april-5-microsoft-windows-11-pro".to_string(),
                        anchor_text: None,
                        kind: LinkKind::Hyperlink,
                        download_state: LinkDownloadState::NotDownloaded,
                        age_estimate: None,
                    },
                    LinkRecord {
                        index: 2,
                        url: "https://example.com/news/follow-up-story".to_string(),
                        anchor_text: None,
                        kind: LinkKind::Hyperlink,
                        download_state: LinkDownloadState::NotDownloaded,
                        age_estimate: None,
                    },
                ],
                ..Default::default()
            },
        );

        state.begin_indirect_link_generation();
        state.collect_indirect_links_from_job(9);

        let drained = state.drain_indirect_links();
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].url, "https://example.com/news/follow-up-story");
    }

    #[test]
    fn indirect_link_pool_dedupes_normalized_urls() {
        let mut pool = IndirectLinkPool::new();

        assert!(pool.add_link(IndirectLink {
            url: "https://Example.com/path/".to_string(),
            source_job_id: 1,
        }));
        assert!(!pool.add_link(IndirectLink {
            url: "https://example.com/path".to_string(),
            source_job_id: 2,
        }));
        assert_eq!(pool.len(), 1);
    }

    #[test]
    fn cache_starts_empty() {
        let state = AppState::new();
        assert_eq!(state.summary_cache().len(), 0);
    }

    #[test]
    fn store_and_retrieve_summary_result() {
        use crate::briefing::ArticleSummaryResult;
        use crate::summary_cache::SummaryCacheKey;
        use harvester_engine::llm::prompt::PromptId;

        let mut state = AppState::new();
        let key = SummaryCacheKey {
            content_hash: "hash1".to_string(),
            prompt_id: PromptId::ArticleSummary,
            prompt_version: 1,
            model_id: "model1".to_string(),
            context_hash: "ctx1".to_string(),
        };
        let result = ArticleSummaryResult {
            title: "Test Title".to_string(),
            summary: "Test Summary".to_string(),
            key_points: vec!["Point 1".to_string()],
            input_tokens: 100,
            output_tokens: 50,
            entities: Default::default(),
        };

        state.store_summary_result(
            key.clone(),
            result.clone(),
            "2026-01-01T00:00:00Z".to_string(),
        );

        let retrieved = state.try_reuse_summary(&key);
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().title, "Test Title");
        assert_eq!(retrieved.unwrap().summary, "Test Summary");
        assert_eq!(state.summary_cache().len(), 1);
    }

    #[test]
    fn briefing_complete_then_job_selected_shows_the_selected_summary() {
        use crate::briefing::{ArticleSummaryResult, BriefingItem, LoadedArticle};

        let mut state = AppState::new();
        state.jobs.insert(
            1,
            JobState {
                url: "https://example.com/article".to_string(),
                stage: Stage::Done,
                outcome: Some(JobResultKind::Success),
                ..Default::default()
            },
        );

        let mut briefing = crate::briefing::BriefingSession::new_loading(None);
        briefing.set_articles(
            vec![LoadedArticle {
                url: "https://example.com/article".to_string(),
                source_title: None,
                prepared_text: "text".to_string(),
                content_hash: "hash".to_string(),
                fetched_utc: None,
            }],
            "collection".to_string(),
        );
        briefing.transition_to_summarizing();
        briefing.start_article(0, 1);
        briefing.complete_article(
            0,
            ArticleSummaryResult {
                title: "Article Title".to_string(),
                summary: "Article summary text".to_string(),
                key_points: vec![],
                input_tokens: 10,
                output_tokens: 5,
                entities: Default::default(),
            },
        );
        briefing.start_stream(
            "[A1] Article Title\nArticle summary text".to_string(),
            "win".to_string(),
            1,
            0,
            0,
            false,
        );
        briefing.enter_streaming("Executive summary".to_string());
        briefing.append_stream_item(BriefingItem {
            headline: "Story 1".to_string(),
            body: "desc".to_string(),
        });
        state.set_briefing(briefing);

        state.select_job(1);
        let view = state.view();
        assert!(view
            .preview_text
            .as_deref()
            .unwrap_or("")
            .contains("Article summary text"));
    }

    #[test]
    fn cache_miss_returns_none() {
        use crate::summary_cache::SummaryCacheKey;
        use harvester_engine::llm::prompt::PromptId;

        let state = AppState::new();
        let key = SummaryCacheKey {
            content_hash: "nonexistent".to_string(),
            prompt_id: PromptId::ArticleSummary,
            prompt_version: 1,
            model_id: "model".to_string(),
            context_hash: "ctx".to_string(),
        };

        assert!(state.try_reuse_summary(&key).is_none());
    }

    #[test]
    fn set_summary_cache_replaces_entire_cache() {
        use crate::briefing::ArticleSummaryResult;
        use crate::summary_cache::{SummaryCache, SummaryCacheEntry, SummaryCacheKey};
        use harvester_engine::llm::prompt::PromptId;

        let mut state = AppState::new();

        let key1 = SummaryCacheKey {
            content_hash: "hash1".to_string(),
            prompt_id: PromptId::ArticleSummary,
            prompt_version: 1,
            model_id: "model1".to_string(),
            context_hash: "ctx1".to_string(),
        };
        let result1 = ArticleSummaryResult {
            title: "Title1".to_string(),
            summary: "Summary1".to_string(),
            key_points: vec![],
            input_tokens: 10,
            output_tokens: 5,
            entities: Default::default(),
        };
        state.store_summary_result(key1.clone(), result1, "2026-01-01T00:00:00Z".to_string());
        assert_eq!(state.summary_cache().len(), 1);

        let mut new_cache = SummaryCache::new();
        let key2 = SummaryCacheKey {
            content_hash: "hash2".to_string(),
            prompt_id: PromptId::ArticleSummary,
            prompt_version: 1,
            model_id: "model2".to_string(),
            context_hash: "ctx2".to_string(),
        };
        let entry2 = SummaryCacheEntry {
            result: ArticleSummaryResult {
                title: "Title2".to_string(),
                summary: "Summary2".to_string(),
                key_points: vec![],
                input_tokens: 20,
                output_tokens: 10,
                entities: Default::default(),
            },
            created_at_utc: "2026-01-02T00:00:00Z".to_string(),
        };
        new_cache.insert(key2.clone(), entry2);

        state.set_summary_cache(new_cache);

        assert_eq!(state.summary_cache().len(), 1);
        assert!(state.try_reuse_summary(&key1).is_none());
        assert_eq!(state.try_reuse_summary(&key2).unwrap().title, "Title2");
    }

    fn make_state_with_summarized_job() -> AppState {
        use crate::briefing::{ArticleSummaryResult, LoadedArticle};
        let mut state = AppState::new();
        state.jobs.insert(
            10,
            JobState {
                url: "https://summarized.example/article".to_string(),
                stage: Stage::Done,
                outcome: Some(JobResultKind::Success),
                ..Default::default()
            },
        );
        let mut briefing = crate::briefing::BriefingSession::new_loading(None);
        briefing.set_articles(
            vec![LoadedArticle {
                url: "https://summarized.example/article".to_string(),
                source_title: None,
                prepared_text: "text".to_string(),
                content_hash: "hash".to_string(),
                fetched_utc: None,
            }],
            "collection".to_string(),
        );
        briefing.transition_to_summarizing();
        briefing.start_article(0, 1);
        briefing.complete_article(
            0,
            ArticleSummaryResult {
                title: "My Title".to_string(),
                summary: "My summary".to_string(),
                key_points: vec!["Point A".to_string()],
                input_tokens: 10,
                output_tokens: 5,
                entities: Default::default(),
            },
        );
        state.set_briefing(briefing);
        state
    }

    fn make_state_with_cached_summary_job() -> AppState {
        use crate::briefing::{ArticleSummaryResult, LoadedArticle};
        use crate::summary_cache::SummaryCacheKey;
        use crate::triage::{ArticleTriageResult, TriageSession};
        use harvester_engine::llm::prompt::PromptId;

        let mut state = AppState::new();
        state.jobs.insert(
            11,
            JobState {
                url: "https://cached-summary.example/article".to_string(),
                stage: Stage::Done,
                outcome: Some(JobResultKind::Success),
                ..Default::default()
            },
        );

        let mut triage = TriageSession::new_loading(None);
        triage.set_articles(vec![LoadedArticle {
            url: "https://cached-summary.example/article".to_string(),
            source_title: Some("Cached Outlet".to_string()),
            prepared_text: "text".to_string(),
            content_hash: "cached-hash".to_string(),
            fetched_utc: None,
        }]);
        triage.transition_to_triaging();
        triage.complete_article(
            0,
            ArticleTriageResult {
                category: "Business".to_string(),
                priority: 5,
                tags: vec!["ai".to_string()],
                rationale: "Relevant".to_string(),
                input_tokens: 10,
                output_tokens: 5,
            },
        );
        state.set_triage(triage);

        let summary_key = SummaryCacheKey::try_new(
            "cached-hash",
            PromptId::ArticleSummary,
            Some(1),
            Some("cached-summary-model"),
            &[],
        )
        .expect("summary cache key");
        state.store_summary_result(
            summary_key,
            ArticleSummaryResult {
                title: "Cached Title".to_string(),
                summary: "Cached summary".to_string(),
                key_points: vec!["Cached point".to_string()],
                input_tokens: 11,
                output_tokens: 7,
                entities: Default::default(),
            },
            "2026-05-25T12:01:00Z".to_string(),
        );

        state
    }

    #[test]
    fn selecting_job_with_summary_shows_formatted_summary() {
        let mut state = make_state_with_summarized_job();
        state.select_job(10);
        let view = state.view();
        let text = view.preview_text.unwrap_or_default();
        assert!(text.contains("My summary"));
        assert!(text.contains("Point A"));
        assert!(text.contains("## Key Points"));
        assert!(!text.contains("**Key Points:**"));
    }

    #[test]
    fn selecting_job_without_summary_shows_placeholder() {
        let mut state = AppState::new();
        state.jobs.insert(
            11,
            JobState {
                url: "https://no-summary.example".to_string(),
                stage: Stage::Done,
                outcome: Some(JobResultKind::Success),
                ..Default::default()
            },
        );
        state.select_job(11);
        let view = state.view();
        let text = view.preview_text.unwrap_or_default();
        assert!(text.contains("No Analysis Available Yet"));
    }

    #[test]
    fn selecting_job_sets_preview_mode_to_selected_job_summary() {
        let mut state = make_state_with_summarized_job();
        state.select_job(10);
        use crate::briefing::BriefingItem;
        let mut s2 = make_state_with_summarized_job();
        s2.briefing_mut()
            .start_stream("[A1] T\nd".to_string(), "win".to_string(), 1, 0, 0, false);
        s2.briefing_mut()
            .enter_streaming("Exec summary".to_string());
        s2.briefing_mut().append_stream_item(BriefingItem {
            headline: "T".to_string(),
            body: "d".to_string(),
        });
        s2.select_job(10);
        let view = s2.view();
        let text = view.preview_text.unwrap_or_default();
        assert!(text.contains("My summary"), "should show summary");
    }

    #[test]
    fn format_summary_omits_title_heading_and_includes_summary_and_key_points() {
        use crate::briefing::ArticleSummaryResult;
        let result = ArticleSummaryResult {
            title: "Test Title".to_string(),
            summary: "Test summary body".to_string(),
            key_points: vec!["KP1".to_string(), "KP2".to_string()],
            input_tokens: 0,
            output_tokens: 0,
            entities: Default::default(),
        };
        let formatted = preview::format_summary_for_preview(&result);
        assert!(!formatted.starts_with("# "));
        assert!(formatted.contains("Test summary body"));
        assert!(formatted.contains("KP1"));
        assert!(formatted.contains("KP2"));
        assert!(formatted.contains("Key Points"));
    }

    #[test]
    fn format_summary_omits_key_points_section_when_empty() {
        use crate::briefing::ArticleSummaryResult;
        let result = ArticleSummaryResult {
            title: "Title Only".to_string(),
            summary: "Summary only".to_string(),
            key_points: vec![],
            input_tokens: 0,
            output_tokens: 0,
            entities: Default::default(),
        };
        let formatted = preview::format_summary_for_preview(&result);
        assert!(formatted.contains("Summary only"));
        assert!(!formatted.contains("Key Points"));
    }

    #[test]
    fn selected_article_url_returns_url_when_summarized_job_selected() {
        let mut state = make_state_with_summarized_job();
        state.select_job(10);
        let url = state.selected_article_url();
        assert_eq!(url, Some("https://summarized.example/article".to_string()));
    }

    #[test]
    fn selected_article_url_returns_none_when_no_summary() {
        let mut state = AppState::new();
        state.jobs.insert(
            12,
            JobState {
                url: "https://no-summary.example".to_string(),
                stage: Stage::Done,
                ..Default::default()
            },
        );
        state.select_job(12);
        assert!(state.selected_article_url().is_none());
    }

    #[test]
    fn selected_article_url_returns_none_when_no_selection() {
        let state = AppState::new();
        assert!(state.selected_article_url().is_none());
    }

    #[test]
    fn view_has_summary_true_for_completed_articles() {
        let state = make_state_with_summarized_job();
        let view = state.view();
        let job = view
            .desktop_job_list
            .rows
            .iter()
            .find(|j| j.job_id == 10)
            .expect("job 10 exists");
        assert!(job.has_summary);
        assert_eq!(job.summary_title.as_deref(), Some("My Title"));
    }

    #[test]
    fn view_has_summary_false_before_briefing() {
        let mut state = AppState::new();
        state.jobs.insert(
            13,
            JobState {
                url: "https://no-briefing.example".to_string(),
                stage: Stage::Done,
                ..Default::default()
            },
        );
        let view = state.view();
        let job = view
            .desktop_job_list
            .rows
            .iter()
            .find(|j| j.job_id == 13)
            .expect("job 13 exists");
        assert!(!job.has_summary);
        assert!(job.summary_title.is_none());
    }

    #[test]
    fn view_uses_cached_summary_without_briefing_session() {
        let state = make_state_with_cached_summary_job();
        let view = state.view();
        let job = view
            .desktop_job_list
            .rows
            .iter()
            .find(|j| j.job_id == 11)
            .expect("job 11 exists");
        assert!(job.has_summary);
        assert_eq!(job.summary_title.as_deref(), Some("Cached Title"));
    }

    #[test]
    fn view_selected_url_populated_when_summarized_job_selected() {
        let mut state = make_state_with_summarized_job();
        state.select_job(10);
        let view = state.view();
        assert_eq!(
            view.selected_url,
            Some("https://summarized.example/article".to_string())
        );
    }

    #[test]
    fn selected_article_url_uses_cached_summary_without_briefing_session() {
        let mut state = make_state_with_cached_summary_job();
        state.select_job(11);
        assert_eq!(
            state.selected_article_url(),
            Some("https://cached-summary.example/article".to_string())
        );
        assert!(state
            .summary_result_for_url("https://cached-summary.example/article")
            .is_some());
    }

    #[test]
    fn view_selected_url_none_when_unsummarized_job_selected() {
        let mut state = AppState::new();
        state.jobs.insert(
            14,
            JobState {
                url: "https://unsummarized.example".to_string(),
                stage: Stage::Done,
                ..Default::default()
            },
        );
        state.select_job(14);
        let view = state.view();
        assert_eq!(
            view.selected_url,
            Some("https://unsummarized.example".to_string())
        );
    }

    #[test]
    fn view_selected_url_none_when_no_selection() {
        let state = make_state_with_summarized_job();
        let view = state.view();
        assert!(view.selected_url.is_none());
    }

    #[test]
    fn ai_warning_banner_present_for_missing_api_key() {
        let mut state = AppState::new();
        state.set_ai_availability(AiAvailability::Unavailable {
            reason: AiUnavailableReason::MissingApiKey,
        });

        let view = state.view();
        assert_eq!(
            view.ai_warning_banner,
            Some(crate::InlineWarningView {
                title: "AI features are disabled".to_string(),
                body: "Set OPENAI_API_KEY in the launch environment and restart to enable triage and briefing.".to_string(),
            })
        );
        assert_eq!(
            view.triage_blocked_reason,
            Some("AI setup is incomplete because OPENAI_API_KEY is not set".to_string())
        );
        assert_eq!(
            view.briefing_blocked_reason,
            Some("AI setup is incomplete because OPENAI_API_KEY is not set".to_string())
        );
        assert_eq!(
            view.right_pane.triage_markdown,
            Some(
                "AI setup required\n\nTriage is disabled because `OPENAI_API_KEY` is not set.\n\nSet `OPENAI_API_KEY` in the launch environment and restart the app to enable article triage.".to_string()
            )
        );
    }

    #[test]
    fn ai_warning_banner_absent_when_ai_available() {
        let state = AppState::new();

        let view = state.view();
        assert!(view.ai_warning_banner.is_none());
    }

    #[test]
    fn provider_alert_banner_shown_when_ai_available() {
        let mut state = AppState::new();
        state.note_provider_out_of_credits("provider quota exhausted: billing".into());

        let view = state.view();
        let banner = view.ai_warning_banner.expect("provider warning banner");
        assert!(banner.title.contains("credits"));
    }

    #[test]
    fn missing_api_key_banner_takes_priority_over_provider_alert() {
        let mut state = AppState::new();
        state.set_ai_availability(AiAvailability::Unavailable {
            reason: AiUnavailableReason::MissingApiKey,
        });
        state.note_provider_out_of_credits("provider quota exhausted: billing".into());

        let view = state.view();
        let banner = view.ai_warning_banner.expect("missing-key warning banner");
        assert_eq!(banner.title, "AI features are disabled");
    }

    #[test]
    fn ai_warning_banner_absent_for_non_key_ai_unavailability() {
        let mut state = AppState::new();
        state.set_ai_availability(AiAvailability::Unavailable {
            reason: AiUnavailableReason::NoTriageModel,
        });

        let view = state.view();
        assert!(view.ai_warning_banner.is_none());
        assert_eq!(
            view.right_pane.triage_markdown,
            Some("Article triage is unavailable because no triage model is available.".to_string())
        );
    }

    #[test]
    fn stop_button_disables_when_session_is_running_but_work_has_settled() {
        use crate::briefing::LoadedArticle;
        use crate::triage::{ArticleTriageResult, TriageSession};

        let mut state = AppState::new();
        state.start_session();

        let mut triage = TriageSession::new_loading(None);
        triage.set_articles(vec![LoadedArticle {
            url: "https://example.com/1".to_string(),
            source_title: None,
            prepared_text: "text".to_string(),
            content_hash: "hash-1".to_string(),
            fetched_utc: None,
        }]);
        triage.transition_to_triaging();
        triage.complete_article(
            0,
            ArticleTriageResult {
                category: "News".to_string(),
                priority: 5,
                tags: Vec::new(),
                rationale: "ok".to_string(),
                input_tokens: 1,
                output_tokens: 1,
            },
        );
        triage.complete();
        state.set_triage(triage);

        let view = state.view();
        assert_eq!(view.session, SessionState::Running);
        assert!(
            !view.stop_finish_button.is_enabled(),
            "settled triage should not leave Stop / Finish active"
        );
    }

    #[test]
    fn stop_button_enables_when_running_session_has_in_flight_jobs() {
        let mut state = AppState::new();
        state.start_session();
        state.jobs.insert(
            1,
            JobState {
                url: "https://queued.example".to_string(),
                stage: Stage::Queued,
                outcome: None,
                ..Default::default()
            },
        );

        let view = state.view();
        assert!(view.stop_finish_button.is_enabled());
    }

    #[test]
    fn ingest_urls_reports_enqueued_job_ids() {
        let mut state = AppState::new();

        let ingest = state.ingest_urls(
            vec![
                "https://example.com/one".to_string(),
                "https://example.com/two".to_string(),
            ],
            chrono::Utc::now(),
        );

        assert_eq!(ingest.enqueued, 2);
        assert_eq!(ingest.enqueued_job_ids, vec![1, 2]);
    }

    #[test]
    fn resolve_preview_prefers_summary_over_triage() {
        use crate::briefing::{ArticleSummaryResult, LoadedArticle};
        use crate::triage::ArticleTriageResult;

        let mut state = AppState::new();
        let url = "https://test.example/article";

        let mut briefing = crate::briefing::BriefingSession::new_loading(None);
        briefing.set_articles(
            vec![LoadedArticle {
                url: url.to_string(),
                source_title: None,
                prepared_text: "text".to_string(),
                content_hash: "hash".to_string(),
                fetched_utc: None,
            }],
            "collection".to_string(),
        );
        briefing.transition_to_summarizing();
        briefing.start_article(0, 1);
        briefing.complete_article(
            0,
            ArticleSummaryResult {
                title: "Test Summary".to_string(),
                summary: "Summary text".to_string(),
                key_points: vec![],
                input_tokens: 10,
                output_tokens: 5,
                entities: Default::default(),
            },
        );
        state.set_briefing(briefing);

        let mut triage = crate::triage::TriageSession::new_loading(None);
        triage.set_articles(vec![LoadedArticle {
            url: url.to_string(),
            source_title: Some("Source headline".to_string()),
            prepared_text: "text".to_string(),
            content_hash: "hash".to_string(),
            fetched_utc: None,
        }]);
        triage.transition_to_triaging();
        triage.start_article(0, 1);
        triage.complete_article(
            0,
            ArticleTriageResult {
                category: "Security".to_string(),
                priority: 7,
                tags: vec![],
                rationale: "Test rationale".to_string(),
                input_tokens: 10,
                output_tokens: 5,
            },
        );
        state.set_triage(triage);

        let (kind, content) = state.resolve_best_preview(url);
        assert_eq!(kind, PreviewContentKind::Summary);
        assert!(content.contains("Summary text"));
    }

    #[test]
    fn resolve_preview_uses_triage_when_summary_missing() {
        use crate::briefing::LoadedArticle;
        use crate::triage::ArticleTriageResult;

        let mut state = AppState::new();
        let url = "https://test.example/article";

        let mut triage = crate::triage::TriageSession::new_loading(None);
        triage.set_articles(vec![LoadedArticle {
            url: url.to_string(),
            source_title: Some("Source headline".to_string()),
            prepared_text: "text".to_string(),
            content_hash: "hash".to_string(),
            fetched_utc: None,
        }]);
        triage.transition_to_triaging();
        triage.start_article(0, 1);
        triage.complete_article(
            0,
            ArticleTriageResult {
                category: "Security".to_string(),
                priority: 7,
                tags: vec!["test-tag".to_string()],
                rationale: "Test rationale".to_string(),
                input_tokens: 10,
                output_tokens: 5,
            },
        );
        state.set_triage(triage);

        let (kind, content) = state.resolve_best_preview(url);
        assert_eq!(kind, PreviewContentKind::Triage);
        assert!(content.contains("# Source headline"));
        assert!(content.contains("Security · Priority P7"));
        assert!(content.contains("## Why It Matters"));
        assert!(content.contains("Test rationale"));
    }

    #[test]
    fn job_filter_status_reads_pre_triage_state_without_building_view() {
        use crate::briefing::LoadedArticle;
        use crate::pre_triage_filter::PreTriagePolicy;

        let prepared_text = std::iter::repeat_n("useful reporting text", 70)
            .collect::<Vec<_>>()
            .join(" ");

        let mut state = AppState::new();
        state.jobs.insert(
            12,
            JobState {
                url: "https://job.example".to_string(),
                stage: Stage::Done,
                ..Default::default()
            },
        );

        let pre_triage = PreTriageSession::load_articles(
            vec![
                LoadedArticle {
                    url: "https://job.example".to_string(),
                    source_title: None,
                    prepared_text: prepared_text.clone(),
                    content_hash: "hash".to_string(),
                    fetched_utc: None,
                },
                LoadedArticle {
                    url: "https://job-2.example".to_string(),
                    source_title: None,
                    prepared_text,
                    content_hash: "hash-2".to_string(),
                    fetched_utc: None,
                },
            ],
            &PreTriagePolicy::default(),
        );
        state.set_pre_triage(pre_triage);

        assert_eq!(
            state.job_filter_status(12),
            Some(JobFilterStatus::AutoIncluded)
        );
    }

    #[test]
    fn resolve_preview_uses_fallback_when_nothing_available() {
        let state = AppState::new();
        let url = "https://test.example/article";

        let (kind, content) = state.resolve_best_preview(url);
        assert_eq!(kind, PreviewContentKind::Fallback);
        assert!(content.contains("No Analysis Available Yet"));
    }

    #[test]
    fn resolve_preview_returns_correct_kind() {
        use crate::briefing::{ArticleSummaryResult, LoadedArticle};

        let mut state = AppState::new();
        let url = "https://test.example/article";

        let mut briefing = crate::briefing::BriefingSession::new_loading(None);
        briefing.set_articles(
            vec![LoadedArticle {
                url: url.to_string(),
                source_title: None,
                prepared_text: "text".to_string(),
                content_hash: "hash".to_string(),
                fetched_utc: None,
            }],
            "collection".to_string(),
        );
        briefing.transition_to_summarizing();
        briefing.start_article(0, 1);
        briefing.complete_article(
            0,
            ArticleSummaryResult {
                title: "Test".to_string(),
                summary: "Summary".to_string(),
                key_points: vec![],
                input_tokens: 10,
                output_tokens: 5,
                entities: Default::default(),
            },
        );
        state.set_briefing(briefing);

        let (kind, _) = state.resolve_best_preview(url);
        assert_eq!(kind, PreviewContentKind::Summary);
    }

    #[path = "../signal_candidate_tests.rs"]
    mod signal_candidate_tests;

    fn insert_done_job(state: &mut AppState, job_id: JobId, url: &str) {
        state.jobs.insert(
            job_id,
            JobState {
                url: url.to_string(),
                stage: Stage::Done,
                outcome: Some(JobResultKind::Success),
                ..Default::default()
            },
        );
    }

    fn utc(ts: &str) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339(ts)
            .expect("valid rfc3339 test timestamp")
            .with_timezone(&chrono::Utc)
    }

    fn set_summary_titles(state: &mut AppState, titles: &[(&str, &str)]) {
        use crate::briefing::{ArticleSummaryResult, LoadedArticle};

        let mut briefing = crate::briefing::BriefingSession::new_loading(None);
        briefing.set_articles(
            titles
                .iter()
                .map(|(url, _)| LoadedArticle {
                    url: (*url).to_string(),
                    source_title: None,
                    prepared_text: "text".to_string(),
                    content_hash: format!("hash-{url}"),
                    fetched_utc: None,
                })
                .collect(),
            "collection".to_string(),
        );
        briefing.transition_to_summarizing();
        for (idx, (_, title)) in titles.iter().enumerate() {
            briefing.start_article(idx, (idx + 1) as u64);
            briefing.complete_article(
                idx,
                ArticleSummaryResult {
                    title: (*title).to_string(),
                    summary: "summary".to_string(),
                    key_points: vec![],
                    input_tokens: 1,
                    output_tokens: 1,
                    entities: Default::default(),
                },
            );
        }
        state.set_briefing(briefing);
    }

    fn set_fetched_utc(
        state: &mut AppState,
        fetched: &[(JobId, Option<chrono::DateTime<chrono::Utc>>)],
    ) {
        for (job_id, timestamp) in fetched {
            state.jobs.get_mut(job_id).expect("job exists").fetched_utc = *timestamp;
        }
    }

    fn set_triage_annotations(state: &mut AppState, annotations: &[(JobId, u8)]) {
        use crate::briefing::LoadedArticle;
        use crate::triage::{ArticleTriageResult, TriageSession};

        let mut triage = TriageSession::new_loading(None);
        triage.set_articles(
            annotations
                .iter()
                .map(|(job_id, _)| LoadedArticle {
                    url: state.jobs.get(job_id).expect("job exists").url.clone(),
                    source_title: None,
                    prepared_text: "test article".to_string(),
                    content_hash: format!("test-hash-{job_id}"),
                    fetched_utc: None,
                })
                .collect(),
        );
        triage.transition_to_triaging();
        for (article_id, (_, priority)) in annotations.iter().enumerate() {
            triage.complete_article(
                article_id,
                ArticleTriageResult {
                    category: "test".to_string(),
                    priority: *priority,
                    tags: Vec::new(),
                    rationale: "test".to_string(),
                    input_tokens: 1,
                    output_tokens: 1,
                },
            );
        }
        triage.complete();
        state.set_triage(triage);
    }

    #[test]
    fn desktop_list_orders_rows_by_priority_descending_with_p5_first() {
        let mut state = AppState::new();
        insert_done_job(&mut state, 1, "https://example.com/low");
        insert_done_job(&mut state, 2, "https://example.com/high");
        insert_done_job(&mut state, 3, "https://example.com/mid");
        set_triage_annotations(&mut state, &[(1, 2), (2, 5), (3, 3)]);

        let view = state.view();

        assert_eq!(
            view.desktop_job_list
                .rows
                .iter()
                .map(|row| row.job_id)
                .collect::<Vec<_>>(),
            vec![2, 3, 1]
        );
    }

    #[test]
    fn desktop_list_places_unannotated_rows_last() {
        let mut state = AppState::new();
        insert_done_job(&mut state, 1, "https://example.com/annotated");
        insert_done_job(&mut state, 2, "https://example.com/unannotated");
        insert_done_job(&mut state, 3, "https://example.com/lower");
        set_triage_annotations(&mut state, &[(1, 2), (3, 1)]);

        let view = state.view();

        assert_eq!(
            view.desktop_job_list
                .rows
                .iter()
                .map(|row| row.job_id)
                .collect::<Vec<_>>(),
            vec![1, 3, 2]
        );
        assert!(view
            .desktop_job_list
            .rows
            .last()
            .unwrap()
            .triage_annotation
            .is_none());
    }

    #[test]
    fn desktop_list_breaks_priority_ties_by_job_id() {
        let mut state = AppState::new();
        insert_done_job(&mut state, 9, "https://example.com/nine");
        insert_done_job(&mut state, 4, "https://example.com/four");
        set_triage_annotations(&mut state, &[(9, 4), (4, 4)]);

        let view = state.view();

        assert_eq!(
            view.desktop_job_list
                .rows
                .iter()
                .map(|row| row.job_id)
                .collect::<Vec<_>>(),
            vec![4, 9]
        );
    }

    #[test]
    fn desktop_list_falls_back_to_selection_order_while_triage_is_running() {
        use crate::briefing::LoadedArticle;
        use crate::triage::{ArticleTriageResult, TriageSession};

        let mut state = AppState::new();
        insert_done_job(&mut state, 1, "https://example.com/first");
        insert_done_job(&mut state, 2, "https://example.com/high");

        let mut triage = TriageSession::new_loading(None);
        triage.set_articles(vec![LoadedArticle {
            url: "https://example.com/high".to_string(),
            source_title: None,
            prepared_text: "test article".to_string(),
            content_hash: "test-hash-high".to_string(),
            fetched_utc: None,
        }]);
        triage.transition_to_triaging();
        triage.complete_article(
            0,
            ArticleTriageResult {
                category: "test".to_string(),
                priority: 5,
                tags: Vec::new(),
                rationale: "test".to_string(),
                input_tokens: 1,
                output_tokens: 1,
            },
        );
        state.set_triage(triage);

        let view = state.view();

        assert!(view.triage_results_reorder_suppressed);
        assert_eq!(
            view.desktop_job_list
                .rows
                .iter()
                .map(|row| row.job_id)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );

        state.triage_mut().complete();
        let settled_view = state.view();
        assert!(!settled_view.triage_results_reorder_suppressed);
        assert_eq!(
            settled_view
                .desktop_job_list
                .rows
                .iter()
                .map(|row| row.job_id)
                .collect::<Vec<_>>(),
            vec![2, 1]
        );
    }

    #[test]
    fn desktop_list_scopes_to_since_checkpoint_by_default() {
        let mut state = AppState::new();
        state.briefing_since_utc = Some(utc("2026-05-01T00:00:00Z"));
        insert_done_job(&mut state, 1, "https://example.com/new");
        insert_done_job(&mut state, 2, "https://example.com/old");
        set_fetched_utc(
            &mut state,
            &[
                (1, Some(utc("2026-05-02T00:00:00Z"))),
                (2, Some(utc("2026-04-30T00:00:00Z"))),
            ],
        );

        let view = state.view();

        assert_eq!(view.desktop_job_list.mode, JobListMode::SinceCheckpoint);
        assert_eq!(
            view.desktop_job_list
                .rows
                .iter()
                .map(|row| row.job_id)
                .collect::<Vec<_>>(),
            vec![1]
        );
    }

    #[test]
    fn desktop_list_last_24_hours_includes_window_boundary_and_future_jobs() {
        let now = utc("2026-05-02T12:00:00Z");
        let mut state = AppState::new();
        for (job_id, url) in [
            (1, "https://example.com/boundary"),
            (2, "https://example.com/outside"),
            (3, "https://example.com/exactly-now"),
            (4, "https://example.com/future"),
        ] {
            insert_done_job(&mut state, job_id, url);
        }
        set_fetched_utc(
            &mut state,
            &[
                (1, Some(now - chrono::Duration::hours(24))),
                (
                    2,
                    Some(now - chrono::Duration::hours(24) - chrono::Duration::seconds(1)),
                ),
                (3, Some(now)),
                (4, Some(now + chrono::Duration::hours(1))),
            ],
        );
        state.job_list_mode = JobListMode::Last24Hours;
        state = update(state, Msg::tick_at(now)).0;

        assert_eq!(
            state
                .view()
                .desktop_job_list
                .rows
                .iter()
                .map(|row| row.job_id)
                .collect::<Vec<_>>(),
            vec![1, 3, 4]
        );
    }

    #[test]
    fn desktop_list_last_24_hours_ignores_checkpoint() {
        let now = utc("2026-05-02T12:00:00Z");
        let mut state = AppState::new();
        insert_done_job(
            &mut state,
            1,
            "https://example.com/recent-before-checkpoint",
        );
        insert_done_job(&mut state, 2, "https://example.com/old-after-checkpoint");
        set_fetched_utc(
            &mut state,
            &[
                (1, Some(now - chrono::Duration::hours(2))),
                (2, Some(now - chrono::Duration::hours(25))),
            ],
        );
        state.briefing_since_utc = Some(now - chrono::Duration::hours(1));

        let since_view = state.view();
        assert_eq!(
            since_view
                .desktop_job_list
                .rows
                .iter()
                .map(|row| row.job_id)
                .collect::<Vec<_>>(),
            Vec::<JobId>::new()
        );

        state.job_list_mode = JobListMode::Last24Hours;
        state = update(state, Msg::tick_at(now)).0;
        assert_eq!(
            state
                .view()
                .desktop_job_list
                .rows
                .iter()
                .map(|row| row.job_id)
                .collect::<Vec<_>>(),
            vec![1]
        );

        let mut old_state = AppState::new();
        insert_done_job(
            &mut old_state,
            1,
            "https://example.com/old-after-checkpoint",
        );
        set_fetched_utc(
            &mut old_state,
            &[(1, Some(now - chrono::Duration::hours(25)))],
        );
        old_state.briefing_since_utc = Some(now - chrono::Duration::hours(26));
        old_state.job_list_mode = JobListMode::SinceCheckpoint;
        assert_eq!(old_state.view().desktop_job_list.rows.len(), 1);
        old_state.job_list_mode = JobListMode::Last24Hours;
        old_state = update(old_state, Msg::tick_at(now)).0;
        assert!(old_state.view().desktop_job_list.rows.is_empty());
    }

    #[test]
    fn desktop_list_last_24_hours_excludes_and_counts_jobs_without_fetch_time() {
        let now = utc("2026-05-02T12:00:00Z");
        let mut state = AppState::new();
        insert_done_job(&mut state, 1, "https://example.com/fetched");
        insert_done_job(&mut state, 2, "https://example.com/missing");
        set_fetched_utc(&mut state, &[(1, Some(now)), (2, None)]);
        state.job_list_mode = JobListMode::Last24Hours;
        state = update(state, Msg::tick_at(now)).0;

        let view = state.view();
        assert_eq!(view.desktop_job_list.rows.len(), 1);
        assert_eq!(
            view.desktop_job_list
                .rows
                .iter()
                .map(|row| row.job_id)
                .collect::<Vec<_>>(),
            vec![1]
        );
        assert_eq!(view.desktop_job_list.hidden_without_fetch_time, 1);
    }

    #[test]
    fn desktop_list_last_24_hours_is_empty_before_first_tick() {
        let mut state = AppState::new();
        insert_done_job(&mut state, 1, "https://example.com/fetched");
        insert_done_job(&mut state, 2, "https://example.com/missing");
        set_fetched_utc(
            &mut state,
            &[(1, Some(utc("2026-05-02T12:00:00Z"))), (2, None)],
        );
        state.job_list_mode = JobListMode::Last24Hours;

        let view = state.view();
        assert!(view.desktop_job_list.rows.is_empty());
        assert_eq!(view.desktop_job_list.scoped_count, 0);
        assert_eq!(view.desktop_job_list.hidden_without_fetch_time, 0);
    }

    #[test]
    fn desktop_list_last_24_hours_searches_within_mode() {
        let now = utc("2026-05-02T12:00:00Z");
        let mut state = AppState::new();
        insert_done_job(&mut state, 1, "https://example.com/selected");
        insert_done_job(&mut state, 2, "https://example.com/match");
        set_fetched_utc(&mut state, &[(1, Some(now)), (2, Some(now))]);
        set_summary_titles(
            &mut state,
            &[
                ("https://example.com/selected", "Other article"),
                ("https://example.com/match", "Needle article"),
            ],
        );
        state.select_job(1);
        state.set_jobs_search_query("needle".to_string());
        state.job_list_mode = JobListMode::Last24Hours;
        state = update(state, Msg::tick_at(now)).0;

        let view = state.view();
        assert_eq!(view.desktop_job_list.scoped_count, 1);
        assert_eq!(view.desktop_job_list.rows[0].job_id, 2);
        assert_eq!(
            view.desktop_job_list
                .selected_job
                .as_ref()
                .expect("selected job")
                .list_visibility,
            SelectedJobVisibility::QueryMismatch
        );
    }

    #[test]
    fn desktop_list_last_24_hours_cap_keeps_newest_rows() {
        let now = utc("2026-05-02T12:00:00Z");
        let mut state = AppState::new();
        for job_id in 1..=DESKTOP_JOB_LIST_MAX_ROWS + 1 {
            insert_done_job(
                &mut state,
                job_id as JobId,
                &format!("https://example.com/{job_id}"),
            );
            set_fetched_utc(
                &mut state,
                &[(
                    job_id as JobId,
                    Some(
                        now - chrono::Duration::seconds(
                            (DESKTOP_JOB_LIST_MAX_ROWS + 1 - job_id) as i64,
                        ),
                    ),
                )],
            );
        }
        state.job_list_mode = JobListMode::Last24Hours;
        state = update(state, Msg::tick_at(now)).0;

        let view = state.view();
        let ids = view
            .desktop_job_list
            .rows
            .iter()
            .map(|row| row.job_id)
            .collect::<Vec<_>>();
        assert_eq!(
            view.desktop_job_list.scoped_count,
            DESKTOP_JOB_LIST_MAX_ROWS + 1
        );
        assert!(view.desktop_job_list.truncated);
        assert_eq!(
            ids,
            (2..=DESKTOP_JOB_LIST_MAX_ROWS + 1)
                .map(|id| id as JobId)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn desktop_list_last_24_hours_searches_before_cap() {
        let now = utc("2026-05-02T12:00:00Z");
        let mut state = AppState::new();
        for job_id in 1..=DESKTOP_JOB_LIST_MAX_ROWS + 1 {
            let url = if job_id == 1 {
                "https://example.com/needle".to_string()
            } else {
                format!("https://example.com/{job_id}")
            };
            insert_done_job(&mut state, job_id as JobId, &url);
            set_fetched_utc(
                &mut state,
                &[(
                    job_id as JobId,
                    Some(
                        now - chrono::Duration::seconds(
                            (DESKTOP_JOB_LIST_MAX_ROWS + 1 - job_id) as i64,
                        ),
                    ),
                )],
            );
        }
        state.job_list_mode = JobListMode::Last24Hours;
        state = update(state, Msg::tick_at(now)).0;
        assert!(!state
            .view()
            .desktop_job_list
            .rows
            .iter()
            .any(|row| row.job_id == 1));

        state.set_jobs_search_query("needle".to_string());
        let view = state.view();
        assert_eq!(view.desktop_job_list.scoped_count, 1);
        assert!(!view.desktop_job_list.truncated);
        assert_eq!(
            view.desktop_job_list
                .rows
                .iter()
                .map(|row| row.job_id)
                .collect::<Vec<_>>(),
            vec![1]
        );
    }

    #[test]
    fn desktop_list_search_survives_last_24_hours_mode_switch() {
        let now = utc("2026-05-02T12:00:00Z");
        let mut state = AppState::new();
        insert_done_job(&mut state, 1, "https://example.com/needle");
        insert_done_job(&mut state, 2, "https://example.com/other");
        set_fetched_utc(
            &mut state,
            &[(1, Some(now - chrono::Duration::hours(2))), (2, Some(now))],
        );
        state.briefing_since_utc = Some(now - chrono::Duration::hours(1));
        state.set_jobs_search_query("needle".to_string());
        state = update(state, Msg::tick_at(now)).0;

        let since_view = state.view();
        assert!(since_view.desktop_job_list.rows.is_empty());
        state = update(
            state,
            Msg::JobListModeSet {
                mode: JobListMode::Last24Hours,
            },
        )
        .0;
        let recent_view = state.view();
        assert_eq!(recent_view.desktop_job_list.query, "needle");
        assert_eq!(recent_view.desktop_job_list.rows[0].job_id, 1);

        state = update(
            state,
            Msg::JobListModeSet {
                mode: JobListMode::SinceCheckpoint,
            },
        )
        .0;
        let restored_view = state.view();
        assert_eq!(restored_view.desktop_job_list.query, "needle");
        assert!(restored_view.desktop_job_list.rows.is_empty());
    }

    #[test]
    fn desktop_list_last_24_hours_slides_on_tick() {
        let first_now = utc("2026-05-02T12:00:00Z");
        let mut state = AppState::new();
        insert_done_job(&mut state, 1, "https://example.com/aging");
        set_fetched_utc(
            &mut state,
            &[(1, Some(first_now - chrono::Duration::hours(23)))],
        );
        state.job_list_mode = JobListMode::Last24Hours;
        state = update(state, Msg::tick_at(first_now)).0;
        assert_eq!(state.view().desktop_job_list.rows.len(), 1);

        state = update(state, Msg::tick_at(first_now + chrono::Duration::hours(2))).0;
        assert!(state.view().desktop_job_list.rows.is_empty());
    }

    #[test]
    fn selected_job_outside_last_24_hours_reports_outside_scope() {
        let now = utc("2026-05-02T12:00:00Z");
        let mut state = AppState::new();
        insert_done_job(&mut state, 1, "https://example.com/old");
        set_fetched_utc(&mut state, &[(1, Some(now - chrono::Duration::hours(25)))]);
        state.job_list_mode = JobListMode::Last24Hours;
        state.select_job(1);
        state = update(state, Msg::tick_at(now)).0;

        assert_eq!(
            state
                .view()
                .desktop_job_list
                .selected_job
                .as_ref()
                .expect("selected job")
                .list_visibility,
            SelectedJobVisibility::OutsideScope
        );
    }

    #[test]
    fn desktop_list_is_empty_in_results_mode_but_keeps_the_selected_job() {
        let mut state = AppState::new();
        insert_done_job(&mut state, 1, "https://example.com/selected");
        state.job_list_mode = JobListMode::Results;
        state.select_job(1);

        let view = state.view();

        assert!(view.desktop_job_list.rows.is_empty());
        assert_eq!(view.desktop_job_list.scoped_count, 0);
        assert_eq!(view.desktop_job_list.visible_count, 0);
        assert!(!view.desktop_job_list.truncated);
        assert_eq!(view.selected_job_id, Some(1));
        assert_eq!(
            view.desktop_job_list
                .selected_job
                .as_ref()
                .map(|job| job.list_visibility),
            Some(SelectedJobVisibility::OutsideScope)
        );
    }

    #[test]
    fn desktop_list_search_narrows_the_scope_case_insensitively() {
        let mut state = AppState::new();
        insert_done_job(&mut state, 1, "https://example.com/plain");
        insert_done_job(&mut state, 2, "https://github.com/example/project");
        insert_done_job(&mut state, 3, "https://example.com/other");
        set_summary_titles(
            &mut state,
            &[
                ("https://example.com/plain", "A quiet article"),
                ("https://github.com/example/project", "Release notes"),
                ("https://example.com/other", "Another article"),
            ],
        );

        state.set_jobs_search_query("RELEASE".to_string());
        let title_view = state.view();
        assert_eq!(
            title_view
                .desktop_job_list
                .rows
                .iter()
                .map(|row| row.job_id)
                .collect::<Vec<_>>(),
            vec![2]
        );

        state.set_jobs_search_query("GITHUB".to_string());
        let url_view = state.view();
        assert_eq!(
            url_view
                .desktop_job_list
                .rows
                .iter()
                .map(|row| row.job_id)
                .collect::<Vec<_>>(),
            vec![2]
        );
    }

    #[test]
    fn desktop_list_cap_applies_after_search() {
        let mut state = AppState::new();
        for job_id in 1..=DESKTOP_JOB_LIST_MAX_ROWS + 1 {
            let url = format!("https://example.com/{job_id}");
            insert_done_job(&mut state, job_id as JobId, &url);
            set_fetched_utc(
                &mut state,
                &[(
                    job_id as JobId,
                    Some(utc("2025-01-01T00:00:00Z") + chrono::Duration::days(job_id as i64)),
                )],
            );
        }
        set_summary_titles(&mut state, &[("https://example.com/1", "Reachable Needle")]);

        let capped_view = state.view();
        assert_eq!(
            capped_view.desktop_job_list.scoped_count,
            DESKTOP_JOB_LIST_MAX_ROWS + 1
        );
        assert!(!capped_view
            .desktop_job_list
            .rows
            .iter()
            .any(|row| row.job_id == 1));

        state.set_jobs_search_query("reachable needle".to_string());
        let searched_view = state.view();
        assert_eq!(
            searched_view
                .desktop_job_list
                .rows
                .iter()
                .map(|row| row.job_id)
                .collect::<Vec<_>>(),
            vec![1]
        );
    }

    #[test]
    fn desktop_list_cap_keeps_the_newest_by_fetch_time_and_emits_ascending_job_id() {
        let mut state = AppState::new();
        insert_done_job(&mut state, 1, "https://example.com/none");
        insert_done_job(&mut state, 2, "https://example.com/old");
        let newest_job_id = DESKTOP_JOB_LIST_MAX_ROWS as JobId + 4;
        for job_id in 3..=newest_job_id {
            insert_done_job(&mut state, job_id, &format!("https://example.com/{job_id}"));
        }
        set_fetched_utc(
            &mut state,
            &[(1, None), (2, Some(utc("2026-05-01T00:00:00Z")))],
        );
        for job_id in 3..=newest_job_id {
            set_fetched_utc(&mut state, &[(job_id, Some(utc("2026-05-02T00:00:00Z")))]);
        }

        let view = state.view();
        let ids = view
            .desktop_job_list
            .rows
            .iter()
            .map(|row| row.job_id)
            .collect::<Vec<_>>();

        assert_eq!(ids.len(), DESKTOP_JOB_LIST_MAX_ROWS);
        assert_eq!(ids, (5..=newest_job_id).collect::<Vec<_>>());
        assert!(!ids.contains(&1));
        assert!(!ids.contains(&2));
    }

    #[test]
    fn desktop_list_reports_truncation_counts() {
        let mut state = AppState::new();
        for job_id in 1..=DESKTOP_JOB_LIST_MAX_ROWS + 1 {
            insert_done_job(
                &mut state,
                job_id as JobId,
                &format!("https://example.com/{job_id}"),
            );
        }

        let view = state.view();

        assert_eq!(
            view.desktop_job_list.scoped_count,
            DESKTOP_JOB_LIST_MAX_ROWS + 1
        );
        assert_eq!(
            view.desktop_job_list.visible_count,
            DESKTOP_JOB_LIST_MAX_ROWS
        );
        assert!(view.desktop_job_list.truncated);
    }

    #[test]
    fn desktop_list_counts_jobs_hidden_for_missing_fetch_time() {
        let mut state = AppState::new();
        state.briefing_since_utc = Some(utc("2026-05-01T00:00:00Z"));
        insert_done_job(&mut state, 1, "https://example.com/new");
        insert_done_job(&mut state, 2, "https://example.com/missing");
        set_fetched_utc(
            &mut state,
            &[(1, Some(utc("2026-05-02T00:00:00Z"))), (2, None)],
        );

        let checkpoint_view = state.view();
        assert_eq!(
            checkpoint_view.desktop_job_list.hidden_without_fetch_time,
            1
        );
        assert_eq!(checkpoint_view.desktop_job_list.rows.len(), 1);

        state.briefing_since_utc = None;
        let no_checkpoint_view = state.view();
        assert_eq!(
            no_checkpoint_view
                .desktop_job_list
                .hidden_without_fetch_time,
            0
        );
        assert_eq!(no_checkpoint_view.desktop_job_list.rows.len(), 2);
    }

    #[test]
    fn desktop_list_rows_carry_no_links_and_the_selected_job_does() {
        let (mut state, _) = update(
            AppState::new(),
            Msg::RestoreCompletedJobs(vec![CompletedJobSnapshot {
                url: "https://example.com/selected".to_string(),
                tokens: Some(10),
                bytes: Some(100),
                links: vec![LinkSnapshotRecord {
                    url: "https://example.com/link".to_string(),
                    downloaded_path: None,
                }],
                fetched_utc: Some("2026-05-02T00:00:00Z".to_string()),
            }]),
        );
        state.select_job(1);

        let view = state.view();

        assert_eq!(view.desktop_job_list.rows[0].link_count, 1);
        assert_eq!(
            view.desktop_job_list
                .selected_job
                .as_ref()
                .expect("selected job")
                .links
                .len(),
            1
        );
    }

    #[test]
    fn selected_job_visibility_reports_exactly_one_reason() {
        let mut outside = AppState::new();
        outside.briefing_since_utc = Some(utc("2026-05-01T00:00:00Z"));
        insert_done_job(&mut outside, 1, "https://example.com/old");
        set_fetched_utc(&mut outside, &[(1, Some(utc("2026-04-30T00:00:00Z")))]);
        outside.select_job(1);
        let outside_view = outside.view();
        assert_eq!(outside_view.selected_job_id, Some(1));
        assert_eq!(
            outside_view
                .desktop_job_list
                .selected_job
                .as_ref()
                .expect("selected job remains present")
                .list_visibility,
            SelectedJobVisibility::OutsideScope
        );

        let mut mismatch = AppState::new();
        insert_done_job(&mut mismatch, 1, "https://example.com/selected");
        insert_done_job(&mut mismatch, 2, "https://example.com/other");
        mismatch.select_job(1);
        mismatch.set_jobs_search_query("other".to_string());
        let mismatch_view = mismatch.view();
        assert_eq!(mismatch_view.selected_job_id, Some(1));
        assert_eq!(
            mismatch_view
                .desktop_job_list
                .selected_job
                .as_ref()
                .expect("selected job remains present")
                .list_visibility,
            SelectedJobVisibility::QueryMismatch
        );

        let mut capped = AppState::new();
        for job_id in 1..=DESKTOP_JOB_LIST_MAX_ROWS + 1 {
            insert_done_job(
                &mut capped,
                job_id as JobId,
                &format!("https://example.com/{job_id}"),
            );
            set_fetched_utc(
                &mut capped,
                &[(
                    job_id as JobId,
                    Some(utc("2025-01-01T00:00:00Z") + chrono::Duration::days(job_id as i64)),
                )],
            );
        }
        capped.select_job(1);
        let capped_view = capped.view();
        assert_eq!(capped_view.selected_job_id, Some(1));
        assert_eq!(
            capped_view
                .desktop_job_list
                .selected_job
                .as_ref()
                .expect("selected job remains present")
                .list_visibility,
            SelectedJobVisibility::Capped
        );

        let mut visible = AppState::new();
        insert_done_job(&mut visible, 1, "https://example.com/visible");
        visible.select_job(1);
        let visible_view = visible.view();
        assert_eq!(visible_view.selected_job_id, Some(1));
        assert_eq!(
            visible_view
                .desktop_job_list
                .selected_job
                .as_ref()
                .expect("selected job remains present")
                .list_visibility,
            SelectedJobVisibility::Visible
        );
    }

    #[test]
    fn selected_job_visible_from_results_candidates() {
        use harvester_engine::llm::dto::{Confidence, SignalCandidateResult, SourceTier};

        let candidate_url = "https://example.com/candidate";
        let mut state = AppState::new();
        insert_done_job(&mut state, 1, candidate_url);
        insert_done_job(&mut state, 2, "https://example.com/not-a-candidate");
        state
            .signal_candidate_mut()
            .enqueue(candidate_url.to_string());
        state.signal_candidate_mut().mark_scoring(candidate_url, 1);
        state.signal_candidate_mut().complete(
            candidate_url,
            SignalCandidateResult {
                signal_score: 90,
                signal_key: "candidate".to_string(),
                themes: vec![],
                draft_gist: "gist".to_string(),
                source_tier: SourceTier::Tier1,
                confidence: Confidence::High,
                reasoning: "reason".to_string(),
                input_tokens: 1,
                output_tokens: 1,
            },
        );
        state.job_list_mode = JobListMode::Results;
        state.select_job(1);
        assert_eq!(
            state
                .view()
                .desktop_job_list
                .selected_job
                .unwrap()
                .list_visibility,
            SelectedJobVisibility::Visible
        );

        state.select_job(2);
        assert_eq!(
            state
                .view()
                .desktop_job_list
                .selected_job
                .unwrap()
                .list_visibility,
            SelectedJobVisibility::OutsideScope
        );
    }

    #[test]
    fn desktop_view_carries_rich_state_for_the_filtered_job_list() {
        use crate::briefing::LoadedArticle;
        use crate::triage::{ArticleTriageResult, TriageSession};

        let urls = [
            "https://example.com/rich-new",
            "https://example.com/rich-second",
            "https://example.com/rich-old",
        ];
        let mut state = AppState::new();
        for (index, url) in urls.iter().enumerate() {
            insert_done_job(&mut state, index as JobId + 1, url);
        }
        set_fetched_utc(
            &mut state,
            &[
                (1, Some(utc("2026-05-02T00:00:00Z"))),
                (2, Some(utc("2026-05-03T00:00:00Z"))),
                (3, Some(utc("2026-04-30T00:00:00Z"))),
            ],
        );
        state.briefing_since_utc = Some(utc("2026-05-01T00:00:00Z"));
        set_summary_titles(
            &mut state,
            &[
                (urls[0], "Needle-rich summary"),
                (urls[1], "Second summary"),
            ],
        );

        let loaded = urls
            .iter()
            .map(|url| LoadedArticle {
                url: (*url).to_string(),
                source_title: Some(format!("Source for {url}")),
                prepared_text: std::iter::repeat_n("substantial", 220)
                    .collect::<Vec<_>>()
                    .join(" "),
                content_hash: format!("rich-hash-{url}"),
                fetched_utc: None,
            })
            .collect::<Vec<_>>();
        let mut triage = TriageSession::new_loading(None);
        triage.set_articles(loaded.clone());
        triage.transition_to_triaging();
        for index in 0..loaded.len() {
            triage.complete_article(
                index,
                ArticleTriageResult {
                    category: format!("Category {index}"),
                    priority: index as u8 + 1,
                    tags: vec![format!("tag-{index}")],
                    rationale: "Rich-state annotation".into(),
                    input_tokens: 10,
                    output_tokens: 5,
                },
            );
        }
        triage.complete();
        state.set_triage(triage);

        let mut pre_triage = PreTriageSession::load_articles(loaded, &PreTriagePolicy::default());
        let decision_key = pre_triage
            .entry_for_url(urls[1])
            .expect("pre-triage entry")
            .key
            .clone();
        pre_triage
            .set_manual_decision(&decision_key, ManualDecision::Exclude)
            .expect("manual decision applies");
        state.set_pre_triage(pre_triage);
        state.select_job(1);
        let (state, _) = update(state, Msg::JobsSearchQueryChanged("needle".into()));

        let desktop_view = state.view();
        assert_eq!(desktop_view.desktop_job_list.query, "needle");
        assert_eq!(desktop_view.desktop_job_list.rows.len(), 1);
        assert!(desktop_view.desktop_job_list.rows[0]
            .summary_title
            .is_some());
        assert!(desktop_view.desktop_job_list.rows[0]
            .triage_annotation
            .is_some());
        assert!(desktop_view.desktop_job_list.rows[0]
            .filter_status
            .is_some());
        assert!(desktop_view.desktop_job_list.selected_job.is_some());
        assert_eq!(desktop_view.desktop_job_list.scoped_count, 1);
    }

    #[test]
    fn desktop_view_caps_recent_rows_before_priority_ordering_from_a_large_corpus() {
        let mut state = AppState::new();
        let corpus_size = DESKTOP_JOB_LIST_MAX_ROWS * 2 + 7;
        for job_id in 1..=corpus_size {
            insert_done_job(
                &mut state,
                job_id as JobId,
                &format!("https://example.com/{job_id}"),
            );
            state.jobs.get_mut(&(job_id as JobId)).unwrap().fetched_utc =
                chrono::DateTime::from_timestamp(job_id as i64, 0);
        }
        set_triage_annotations(&mut state, &[(1, 5)]);

        let desktop = state.view();

        assert_eq!(desktop.job_count, corpus_size);
        assert_eq!(desktop.desktop_job_list.scoped_count, corpus_size);
        assert_eq!(
            desktop.desktop_job_list.rows.len(),
            DESKTOP_JOB_LIST_MAX_ROWS
        );
        assert!(!desktop
            .desktop_job_list
            .rows
            .iter()
            .any(|row| row.job_id == 1));
        assert_eq!(
            desktop
                .desktop_job_list
                .rows
                .first()
                .map(|row| row.url.as_str()),
            Some("https://example.com/408")
        );
        assert_eq!(
            desktop
                .desktop_job_list
                .rows
                .last()
                .map(|row| row.url.as_str()),
            Some("https://example.com/807")
        );
    }

    #[test]
    fn desktop_rows_match_scope_when_query_empty() {
        let mut state = AppState::new();
        state.job_list_mode = JobListMode::SinceCheckpoint;
        state.briefing_since_utc = Some(utc("2026-05-01T00:00:00Z"));
        insert_done_job(&mut state, 1, "https://example.com/new");
        insert_done_job(&mut state, 2, "https://example.com/old");
        insert_done_job(&mut state, 3, "https://example.com/undated");
        state.jobs.get_mut(&1).unwrap().fetched_utc = Some(utc("2026-05-02T00:00:00Z"));
        state.jobs.get_mut(&2).unwrap().fetched_utc = Some(utc("2026-04-30T00:00:00Z"));

        let view = state.view();

        assert_eq!(
            view.desktop_job_list
                .rows
                .iter()
                .map(|row| row.job_id)
                .collect::<Vec<_>>(),
            vec![1]
        );
    }

    #[test]
    fn restored_jobs_after_checkpoint_remain_visible_in_desktop_list() {
        let (mut state, _) = update(
            AppState::new(),
            Msg::RestoreCompletedJobs(vec![
                CompletedJobSnapshot {
                    url: "https://example.com/new".to_string(),
                    tokens: Some(10),
                    bytes: Some(100),
                    links: Vec::new(),
                    fetched_utc: Some("2026-05-25T03:54:45.911305400+00:00".to_string()),
                },
                CompletedJobSnapshot {
                    url: "https://example.com/old".to_string(),
                    tokens: Some(20),
                    bytes: Some(200),
                    links: Vec::new(),
                    fetched_utc: Some("2026-05-23T03:54:45.911305400+00:00".to_string()),
                },
            ]),
        );
        state.job_list_mode = JobListMode::SinceCheckpoint;
        state.briefing_since_utc = Some(utc("2026-05-24T13:07:21.788858700+00:00"));

        let view = state.view();

        assert_eq!(view.job_count, 2);
        assert_eq!(
            view.desktop_job_list
                .rows
                .iter()
                .map(|row| row.job_id)
                .collect::<Vec<_>>(),
            vec![1]
        );
        assert_eq!(view.desktop_job_list.scoped_count, 1);
    }

    #[test]
    fn desktop_rows_substring_case_insensitive() {
        let mut state = AppState::new();
        insert_done_job(&mut state, 1, "https://example.com/a");
        insert_done_job(&mut state, 2, "https://example.com/b");
        insert_done_job(&mut state, 3, "https://example.com/c");
        set_summary_titles(
            &mut state,
            &[
                ("https://example.com/a", "Kubernetes Pods"),
                ("https://example.com/b", "Rust Async"),
                ("https://example.com/c", "K-quotes"),
            ],
        );
        state.set_jobs_search_query("KUBE".to_string());

        let view = state.view();

        assert_eq!(
            view.desktop_job_list
                .rows
                .iter()
                .map(|row| row.job_id)
                .collect::<Vec<_>>(),
            vec![1]
        );
    }

    #[test]
    fn desktop_rows_match_url_when_title_lacks_term() {
        let mut state = AppState::new();
        insert_done_job(&mut state, 1, "https://github.com/example/project");
        insert_done_job(&mut state, 2, "https://example.com/plain");
        set_summary_titles(
            &mut state,
            &[
                ("https://github.com/example/project", "Release notes"),
                ("https://example.com/plain", "Git workflow"),
            ],
        );
        state.set_jobs_search_query("github".to_string());

        let view = state.view();

        assert_eq!(
            view.desktop_job_list
                .rows
                .iter()
                .map(|row| row.job_id)
                .collect::<Vec<_>>(),
            vec![1]
        );
    }

    #[test]
    fn selection_survives_a_search_query_that_excludes_its_row() {
        let mut state = AppState::new();
        insert_done_job(&mut state, 1, "https://example.com/kube");
        insert_done_job(&mut state, 2, "https://example.com/rust");
        state.select_job(1);
        state.set_jobs_search_query("kube".to_string());

        let view = state.view();
        assert_eq!(view.selected_job_id, Some(1));
        assert!(view.desktop_job_list.selected_job.is_some());

        state.set_jobs_search_query("rust".to_string());
        let view = state.view();
        assert_eq!(view.selected_job_id, Some(1));
        assert_eq!(view.desktop_job_list.rows[0].job_id, 2);
        assert!(matches!(
            view.desktop_job_list.selected_job,
            Some(ref selected)
                if selected.list_visibility == SelectedJobVisibility::QueryMismatch
        ));
    }

    #[test]
    fn desktop_job_list_changes_with_search_query() {
        let mut state = AppState::new();
        insert_done_job(&mut state, 1, "https://example.com/kube");
        insert_done_job(&mut state, 2, "https://example.com/rust");

        let unfiltered_job_count = state.view().desktop_job_list.rows.len();
        state.set_jobs_search_query("kube".to_string());
        let filtered_view = state.view();

        assert_eq!(unfiltered_job_count, 2);
        assert_eq!(
            filtered_view
                .desktop_job_list
                .rows
                .iter()
                .map(|row| row.job_id)
                .collect::<Vec<_>>(),
            vec![1]
        );
    }

    #[test]
    fn desktop_scope_count_reflects_search_query() {
        let mut state = AppState::new();
        insert_done_job(&mut state, 1, "https://example.com/kube");
        insert_done_job(&mut state, 2, "https://example.com/rust");

        let view = state.view();
        assert_eq!(view.desktop_job_list.scoped_count, 2);

        state.set_jobs_search_query("kube".to_string());
        let view = state.view();
        assert_eq!(view.desktop_job_list.scoped_count, 1);
        assert_eq!(view.desktop_job_list.rows.len(), 1);
    }

    #[test]
    fn preview_metadata_for_selected_done_job_exposes_source_and_status() {
        let mut state = AppState::new();
        state.jobs.insert(
            1,
            JobState {
                url: "https://epochai.substack.com/p/what".to_string(),
                stage: Stage::Done,
                outcome: Some(JobResultKind::Success),
                ..Default::default()
            },
        );
        state.select_job(1);

        let view = state.view();

        let preview_context = view
            .preview_context
            .as_ref()
            .expect("expected preview metadata for selected job");
        assert_eq!(preview_context.source_label, "epochai.substack.com");
        assert_eq!(preview_context.status_label, "Done");
        assert_eq!(preview_context.attention_label, None);
    }
}

#[cfg(test)]
mod briefing_history_state_tests {
    use super::*;
    use crate::briefing::BriefingHistoryEntry;

    fn entry(ts: &str) -> BriefingHistoryEntry {
        BriefingHistoryEntry {
            generated_at_utc: ts.to_string(),
            executive_summary: format!("Summary {ts}"),
            top_stories: vec![],
            article_count: 1,
        }
    }

    #[test]
    fn starts_empty() {
        let state = AppState::new();
        assert!(state.briefing_history().is_empty());
    }

    #[test]
    fn push_adds_newest_first() {
        let mut state = AppState::new();
        state.push_briefing_history(entry("2026-02-20T00:00:00Z"));
        state.push_briefing_history(entry("2026-02-21T00:00:00Z"));
        assert_eq!(
            state.briefing_history()[0].generated_at_utc,
            "2026-02-21T00:00:00Z"
        );
        assert_eq!(
            state.briefing_history()[1].generated_at_utc,
            "2026-02-20T00:00:00Z"
        );
    }

    #[test]
    fn push_caps_at_three() {
        let mut state = AppState::new();
        for i in 1..=4 {
            state.push_briefing_history(entry(&format!("2026-02-2{}T00:00:00Z", i)));
        }
        assert_eq!(state.briefing_history().len(), 3);
        assert_eq!(
            state.briefing_history()[0].generated_at_utc,
            "2026-02-24T00:00:00Z"
        );
    }
}

#[cfg(test)]
mod poll_stats_view_tests {
    use super::*;

    #[test]
    fn ingest_skips_blacklisted_domain() {
        use harvester_engine::FetchOutcomeClass;
        let mut state = AppState::new();
        let now = chrono::DateTime::from_timestamp(0, 0).unwrap();
        for _ in 0..3 {
            state.blacklist.record_outcome(
                "bloomberg.com",
                FetchOutcomeClass::PermanentBlock,
                Some("http status 403"),
                now,
            );
        }
        let result = state.ingest_urls(
            vec![
                "https://www.bloomberg.com/news/a".to_string(),
                "https://example.com/ok".to_string(),
            ],
            now,
        );
        assert_eq!(result.enqueued, 1);
        assert!(result.skipped >= 1);
    }

    #[test]
    fn ingest_blacklisted_url_not_marked_seen_and_can_be_enqueued_after_cooldown() {
        use harvester_engine::FetchOutcomeClass;
        let mut state = AppState::new();
        let t0 = chrono::DateTime::from_timestamp(0, 0).unwrap();
        let t_after_cooldown = chrono::DateTime::from_timestamp(8 * 86_400, 0).unwrap();
        for _ in 0..3 {
            state.blacklist.record_outcome(
                "bloomberg.com",
                FetchOutcomeClass::PermanentBlock,
                Some("http status 403"),
                t0,
            );
        }

        let blocked = state.ingest_urls(vec!["https://www.bloomberg.com/news/a".to_string()], t0);
        assert_eq!(blocked.enqueued, 0);
        assert_eq!(blocked.skipped, 1);

        let after_cooldown = state.ingest_urls(
            vec!["https://www.bloomberg.com/news/a".to_string()],
            t_after_cooldown,
        );
        assert_eq!(
            after_cooldown.enqueued, 1,
            "URL must be enqueueable after cooldown expires"
        );
        assert_eq!(after_cooldown.skipped, 0);
    }

    #[test]
    fn ingest_indirect_links_skips_blacklisted_domain() {
        use harvester_engine::FetchOutcomeClass;
        let mut state = AppState::new();
        let now = chrono::DateTime::from_timestamp(0, 0).unwrap();
        for _ in 0..3 {
            state.blacklist.record_outcome(
                "bloomberg.com",
                FetchOutcomeClass::PermanentBlock,
                Some("http status 403"),
                now,
            );
        }
        let result = state.ingest_indirect_links(
            vec![
                IndirectLink {
                    url: "https://www.bloomberg.com/news/article".to_string(),
                    source_job_id: 1,
                },
                IndirectLink {
                    url: "https://example.com/allowed".to_string(),
                    source_job_id: 1,
                },
            ],
            now,
        );
        assert_eq!(result.enqueued, 1);
        assert!(result.skipped >= 1);
        let enqueued_id = result.enqueued_job_ids[0];
        let job = state.jobs.get(&enqueued_id).expect("enqueued job exists");
        assert_eq!(job.url, "https://example.com/allowed");
        assert!(matches!(job.origin, JobOrigin::Indirect { .. }));
    }
}

#[cfg(test)]
mod trends_view_tests {
    use super::*;

    #[test]
    fn trends_workspace_keeps_selected_article_context() {
        let mut state = AppState::new();
        state.jobs.insert(
            1,
            JobState {
                url: "https://epochai.substack.com/p/what".to_string(),
                stage: Stage::Done,
                outcome: Some(JobResultKind::Success),
                ..Default::default()
            },
        );
        state.select_job(1);
        state.set_workspace_view(crate::WorkspaceView::Trends);

        let view = state.view();

        assert!(
            view.preview_context
                .as_ref()
                .is_some_and(|context| context.source_label == "epochai.substack.com"),
            "selected article metadata may still exist in state"
        );
    }
}
