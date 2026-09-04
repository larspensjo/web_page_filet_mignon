fn main() {
    let lockfile = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.lock");
    println!("cargo:rerun-if-changed={}", lockfile.display());
    let source = std::fs::read_to_string(lockfile).expect("workspace Cargo.lock");
    for package in ["wry", "webview2-com"] {
        println!(
            "cargo:rustc-env=HARVESTER_UI_{}_VERSION={}",
            package.replace('-', "_").to_uppercase(),
            lockfile_version(&source, package)
        );
    }
    tauri_build::build();
}

fn lockfile_version(lockfile: &str, package: &str) -> String {
    lockfile
        .split("[[package]]")
        .find_map(|entry| {
            let name = entry.lines().find_map(|line| {
                line.strip_prefix("name = \"")
                    .and_then(|value| value.strip_suffix('\"'))
            })?;
            (name == package).then(|| {
                entry
                    .lines()
                    .find_map(|line| {
                        line.strip_prefix("version = \"")
                            .and_then(|value| value.strip_suffix('\"'))
                    })
                    .expect("package version")
                    .to_owned()
            })
        })
        .expect("pinned package in Cargo.lock")
}
