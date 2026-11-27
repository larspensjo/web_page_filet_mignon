use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use tempfile::{NamedTempFile, TempPath};
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
    prepare_output_dir(dir)?;
    // Basic writability probe for callers that do not stage a file themselves.
    NamedTempFile::new_in(dir).map_err(|e| PersistError::OutputDir(e.to_string()))?;
    Ok(())
}

fn prepare_output_dir(dir: &Path) -> Result<(), PersistError> {
    if dir.exists() {
        let meta = fs::metadata(dir).map_err(|e| PersistError::OutputDir(e.to_string()))?;
        if !meta.is_dir() {
            return Err(PersistError::OutputDir("path is not a directory".into()));
        }
    } else {
        fs::create_dir_all(dir).map_err(|e| PersistError::OutputDir(e.to_string()))?;
    }
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

    pub fn write(
        &self,
        filename: impl AsRef<Path>,
        content: &str,
    ) -> Result<PathBuf, PersistError> {
        self.write_with(filename, content, replace_atomic_file)
    }

    fn write_with(
        &self,
        filename: impl AsRef<Path>,
        content: &str,
        replace: impl FnOnce(NamedTempFile, &Path) -> io::Result<()>,
    ) -> Result<PathBuf, PersistError> {
        prepare_output_dir(&self.dir)?;

        let target = self.dir.join(filename);
        let mut tmp = stage_atomic_file(&target).map_err(|error| {
            if target.parent() == Some(self.dir.as_path()) {
                PersistError::OutputDir(error.to_string())
            } else {
                PersistError::Io(error)
            }
        })?;
        tmp.write_all(content.as_bytes())?;
        tmp.flush()?;
        tmp.as_file_mut().sync_all()?;
        replace(tmp, &target)?;
        Ok(target)
    }
}

pub fn atomic_temp_prefix(filename: &OsStr) -> OsString {
    let mut prefix = OsString::from(".");
    prefix.push(filename);
    prefix.push(".tmp-");
    prefix
}

/// Stage a replacement beside its target so publication uses a same-directory rename.
pub fn stage_atomic_file(target: &Path) -> io::Result<NamedTempFile> {
    let filename = target.file_name().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "target must have a filename")
    })?;
    let parent = target
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    tempfile::Builder::new()
        .prefix(&atomic_temp_prefix(filename))
        .tempfile_in(parent)
}

/// Install a staged file with a replacing rename, retrying briefly on permission errors.
pub fn replace_atomic_file(temp: NamedTempFile, target: &Path) -> io::Result<()> {
    const MAX_RETRIES: usize = 4;
    const RETRY_DELAY: Duration = Duration::from_millis(50);

    // keep() clears the Windows temporary attribute by path before the rename.
    // Re-arm cleanup on the absolute staging path until the rename succeeds.
    let mut staged = TempPath::try_from_path(temp.into_temp_path().keep()?)?;
    let mut retries = MAX_RETRIES;
    loop {
        match fs::rename(&staged, target) {
            Ok(()) => break,
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied && retries > 0 => {
                retries -= 1;
                thread::sleep(RETRY_DELAY);
            }
            Err(error) => return Err(error),
        }
    }
    staged.disable_cleanup(true);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_target_is_readable_at_rename_and_staging_is_cleaned_up() {
        let directory = tempfile::tempdir().unwrap();
        let writer = AtomicFileWriter::new(directory.path().to_path_buf());
        let target = directory.path().join("archive.md");
        fs::write(&target, b"old complete archive").unwrap();

        for fail in [true, false] {
            let result = writer.write_with("archive.md", "new complete archive", |temp, path| {
                assert_eq!(path, target);
                assert_eq!(fs::read(path)?, b"old complete archive");
                assert_eq!(temp.path().parent(), target.parent());
                assert!(temp
                    .path()
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(".archive.md.tmp-"));
                assert_eq!(fs::read(temp.path())?, b"new complete archive");
                if fail {
                    return Err(io::Error::other("injected rename failure"));
                }
                replace_atomic_file(temp, path)
            });
            assert_eq!(result.is_err(), fail);
            assert_eq!(
                fs::read(&target).unwrap(),
                if fail {
                    b"old complete archive"
                } else {
                    b"new complete archive"
                }
            );
            assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
        }
    }

    #[cfg(windows)]
    #[test]
    fn replaces_target_open_for_read_and_clears_temporary_attribute() {
        use std::io::Read;
        use std::os::windows::fs::MetadataExt;

        const FILE_ATTRIBUTE_TEMPORARY: u32 = 0x100;
        let directory = tempfile::tempdir().unwrap();
        let writer = AtomicFileWriter::new(directory.path().to_path_buf());
        let target = directory.path().join("archive.md");
        fs::write(&target, b"old archive").unwrap();
        let mut reader = fs::File::open(&target).unwrap();

        assert_eq!(writer.write("archive.md", "new archive").unwrap(), target);
        assert_eq!(fs::read(&target).unwrap(), b"new archive");
        assert_eq!(
            fs::metadata(&target).unwrap().file_attributes() & FILE_ATTRIBUTE_TEMPORARY,
            0
        );
        let mut old = Vec::new();
        reader.read_to_end(&mut old).unwrap();
        assert_eq!(old, b"old archive");
    }

    #[cfg(windows)]
    #[test]
    fn removes_staging_file_after_retries_when_reader_denies_delete_sharing() {
        use std::os::windows::fs::OpenOptionsExt;
        use std::time::Instant;

        let directory = tempfile::tempdir().unwrap();
        let writer = AtomicFileWriter::new(directory.path().to_path_buf());
        let target = directory.path().join("archive.md");
        fs::write(&target, b"old archive").unwrap();
        let reader = fs::File::options()
            .read(true)
            .share_mode(1 | 2) // FILE_SHARE_READ | FILE_SHARE_WRITE
            .open(&target)
            .unwrap();

        let start = Instant::now();
        assert!(matches!(
            writer.write("archive.md", "new archive"),
            Err(PersistError::Io(error)) if error.kind() == io::ErrorKind::PermissionDenied
        ));
        assert!(start.elapsed() >= Duration::from_millis(200));
        assert_eq!(fs::read(&target).unwrap(), b"old archive");
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
        drop(reader);
    }
}
