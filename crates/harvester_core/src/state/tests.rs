use super::*;

#[cfg(test)]
mod app_state_tests {
    use super::*;
    use crate::{
        update, BatchNextAction, BatchStatus, ManualDecision, Msg, PreTriageActionability,
        PreTriagePolicy, PreTriageSession,
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
    fn briefing_complete_then_job_selected_shows_summary_not_briefing() {
        use crate::briefing::{
            ArticleSummaryResult, BriefingResult, BriefingStoryResult, LoadedArticle,
        };

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
        briefing.set_briefing_request_id(2);
        briefing.complete_briefing(BriefingResult {
            executive_summary: "Executive summary".to_string(),
            top_stories: vec![BriefingStoryResult {
                headline: "Story 1".to_string(),
                body: "desc".to_string(),
            }],
            article_count: 1,
            input_tokens: 20,
            output_tokens: 10,
        });
        state.set_briefing(briefing);

        let view = state.view();
        assert!(view
            .preview_text
            .as_deref()
            .unwrap_or("")
            .contains("Executive Briefing"));

        state.select_job(1);
        let view = state.view();
        assert!(view
            .preview_text
            .as_deref()
            .unwrap_or("")
            .contains("Article Title"));
        assert!(!view
            .preview_text
            .as_deref()
            .unwrap_or("")
            .contains("Executive Briefing"));
    }

    #[test]
    fn job_selected_then_briefing_completes_shows_briefing() {
        use crate::briefing::{
            ArticleSummaryResult, BriefingResult, BriefingStoryResult, LoadedArticle,
        };

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
        briefing.set_briefing_request_id(2);
        briefing.complete_briefing(BriefingResult {
            executive_summary: "Executive summary".to_string(),
            top_stories: vec![BriefingStoryResult {
                headline: "Story 1".to_string(),
                body: "desc".to_string(),
            }],
            article_count: 1,
            input_tokens: 20,
            output_tokens: 10,
        });
        state.set_briefing(briefing);

        state.select_job(1);
        let view = state.view();
        assert!(view
            .preview_text
            .as_deref()
            .unwrap_or("")
            .contains("Article Title"));

        state.revert_preview_to_briefing();
        let view = state.view();
        assert!(view
            .preview_text
            .as_deref()
            .unwrap_or("")
            .contains("Executive Briefing"));
    }

    #[test]
    fn no_selection_shows_briefing_when_complete() {
        use crate::briefing::{BriefingResult, BriefingStoryResult};

        let mut state = AppState::new();
        let mut briefing = crate::briefing::BriefingSession::new_loading(None);
        briefing.set_articles(vec![], "collection".to_string());
        briefing.set_briefing_request_id(1);
        briefing.complete_briefing(BriefingResult {
            executive_summary: "Executive summary text".to_string(),
            top_stories: vec![BriefingStoryResult {
                headline: "Story".to_string(),
                body: "desc".to_string(),
            }],
            article_count: 0,
            input_tokens: 10,
            output_tokens: 5,
        });
        state.set_briefing(briefing);

        let view = state.view();
        assert!(view
            .preview_text
            .as_deref()
            .unwrap_or("")
            .contains("Executive Briefing"));
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

    #[test]
    fn selecting_job_with_summary_shows_formatted_summary() {
        let mut state = make_state_with_summarized_job();
        state.select_job(10);
        let view = state.view();
        let text = view.preview_text.unwrap_or_default();
        assert!(text.contains("My Title"));
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
        use crate::briefing::{BriefingResult, BriefingStoryResult};
        let mut s2 = make_state_with_summarized_job();
        s2.briefing_mut().set_briefing_request_id(99);
        s2.briefing_mut().complete_briefing(BriefingResult {
            executive_summary: "Exec summary".to_string(),
            top_stories: vec![BriefingStoryResult {
                headline: "T".to_string(),
                body: "d".to_string(),
            }],
            article_count: 1,
            input_tokens: 10,
            output_tokens: 5,
        });
        s2.select_job(10);
        let view = s2.view();
        let text = view.preview_text.unwrap_or_default();
        assert!(
            !text.contains("Exec summary"),
            "should not show briefing text when job selected"
        );
        assert!(text.contains("My Title"), "should show summary");
    }

    #[test]
    fn format_summary_includes_title_summary_and_key_points() {
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
        assert!(formatted.contains("Test Title"));
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
        assert!(formatted.contains("Title Only"));
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
            .jobs
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
            .jobs
            .iter()
            .find(|j| j.job_id == 13)
            .expect("job 13 exists");
        assert!(!job.has_summary);
        assert!(job.summary_title.is_none());
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
    fn operation_progress_from_poll() {
        let mut state = AppState::new();
        assert!(state.start_poll());
        state.set_poll_total(3);

        let view = state.view();
        assert_eq!(
            view.operation_progress,
            Some(OperationProgress {
                label: "Polling".to_string(),
                completed: 0,
                total: 3,
            })
        );
    }

    #[test]
    fn operation_progress_from_triage() {
        use crate::briefing::LoadedArticle;
        use crate::triage::ArticleTriageResult;

        let mut state = AppState::new();
        let mut triage = crate::triage::TriageSession::new_loading(None);
        triage.set_articles(vec![
            LoadedArticle {
                url: "https://example.com/1".to_string(),
                source_title: None,
                prepared_text: "text".to_string(),
                content_hash: "hash-1".to_string(),
                fetched_utc: None,
            },
            LoadedArticle {
                url: "https://example.com/2".to_string(),
                source_title: None,
                prepared_text: "text".to_string(),
                content_hash: "hash-2".to_string(),
                fetched_utc: None,
            },
        ]);
        triage.transition_to_triaging();
        triage.start_article(0, 1);
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
        state.set_triage(triage);

        let view = state.view();
        assert_eq!(
            view.operation_progress,
            Some(OperationProgress {
                label: "Triaging".to_string(),
                completed: 1,
                total: 2,
            })
        );
    }

    #[test]
    fn operation_progress_from_briefing() {
        use crate::briefing::{ArticleSummaryResult, LoadedArticle};

        let mut state = AppState::new();
        let mut briefing = crate::briefing::BriefingSession::new_loading(None);
        briefing.set_articles(
            vec![
                LoadedArticle {
                    url: "https://example.com/1".to_string(),
                    source_title: None,
                    prepared_text: "text".to_string(),
                    content_hash: "hash-1".to_string(),
                    fetched_utc: None,
                },
                LoadedArticle {
                    url: "https://example.com/2".to_string(),
                    source_title: None,
                    prepared_text: "text".to_string(),
                    content_hash: "hash-2".to_string(),
                    fetched_utc: None,
                },
            ],
            "collection".to_string(),
        );
        briefing.transition_to_summarizing();
        briefing.start_article(0, 1);
        briefing.complete_article(
            0,
            ArticleSummaryResult {
                title: "Title".to_string(),
                summary: "Summary".to_string(),
                key_points: Vec::new(),
                input_tokens: 1,
                output_tokens: 1,
                entities: Default::default(),
            },
        );
        state.set_briefing(briefing);

        let view = state.view();
        assert_eq!(
            view.operation_progress,
            Some(OperationProgress {
                label: "Summarizing".to_string(),
                completed: 1,
                total: 2,
            })
        );
    }

    fn startup_pre_triage_loading_state(saved_count: usize) -> AppState {
        let mut state = AppState::new();
        state.set_pre_triage_load_context(
            crate::pre_triage_coordinator::PreTriageRefreshReason::RestoreCompletedJobs,
            saved_count,
        );
        state.set_pre_triage(PreTriageSession::new_loading());
        state
    }

    #[test]
    fn operation_progress_from_pre_triage_loading() {
        let state = startup_pre_triage_loading_state(2);

        let view = state.view();
        assert_eq!(
            view.operation_progress,
            Some(OperationProgress {
                label: "Preparing triage set (2 saved)".to_string(),
                completed: 0,
                total: 1,
            })
        );
        assert_eq!(
            view.triage_progress,
            Some("Preparing triage set from 2 saved articles...".to_string())
        );
        assert_eq!(
            view.triage_blocked_reason,
            Some("Triage is unavailable while startup prepares the article set".to_string())
        );
    }

    #[test]
    fn operation_progress_from_pre_triage_scan_progress() {
        let mut state = startup_pre_triage_loading_state(2);
        state.set_triage_in_flight(7);
        state.set_pre_triage_load_progress(7, 42, 190);

        let view = state.view();
        assert_eq!(
            view.operation_progress,
            Some(OperationProgress {
                label: "Preparing triage set (2 saved)".to_string(),
                completed: 42,
                total: 190,
            })
        );
    }

    #[test]
    fn operation_progress_from_pre_triage_loading_falls_back_when_total_unknown() {
        let mut state = startup_pre_triage_loading_state(2);
        state.set_triage_in_flight(7);
        state.set_pre_triage_load_progress(7, 0, 0);

        let view = state.view();
        assert_eq!(
            view.operation_progress,
            Some(OperationProgress {
                label: "Preparing triage set (2 saved)".to_string(),
                completed: 0,
                total: 1,
            })
        );
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
        assert_eq!(
            view.right_pane.briefing_markdown,
            Some(
                "AI setup required\n\nBriefing is disabled because `OPENAI_API_KEY` is not set.\n\nSet `OPENAI_API_KEY` in the launch environment and restart the app to enable briefing generation.".to_string()
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
        assert_eq!(
            view.right_pane.briefing_markdown,
            Some("Briefing is unavailable because no triage model is available.".to_string())
        );
    }

    #[test]
    fn layout_view_shows_operation_progress_during_pre_triage_loading() {
        let state = startup_pre_triage_loading_state(1);

        let layout = state.layout_view();
        assert!(
            layout.operation_progress_visible,
            "pre-triage loading must show footer progress controls"
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
    fn operation_progress_poll_takes_precedence() {
        use crate::briefing::LoadedArticle;

        let mut state = AppState::new();
        assert!(state.start_poll());
        state.set_poll_total(4);

        let mut triage = crate::triage::TriageSession::new_loading(None);
        triage.set_articles(vec![LoadedArticle {
            url: "https://example.com/1".to_string(),
            source_title: None,
            prepared_text: "text".to_string(),
            content_hash: "hash-1".to_string(),
            fetched_utc: None,
        }]);
        triage.transition_to_triaging();
        state.set_triage(triage);

        let view = state.view();
        assert_eq!(
            view.operation_progress,
            Some(OperationProgress {
                label: "Polling".to_string(),
                completed: 0,
                total: 4,
            })
        );
    }

    #[test]
    fn operation_progress_poll_still_takes_precedence_over_pre_triage_loading() {
        let mut state = startup_pre_triage_loading_state(3);
        assert!(state.start_poll());
        state.set_poll_total(4);

        let view = state.view();
        assert_eq!(
            view.operation_progress,
            Some(OperationProgress {
                label: "Polling".to_string(),
                completed: 0,
                total: 4,
            })
        );
    }

    #[test]
    fn operation_progress_triage_still_takes_precedence_over_pre_triage_loading() {
        use crate::briefing::LoadedArticle;

        let mut state = startup_pre_triage_loading_state(2);
        let mut triage = crate::triage::TriageSession::new_loading(None);
        triage.set_articles(vec![LoadedArticle {
            url: "https://example.com/1".to_string(),
            source_title: None,
            prepared_text: "text".to_string(),
            content_hash: "hash-1".to_string(),
            fetched_utc: None,
        }]);
        triage.transition_to_triaging();
        state.set_triage(triage);

        let view = state.view();
        assert_eq!(
            view.operation_progress,
            Some(OperationProgress {
                label: "Triaging".to_string(),
                completed: 0,
                total: 1,
            })
        );
    }

    #[test]
    fn operation_progress_none_when_idle() {
        let state = AppState::new();
        let view = state.view();
        assert!(view.operation_progress.is_none());
    }

    #[test]
    fn operation_progress_none_after_pre_triage_ready() {
        let mut state = startup_pre_triage_loading_state(1);
        let pre_triage = PreTriageSession::load_articles(
            vec![article_with_words("https://example.com/ready", 220)],
            &PreTriagePolicy::default(),
        );
        state.set_pre_triage(pre_triage);

        let view = state.view();
        assert!(view.operation_progress.is_none());
        assert_eq!(
            view.triage_progress,
            Some(
                "Pre-triage: 1 include, 0 review, 0 filtered; Run Triage uses current selection"
                    .to_string()
            )
        );
    }

    #[test]
    fn operation_progress_none_during_triage_loading() {
        let mut state = AppState::new();
        let triage = crate::triage::TriageSession::new_loading(None);
        state.set_triage(triage);

        let view = state.view();
        assert!(view.operation_progress.is_none());
    }

    #[test]
    fn default_app_state_has_closed_empty_prompt_lab() {
        let state = AppState::new();
        let lab = state.prompt_lab();
        assert!(!lab.is_visible());
        assert_eq!(lab.run_count(), 0);
        assert!(!lab.has_in_flight_run());
    }

    #[test]
    fn allocate_prompt_lab_run_id_is_monotonic() {
        let mut state = AppState::new();
        let id1 = state.allocate_next_prompt_lab_run_id();
        let id2 = state.allocate_next_prompt_lab_run_id();
        let id3 = state.allocate_next_prompt_lab_run_id();
        assert!(id2.0 > id1.0, "run IDs must increase");
        assert!(id3.0 > id2.0, "run IDs must increase");
    }

    #[test]
    fn prompt_lab_and_llm_request_id_counters_are_independent() {
        let mut state = AppState::new();
        let llm1 = state.allocate_next_llm_request_id();
        let llm2 = state.allocate_next_llm_request_id();
        let lab1 = state.allocate_next_prompt_lab_run_id();
        let lab2 = state.allocate_next_prompt_lab_run_id();
        assert!(llm2 > llm1, "llm counter must increase");
        assert!(lab2.0 > lab1.0, "lab counter must increase");
        let llm3 = state.allocate_next_llm_request_id();
        let lab3 = state.allocate_next_prompt_lab_run_id();
        assert!(llm3 > llm2, "llm counter advances after lab allocations");
        assert!(
            lab3.0 > lab2.0,
            "lab counter advances after llm allocations"
        );
    }

    #[test]
    fn clear_prompt_lab_history_preserves_pending_entries() {
        use harvester_engine::llm::prompt::PromptId;
        let mut state = AppState::new();
        state.open_prompt_lab();

        let req_id = state.allocate_next_llm_request_id();
        let run_id = state.allocate_next_prompt_lab_run_id();
        state.add_prompt_lab_pending_run(PromptLabPendingRunRegistration {
            run_id,
            stage: crate::prompt_lab::PromptLabStage::Triage,
            prompt_id: PromptId::ArticleTriage,
            input_snapshot: "input".to_string(),
            request_id: req_id,
            overrides: PromptLabRunOverrides::default(),
            compare_batch_id: None,
            compare_candidate_id: None,
        });

        let req_id2 = state.allocate_next_llm_request_id();
        let run_id2 = state.allocate_next_prompt_lab_run_id();
        state.add_prompt_lab_pending_run(PromptLabPendingRunRegistration {
            run_id: run_id2,
            stage: crate::prompt_lab::PromptLabStage::Triage,
            prompt_id: PromptId::ArticleTriage,
            input_snapshot: "input2".to_string(),
            request_id: req_id2,
            overrides: PromptLabRunOverrides::default(),
            compare_batch_id: None,
            compare_candidate_id: None,
        });
        state.complete_prompt_lab_run(
            run_id2,
            "{}".to_string(),
            harvester_engine::llm::run_metadata::LlmRunMetadata::stub(),
        );
        state.consume_prompt_lab_ownership(req_id2);

        state.clear_prompt_lab_history();

        assert_eq!(state.prompt_lab().run_count(), 1);
        assert!(state.prompt_lab().ownership_for(req_id).is_some());
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
        assert!(content.contains("Test Summary"));
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
        let key = pre_triage
            .entry_for_url("https://job.example")
            .expect("article should produce pre-triage entry")
            .key
            .clone();
        state.set_pre_triage(pre_triage);

        assert_eq!(
            state.job_filter_status(12),
            Some(JobFilterStatus::AutoIncluded)
        );

        assert!(state.set_pre_triage_manual_decision(key, ManualDecision::Exclude));
        assert_eq!(
            state.job_filter_status(12),
            Some(JobFilterStatus::ManuallyExcluded)
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

    #[test]
    fn left_header_for_triage_results_since_checkpoint_has_stable_title_and_meta() {
        use crate::briefing::LoadedArticle;
        use crate::triage::ArticleTriageResult;

        let mut state = AppState::new();
        state.job_list_scope = JobListScope::SinceCheckpoint;
        state.left_tab = LeftTab::TriageResults;
        state.jobs.insert(
            1,
            JobState {
                url: "https://example.com/article".to_string(),
                stage: Stage::Done,
                outcome: Some(JobResultKind::Success),
                ..Default::default()
            },
        );
        let mut triage = crate::triage::TriageSession::new_loading(None);
        triage.set_articles(vec![LoadedArticle {
            url: "https://example.com/article".to_string(),
            source_title: None,
            prepared_text: "text".to_string(),
            content_hash: "hash".to_string(),
            fetched_utc: None,
        }]);
        triage.transition_to_triaging();
        triage.start_article(0, 1);
        triage.complete_article(
            0,
            ArticleTriageResult {
                category: "Keep".to_string(),
                priority: 1,
                tags: vec![],
                rationale: "ok".to_string(),
                input_tokens: 1,
                output_tokens: 1,
            },
        );
        state.set_triage(triage);

        let view = state.view();

        assert_eq!(view.left_pane_header.title, "Triage Results");
        assert_eq!(
            view.left_pane_header.scope_label.as_deref(),
            Some("Since checkpoint")
        );
        assert_eq!(
            view.left_pane_header.count_label.as_deref(),
            Some("1 with triage")
        );
        assert_eq!(view.left_pane_header.state_label.as_deref(), None);
    }

    #[test]
    fn left_header_shows_empty_state_in_meta_not_title() {
        let mut state = AppState::new();
        state.left_tab = LeftTab::TriageResults;

        let view = state.view();

        assert_eq!(view.left_pane_header.title, "Triage Results");
        assert_eq!(
            view.left_pane_header.count_label.as_deref(),
            Some("no triage results yet")
        );
        assert_eq!(view.left_pane_header.state_label.as_deref(), None);
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

    #[test]
    fn briefing_tab_still_uses_page_level_header_override() {
        use crate::briefing::{BriefingResult, BriefingSession};

        let mut state = AppState::new();
        let mut briefing = BriefingSession::new_loading(None);
        briefing.set_articles(vec![], "collection".to_string());
        briefing.complete_briefing(BriefingResult {
            executive_summary: "Summary".to_string(),
            top_stories: vec![],
            article_count: 0,
            input_tokens: 10,
            output_tokens: 5,
        });
        state.set_briefing(briefing);
        state.select_tab(AppTab::Briefing);

        let view = state.view();

        assert!(
            view.preview_header_text.is_some(),
            "briefing tab must provide a preview header"
        );
        assert!(
            view.preview_header_text
                .as_deref()
                .unwrap()
                .contains("Briefing"),
            "briefing tab header must identify the briefing source"
        );
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
    fn poll_stats_tab_still_uses_page_level_header_override() {
        let mut state = AppState::new();
        state.select_tab(AppTab::PollStats);
        let view = state.view();
        assert!(
            view.preview_header_text.is_some(),
            "poll stats tab must provide a preview header override"
        );
        assert!(
            view.preview_header_text
                .as_deref()
                .unwrap()
                .contains("Poll Stats"),
            "poll stats tab header must identify the poll-stats source"
        );
    }

    #[test]
    fn poll_stats_header_not_overridden_on_other_tabs() {
        let mut state = AppState::new();
        state.select_tab(AppTab::Triage);
        let view = state.view();
        assert_eq!(view.preview_header_text, None);
    }
}

#[cfg(test)]
mod trends_view_tests {
    use super::*;

    #[test]
    fn trends_tab_uses_page_level_header_override_instead_of_selected_article_context() {
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
        state.select_tab(AppTab::Trends);

        let view = state.view();
        let layout = state.layout_view();

        assert!(
            view.preview_header_text
                .as_deref()
                .unwrap_or("")
                .contains("Trends"),
            "trends tab must provide a page-level header override"
        );
        assert!(
            view.preview_context
                .as_ref()
                .is_some_and(|context| context.source_label == "epochai.substack.com"),
            "selected article metadata may still exist in state"
        );
        assert!(
            layout.preview_header_override_visible,
            "trends tab should render the page-level header row"
        );
        assert!(
            !layout.preview_context_visible,
            "trends tab should hide the selected article metadata row"
        );
    }
}
