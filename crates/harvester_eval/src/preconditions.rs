//! Human-readable and JSON preconditions for a prospective frozen dataset.

use std::collections::BTreeMap;

use chrono::DateTime;
use serde::Serialize;

use crate::manifest::{DatasetSplit, Manifest};

/// The machine-readable precondition report emitted by `freeze --dry-run`.
#[derive(Debug, Clone, Serialize)]
pub struct PreconditionReport {
    pub matching_records: u64,
    pub rejection_counts: RejectionCounts,
    pub selected: u64,
    pub mapping: MappingCounts,
    pub priority_histogram: BTreeMap<u8, u64>,
    pub projected_priority_5: u64,
    pub projected_priority_4_plus_5: u64,
    pub priority_5_warning: bool,
    pub truncation_count: u64,
    pub exact_duplicates: u64,
    pub duplicate_groups: u64,
    pub cross_path_duplicates: u64,
    pub sync_records: u64,
    pub batch_records: u64,
    pub identity_anomalies: u64,
    pub validation_failures: u64,
    pub instruction_like: u64,
    pub total_frozen_bytes: u64,
    pub rough_input_tokens: u64,
    pub date_range: Option<DateRange>,
    pub unmapped_article_ids: Vec<String>,
    pub splits: BTreeMap<String, u64>,
    pub index_files_skipped: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct RejectionCounts {
    pub scanned: u64,
    pub load_failed: u64,
    pub wrong_prompt: u64,
    pub wrong_version: u64,
    pub prompt_mismatch: u64,
    pub validation_failed: u64,
    pub recovery_failed: u64,
}
#[derive(Debug, Clone, Serialize)]
pub struct MappingCounts {
    pub mapped_by_hash: u64,
    pub mapped_by_prefix: u64,
    pub unmapped: u64,
    pub ambiguous: u64,
}
#[derive(Debug, Clone, Serialize)]
pub struct DateRange {
    pub oldest: String,
    pub newest: String,
}

impl PreconditionReport {
    pub fn from_manifest(manifest: &Manifest, min_priority_5: u64) -> Self {
        let mut priority_histogram = BTreeMap::new();
        let mut mapping = MappingCounts {
            mapped_by_hash: 0,
            mapped_by_prefix: 0,
            unmapped: 0,
            ambiguous: 0,
        };
        let mut splits = BTreeMap::new();
        let mut dates = Vec::new();
        let mut total_frozen_bytes = 0_u64;
        for article in &manifest.articles {
            *priority_histogram
                .entry(article.baseline.priority)
                .or_default() += 1;
            *splits
                .entry(match article.split {
                    DatasetSplit::Dev => "dev".to_string(),
                    DatasetSplit::Heldout => "heldout".to_string(),
                })
                .or_default() += 1;
            match article.mapping_status.as_str() {
                "mapped_by_hash" => mapping.mapped_by_hash += 1,
                "mapped_by_prefix" => mapping.mapped_by_prefix += 1,
                "ambiguous" => mapping.ambiguous += 1,
                _ => mapping.unmapped += 1,
            }
            dates.push(article.baseline.recorded_utc.clone());
            total_frozen_bytes += article.text_bytes as u64;
        }
        dates.sort_by(|left, right| {
            match (
                DateTime::parse_from_rfc3339(left),
                DateTime::parse_from_rfc3339(right),
            ) {
                (Ok(left), Ok(right)) => left.cmp(&right),
                _ => left.cmp(right),
            }
        });
        let projected_priority_5 = *priority_histogram.get(&5).unwrap_or(&0);
        let projected_priority_4_plus_5 =
            projected_priority_5 + priority_histogram.get(&4).copied().unwrap_or(0);
        Self {
            matching_records: manifest.counts.identity_matched,
            rejection_counts: RejectionCounts {
                scanned: manifest.counts.records_scanned,
                load_failed: manifest.counts.load_failed,
                wrong_prompt: manifest
                    .counts
                    .records_scanned
                    .saturating_sub(manifest.counts.load_failed)
                    .saturating_sub(manifest.counts.prompt_id_matched),
                wrong_version: manifest.counts.prompt_id_matched - manifest.counts.version_matched,
                prompt_mismatch: manifest.counts.version_matched - manifest.counts.identity_matched,
                validation_failed: manifest.counts.validation_failed,
                recovery_failed: manifest.counts.recovery_failed,
            },
            selected: manifest.counts.selected,
            mapping,
            priority_histogram,
            projected_priority_5,
            projected_priority_4_plus_5,
            priority_5_warning: projected_priority_5 < min_priority_5,
            truncation_count: manifest.counts.truncated,
            exact_duplicates: manifest.counts.exact_duplicates,
            duplicate_groups: manifest.counts.duplicate_groups,
            cross_path_duplicates: manifest.counts.cross_path_duplicates,
            sync_records: manifest.counts.sync_records,
            batch_records: manifest.counts.batch_records,
            identity_anomalies: manifest.counts.identity_anomalies,
            validation_failures: manifest.counts.validation_failed,
            instruction_like: manifest.counts.instruction_like,
            total_frozen_bytes,
            rough_input_tokens: total_frozen_bytes.div_ceil(4),
            date_range: dates
                .first()
                .zip(dates.last())
                .map(|(oldest, newest)| DateRange {
                    oldest: oldest.clone(),
                    newest: newest.clone(),
                }),
            unmapped_article_ids: manifest
                .articles
                .iter()
                .filter(|article| article.mapping_status == "unmapped")
                .map(|article| article.article_id.clone())
                .collect(),
            splits,
            index_files_skipped: manifest.counts.index_files_skipped,
        }
    }

