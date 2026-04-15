use std::path::PathBuf;

fn rotated_log_path(log_path: &std::path::Path, index: usize) -> PathBuf {
    let file_name = log_path
        .file_name()
        .unwrap_or_else(|| std::ffi::OsStr::new("mcp.log"));
    let mut rotated_name = file_name.to_os_string();
    rotated_name.push(format!(".{index}"));
    log_path.with_file_name(rotated_name)
}

pub(crate) fn rotate_log_files(
    log_path: &std::path::Path,
    retain_runs: usize,
) -> std::io::Result<Vec<String>> {
    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut actions = Vec::new();
    if retain_runs == 0 {
        if log_path.exists() {
            std::fs::remove_file(log_path)?;
            actions.push(format!("removed previous log {:?}", log_path));
        }
        return Ok(actions);
    }

    let oldest_archive = rotated_log_path(log_path, retain_runs);
    if oldest_archive.exists() {
        std::fs::remove_file(&oldest_archive)?;
        actions.push(format!("removed oldest archive {:?}", oldest_archive));
    }

    for index in (1..retain_runs).rev() {
        let source = rotated_log_path(log_path, index);
        if !source.exists() {
            continue;
        }
        let target = rotated_log_path(log_path, index + 1);
        std::fs::rename(&source, &target)?;
        actions.push(format!("rotated {:?} -> {:?}", source, target));
    }

    if log_path.exists() {
        let target = rotated_log_path(log_path, 1);
        std::fs::rename(log_path, &target)?;
        actions.push(format!("rotated {:?} -> {:?}", log_path, target));
    }

    Ok(actions)
}
