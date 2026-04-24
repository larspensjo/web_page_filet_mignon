use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

use harvester_core::{Effect, JobResultKind, LlmResultKind, Msg};
use harvester_engine::llm::load_context_file;
use harvester_engine::llm::prompt::PromptId;
use harvester_engine::llm::types::ProviderKind;
use harvester_engine::llm::{LlmCompletionError, LlmEvent};
use harvester_engine::llm::{DEFAULT_BRIEFING_MODEL, OPENAI_MODEL_GPT_4O_MINI};
use harvester_engine::{FetchSettings, UrlPolicy};
use tempfile::tempdir;

use crate::effect_helpers::{build_local_model_catalog, download_link_page, map_llm_event};
use crate::RuntimePaths;

use super::{EffectRunner, NoOpPlatformHandler};

fn make_test_runtime_paths(base: &Path) -> RuntimePaths {
    RuntimePaths {
        output_dir: base.to_path_buf(),
        contexts_dir: base.join("contexts"),
        prompts_dir: base.join("prompts"),
        sources_path: base.join("sources.ron"),
        seen_set_path: base.join("seen_set.ron"),
        summary_cache_path: base.join("summary_cache.ron"),
        triage_cache_path: base.join("triage_cache.ron"),
        state_path: base.join("state.json"),
        briefing_history_path: base.join(".briefing_history.ron"),
        briefing_checkpoint_path: base.join(".briefing_checkpoint.ron"),
        entity_index_path: base.join(".entity_index.ron"),
        brave_seen_set_path: base.join(".brave_seen_set.ron"),
        brave_metadata_path: base.join(".brave_metadata.ron"),
    }
}

fn runner_with_receiver(base: &Path) -> (EffectRunner, mpsc::Receiver<Msg>) {
    let (tx, rx) = mpsc::channel();
    let paths = make_test_runtime_paths(base);
    let platform_handler = Box::new(NoOpPlatformHandler);
    (EffectRunner::new(paths, tx, platform_handler), rx)
}

fn write_prompt_context(dir: &Path, filename: &str, prompt_id: PromptId) {
    fs::create_dir_all(dir).expect("create contexts dir");
    fs::write(
        dir.join(filename),
        format!(
            "[meta]\nprompt_id = \"{}\"\nschema_version = 1\nversion = 1\nupdated = \"2026-04-24\"\n\n[variables]\npolicy = \"test policy\"\n",
            prompt_id
        ),
    )
    .expect("write context");
}

fn write_markdown(dir: &Path, filename: &str, url: &str) {
    use harvester_engine::{build_markdown_document, WhitespaceTokenCounter};
    let counter = WhitespaceTokenCounter;
    let (_, markdown) = build_markdown_document(
        url,
        Some("Title"),
        "utf-8",
        "2026-02-14T00:00:00Z",
        "body",
        &counter,
    );
    fs::write(dir.join(filename), markdown).expect("write markdown");
}

fn write_markdown_with_fetched_utc(dir: &Path, filename: &str, url: &str, fetched_utc: &str) {
    use harvester_engine::{build_markdown_document, WhitespaceTokenCounter};
    let counter = WhitespaceTokenCounter;
    let (_, markdown) =
        build_markdown_document(url, Some("Title"), "utf-8", fetched_utc, "body", &counter);
    fs::write(dir.join(filename), markdown).expect("write markdown");
}

#[test]
fn load_prompt_contexts_fails_when_required_triage_context_is_missing() {
    let temp = tempdir().expect("tempdir");
    let contexts_dir = temp.path().join("contexts");
    fs::create_dir_all(&contexts_dir).expect("create contexts dir");
    write_prompt_context(
        &contexts_dir,
        "article_summary.toml",
        PromptId::ArticleSummary,
    );

    let (runner, rx) = runner_with_receiver(temp.path());
    runner.enqueue(vec![Effect::LoadPromptContexts]);

    let msg = rx
        .recv_timeout(Duration::from_secs(1))
        .expect("expected contexts loaded msg");
    match msg {
        Msg::PromptContextsLoaded { contexts } => {
            assert!(contexts.contains_key(&PromptId::ArticleSummary));
            assert!(!contexts.contains_key(&PromptId::ArticleTriage));
        }
        other => panic!("unexpected message: {:?}", other),
    }

    let msg = rx
        .recv_timeout(Duration::from_secs(1))
        .expect("expected context load failed msg");

    match msg {
        Msg::PromptContextsLoadFailed { reason } => {
            assert!(reason.contains("required ArticleTriage context file missing"));
            assert!(reason.contains("article_triage.toml"));
        }
        other => panic!("unexpected message: {:?}", other),
    }
}

