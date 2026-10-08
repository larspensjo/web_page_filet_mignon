//! Extracted links, addressed solely by the canonical article identity hash.
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use engine_logging::{engine_info, engine_warn};
use harvester_engine::{archive_url_key, ExtractedLink};
use sha2::{Digest, Sha256};

// Startup/geometry recovery cannot remove a download's live atomic temporary.
static LINK_IO: Mutex<()> = Mutex::new(());

fn store_dir(output: &Path) -> Result<PathBuf, String> {
    let dir = output.join(".article_links");
    fs::create_dir_all(&dir).map_err(|e| format!("{}: create link store: {e}", dir.display()))?;
    let root = output.canonicalize().map_err(|e| e.to_string())?;
    let resolved = dir.canonicalize().map_err(|e| e.to_string())?;
    if resolved != root.join(".article_links") {
        return Err(format!(
            "{}: link store escapes output folder",
            dir.display()
        ));
    }
    Ok(resolved)
}

pub(crate) fn remove_partial_links(output: &Path) -> Result<(), String> {
    let _guard = LINK_IO.lock().map_err(|e| e.to_string())?;
    if !output.join(".article_links").exists() {
        return Ok(());
    }
    let dir = store_dir(output)?;
    for entry in
        fs::read_dir(&dir).map_err(|e| format!("{}: list link temporaries: {e}", dir.display()))?
    {
        let entry = entry.map_err(|e| e.to_string())?;
        if entry.file_name().to_string_lossy().starts_with(".partial-") {
            fs::remove_file(entry.path())
                .map_err(|e| format!("{}: remove link temporary: {e}", entry.path().display()))?;
        }
    }
    Ok(())
}

pub(crate) fn link_path(output: &Path, url: &str) -> Result<PathBuf, String> {
    let hash = Sha256::digest(archive_url_key(url).as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let path = store_dir(output)?.join(format!("{hash}.json"));
    // Refuse file symlinks as well as directory redirects. URL text never enters a path.
    if fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(format!("{}: redirected link file", path.display()));
    }
    Ok(path)
}

pub fn write_article_links(
    output: &Path,
    url: &str,
    links: &[ExtractedLink],
) -> Result<(), String> {
    if links.is_empty() {
        return Ok(());
    }
    let _guard = LINK_IO.lock().map_err(|e| e.to_string())?;
    let path = link_path(output, url)?;
    let content = serde_json::to_vec(links).map_err(|e| e.to_string())?;
    let mut temp = tempfile::Builder::new()
        .prefix(".partial-")
        .tempfile_in(path.parent().expect("link directory"))
        .map_err(|e| format!("{}: create link temporary: {e}", path.display()))?;
    temp.write_all(&content)
        .and_then(|_| temp.flush())
        .and_then(|_| temp.as_file().sync_all())
        .map_err(|e| format!("{}: flush link temporary: {e}", path.display()))?;
    // persist replaces atomically, including on Windows; do not unlink the target first.
    temp.persist(&path)
        .map_err(|e| format!("{}: publish links: {}", path.display(), e.error))?;
    Ok(())
}

pub(crate) fn store_observed(
    output: &Path,
    url: &str,
    links: &[ExtractedLink],
    observer: &Option<crate::FileWriteObserver>,
) {
    if links.is_empty() {
        return;
    }
    let started = std::time::Instant::now();
    engine_info!(
        "[article-links] operation=store url={} output={} links={} start",
        url,
        output.display(),
        links.len()
    );
    if let Err(error) = write_article_links(output, url, links) {
        engine_warn!(
            "[article-links] store url={} output={} error={}",
            url,
            output.display(),
            error
        );
    } else if let Some(observer) = observer {
        if let Ok(path) = link_path(output, url) {
            observer(
                &path,
                fs::metadata(&path).map(|m| m.len()).unwrap_or(0),
                started.elapsed(),
            );
        }
    }
    engine_info!(
        "[article-links] operation=store url={} elapsed_ms={}",
        url,
        started.elapsed().as_millis()
    );
}

pub fn load_article_links(output: &Path, url: &str) -> Vec<ExtractedLink> {
    match try_load_article_links(output, url) {
        Ok(links) => links,
        Err(e) => {
            engine_warn!(
                "[article-links] load url={} output={} error={}",
                url,
                output.display(),
                e
            );
            vec![]
        }
    }
}

