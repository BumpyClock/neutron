use std::sync::{Arc, Mutex};

use neutron_components_app::gpui::{App, Entity, Global, Window};
use neutron_components_app::{
    AppDeclaration, AppEvent, DesktopApp, ExitPolicy, LaunchDecision, LaunchSpec, ProcessLaunch,
    Shell as _, Surface, SurfaceKey,
};
use serde_json::json;

use crate::cli::Scenario;
use crate::native_window::{ConformanceView, build_conformance_view, observe_native_window};
use crate::protocol::Protocol;
use crate::scenarios::{
    self, Handoff, ScenarioLaunch, ScenarioOutcome, ScenarioState, catch_run, finish_normal_run,
    lock_or_recover, observe_app_event,
};

const RECREATED_KEY: SurfaceKey<ConformanceView, ()> = SurfaceKey::new("window-cycle-recreated");

static TAIL: Handoff<ScenarioLaunch> = Handoff::new();
/// The cycle state must be readable after `AppShell::run` returns (to check
/// completion): installed by `before_primary`, taken by [`run`].
static CYCLE_TAIL: Handoff<WindowCycleState> = Handoff::new();

struct WindowCycleApp;

impl DesktopApp for WindowCycleApp {
    fn declaration() -> AppDeclaration {
        AppDeclaration::new(crate::APP_IDENTITY)
            .exit_policy(ExitPolicy::Explicit)
            .on_event(on_event)
            .surface(
                Surface::new(RECREATED_KEY, build_conformance_view)
                    .title("Window Cycle Recreated")
                    .after_open(after_open_recreated),
            )
            .launch(
                LaunchSpec::new(parse)
                    .before_primary(before_primary)
                    .primary_surface(
                        Surface::new(SurfaceKey::primary(), build_conformance_view)
                            .title("Window Cycle Initial")
                            .after_open(after_open_initial),
                    ),
            )
    }
}

fn parse(process: &ProcessLaunch) -> anyhow::Result<LaunchDecision<ScenarioLaunch>> {
    scenarios::expect_scenario(process, Scenario::WindowCycle)?;
    let (protocol, state) = scenarios::parse_core(Scenario::WindowCycle, "explicit")?;
    let launch = ScenarioLaunch { protocol, state };
    TAIL.install(launch.clone())?;
    Ok(LaunchDecision::Run(launch))
}

/// The global every window-cycle hook reads: the conformance protocol/state
/// plus this scenario's own cycle-phase tracker.
#[derive(Clone)]
struct WindowCycleGlobal {
    protocol: Protocol,
    state: ScenarioState,
    cycle: WindowCycleState,
}

impl Global for WindowCycleGlobal {}

fn before_primary(value: &ScenarioLaunch, cx: &mut App) -> anyhow::Result<()> {
    value
        .state
        .emit(&value.protocol, "startup_transaction_started", json!({}));
    let cycle = WindowCycleState::default();
    CYCLE_TAIL.install(cycle.clone())?;
    cx.set_global(WindowCycleGlobal {
        protocol: value.protocol.clone(),
        state: value.state.clone(),
        cycle,
    });
    Ok(())
}

fn on_event(event: &AppEvent, cx: &mut App) -> anyhow::Result<()> {
    // `try_global`, not `global`: a framework failure before `before_primary`
    // ran can still dispatch a shutdown event through this hook, and that
    // earlier failure must not compound into a panic here.
    let Some(global) = cx.try_global::<WindowCycleGlobal>().cloned() else {
        return Ok(());
    };
    observe_app_event(&global.protocol, &global.state, event);
    if matches!(event, AppEvent::LastWindowClosed) {
        match global.cycle.last_window_closed() {
            Ok(true) => {
                if !cx.windows().is_empty() {
                    fail_window_cycle(
                        cx,
                        &global.protocol,
                        &global.state,
                        "last-window lifecycle event fired before native windows closed",
                    );
                    return Ok(());
                }
                global.state.emit(
                    &global.protocol,
                    "window_closed",
                    json!({"generation": 1, "source": "last_window_closed"}),
                );
                global.state.emit(
                    &global.protocol,
                    "explicit_hold_verified",
                    json!({"window_count": 0}),
                );
                cx.defer(move |cx| open_window_cycle_replacement(cx));
            }
            Ok(false) => {}
            Err(error) => fail_window_cycle(cx, &global.protocol, &global.state, &error),
        }
    }
    Ok(())
}

fn open_window_cycle_replacement(cx: &mut App) {
    let global = cx.global::<WindowCycleGlobal>().clone();
    match cx.open_surface(RECREATED_KEY, &()) {
        Ok(_) => {
            if let Err(error) = global.cycle.recreated_window_opened() {
                fail_window_cycle(cx, &global.protocol, &global.state, &error);
                return;
            }
            global.state.emit(
                &global.protocol,
                "window_recreated",
                json!({"generation": 2, "key": "window-cycle-recreated"}),
            );
        }
        Err(error) => fail_window_cycle(
            cx,
            &global.protocol,
            &global.state,
            &format!("could not recreate native window: {error:#}"),
        ),
    }
}

fn after_open_initial(_content: &Entity<ConformanceView>, window: &mut Window, cx: &mut App) {
    let global = cx.global::<WindowCycleGlobal>().clone();
    observe_native_window(
        window,
        cx,
        &global.protocol,
        &global.state,
        "window-cycle-initial",
        "Window Cycle Initial",
        after_first_presentation_initial,
    );
}

