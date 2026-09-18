//! Dataset selection, freeze writing, and precondition data collection.

use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::bail;
use chrono::{DateTime, Utc};
use engine_logging::{engine_info, engine_warn};
use harvester_engine::llm::{load_replay_record, validate_triage, PromptId, TriageResult};

use crate::article_index::{
    build_index, build_or_load_index, match_article, ArticleIndex, MappingOutcome,
};
use crate::identity::{
    classify_path, evidence_hash, is_truncated, verify_source_identity, PathKind,
};
use crate::manifest::{
    write_manifest, ArticleEntry, ArticleFile, Baseline, DatasetSplit, FrozenInputs, Manifest,
    ManifestCounts, Selection, Usage, MANIFEST_VERSION,
};
use crate::preconditions::PreconditionReport;
use crate::prompt_identity::{
    classify_record, load_prompt_identity, sha256, PromptIdentity, PromptMatch,
};
use crate::recovery::recover_document_text;

/// Inputs shared by dry-run and persistent freezes.
#[derive(Debug, Clone)]
pub struct FreezeOptions {
    pub output_dir: PathBuf,
    pub linked_dir: Option<PathBuf>,
    pub experiment_dir: PathBuf,
    pub limit: usize,
    pub prompt_version: u32,
    pub context_file: PathBuf,
    pub dev_share: f64,
    pub split_seed: u64,
    pub min_priority_5: u64,
    pub dry_run: bool,
}

/// Result available to both the command writer and integration tests.
#[derive(Debug)]
pub struct FreezeResult {
    pub manifest: Manifest,
    pub preconditions: PreconditionReport,
}

#[derive(Debug, Clone)]
struct Candidate {
    evidence_hash: String,
    record_path: PathBuf,
    timestamp_utc: String,
    request_id: String,
    path_kind: PathKind,
    triage: TriageResult,
    source_hashes: BTreeSet<String>,
    path_kinds: BTreeSet<String>,
}

