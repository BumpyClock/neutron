//! Opt-in `story-smoke` runtime evidence for the `neutron-story` binary.
//!
//! Stage 1 sets [`EVIDENCE_PATH_VARIABLE`] to a path inside the running job's
//! artifact directory; `--smoke` then writes one synchronized JSONL stream
//! there. The variable is the only switch: a normal gallery run, and an
//! ordinary `--smoke` run outside Stage 1, never create an evidence file.
//!
//! The record shape matches the native conformance protocol
//! (`schema`/`sequence`/`scenario`/`event`/`data`) so
//! `neutron-components-conformance --validate story-smoke` reads this stream
//! with its existing parser. The `scenario` value is fixed to
//! [`SCENARIO`].
//!
//! Scope: this stream proves *integration* facts only — that the real typed
//! `DesktopApp` declaration reached its primary Gallery surface, resolved its
//! platform menu model, loaded its bundled themes, presented once, and shut
//! down cleanly through AppShell. Native window handles, renderer selection,
//! clipboard, input, and accessibility evidence stay owned by the conformance
//! scenarios; duplicating them here would claim proof this binary does not
//! collect.

use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use anyhow::Context as _;
use serde::Serialize;
use serde_json::{Value, json};

/// The evidence schema version. Shared with the conformance protocol.
pub(crate) const SCHEMA_VERSION: u8 = 1;
/// The fixed scenario name every record carries.
pub(crate) const SCENARIO: &str = "story-smoke";
/// The only switch that turns evidence on.
pub(crate) const EVIDENCE_PATH_VARIABLE: &str = "GPUI_STAGE1_STORY_EVIDENCE_PATH";

/// The strict record order the validator enforces. Declared here so the
/// emitting sites and the documented contract cannot drift apart.
pub(crate) const STORY_STARTED: &str = "story_started";
pub(crate) const PRIMARY_OPENED: &str = "primary_opened";
pub(crate) const MENU_PROJECTED: &str = "menu_projected";
pub(crate) const THEMES_LOADED: &str = "themes_loaded";
pub(crate) const FIRST_PRESENTED: &str = "first_presented";
pub(crate) const QUIT_REQUESTED: &str = "quit_requested";
pub(crate) const SHUTDOWN_REQUESTED: &str = "shutdown_requested";
pub(crate) const WILL_EXIT: &str = "will_exit";
pub(crate) const RUN_RETURNED: &str = "run_returned";
pub(crate) const TERMINAL: &str = "terminal";

/// This process's active evidence stream, installed by the `--smoke` launch
/// path so the surface hooks, the lifecycle hook, and `main`'s post-run tail
/// all write through one writer and one sequence.
static ACTIVE: OnceLock<StoryEvidence> = OnceLock::new();

/// A synchronized JSONL evidence writer. Cheap to clone: every clone shares
/// one writer, one sequence counter, and one failure slot.
#[derive(Clone)]
pub(crate) struct StoryEvidence {
    state: Arc<Mutex<EvidenceState>>,
}

struct EvidenceState {
    next_sequence: u64,
    terminal_written: bool,
    failure: Option<String>,
    writer: Box<dyn Write + Send>,
}

#[derive(Serialize)]
struct Record<'a> {
    schema: u8,
    sequence: u64,
    scenario: &'a str,
    event: &'a str,
    data: Value,
}

impl StoryEvidence {
    /// Open the evidence stream named by [`EVIDENCE_PATH_VARIABLE`].
    ///
    /// Returns `Ok(None)` when the variable is absent: that is an ordinary
    /// run, and no file is created.
    ///
    /// # Errors
    ///
    /// Returns an error when the variable is set but empty, is not valid
    /// Unicode, or the file cannot be created. Stage 1 must fail loudly here
    /// rather than run without the evidence it was asked to produce.
    pub(crate) fn from_env() -> anyhow::Result<Option<Self>> {
        let path = match std::env::var(EVIDENCE_PATH_VARIABLE) {
            Ok(path) => PathBuf::from(path),
            Err(std::env::VarError::NotPresent) => return Ok(None),
            Err(error) => {
                return Err(
                    anyhow::Error::new(error).context(format!("read {EVIDENCE_PATH_VARIABLE}"))
                );
            }
        };
        if path.as_os_str().is_empty() {
            anyhow::bail!("{EVIDENCE_PATH_VARIABLE} was set to an empty path");
        }
        let file = File::create(&path)
            .with_context(|| format!("create story evidence file {}", path.display()))?;
        Ok(Some(Self::with_writer(Box::new(file))))
    }

