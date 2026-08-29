use neutron_components_app::gpui::{Action, App, Entity, OwnedMenuItem, Window, actions};
use neutron_components_app::{
    AppDeclaration, AppEvent, Command, CommandId, DesktopApp, ExitPolicy, LaunchDecision,
    LaunchSpec, Menu, MenuBar, MenuKey, ProcessLaunch, Shell as _, Surface, SurfaceKey,
};
use serde::Serialize;
use serde_json::json;

use crate::cli::Scenario;
use crate::native_window::{ConformanceView, build_conformance_view, observe_native_window};
use crate::protocol::Protocol;
use crate::scenarios::{
    self, ConformanceGlobal, Handoff, ScenarioLaunch, ScenarioOutcome, ScenarioState, catch_run,
    finish_normal_run, observe_app_event,
};

const CONFORMANCE_MENU: &str = "Conformance";
const MENU_CHECKED_COMMAND_ID: &str = "conformance.menu-checked";
const MENU_UNCHECKED_COMMAND_ID: &str = "conformance.menu-unchecked";
const MENU_DISABLED_COMMAND_ID: &str = "conformance.menu-disabled";
const MENU_CHECKED_COMMAND_LABEL: &str = "Checked Conformance Command";
const MENU_UNCHECKED_COMMAND_LABEL: &str = "Unchecked Conformance Command";
const MENU_DISABLED_COMMAND_LABEL: &str = "Disabled Conformance Command";

actions!(
    conformance,
    [
        DispatchCheckedMenuCommand,
        DispatchUncheckedMenuCommand,
        DispatchDisabledMenuCommand,
    ]
);

static TAIL: Handoff<ScenarioLaunch> = Handoff::new();

struct MenuCommandApp;

impl DesktopApp for MenuCommandApp {
    fn declaration() -> AppDeclaration {
        let conformance_menu = MenuKey::new(CONFORMANCE_MENU).expect("static menu key is valid");
        AppDeclaration::new(crate::APP_IDENTITY)
            .exit_policy(ExitPolicy::Explicit)
            .on_event(on_event)
            // A single custom menu replaces the whole bar, so About and the
            // theme convention (which both need a standard menu to live in)
            // are dropped rather than left stranded.
            .without_about()
            .without_theme()
            .command(
                Command::app(
                    CommandId::new(MENU_CHECKED_COMMAND_ID),
                    DispatchCheckedMenuCommand,
                    handle_checked,
                )
                .label(MENU_CHECKED_COMMAND_LABEL)
                .checked(menu_checked),
            )
            .command(
                Command::app(
                    CommandId::new(MENU_UNCHECKED_COMMAND_ID),
                    DispatchUncheckedMenuCommand,
                    handle_unchecked,
                )
                .label(MENU_UNCHECKED_COMMAND_LABEL)
                .checked(menu_unchecked),
            )
            .command(
                Command::app(
                    CommandId::new(MENU_DISABLED_COMMAND_ID),
                    DispatchDisabledMenuCommand,
                    handle_disabled,
                )
                .label(MENU_DISABLED_COMMAND_LABEL)
                .checked(menu_unchecked)
                .enabled(menu_disabled),
            )
            .menu_bar(MenuBar::custom(vec![
                Menu::new(conformance_menu, CONFORMANCE_MENU)
                    .command(CommandId::new(MENU_CHECKED_COMMAND_ID))
                    .command(CommandId::new(MENU_UNCHECKED_COMMAND_ID))
                    .command(CommandId::new(MENU_DISABLED_COMMAND_ID)),
            ]))
            .launch(
                LaunchSpec::new(parse)
                    .before_primary(before_primary)
                    .primary_surface(
                        Surface::new(SurfaceKey::primary(), build_conformance_view)
                            .title("Menu Command")
                            .after_open(after_open),
                    ),
            )
    }
}

fn parse(process: &ProcessLaunch) -> anyhow::Result<LaunchDecision<ScenarioLaunch>> {
    scenarios::expect_scenario(process, Scenario::MenuCommand)?;
    let (protocol, state) = scenarios::parse_core(Scenario::MenuCommand, "explicit")?;
    let launch = ScenarioLaunch { protocol, state };
    TAIL.install(launch.clone())?;
    Ok(LaunchDecision::Run(launch))
}

