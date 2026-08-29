//! Scenario dispatch and the infrastructure every scenario module shares.
//!
//! Each scenario is one zero-sized `DesktopApp` type in its own submodule, run
//! through [`catch_run::<ConcreteApp>`](catch_run). Every scenario declares its
//! own `LaunchSpec<T>`: `T`'s parser re-parses the complete process facts to
//! confirm they still select this scenario (see [`expect_scenario`]), then
//! takes this run's one canonical `Protocol` — installed once by `main`
//! before it called [`run`] (see [`install_launch_protocol`]) — and builds
//! the `ScenarioState` this run uses throughout, emitting `scenario_started`
//! once that is done (see [`parse_core`]). There is exactly one `Protocol`
//! instance/sequence for the whole process: every scenario clones the same
//! handed-off value instead of constructing an independent one, so a fault
//! anywhere in the run — including one that escapes a scenario's own tail
//! entirely, back to `main` — always continues the same stream rather than
//! opening a second one. `T` flows immutably into `before_primary(&T, cx)`
//! and, for scenarios with a primary surface, into `Surface<View, T>`'s
//! build hook; `before_primary` installs a GPUI global from it, and every
//! later non-capturing hook (`on_event`, `after_open`, command handlers,
//! ...) reads that global through `cx` instead — using `try_global` where
//! the hook could conceivably run before `before_primary` (for example
//! `on_event`, if the framework itself fails before startup reaches the
//! launch hook), so a framework failure that early cannot compound into a
//! panic.
//!
//! `AppShell::run` never returns the parsed `T` to its caller, so one bridge
//! remains: [`Handoff`], a checked, install-once/take-once cell a scenario's
//! parser (or a launch/surface hook that has `&T`/`cx` but cannot return
//! anything) installs during the run. It is never a runtime hook's source of
//! truth — only `run()` reads it, and only after `AppShell::run` returns.
//! [`catch_run`] catches a panic from anywhere inside `AppShell::run`, so a
//! scenario's `run()` can usually reach its [`Handoff`] afterward — installed
//! by the launch parser before any platform or GPUI code that could panic.
//! A panic early enough to preempt that install still leaves the handoff
//! empty; [`recover_tail`] is the shared tail every scenario's `run()` uses to
//! recover it, and it never masks that case behind an unrelated
//! "handoff missing" message — a pre-install panic is rethrown with its
//! original payload intact, and a post-return miss (a bug in that scenario's
//! own wiring) surfaces as a normal reported error instead.
//! A scenario whose run spawned its own out-of-band worker (a thread, a
//! listener, ...) cancels and joins it unconditionally at that point,
//! whether `AppShell::run` returned or panicked, before writing any
//! terminal record — see [`finish_panicked_after_cleanup`], the panic tail
//! for those scenarios, and [`finish_panicked`], the plain panic tail for
//! scenarios with nothing of their own to clean up. Both report a
//! `panicked` terminal record through the same canonical `Protocol` the
//! rest of the run used, rather than one `main` constructed independently.
//! [`finish`] and [`finish_normal_run`] are the shared "write one terminal
//! record" tails every other outcome uses.

pub(crate) mod clipboard;
pub(crate) mod interaction_contracts;
pub(crate) mod lifecycle_background_quit;
pub(crate) mod lifecycle_clean;
pub(crate) mod lifecycle_startup_failure;
pub(crate) mod menu_command;
pub(crate) mod window_cycle;

use std::panic::AssertUnwindSafe;
use std::sync::{Arc, Mutex, MutexGuard};

use anyhow::Context as _;
use neutron_components_app::gpui::Global;
use neutron_components_app::{
    AppEvent, AppShell, AppShellError, DesktopApp, ProcessLaunch, ShutdownReason,
};
use serde_json::{Value, json};

use crate::cli::{self, Command, Scenario};
use crate::protocol::{Protocol, TerminalOutcome};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ScenarioOutcome {
    Passed,
    ExpectedStartupFailure,
}

/// Shared status used by callbacks that complete after AppShell startup.
#[derive(Clone, Default)]
pub(crate) struct ScenarioState {
    inner: Arc<Mutex<ScenarioStateInner>>,
}

#[derive(Default)]
struct ScenarioStateInner {
    failure: Option<String>,
    first_presentation_observed: bool,
    background_dispatch_executed: bool,
    background_quit_requested: bool,
    clipboard_ready: bool,
    clipboard_acknowledged: bool,
    clipboard_quit_requested: bool,
    menu_command_callback_count: usize,
}

