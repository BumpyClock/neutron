use anyhow::{Context as _, ensure};
use neutron_components::button::{Button, Toggle};
use neutron_components::input::{Input, InputState, SelectAll};
use neutron_components::v_flex;
use neutron_components_app::gpui::{
    AccessibleAction, App, AppContext as _, Context, DevicePixels, Entity, EntityInputHandler as _,
    Focusable as _, Global, IntoElement, ParentElement as _, Render, Role, Toggled, Window, px,
    size,
};
use neutron_components_app::{
    AppDeclaration, AppEvent, DesktopApp, ExitPolicy, LaunchDecision, LaunchSpec, ProcessLaunch,
    Shell as _, Surface, SurfaceKey,
};
use serde_json::json;

use crate::cli::Scenario;
use crate::protocol::Protocol;
use crate::scenarios::{
    self, ConformanceGlobal, Handoff, ScenarioLaunch, ScenarioOutcome, ScenarioState, catch_run,
    finish_normal_run, observe_app_event,
};

static TAIL: Handoff<ScenarioLaunch> = Handoff::new();

struct InteractionContractsApp;

impl DesktopApp for InteractionContractsApp {
    fn declaration() -> AppDeclaration {
        AppDeclaration::new(crate::APP_IDENTITY)
            .exit_policy(ExitPolicy::Explicit)
            .on_event(on_event)
            .launch(
                LaunchSpec::new(parse)
                    .before_primary(before_primary)
                    .primary_surface(
                        Surface::new(SurfaceKey::primary(), build_fixture)
                            .title("Interaction Contracts")
                            .after_open(after_open),
                    ),
            )
    }
}

fn parse(process: &ProcessLaunch) -> anyhow::Result<LaunchDecision<ScenarioLaunch>> {
    scenarios::expect_scenario(process, Scenario::InteractionContracts)?;
    let (protocol, state) = scenarios::parse_core(Scenario::InteractionContracts, "explicit")?;
    let launch = ScenarioLaunch { protocol, state };
    TAIL.install(launch.clone())?;
    Ok(LaunchDecision::Run(launch))
}

struct InteractionFixture {
    first: Entity<InputState>,
    second: Entity<InputState>,
}

impl Render for InteractionFixture {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .child(Input::new(&self.first).aria_label("Interaction first"))
            .child(Input::new(&self.second).aria_label("Interaction second"))
            .child(
                Button::new("interaction-action")
                    .label("Interaction action")
                    .on_click(|_, _, _| {}),
            )
            .child(
                Toggle::new("interaction-toggle")
                    .label("Interaction toggle")
                    .checked(true)
                    .on_click(|_, _, _| {}),
            )
    }
}

/// The global `after_first_presentation` reads: the conformance protocol/state
/// plus the fixture entity `after_open` observed, so the presentation
/// continuation can reach `first`/`second` without a declaration hook that
/// would need to capture them.
#[derive(Clone)]
struct InteractionGlobal {
    protocol: Protocol,
    state: ScenarioState,
    fixture: Entity<InteractionFixture>,
}

impl Global for InteractionGlobal {}

fn build_fixture(
    _args: &ScenarioLaunch,
    window: &mut Window,
    cx: &mut App,
) -> Entity<InteractionFixture> {
    cx.new(|cx| {
        let first = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(false)
                .default_value("alpha")
        });
        let second = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(false)
                .default_value("second")
        });
        InteractionFixture { first, second }
    })
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
    // ran can still dispatch a shutdown event through this hook, and that
    // earlier failure must not compound into a panic here.
    let Some(global) = cx.try_global::<ConformanceGlobal>() else {
        return Ok(());
    };
    observe_app_event(&global.protocol, &global.state, event);
    Ok(())
}

