use std::sync::Mutex;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};

use anyhow::Context as _;
use neutron_components_app::gpui::{App, Global};
use neutron_components_app::{
    AppDeclaration, AppEvent, AppProxy, DesktopApp, ExitPolicy, InitialActivation, LaunchDecision,
    LaunchSpec, ProcessLaunch, Shell as _, ShellHold,
};
use serde_json::json;

use crate::cli::Scenario;
use crate::protocol::Protocol;
use crate::scenarios::{
    self, Handoff, ScenarioOutcome, ScenarioState, catch_run, emit_after_run, lock_or_recover,
    observe_app_event, panic_message,
};

/// Recovered by [`run`] after `AppShell::run` returns: this run's canonical
/// `Protocol`/`ScenarioState` plus an independent clone of the dispatch
/// trigger, so `run` can unblock a worker that never received a legitimate
/// `Started` trigger (for example, because `AppShell::run` panicked before
/// `on_event` ever dispatched one) without depending on GPUI dropping its
/// own clone of the sender. Installed by [`parse`].
static TAIL: Handoff<BackgroundQuitTail> = Handoff::new();
/// Installed by `before_primary`, which spawns the worker (this scenario
/// opens no primary surface for `after_open` to spawn it from instead), and
/// taken by [`run`] after `AppShell::run` returns.
static WORKER_TAIL: Handoff<JoinHandle<BackgroundWorkerReport>> = Handoff::new();

struct BackgroundQuitTail {
    protocol: Protocol,
    state: ScenarioState,
    cancel_trigger: mpsc::Sender<BackgroundDispatchSignal>,
}

/// This run's launch value: the conformance protocol/state plus the
/// dispatch channel halves the launch parser built, before any platform or
/// GPUI code runs. `dispatch` is drained exactly once, by `before_primary` —
/// a `Mutex` because `before_primary` only ever sees `&BackgroundQuitLaunch`,
/// never an owned value it could destructure directly.
struct BackgroundQuitLaunch {
    protocol: Protocol,
    state: ScenarioState,
    dispatch: Mutex<Option<BackgroundQuitDispatch>>,
}

/// The channel halves `before_primary` installs into the global (`trigger`,
/// for `on_event` to consume once) and hands to the worker it spawns
/// (`waiter`).
struct BackgroundQuitDispatch {
    trigger: mpsc::Sender<BackgroundDispatchSignal>,
    waiter: mpsc::Receiver<BackgroundDispatchSignal>,
}

/// A legitimate `Started` trigger from `on_event`, or a cancellation `run`
/// sends — through its own independent [`BackgroundQuitTail::cancel_trigger`]
/// clone, never the global's — to unblock the worker after a panic instead
/// of leaving it blocked on [`mpsc::Receiver::recv`] forever.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BackgroundDispatchSignal {
    Start,
    Cancel,
}

struct LifecycleBackgroundQuitApp;

impl DesktopApp for LifecycleBackgroundQuitApp {
    fn declaration() -> AppDeclaration {
        AppDeclaration::new(crate::APP_IDENTITY)
            .initial_activation(InitialActivation::Passive)
            .exit_policy(ExitPolicy::WhenIdle)
            .on_event(on_event)
            .launch(LaunchSpec::new(parse).before_primary(before_primary))
    }
}

fn parse(process: &ProcessLaunch) -> anyhow::Result<LaunchDecision<BackgroundQuitLaunch>> {
    scenarios::expect_scenario(process, Scenario::LifecycleBackgroundQuit)?;
    let (protocol, state) = scenarios::parse_core(Scenario::LifecycleBackgroundQuit, "when_idle")?;

    let (trigger, waiter) = mpsc::channel();
    TAIL.install(BackgroundQuitTail {
        protocol: protocol.clone(),
        state: state.clone(),
        cancel_trigger: trigger.clone(),
    })?;

    Ok(LaunchDecision::Run(BackgroundQuitLaunch {
        protocol,
        state,
        dispatch: Mutex::new(Some(BackgroundQuitDispatch { trigger, waiter })),
    }))
}