#[test]
fn load_prompt_contexts_fails_when_required_triage_context_is_invalid() {
    let temp = tempdir().expect("tempdir");
    let contexts_dir = temp.path().join("contexts");
    fs::create_dir_all(&contexts_dir).expect("create contexts dir");
    fs::write(contexts_dir.join("article_triage.toml"), "bad toml").expect("write invalid file");
    write_prompt_context(
        &contexts_dir,
        "article_summary.toml",
        PromptId::ArticleSummary,
    );

    let (runner, rx) = runner_with_receiver(temp.path());
    runner.enqueue(vec![Effect::LoadPromptContexts]);

    let msg = rx
        .recv_timeout(Duration::from_secs(1))
        .expect("expected contexts loaded msg");
    match msg {
        Msg::PromptContextsLoaded { contexts } => {
            assert!(contexts.contains_key(&PromptId::ArticleSummary));
            assert!(!contexts.contains_key(&PromptId::ArticleTriage));
        }
        other => panic!("unexpected message: {:?}", other),
    }

    let msg = rx
        .recv_timeout(Duration::from_secs(1))
        .expect("expected context load failed msg");

    match msg {
        Msg::PromptContextsLoadFailed { reason } => {
            assert!(reason.contains("required ArticleTriage context failed to load"));
            assert!(reason.contains("article_triage.toml"));
        }
        other => panic!("unexpected message: {:?}", other),
    }
}

#[test]
fn build_local_model_catalog_uses_effective_models_with_dedup_and_sort() {
    let mut effective_models = HashMap::new();
    effective_models.insert(
        PromptId::ArticleTriage,
        OPENAI_MODEL_GPT_4O_MINI.to_string(),
    );
    effective_models.insert(PromptId::ArticleSummary, "o3-mini".to_string());
    effective_models.insert(
        PromptId::AggregateBriefing,
        DEFAULT_BRIEFING_MODEL.to_string(),
    );

    let models = build_local_model_catalog(Some(ProviderKind::OpenAi), &effective_models);
    let names: Vec<_> = models.iter().map(|m| m.model_name().to_string()).collect();

    assert_eq!(
        names,
        vec![
            OPENAI_MODEL_GPT_4O_MINI.to_string(),
            DEFAULT_BRIEFING_MODEL.to_string(),
            "o3-mini".to_string()
        ]
    );
}

#[test]
fn build_local_model_catalog_returns_empty_without_provider_kind() {
    let mut effective_models = HashMap::new();
    effective_models.insert(
        PromptId::ArticleTriage,
        OPENAI_MODEL_GPT_4O_MINI.to_string(),
    );

    let models = build_local_model_catalog(None, &effective_models);

    assert!(models.is_empty());
}

#[test]
fn download_link_page_rejects_disallowed_scheme_before_request() {
    let temp = tempdir().expect("tempdir");
    let fetch_settings = FetchSettings::default();
    let policy = UrlPolicy::default();
    let err = download_link_page("file:///etc/passwd", temp.path(), &policy, &fetch_settings)
        .unwrap_err();

    assert!(
        err.contains("url policy violation"),
        "expected url policy error, got '{}'",
        err
    );
}

#[test]
fn enqueue_url_effect_is_rejected_by_url_policy() {
    let temp = tempdir().expect("tempdir");
    let (runner, rx) = runner_with_receiver(temp.path());
    let job_id = 42;
    runner.enqueue(vec![Effect::EnqueueUrl {
        job_id,
        url: "file:///etc/passwd".to_string(),
    }]);

    let msg = rx
        .recv_timeout(Duration::from_secs(1))
        .expect("expected job done msg");

    match msg {
        Msg::JobDone {
            job_id: received,
            result: JobResultKind::Failed { reason },
            ..
        } => {
            assert_eq!(received, job_id);
            assert!(reason.contains("url policy"));
        }
        other => panic!("unexpected message: {:?}", other),
    }
}

