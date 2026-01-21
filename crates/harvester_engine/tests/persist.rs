use std::fs;
use harvester_engine::{ensure_output_dir, AtomicFileWriter};
use tempfile::TempDir;

#[test]
fn creates_missing_output_dir() {
    let temp = TempDir::new().unwrap();
    let new_dir = temp.path().join("out");
    assert!(!new_dir.exists());
    ensure_output_dir(&new_dir).unwrap();
    assert!(new_dir.is_dir());
}

#[test]
fn atomic_write_replaces_existing_and_is_atomic() {
    let temp = TempDir::new().unwrap();
    let writer = AtomicFileWriter::new(temp.path().to_path_buf());

    let first = writer.write("doc.md", "hello").unwrap();
    assert_eq!(first.file_name().unwrap(), "doc.md");
    assert_eq!(fs::read_to_string(&first).unwrap(), "hello");

    // Replace existing
    let second = writer.write("doc.md", "world").unwrap();
    assert_eq!(first, second);
    assert_eq!(fs::read_to_string(&second).unwrap(), "world");
}

#[test]
fn no_partial_file_on_error() {
    let temp = TempDir::new().unwrap();
    let file_path = temp.path().join("not_a_dir");
    fs::write(&file_path, "x").unwrap();

    let writer = AtomicFileWriter::new(file_path.clone());
    let result = writer.write("doc.md", "data");
    assert!(result.is_err());
    assert!(!file_path.with_file_name("doc.md").exists());
}
