use std::{cell::RefCell, rc::Rc};

use gpui::{
    AccessibleAction, App, AppContext as _, Context, Entity, Focusable as _, IntoElement,
    ParentElement as _, Render, Role, Styled as _, TestAppContext, Toggled, VisualTestContext,
    Window, accesskit, px,
};

use crate::{
    Disableable as _,
    button::{Button, Toggle},
    checkbox::Checkbox,
    input::{Input, InputState},
    list::ListItem,
    v_flex,
};

struct AccessibilityFixture {
    input: Entity<InputState>,
    disabled_input: Entity<InputState>,
}

impl AccessibilityFixture {
    fn new(window: &mut Window, cx: &mut App) -> Self {
        Self {
            input: cx.new(|cx| {
                InputState::new(window, cx)
                    .multi_line(false)
                    .default_value("query")
            }),
            disabled_input: cx.new(|cx| {
                InputState::new(window, cx)
                    .multi_line(false)
                    .default_value("locked")
            }),
        }
    }
}

impl Render for AccessibilityFixture {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .child(
                Button::new("a11y-save")
                    .label("Save")
                    .on_click(|_, _, _| {}),
            )
            .child(
                Button::new("a11y-delete")
                    .label("Delete")
                    .disabled(true)
                    .on_click(|_, _, _| {}),
            )
            .child(
                Checkbox::new("a11y-sync")
                    .label("Enable sync")
                    .checked(true)
                    .on_click(|_, _, _| {}),
            )
            .child(
                Checkbox::new("a11y-locked-sync")
                    .label("Locked sync")
                    .disabled(true)
                    .on_click(|_, _, _| {}),
            )
            .child(
                Toggle::new("a11y-notifications")
                    .label("Notifications")
                    .checked(true)
                    .on_click(|_, _, _| {}),
            )
            .child(
                Toggle::new("a11y-locked-notifications")
                    .label("Locked notifications")
                    .disabled(true)
                    .on_click(|_, _, _| {}),
            )
            .child(Input::new(&self.input).aria_label("Search"))
            .child(
                Input::new(&self.disabled_input)
                    .aria_label("Locked search")
                    .disabled(true),
            )
            .child(
                ListItem::new("a11y-selected-row")
                    .selected(true)
                    .child("Selected row"),
            )
            .child(
                ListItem::new("a11y-disabled-row")
                    .disabled(true)
                    .child("Locked row"),
            )
    }
}

struct FocusTextFixture {
    first: Entity<InputState>,
    second: Entity<InputState>,
}

impl FocusTextFixture {
    fn new(window: &mut Window, cx: &mut App) -> Self {
        Self {
            first: cx.new(|cx| InputState::new(window, cx).default_value("first")),
            second: cx.new(|cx| InputState::new(window, cx).default_value("second")),
        }
    }
}

impl Render for FocusTextFixture {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .child(Input::new(&self.first).aria_label("First field"))
            .child(Input::new(&self.second).aria_label("Second field"))
    }
}

struct ScaleFixture;

impl Render for ScaleFixture {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        v_flex().items_start().child(
            Button::new("scale-target")
                // Preserve the fixed layout box while giving the node an accessible name.
                .tooltip("Scale target")
                .w(px(7.))
                .h(px(9.))
                .min_w_0()
                .min_h_0()
                .overflow_hidden(),
        )
    }
}

fn node_with_label<'a>(
    update: &'a accesskit::TreeUpdate,
    role: Role,
    label: &str,
) -> (accesskit::NodeId, &'a accesskit::Node) {
    update
        .nodes
        .iter()
        .find_map(|(id, node)| {
            (node.role() == role && node.label() == Some(label)).then_some((*id, node))
        })
        .unwrap_or_else(|| panic!("missing {role:?} node labeled {label:?}"))
}

#[gpui::test]
fn stage1_contract_focus_traversal_reaches_text_target_before_insertion(cx: &mut TestAppContext) {
    let fields = Rc::new(RefCell::new(None));
    let fields_for_root = fields.clone();
    let window = cx.update(|cx| {
        crate::init(cx);
        cx.open_window(Default::default(), |window, cx| {
            let fixture = FocusTextFixture::new(window, cx);
            fields_for_root.replace(Some((fixture.first.clone(), fixture.second.clone())));
            let fixture = cx.new(|_| fixture);
            cx.new(|cx| crate::Root::new(fixture, window, cx))
        })
        .unwrap()
    });
    let (first, second) = fields.borrow_mut().take().unwrap();
    let mut visual_cx = VisualTestContext::from_window(window.into(), cx);

    visual_cx.update(|window, cx| {
        window.draw(cx).clear();
        first.update(cx, |input, cx| input.focus(window, cx));
        assert!(first.focus_handle(cx).is_focused(window));

        window.focus_next(cx);
        assert!(second.focus_handle(cx).is_focused(window));
        second.update(cx, |input, cx| input.insert("!", window, cx));
        assert_eq!(second.read(cx).value(), "!second");
    });
}