#[test]
fn download_link_page_effect_is_rejected_by_authorization() {
    let temp = tempdir().expect("tempdir");
    let (runner, rx) = runner_with_receiver(temp.path());
    let job_id = 7;
    let link_index = 1;
    runner.enqueue(vec![Effect::DownloadLinkedPage {
        job_id,
        link_index,
        url: "file:///tmp/secret".to_string(),
    }]);

    let msg = rx
        .recv_timeout(Duration::from_secs(1))
        .expect("expected link download failed msg");

    match msg {
        Msg::LinkDownloadFailed {
            job_id: received,
            link_index: received_index,
            error,
        } => {
            assert_eq!(received, job_id);
            assert_eq!(received_index, link_index);
            assert!(error.contains("url policy"));
        }
        other => panic!("unexpected message: {:?}", other),
    }
}

#[test]
fn delete_linked_page_effect_is_rejected_on_unsafe_path() {
    let temp = tempdir().expect("tempdir");
    let (runner, rx) = runner_with_receiver(temp.path());
    let job_id = 11;
    let link_index = 3;
    runner.enqueue(vec![Effect::DeleteLinkedPage {
        job_id,
        link_index,
        path: std::path::PathBuf::from("../outside.md"),
    }]);

    let msg = rx
        .recv_timeout(Duration::from_secs(1))
        .expect("expected link deleted msg");

    match msg {
        Msg::LinkDeleted {
            job_id: received,
            link_index: received_index,
        } => {
            assert_eq!(received, job_id);
            assert_eq!(received_index, link_index);
        }
        other => panic!("unexpected message: {:?}", other),
    }
}

#[test]
fn save_prompt_context_file_writes_file_and_dispatches_saved_msg() {
    let temp = tempdir().expect("tempdir");
    let (runner, rx) = runner_with_receiver(temp.path());
    let prompt_id = PromptId::ArticleTriage;
    runner.enqueue(vec![Effect::SavePromptContextFile {
        prompt_id,
        context_pairs: vec![("foo".into(), "bar".into())],
    }]);

    let msg = rx
        .recv_timeout(Duration::from_secs(1))
        .expect("expected context saved msg");

    match msg {
        Msg::PromptLabContextSaved {
            prompt_id: received,
            path,
            version,
        } => {
            assert_eq!(received, prompt_id);
            let saved = load_context_file(&std::path::PathBuf::from(&path)).expect("load saved");
            assert_eq!(saved.meta.prompt_id, prompt_id.to_string());
            assert_eq!(saved.meta.schema_version, 1);
            assert_eq!(version, saved.meta.version as u64);
            assert_eq!(saved.variables.get("foo").map(String::as_str), Some("bar"));
        }
        other => panic!("unexpected message: {:?}", other),
    }
}

#[test]
fn save_prompt_context_file_reports_failure_when_existing_file_invalid() {
    let temp = tempdir().expect("tempdir");
    let contexts_dir = temp.path().join("contexts");
    fs::create_dir_all(&contexts_dir).expect("create contexts dir");
    fs::write(contexts_dir.join("article_triage.toml"), "bad toml").expect("write invalid file");

    let (runner, rx) = runner_with_receiver(temp.path());
    runner.enqueue(vec![Effect::SavePromptContextFile {
        prompt_id: PromptId::ArticleTriage,
        context_pairs: vec![("foo".into(), "bar".into())],
    }]);

    let msg = rx
        .recv_timeout(Duration::from_secs(1))
        .expect("expected save failed msg");

    match msg {
        Msg::PromptLabContextSaveFailed { prompt_id, reason } => {
            assert_eq!(prompt_id, PromptId::ArticleTriage);
            assert!(reason.contains("failed to read existing context"));
        }
        other => panic!("unexpected message: {:?}", other),
    }
}

#[test]
fn save_briefing_checkpoint_dispatches_success_ack() {
    let temp = tempdir().expect("tempdir");
    let (runner, rx) = runner_with_receiver(temp.path());
    let since_utc = chrono::DateTime::parse_from_rfc3339("2026-03-22T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    runner.enqueue(vec![Effect::SaveBriefingCheckpoint {
        save_id: 7,
        since_utc: Some(since_utc),
    }]);

    let msg = rx
        .recv_timeout(Duration::from_secs(1))
        .expect("expected checkpoint saved msg");

    match msg {
        Msg::BriefingCheckpointSaveSucceeded { save_id } => {
            assert_eq!(save_id, 7);
        }
        other => panic!("unexpected message: {:?}", other),
    }
}