pub(crate) fn try_load_article_links(
    output: &Path,
    url: &str,
) -> Result<Vec<ExtractedLink>, String> {
    let path = link_path(output, url)?;
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(format!("{}: read links: {e}", path.display())),
    };
    let records: Vec<serde_json::Value> = serde_json::from_slice(&bytes)
        .map_err(|e| format!("{}: parse links: {e}", path.display()))?;
    Ok(records
        .into_iter()
        .enumerate()
        .filter_map(
            |(index, record)| match serde_json::from_value::<ExtractedLink>(record) {
                Ok(link) => Some(link),
                Err(e) => {
                    engine_warn!(
                        "[article-links] skip record={} path={} url={} error={}",
                        index,
                        path.display(),
                        url,
                        e
                    );
                    None
                }
            },
        )
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_links_create_no_store_and_never_replace_existing_links() {
        let dir = tempfile::tempdir().unwrap();
        let url = "https://example.com/a";
        write_article_links(dir.path(), url, &[]).unwrap();
        assert!(!dir.path().join(".article_links").exists());
        let links = vec![ExtractedLink {
            url: "https://example.com/link".into(),
            text: Some("Kept".into()),
            kind: harvester_engine::LinkKind::Hyperlink,
        }];
        write_article_links(dir.path(), url, &links).unwrap();
        let path = link_path(dir.path(), url).unwrap();
        let bytes = fs::read(&path).unwrap();
        write_article_links(dir.path(), url, &[]).unwrap();
        assert_eq!(fs::read(path).unwrap(), bytes);
    }

    #[test]
    fn link_paths_use_only_canonical_hash_and_cannot_escape_output() {
        let dir = tempfile::tempdir().unwrap();
        for url in [
            "../../outside",
            "C:\\owner\\data",
            "/tmp/escape",
            "https://example.com/../../a?q=../b#x",
        ] {
            let path = link_path(dir.path(), url).unwrap();
            assert_eq!(
                path.parent().unwrap(),
                dir.path().canonicalize().unwrap().join(".article_links")
            );
            let filename = path.file_stem().unwrap().to_str().unwrap();
            assert_eq!(filename.len(), 64);
            assert!(filename.chars().all(|c| c.is_ascii_hexdigit()));
            write_article_links(dir.path(), url, &[]).unwrap();
        }
        assert_eq!(
            link_path(dir.path(), "https://EXAMPLE.com:443/a#one").unwrap(),
            link_path(dir.path(), "https://example.com/a#two").unwrap()
        );
    }

    #[test]
    fn malformed_file_or_individual_record_is_skipped_without_losing_valid_links() {
        let dir = tempfile::tempdir().unwrap();
        let url = "https://example.com/a";
        let path = link_path(dir.path(), url).unwrap();
        fs::write(&path, "{torn").unwrap();
        assert!(load_article_links(dir.path(), url).is_empty());
        fs::write(&path, r#"[{"url":"https://example.com/good","text":"Read","kind":"Hyperlink"},{"url":42},{"url":"https://example.com/bad","kind":"Unknown"}]"#).unwrap();
        let links = load_article_links(dir.path(), url);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].text.as_deref(), Some("Read"));
        write_article_links(dir.path(), url, &links).unwrap();
        assert_eq!(load_article_links(dir.path(), url), links);
        let state = dir.path().join(".harvester_state.ron");
        let old = r#"(completed: [(url: "https://example.com/legacy", tokens: None, bytes: None, links: [(url: 42), (url: "https://example.com/good", downloaded_path: None)])])"#;
        fs::write(&state, old).unwrap();
        crate::migrate_runtime_state(&state).unwrap();
        assert_eq!(crate::load_completed_jobs(&state).len(), 1);
        assert_eq!(
            load_article_links(dir.path(), "https://example.com/legacy")[0].url,
            "https://example.com/good"
        );
        assert_eq!(
            fs::read(dir.path().join(".harvester_state.pre-slim.ron")).unwrap(),
            old.as_bytes()
        );
    }

    #[test]
    fn redirected_link_directory_cannot_read_or_write_outside_output() {
        let dir = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        let redirect = dir.path().join(".article_links");
        #[cfg(windows)]
        assert!(std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(&redirect)
            .arg(external.path())
            .output()
            .unwrap()
            .status
            .success());
        #[cfg(unix)]
        std::os::unix::fs::symlink(external.path(), &redirect).unwrap();
        assert!(write_article_links(
            dir.path(),
            "https://example.com/a",
            &[ExtractedLink {
                url: "https://example.com/link".into(),
                text: None,
                kind: harvester_engine::LinkKind::Hyperlink,
            }]
        )
        .is_err());
        assert!(load_article_links(dir.path(), "https://example.com/a").is_empty());
        assert_eq!(fs::read_dir(external.path()).unwrap().count(), 0);
        #[cfg(windows)]
        fs::remove_dir(redirect).unwrap();
        #[cfg(unix)]
        fs::remove_file(redirect).unwrap();
    }
}
