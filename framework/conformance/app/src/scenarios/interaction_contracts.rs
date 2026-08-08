use std::cell::RefCell;
use std::rc::Rc;

use anyhow::{Context as _, ensure};
use gpui::{
    AccessibleAction, App, AppContext as _, Context, DevicePixels, Entity, EntityInputHandler as _,
    Focusable as _, IntoElement, ParentElement as _, Render, Role, Toggled, Window, px, size,
};
use gpui_component::{
    button::{Button, Toggle},
    input::{Input, InputState, SelectAll},
    v_flex,
};
use gpui_component_app::{AppShell, AppShellExt as _, ExitPolicy};
use serde_json::json;

use super::{ScenarioOutcome, ScenarioState, finish_normal_run, observe_app_event};
use crate::native_window::open_native_window_with_root;
use crate::protocol::Protocol;

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

pub(super) fn run(protocol: Protocol) -> anyhow::Result<ScenarioOutcome> {
    let state = ScenarioState::default();
    let event_protocol = protocol.clone();
    let event_state = state.clone();
    let startup_protocol = protocol.clone();
    let startup_state = state.clone();

    let result = AppShell::builder(crate::APP_IDENTITY)
        .shell_preferences()
        .exit_policy(ExitPolicy::Explicit)
        .on_event(move |event, _cx| {
            observe_app_event(&event_protocol, &event_state, event);
            Ok(())
        })
        .start(move |_, cx| {
            gpui_component::init(cx);
            startup_state.emit(&startup_protocol, "startup_transaction_started", json!({}));
            let fields = Rc::new(RefCell::new(None));
            let fields_for_root = fields.clone();
            let presentation_protocol = startup_protocol.clone();
            let presentation_state = startup_state.clone();

            open_native_window_with_root(
                cx,
                startup_protocol,
                startup_state,
                "interaction-contracts",
                "Interaction Contracts",
                move |window, cx| {
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
                    fields_for_root.replace(Some((first.clone(), second.clone())));
                    cx.new(|_| InteractionFixture { first, second })
                },
                move |window, cx| {
                    let Some((first, second)) = fields.borrow_mut().take() else {
                        fail(
                            window,
                            cx,
                            &presentation_protocol,
                            &presentation_state,
                            "interaction fields were unavailable after presentation",
                        );
                        return;
                    };
                    if let Err(error) = prepare_focus_text(window, cx, &first, &second) {
                        fail(
                            window,
                            cx,
                            &presentation_protocol,
                            &presentation_state,
                            &format!("focus preparation failed: {error:#}"),
                        );
                        return;
                    }

                    window.refresh();
                    window.on_next_frame(move |window, _cx| {
                        let verification_protocol = presentation_protocol.clone();
                        let verification_state = presentation_state.clone();
                        window.on_next_frame(move |window, cx| {
                            window.dispatch_action(Box::new(SelectAll), cx);
                            window.refresh();
                            let selection_protocol = verification_protocol.clone();
                            let selection_state = verification_state.clone();
                            window.on_next_frame(move |window, cx| {
                                if let Err(error) = verify_interactions(
                                    window,
                                    cx,
                                    &selection_protocol,
                                    &first,
                                    &second,
                                ) {
                                    fail(
                                        window,
                                        cx,
                                        &selection_protocol,
                                        &selection_state,
                                        &format!("interaction verification failed: {error:#}"),
                                    );
                                    return;
                                }

                                window.set_a11y_active_for_test(true);
                                window.refresh();
                                let accessibility_protocol = selection_protocol.clone();
                                let accessibility_state = selection_state.clone();
                                window.on_next_frame(move |window, cx| {
                                    if let Err(error) =
                                        verify_accessibility(window, &accessibility_protocol)
                                    {
                                        fail(
                                            window,
                                            cx,
                                            &accessibility_protocol,
                                            &accessibility_state,
                                            &format!(
                                                "accessibility verification failed: {error:#}"
                                            ),
                                        );
                                        return;
                                    }
                                    window.set_a11y_active_for_test(false);
                                    accessibility_state.emit(
                                        &accessibility_protocol,
                                        "quit_requested",
                                        json!({"source": "interaction_contracts_verified"}),
                                    );
                                    cx.request_quit();
                                });
                            });
                        });
                        window.refresh();
                    });
                },
            )?;
            Ok(())
        })
        .run();

    finish_normal_run(protocol, state, result)
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
