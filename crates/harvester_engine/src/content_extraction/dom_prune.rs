use scraper::ElementRef;

use crate::content_extraction::policy::DomPrunePolicy;

/// Returns true if the element should be removed per policy.
pub fn should_prune_element(element: &ElementRef<'_>, policy: &DomPrunePolicy) -> bool {
    let tag = element.value().name().to_ascii_lowercase();
    if policy.drop_tags.contains(tag.as_str()) {
        return true;
    }

    let class_tokens = attr_tokens(element, "class");
    let id_tokens = attr_tokens(element, "id");

    // Single pass: allow-list check first, then drop-list check.
    // Allow-list wins unconditionally — we continue scanning even after a drop match
    // to ensure a later allow token can still veto the drop.
    let mut should_drop = false;
    for token in class_tokens.iter().chain(id_tokens.iter()) {
        if policy.allow_attr_tokens.contains(token.as_str()) {
            return false; // allow-list wins
        }
        if policy.drop_attr_tokens.contains(token.as_str()) {
            should_drop = true; // continue checking allow-list
        }
    }
    should_drop
}

fn attr_tokens(element: &ElementRef<'_>, attr: &str) -> Vec<String> {
    element
        .value()
        .attr(attr)
        .unwrap_or("")
        .split_whitespace()
        .map(|t| t.to_lowercase())
        .collect()
}

#[cfg(test)]
mod tests {
    use scraper::Html;

    use super::*;

    fn parse_first_element(html: &str) -> Html {
        Html::parse_fragment(html)
    }

    fn first_element_ref(doc: &Html) -> ElementRef<'_> {
        doc.root_element()
            .first_child()
            .and_then(ElementRef::wrap)
            .expect("no element found")
    }

    #[test]
    fn nav_tag_is_pruned_by_default_policy() {
        let doc = parse_first_element("<nav>some nav</nav>");
        let el = first_element_ref(&doc);
        assert!(should_prune_element(&el, &DomPrunePolicy::default()));
    }

    #[test]
    fn article_tag_is_not_pruned() {
        let doc = parse_first_element("<article>some content</article>");
        let el = first_element_ref(&doc);
        assert!(!should_prune_element(&el, &DomPrunePolicy::default()));
    }

    #[test]
    fn element_with_newsletter_class_is_pruned() {
        let doc = parse_first_element(r#"<div class="newsletter">Sign up</div>"#);
        let el = first_element_ref(&doc);
        assert!(should_prune_element(&el, &DomPrunePolicy::default()));
    }

    #[test]
    fn element_with_comments_id_is_pruned() {
        let doc = parse_first_element(r#"<section id="comments">...</section>"#);
        let el = first_element_ref(&doc);
        assert!(should_prune_element(&el, &DomPrunePolicy::default()));
    }

    #[test]
    fn allow_attr_token_overrides_drop_token() {
        let mut policy = DomPrunePolicy::default();
        policy.allow_attr_tokens.insert("newsletter");

        let doc = parse_first_element(r#"<div class="newsletter">Sign up</div>"#);
        let el = first_element_ref(&doc);
        assert!(!should_prune_element(&el, &policy));
    }
}