#[test]
fn save_briefing_checkpoint_dispatches_failure_ack() {
    let temp = tempdir().expect("tempdir");
    fs::create_dir_all(temp.path().join(".briefing_checkpoint.ron"))
        .expect("create conflicting checkpoint directory");
    let (runner, rx) = runner_with_receiver(temp.path());
    let since_utc = chrono::DateTime::parse_from_rfc3339("2026-03-22T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    runner.enqueue(vec![Effect::SaveBriefingCheckpoint {
        save_id: 9,
        since_utc: Some(since_utc),
    }]);

    let msg = rx
        .recv_timeout(Duration::from_secs(1))
        .expect("expected checkpoint save failed msg");

    match msg {
        Msg::BriefingCheckpointSaveFailed { save_id, reason } => {
            assert_eq!(save_id, 9);
            assert!(!reason.is_empty());
        }
        other => panic!("unexpected message: {:?}", other),
    }
}

#[test]
fn save_prompt_template_file_writes_file_and_dispatches_saved_msg() {
    let temp = tempdir().expect("tempdir");
    let (runner, rx) = runner_with_receiver(temp.path());
    let prompt_id = PromptId::ArticleTriage;
    let system_template = "system {{context}}".to_string();
    let user_template = "user {{context}}".to_string();
    let description = "desc".to_string();
    let expected_format = "json".to_string();
    runner.enqueue(vec![Effect::SavePromptTemplateFile {
        prompt_id,
        system_template: system_template.clone(),
        user_template: user_template.clone(),
        description: description.clone(),
        expected_format: expected_format.clone(),
    }]);

    let msg = rx
        .recv_timeout(Duration::from_secs(1))
        .expect("expected template saved msg");

    match msg {
        Msg::PromptLabTemplateSaved {
            prompt_id: received,
            version,
            path,
        } => {
            assert_eq!(received, prompt_id);
            let template_path = std::path::PathBuf::from(&path);
            let prompts_dir = template_path
                .parent()
                .and_then(|dir| dir.parent())
                .expect("template file stored under prompts/<prompt_id>");
            let loaded = crate::load_prompt_templates(prompts_dir)
                .into_iter()
                .collect::<Result<Vec<_>, _>>()
                .expect("load saved templates");
            let saved = loaded
                .into_iter()
                .find(|template| template.path == template_path)
                .expect("saved template present");
            assert_eq!(saved.prompt_id, prompt_id);
            assert_eq!(saved.template_file.version, version);
            assert_eq!(saved.template_file.system_template, system_template);
            assert_eq!(saved.template_file.user_template, user_template);
            assert_eq!(saved.template_file.description, description);
            assert_eq!(saved.template_file.expected_format, expected_format);
        }
        other => panic!("unexpected message: {:?}", other),
    }
}

#[test]
fn load_articles_for_briefing_prereq_dispatches_loaded_message() {
    let temp = tempdir().expect("tempdir");
    write_markdown(temp.path(), "a.md", "https://example.com/a");
    let (runner, rx) = runner_with_receiver(temp.path());
    runner.enqueue(vec![Effect::LoadArticlesForBriefingPrereq {
        ordered_urls: vec!["https://example.com/a".to_string()],
        since_utc: None,
    }]);

    let msg = rx
        .recv_timeout(Duration::from_secs(1))
        .expect("expected prereq loaded message");
    match msg {
        Msg::BriefingPrereqArticlesLoaded { articles } => {
            assert_eq!(articles.len(), 1);
            assert_eq!(articles[0].url, "https://example.com/a");
        }
        other => panic!("unexpected message: {:?}", other),
    }
}