/// The global every background-quit hook reads: the conformance
/// protocol/state plus the trigger `on_event`'s `Started` handler consumes.
/// Not `Clone` — `dispatch_trigger`'s `Mutex` is read by reference instead —
/// since only `before_primary` ever installs it and only `on_event` ever
/// takes it.
struct BackgroundQuitGlobal {
    protocol: Protocol,
    state: ScenarioState,
    dispatch_trigger: Mutex<Option<mpsc::Sender<BackgroundDispatchSignal>>>,
}

impl Global for BackgroundQuitGlobal {}

/// A background, zero-window application: all startup work — installing the
/// global, spawning the worker, and taking its liveness hold — happens here,
/// in place of the old transactional `start` hook.
fn before_primary(value: &BackgroundQuitLaunch, cx: &mut App) -> anyhow::Result<()> {
    value
        .state
        .emit(&value.protocol, "startup_transaction_started", json!({}));

    let dispatch = lock_or_recover(&value.dispatch).take().expect(
        "the launch parser installs the background-quit dispatch channel before AppShell::run",
    );
    let BackgroundQuitDispatch { trigger, waiter } = dispatch;
    cx.set_global(BackgroundQuitGlobal {
        protocol: value.protocol.clone(),
        state: value.state.clone(),
        dispatch_trigger: Mutex::new(Some(trigger)),
    });

    let hold = cx.hold("lifecycle-background-quit");
    let proxy = cx.app_proxy();
    let protocol = value.protocol.clone();
    let state = value.state.clone();
    let worker = thread::spawn(move || run_background_worker(waiter, proxy, protocol, state, hold));
    WORKER_TAIL.install(worker)
}