fn after_open(content: &Entity<InteractionFixture>, window: &mut Window, cx: &mut App) {
    let conformance = cx.global::<ConformanceGlobal>().clone();
    cx.set_global(InteractionGlobal {
        protocol: conformance.protocol.clone(),
        state: conformance.state.clone(),
        fixture: content.clone(),
    });

    crate::native_window::observe_native_window(
        window,
        cx,
        &conformance.protocol,
        &conformance.state,
        "interaction-contracts",
        "Interaction Contracts",
        after_first_presentation,
    );
}

fn after_first_presentation(window: &mut Window, cx: &mut App) {
    let global = cx.global::<InteractionGlobal>().clone();
    let (first, second) = {
        let fixture = global.fixture.read(cx);
        (fixture.first.clone(), fixture.second.clone())
    };

    if let Err(error) = prepare_focus_text(window, cx, &first, &second) {
        fail(
            window,
            cx,
            &global.protocol,
            &global.state,
            &format!("focus preparation failed: {error:#}"),
        );
        return;
    }

    window.refresh();
    window.on_next_frame(move |window, _cx| {
        window.on_next_frame(move |window, cx| {
            window.dispatch_action(Box::new(SelectAll), cx);
            window.refresh();
            window.on_next_frame(move |window, cx| {
                if let Err(error) =
                    verify_interactions(window, cx, &global.protocol, &first, &second)
                {
                    fail(
                        window,
                        cx,
                        &global.protocol,
                        &global.state,
                        &format!("interaction verification failed: {error:#}"),
                    );
                    return;
                }

                window.set_a11y_active_for_test(true);
                window.refresh();
                window.on_next_frame(move |window, cx| {
                    if let Err(error) = verify_accessibility(window, &global.protocol) {
                        fail(
                            window,
                            cx,
                            &global.protocol,
                            &global.state,
                            &format!("accessibility verification failed: {error:#}"),
                        );
                        return;
                    }
                    window.set_a11y_active_for_test(false);
                    global.state.emit(
                        &global.protocol,
                        "quit_requested",
                        json!({"source": "interaction_contracts_verified"}),
                    );
                    cx.request_quit();
                });
            });
        });
    });
}

fn prepare_focus_text(
    window: &mut Window,
    cx: &mut App,
    first: &Entity<InputState>,
    second: &Entity<InputState>,
) -> anyhow::Result<()> {
    first.update(cx, |input, cx| input.focus(window, cx));
    ensure!(
        first.focus_handle(cx).is_focused(window),
        "first input did not receive focus"
    );
    window.focus_next(cx);
    ensure!(
        second.focus_handle(cx).is_focused(window),
        "focus traversal did not reach second input"
    );
    second.update(cx, |input, cx| input.insert("!", window, cx));
    ensure!(
        second.read(cx).value() == "!second",
        "text insertion mismatch"
    );
    Ok(())
}