    fn with_writer(writer: Box<dyn Write + Send>) -> Self {
        Self {
            state: Arc::new(Mutex::new(EvidenceState {
                next_sequence: 1,
                terminal_written: false,
                failure: None,
                writer,
            })),
        }
    }

    /// Write one non-terminal record and flush it before returning.
    ///
    /// # Errors
    ///
    /// Returns the serialization or I/O error, and records it as this
    /// stream's failure so the post-run tail still reports a failed run.
    pub(crate) fn emit(&self, event: &'static str, data: Value) -> anyhow::Result<()> {
        self.write_record(event, data, false).map(|_| ())
    }

    /// Write the single terminal record. A repeated attempt is ignored.
    fn terminal(&self, outcome: &TerminalOutcome) -> anyhow::Result<()> {
        self.write_record(TERMINAL, outcome.data(), true)
            .map(|_| ())
    }

    /// Record a failure observed on a path that cannot propagate an error.
    /// The first failure wins: later noise never hides the original cause.
    pub(crate) fn record_failure(&self, failure: impl Into<String>) {
        let mut state = self.lock();
        if state.failure.is_none() {
            state.failure = Some(failure.into());
        }
    }

    /// This stream's first recorded failure, if any.
    pub(crate) fn failure(&self) -> Option<String> {
        self.lock().failure.clone()
    }

    fn write_record(
        &self,
        event: &'static str,
        data: Value,
        terminal: bool,
    ) -> anyhow::Result<bool> {
        let mut state = self.lock();
        if state.terminal_written {
            if terminal {
                return Ok(false);
            }
            anyhow::bail!("cannot write story evidence record after terminal");
        }

        let sequence = state.next_sequence;
        state.next_sequence = sequence
            .checked_add(1)
            .context("story evidence sequence overflow")?;
        if terminal {
            // Set before writing so a partial I/O failure cannot let another
            // caller append a second terminal line.
            state.terminal_written = true;
        }

        let record = Record {
            schema: SCHEMA_VERSION,
            sequence,
            scenario: SCENARIO,
            event,
            data,
        };
        let result = serde_json::to_writer(&mut state.writer, &record)
            .context("serialize story evidence record")
            .and_then(|()| {
                state
                    .writer
                    .write_all(b"\n")
                    .context("terminate story evidence record")
            })
            .and_then(|()| state.writer.flush().context("flush story evidence record"));
        if let Err(error) = result {
            let failure = format!("story evidence write failed at {event}: {error:#}");
            if state.failure.is_none() {
                state.failure = Some(failure);
            }
            return Err(error);
        }
        Ok(true)
    }

    fn lock(&self) -> MutexGuard<'_, EvidenceState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// The terminal record's outcome. `passed` is only ever written by
/// [`finish`], after `AppShell::run` has actually returned.
enum TerminalOutcome {
    Passed,
    Failed(String),
}

impl TerminalOutcome {
    const fn exit_code(&self) -> u8 {
        match self {
            Self::Passed => 0,
            Self::Failed(_) => 1,
        }
    }

    fn data(&self) -> Value {
        match self {
            Self::Passed => json!({"outcome": "passed", "exit_code": self.exit_code()}),
            Self::Failed(error) => json!({
                "outcome": "failed",
                "exit_code": self.exit_code(),
                "error": error,
            }),
        }
    }
}

/// Install this process's evidence stream.
///
/// # Errors
///
/// Returns an error if a stream was already installed, which would mean two
/// launch paths ran in one process.
pub(crate) fn install(evidence: StoryEvidence) -> anyhow::Result<()> {
    ACTIVE
        .set(evidence)
        .map_err(|_| anyhow::anyhow!("story evidence was already installed"))
}

/// This process's evidence stream, or `None` for an ordinary run.
pub(crate) fn active() -> Option<&'static StoryEvidence> {
    ACTIVE.get()
}