fn after_first_presentation_initial(window: &mut Window, cx: &mut App) {
    let global = cx.global::<WindowCycleGlobal>().clone();
    if let Err(error) = global.cycle.first_window_presented() {
        fail_window_cycle(cx, &global.protocol, &global.state, &error);
        return;
    }
    global.state.emit(
        &global.protocol,
        "window_close_requested",
        json!({"generation": 1}),
    );
    let handle = window.window_handle();
    cx.defer(move |cx| {
        if let Err(error) = handle.update(cx, |_, window, _| window.remove_window()) {
            let global = cx.global::<WindowCycleGlobal>().clone();
            fail_window_cycle(
                cx,
                &global.protocol,
                &global.state,
                &format!("could not close initial native window: {error:?}"),
            );
        }
    });
}

fn after_open_recreated(_content: &Entity<ConformanceView>, window: &mut Window, cx: &mut App) {
    let global = cx.global::<WindowCycleGlobal>().clone();
    observe_native_window(
        window,
        cx,
        &global.protocol,
        &global.state,
        "window-cycle-recreated",
        "Window Cycle Recreated",
        after_first_presentation_recreated,
    );
}

fn after_first_presentation_recreated(_window: &mut Window, cx: &mut App) {
    let global = cx.global::<WindowCycleGlobal>().clone();
    if let Err(error) = global.cycle.recreated_window_presented() {
        fail_window_cycle(cx, &global.protocol, &global.state, &error);
        return;
    }
    global.state.emit(
        &global.protocol,
        "window_cycle_verified",
        json!({
            "key": "window-cycle",
            "opened": 2,
            "presentations": 2,
            "closed": 1,
            "zero_windows": true,
        }),
    );
    global.state.emit(
        &global.protocol,
        "quit_requested",
        json!({"source": "window_cycle_recreated_presentation"}),
    );
    cx.request_quit();
}

fn fail_window_cycle(cx: &mut App, protocol: &Protocol, state: &ScenarioState, failure: &str) {
    state.record_failure(failure.to_owned());
    state.emit(protocol, "window_cycle_failed", json!({"reason": failure}));
    cx.request_quit();
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WindowCyclePhase {
    AwaitingInitialPresentation,
    AwaitingLastWindowClosed,
    Recreating,
    AwaitingRecreatedPresentation,
    Quitting,
}

#[derive(Clone)]
struct WindowCycleState {
    phase: Arc<Mutex<WindowCyclePhase>>,
}

impl Default for WindowCycleState {
    fn default() -> Self {
        Self {
            phase: Arc::new(Mutex::new(WindowCyclePhase::AwaitingInitialPresentation)),
        }
    }
}

impl WindowCycleState {
    fn first_window_presented(&self) -> Result<(), String> {
        self.transition(
            WindowCyclePhase::AwaitingInitialPresentation,
            WindowCyclePhase::AwaitingLastWindowClosed,
            "initial window presentation",
        )
    }

    fn last_window_closed(&self) -> Result<bool, String> {
        let mut phase = lock_or_recover(&self.phase);
        match *phase {
            WindowCyclePhase::AwaitingLastWindowClosed => {
                *phase = WindowCyclePhase::Recreating;
                Ok(true)
            }
            WindowCyclePhase::Quitting => Ok(false),
            actual => Err(format!(
                "last-window lifecycle event occurred during unexpected phase {actual:?}"
            )),
        }
    }

    fn recreated_window_opened(&self) -> Result<(), String> {
        self.transition(
            WindowCyclePhase::Recreating,
            WindowCyclePhase::AwaitingRecreatedPresentation,
            "recreated window open",
        )
    }

    fn recreated_window_presented(&self) -> Result<(), String> {
        self.transition(
            WindowCyclePhase::AwaitingRecreatedPresentation,
            WindowCyclePhase::Quitting,
            "recreated window presentation",
        )
    }

    fn is_complete(&self) -> bool {
        *lock_or_recover(&self.phase) == WindowCyclePhase::Quitting
    }

    fn transition(
        &self,
        expected: WindowCyclePhase,
        next: WindowCyclePhase,
        operation: &str,
    ) -> Result<(), String> {
        let mut phase = lock_or_recover(&self.phase);
        if *phase != expected {
            return Err(format!(
                "{operation} occurred during unexpected phase {phase:?}"
            ));
        }
        *phase = next;
        Ok(())
    }
}

pub(crate) fn run() -> anyhow::Result<ScenarioOutcome> {
    let (tail, result) = scenarios::recover_tail(&TAIL, catch_run::<WindowCycleApp>())?;

    let result = match result {
        Ok(result) => result,
        Err(panic) => return scenarios::finish_panicked(&tail.protocol, panic),
    };

    let complete = CYCLE_TAIL
        .take()
        .ok()
        .map(|cycle| cycle.is_complete())
        .unwrap_or(false);
    if !complete {
        tail.state
            .record_failure("window-cycle ended before verification".to_owned());
    }
    finish_normal_run(&tail.protocol, &tail.state, result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_cycle_requires_last_window_closed_before_recreation() {
        let cycle = WindowCycleState::default();

        assert!(!cycle.is_complete());
        assert!(cycle.last_window_closed().is_err());
        cycle
            .first_window_presented()
            .expect("initial presentation should advance the cycle");
        assert!(
            cycle
                .last_window_closed()
                .expect("last-window lifecycle event should schedule recreation")
        );
        cycle
            .recreated_window_opened()
            .expect("recreated window should open after the lifecycle event");
        cycle
            .recreated_window_presented()
            .expect("recreated presentation should complete the cycle");
        assert!(cycle.is_complete());
        assert!(
            !cycle
                .last_window_closed()
                .expect("shutdown-driven close should not recreate another window")
        );
    }

    #[test]
    fn parse_rejects_a_mismatched_scenario_selection() {
        let process = ProcessLaunch::new(vec!["--scenario".into(), "lifecycle-clean".into()], None);

        assert!(parse(&process).is_err());
    }
}
