//! Read-only archive benchmark and clean-text duplicate measurement.

use std::collections::{BTreeMap, BTreeSet};
use std::{env, fs, path::PathBuf};

use chrono::{DateTime, Utc};
use harvester_core::signal_key_merge::{
    group_signal_keys,
    report::{build_grouping_report, exported_docs, parse_archive_index, ReportExpectations},
};
use harvester_engine::llm::EMPTY_CLEAN_TEXT_HASH;
use harvester_engine::{archive_url_key, scan_archive_window_metadata, WindowArticle};

const FIXTURE: &str =
    include_str!("../tests/fixtures/signal_key_merge/archive-2026-10-07.index.txt");
const EXPECTATIONS: &str = include_str!("../tests/fixtures/signal_key_merge/expectations.json");
const USAGE: &str = "usage: archive_redundancy_report --archive <file>\n       archive_redundancy_report --corpus <dir> --since <rfc3339> --until <rfc3339> [--archive <file>]";

#[derive(Default)]
struct Args {
    archive: Option<PathBuf>,
    corpus: Option<PathBuf>,
    since: Option<DateTime<Utc>>,
    until: Option<DateTime<Utc>>,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args::default();
    let mut seen = BTreeSet::new();
    let mut input = env::args().skip(1);
    while let Some(flag) = input.next() {
        if !matches!(
            flag.as_str(),
            "--archive" | "--corpus" | "--since" | "--until"
        ) {
            return Err(format!("unknown argument {flag}\n{USAGE}"));
        }
        if !seen.insert(flag.clone()) {
            return Err(format!("repeated argument {flag}"));
        }
        let value = input
            .next()
            .ok_or_else(|| format!("missing value for {flag}"))?;
        match flag.as_str() {
            "--archive" => args.archive = Some(value.into()),
            "--corpus" => args.corpus = Some(value.into()),
            _ => {
                let date = DateTime::parse_from_rfc3339(&value)
                    .map_err(|err| format!("{flag}: {err}"))?
                    .with_timezone(&Utc);
                if flag == "--since" {
                    args.since = Some(date);
                } else {
                    args.until = Some(date);
                }
            }
        }
    }
    if args.corpus.is_some() {
        if args.since.is_none() || args.until.is_none() {
            return Err(format!("--corpus requires --since and --until\n{USAGE}"));
        }
    } else if args.archive.is_none() || args.since.is_some() || args.until.is_some() {
        return Err(USAGE.into());
    }
    Ok(args)
}

fn print_content_report(articles: &[WindowArticle], docs: Option<&BTreeMap<String, usize>>) {
    let mut by_hash: BTreeMap<&str, Vec<&WindowArticle>> = BTreeMap::new();
    for article in articles {
        by_hash
            .entry(&article.content_hash)
            .or_default()
            .push(article);
    }
    println!("Window articles: {}", articles.len());
    if let Some(docs) = docs {
        let matched = articles
            .iter()
            .filter(|a| docs.contains_key(&archive_url_key(&a.url)))
            .count();
        println!("Archive documents in window: {matched} / {}", docs.len());
    }
    // SHA-256 of empty clean text represents failed extraction, not copies.
    let empty_text = by_hash.remove(EMPTY_CLEAN_TEXT_HASH);
    let duplicate_groups: Vec<_> = by_hash
        .into_iter()
        .filter(|(_, members)| members.len() > 1)
        .collect();
    println!(
        "Identical-content candidate groups in window (excluding empty clean text): {}",
        duplicate_groups.len()
    );
    if let Some(docs) = docs {
        let exported_groups = duplicate_groups
            .iter()
            .filter(|(_, members)| {
                members
                    .iter()
                    .filter(|a| docs.contains_key(&archive_url_key(&a.url)))
                    .count()
                    > 1
            })
            .count();
        println!("Identical-content candidate groups among exported documents: {exported_groups}");
    }
    let mut reported_groups: Vec<_> = duplicate_groups
        .into_iter()
        .map(|(hash, members)| (format!("Content hash: {hash}"), members))
        .collect();
    if let Some(members) = empty_text {
        reported_groups.push((
            "Empty clean text (extraction failure; not an identical-content candidate)".into(),
            members,
        ));
    }
    for (label, mut members) in reported_groups {
        members.sort_by_key(|a| &a.url);
        println!("\n{label} ({} articles)", members.len());
        for article in members {
            let marker = match docs {
                None => "archive not supplied".into(),
                Some(docs) => docs.get(&archive_url_key(&article.url)).map_or_else(
                    || "not exported".into(),
                    |doc| format!("exported doc {doc}"),
                ),
            };
            println!(
                "  [{marker}] {} | {} | {}",
                article.url,
                article.source_title.as_deref().unwrap_or("(no title)"),
                article.fetched_utc.as_deref().unwrap_or("(no fetch time)")
            );
        }
    }
}

fn run() -> Result<(), String> {
    let args = parse_args()?;
    let archive = args
        .archive
        .as_ref()
        .map(|path| fs::read_to_string(path).map_err(|err| format!("{}: {err}", path.display())))
        .transpose()?;
    if let Some(corpus) = args.corpus {
        let since = args.since.expect("validated since");
        let until = args.until.expect("validated until");
        let articles = scan_archive_window_metadata(&corpus, since, until)?;
        println!(
            "Window: {} .. {} (inclusive)",
            since.to_rfc3339(),
            until.to_rfc3339()
        );
        let docs = archive.as_deref().map(exported_docs).transpose()?;
        print_content_report(&articles, docs.as_ref());
    } else {
        let rows = parse_archive_index(archive.as_deref().expect("validated archive"))?;
        // Human labels apply only to the exact selected-index benchmark, never
        // to unrelated archives that happen to reuse the same document numbers.
        let expectations = if rows == parse_archive_index(FIXTURE)? {
            serde_json::from_str(EXPECTATIONS).map_err(|err| err.to_string())?
        } else {
            ReportExpectations::default()
        };
        let groups = group_signal_keys(
            rows.iter()
                .filter(|row| !row.signal_key.is_empty())
                .map(|row| &row.signal_key),
        );
        print!("{}", build_grouping_report(&rows, &groups, &expectations)?);
    }
    Ok(())
}

fn main() -> Result<(), String> {
    run()
}