/// Performs a dry run or writes the reproducible frozen dataset.
pub fn freeze(options: &FreezeOptions) -> anyhow::Result<FreezeResult> {
    if !(0.0..=1.0).contains(&options.dev_share) {
        bail!("--dev-share must be between 0 and 1");
    }
    if options.prompt_version != 4 {
        bail!("Phase 1 supports only --prompt-version 4");
    }
    let identity = load_prompt_identity(&options.context_file)?;
    let mut counts = ManifestCounts::default();
    let mut candidates: HashMap<String, Candidate> = HashMap::new();
    let replay_dir = options.output_dir.join("llm_results");
    if !replay_dir.is_dir() {
        bail!(
            "replay directory is missing: {}; choose the Harvester output directory that contains llm_results",
            replay_dir.display()
        );
    }
    let mut paths = replay_paths(&replay_dir)?;
    if paths.is_empty() {
        bail!(
            "replay directory contains no JSON records: {}",
            replay_dir.display()
        );
    }
    paths.sort();
    for path in paths {
        counts.records_scanned += 1;
        let record = match load_replay_record(&path) {
            Ok(record) => record,
            Err(error) => {
                counts.load_failed += 1;
                engine_warn!("[harvester_eval] run_id=freeze article_id=unknown operation=load_replay path={} error={error}", path.display());
                continue;
            }
        };
        if record.prompt_id != PromptId::ArticleTriage {
            continue;
        }
        counts.prompt_id_matched += 1;
        if record.prompt_version != options.prompt_version {
            continue;
        }
        counts.version_matched += 1;
        if classify_record(&record, options.prompt_version, &identity) != PromptMatch::Match {
            continue;
        }
        counts.identity_matched += 1;
        if record.validation_error.is_some() || record.validated_output.is_none() {
            counts.validation_failed += 1;
            engine_warn!("[harvester_eval] run_id=freeze article_id=unknown request_id={} operation=validate_replay path={} error=recorded_validation_failure", record.request_id, path.display());
            continue;
        }
        let triage_json = match serde_json::to_string(
            record
                .validated_output
                .as_ref()
                .expect("validated output was checked above"),
        ) {
            Ok(value) => value,
            Err(error) => {
                counts.validation_failed += 1;
                engine_warn!("[harvester_eval] run_id=freeze article_id=unknown request_id={} operation=serialize_validated_output path={} error={error}", record.request_id, path.display());
                continue;
            }
        };
        let triage = match validate_triage(&triage_json) {
            Ok(value) => value,
            Err(error) => {
                counts.validation_failed += 1;
                engine_warn!("[harvester_eval] run_id=freeze article_id=unknown request_id={} operation=revalidate_triage path={} error={error}", record.request_id, path.display());
                continue;
            }
        };
        let recovered = match recover_document_text(&record.rendered_user_message) {
            Ok(value) => value,
            Err(error) => {
                counts.recovery_failed += 1;
                engine_warn!("[harvester_eval] run_id=freeze article_id=unknown operation=recover path={} error={error}", path.display());
                continue;
            }
        };
        let path_kind = classify_path(&record);
        let source_identity = verify_source_identity(path_kind, &record, &recovered);
        if source_identity.anomaly.is_some() {
            counts.identity_anomalies += 1;
        }
        let evidence = evidence_hash(&recovered.text);
        let candidate = Candidate {
            evidence_hash: evidence.clone(),
            record_path: path,
            timestamp_utc: record.timestamp_utc.clone(),
            request_id: record.request_id.clone(),
            path_kind,
            triage,
            source_hashes: BTreeSet::from([record.input_content_hash.clone()]),
            path_kinds: BTreeSet::from([format!("{path_kind:?}")]),
        };
        candidates
            .entry(evidence)
            .and_modify(|existing| {
                existing
                    .source_hashes
                    .extend(candidate.source_hashes.clone());
                existing.path_kinds.extend(candidate.path_kinds.clone());
                if is_newer(&candidate, existing) {
                    existing.record_path = candidate.record_path.clone();
                    existing.timestamp_utc = candidate.timestamp_utc.clone();
                    existing.request_id = candidate.request_id.clone();
                    existing.path_kind = candidate.path_kind;
                    existing.triage = candidate.triage.clone();
                }
            })
            .or_insert(candidate);
    }
    if counts.identity_matched == 0 {
        bail!(
            "no replay records in {} match ArticleTriage prompt version {} and the configured prompt identity",
            replay_dir.display(),
            options.prompt_version
        );
    }
    if candidates.is_empty() {
        bail!(
            "no usable matching replay records in {}; matching records failed validation or text recovery",
            replay_dir.display()
        );
    }
    counts.distinct_articles = candidates.len() as u64;
    let usable_records = counts
        .identity_matched
        .saturating_sub(counts.validation_failed)
        .saturating_sub(counts.recovery_failed);
    counts.exact_duplicates = usable_records.saturating_sub(counts.distinct_articles);
    let mut candidates = candidates.into_values().collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        compare_timestamps(&right.timestamp_utc, &left.timestamp_utc)
            .then_with(|| right.request_id.cmp(&left.request_id))
            .then_with(|| right.evidence_hash.cmp(&left.evidence_hash))
    });
    candidates.truncate(options.limit);
    let cross_path_duplicates = candidates
        .iter()
        .filter(|candidate| candidate.path_kinds.len() > 1)
        .count() as u64;
    let index = if options.dry_run {
        build_index(&options.output_dir, options.linked_dir.as_deref())?
    } else {
        build_or_load_index(
            &options.output_dir,
            options.linked_dir.as_deref(),
            &options.experiment_dir.join("article-index.json"),
        )?
    };
    counts.index_files_skipped = index.stats.skipped as u64;
    let entries = candidates
        .into_iter()
        .map(|candidate| make_entry(candidate, &index))
        .collect::<anyhow::Result<Vec<_>>>()?;
    counts.selected = entries.len() as u64;
    counts.sync_records = entries
        .iter()
        .filter(|entry| entry.path_kind == PathKind::Sync)
        .count() as u64;
    counts.batch_records = entries
        .iter()
        .filter(|entry| entry.path_kind == PathKind::Batch)
        .count() as u64;
    counts.truncated = entries.iter().filter(|entry| entry.truncated).count() as u64;
    counts.mapped = entries
        .iter()
        .filter(|entry| entry.mapping_status.starts_with("mapped_"))
        .count() as u64;
    counts.unmapped = entries
        .iter()
        .filter(|entry| entry.mapping_status == "unmapped")
        .count() as u64;
    counts.ambiguous = entries
        .iter()
        .filter(|entry| entry.mapping_status == "ambiguous")
        .count() as u64;
    counts.cross_path_duplicates = cross_path_duplicates;
    counts.duplicate_groups = duplicate_group_count(&entries);
    counts.instruction_like = entries
        .iter()
        .filter(|entry| entry.instruction_like)
        .count() as u64;
    let mut manifest = Manifest {
        manifest_version: MANIFEST_VERSION,
        tool_version: env!("CARGO_PKG_VERSION").to_string(),
        created_utc: Utc::now().to_rfc3339(),
        source_output_dir: options.output_dir.to_string_lossy().into_owned(),
        selection: Selection {
            prompt_id: "ArticleTriage".into(),
            prompt_version: options.prompt_version,
            limit: options.limit,
            ordering: "timestamp_utc descending, request_id descending, evidence_hash descending"
                .into(),
            dev_share: options.dev_share,
            split_seed: options.split_seed,
            context_file: options.context_file.to_string_lossy().into_owned(),
        },
        frozen_inputs: FrozenInputs {
            rubric_path: "rubric.txt".into(),
            rubric_sha256: identity.rubric_sha256.clone(),
            prompt_identity_path: "prompt-identity.txt".into(),
            system_sha256: identity.system_sha256.clone(),
            template_sha256: identity.template_sha256.clone(),
            context_version: identity.context_version,
        },
        counts,
        articles: entries,
        manifest_hash: String::new(),
    };
    assign_splits(
        &mut manifest.articles,
        options.split_seed,
        options.dev_share,
    );
    manifest.refresh_hash()?;
    let preconditions = PreconditionReport::from_manifest(&manifest, options.min_priority_5);
    if !options.dry_run {
        write_frozen_inputs(&options.experiment_dir, &manifest, &identity)?;
    }
    engine_info!("[harvester_eval] run_id=freeze article_id=all operation=freeze_complete records_scanned={} selected={} mapped={} unmapped={} ambiguous={} validation_failed={} recovery_failed={}", manifest.counts.records_scanned, manifest.counts.selected, manifest.counts.mapped, manifest.counts.unmapped, manifest.counts.ambiguous, manifest.counts.validation_failed, manifest.counts.recovery_failed);
    Ok(FreezeResult {
        manifest,
        preconditions,
    })
}