    pub fn render(&self) -> String {
        let percentage = |part: u64, total: u64| {
            if total == 0 {
                0.0
            } else {
                part as f64 * 100.0 / total as f64
            }
        };
        let rejected = &self.rejection_counts;
        let mut text = format!(
            "Dataset preconditions\nmatching records: {}\nrejections (of {} scanned): load_failed={} ({:.1}%) wrong_prompt={} ({:.1}%) wrong_version={} ({:.1}%) prompt_mismatch={} ({:.1}%) validation_failed={} ({:.1}%) recovery_failed={} ({:.1}%)\nselected distinct articles: {}\nmapping: hash={} ({:.1}%) prefix={} ({:.1}%) unmapped={} ({:.1}%) ambiguous={} ({:.1}%)\npriority histogram: {:?}\npriority 5 projected: {}; priority 4+5 projected: {}\ntruncated: {} ({:.1}%)\nexact duplicates: {}; near-duplicate groups: {}; cross-path duplicates: {}\nsync/batch: {} ({:.1}%)/{} ({:.1}%)\nidentity anomalies: {}; validation failures: {}; instruction-like: {}; index files skipped: {}\nfrozen bytes: {}; rough input tokens: {}\n",
            self.matching_records,
            rejected.scanned,
            rejected.load_failed,
            percentage(rejected.load_failed, rejected.scanned),
            rejected.wrong_prompt,
            percentage(rejected.wrong_prompt, rejected.scanned),
            rejected.wrong_version,
            percentage(rejected.wrong_version, rejected.scanned),
            rejected.prompt_mismatch,
            percentage(rejected.prompt_mismatch, rejected.scanned),
            rejected.validation_failed,
            percentage(rejected.validation_failed, rejected.scanned),
            rejected.recovery_failed,
            percentage(rejected.recovery_failed, rejected.scanned),
            self.selected,
            self.mapping.mapped_by_hash,
            percentage(self.mapping.mapped_by_hash, self.selected),
            self.mapping.mapped_by_prefix,
            percentage(self.mapping.mapped_by_prefix, self.selected),
            self.mapping.unmapped,
            percentage(self.mapping.unmapped, self.selected),
            self.mapping.ambiguous,
            percentage(self.mapping.ambiguous, self.selected),
            self.priority_histogram,
            self.projected_priority_5,
            self.projected_priority_4_plus_5,
            self.truncation_count,
            percentage(self.truncation_count, self.selected),
            self.exact_duplicates,
            self.duplicate_groups,
            self.cross_path_duplicates,
            self.sync_records,
            percentage(self.sync_records, self.selected),
            self.batch_records,
            percentage(self.batch_records, self.selected),
            self.identity_anomalies,
            self.validation_failures,
            self.instruction_like,
            self.index_files_skipped,
            self.total_frozen_bytes,
            self.rough_input_tokens
        );
        if let Some(range) = &self.date_range {
            text.push_str(&format!(
                "date range: {} to {}\n",
                range.oldest, range.newest
            ));
        }
        text.push_str(&format!(
            "splits: dev={} heldout={}\n",
            self.splits.get("dev").copied().unwrap_or(0),
            self.splits.get("heldout").copied().unwrap_or(0)
        ));
        if self.priority_5_warning {
            text.push_str("WARNING: priority-5 support is below the requested minimum.\n");
        }
        if !self.unmapped_article_ids.is_empty() {
            text.push_str(&format!(
                "unmapped article ids: {}\n",
                self.unmapped_article_ids.join(", ")
            ));
        }
        text
    }
}
