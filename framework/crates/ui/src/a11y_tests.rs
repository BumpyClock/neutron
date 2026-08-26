use std::{cell::RefCell, rc::Rc};

use gpui::{
    AccessibleAction, App, AppContext as _, Context, Entity, Focusable as _, IntoElement,
    ParentElement as _, Render, Role, Styled as _, TestAppContext, Toggled, VisualTestContext,
    Window, accesskit, px,
};

use crate::{
    Disableable as _,
    button::{Button, ButtonVariants as _, Toggle},
    checkbox::Checkbox,
    input::{Input, InputState},
    link::Link,
    list::ListItem,
    radio::{Radio, RadioGroup},
    searchable_list::SearchableListItemElement,
    switch::Switch,
    v_flex,
};

struct AccessibilityFixture {
    input: Entity<InputState>,
    disabled_input: Entity<InputState>,
    masked_input: Entity<InputState>,
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
            masked_input: cx.new(|cx| {
                InputState::new(window, cx)
                    .masked(true)
                    .default_value("secret")
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
            .child(Button::new("a11y-pin").label("Pin").toggled(true))
            .child(Button::new("a11y-docs").label("Documentation").link())
            .child(
                Link::new("a11y-standalone-link")
                    .aria_label("Standalone docs")
                    .child("Standalone docs"),
            )
            .child(
                Link::new("a11y-disabled-link")
                    .aria_label("Disabled docs")
                    .disabled(true)
                    .child("Disabled docs"),
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
            .child(
                Switch::new("a11y-wifi")
                    .label("Wi-Fi")
                    .checked(true)
                    .on_click(|_, _, _| {}),
            )
            .child(
                Switch::new("a11y-locked-wifi")
                    .label("Locked Wi-Fi")
                    .disabled(true)
                    .on_click(|_, _, _| {}),
            )
            .child(
                Radio::new("a11y-radio")
                    .label("Stable channel")
                    .checked(true),
            )
            .child(
                Radio::new("a11y-disabled-radio")
                    .label("Locked channel")
                    .disabled(true),
            )
            .child(
                RadioGroup::vertical("a11y-radio-group")
                    .child(Radio::new("a11y-group-open").label("Group open"))
                    .child(
                        Radio::new("a11y-group-locked")
                            .label("Group locked")
                            .disabled(true),
                    ),
            )
            .child(Input::new(&self.input).aria_label("Search"))
            .child(
                Input::new(&self.disabled_input)
                    .aria_label("Locked search")
                    .disabled(true),
            )
            .child(Input::new(&self.masked_input).aria_label("Password"))
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
            .child(
                SearchableListItemElement::new(0)
                    .checked(true)
                    .disabled(true)
                    .child("Locked option"),
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
        window.draw(cx).clear(cx);
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
            input_for_root.replace(Some((fixture.input.clone(), fixture.masked_input.clone())));
            let fixture = cx.new(|_| fixture);
            cx.new(|cx| crate::Root::new(fixture, window, cx))
        })
        .unwrap()
    });
    let (input, masked_input) = input_slot.borrow_mut().take().unwrap();
    let mut visual_cx = VisualTestContext::from_window(window.into(), cx);

