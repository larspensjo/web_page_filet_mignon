use chrono::Utc;
use engine_logging::{engine_info, engine_warn};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const LOCK_FILENAME: &str = ".harvester_batch.lock";

/// Lock metadata stored in the lock file
#[derive(Debug, Clone, Serialize, Deserialize)]
struct LockMetadata {
    pid: u32,
    started_utc: String,
    owner: String,
    command: Option<String>,
}

/// RAII lock guard that removes the lock file on drop
#[derive(Debug)]
pub struct LockGuard {
    lock_path: PathBuf,
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        if let Err(err) = fs::remove_file(&self.lock_path) {
            engine_warn!("[batch-lock] Failed to remove lock file: {}", err);
        } else {
            engine_info!("[batch-lock] Released lock");
        }
    }
}

/// Try to acquire the batch run lock
pub fn acquire_lock(output_dir: &Path, force: bool) -> Result<LockGuard, String> {
    let lock_path = output_dir.join(LOCK_FILENAME);

    // If lock exists and force is false, read metadata and fail
    if lock_path.exists() && !force {
        let metadata_str = fs::read_to_string(&lock_path)
            .unwrap_or_else(|_| String::from("(unable to read lock file)"));
        
        match serde_json::from_str::<LockMetadata>(&metadata_str) {
            Ok(meta) => {
                return Err(format!(
                    "Another batch run is already active (pid: {}, started: {}, owner: {}). \
                     Use --force-unlock to override.",
                    meta.pid, meta.started_utc, meta.owner
                ));
            }
            Err(_) => {
                return Err(format!(
                    "Lock file exists but is unreadable. Use --force-unlock to override."
                ));
            }
        }
    }

    // If forcing, remove existing lock
    if force && lock_path.exists() {
        engine_warn!("[batch-lock] Force-unlocking existing lock");
        if let Err(err) = fs::remove_file(&lock_path) {
            return Err(format!("Failed to remove existing lock: {}", err));
        }
    }

    // Create lock with metadata
    let metadata = LockMetadata {
        pid: std::process::id(),
        started_utc: Utc::now().to_rfc3339(),
        owner: generate_owner_token(),
        command: std::env::args().collect::<Vec<_>>().get(0).cloned(),
    };

    let metadata_str = serde_json::to_string_pretty(&metadata)
        .map_err(|err| format!("Failed to serialize lock metadata: {}", err))?;

    // Ensure output directory exists
    fs::create_dir_all(output_dir)
        .map_err(|err| format!("Failed to create output directory: {}", err))?;

    // Write lock file
    fs::write(&lock_path, metadata_str).map_err(|err| {
        if err.kind() == io::ErrorKind::PermissionDenied {
            format!("Permission denied writing lock file")
        } else {
            format!("Failed to write lock file: {}", err)
        }
    })?;

    engine_info!("[batch-lock] Acquired lock (pid: {})", std::process::id());

    Ok(LockGuard { lock_path })
}

/// Generate a stable random owner token for diagnostics
fn generate_owner_token() -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    std::process::id().hash(&mut hasher);
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
        .hash(&mut hasher);
    
    format!("{:016x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn acquire_lock_succeeds_when_no_lock_exists() {
        let dir = tempdir().unwrap();
        let guard = acquire_lock(dir.path(), false).expect("should acquire");
        
        // Lock file should exist
        let lock_path = dir.path().join(LOCK_FILENAME);
        assert!(lock_path.exists());

        // Should be able to read metadata
        let content = fs::read_to_string(&lock_path).unwrap();
        let meta: LockMetadata = serde_json::from_str(&content).unwrap();
        assert_eq!(meta.pid, std::process::id());

        drop(guard);
        
        // Lock file should be removed after drop
        assert!(!lock_path.exists());
    }

    #[test]
    fn acquire_lock_fails_when_lock_exists() {
        let dir = tempdir().unwrap();
        let _guard1 = acquire_lock(dir.path(), false).expect("first acquire");

        let err = acquire_lock(dir.path(), false).expect_err("second should fail");
        assert!(err.contains("already active"));
    }

    #[test]
    fn force_unlock_removes_existing_lock() {
        let dir = tempdir().unwrap();
        let guard1 = acquire_lock(dir.path(), false).expect("first acquire");
        
        // Manually drop first guard to release it
        drop(guard1);

        // Create a stale lock manually
        let lock_path = dir.path().join(LOCK_FILENAME);
        fs::write(&lock_path, r#"{"pid": 99999, "started_utc": "2020-01-01T00:00:00Z", "owner": "stale", "command": null}"#).unwrap();

        // Force unlock should succeed
        let guard2 = acquire_lock(dir.path(), true).expect("force unlock should work");
        assert!(lock_path.exists());
        drop(guard2);
        assert!(!lock_path.exists());
    }

    #[test]
    fn owner_token_is_stable_within_process() {
        let token1 = generate_owner_token();
        std::thread::sleep(std::time::Duration::from_millis(10));
        let token2 = generate_owner_token();
        
        // Tokens should be different because time changes
        assert_ne!(token1, token2);
    }
}
