use super::{url_lookup_aliases, DateTime, TriageLoadEntry, Utc};
use engine_logging::engine_warn;
use std::collections::{HashMap, HashSet};
use std::hash::Hash;

#[derive(Clone, Copy)]
struct Article<'a> {
    index: usize,
    url: &'a str,
    inside_window: bool,
}

struct StrippedAlias<'a> {
    first: Article<'a>,
    ambiguous: bool,
}

#[derive(Default)]
pub(super) struct ArticleMatcher<'a> {
    exact: HashMap<&'a str, Article<'a>>,
    aliases: HashMap<String, Article<'a>>,
    without_query: HashMap<String, StrippedAlias<'a>>,
    fetched: HashMap<DateTime<Utc>, Option<Article<'a>>>,
}

enum Match<'a> {
    Unmatched,
    AlreadyClaimed,
    Selected(Article<'a>),
}

impl<'a> Match<'a> {
    fn claim(&mut self, article: Article<'a>, claimed: &mut HashSet<&'a str>) {
        *self = if claimed.insert(article.url) {
            Self::Selected(article)
        } else {
            Self::AlreadyClaimed
        };
    }
}

fn prefer_window<'a, K: Eq + Hash>(
    map: &mut HashMap<K, Article<'a>>,
    key: K,
    article: Article<'a>,
) {
    map.entry(key)
        .and_modify(|first| {
            if !first.inside_window && article.inside_window {
                *first = article;
            }
        })
        .or_insert(article);
}

fn query_and_base(alias: &str) -> (Option<&str>, &str) {
    match alias.split_once('?') {
        Some((base, query)) => (Some(query), base.trim_end_matches('/')),
        None => (None, alias),
    }
}

impl<'a> ArticleMatcher<'a> {
    pub(super) fn index(
        &mut self,
        index: usize,
        url: &'a str,
        fetched: Option<DateTime<Utc>>,
        inside_window: bool,
    ) {
        let article = Article {
            index,
            url,
            inside_window,
        };
        prefer_window(&mut self.exact, url, article);
        for alias in url_lookup_aliases(url) {
            let (_, base) = query_and_base(&alias);
            self.without_query
                .entry(base.to_owned())
                .and_modify(|existing| {
                    if inside_window && !existing.first.inside_window {
                        *existing = StrippedAlias {
                            first: article,
                            ambiguous: false,
                        };
                    } else if inside_window == existing.first.inside_window {
                        existing.ambiguous |= existing.first.url != url;
                    }
                })
                .or_insert(StrippedAlias {
                    first: article,
                    ambiguous: false,
                });
            prefer_window(&mut self.aliases, alias, article);
        }
        if let Some(fetched) = fetched {
            self.fetched
                .entry(fetched)
                .and_modify(|first| *first = None)
                .or_insert(Some(article));
        }
    }