impl ScenarioState {
    pub(crate) fn emit(&self, protocol: &Protocol, event: &'static str, data: Value) {
        if let Err(error) = protocol.emit(event, data) {
            self.record_failure(format!(
                "could not write {event} protocol record: {error:#}"
            ));
        }
    }

    pub(crate) fn record_failure(&self, failure: String) {
        let mut state = self.lock();
        if state.failure.is_none() {
            state.failure = Some(failure);
        }
    }

    pub(crate) fn failure(&self) -> Option<String> {
        self.lock().failure.clone()
    }

    pub(crate) fn mark_first_presentation(&self) {
        self.lock().first_presentation_observed = true;
    }

    pub(crate) fn first_presentation_observed(&self) -> bool {
        self.lock().first_presentation_observed
    }

    pub(crate) fn mark_background_dispatch_executed(&self) {
        self.lock().background_dispatch_executed = true;
    }

    pub(crate) fn claim_background_quit(&self) -> bool {
        let mut state = self.lock();
        if state.background_dispatch_executed && !state.background_quit_requested {
            state.background_quit_requested = true;
            true
        } else {
            false
        }
    }

    pub(crate) fn background_quit_requested(&self) -> bool {
        self.lock().background_quit_requested
    }

    pub(crate) fn mark_clipboard_ready(&self) {
        self.lock().clipboard_ready = true;
    }

    pub(crate) fn claim_clipboard_acknowledgement(&self) -> bool {
        let mut state = self.lock();
        if state.clipboard_ready && !state.clipboard_acknowledged {
            state.clipboard_acknowledged = true;
            true
        } else {
            false
        }
    }

    pub(crate) fn claim_clipboard_quit(&self) -> bool {
        let mut state = self.lock();
        if state.clipboard_acknowledged && !state.clipboard_quit_requested {
            state.clipboard_quit_requested = true;
            true
        } else {
            false
        }
    }

    pub(crate) fn clipboard_acknowledged(&self) -> bool {
        self.lock().clipboard_acknowledged
    }

    pub(crate) fn clipboard_quit_requested(&self) -> bool {
        self.lock().clipboard_quit_requested
    }

    pub(crate) fn record_menu_command_dispatch(&self) -> usize {
        let mut state = self.lock();
        state.menu_command_callback_count += 1;
        state.menu_command_callback_count
    }

    pub(crate) fn menu_command_callback_count(&self) -> usize {
        self.lock().menu_command_callback_count
    }

    fn lock(&self) -> MutexGuard<'_, ScenarioStateInner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// The GPUI global every non-capturing declaration hook reads its
/// `Protocol`/`ScenarioState` from, installed by `before_primary`.
#[derive(Clone)]
pub(crate) struct ConformanceGlobal {
    pub(crate) protocol: Protocol,
    pub(crate) state: ScenarioState,
}

impl Global for ConformanceGlobal {}

/// The immutable per-run launch value most scenarios declare as their
/// `LaunchSpec<T>` type: this run's canonical `Protocol`/`ScenarioState`,
/// built once by the launch parser (see [`parse_core`]) from the process
/// facts it confirmed select this scenario (see [`expect_scenario`]).
///
/// A scenario that needs more (network setup, a cycle tracker, ...) declares
/// its own launch type instead, typically embedding a clone of these same two
/// values.
#[derive(Clone)]
pub(crate) struct ScenarioLaunch {
    pub(crate) protocol: Protocol,
    pub(crate) state: ScenarioState,
}

/// A checked, install-once/take-once bridge for state a launch parser (or an
/// early, non-capturing declaration hook that has `&T`/`cx` but cannot return
/// anything) constructs during a run that `AppShell::run` has no way to hand
/// back to its caller. A scenario's own `run()` is the only reader, and only
/// after `AppShell::run` returns — a runtime hook always reads typed `T` or a
/// GPUI global instead, never a `Handoff`.
///
/// A second `install` before a `take` is a bug in that scenario's own launch
/// wiring, not a race (at most one scenario runs per process): rejected
/// rather than silently overwriting the first value. `take` clears the slot,
/// so a later install (a fresh test run in the same process) is never
/// confused for a stale one.
pub(crate) struct Handoff<T> {
    slot: Mutex<Option<T>>,
}

impl<T> Handoff<T> {
    pub(crate) const fn new() -> Self {
        Self {
            slot: Mutex::new(None),
        }
    }

