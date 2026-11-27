use crate::replay_support::{canned_prompt_kind, routing_canned_provider};
use harvester_batch::{Args, HostServices, InterruptHandler};
use harvester_core::{
    morning_archive::{target_key, CheckpointWrite},
    AiAvailability, Effect, Msg,
};
use harvester_io::{
    archive_publication::PublishStep,
    credentials::{decode_blob, CredentialEntry, DecodedBlob, KeyReadOutcome, StoredKeyReads},
    host_bootstrap::{build_effect_runner_with_provider, llm_quota_limits_from_engine},
    load_briefing_checkpoint, save_briefing_checkpoint, FileWriteObserver, NoOpPlatformHandler,
    PersistenceWorker,
};
use openai_provider_kit::{LlmError, LlmProvider, LlmRequest, LlmResponse};
use serde_json::Value;
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{mpsc, Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

pub const CHILD_MARKER: &str = "HARVESTER_ARCHIVE_CONTRACT_ARGS";
pub const SINCE: &str = "2026-09-18T00:00:00.123Z";
const URL: &str = "https://carry-over.synthetic/article-0";
const CHECKPOINT: &str = ".briefing_checkpoint.ron";
const PREFIX: &str = "HARVESTER_RESULT ";

pub struct Fixture {
    root: tempfile::TempDir,
    pub output: PathBuf,
    pub target: PathBuf,
    pub checkpoint: PathBuf,
}

impl Fixture {
    pub fn empty() -> Self {
        let root = tempfile::tempdir().unwrap();
        let output = root.path().join("output");
        let target = root.path().join("archive.md");
        let checkpoint = output.join(CHECKPOINT);
        fs::create_dir(&output).unwrap();
        fs::write(output.join(".sources.ron"), "(sources: [])").unwrap();
        fs::write(
            output.join(".harvester-contract-fixture"),
            "credential-free fixture",
        )
        .unwrap();
        write_prompt_contexts(&root.path().join("contexts"));
        save_briefing_checkpoint(&checkpoint, Some(SINCE)).unwrap();
        Self {
            root,
            output,
            target,
            checkpoint,
        }
    }

    pub fn with_article() -> Self {
        let fixture = Self::empty();
        fs::write(
            fixture.output.join("article-0.md"),
            include_str!("../fixtures/carry_over/article-0.md"),
        )
        .unwrap();
        fs::write(
            fixture.output.join(".harvester_state.ron"),
            format!(r#"(completed: [(url: "{URL}", tokens: Some(417), bytes: None, fetched_utc: Some("2026-09-20T09:15:00Z"))])"#),
        ).unwrap();
        fixture
    }

    pub fn seed_target(&self, since: &str, until: &str) -> String {
        let old = format!(
            "===== DOC START =====\nexport_schema: 2\nfetched_utc: 2026-09-18T10:00:00Z\n\nPrevious archive\n===== DOC END =====\n\n\
             ===== ARCHIVE INDEX =====\nexport_schema: 2\ndoc_count: 1\nwindow_since_utc: {since}\nwindow_until_utc: {until}\n\
             doc | line | fetched_utc | priority | signal_key | title\n1 | 1 | 2026-09-18T10:00:00Z | 5 | old | Previous archive\n===== INDEX END =====\n"
        );
        fs::write(&self.target, &old).unwrap();
        old
    }

    pub fn seed_record(&self, last_archive: &str) {
        let key = target_key(&self.target);
        fs::write(
            &self.checkpoint,
            format!(
                r#"(since_utc: Some("{SINCE}"), portfolio: {{{key:?}: (confirmed_until_utc: Some("{SINCE}"), last_archive_until_utc: Some("{last_archive}"), pending_until_utc: None)}})"#,
            ),
        ).unwrap();
    }

    pub fn seed_strikes(&self) {
        let key = target_key(&self.target);
        fs::write(
            &self.checkpoint,
            format!(
                r#"(since_utc: Some("{SINCE}"), portfolio: {{{key:?}: (confirmed_until_utc: Some("{SINCE}"), last_archive_until_utc: None, pending_until_utc: None, strikes: {{"{URL}": (count: 2, last_until: "2026-09-21T00:00:00Z")}})}})"#,
            ),
        ).unwrap();
    }

    pub fn sidecar(&self) -> PathBuf {
        self.target.with_file_name("archive.md.processed")
    }

    pub fn confirm(&self, until: &str) {
        let bound: chrono::DateTime<chrono::Utc> = until.parse().unwrap();
        // A newly published upper bound can still be in the fractional second
        // that rounded up to it. The consumer can confirm it once it has passed.
        while let Ok(remaining) = (bound - chrono::Utc::now()).to_std() {
            if remaining.is_zero() {
                break;
            }
            assert!(remaining <= Duration::from_secs(1));
            thread::sleep(remaining);
        }
        fs::write(
            self.sidecar(),
            format!(r#"{{"until_utc":"{until}","doc_count":1,"commit":"fixture"}}"#),
        )
        .unwrap();
    }

    pub fn brave_sources(&self, has_success: bool) {
        let curated = if has_success {
            r#",(id: "curated", source_type: CuratedList(urls: []), enabled: true, description: "")"#
        } else {
            ""
        };
        fs::write(
            self.output.join(".sources.ron"),
            format!(r#"(sources: [(id: "brave", source_type: BraveNews((query: "fixture")), enabled: true, description: ""){curated}])"#),
        ).unwrap();
    }

    pub fn kept(&self) -> Vec<PathBuf> {
        fs::read_dir(self.root.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("archive-unprocessed-")
            })
            .collect()
    }

    pub fn archive(&self) -> String {
        fs::read_to_string(&self.target).unwrap()
    }

    pub fn record(&self) -> String {
        fs::read_to_string(&self.checkpoint).unwrap()
    }

    pub fn checkpoint_since(&self) -> Option<String> {
        load_briefing_checkpoint(&self.checkpoint)
    }

    pub fn log(&self) -> String {
        fs::read_to_string(self.root.path().join("engine.log")).unwrap()
    }

    pub fn assert_no_poll(&self) {
        assert!(!self.log().contains("[source-poll] polling requested"));
    }

    pub fn remove_required_context(&self) {
        fs::remove_file(self.root.path().join("contexts/article_triage.toml")).unwrap();
    }

    pub fn state_exists(&self) -> bool {
        self.output.join(".harvester_state.ron").is_file()
    }

    #[cfg(not(windows))]
    pub fn clear_crash_lock(&self) {
        // The non-Windows create-new fallback has no delete-on-close handle.
        fs::remove_file(self.output.join(".harvester.lock")).unwrap();
    }

    pub fn binary(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_harvester_batch"));
        command
            .current_dir(self.root.path())
            .arg("--output-dir")
            .arg(&self.output);
        command
    }

    pub fn run(&self, mode: &str) -> Run {
        let argv = [
            "harvester_batch",
            "--output-dir",
            self.output.to_str().unwrap(),
            "--archive-out",
            self.target.to_str().unwrap(),
            "--signal-candidate-threshold",
            "0",
        ];
        let mut run = Command::new(std::env::current_exe().unwrap())
            .current_dir(self.root.path())
            .args(["--exact", "archive_contract_child", "--nocapture"])
            .env(CHILD_MARKER, serde_json::to_string(&argv).unwrap())
            .env("HARVESTER_ARCHIVE_CONTRACT_MODE", mode)
            .capture();
        run.engine_log =
            fs::read_to_string(self.root.path().join("engine.log")).unwrap_or_default();
        run
    }
}

fn write_prompt_contexts(dir: &Path) {
    fs::create_dir(dir).unwrap();
    for (filename, prompt_id, variable) in [
        (
            "article_triage.toml",
            "ArticleTriage",
            "triage_instructions",
        ),
        (
            "article_summary.toml",
            "ArticleSummary",
            "summary_instructions",
        ),
        (
            "article_signal_candidate.toml",
            "ArticleSignalCandidate",
            "context",
        ),
    ] {
        fs::write(
            dir.join(filename),
            format!(
                "[meta]\nprompt_id = \"{prompt_id}\"\nschema_version = 1\nversion = 1\nupdated = \"2026-09-18\"\n\n[variables]\n{variable} = \"Synthetic business signal fixture\"\n"
            ),
        )
        .unwrap();
    }
}

pub trait Capture {
    fn capture(&mut self) -> Run;
}

impl Capture for Command {
    fn capture(&mut self) -> Run {
        let stdout = tempfile::tempfile().unwrap();
        let stderr = tempfile::tempfile().unwrap();
        let mut child = self
            .stdout(Stdio::from(stdout.try_clone().unwrap()))
            .stderr(Stdio::from(stderr.try_clone().unwrap()))
            .spawn()
            .unwrap();
        let started = Instant::now();
        let code = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status.code();
            }
            if started.elapsed() >= Duration::from_secs(25) {
                let _ = child.kill();
                child.wait().unwrap();
                panic!(
                    "contract host timed out\n{}\n{}",
                    read_stream(stdout),
                    read_stream(stderr)
                );
            }
            thread::sleep(Duration::from_millis(15));
        };
        Run {
            code,
            stdout: read_stream(stdout),
            stderr: read_stream(stderr),
            engine_log: String::new(),
            elapsed: started.elapsed(),
        }
    }
}

fn read_stream(mut file: File) -> String {
    use std::io::{Read, Seek};
    file.rewind().unwrap();
    let mut text = String::new();
    file.read_to_string(&mut text).unwrap();
    text
}

pub struct Run {
    code: Option<i32>,
    stdout: String,
    stderr: String,
    engine_log: String,
    pub elapsed: Duration,
}

impl Run {
    pub fn result(&self, code: i32, status: &str, reason: Option<&str>) -> Value {
        self.assert_exit_code(code);
        assert!(!self.stdout.starts_with('\u{feff}'));
        assert_eq!(
            self.stdout
                .lines()
                .filter(|line| line.starts_with(PREFIX))
                .count(),
            1,
            "{}",
            self.stdout
        );
        let line = self.stdout.lines().last().unwrap();
        assert!(line.is_ascii());
        let result: Value = serde_json::from_str(line.strip_prefix(PREFIX).unwrap()).unwrap();
        assert_eq!(result["status"], status);
        assert_eq!(result["reason"].as_str(), reason);
        assert!(result.get("reason").is_some());
        result
    }

    pub fn crash(&self, code: i32) {
        self.assert_exit_code(code);
        assert!(
            !self.stdout.lines().any(|line| line.starts_with(PREFIX)),
            "{}",
            self.stdout
        );
    }

    fn assert_exit_code(&self, code: i32) {
        assert_eq!(
            self.code,
            Some(code),
            "{}\n{}\n{}",
            self.stdout,
            self.stderr,
            self.engine_log
        );
    }
}

// An extension trait keeps the entire fixture constructor in this test crate.
pub trait ContractFixture {
    fn contract_fixture(args: &Args) -> Self;
}

impl ContractFixture for HostServices {
    fn contract_fixture(args: &Args) -> Self {
        assert!(!args.use_stored_keys);
        assert!(args
            .output_dir
            .join(".harvester-contract-fixture")
            .is_file());
        let mode = std::env::var("HARVESTER_ARCHIVE_CONTRACT_MODE").unwrap();
        let (mode, flush_fails) = match mode.strip_suffix("+flush-failure") {
            Some(primary) => (primary.to_owned(), true),
            None => {
                let flush_fails = mode == "flush-failure";
                (mode, flush_fails)
            }
        };
        let target = args.archive_out.clone().unwrap();
        let interrupt = Arc::new(Mutex::new(None::<InterruptHandler>));
        let install_interrupt = Arc::clone(&interrupt);
        let key_mode = mode.clone();
        let routing_mode = mode.clone();
        let failing_record_reply = Arc::new(Mutex::new(None::<mpsc::Sender<Msg>>));
        let router_reply = Arc::clone(&failing_record_reply);
        let services = HostServices::with_adapters(
            move || canned_keys(&key_mode),
            move |paths, args, tx, _keys, defaults| {
                if mode == "runtime-failure" {
                    return Err("fixture effect runner setup failure".to_string());
                }
                assert_ne!(mode, "panic", "fixture unexpected panic");
                if mode.ends_with("-record-failure") {
                    *failing_record_reply.lock().unwrap() = Some(tx.clone());
                }
                let (io_tx, io_rx) = mpsc::channel();
                let forward_mode = mode.clone();
                let forward_interrupt = Arc::clone(&interrupt);
                thread::spawn(move || {
                    forward_replies(io_rx, tx, &forward_mode, &forward_interrupt)
                });
                let provider = Arc::new(ContractProvider {
                    canned: routing_canned_provider(URL, 5),
                    fail_scoring: mode == "scoring-failure",
                });
                let order = paths.output_dir.join("publication-order");
                let file_observer: Option<FileWriteObserver> =
                    (mode == "observe-order").then(|| {
                        let order = order.clone();
                        let observer: FileWriteObserver = Arc::new(move |path, _, _| {
                            if path.file_name().is_some_and(|name| name == CHECKPOINT) {
                                let since = load_briefing_checkpoint(path);
                                let bytes = fs::read_to_string(path).unwrap();
                                if since.as_deref() != Some(SINCE) {
                                    append_event(&order, "checkpoint");
                                } else if bytes.contains("pending_until_utc: Some(") {
                                    append_event(&order, "intent");
                                }
                            }
                        });
                        observer
                    });
                let (mut runner, config) = build_effect_runner_with_provider(
                    paths,
                    io_tx,
                    args.llm_concurrency,
                    defaults,
                    provider,
                    Box::new(NoOpPlatformHandler),
                    Box::new(PersistenceWorker::new(
                        paths.state_path.clone(),
                        paths.blacklist_path.clone(),
                    )),
                    file_observer,
                );
                if mode == "cycle-disconnected" || flush_fails {
                    runner = runner.with_result_sink(Box::new(FlushObserver {
                        flag: paths.output_dir.join("flush-called"),
                        fail: flush_fails,
                    }));
                }
                let checkpoint = paths.briefing_checkpoint_path.clone();
                runner = runner.with_archive_publication_observer(Arc::new(move |step| {
                    if step == PublishStep::KeptPrevious && mode == "slow-publication" {
                        thread::sleep(Duration::from_millis(2_750));
                    }
                    if step == PublishStep::ReplacedTarget && mode == "lose-publication-receipt" {
                        panic!("fixture export worker stopped after renaming the target");
                    }
                    if mode
                        .strip_prefix("crash-")
                        .is_some_and(|name| name == format!("{step:?}"))
                    {
                        std::process::exit(91);
                    }
                    if step == PublishStep::CleanedStaleTemps && mode.starts_with("interrupt-") {
                        let handler = interrupt.lock().unwrap();
                        let handler = handler.as_ref().unwrap();
                        handler();
                        if mode == "interrupt-twice" {
                            handler();
                        }
                    }
                    if step == PublishStep::CheckedTarget
                        && matches!(
                            mode.as_str(),
                            "export-failure" | "interrupt-export-failure" | "drop-failure"
                        )
                    {
                        fs::create_dir(&target).unwrap();
                    }
                    if step == PublishStep::ReplacedTarget && mode == "observe-order" {
                        assert_eq!(
                            load_briefing_checkpoint(&checkpoint).as_deref(),
                            Some(SINCE)
                        );
                        append_event(&order, "rename");
                    }
                }));
                Ok((
                    runner,
                    Some(llm_quota_limits_from_engine(&config.quotas)),
                    AiAvailability::Available,
                ))
            },
            move |handler| *install_interrupt.lock().unwrap() = Some(handler),
        );
        services
            .with_watchdog_timeout(Duration::from_secs(2))
            .with_effect_router(move |runner, effects| {
                let effects = effects
                    .into_iter()
                    .filter(|effect| {
                        let Effect::SavePortfolioRecord {
                            save_id,
                            record,
                            checkpoint,
                            ..
                        } = effect
                        else {
                            return true;
                        };
                        let fail = match routing_mode.as_str() {
                            "startup-record-failure" => {
                                *checkpoint == CheckpointWrite::Unchanged
                                    && record.pending_until_utc.is_none()
                            }
                            "intent-record-failure" => record.pending_until_utc.is_some(),
                            "completion-record-failure" => {
                                matches!(checkpoint, CheckpointWrite::Set(_))
                            }
                            _ => false,
                        };
                        if fail {
                            router_reply
                                .lock()
                                .unwrap()
                                .as_ref()
                                .unwrap()
                                .send(Msg::PortfolioRecordSaved {
                                    save_id: *save_id,
                                    result: Err("fixture checkpoint write failure".into()),
                                })
                                .unwrap();
                        }
                        !fail
                    })
                    .collect();
                runner.enqueue(effects);
            })
    }
}

fn canned_keys(mode: &str) -> StoredKeyReads {
    let openai = match mode {
        "missing-key" => KeyReadOutcome::NotFound,
        "empty-key" => KeyReadOutcome::Empty,
        "unreadable-key" => KeyReadOutcome::Unreadable {
            entry: CredentialEntry::OpenAi,
        },
        _ => {
            let DecodedBlob::Key {
                key,
                chars,
                encoding,
            } = decode_blob(b"canned-fixture")
            else {
                unreachable!()
            };
            KeyReadOutcome::Found {
                key,
                chars,
                encoding,
            }
        }
    };
    StoredKeyReads {
        openai,
        brave: KeyReadOutcome::NotFound,
    }
}

fn forward_replies(
    rx: mpsc::Receiver<Msg>,
    tx: mpsc::Sender<Msg>,
    mode: &str,
    interrupt: &Mutex<Option<InterruptHandler>>,
) {
    for msg in rx {
        if mode == "startup-disconnected"
            || (mode == "cycle-disconnected" && matches!(msg, Msg::PollStarted { .. }))
            || (mode == "close-success" && matches!(msg, Msg::BatchArchivePublished { .. }))
        {
            return;
        }
        let msg = match (mode, msg) {
            ("startup-watchdog", Msg::LlmMetadataLoaded { .. })
            | ("drop-success", Msg::BatchArchivePublished { .. })
            | ("drop-failure", Msg::BatchArchivePublicationFailed { .. }) => continue,
            (_, msg) => msg,
        };
        if mode == "interrupt-before" && matches!(msg, Msg::PollStarted { .. }) {
            interrupt.lock().unwrap().as_ref().unwrap()();
        }
        if tx.send(msg).is_err() {
            return;
        }
    }
}

fn append_event(path: &Path, event: &str) {
    let mut file = OpenOptions::new()
        .append(true)
        .create(true)
        .open(path)
        .unwrap();
    writeln!(file, "{event}").unwrap();
}

struct ContractProvider {
    canned: Arc<dyn LlmProvider>,
    fail_scoring: bool,
}

#[async_trait::async_trait]
impl LlmProvider for ContractProvider {
    async fn complete(&self, request: &LlmRequest) -> Result<LlmResponse, LlmError> {
        if self.fail_scoring && canned_prompt_kind(request) == "ArticleSignalCandidate" {
            return Err(LlmError::InvalidResponse {
                detail: "canned scoring failure".into(),
            });
        }
        self.canned.complete(request).await
    }

    fn provider_name(&self) -> &str {
        "archive-contract-canned"
    }
}

struct FlushObserver {
    flag: PathBuf,
    fail: bool,
}

impl harvester_io::result_sink::ResultSink for FlushObserver {
    fn enqueue(&self, _: Vec<harvester_core::SavedResult>) {}

    fn flush(&self) -> io::Result<()> {
        fs::write(&self.flag, b"flush")?;
        if self.fail {
            Err(io::Error::other("fixture results flush failure"))
        } else {
            Ok(())
        }
    }

    fn request_flush(&self) -> io::Result<()> {
        Ok(())
    }
}