fn verify_interactions(
    window: &mut Window,
    cx: &mut App,
    protocol: &Protocol,
    first: &Entity<InputState>,
    second: &Entity<InputState>,
) -> anyhow::Result<()> {
    let selected = second.update(cx, |input, cx| input.selected_text_range(false, window, cx));
    ensure!(
        selected.as_ref().map(|selection| selection.range.clone()) == Some(0..7),
        "text selection mismatch: {selected:?}"
    );
    protocol.emit(
        "focus_text_verified",
        json!({
            "activation_order": ["first", "second"],
            "inserted": "!",
            "selection_utf16": [0, 7],
            "value": "!second",
        }),
    )?;

    let composition = first.update(cx, |input, cx| {
        input.replace_and_mark_text_in_range(Some(0..5), "漢", Some(0..1), window, cx);
        let marked = input.marked_text_range(window, cx);
        let selection = input.selected_text_range(false, window, cx);
        let marked_value = input.value();
        input.unmark_text(window, cx);
        (marked, selection, marked_value, input.value())
    });
    ensure!(
        composition.0 == Some(0..1),
        "marked range mismatch: {:?}",
        composition.0
    );
    ensure!(
        composition
            .1
            .as_ref()
            .map(|selection| selection.range.clone())
            == Some(0..1),
        "composition selection mismatch: {:?}",
        composition.1
    );
    ensure!(composition.2 == "漢", "marked value mismatch");
    ensure!(composition.3 == "漢", "committed value mismatch");
    protocol.emit(
        "composition_verified",
        json!({
            "committed_value": "漢",
            "marked_range_utf16": [0, 1],
            "selection_utf16": [0, 1],
            "terminal": "unmark",
        }),
    )?;

    let conversions = [
        (
            1.25,
            size(px(8.), px(12.)),
            size(DevicePixels(10), DevicePixels(15)),
        ),
        (
            1.5,
            size(px(2.), px(4.)),
            size(DevicePixels(3), DevicePixels(6)),
        ),
        (
            2.0,
            size(px(1.5), px(2.5)),
            size(DevicePixels(3), DevicePixels(5)),
        ),
    ];
    for (scale, logical, device) in conversions {
        ensure!(
            logical.to_device_pixels(scale) == device,
            "logical/device scale mismatch"
        );
        ensure!(
            device.to_pixels(scale) == logical,
            "device/logical scale mismatch"
        );
    }
    protocol.emit(
        "scale_verified",
        json!({
            "native_scale_factor": window.scale_factor(),
            "tested_scale_factors": [1.25, 1.5, 2.0],
        }),
    )?;
    Ok(())
}

fn verify_accessibility(window: &Window, protocol: &Protocol) -> anyhow::Result<()> {
    let update = window
        .last_a11y_tree_for_test()
        .context("native window did not submit an accessibility tree")?;
    let second = update
        .nodes
        .iter()
        .find(|(_, node)| {
            node.role() == Role::TextInput && node.label() == Some("Interaction second")
        })
        .context("missing second text-input accessibility node")?;
    ensure!(
        second.1.value() == Some("!second"),
        "accessibility value mismatch"
    );
    ensure!(
        second.1.supports_action(AccessibleAction::Focus),
        "text input did not publish focus action"
    );
    ensure!(
        update.focus == second.0,
        "accessibility focus did not match second input"
    );

    let action = update
        .nodes
        .iter()
        .find(|(_, node)| node.role() == Role::Button && node.label() == Some("Interaction action"))
        .context("missing action button accessibility node")?;
    ensure!(
        action.1.supports_action(AccessibleAction::Click),
        "button did not publish click action"
    );
    let toggle = update
        .nodes
        .iter()
        .find(|(_, node)| node.role() == Role::Switch && node.label() == Some("Interaction toggle"))
        .context("missing toggle accessibility node")?;
    ensure!(
        toggle.1.toggled() == Some(Toggled::True),
        "toggle state mismatch"
    );

    protocol.emit(
        "accessibility_verified",
        json!({
            "button_label": "Interaction action",
            "button_supports_click": true,
            "focused_label": "Interaction second",
            "focused_role": "text_input",
            "focused_supports_focus": true,
            "focused_value": "!second",
            "node_count": update.nodes.len(),
            "published": ["button", "switch", "text_input"],
            "toggle_label": "Interaction toggle",
            "toggle_state": "true",
        }),
    )?;
    Ok(())
}

fn fail(
    _window: &mut Window,
    cx: &mut App,
    protocol: &Protocol,
    state: &ScenarioState,
    failure: &str,
) {
    state.record_failure(failure.to_owned());
    state.emit(
        protocol,
        "interaction_contracts_failed",
        json!({"reason": failure}),
    );
    cx.request_quit();
}

pub(crate) fn run() -> anyhow::Result<ScenarioOutcome> {
    let (tail, result) = scenarios::recover_tail(&TAIL, catch_run::<InteractionContractsApp>())?;

    match result {
        Ok(result) => finish_normal_run(&tail.protocol, &tail.state, result),
        Err(panic) => scenarios::finish_panicked(&tail.protocol, panic),
    }
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
