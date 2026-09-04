use std::path::{Path, PathBuf};

pub const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; font-src 'self'; img-src 'self' data:; connect-src 'self' ipc: http://ipc.localhost; object-src 'none'; base-uri 'self'; frame-ancestors 'none'; form-action 'none'";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedAsset {
    pub path: PathBuf,
    pub mime: &'static str,
}

pub fn resolve(dist_root: &Path, request_path: &str) -> Option<ResolvedAsset> {
    let normalized = request_path.replace('\\', "/");
    if normalized.starts_with("//") {
        return None;
    }
    let request = normalized.strip_prefix('/').unwrap_or(&normalized);
    if request.contains("..") || request.as_bytes().get(1) == Some(&b':') {
        return None;
    }
    let candidate = dist_root.join(request);
    let path = if candidate.is_file() {
        candidate
    } else if Path::new(&request).extension().is_none() {
        dist_root.join("index.html")
    } else {
        return None;
    };
    let canonical_root = dist_root.canonicalize().ok()?;
    let canonical_path = path.canonicalize().ok()?;
    if !canonical_path.starts_with(canonical_root) {
        return None;
    }
    Some(ResolvedAsset {
        mime: mime_for(&canonical_path),
        path: canonical_path,
    })
}

fn mime_for(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
    {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "txt" => "text/plain; charset=utf-8",
        "wasm" => "application/wasm",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn confines_paths_and_falls_back_to_spa_index() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("index.html"), "index").unwrap();
        std::fs::write(temp.path().join("app.js"), "js").unwrap();
        assert_eq!(
            resolve(temp.path(), "app.js").unwrap().mime,
            "text/javascript; charset=utf-8"
        );
        for path in ["/", "/app.js", "/deep/route"] {
            assert!(resolve(temp.path(), path).is_some());
        }
        for path in ["//server/share", "/../x", "C:/x"] {
            assert!(resolve(temp.path(), path).is_none());
        }
    }
}
