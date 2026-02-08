use std::fs;
use std::path::Path;

use harvester_engine::is_confined_to;
use tempfile::tempdir;

fn create_linked_file(root: &Path, relative: impl AsRef<Path>) {
    let file_path = root.join(&relative);
    if let Some(parent) = file_path.parent() {
        fs::create_dir_all(parent).expect("create parent dir");
    }
    fs::write(&file_path, b"data").expect("create file");
}

#[test]
fn allows_relative_path_within_root() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().join("output");
    fs::create_dir_all(&root).expect("create root");
    let relative = Path::new("linked/foo.md");
    create_linked_file(&root, relative);

    assert!(is_confined_to(relative, &root));
}

#[test]
fn allows_various_normal_relative_paths() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().join("output");
    fs::create_dir_all(&root).expect("create root");
    create_linked_file(&root, Path::new("plain.md"));
    create_linked_file(&root, Path::new("nested/deeper.md"));

    assert!(is_confined_to(Path::new("plain.md"), &root));
    assert!(is_confined_to(Path::new("nested/deeper.md"), &root));
}

#[test]
fn denies_parent_traversal_outside_root() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().join("output");
    fs::create_dir_all(&root).expect("create root");

    assert!(!is_confined_to(Path::new("../../../etc/passwd"), &root));
}

#[test]
fn denies_nested_parent_traversal() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().join("output");
    fs::create_dir_all(&root).expect("create root");

    assert!(!is_confined_to(
        Path::new("linked/../../../etc/passwd"),
        &root
    ));
}
#[test]
fn denies_absolute_path_outside_root() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().join("output");
    fs::create_dir_all(&root).expect("create root");

    let outside = temp.path().join("outside.md");
    fs::write(&outside, b"data").expect("create outside");

    assert!(!is_confined_to(outside.as_path(), &root));
}

#[cfg(windows)]
#[test]
fn denies_windows_drive_absolute_path() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().join("output");
    fs::create_dir_all(&root).expect("create root");

    assert!(!is_confined_to(Path::new("C:\\evil"), &root));
}
