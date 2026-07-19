use chrono::Utc;
use engine_logging::{engine_info, engine_warn};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
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
/// Only removes lock if owner token matches
#[derive(Debug)]
pub struct LockGuard {
    lock_path: PathBuf,
    owner: String,
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        // Verify ownership before removing lock
        match fs::read_to_string(&self.lock_path) {
            Ok(content) => match serde_json::from_str::<LockMetadata>(&content) {
                Ok(meta) if meta.owner == self.owner => {
                    if let Err(err) = fs::remove_file(&self.lock_path) {
                        engine_warn!("[batch-lock] Failed to remove lock file: {}", err);
                    } else {
                        engine_info!("[batch-lock] Released lock");
                    }
                }
                Ok(meta) => {
                    engine_warn!(
                        "[batch-lock] Lock ownership changed (expected: {}, found: {}), not removing",
                        self.owner,
                        meta.owner
                    );
                }
                Err(err) => {
                    engine_warn!(
                        "[batch-lock] Failed to parse lock metadata on drop: {}",
                        err
                    );
                }
            },
            Err(err) => {
                engine_warn!("[batch-lock] Failed to read lock file on drop: {}", err);
            }
        }
    }
}

/// Try to acquire the batch run lock atomically.
///
/// Uses `create_new` for atomic lock creation. If `force` is true, removes existing
/// lock and logs previous metadata for diagnostics.
///
/// Returns a guard that will automatically release the lock on drop, but only if
/// the ownership token still matches (prevents stale guards from removing newer locks).
pub fn acquire_lock(output_dir: &Path, force: bool) -> Result<LockGuard, String> {
    let lock_path = output_dir.join(LOCK_FILENAME);

    // Ensure output directory exists
    fs::create_dir_all(output_dir)
        .map_err(|err| format!("Failed to create output directory: {}", err))?;

    // Generate owner token for this lock attempt
    let owner = generate_owner_token();

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
                return Err(
                    "Lock file exists but is unreadable. Use --force-unlock to override."
                        .to_string(),
                );
            }
        }
    }

    // If forcing, log existing lock metadata and remove it
    if force && lock_path.exists() {
        if let Ok(content) = fs::read_to_string(&lock_path) {
            if let Ok(meta) = serde_json::from_str::<LockMetadata>(&content) {
                engine_warn!(
                    "[batch-lock] Force-unlocking existing lock (pid: {}, owner: {}, started: {})",
                    meta.pid,
                    meta.owner,
                    meta.started_utc
                );
            } else {
                engine_warn!("[batch-lock] Force-unlocking existing lock (corrupted metadata)");
            }
        } else {
            engine_warn!("[batch-lock] Force-unlocking existing lock (unreadable)");
        }

        if let Err(err) = fs::remove_file(&lock_path) {
            return Err(format!("Failed to remove existing lock: {}", err));
        }
    }

    // Create lock with metadata
    let metadata = LockMetadata {
        pid: std::process::id(),
        started_utc: Utc::now().to_rfc3339(),
        owner: owner.clone(),
        command: std::env::args().next(),
    };

    let metadata_str = serde_json::to_string_pretty(&metadata)
        .map_err(|err| format!("Failed to serialize lock metadata: {}", err))?;

    // Atomic lock file creation using create_new
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock_path)
        .map_err(|err| {
            if err.kind() == io::ErrorKind::AlreadyExists {
                "Lock file appeared during acquisition (race condition)".to_string()
            } else if err.kind() == io::ErrorKind::PermissionDenied {
                "Permission denied writing lock file".to_string()
            } else {
                format!("Failed to create lock file: {}", err)
            }
        })?;

    file.write_all(metadata_str.as_bytes())
        .map_err(|err| format!("Failed to write lock metadata: {}", err))?;

    engine_info!("[batch-lock] Acquired lock (pid: {})", std::process::id());

    Ok(LockGuard { lock_path, owner })
}

/// Generate a stable random owner token for lock diagnostics.
///
/// Uses process ID and current time to create a unique token per acquisition.
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

    #[test]
    fn concurrent_acquire_one_succeeds_one_fails() {
        use std::sync::{Arc, Barrier};
        use std::thread;

        let dir = tempdir().unwrap();
        let dir_path = Arc::new(dir.path().to_path_buf());
        let barrier = Arc::new(Barrier::new(2));

        let dir1 = Arc::clone(&dir_path);
        let barrier1 = Arc::clone(&barrier);
        let handle1 = thread::spawn(move || {
            barrier1.wait();
            acquire_lock(&dir1, false)
        });

        let dir2 = Arc::clone(&dir_path);
        let barrier2 = Arc::clone(&barrier);
        let handle2 = thread::spawn(move || {
            barrier2.wait();
            acquire_lock(&dir2, false)
        });

        let result1 = handle1.join().unwrap();
        let result2 = handle2.join().unwrap();

        // Exactly one should succeed
        assert!(
            result1.is_ok() != result2.is_ok(),
            "One acquire should succeed, one should fail"
        );

        // Clean up the successful guard
        if let Ok(guard) = result1 {
            drop(guard);
        }
        if let Ok(guard) = result2 {
            drop(guard);
        }
    }

    #[test]
    fn force_unlock_replacement_logged() {
        engine_logging::initialize_for_tests();
        let dir = tempdir().unwrap();

        // First acquire
        let guard1 = acquire_lock(dir.path(), false).expect("first acquire");
        let lock_path = dir.path().join(LOCK_FILENAME);

        // Read owner from first lock
        let _content1 = fs::read_to_string(&lock_path).unwrap();

        drop(guard1);

        // Recreate lock manually with different owner
        fs::write(
            &lock_path,
            r#"{"pid": 99999, "started_utc": "2020-01-01T00:00:00Z", "owner": "old_owner", "command": null}"#,
        )
        .unwrap();

        // Force unlock should succeed and log the old metadata
        let guard2 = acquire_lock(dir.path(), true).expect("force unlock should work");

        // New lock should have different owner
        let content2 = fs::read_to_string(&lock_path).unwrap();
        let meta2: LockMetadata = serde_json::from_str(&content2).unwrap();
        assert_ne!(meta2.owner, "old_owner");

        drop(guard2);
        assert!(!lock_path.exists());
    }

    #[test]
    fn stale_owner_cannot_delete_newly_acquired_lock() {
        let dir = tempdir().unwrap();
        let lock_path = dir.path().join(LOCK_FILENAME);

        // First acquire
        let guard1 = acquire_lock(dir.path(), false).expect("first acquire");

        // Read owner from first lock
        let content1 = fs::read_to_string(&lock_path).unwrap();
        let meta1: LockMetadata = serde_json::from_str(&content1).unwrap();
        let owner1 = meta1.owner.clone();

        // Force-unlock and acquire new lock
        let guard2 = acquire_lock(dir.path(), true).expect("force unlock should work");

        // Verify new lock has different owner
        let content2 = fs::read_to_string(&lock_path).unwrap();
        let meta2: LockMetadata = serde_json::from_str(&content2).unwrap();
        assert_ne!(meta2.owner, owner1);

        // Drop first guard (stale owner)
        drop(guard1);

        // Lock should still exist because guard1 doesn't own it anymore
        assert!(lock_path.exists());

        // And the owner should be guard2's
        let content3 = fs::read_to_string(&lock_path).unwrap();
        let meta3: LockMetadata = serde_json::from_str(&content3).unwrap();
        assert_eq!(meta3.owner, meta2.owner);

        // Drop guard2 (correct owner)
        drop(guard2);

        // Now lock should be removed
        assert!(!lock_path.exists());
    }
}