fn on_event(event: &AppEvent, cx: &mut App) -> anyhow::Result<()> {
    // `try_global`, not `global`: a framework failure before `before_primary`
    // ran can still dispatch a shutdown event through this hook, and that
    // earlier failure must not compound into a panic here.
    let Some(global) = cx.try_global::<BackgroundQuitGlobal>() else {
        return Ok(());
    };
    observe_app_event(&global.protocol, &global.state, event);
    if matches!(event, AppEvent::Started) {
        let Some(trigger) = lock_or_recover(&global.dispatch_trigger).take() else {
            global.state.record_failure(
                "background dispatch trigger was delivered more than once".to_owned(),
            );
            return Ok(());
        };
        global.state.emit(
            &global.protocol,
            "background_dispatch_triggered",
            json!({"source": "app_started"}),
        );
        if trigger.send(BackgroundDispatchSignal::Start).is_err() {
            global
                .state
                .record_failure("background worker stopped before dispatch trigger".to_owned());
            global.state.emit(
                &global.protocol,
                "background_dispatch_trigger_failed",
                json!({"reason": "worker_disconnected"}),
            );
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BackgroundWorkerReport {
    Accepted,
    Rejected,
    NotTriggered,
}

impl BackgroundWorkerReport {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
            Self::NotTriggered => "not_triggered",
        }
    }
}

fn run_background_worker(
    dispatch_waiter: mpsc::Receiver<BackgroundDispatchSignal>,
    proxy: AppProxy,
    protocol: Protocol,
    state: ScenarioState,
    hold: ShellHold,
) -> BackgroundWorkerReport {
    state.emit(&protocol, "background_worker_started", json!({}));
    match dispatch_waiter.recv() {
        Ok(BackgroundDispatchSignal::Start) => {}
        Ok(BackgroundDispatchSignal::Cancel) => {
            state.emit(
                &protocol,
                "background_dispatch_not_triggered",
                json!({"reason": "run_cancelled_after_panic"}),
            );
            return BackgroundWorkerReport::NotTriggered;
        }
        Err(_) => {
            state.emit(
                &protocol,
                "background_dispatch_not_triggered",
                json!({"reason": "startup_did_not_complete"}),
            );
            return BackgroundWorkerReport::NotTriggered;
        }
    }

    let callback_protocol = protocol.clone();
    let callback_state = state.clone();
    let admission = proxy.dispatch(move |cx| {
        callback_state.emit(
            &callback_protocol,
            "background_dispatch_executed",
            json!({"result": "executed"}),
        );
        callback_state.mark_background_dispatch_executed();
        if !cx.windows().is_empty() {
            callback_state
                .record_failure("background-quit dispatch found native windows".to_owned());
            callback_state.emit(
                &callback_protocol,
                "background_zero_windows_failed",
                json!({"window_count": cx.windows().len()}),
            );
            drop(hold);
            return;
        }
        callback_state.emit(
            &callback_protocol,
            "background_zero_windows_verified",
            json!({"window_count": 0}),
        );
        if callback_state.claim_background_quit() {
            callback_state.emit(
                &callback_protocol,
                "background_hold_released",
                json!({"reason": hold.reason()}),
            );
            drop(hold);
        } else {
            callback_state.record_failure(
                "background-quit dispatch could not release its shell hold".to_owned(),
            );
            drop(hold);
        }
    });

    match admission {
        Ok(()) => {
            state.emit(
                &protocol,
                "background_dispatch_admission",
                json!({"accepted": true, "result": "queued"}),
            );
            BackgroundWorkerReport::Accepted
        }
        Err(error) => {
            state.record_failure(format!(
                "background AppProxy dispatch was rejected: {error}"
            ));
            state.emit(
                &protocol,
                "background_dispatch_admission",
                json!({"accepted": false, "result": "app_closed"}),
            );
            BackgroundWorkerReport::Rejected
        }
    }
}

fn join_background_worker(protocol: &Protocol, state: &ScenarioState) -> anyhow::Result<()> {
    let worker = WORKER_TAIL
        .take()
        .context("background worker was never started")?;
    let report = worker
        .join()
        .map_err(|panic| anyhow::anyhow!("background worker panicked: {}", panic_message(panic)))?;
    state.emit(
        protocol,
        "background_worker_joined",
        json!({"dispatch_admission": report.as_str()}),
    );
    if report != BackgroundWorkerReport::Accepted {
        anyhow::bail!(
            "background-quit worker did not admit its AppProxy dispatch: {}",
            report.as_str()
        );
    }
    Ok(())
}

pub(crate) fn run() -> anyhow::Result<ScenarioOutcome> {
    let (tail, result) = scenarios::recover_tail(&TAIL, catch_run::<LifecycleBackgroundQuitApp>())?;

    // Unblock the worker's dispatch wait unconditionally, whether
    // `AppShell::run` returned or panicked, through `run`'s own independent
    // trigger clone -- never the global's, which a panic mid-run may leave
    // undropped -- before either tail below joins it and writes a terminal
    // record. A panic must never leave the worker thread blocked forever.
    let _ = tail.cancel_trigger.send(BackgroundDispatchSignal::Cancel);

    let result = match result {
        Ok(result) => result,
        Err(panic) => {
            let cleanup = join_background_worker(&tail.protocol, &tail.state);
            return scenarios::finish_panicked_after_cleanup(&tail.protocol, panic, cleanup);
        }
    };

    emit_after_run(&tail.protocol, &result)?;

    let outcome = (|| -> anyhow::Result<ScenarioOutcome> {
        join_background_worker(&tail.protocol, &tail.state)?;
        if let Err(error) = result {
            anyhow::bail!("background-quit scenario returned AppShell error: {error:#}");
        }
        if !tail.state.background_quit_requested() {
            tail.state
                .record_failure("background-quit ended before releasing its shell hold".to_owned());
        }
        if let Some(failure) = tail.state.failure() {
            anyhow::bail!("background-quit scenario failed: {failure}");
        }
        Ok(ScenarioOutcome::Passed)
    })();

    scenarios::finish(&tail.protocol, outcome)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_rejects_a_mismatched_scenario_selection() {
        let process = ProcessLaunch::new(vec!["--scenario".into(), "lifecycle-clean".into()], None);

        assert!(parse(&process).is_err());
    }
}