#[test]
fn load_articles_for_triage_respects_since_utc_filter() {
    let temp = tempdir().expect("tempdir");
    write_markdown_with_fetched_utc(
        temp.path(),
        "old.md",
        "https://example.com/old",
        "2026-03-20T00:00:00Z",
    );
    write_markdown_with_fetched_utc(
        temp.path(),
        "new.md",
        "https://example.com/new",
        "2026-03-22T00:00:00Z",
    );
    let (runner, rx) = runner_with_receiver(temp.path());
    let since = chrono::DateTime::parse_from_rfc3339("2026-03-21T18:17:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);

    runner.enqueue(vec![Effect::LoadArticlesForTriage {
        request_id: 1,
        ordered_urls: vec![
            "https://example.com/old".to_string(),
            "https://example.com/new".to_string(),
        ],
        since_utc: Some(since),
    }]);

    let mut saw_progress = false;
    loop {
        let msg = rx
            .recv_timeout(Duration::from_secs(1))
            .expect("expected triage load message");
        match msg {
            Msg::TriageArticlesLoadProgress {
                request_id,
                files_scanned,
                files_total,
                ..
            } => {
                assert_eq!(request_id, 1);
                assert_eq!(files_total, 2);
                assert!(files_scanned >= 1);
                saw_progress = true;
            }
            Msg::TriageArticlesLoaded {
                request_id,
                articles,
            } => {
                assert_eq!(request_id, 1);
                assert_eq!(articles.len(), 1);
                assert_eq!(articles[0].url, "https://example.com/new");
                assert!(
                    saw_progress,
                    "triage loads should emit progress before completion"
                );
                break;
            }
            other => panic!("unexpected message: {:?}", other),
        }
    }
}

#[test]
fn load_articles_for_briefing_with_empty_ordered_urls_dispatches_empty_articles_loaded() {
    let temp = tempdir().expect("tempdir");
    write_markdown(temp.path(), "a.md", "https://example.com/a");
    let (runner, rx) = runner_with_receiver(temp.path());
    runner.enqueue(vec![Effect::LoadArticlesForBriefing {
        ordered_urls: Vec::new(),
        since_utc: None,
    }]);

    let msg = rx
        .recv_timeout(Duration::from_secs(1))
        .expect("expected articles loaded message");
    match msg {
        Msg::ArticlesLoaded {
            articles,
            collection_text,
        } => {
            assert!(articles.is_empty());
            assert!(collection_text.is_empty());
        }
        other => panic!("unexpected message: {:?}", other),
    }
}

// --- map_llm_event failure metadata propagation tests ---

fn make_failure_metadata() -> harvester_engine::llm::run_metadata::LlmFailureMetadata {
    use harvester_engine::llm::run_metadata::LlmFailureMetadata;
    LlmFailureMetadata {
        prompt_id: PromptId::ArticleTriage,
        prompt_version: 1,
        resolved_model: Some(OPENAI_MODEL_GPT_4O_MINI.to_string()),
        input_bytes: 100,
        wall_ms: Some(200),
        timestamp_utc: "2026-02-15T00:00:00Z".to_string(),
    }
}

#[test]
fn map_llm_event_validation_failed_with_metadata_propagates_it() {
    let failure_metadata = make_failure_metadata();
    let event = LlmEvent::Completed {
        request_id: 1,
        result: Err(LlmCompletionError::ValidationFailed {
            reason: "bad json".to_string(),
            raw_response: "{}".to_string(),
            failure_metadata: Some(failure_metadata),
        }),
    };
    let msg = map_llm_event(event);
    if let Msg::LlmCompleted { metadata, .. } = msg {
        assert!(
            metadata.is_some(),
            "ValidationFailed with metadata should propagate it"
        );
        assert!(!metadata.unwrap().parse_ok);
    } else {
        panic!("expected LlmCompleted");
    }
}

#[test]
fn map_llm_event_quota_exhausted_with_metadata_propagates_it() {
    let failure_metadata = make_failure_metadata();
    let event = LlmEvent::Completed {
        request_id: 1,
        result: Err(LlmCompletionError::QuotaExhausted {
            description: "rate limited".to_string(),
            failure_metadata: Some(failure_metadata),
        }),
    };
    let msg = map_llm_event(event);
    if let Msg::LlmCompleted { metadata, .. } = msg {
        assert!(
            metadata.is_some(),
            "QuotaExhausted with metadata should propagate it"
        );
    } else {
        panic!("expected LlmCompleted");
    }
}

#[test]
fn map_llm_event_persistence_failed_with_metadata_propagates_it() {
    let failure_metadata = make_failure_metadata();
    let event = LlmEvent::Completed {
        request_id: 1,
        result: Err(LlmCompletionError::PersistenceFailed {
            detail: "disk full".to_string(),
            failure_metadata: Some(failure_metadata),
        }),
    };
    let msg = map_llm_event(event);
    if let Msg::LlmCompleted { metadata, .. } = msg {
        assert!(
            metadata.is_some(),
            "PersistenceFailed with metadata should propagate it"
        );
    } else {
        panic!("expected LlmCompleted");
    }
}

#[test]
fn map_llm_event_unsupported_model_has_none_metadata() {
    use harvester_engine::llm::types::{ModelId, ProviderKind};
    let event = LlmEvent::Completed {
        request_id: 1,
        result: Err(LlmCompletionError::UnsupportedModel {
            model: ModelId::new(ProviderKind::OpenAi, "bad-model"),
            reason: "unknown".to_string(),
        }),
    };
    let msg = map_llm_event(event);
    if let Msg::LlmCompleted {
        metadata, result, ..
    } = msg
    {
        assert!(
            metadata.is_none(),
            "UnsupportedModel is pre-flight so metadata=None"
        );
        assert!(matches!(result, LlmResultKind::Failed { .. }));
    } else {
        panic!("expected LlmCompleted");
    }
}

#[test]
fn resolve_effect_success_emits_ok_msg() {
    let temp = tempdir().expect("tempdir");
    write_markdown(temp.path(), "a.md", "https://example.com/a");
    let (runner, rx) = runner_with_receiver(temp.path());
    runner.enqueue(vec![Effect::ResolvePromptLabInputFromUrl {
        resolve_id: 7,
        url: "https://example.com/a".to_string(),
    }]);

    let msg = rx
        .recv_timeout(Duration::from_secs(2))
        .expect("expected prompt lab resolve msg");
    match msg {
        Msg::PromptLabInputResolved {
            resolve_id,
            result: Ok(snapshot),
        } => {
            assert_eq!(resolve_id, 7);
            assert!(!snapshot.is_empty());
        }
        other => panic!("unexpected message: {:?}", other),
    }
}

#[test]
fn resolve_effect_failure_emits_err_msg() {
    let temp = tempdir().expect("tempdir");
    let (runner, rx) = runner_with_receiver(temp.path());
    runner.enqueue(vec![Effect::ResolvePromptLabInputFromUrl {
        resolve_id: 8,
        url: "https://example.com/missing".to_string(),
    }]);

    let msg = rx
        .recv_timeout(Duration::from_secs(2))
        .expect("expected prompt lab resolve msg");
    match msg {
        Msg::PromptLabInputResolved {
            resolve_id,
            result: Err(reason),
        } => {
            assert_eq!(resolve_id, 8);
            assert!(!reason.is_empty());
        }
        other => panic!("unexpected message: {:?}", other),
    }
}

#[test]
fn archive_requested_writes_archive_markdown_for_selected_urls() {
    let temp = tempdir().expect("tempdir");
    let output = temp.path();
    let md_a = "---\nurl: \"https://example.com/a\"\ntitle: \"A\"\ntoken_count: 1\nfetched_utc: \"2026-02-10T00:00:00Z\"\nencoding: \"UTF-8\"\n---\n\nA body\n";
    let md_b = "---\nurl: \"https://example.com/b\"\ntitle: \"B\"\ntoken_count: 2\nfetched_utc: \"2026-02-11T00:00:00Z\"\nencoding: \"UTF-8\"\n---\n\nB body\n";
    fs::write(output.join("a.md"), md_a).expect("write a");
    fs::write(output.join("b.md"), md_b).expect("write b");

    let (runner, rx) = runner_with_receiver(output);
    runner.enqueue(vec![Effect::ArchiveRequested {
        request_id: 7,
        basename: "archive.md".to_string(),
        ordered_urls: vec!["https://example.com/b".to_string()],
        since_utc: None,
        requested_checkpoint: None,
    }]);

    let msg = rx
        .recv_timeout(Duration::from_secs(1))
        .expect("expected archive export completed message");
    let archive_path = match msg {
        Msg::ArchiveExportCompleted {
            request_id,
            path,
            doc_count,
            requested_checkpoint,
        } => {
            assert_eq!(request_id, 7);
            assert_eq!(doc_count, 1);
            assert_eq!(requested_checkpoint, None);
            path
        }
        other => panic!("unexpected message: {:?}", other),
    };
    assert_eq!(
        archive_path.file_name().and_then(|n| n.to_str()),
        Some("archive.md")
    );
    let archive = fs::read_to_string(&archive_path).expect("read archive");
    assert!(archive.starts_with("===== DOC START ====="));
    assert!(archive.contains("https://example.com/b"));
    assert!(archive.contains("B body"));
    assert!(!archive.contains("https://example.com/a"));
    assert!(!archive.contains("A body"));
}