fn make_entry(candidate: Candidate, index: &ArticleIndex) -> anyhow::Result<ArticleEntry> {
    let record = load_replay_record(&candidate.record_path).map_err(anyhow::Error::msg)?;
    let recovered =
        recover_document_text(&record.rendered_user_message).map_err(anyhow::Error::msg)?;
    let source = verify_source_identity(candidate.path_kind, &record, &recovered);
    let mapping = match_article(&record.input_content_hash, &recovered.text, index)?;
    let article_file = match &mapping {
        MappingOutcome::MappedByHash(entry_index) | MappingOutcome::MappedByPrefix(entry_index) => {
            Some(article_file(&index.entries[*entry_index]))
        }
        MappingOutcome::Unmapped | MappingOutcome::Ambiguous(_) => None,
    };
    let mut ambiguous_paths = match &mapping {
        MappingOutcome::Ambiguous(entry_indexes) => entry_indexes
            .iter()
            .map(|entry_index| index.entries[*entry_index].path.clone())
            .collect::<Vec<_>>(),
        _ => Vec::new(),
    };
    ambiguous_paths.sort();
    Ok(ArticleEntry {
        article_id: candidate.evidence_hash[..16].to_string(),
        evidence_hash: candidate.evidence_hash,
        source_content_hash: record.input_content_hash.clone(),
        source_content_hashes: candidate.source_hashes.into_iter().collect(),
        path_kind: candidate.path_kind,
        text_path: format!("articles/{}.txt", &evidence_hash(&recovered.text)[..16]),
        text_bytes: recovered.text.len(),
        truncated: is_truncated(&recovered.text),
        nonce: recovered.nonce,
        nonce_verified: recovered.nonce_verified,
        evidence_matches_source: source.matches_evidence,
        identity_anomaly: source.anomaly,
        split: DatasetSplit::Dev,
        duplicate_group: duplicate_group_key(&recovered.text),
        instruction_like: instruction_like(&recovered.text),
        article_file,
        ambiguous_paths,
        mapping_status: mapping.status().to_string(),
        baseline: Baseline {
            record_path: candidate.record_path.to_string_lossy().into_owned(),
            request_id: record.request_id,
            path_kind: candidate.path_kind,
            model_id: record.model_id,
            recorded_utc: record.timestamp_utc,
            priority: candidate.triage.priority.value(),
            category: candidate.triage.category,
            tags: candidate.triage.tags,
            rationale: candidate.triage.rationale,
            usage: Usage {
                input_tokens: record.usage.input_tokens,
                output_tokens: record.usage.output_tokens,
                cached_input_tokens: record.usage.cached_input_tokens,
            },
            cost_microdollars: record.cost_microdollars,
            wall_ms: record.wall_ms,
            cache_status: record.cache_status,
        },
    })
}

