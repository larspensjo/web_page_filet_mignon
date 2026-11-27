use std::{fs, process::Command};

#[test]
fn real_batch_binary_without_launch_flag_never_reads_credentials() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("output");
    fs::create_dir(&output).unwrap();
    fs::write(output.join(".sources.ron"), "(sources: [])").unwrap();

    let result = Command::new(env!("CARGO_BIN_EXE_harvester_batch"))
        .current_dir(dir.path())
        .output()
        .unwrap();
    let stdout = String::from_utf8(result.stdout).unwrap();
    assert_eq!(result.status.code(), Some(0), "{stdout}");
    let log = fs::read_to_string(dir.path().join("engine.log")).unwrap();
    assert!(!log.contains("[credentials] read requested"), "{log}");
    assert!(
        log.contains(
            "stored keys not requested (launched without --use-stored-keys); AI features disabled"
        ),
        "{log}"
    );
    assert!(
        !log.contains("OpenAI key not found in Credential Manager"),
        "{log}"
    );
    for entry in ["Harvester/OpenAI", "Harvester/BraveSearch"] {
        let prefix = format!("[credentials] {entry}:");
        let expected = format!("{prefix} not requested (launched without --use-stored-keys)");
        let statuses: Vec<_> = stdout
            .lines()
            .filter(|line| line.starts_with(&prefix))
            .collect();
        assert_eq!(statuses, [expected], "{stdout}");
    }
}

#[test]
fn batch_reports_legacy_key_fields_only_from_the_poll_worker() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("output");
    fs::create_dir(&output).unwrap();
    fs::write(
        output.join(".sources.ron"),
        r#"(sources: [
            (id: "brave-one", source_type: BraveNews((query: "one", api_key_env: "X")),
                enabled: true, description: ""),
            (id: "curated", source_type: CuratedList(urls: []),
                enabled: true, description: ""),
            (id: "brave-two", source_type: BraveNews((query: "two", api_key_env: "Y")),
                enabled: false, description: ""),
        ])"#,
    )
    .unwrap();

    let result = Command::new(env!("CARGO_BIN_EXE_harvester_batch"))
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(0));
    let log = fs::read_to_string(dir.path().join("engine.log")).unwrap();
    let startup = log.find("Source registry entries loaded=3").expect(&log);
    let poll_load = log.find("[source-config] loading").expect(&log);
    let notice = "[sources] api_key_env is ignored; Brave sources use Credential Manager entry Harvester/BraveSearch; sources carrying it: brave-one, brave-two";
    assert_eq!(log.matches(notice).count(), 1, "{log}");
    assert!(
        startup < poll_load && poll_load < log.find(notice).unwrap(),
        "{log}"
    );
    assert!(log.contains("[brave-poll] brave-one failed: Brave Search key not found in Credential Manager (Harvester/BraveSearch)"));
    assert!(log.contains("[curated-poll] curated => 0 URL(s)"));
}