/// Write the post-run tail: `run_returned` and then the one terminal record.
///
/// Called by `main` *after* `AppShell::run` returns, so a `passed` terminal
/// can never be claimed by a still-running application. A no-op when no
/// evidence stream is active.
///
/// # Errors
///
/// Returns an error when a record cannot be written, or when the run itself
/// failed, or when an earlier hook recorded a failure. `main` keeps its own
/// exit-code contract; this only reports.
pub(crate) fn finish(
    outcome: &Result<(), neutron_components_app::AppShellError>,
) -> anyhow::Result<()> {
    let Some(evidence) = active() else {
        return Ok(());
    };

    let run_result = match outcome {
        Ok(()) => "ok",
        Err(_) => "error",
    };
    evidence.emit(RUN_RETURNED, json!({"result": run_result}))?;

    let failure = match outcome {
        Err(error) => Some(format!("AppShell::run returned an error: {error}")),
        Ok(()) => evidence.failure(),
    };
    match failure {
        Some(failure) => {
            evidence.terminal(&TerminalOutcome::Failed(failure.clone()))?;
            Err(anyhow::anyhow!(failure))
        }
        None => evidence.terminal(&TerminalOutcome::Passed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Default)]
    struct TestWriter {
        bytes: Arc<Mutex<Vec<u8>>>,
    }

    impl Write for TestWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.bytes
                .lock()
                .expect("test writer mutex should not be poisoned")
                .extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn records(bytes: &Arc<Mutex<Vec<u8>>>) -> Vec<Value> {
        let output = String::from_utf8(
            bytes
                .lock()
                .expect("test writer mutex should not be poisoned")
                .clone(),
        )
        .expect("evidence should be utf-8");
        output
            .lines()
            .map(|line| serde_json::from_str(line).expect("each line should be JSON"))
            .collect()
    }

    #[test]
    fn records_are_schema_versioned_and_contiguous() {
        let writer = TestWriter::default();
        let bytes = Arc::clone(&writer.bytes);
        let evidence = StoryEvidence::with_writer(Box::new(writer));

        evidence
            .emit(STORY_STARTED, json!({"runner": "neutron-story"}))
            .expect("first record should write");
        evidence
            .emit(PRIMARY_OPENED, json!({"surface": "primary"}))
            .expect("second record should write");
        evidence
            .terminal(&TerminalOutcome::Passed)
            .expect("terminal should write");

        let records = records(&bytes);
        assert_eq!(records.len(), 3);
        for (index, record) in records.iter().enumerate() {
            assert_eq!(record["schema"], SCHEMA_VERSION);
            assert_eq!(record["sequence"], index + 1);
            assert_eq!(record["scenario"], SCENARIO);
        }
        assert_eq!(records[2]["event"], TERMINAL);
        assert_eq!(records[2]["data"]["outcome"], "passed");
        assert_eq!(records[2]["data"]["exit_code"], 0);
    }

    #[test]
    fn a_second_terminal_is_ignored_and_later_records_are_rejected() {
        let writer = TestWriter::default();
        let bytes = Arc::clone(&writer.bytes);
        let evidence = StoryEvidence::with_writer(Box::new(writer));

        evidence
            .terminal(&TerminalOutcome::Passed)
            .expect("terminal should write");
        evidence
            .terminal(&TerminalOutcome::Failed("duplicate".to_owned()))
            .expect("a repeated terminal is ignored, not an error");
        evidence
            .emit(WILL_EXIT, json!({}))
            .expect_err("no record may follow the terminal");

        assert_eq!(records(&bytes).len(), 1);
    }

    #[test]
    fn the_first_recorded_failure_wins() {
        let evidence = StoryEvidence::with_writer(Box::new(TestWriter::default()));

        evidence.record_failure("first");
        evidence.record_failure("second");

        assert_eq!(evidence.failure().as_deref(), Some("first"));
    }

    #[test]
    fn an_absent_variable_creates_no_evidence() {
        // Deliberately not `set_var`/`remove_var`: the test process shares one
        // environment, and Stage 1 is the only caller that sets this. An
        // unset variable is the ordinary case every developer run takes.
        assert!(
            std::env::var_os(EVIDENCE_PATH_VARIABLE).is_some()
                || StoryEvidence::from_env()
                    .expect("an absent variable is not an error")
                    .is_none()
        );
    }
}