    fn exact(&self, url: &str) -> Option<Article<'a>> {
        self.exact.get(url).copied()
    }

    fn alias(&self, url: &str) -> Option<Article<'a>> {
        url_lookup_aliases(url)
            .iter()
            .filter_map(|alias| self.aliases.get(alias).copied())
            .min_by_key(|article| !article.inside_window)
    }

    fn query_stripped(&self, url: &str) -> Option<Article<'a>> {
        url_lookup_aliases(url)
            .iter()
            .filter_map(|alias| {
                let (query, base) = query_and_base(alias);
                let candidate = self.without_query.get(base)?;
                let article = candidate.first;
                // Two explicit but different queries may identify different articles,
                // even if only one of them has a file on disk.
                let (article_query, _) =
                    query_and_base(article.url.split('#').next().unwrap_or_default());
                (!candidate.ambiguous && (query.is_none() || article_query.is_none()))
                    .then_some(article)
            })
            .min_by_key(|article| !article.inside_window)
    }

    pub(super) fn select(
        &self,
        entries: &[TriageLoadEntry],
        since: Option<DateTime<Utc>>,
        request_id: Option<u64>,
    ) -> Vec<usize> {
        let mut matched: Vec<_> = entries.iter().map(|_| Match::Unmatched).collect();
        let mut claimed = HashSet::new();
        let match_jobs_by_url = |find: fn(&Self, &str) -> Option<Article<'a>>,
                                 matched: &mut [Match<'a>],
                                 claimed: &mut HashSet<&'a str>| {
            // Reserve stronger matches across the whole request. At each rank a
            // saved final URL is stronger evidence than the original requested URL.
            for use_final in [true, false] {
                for (entry, result) in entries.iter().zip(matched.iter_mut()) {
                    if let (TriageLoadEntry::Job { url, final_url, .. }, Match::Unmatched) =
                        (entry, &*result)
                    {
                        let url = if use_final {
                            final_url.as_deref()
                        } else {
                            Some(url.as_str())
                        };
                        if let Some(article) = url.and_then(|url| find(self, url)) {
                            result.claim(article, claimed);
                        }
                    }
                }
            }
        };
        for find in [Self::exact, Self::alias] {
            match_jobs_by_url(find, &mut matched, &mut claimed);
        }

        let mut job_times = HashMap::new();
        for entry in entries {
            if let TriageLoadEntry::Job {
                fetched_utc: Some(fetched),
                ..
            } = entry
            {
                *job_times.entry(*fetched).or_insert(0) += 1;
            }
        }
        for (entry, result) in entries.iter().zip(&mut matched) {
            if let (
                TriageLoadEntry::Job {
                    fetched_utc: Some(fetched),
                    ..
                },
                Match::Unmatched,
            ) = (entry, &*result)
            {
                if job_times.get(fetched) == Some(&1) {
                    if let Some(Some(article)) = self.fetched.get(fetched) {
                        result.claim(*article, &mut claimed);
                    }
                }
            }
        }
        match_jobs_by_url(Self::query_stripped, &mut matched, &mut claimed);

        // A retained article without its job can still be re-prepared. It must
        // never take a match away from a completed job.
        for find in [Self::exact, Self::alias, Self::query_stripped] {
            for (entry, result) in entries.iter().zip(&mut matched) {
                if let (TriageLoadEntry::Url(url), Match::Unmatched) = (entry, &*result) {
                    if let Some(article) = find(self, url) {
                        result.claim(article, &mut claimed);
                    }
                }
            }
        }

        let mut unmatched = 0;
        let mut outside_window = 0;
        let mut already_claimed = 0;
        let mut examples = Vec::new();
        let mut selected = Vec::new();
        for (entry, result) in entries.iter().zip(matched) {
            match (entry, result) {
                (_, Match::Selected(article)) if article.inside_window => {
                    selected.push(article.index);
                }
                (TriageLoadEntry::Job { .. }, Match::AlreadyClaimed) => already_claimed += 1,
                (
                    TriageLoadEntry::Job {
                        id,
                        url,
                        fetched_utc,
                        ..
                    },
                    Match::Unmatched,
                ) => {
                    if since
                        .zip(*fetched_utc)
                        .is_some_and(|(since, fetched)| fetched < since)
                    {
                        outside_window += 1;
                    } else {
                        unmatched += 1;
                        if examples.len() < 20 {
                            examples.push(match id {
                                Some(id) => format!("{url:?} (job_id={id})"),
                                None => format!("{url:?}"),
                            });
                        }
                    }
                }
                _ => {}
            }
        }
        if unmatched + outside_window + already_claimed > 0 {
            let request = request_id
                .map(|id| format!(" request_id={id}"))
                .unwrap_or_default();
            let names = if examples.is_empty() {
                String::new()
            } else {
                format!(
                    " Showing {} of {unmatched}: {}",
                    examples.len(),
                    examples.join(", ")
                )
            };
            engine_warn!(
                "[corpus-index]{request} {unmatched} completed job(s) in the load window or without a fetch time could not be matched to an article file. {outside_window} completed job(s) outside the load window could not be matched to an article file. {already_claimed} completed job(s) resolved to an article file already matched to another job.{names}"
            );
        }
        selected
    }
}
