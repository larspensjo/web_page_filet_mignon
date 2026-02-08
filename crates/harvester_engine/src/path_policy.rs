use std::path::Path;

/// Determines whether `candidate` resolves to a location strictly inside `root`.
pub fn is_confined_to(candidate: &Path, root: &Path) -> bool {
    let resolved_root = match root.canonicalize() {
        Ok(canonical) => canonical,
        Err(_) => return false,
    };

    let resolved_candidate = match root.join(candidate).canonicalize() {
        Ok(canonical) => canonical,
        Err(_) => return false,
    };

    resolved_candidate.starts_with(resolved_root)
}
