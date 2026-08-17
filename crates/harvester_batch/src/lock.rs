use chrono::Utc;
use engine_logging::{engine_info, engine_warn};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
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

/// RAII lock guard holding the lock file open for the lifetime of the run.
///
/// The held handle, not the file's existence, is what excludes a second run.
/// On Windows the file is opened delete-on-close with a share mode that denies
/// write access, so the operating system refuses a concurrent acquisition and
/// releases the lock whenever this process ends — including the terminations
/// that never run `Drop`, such as the second Ctrl-C's immediate exit.
#[derive(Debug)]
pub struct LockGuard {
    lock_path: PathBuf,
    owner: String,
    file: Option<File>,
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        // Closing the handle is the release. Where the platform deletes on
        // close, the file is gone once this returns.
        drop(self.file.take());

        if !self.lock_path.exists() {
            engine_info!("[batch-lock] Released lock");
            return;
        }

        // Platforms without delete-on-close still need the ownership-checked
        // removal, which also protects a file that outlived its handle.
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

/// Try to acquire the batch run lock.
///
/// Exclusion comes from holding the lock file open, not from the file existing:
/// the open fails while another run holds the handle, and succeeds once that
/// run's process has ended by any means. A lock file that outlived its owner —
/// after a hard exit, a kill, or a crash — is therefore reclaimed rather than
/// blocking every later run until someone deletes it by hand.
///
/// If `force` is true, removes the existing lock and logs previous metadata for
/// diagnostics, which overrides even a genuinely live holder.
///
/// Returns a guard that releases the lock by closing the handle, and that
/// removes a surviving lock file only if the ownership token still matches
/// (prevents stale guards from removing newer locks).
pub fn acquire_lock(output_dir: &Path, force: bool) -> Result<LockGuard, String> {
    let lock_path = output_dir.join(LOCK_FILENAME);

    // Ensure output directory exists
    fs::create_dir_all(output_dir)
        .map_err(|err| format!("Failed to create output directory: {}", err))?;

    // Generate owner token for this lock attempt
    let owner = generate_owner_token();

    // Read before opening: acquisition truncates the file, and the reclaim log
    // needs the departed run's identity.
    let previous = read_lock_metadata(&lock_path);

    // If forcing, log existing lock metadata and remove it
    if force && lock_path.exists() {
        match &previous {
            Some(meta) => engine_warn!(
                "[batch-lock] Force-unlocking existing lock (pid: {}, owner: {}, started: {})",
                meta.pid,
                meta.owner,
                meta.started_utc
            ),
            None => {
                engine_warn!("[batch-lock] Force-unlocking existing lock (corrupted or unreadable)")
            }
        }

        if let Err(err) = fs::remove_file(&lock_path) {
            return Err(format!("Failed to remove existing lock: {}", err));
        }
    }

    // A lock file still present here belongs to a run that is no longer
    // holding it; acquiring the handle below is what proves that.
    let reclaiming = lock_path.exists();

    let mut file = match open_lock_handle(&lock_path) {
        Ok(file) => file,
        Err(err) if is_lock_held(&err) => return Err(describe_active_lock(&lock_path)),
        Err(err) if err.kind() == io::ErrorKind::PermissionDenied => {
            return Err("Permission denied writing lock file".to_string());
        }
        Err(err) => return Err(format!("Failed to create lock file: {}", err)),
    };

    if reclaiming {
        match &previous {
            Some(meta) => engine_warn!(
                "[batch-lock] Reclaimed lock left by a run that is no longer holding it \
                 (pid: {}, owner: {}, started: {})",
                meta.pid,
                meta.owner,
                meta.started_utc
            ),
            None => engine_warn!(
                "[batch-lock] Reclaimed lock file with corrupted or unreadable metadata"
            ),
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

    file.write_all(metadata_str.as_bytes())
        .map_err(|err| format!("Failed to write lock metadata: {}", err))?;
    file.flush()
        .map_err(|err| format!("Failed to flush lock metadata: {}", err))?;

    engine_info!("[batch-lock] Acquired lock (pid: {})", std::process::id());

    Ok(LockGuard {
        lock_path,
        owner,
        file: Some(file),
    })
}

/// Read the lock file's metadata, if it exists and parses.
fn read_lock_metadata(lock_path: &Path) -> Option<LockMetadata> {
    let content = fs::read_to_string(lock_path).ok()?;
    serde_json::from_str::<LockMetadata>(&content).ok()
}

/// Build the diagnostic for a lock another run is actively holding.
///
/// Re-reads the metadata so a holder that had not finished writing it when this
/// acquisition began is still reported by identity where possible.
fn describe_active_lock(lock_path: &Path) -> String {
    match read_lock_metadata(lock_path) {
        Some(meta) => format!(
            "Another batch run is already active (pid: {}, started: {}, owner: {}). \
             Use --force-unlock to override.",
            meta.pid, meta.started_utc, meta.owner
        ),
        None => "Lock file exists but is unreadable. Use --force-unlock to override.".to_string(),
    }
}

/// Open the lock file so that the operating system owns the exclusion.
///
/// The share mode denies write access to every other handle, so a second
/// acquisition fails while this one is open, and `FILE_FLAG_DELETE_ON_CLOSE`
/// makes Windows remove the file when the handle closes — which it does at
/// process termination even when no destructor runs.
#[cfg(windows)]
fn open_lock_handle(lock_path: &Path) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;

    const FILE_SHARE_READ: u32 = 0x0000_0001;
    const FILE_SHARE_DELETE: u32 = 0x0000_0004;
    const FILE_FLAG_DELETE_ON_CLOSE: u32 = 0x0400_0000;

    OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_DELETE_ON_CLOSE)
        .open(lock_path)
}

/// Non-Windows fallback: exclusion is the atomic create, as before.
///
/// Without a delete-on-close equivalent, a lock file that outlives its owner
/// still blocks until `--force-unlock` clears it.
#[cfg(not(windows))]
fn open_lock_handle(lock_path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(lock_path)
}

/// Does this open failure mean another run is holding the lock?
#[cfg(windows)]
fn is_lock_held(err: &io::Error) -> bool {
    const ERROR_SHARING_VIOLATION: i32 = 32;
    err.raw_os_error() == Some(ERROR_SHARING_VIOLATION)
}

#[cfg(not(windows))]
fn is_lock_held(err: &io::Error) -> bool {
    err.kind() == io::ErrorKind::AlreadyExists
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

    /// Regression: a lock file left behind by a run that died without releasing
    /// it (a second Ctrl-C's immediate exit, a kill, a crash) blocked every
    /// later run, because acquisition only asked whether the file existed.
    #[test]
    fn lock_left_by_a_departed_run_is_reclaimed_without_force() {
        engine_logging::initialize_for_tests();
        let dir = tempdir().unwrap();
        let lock_path = dir.path().join(LOCK_FILENAME);

        // No handle is open on this file: its owner is gone.
        fs::write(
            &lock_path,
            r#"{"pid": 99999, "started_utc": "2020-01-01T00:00:00Z", "owner": "departed", "command": null}"#,
        )
        .unwrap();

        let guard = acquire_lock(dir.path(), false).expect("stale lock should be reclaimed");

        let meta = read_lock_metadata(&lock_path).expect("metadata should be rewritten");
        assert_eq!(meta.pid, std::process::id());
        assert_ne!(meta.owner, "departed");

        drop(guard);
        assert!(!lock_path.exists());
    }

    /// The reclaim path must not weaken exclusion: a lock whose handle is still
    /// open is refused, and the diagnostic still identifies the holder.
    #[test]
    fn lock_held_by_a_live_run_is_still_refused_with_holder_identity() {
        let dir = tempdir().unwrap();
        let guard = acquire_lock(dir.path(), false).expect("first acquire");

        let err = acquire_lock(dir.path(), false).expect_err("second should fail");
        assert!(
            err.contains("already active"),
            "unexpected message: {}",
            err
        );
        assert!(
            err.contains(&std::process::id().to_string()),
            "holder pid should be named: {}",
            err
        );

        drop(guard);
    }

    /// The operating system, not `Drop`, is what releases the lock: closing the
    /// handle removes the file even when no guard runs. This is the same
    /// mechanism that fires when the process is terminated outright.
    #[cfg(windows)]
    #[test]
    fn closing_the_handle_removes_the_lock_file_without_a_guard() {
        let dir = tempdir().unwrap();
        let lock_path = dir.path().join(LOCK_FILENAME);

        let file = open_lock_handle(&lock_path).expect("open lock handle");
        assert!(lock_path.exists());

        drop(file);
        assert!(
            !lock_path.exists(),
            "closing the handle should remove the lock file"
        );
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
