use std::{fs, process::Command};

#[test]
fn cli_names_refused_store_at_start_and_finish_polls_and_exits_nonzero() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("output");
    fs::create_dir(&output).unwrap();
    fs::write(output.join(".sources.ron"), "(sources: [])").unwrap();
    let corrupt = output.join(".summary_cache.ron");
    fs::write(&corrupt, b"damaged paid results").unwrap();
    // An empty source registry and no key keep this real CLI bootstrap keyless
    // and network-free. engine.log is confined to the temporary working folder.
    let result = Command::new(env!("CARGO_BIN_EXE_harvester_batch"))
        .current_dir(dir.path())
        .env_remove("OPENAI_API_KEY")
        .args(["--single-shot", "--output-dir"])
        .arg(&output)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&result.stderr);
    let stdout = String::from_utf8_lossy(&result.stdout);
    assert_eq!(result.status.code(), Some(1), "{stdout}\n{stderr}");
    assert!(
        stderr.matches(".summary_cache.ron").count() >= 2,
        "{stderr}"
    );
    assert!(stderr.contains("parse RON"), "{stderr}");
    assert!(stderr.contains("Final summary: AI features unavailable"));
    assert!(stdout.contains("Batch complete: 1 cycles"), "{stdout}");
    assert_eq!(fs::read(&corrupt).unwrap(), b"damaged paid results");
    assert!(!output.join(".summary_cache.jsonl").exists());
}

#[test]
fn cli_summary_refresh_refuses_any_damaged_store_before_model_setup() {
    for kind in ["triage", "summary", "signal_candidate"] {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("output");
        fs::create_dir(&output).unwrap();
        let filename = format!(".{kind}_cache.ron");
        let corrupt = output.join(&filename);
        fs::write(&corrupt, b"damaged paid results").unwrap();
        let result = Command::new(env!("CARGO_BIN_EXE_harvester_batch"))
            .current_dir(dir.path())
            .env_remove("OPENAI_API_KEY")
            .args(["--refresh-stale-summaries-limit", "1", "--output-dir"])
            .arg(&output)
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&result.stderr);
        assert!(!result.status.success());
        assert!(stderr.contains(&filename), "{stderr}");
        assert!(stderr.contains("parse RON"), "{stderr}");
        assert_eq!(fs::read(&corrupt).unwrap(), b"damaged paid results");
        assert!(!corrupt.with_extension("jsonl").exists());
    }
}