    pub(crate) fn install(&self, value: T) -> anyhow::Result<()> {
        let mut slot = lock_or_recover(&self.slot);
        anyhow::ensure!(slot.is_none(), "handoff was already installed");
        *slot = Some(value);
        Ok(())
    }

    pub(crate) fn take(&self) -> anyhow::Result<T> {
        lock_or_recover(&self.slot)
            .take()
            .ok_or_else(|| anyhow::anyhow!("handoff was never installed"))
    }
}

/// This run's single canonical `Protocol`, constructed exactly once by
/// `main` before it calls [`run`] and consumed exactly once by
/// [`parse_core`] — the first code any scenario's own launch parser runs.
/// `main` keeps its own clone of the value it installs here (`Protocol` is a
/// cheap `Arc` handle over one shared sequence/writer), so if a fault
/// escapes a scenario's own tail entirely — for example a panic in that
/// tail's own cleanup, outside anything [`catch_run`] wraps — `main`'s
/// fallback terminal write still lands on this same stream/sequence instead
/// of opening an independent second one.
static LAUNCH_PROTOCOL: Handoff<Protocol> = Handoff::new();

/// Install this run's canonical `Protocol` for [`parse_core`] to consume.
/// `main` calls this exactly once, before `AppShell::run` (through
/// [`run`]) reaches the scenario's own launch parser.
pub(crate) fn install_launch_protocol(protocol: Protocol) -> anyhow::Result<()> {
    LAUNCH_PROTOCOL.install(protocol)
}

/// Confirm that `process` — parsed with exactly the grammar `main` uses —
/// still selects `expected`. Every scenario's launch parser calls this first:
/// `main` already chose this scenario's `DesktopApp` before `AppShell::run`
/// re-parses the same real process facts, so a mismatch here is a bug, not an
/// expected runtime condition — but the parser is `main`'s only readback, so
/// it is checked and reported rather than assumed.
pub(crate) fn expect_scenario(process: &ProcessLaunch, expected: Scenario) -> anyhow::Result<()> {
    match cli::parse_process(process)? {
        Command::Run(scenario) if scenario == expected => Ok(()),
        Command::Run(other) => Err(anyhow::anyhow!(
            "launch parser for {expected} received process facts selecting {other} instead"
        )),
        _ => Err(anyhow::anyhow!(
            "launch parser for {expected} received process facts that do not run a scenario"
        )),
    }
}

/// Take this run's canonical `Protocol` (installed once by `main`, before it
/// called [`run`], via [`install_launch_protocol`]) and a fresh
/// `ScenarioState`, and emit `scenario_started` — the first protocol record,
/// written once [`expect_scenario`] has confirmed the selection but still
/// before any platform or GPUI code runs.
pub(crate) fn parse_core(
    expected: Scenario,
    exit_policy: &'static str,
) -> anyhow::Result<(Protocol, ScenarioState)> {
    let protocol = LAUNCH_PROTOCOL
        .take()
        .context("canonical protocol handoff was not installed before the launch parser ran")?;
    anyhow::ensure!(
        protocol.scenario() == expected,
        "canonical protocol handoff carried {} but the launch parser confirmed {expected}",
        protocol.scenario()
    );
    protocol.emit(
        "scenario_started",
        json!({"runner": "native", "exit_policy": exit_policy}),
    )?;
    Ok((protocol, ScenarioState::default()))
}

/// Run `A`, catching a panic from anywhere inside `AppShell::run`.
///
/// Every scenario's `run()` calls this instead of `AppShell::run::<A>()`
/// directly, then passes the result to [`recover_tail`] alongside its own
/// `Handoff` so it can usually still reach its own tail afterward and report
/// one terminal record through the same canonical `Protocol` the rest of the
/// run used, whether `AppShell::run` returned normally or panicked (see
/// [`finish_panicked`]).
pub(crate) fn catch_run<A: DesktopApp>() -> std::thread::Result<Result<(), AppShellError>> {
    std::panic::catch_unwind(AssertUnwindSafe(AppShell::run::<A>))
}

/// Recover `tail`'s installed value alongside `result`, the outcome of the
/// [`catch_run`] a scenario's `run()` just performed.
///
/// The ordinary case: the launch parser installed `tail` before any platform
/// or GPUI code that could panic, so this returns it alongside `result`
/// unchanged, whether `AppShell::run` returned normally or panicked — the
/// caller still needs `tail` in the panic branch to report through the same
/// canonical `Protocol`.
///
/// Two failure cases exist and are deliberately not conflated:
/// - `AppShell::run` panicked *before* the launch parser reached
///   [`Handoff::install`] (a framework failure early enough to preempt the
///   scenario's own wiring): `tail` is missing, so this re-raises the
///   original panic with [`std::panic::resume_unwind`] instead of losing its
///   payload behind an unrelated "handoff missing" message.
/// - `AppShell::run` returned normally but `tail` is still missing (a bug in
///   that scenario's own launch wiring, not a race — at most one scenario
///   runs per process): this returns the handoff's own error so the caller
///   reports a normal scenario failure instead of panicking the harness.
pub(crate) fn recover_tail<T>(
    tail: &Handoff<T>,
    result: std::thread::Result<Result<(), AppShellError>>,
) -> anyhow::Result<(T, std::thread::Result<Result<(), AppShellError>>)> {
    match tail.take() {
        Ok(value) => Ok((value, result)),
        Err(missing) => match result {
            Err(panic) => std::panic::resume_unwind(panic),
            Ok(_) => Err(missing),
        },
    }
}

pub(crate) fn run(scenario: Scenario) -> anyhow::Result<ScenarioOutcome> {
    match scenario {
        Scenario::LifecycleClean => lifecycle_clean::run(),
        Scenario::LifecycleStartupFailure => lifecycle_startup_failure::run(),
        Scenario::LifecycleBackgroundQuit => lifecycle_background_quit::run(),
        Scenario::WindowCycle => window_cycle::run(),
        Scenario::MenuCommand => menu_command::run(),
        Scenario::Clipboard => clipboard::run(),
        Scenario::InteractionContracts => interaction_contracts::run(),
        // Unreachable through `main`: the CLI rejects `--scenario
        // story-smoke` before dispatch, because `neutron-story --smoke`
        // produces that stream. Reported rather than silently started, so a
        // future caller cannot construct a conformance `DesktopApp` for it.
        Scenario::StorySmoke => Err(anyhow::anyhow!(
            "story-smoke is validate-only; run neutron-story --smoke to produce it"
        )),
    }
}

/// Every non-startup-failure scenario reduces its `AppShell::run` result and
/// its `ScenarioState` through this same tail.
pub(crate) fn finish_normal_run(
    protocol: &Protocol,
    state: &ScenarioState,
    result: Result<(), AppShellError>,
) -> anyhow::Result<ScenarioOutcome> {
    emit_after_run(protocol, &result)?;
    let outcome = if let Err(error) = result {
        Err(anyhow::Error::new(error).context("native lifecycle scenario returned AppShell error"))
    } else if let Some(failure) = state.failure() {
        Err(anyhow::anyhow!(
            "native lifecycle scenario failed: {failure}"
        ))
    } else {
        Ok(ScenarioOutcome::Passed)
    };
    finish(protocol, outcome)
}

/// Write the one terminal record for an already-decided `outcome` and return
/// it unchanged, so `main` only has to turn it into an exit code. Shared by
/// [`finish_normal_run`] and by the scenarios (`lifecycle-startup-failure`,
/// `clipboard`) that decide their own outcome instead.
pub(crate) fn finish(
    protocol: &Protocol,
    outcome: anyhow::Result<ScenarioOutcome>,
) -> anyhow::Result<ScenarioOutcome> {
    let terminal = match &outcome {
        Ok(ScenarioOutcome::Passed) => TerminalOutcome::Passed,
        Ok(ScenarioOutcome::ExpectedStartupFailure) => TerminalOutcome::ExpectedStartupFailure,
        Err(error) => TerminalOutcome::Failed(format!("{error:#}")),
    };
    protocol.terminal(terminal)?;
    outcome
}

/// The tail [`catch_run`]'s caller uses when `AppShell::run` panicked: write
/// the `panicked` terminal record through `protocol` (recovered from the
/// scenario's own `Handoff`, so it is the same one used throughout the run)
/// and report the panic the same way `main` reports any other scenario
/// failure.
pub(crate) fn finish_panicked(
    protocol: &Protocol,
    panic: Box<dyn std::any::Any + Send>,
) -> anyhow::Result<ScenarioOutcome> {
    let message = format!("conformance scenario panicked: {}", panic_message(panic));
    protocol.terminal(TerminalOutcome::Panicked(message.clone()))?;
    Err(anyhow::anyhow!(message))
}

/// The tail a scenario whose run spawns its own out-of-band worker (a
/// thread, a listener, ...) uses when `AppShell::run` panicked: unlike
/// [`finish_panicked`], this scenario cannot write its terminal record yet,
/// because that worker may still be running — cancelling and joining it is
/// this scenario's responsibility, done by its caller before calling this
/// function, and passed here as `cleanup`. A cleanup failure is folded into
/// the panic diagnostic instead of replacing it, so the original panic is
/// never lost, and the terminal record is written only once cleanup has
/// been attempted — never before it.
pub(crate) fn finish_panicked_after_cleanup(
    protocol: &Protocol,
    panic: Box<dyn std::any::Any + Send>,
    cleanup: anyhow::Result<()>,
) -> anyhow::Result<ScenarioOutcome> {
    let mut message = format!("conformance scenario panicked: {}", panic_message(panic));
    if let Err(cleanup_error) = cleanup {
        message = format!("{message}; scenario cleanup after the panic failed: {cleanup_error:#}");
    }
    protocol.terminal(TerminalOutcome::Panicked(message.clone()))?;
    Err(anyhow::anyhow!(message))
}

/// These records are deliberately emitted after `AppShell::run` returns.
pub(crate) fn emit_after_run(
    protocol: &Protocol,
    result: &Result<(), AppShellError>,
) -> anyhow::Result<()> {
    protocol.emit("shutdown_complete", json!({}))?;
    protocol.emit(
        "run_returned",
        json!({
            "result": if result.is_ok() { "ok" } else { "error" },
        }),
    )?;
    Ok(())
}

pub(crate) fn observe_app_event(protocol: &Protocol, state: &ScenarioState, event: &AppEvent) {
    state.emit(protocol, "app_event", json!({"kind": event.name()}));
    match event {
        AppEvent::ShutdownRequested(reason) => {
            state.emit(
                protocol,
                "shutdown_started",
                json!({"reason": shutdown_reason_name(*reason)}),
            );
        }
        AppEvent::WillExit => {
            state.emit(protocol, "will_exit", json!({}));
        }
        _ => {}
    }
}

fn shutdown_reason_name(reason: ShutdownReason) -> &'static str {
    match reason {
        ShutdownReason::StartupFailure => "startup_failure",
        ShutdownReason::LastWindowClosed => "last_window_closed",
        ShutdownReason::Requested => "requested",
        ShutdownReason::PlatformQuit => "platform_quit",
        _ => "other",
    }
}

