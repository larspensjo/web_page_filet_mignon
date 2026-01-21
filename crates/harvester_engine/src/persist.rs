use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use tempfile::NamedTempFile;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PersistError {
    #[error("output directory missing or not writable: {0}")]
    OutputDir(String),
    #[error("io error: {0}")]
    Io(#[from] io::Error),
}

/// Ensure output directory exists; create if missing.
pub fn ensure_output_dir(dir: &Path) -> Result<(), PersistError> {
    if dir.exists() {
        let meta = fs::metadata(dir).map_err(|e| PersistError::OutputDir(e.to_string()))?;
        if !meta.is_dir() {
            return Err(PersistError::OutputDir("path is not a directory".into()));
        }
    } else {
        fs::create_dir_all(dir).map_err(|e| PersistError::OutputDir(e.to_string()))?;
    }
    // Basic writability probe: try creating a temp file.
    NamedTempFile::new_in(dir).map_err(|e| PersistError::OutputDir(e.to_string()))?;
    Ok(())
}

/// Atomically write content to `{dir}/{filename}` by writing a temp file then renaming.
pub struct AtomicFileWriter {
    dir: PathBuf,
}

impl AtomicFileWriter {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub fn write(&self, filename: &str, content: &str) -> Result<PathBuf, PersistError> {
        ensure_output_dir(&self.dir)?;

        let target = self.dir.join(filename);
        let mut tmp = NamedTempFile::new_in(&self.dir)?;
        tmp.write_all(content.as_bytes())?;
        tmp.flush()?;
        tmp.as_file_mut().sync_all()?;

        // Replace existing file if present to keep determinism.
        if target.exists() {
            fs::remove_file(&target)?;
        }
        tmp.persist(&target)
            .map_err(|e| PersistError::Io(e.error))?;
        Ok(target)
    }
}
