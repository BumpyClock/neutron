use neutron_components_app::gpui::App;
use neutron_components_app::{
    AppDeclaration, AppEvent, AppShellError, DesktopApp, ExitPolicy, LaunchDecision, LaunchSpec,
    ProcessLaunch,
};
use serde_json::json;

use crate::cli::Scenario;
use crate::scenarios::{
    self, ConformanceGlobal, Handoff, ScenarioLaunch, ScenarioOutcome, catch_run, emit_after_run,
    observe_app_event,
};

static TAIL: Handoff<ScenarioLaunch> = Handoff::new();

struct LifecycleStartupFailureApp;

impl DesktopApp for LifecycleStartupFailureApp {
    fn declaration() -> AppDeclaration {
        AppDeclaration::new(crate::APP_IDENTITY)
            .exit_policy(ExitPolicy::Explicit)
            .on_event(on_event)
            .launch(LaunchSpec::new(parse).before_primary(before_primary))
    }
}

fn parse(process: &ProcessLaunch) -> anyhow::Result<LaunchDecision<ScenarioLaunch>> {
    scenarios::expect_scenario(process, Scenario::LifecycleStartupFailure)?;
    let (protocol, state) = scenarios::parse_core(Scenario::LifecycleStartupFailure, "explicit")?;
    let launch = ScenarioLaunch { protocol, state };
    TAIL.install(launch.clone())?;
    Ok(LaunchDecision::Run(launch))
}

/// The scenario has no primary surface: the failure is triggered here, in the
/// launch-specific hook that runs immediately before the (never-created)
/// primary surface, mirroring the old transactional `start` failure.
fn before_primary(value: &ScenarioLaunch, cx: &mut App) -> anyhow::Result<()> {
    cx.set_global(ConformanceGlobal {
        protocol: value.protocol.clone(),
        state: value.state.clone(),
    });
    value.state.emit(
        &value.protocol,
        "startup_failure_triggered",
        json!({"source": "transactional_start"}),
    );
    anyhow::bail!("intentional lifecycle startup failure")
}

fn on_event(event: &AppEvent, cx: &mut App) -> anyhow::Result<()> {
    // `try_global`, not `global`: a framework failure before `before_primary`
    // ran can still dispatch a shutdown event through this hook, and that
    // earlier failure must not compound into a panic here.
    let Some(global) = cx.try_global::<ConformanceGlobal>() else {
        return Ok(());
    };
    observe_app_event(&global.protocol, &global.state, event);
    Ok(())
}

pub(crate) fn run() -> anyhow::Result<ScenarioOutcome> {
    let (tail, result) = scenarios::recover_tail(&TAIL, catch_run::<LifecycleStartupFailureApp>())?;

    let result = match result {
        Ok(result) => result,
        Err(panic) => return scenarios::finish_panicked(&tail.protocol, panic),
    };

    emit_after_run(&tail.protocol, &result)?;
    let outcome = match &result {
        Err(AppShellError::Startup(_)) => {
            if let Some(failure) = tail.state.failure() {
                Err(anyhow::anyhow!(
                    "startup-failure protocol failed: {failure}"
                ))
            } else {
                Ok(ScenarioOutcome::ExpectedStartupFailure)
            }
        }
        Ok(()) => Err(anyhow::anyhow!(
            "startup-failure scenario unexpectedly returned success"
        )),
        Err(error) => Err(anyhow::anyhow!(
            "startup-failure scenario returned unexpected error: {error:#}"
        )),
    };
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