pub(crate) fn lock_or_recover<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub(crate) fn panic_message(panic: Box<dyn std::any::Any + Send + 'static>) -> String {
    if let Some(message) = panic.downcast_ref::<&str>() {
        (*message).to_owned()
    } else if let Some(message) = panic.downcast_ref::<String>() {
        message.clone()
    } else {
        "non-string panic payload".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use std::io::{self, Write};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread::{self, JoinHandle};

    use super::*;

    /// An in-memory writer, mirroring `protocol::tests::TestWriter`, so
    /// these tests can inspect the exact record stream a `Protocol` writes
    /// without touching stdout or the real process-wide launch-protocol
    /// handoff.
    #[derive(Clone, Default)]
    struct TestWriter {
        bytes: Arc<Mutex<Vec<u8>>>,
    }

    impl Write for TestWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.bytes
                .lock()
                .expect("test output mutex should not be poisoned")
                .extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn records_from(bytes: &Arc<Mutex<Vec<u8>>>) -> Vec<Value> {
        let output = String::from_utf8(
            bytes
                .lock()
                .expect("test output mutex should not be poisoned")
                .clone(),
        )
        .expect("protocol output should be utf-8");
        output
            .lines()
            .map(|line| serde_json::from_str(line).expect("each line should be one JSON record"))
            .collect()
    }

    #[test]
    fn background_hold_release_requires_dispatch() {
        let state = ScenarioState::default();

        assert!(!state.background_quit_requested());
        assert!(!state.claim_background_quit());
        state.mark_background_dispatch_executed();
        assert!(state.claim_background_quit());
        assert!(state.background_quit_requested());
    }

    #[test]
    fn menu_command_callback_count_tracks_duplicate_dispatches() {
        let state = ScenarioState::default();

        assert_eq!(state.record_menu_command_dispatch(), 1);
        assert_eq!(state.record_menu_command_dispatch(), 2);
        assert_eq!(state.menu_command_callback_count(), 2);
    }

    #[test]
    fn clipboard_quit_requires_external_acknowledgement() {
        let state = ScenarioState::default();

        assert!(!state.clipboard_acknowledged());
        assert!(!state.clipboard_quit_requested());
        assert!(!state.claim_clipboard_acknowledgement());
        assert!(!state.claim_clipboard_quit());
        state.mark_clipboard_ready();
        assert!(state.claim_clipboard_acknowledgement());
        assert!(state.clipboard_acknowledged());
        assert!(!state.claim_clipboard_acknowledgement());
        assert!(state.claim_clipboard_quit());
        assert!(state.clipboard_quit_requested());
        assert!(!state.claim_clipboard_quit());
    }

    #[test]
    fn handoff_rejects_a_second_install_before_take() {
        let handoff: Handoff<u32> = Handoff::new();

        handoff.install(1).expect("first install should succeed");

        assert!(handoff.install(2).is_err());
        assert_eq!(
            handoff
                .take()
                .expect("the first installed value should be recoverable"),
            1
        );
    }

    #[test]
    fn handoff_rejects_take_before_any_install() {
        let handoff: Handoff<u32> = Handoff::new();

        assert!(handoff.take().is_err());
    }

    #[test]
    fn handoff_clears_after_take_so_it_can_be_reinstalled() {
        let handoff: Handoff<u32> = Handoff::new();

        handoff.install(1).expect("first install should succeed");
        assert_eq!(handoff.take().expect("value should be recoverable"), 1);

        handoff
            .install(2)
            .expect("a cleared handoff should accept a new install");
        assert_eq!(
            handoff.take().expect("second value should be recoverable"),
            2
        );
    }

    /// A panic early enough to preempt the launch parser's own `install`
    /// leaves the handoff empty. `recover_tail` must rethrow that original
    /// panic — payload intact — rather than masking it behind its own
    /// "handoff missing" message.
    #[test]
    fn recover_tail_rethrows_the_original_panic_when_the_handoff_is_empty() {
        let handoff: Handoff<u32> = Handoff::new();
        let original: std::thread::Result<Result<(), AppShellError>> =
            std::panic::catch_unwind(|| -> Result<(), AppShellError> {
                panic!("original scenario panic")
            });
        assert!(original.is_err(), "the setup panic should be caught");

        let rethrown =
            std::panic::catch_unwind(AssertUnwindSafe(|| recover_tail(&handoff, original)))
                .expect_err("a missing handoff must rethrow the original panic, not swallow it");

        let payload = rethrown
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| rethrown.downcast_ref::<String>().map(String::as_str));
        assert_eq!(
            payload,
            Some("original scenario panic"),
            "the rethrown panic must carry the original payload unchanged",
        );
    }

    /// `AppShell::run` returning normally with the handoff still empty is a
    /// bug in that scenario's own launch wiring, not a race — `recover_tail`
    /// reports it as a normal error instead of panicking the harness.
    #[test]
    fn recover_tail_returns_the_handoff_error_when_the_run_returned_normally_without_a_tail() {
        let handoff: Handoff<u32> = Handoff::new();

        let error = recover_tail(&handoff, Ok(Ok(())))
            .expect_err("a missing handoff with no panic must surface as a normal error");

        assert!(
            error.to_string().contains("handoff was never installed"),
            "unexpected error: {error:#}",
        );
    }

    #[test]
    fn expect_scenario_accepts_a_matching_process() {
        let process = ProcessLaunch::new(vec!["--scenario".into(), "lifecycle-clean".into()], None);

        expect_scenario(&process, Scenario::LifecycleClean)
            .expect("the matching scenario should be accepted");
    }

    #[test]
    fn expect_scenario_rejects_a_mismatched_scenario() {
        let process = ProcessLaunch::new(vec!["--scenario".into(), "window-cycle".into()], None);

        let error = expect_scenario(&process, Scenario::LifecycleClean).unwrap_err();

        assert!(error.to_string().contains("window-cycle"));
    }

    #[test]
    fn expect_scenario_rejects_process_facts_that_do_not_run_a_scenario() {
        let process = ProcessLaunch::new(vec!["--validate".into(), "lifecycle-clean".into()], None);

        assert!(expect_scenario(&process, Scenario::LifecycleClean).is_err());
    }

    #[test]
    fn expect_scenario_propagates_a_cli_parse_failure() {
        let process = ProcessLaunch::new(vec!["--scenario".into(), "not-a-scenario".into()], None);

        assert!(expect_scenario(&process, Scenario::LifecycleClean).is_err());
    }

    /// Mirrors the canonical-protocol handoff `main`/`install_launch_protocol`
    /// and each scenario's own launch parser/`parse_core` use, but through a
    /// local `Handoff` instead of the real process-wide `LAUNCH_PROTOCOL`, so
    /// this test cannot race any other test that touches that static. Proves
    /// there is exactly one `Protocol` instance/sequence for a whole run: a
    /// fault that escapes a scenario's own tail entirely still reaches
    /// `main`'s kept clone, and that fallback continues the same
    /// stream/sequence instead of starting an independent second one.
    #[test]
    fn a_protocol_handed_off_and_taken_keeps_one_shared_sequence() {
        let writer = TestWriter::default();
        let bytes = Arc::clone(&writer.bytes);
        let protocol = Protocol::with_writer(Scenario::Clipboard, Box::new(writer));
        let main_fallback_clone = protocol.clone();

        let handoff: Handoff<Protocol> = Handoff::new();
        handoff.install(protocol).expect(
            "main installs the canonical protocol before the scenario's launch parser runs",
        );

        let taken = handoff
            .take()
            .expect("the launch parser takes the canonical protocol exactly once");
        taken
            .emit(
                "scenario_started",
                json!({"runner": "native", "exit_policy": "explicit"}),
            )
            .expect("scenario_started should write on the canonical stream");

        // The scenario's own tail never got a chance to write a terminal
        // record (that is exactly the fault this models), so `main`'s
        // fallback -- which only ever held the clone it kept before
        // installing the handoff -- writes the real one.
        assert!(
            main_fallback_clone
                .terminal(TerminalOutcome::Panicked("boom".into()))
                .expect("fallback terminal should write")
        );
        // A second attempt through the other clone must be the documented
        // no-op, never a second, independent terminal record.
        assert!(
            !taken
                .terminal(TerminalOutcome::Passed)
                .expect("a repeated terminal attempt must not error")
        );

        let records = records_from(&bytes);
        assert_eq!(
            records.len(),
            2,
            "one scenario_started and one terminal -- never a second sequence-1 stream"
        );
        assert_eq!(records[0]["event"], "scenario_started");
        assert_eq!(records[0]["sequence"], 1);
        assert_eq!(records[1]["event"], "terminal");
        assert_eq!(records[1]["sequence"], 2);
        assert_eq!(records[1]["data"]["outcome"], "panicked");
    }

    #[test]
    fn finish_panicked_after_cleanup_preserves_the_panic_when_cleanup_also_fails() {
        let writer = TestWriter::default();
        let bytes = Arc::clone(&writer.bytes);
        let protocol = Protocol::with_writer(Scenario::LifecycleBackgroundQuit, Box::new(writer));
        protocol
            .emit(
                "scenario_started",
                json!({"runner": "native", "exit_policy": "when_idle"}),
            )
            .expect("scenario_started should write");

        let panic: Box<dyn std::any::Any + Send> =
            Box::new("AppShell::run panicked mid-startup".to_owned());
        let cleanup: anyhow::Result<()> = Err(anyhow::anyhow!("background worker could not join"));

        let error = finish_panicked_after_cleanup(&protocol, panic, cleanup)
            .expect_err("a panicked run must still report an error outcome");
        let message = error.to_string();
        assert!(
            message.contains("AppShell::run panicked mid-startup"),
            "the original panic must not be lost: {message}"
        );
        assert!(
            message.contains("background worker could not join"),
            "the cleanup failure must be folded into the diagnostic: {message}"
        );

        let records = records_from(&bytes);
        assert_eq!(
            records.len(),
            2,
            "exactly one terminal record must be written"
        );
        assert_eq!(records[1]["event"], "terminal");
        assert_eq!(records[1]["data"]["outcome"], "panicked");
        let panic_field = records[1]["data"]["panic"]
            .as_str()
            .expect("terminal panic field should be a string");
        assert!(panic_field.contains("AppShell::run panicked mid-startup"));
        assert!(panic_field.contains("background worker could not join"));
    }

    #[test]
    fn finish_panicked_after_cleanup_still_reports_the_panic_when_cleanup_succeeds() {
        let writer = TestWriter::default();
        let bytes = Arc::clone(&writer.bytes);
        let protocol = Protocol::with_writer(Scenario::Clipboard, Box::new(writer));
        protocol
            .emit(
                "scenario_started",
                json!({"runner": "native", "exit_policy": "explicit"}),
            )
            .expect("scenario_started should write");

        let panic: Box<dyn std::any::Any + Send> = Box::new("boom".to_owned());
        let error = finish_panicked_after_cleanup(&protocol, panic, Ok(()))
            .expect_err("a panicked run must still report an error outcome");
        assert!(error.to_string().contains("boom"));

        let records = records_from(&bytes);
        assert_eq!(records.len(), 2);
        assert_eq!(records[1]["data"]["outcome"], "panicked");
        assert_eq!(
            records[1]["data"]["panic"], "conformance scenario panicked: boom",
            "a clean cleanup must not add spurious text to the diagnostic"
        );
    }

    /// Models a panic before a scenario's tail installed its worker handoff
    /// (for example, a panic inside `before_primary`, before it reaches
    /// `thread::spawn`) -- using a local `Handoff`, standing in for a
    /// scenario's own `WORKER_TAIL`. Joining must fail explicitly, never
    /// hang, and that failure must fold into the panic diagnostic instead of
    /// replacing it.
    #[test]
    fn worker_cleanup_after_a_panic_handles_a_worker_that_was_never_installed() {
        let worker_tail: Handoff<JoinHandle<()>> = Handoff::new();

        let writer = TestWriter::default();
        let protocol = Protocol::with_writer(Scenario::LifecycleBackgroundQuit, Box::new(writer));
        protocol
            .emit(
                "scenario_started",
                json!({"runner": "native", "exit_policy": "when_idle"}),
            )
            .expect("scenario_started should write");

        let panic: Box<dyn std::any::Any + Send> =
            Box::new("panic before the worker was spawned".to_owned());
        let cleanup =
            worker_tail
                .take()
                .context("worker was never started")
                .map(|worker: JoinHandle<()>| {
                    worker.join().expect("test worker should not panic");
                });
        assert!(
            cleanup.is_err(),
            "joining a worker that was never installed must fail explicitly, not hang"
        );

        let message = finish_panicked_after_cleanup(&protocol, panic, cleanup)
            .expect_err("a panicked run must still report an error outcome")
            .to_string();
        assert!(message.contains("panic before the worker was spawned"));
        assert!(message.contains("worker was never started"));
    }

    /// Models a panic after a scenario's tail installed its worker handoff
    /// and spawned the worker -- using a local `Handoff`, standing in for a
    /// scenario's own `WORKER_TAIL`. Cleanup must join it -- `JoinHandle::join`
    /// blocks until the thread finishes, so a successful join here is proof
    /// the worker is no longer running -- before the panic diagnostic is
    /// ever produced.
    #[test]
    fn worker_cleanup_after_a_panic_joins_a_worker_that_was_already_spawned() {
        let worker_tail: Handoff<JoinHandle<()>> = Handoff::new();
        let worker_ran = Arc::new(AtomicBool::new(false));
        let worker_ran_marker = Arc::clone(&worker_ran);
        let worker = thread::spawn(move || {
            worker_ran_marker.store(true, Ordering::SeqCst);
        });
        worker_tail
            .install(worker)
            .expect("this scenario's before_primary installs its worker exactly once");

        let writer = TestWriter::default();
        let protocol = Protocol::with_writer(Scenario::LifecycleBackgroundQuit, Box::new(writer));
        protocol
            .emit(
                "scenario_started",
                json!({"runner": "native", "exit_policy": "when_idle"}),
            )
            .expect("scenario_started should write");

        let panic: Box<dyn std::any::Any + Send> =
            Box::new("panic after the worker was spawned".to_owned());
        let cleanup =
            worker_tail
                .take()
                .context("worker was never started")
                .map(|worker: JoinHandle<()>| {
                    worker.join().expect("test worker should not panic");
                });
        assert!(cleanup.is_ok(), "joining a spawned worker must succeed");
        assert!(
            worker_ran.load(Ordering::SeqCst),
            "the worker must have finished before cleanup completes"
        );

        let message = finish_panicked_after_cleanup(&protocol, panic, cleanup)
            .expect_err("a panicked run must still report an error outcome")
            .to_string();
        assert!(message.contains("panic after the worker was spawned"));
        assert!(
            !message.contains("cleanup"),
            "a clean join must not add spurious cleanup text: {message}"
        );
    }
}