#[gpui::test]
fn stage1_contract_representative_components_project_accessibility_tree(cx: &mut TestAppContext) {
    let input_slot = Rc::new(RefCell::new(None));
    let input_for_root = input_slot.clone();
    let window = cx.update(|cx| {
        crate::init(cx);
        cx.open_window(Default::default(), |window, cx| {
            let fixture = AccessibilityFixture::new(window, cx);
            input_for_root.replace(Some(fixture.input.clone()));
            let fixture = cx.new(|_| fixture);
            cx.new(|cx| crate::Root::new(fixture, window, cx))
        })
        .unwrap()
    });
    let input = input_slot.borrow_mut().take().unwrap();
    let mut visual_cx = VisualTestContext::from_window(window.into(), cx);

    let update = visual_cx.update(|window, cx| {
        input.update(cx, |input, cx| input.focus(window, cx));
        window.set_a11y_active_for_test(true);
        window.draw(cx).clear();
        let update = window
            .last_a11y_tree_for_test()
            .cloned()
            .expect("accessibility tree should be captured after drawing");
        window.set_a11y_active_for_test(false);
        update
    });

    let (_, save) = node_with_label(&update, Role::Button, "Save");
    assert!(save.supports_action(AccessibleAction::Click));
    assert!(save.supports_action(AccessibleAction::Focus));

    let (_, delete) = node_with_label(&update, Role::Button, "Delete");
    assert!(delete.is_disabled());
    assert!(!delete.supports_action(AccessibleAction::Click));
    assert!(!delete.supports_action(AccessibleAction::Focus));

    let (_, sync) = node_with_label(&update, Role::CheckBox, "Enable sync");
    assert_eq!(sync.toggled(), Some(Toggled::True));
    assert!(sync.supports_action(AccessibleAction::Click));
    assert!(sync.supports_action(AccessibleAction::Focus));

    let (_, locked_sync) = node_with_label(&update, Role::CheckBox, "Locked sync");
    assert!(locked_sync.is_disabled());
    assert!(!locked_sync.supports_action(AccessibleAction::Click));
    assert!(!locked_sync.supports_action(AccessibleAction::Focus));

    let (_, notifications) = node_with_label(&update, Role::Switch, "Notifications");
    assert_eq!(notifications.toggled(), Some(Toggled::True));
    assert!(notifications.supports_action(AccessibleAction::Click));
    assert!(notifications.supports_action(AccessibleAction::Focus));

    let (_, locked_notifications) = node_with_label(&update, Role::Switch, "Locked notifications");
    assert!(locked_notifications.is_disabled());
    assert!(!locked_notifications.supports_action(AccessibleAction::Click));
    assert!(!locked_notifications.supports_action(AccessibleAction::Focus));

    let (search_id, search) = node_with_label(&update, Role::TextInput, "Search");
    assert_eq!(search.value(), Some("query"));
    assert!(search.supports_action(AccessibleAction::Focus));
    assert_eq!(update.focus, search_id);

    let (_, locked_search) = node_with_label(&update, Role::TextInput, "Locked search");
    assert!(locked_search.is_disabled());
    assert!(!locked_search.supports_action(AccessibleAction::Focus));

    let list_items = update
        .nodes
        .iter()
        .filter_map(|(_, node)| (node.role() == Role::ListBoxOption).then_some(node))
        .collect::<Vec<_>>();
    assert!(
        list_items
            .iter()
            .any(|node| node.is_selected() == Some(true))
    );
    assert!(list_items.iter().any(|node| node.is_disabled()));
}

#[gpui::test]
fn stage1_contract_component_bounds_round_to_device_pixels_at_common_scales(
    cx: &mut TestAppContext,
) {
    let window = cx.update(|cx| {
        crate::init(cx);
        cx.open_window(Default::default(), |window, cx| {
            let fixture = cx.new(|_| ScaleFixture);
            cx.new(|cx| crate::Root::new(fixture, window, cx))
        })
        .unwrap()
    });
    let mut visual_cx = VisualTestContext::from_window(window.into(), cx);

    visual_cx.simulate_scale_factor(1.0);
    let update = visual_cx.update(|window, cx| {
        window.set_a11y_active_for_test(true);
        window.draw(cx).clear();
        let update = window
            .last_a11y_tree_for_test()
            .cloned()
            .expect("accessibility tree should be captured after drawing");
        window.set_a11y_active_for_test(false);
        update
    });
    let (_, target) = node_with_label(&update, Role::Button, "Scale target");
    let bounds = target
        .bounds()
        .expect("button should have accessibility bounds");
    assert_eq!(
        (bounds.x0, bounds.y0, bounds.x1, bounds.y1),
        (0.0, 0.0, 7.0, 9.0)
    );

    for (scale_factor, width, height) in [(1.25, 9.0, 11.0), (1.5, 11.0, 14.0), (2.0, 14.0, 18.0)] {
        visual_cx.simulate_scale_factor(scale_factor);
        let update = visual_cx.update(|window, cx| {
            window.set_a11y_active_for_test(true);
            window.draw(cx).clear();
            let update = window
                .last_a11y_tree_for_test()
                .cloned()
                .expect("accessibility tree should be captured after drawing");
            window.set_a11y_active_for_test(false);
            update
        });
        let (_, target) = node_with_label(&update, Role::Button, "Scale target");
        let bounds = target
            .bounds()
            .expect("button should have accessibility bounds");
        assert_eq!(
            (bounds.x0, bounds.y0, bounds.x1, bounds.y1),
            (0.0, 0.0, width, height),
            "scale factor {scale_factor}"
        );
    }
}