fn article_file(entry: &crate::article_index::ArticleIndexEntry) -> ArticleFile {
    ArticleFile {
        path: entry.path.clone(),
        title: entry.title.clone(),
        url: entry.url.clone(),
        fetched_utc: entry.fetched_utc.clone(),
    }
}

fn replay_paths(dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    Ok(fs::read_dir(dir)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path.extension().and_then(|extension| extension.to_str()) == Some("json")
        })
        .collect())
}

fn is_newer(candidate: &Candidate, existing: &Candidate) -> bool {
    compare_timestamps(&candidate.timestamp_utc, &existing.timestamp_utc)
        .then_with(|| candidate.request_id.cmp(&existing.request_id))
        == Ordering::Greater
}

fn compare_timestamps(left: &str, right: &str) -> Ordering {
    match (
        DateTime::parse_from_rfc3339(left),
        DateTime::parse_from_rfc3339(right),
    ) {
        (Ok(left), Ok(right)) => left.cmp(&right),
        _ => left.cmp(right),
    }
}

fn duplicate_group_key(text: &str) -> String {
    let normalized = text
        .chars()
        .take(2048)
        .map(|character| {
            if character.is_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    sha256(&normalized)
}

fn duplicate_group_count(entries: &[ArticleEntry]) -> u64 {
    let mut groups = HashMap::<&str, u64>::new();
    for entry in entries {
        *groups.entry(&entry.duplicate_group).or_default() += 1;
    }
    groups.values().filter(|count| **count > 1).count() as u64
}

fn assign_splits(entries: &mut [ArticleEntry], seed: u64, dev_share: f64) {
    let mut groups = entries
        .iter()
        .map(|entry| entry.duplicate_group.clone())
        .collect::<Vec<_>>();
    groups.sort();
    groups.dedup();
    let assignments = split_assignments(&groups, seed, dev_share);
    for entry in entries {
        entry.split = *assignments
            .get(&entry.duplicate_group)
            .expect("every duplicate group receives a split");
    }
}

fn split_assignments(
    groups: &[String],
    seed: u64,
    dev_share: f64,
) -> HashMap<String, DatasetSplit> {
    let mut groups = groups.to_vec();
    groups.sort_by_key(|group| sha256(&format!("{seed}:{group}")));
    let dev_groups = ((groups.len() as f64) * dev_share).round() as usize;
    let dev = groups
        .iter()
        .take(dev_groups)
        .cloned()
        .collect::<BTreeSet<_>>();
    groups
        .into_iter()
        .map(|group| {
            let split = if dev.contains(&group) {
                DatasetSplit::Dev
            } else {
                DatasetSplit::Heldout
            };
            (group, split)
        })
        .collect()
}

fn instruction_like(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    [
        "ignore previous instructions",
        "system prompt",
        "```instruction",
        "```system",
    ]
    .iter()
    .any(|needle| text.contains(needle))
}

fn write_frozen_inputs(
    experiment_dir: &Path,
    manifest: &Manifest,
    identity: &PromptIdentity,
) -> anyhow::Result<()> {
    fs::create_dir_all(experiment_dir.join("articles"))?;
    fs::write(experiment_dir.join("rubric.txt"), &identity.rubric_text)?;
    fs::write(
        experiment_dir.join("prompt-identity.txt"),
        &identity.system_message,
    )?;
    for entry in &manifest.articles {
        let record = load_replay_record(Path::new(&entry.baseline.record_path))
            .map_err(anyhow::Error::msg)?;
        let recovered =
            recover_document_text(&record.rendered_user_message).map_err(anyhow::Error::msg)?;
        let target = experiment_dir.join(&entry.text_path);
        fs::write(target, recovered.text)?;
    }
    write_manifest(experiment_dir, manifest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_assignment_is_deterministic_and_keeps_half_of_ten_groups_in_dev() {
        let groups = (0..10)
            .map(|value| format!("group-{value}"))
            .collect::<Vec<_>>();
        let first = split_assignments(&groups, 7, 0.5);
        assert_eq!(first, split_assignments(&groups, 7, 0.5));
        assert_eq!(
            first
                .values()
                .filter(|split| **split == DatasetSplit::Dev)
                .count(),
            5
        );
        assert_ne!(first, split_assignments(&groups, 8, 0.5));
    }
}