    let update = visual_cx.update(|window, cx| {
        input.update(cx, |input, cx| input.focus(window, cx));
        window.set_a11y_active_for_test(true);
        window.draw(cx).clear(cx);
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

    let (_, pin) = node_with_label(&update, Role::Button, "Pin");
    assert_eq!(pin.toggled(), Some(Toggled::True));

    let (_, docs) = node_with_label(&update, Role::Link, "Documentation");
    assert_eq!(docs.toggled(), None);

    let (_, standalone_docs) = node_with_label(&update, Role::Link, "Standalone docs");
    assert!(standalone_docs.supports_action(AccessibleAction::Click));
    assert!(standalone_docs.supports_action(AccessibleAction::Focus));

    let (_, disabled_docs) = node_with_label(&update, Role::Link, "Disabled docs");
    assert!(disabled_docs.is_disabled());
    assert!(!disabled_docs.supports_action(AccessibleAction::Click));
    assert!(!disabled_docs.supports_action(AccessibleAction::Focus));

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

    let (_, wifi) = node_with_label(&update, Role::Switch, "Wi-Fi");
    assert_eq!(wifi.toggled(), Some(Toggled::True));
    assert!(wifi.supports_action(AccessibleAction::Click));
    assert!(wifi.supports_action(AccessibleAction::Focus));

    let (_, locked_wifi) = node_with_label(&update, Role::Switch, "Locked Wi-Fi");
    assert_eq!(locked_wifi.toggled(), Some(Toggled::False));
    assert!(locked_wifi.is_disabled());
    assert!(!locked_wifi.supports_action(AccessibleAction::Click));
    assert!(!locked_wifi.supports_action(AccessibleAction::Focus));

    let (_, radio) = node_with_label(&update, Role::RadioButton, "Stable channel");
    assert_eq!(radio.toggled(), Some(Toggled::True));
    assert!(radio.supports_action(AccessibleAction::Click));
    assert!(radio.supports_action(AccessibleAction::Focus));

    let (_, disabled_radio) = node_with_label(&update, Role::RadioButton, "Locked channel");
    assert_eq!(disabled_radio.toggled(), Some(Toggled::False));
    assert!(disabled_radio.is_disabled());
    assert!(!disabled_radio.supports_action(AccessibleAction::Click));
    assert!(!disabled_radio.supports_action(AccessibleAction::Focus));

    assert!(
        update
            .nodes
            .iter()
            .any(|(_, node)| node.role() == Role::RadioGroup)
    );
    let (_, group_open) = node_with_label(&update, Role::RadioButton, "Group open");
    assert!(!group_open.is_disabled());
    let (_, group_locked) = node_with_label(&update, Role::RadioButton, "Group locked");
    assert!(group_locked.is_disabled());

    let (search_id, search) = node_with_label(&update, Role::TextInput, "Search");
    assert_eq!(search.value(), Some("query"));
    assert!(search.supports_action(AccessibleAction::Focus));
    assert!(search.supports_action(AccessibleAction::SetValue));
    assert_eq!(update.focus, search_id);

    let (_, locked_search) = node_with_label(&update, Role::TextInput, "Locked search");
    assert!(locked_search.is_disabled());
    assert!(!locked_search.supports_action(AccessibleAction::Focus));
    assert!(!locked_search.supports_action(AccessibleAction::SetValue));

    let (password_id, password) = node_with_label(&update, Role::TextInput, "Password");
    assert_eq!(password.value(), None);
    assert!(password.supports_action(AccessibleAction::SetValue));

    visual_cx.update(|window, cx| {
        window.handle_a11y_action_for_test(
            accesskit::ActionRequest {
                action: AccessibleAction::SetValue,
                target_tree: accesskit::TreeId::ROOT,
                target_node: search_id,
                data: Some(accesskit::ActionData::Value("updated".into())),
            },
            cx,
        );
        window.handle_a11y_action_for_test(
            accesskit::ActionRequest {
                action: AccessibleAction::SetValue,
                target_tree: accesskit::TreeId::ROOT,
                target_node: password_id,
                data: Some(accesskit::ActionData::Value("new secret".into())),
            },
            cx,
        );
        window.handle_a11y_action_for_test(
            accesskit::ActionRequest {
                action: AccessibleAction::SetValue,
                target_tree: accesskit::TreeId::ROOT,
                target_node: search_id,
                data: Some(accesskit::ActionData::NumericValue(42.)),
            },
            cx,
        );
    });
    assert_eq!(
        input.read_with(&visual_cx, |input, _| input.value()),
        "updated"
    );
    assert_eq!(
        masked_input.read_with(&visual_cx, |input, _| input.value()),
        "new secret"
    );

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

    assert!(
        list_items
            .iter()
            .any(|node| node.is_selected() == Some(true) && node.is_disabled())
    );
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
        window.draw(cx).clear(cx);
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
            window.draw(cx).clear(cx);
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
