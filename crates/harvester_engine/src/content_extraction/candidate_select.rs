use scraper::{ElementRef, Html, Selector};

use crate::content_extraction::diagnostics::CandidateKind;
use crate::content_extraction::policy::CandidatePolicy;

pub struct CandidateInfo {
    pub kind: CandidateKind,
    pub score: f64,
}

type CandidateSpec = (&'static str, fn(&str) -> CandidateKind, f64);

/// Score and select the best article container from the document.
pub fn select_candidate<'a>(
    doc: &'a Html,
    policy: &CandidatePolicy,
) -> (ElementRef<'a>, CandidateInfo) {
    // These compile once per extraction; cost is negligible vs. DOM traversal
    let para_sel = Selector::parse("p").expect("valid selector");
    let a_sel = Selector::parse("a").expect("valid selector");

    let named_candidates: &[CandidateSpec] = &[
        ("article", |_| CandidateKind::Article, 2.0),
        ("main", |_| CandidateKind::Main, 1.8),
        ("[role=main]", |_| CandidateKind::RoleMain, 1.8),
        (
            ".article-body",
            |s| CandidateKind::ContentClass(s.into()),
            1.3,
        ),
        (
            ".entry-content",
            |s| CandidateKind::ContentClass(s.into()),
            1.3,
        ),
        (
            ".post-content",
            |s| CandidateKind::ContentClass(s.into()),
            1.3,
        ),
        (
            ".story-body",
            |s| CandidateKind::ContentClass(s.into()),
            1.3,
        ),
        (
            ".article-content",
            |s| CandidateKind::ContentClass(s.into()),
            1.3,
        ),
        (
            ".article__body",
            |s| CandidateKind::ContentClass(s.into()),
            1.3,
        ),
        (".post-body", |s| CandidateKind::ContentClass(s.into()), 1.3),
    ];

    let mut best_element: Option<ElementRef<'a>> = None;
    let mut best_info: Option<CandidateInfo> = None;
    let mut evaluated = 0usize;

    'outer: for (selector_str, kind_fn, semantic_bonus) in named_candidates {
        let sel = match Selector::parse(selector_str) {
            Ok(s) => s,
            Err(_) => continue,
        };
        for element in doc.select(&sel) {
            if evaluated >= policy.max_candidates {
                break 'outer;
            }
            evaluated += 1;

            let text_chars = count_text_chars(element);
            if text_chars < policy.min_text_chars {
                continue;
            }

            let para_count = count_para_elements(element, &para_sel);
            let link_text = count_link_text_chars(element, &a_sel);
            let link_density = if text_chars > 0 {
                link_text as f64 / text_chars as f64
            } else {
                0.0
            };

            let text_score = text_chars as f64;
            let para_bonus = 1.0 + (para_count as f64).sqrt() * 0.5;
            let link_penalty = if link_density > policy.max_link_density {
                0.3
            } else {
                1.0
            };
            // semantic_bonus prefers specific containers (article, main) over generic ones (body)
            let score = text_score * para_bonus * link_penalty * semantic_bonus;

            let is_better = best_info
                .as_ref()
                .is_none_or(|b: &CandidateInfo| score > b.score);
            if is_better {
                best_element = Some(element);
                best_info = Some(CandidateInfo {
                    kind: kind_fn(selector_str),
                    score,
                });
            }
        }
    }

    if let (Some(element), Some(info)) = (best_element, best_info) {
        return (element, info);
    }

    let body_sel = Selector::parse("body").expect("valid selector");
    // Fall back to body, or the virtual root if the document has no <body> element.
    // This is the ultimate fallback — score 0.0 signals no meaningful candidate was found.
    if let Some(body) = doc.select(&body_sel).next() {
        return (
            body,
            CandidateInfo {
                kind: CandidateKind::Body,
                score: 0.0,
            },
        );
    }
    (
        doc.root_element(),
        CandidateInfo {
            kind: CandidateKind::Body,
            score: 0.0,
        },
    )
}

fn count_text_chars(element: ElementRef<'_>) -> usize {
    element.text().map(|t| t.len()).sum()
}

fn count_para_elements(element: ElementRef<'_>, para_sel: &Selector) -> usize {
    element.select(para_sel).count()
}

fn count_link_text_chars(element: ElementRef<'_>, a_sel: &Selector) -> usize {
    element
        .select(a_sel)
        .map(|a| a.text().map(|t| t.len()).sum::<usize>())
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> CandidatePolicy {
        CandidatePolicy::default()
    }

    fn long_text(chars: usize) -> String {
        "x".repeat(chars)
    }

    #[test]
    fn article_with_long_text_beats_body_with_less() {
        let text = long_text(500);
        let html = format!(
            "<html><body><p>{}</p><article><p>{}</p></article></body></html>",
            long_text(80),
            text
        );
        let doc = Html::parse_document(&html);
        let (_, info) = select_candidate(&doc, &policy());
        assert!(matches!(info.kind, CandidateKind::Article));
        assert!(info.score > 0.0);
    }

    #[test]
    fn high_link_density_element_gets_penalized() {
        let link_text = r##"<a href="#">link</a> "##.repeat(60);
        let plain_text = "Some real content here. ".repeat(5);
        let html =
            format!("<html><body><article><p>{plain_text}</p>{link_text}</article></body></html>");
        let doc = Html::parse_document(&html);
        let (_, info) = select_candidate(&doc, &policy());
        // Article is selected but the score is penalized
        assert!(matches!(
            info.kind,
            CandidateKind::Article | CandidateKind::Body
        ));
        // With link_penalty=0.3, para_bonus≈1.0 (0 paragraphs gives 1.0), semantic_bonus=2.0,
        // the penalized score should be roughly 0.3 * 2.0 * total_chars = 0.6 * total_chars,
        // which is well below total_chars itself.
        let total_chars = count_text_chars(
            doc.select(&Selector::parse("article").unwrap())
                .next()
                .unwrap(),
        );
        assert!(info.score < total_chars as f64);
    }

    #[test]
    fn body_fallback_when_no_good_candidates() {
        // Very short text - below min_text_chars
        let html = "<html><body><article><p>Short.</p></article></body></html>";
        let doc = Html::parse_document(html);
        let (_, info) = select_candidate(&doc, &policy());
        assert!(matches!(info.kind, CandidateKind::Body));
        assert_eq!(info.score, 0.0);
    }

    #[test]
    fn main_preferred_when_scores_are_similar() {
        let text = long_text(300);
        // Both main and a content class, main should win due to selector priority
        let html = format!(
            r#"<html><body><main><p>{text}</p></main><div class="article-body"><p>{text}</p></div></body></html>"#
        );
        let doc = Html::parse_document(&html);
        let (_, info) = select_candidate(&doc, &policy());
        // main comes before .article-body in selector priority, so when scores are equal, first wins
        assert!(matches!(info.kind, CandidateKind::Main));
    }
}