fn before_primary(value: &ScenarioLaunch, cx: &mut App) -> anyhow::Result<()> {
    value
        .state
        .emit(&value.protocol, "startup_transaction_started", json!({}));
    value.state.emit(
        &value.protocol,
        "menu_commands_registered",
        json!({
            "menu": CONFORMANCE_MENU,
            "command_ids": [
                MENU_CHECKED_COMMAND_ID,
                MENU_UNCHECKED_COMMAND_ID,
                MENU_DISABLED_COMMAND_ID,
            ],
        }),
    );
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

fn menu_checked(_: &App) -> bool {
    true
}

fn menu_unchecked(_: &App) -> bool {
    false
}

fn menu_disabled(_: &App) -> bool {
    false
}

fn handle_checked(_: &DispatchCheckedMenuCommand, cx: &mut App) -> anyhow::Result<()> {
    let global = cx.global::<ConformanceGlobal>().clone();
    let callback_count = global.state.record_menu_command_dispatch();
    if callback_count != 1 {
        fail_menu_command(
            cx,
            &global.protocol,
            &global.state,
            "enabled checked menu command was dispatched more than once",
        );
        return Ok(());
    }
    global.state.emit(
        &global.protocol,
        "menu_command_dispatched",
        json!({
            "command_id": MENU_CHECKED_COMMAND_ID,
            "dispatch": "app_action",
            "callback_count": callback_count,
        }),
    );
    global.state.emit(
        &global.protocol,
        "menu_command_verified",
        json!({"registered": true, "dispatched": true}),
    );
    global.state.emit(
        &global.protocol,
        "quit_requested",
        json!({"source": "projected_menu_command"}),
    );
    cx.request_quit();
    Ok(())
}

fn handle_unchecked(_: &DispatchUncheckedMenuCommand, cx: &mut App) -> anyhow::Result<()> {
    let global = cx.global::<ConformanceGlobal>().clone();
    fail_menu_command(
        cx,
        &global.protocol,
        &global.state,
        "unchecked menu command was dispatched unexpectedly",
    );
    Ok(())
}

fn handle_disabled(_: &DispatchDisabledMenuCommand, cx: &mut App) -> anyhow::Result<()> {
    let global = cx.global::<ConformanceGlobal>().clone();
    fail_menu_command(
        cx,
        &global.protocol,
        &global.state,
        "disabled menu command was dispatched unexpectedly",
    );
    Ok(())
}

fn after_open(_content: &Entity<ConformanceView>, window: &mut Window, cx: &mut App) {
    let global = cx.global::<ConformanceGlobal>().clone();
    observe_native_window(
        window,
        cx,
        &global.protocol,
        &global.state,
        "menu-command",
        "Menu Command",
        after_first_presentation,
    );
}

fn after_first_presentation(_window: &mut Window, cx: &mut App) {
    cx.defer(|cx| {
        let global = cx.global::<ConformanceGlobal>().clone();
        match projected_menu_command(cx) {
            Ok(projected) => {
                global.state.emit(
                    &global.protocol,
                    "menu_projection_observed",
                    json!({
                        "projection": "owned_menu_model",
                        "items": projected.items,
                    }),
                );
                cx.dispatch_action(projected.checked_action.as_ref());
            }
            Err(error) => {
                fail_menu_command(
                    cx,
                    &global.protocol,
                    &global.state,
                    &format!("could not obtain projected native menu commands: {error:#}"),
                );
            }
        }
    });
}

fn fail_menu_command(cx: &mut App, protocol: &Protocol, state: &ScenarioState, failure: &str) {
    state.record_failure(failure.to_owned());
    state.emit(protocol, "menu_command_failed", json!({"reason": failure}));
    cx.request_quit();
}

struct ProjectedMenuCommand {
    checked_action: Box<dyn Action>,
    items: Vec<MenuProjectionItem>,
}

#[derive(Serialize)]
struct MenuProjectionItem {
    label: &'static str,
    checked: bool,
    disabled: bool,
}

/// `OwnedMenuItem` exposes checked and disabled state but no independent
/// checkability flag, so the unchecked entry proves only its observed state.
fn projected_menu_command(cx: &App) -> anyhow::Result<ProjectedMenuCommand> {
    let menus = cx
        .get_menus()
        .ok_or_else(|| anyhow::anyhow!("native menu projection was unavailable"))?;
    let menu = menus
        .iter()
        .find(|menu| menu.name == CONFORMANCE_MENU)
        .ok_or_else(|| anyhow::anyhow!("conformance menu was not projected"))?;
    let action_count = menu
        .items
        .iter()
        .filter(|item| matches!(item, OwnedMenuItem::Action { .. }))
        .count();
    if action_count != 3 {
        anyhow::bail!("conformance menu projected {action_count} actions instead of three");
    }

    let expected = [
        (MENU_CHECKED_COMMAND_LABEL, true, false),
        (MENU_UNCHECKED_COMMAND_LABEL, false, false),
        (MENU_DISABLED_COMMAND_LABEL, false, true),
    ];
    let mut checked_action = None;
    let mut items = Vec::with_capacity(expected.len());
    for (label, checked, disabled) in expected {
        let mut matches = menu.items.iter().filter_map(|item| match item {
            OwnedMenuItem::Action {
                name,
                action,
                checked: actual_checked,
                disabled: actual_disabled,
                ..
            } if name == label => Some((action, *actual_checked, *actual_disabled)),
            _ => None,
        });
        let Some((action, actual_checked, actual_disabled)) = matches.next() else {
            anyhow::bail!("conformance menu did not project {label:?}");
        };
        if matches.next().is_some() {
            anyhow::bail!("conformance menu projected {label:?} more than once");
        }
        if actual_checked != checked || actual_disabled != disabled {
            anyhow::bail!(
                "conformance menu projected {label:?} as checked={actual_checked}, disabled={actual_disabled}"
            );
        }
        if label == MENU_CHECKED_COMMAND_LABEL {
            checked_action = Some(action.boxed_clone());
        }
        items.push(MenuProjectionItem {
            label,
            checked: actual_checked,
            disabled: actual_disabled,
        });
    }

    Ok(ProjectedMenuCommand {
        checked_action: checked_action
            .ok_or_else(|| anyhow::anyhow!("checked menu command was not projected"))?,
        items,
    })
}

pub(crate) fn run() -> anyhow::Result<ScenarioOutcome> {
    let (tail, result) = scenarios::recover_tail(&TAIL, catch_run::<MenuCommandApp>())?;

    let result = match result {
        Ok(result) => result,
        Err(panic) => return scenarios::finish_panicked(&tail.protocol, panic),
    };

    if tail.state.menu_command_callback_count() != 1 {
        tail.state.record_failure(format!(
            "enabled checked menu command callback count was {} instead of one",
            tail.state.menu_command_callback_count()
        ));
    }
    finish_normal_run(&tail.protocol, &tail.state, result)
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
