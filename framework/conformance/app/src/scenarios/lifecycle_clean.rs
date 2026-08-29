use neutron_components_app::gpui::{App, Entity, Window};
use neutron_components_app::{
    AppDeclaration, AppEvent, DesktopApp, ExitPolicy, LaunchDecision, LaunchSpec, ProcessLaunch,
    Shell as _, Surface, SurfaceKey,
};
use serde_json::json;

use crate::cli::Scenario;
use crate::native_window::{ConformanceView, build_conformance_view, observe_native_window};
use crate::scenarios::{
    self, ConformanceGlobal, Handoff, ScenarioLaunch, ScenarioOutcome, catch_run,
    finish_normal_run, observe_app_event,
};

/// Recovered by [`run`] after `AppShell::run` returns: this run's canonical
/// `Protocol`/`ScenarioState`, installed by [`parse`] before any platform or
/// GPUI code runs.
static TAIL: Handoff<ScenarioLaunch> = Handoff::new();

struct LifecycleCleanApp;

impl DesktopApp for LifecycleCleanApp {
    fn declaration() -> AppDeclaration {
        AppDeclaration::new(crate::APP_IDENTITY)
            .exit_policy(ExitPolicy::Explicit)
            .on_event(on_event)
            .launch(
                LaunchSpec::new(parse)
                    .before_primary(before_primary)
                    .primary_surface(
                        Surface::new(SurfaceKey::primary(), build_conformance_view)
                            .title("Lifecycle Clean")
                            .after_open(after_open),
                    ),
            )
    }
}

fn parse(process: &ProcessLaunch) -> anyhow::Result<LaunchDecision<ScenarioLaunch>> {
    scenarios::expect_scenario(process, Scenario::LifecycleClean)?;
    let (protocol, state) = scenarios::parse_core(Scenario::LifecycleClean, "explicit")?;
    let launch = ScenarioLaunch { protocol, state };
    TAIL.install(launch.clone())?;
    Ok(LaunchDecision::Run(launch))
}

fn before_primary(value: &ScenarioLaunch, cx: &mut App) -> anyhow::Result<()> {
    value
        .state
        .emit(&value.protocol, "startup_transaction_started", json!({}));
    cx.set_global(ConformanceGlobal {
        protocol: value.protocol.clone(),
        state: value.state.clone(),
    });
    Ok(())
}

fn on_event(event: &AppEvent, cx: &mut App) -> anyhow::Result<()> {
    // `try_global`, not `global`: a framework failure before `before_primary`
    // ran (for example a plugin-init fault) can still dispatch a shutdown
    // event through this hook, and that earlier failure must not compound
    // into a panic here.
    let Some(global) = cx.try_global::<ConformanceGlobal>() else {
        return Ok(());
    };
    observe_app_event(&global.protocol, &global.state, event);
    Ok(())
}

fn after_open(_content: &Entity<ConformanceView>, window: &mut Window, cx: &mut App) {
    let global = cx.global::<ConformanceGlobal>().clone();
    observe_native_window(
        window,
        cx,
        &global.protocol,
        &global.state,
        "main",
        "Lifecycle Clean",
        after_first_presentation,
    );
}

fn after_first_presentation(_window: &mut Window, cx: &mut App) {
    let global = cx.global::<ConformanceGlobal>().clone();
    global.state.mark_first_presentation();
    global.state.emit(
        &global.protocol,
        "quit_requested",
        json!({"source": "first_presentation"}),
    );
    cx.request_quit();
}

pub(crate) fn run() -> anyhow::Result<ScenarioOutcome> {
    let (tail, result) = scenarios::recover_tail(&TAIL, catch_run::<LifecycleCleanApp>())?;

    match result {
        Ok(result) => {
            if !tail.state.first_presentation_observed() {
                tail.state
                    .record_failure("lifecycle-clean ended before first presentation".to_owned());
            }
            finish_normal_run(&tail.protocol, &tail.state, result)
        }
        Err(panic) => scenarios::finish_panicked(&tail.protocol, panic),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_rejects_a_mismatched_scenario_selection() {
        let process = ProcessLaunch::new(vec!["--scenario".into(), "window-cycle".into()], None);

        assert!(parse(&process).is_err());
    }
}
