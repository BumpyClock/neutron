use crate::{
    ActiveTheme, Disableable, Side, Sizable, Size, StyledExt,
    animation::{reduced_motion, theme_spring_config},
    h_flex,
    text::Text,
    tooltip::Tooltip,
};
use gpui::{
    AnimationExt as _, App, ElementId, InteractiveElement, IntoElement, ParentElement as _,
    RenderOnce, Role, SharedString, SpringAnimation, StatefulInteractiveElement, StyleRefinement,
    Styled, Toggled, Window, div, prelude::FluentBuilder as _, px,
};
use std::rc::Rc;

/// A Switch element that can be toggled on or off.
#[derive(IntoElement)]
pub struct Switch {
    id: ElementId,
    style: StyleRefinement,
    checked: bool,
    disabled: bool,
    label: Option<Text>,
    label_side: Side,
    on_click: Option<Rc<dyn Fn(&bool, &mut Window, &mut App)>>,
    size: Size,
    tooltip: Option<SharedString>,
    tab_index: isize,
    tab_stop: bool,
}

impl Switch {
    /// Create a new Switch element.
    pub fn new(id: impl Into<ElementId>) -> Self {
        let id: ElementId = id.into();
        Self {
            id,
            style: StyleRefinement::default(),
            checked: false,
            disabled: false,
            label: None,
            on_click: None,
            label_side: Side::Right,
            size: Size::Medium,
            tooltip: None,
            tab_index: 0,
            tab_stop: true,
        }
    }

    /// Set the checked state of the switch.
    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    /// Set the label of the switch.
    pub fn label(mut self, label: impl Into<Text>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Add a click handler for the switch.
    pub fn on_click<F>(mut self, handler: F) -> Self
    where
        F: Fn(&bool, &mut Window, &mut App) + 'static,
    {
        self.on_click = Some(Rc::new(handler));
        self
    }

    /// Set tooltip for the switch.
    pub fn tooltip(mut self, tooltip: impl Into<SharedString>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    /// Set the focus traversal index.
    pub fn tab_index(mut self, tab_index: isize) -> Self {
        self.tab_index = tab_index;
        self
    }

    /// Set whether the switch participates in focus traversal.
    pub fn tab_stop(mut self, tab_stop: bool) -> Self {
        self.tab_stop = tab_stop;
        self
    }
}

impl Styled for Switch {
    fn style(&mut self) -> &mut gpui::StyleRefinement {
        &mut self.style
    }
}

impl Sizable for Switch {
    fn with_size(mut self, size: impl Into<Size>) -> Self {
        self.size = size.into();
        self
    }
}

impl Disableable for Switch {
    fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl RenderOnce for Switch {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let checked = self.checked;
        let disabled = self.disabled;
        let on_click = self.on_click.clone();
        let focus_handle = window
            .use_keyed_state((self.id.clone(), "focus"), cx, |_, cx| cx.focus_handle())
            .read(cx)
            .clone();
        let aria_label = self
            .label
            .as_ref()
            .map(|label| label.get_text(cx))
            .or_else(|| self.tooltip.clone());
        let reduced_motion = reduced_motion(cx);
        let spring_config = theme_spring_config(&cx.theme().motion);

        let (bg, toggle_bg) = match checked {
            true => (cx.theme().primary, cx.theme().switch_thumb),
            false => (cx.theme().switch, cx.theme().switch_thumb),
        };

        let (bg, toggle_bg) = if disabled {
            (
                if checked { bg.alpha(0.5) } else { bg },
                toggle_bg.alpha(0.35),
            )
        } else {
            (bg, toggle_bg)
        };

        let (bg_width, bg_height) = match self.size {
            Size::XSmall | Size::Small => (px(28.), px(16.)),
            _ => (px(36.), px(20.)),
        };
        let bar_width = match self.size {
            Size::XSmall | Size::Small => px(12.),
            _ => px(16.),
        };
        let inset = px(2.);
        let radius = if cx.theme().radius >= px(4.) {
            bg_height
        } else {
            cx.theme().radius
        };

        div().refine_style(&self.style).child(
            h_flex()
                .id(self.id.clone())
                .role(Role::Switch)
                .aria_toggled(if checked {
                    Toggled::True
                } else {
                    Toggled::False
                })
                .aria_disabled(disabled)
                .when_some(aria_label, |this, label| this.aria_label(label))
                .when(!disabled, |this| {
                    this.track_focus(
                        &focus_handle
                            .tab_index(self.tab_index)
                            .tab_stop(self.tab_stop),
                    )
                })
                .when(disabled, |this| {
                    this.on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                        cx.stop_propagation();
                    })
                })
                .gap_2()
                .items_start()
                .when(self.label_side.is_left(), |this| this.flex_row_reverse())
                .child(
                    // Switch Bar
                    div()
                        .id(self.id.clone())
                        .w(bg_width)
                        .h(bg_height)
                        .rounded(radius)
                        .flex()
                        .items_center()
                        .border(inset)
                        .border_color(cx.theme().transparent)
                        .bg(bg)
                        .when_some(self.tooltip.clone(), |this, tooltip| {
                            this.tooltip(move |window, cx| {
                                Tooltip::new(tooltip.clone()).build(window, cx)
                            })
                        })
                        .child(
                            // Switch Toggle
                            div()
                                .rounded(radius)
                                .bg(toggle_bg)
                                .shadow_md()
                                .size(bar_width)
                                .debug_selector(|| "switch-thumb".into())
                                .map(|this| {
                                    let max_x = bg_width - bar_width - inset * 2;
                                    let target = if checked { max_x } else { px(0.) };
                                    if reduced_motion {
                                        this.left(target).into_any_element()
                                    } else {
                                        this.with_spring(
                                            "move",
                                            SpringAnimation::new(spring_config)
                                                .to(target)
                                                .with_epsilon(0.01),
                                            |this, position| this.left(position),
                                        )
                                        .into_any_element()
                                    }
                                }),
                        ),
                )
                .when_some(self.label, |this, label| {
                    this.child(div().line_height(bg_height).child(label).map(
                        |this| match self.size {
                            Size::XSmall | Size::Small => this.text_sm(),
                            _ => this.text_base(),
                        },
                    ))
                })
                .when_some(
                    on_click.as_ref().map(|c| c.clone()).filter(|_| !disabled),
                    |this, on_click| {
                        this.on_click(move |_, window, cx| {
                            cx.stop_propagation();
                            on_click(&!checked, window, cx);
                        })
                    },
                ),
        )
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, rc::Rc, time::Duration};

    use gpui::{
        Context, KeyDownEvent, KeyUpEvent, Keystroke, Modifiers, Render,
        StatefulInteractiveElement as _, TestAppContext, VisualTestContext, point,
    };

    use super::*;

    struct SwitchHarness {
        disabled: bool,
        toggles: Rc<Cell<usize>>,
        parent_clicks: Rc<Cell<usize>>,
    }

    struct MotionHarness {
        checked: bool,
    }

    impl Render for MotionHarness {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            Switch::new("motion-switch")
                .checked(self.checked)
                .on_click(cx.listener(|this, checked, _, cx| {
                    this.checked = *checked;
                    cx.notify();
                }))
        }
    }

    impl Render for SwitchHarness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let toggles = self.toggles.clone();
            let parent_clicks = self.parent_clicks.clone();
            div()
                .id("switch-parent")
                .tab_group()
                .size(px(100.))
                .on_click(move |_, _, _| parent_clicks.set(parent_clicks.get() + 1))
                .child(Switch::new("switch").disabled(self.disabled).on_click(
                    move |checked, _, _| {
                        assert!(*checked);
                        toggles.set(toggles.get() + 1);
                    },
                ))
        }
    }

    fn harness(
        cx: &mut TestAppContext,
        disabled: bool,
    ) -> (&mut VisualTestContext, Rc<Cell<usize>>, Rc<Cell<usize>>) {
        cx.update(crate::init);
        let toggles = Rc::new(Cell::new(0));
        let parent_clicks = Rc::new(Cell::new(0));
        let (_, cx) = cx.add_window_view({
            let toggles = toggles.clone();
            let parent_clicks = parent_clicks.clone();
            move |_, _| SwitchHarness {
                disabled,
                toggles,
                parent_clicks,
            }
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        (cx, toggles, parent_clicks)
    }

    fn activate_key(cx: &mut VisualTestContext, key: &str) {
        let keystroke = Keystroke::parse(key).unwrap();
        cx.simulate_event(KeyDownEvent {
            keystroke: keystroke.clone(),
            is_held: false,
            prefer_character_input: false,
        });
        cx.simulate_event(KeyUpEvent { keystroke });
    }

    #[gpui::test]
    fn pointer_activation_fires_once_and_isolates_parent(cx: &mut TestAppContext) {
        let (cx, toggles, parent_clicks) = harness(cx, false);
        cx.simulate_click(point(px(10.), px(10.)), Modifiers::default());

        assert_eq!(toggles.get(), 1);
        assert_eq!(parent_clicks.get(), 0);
    }

    #[gpui::test]
    fn switch_supports_tab_enter_and_space(cx: &mut TestAppContext) {
        let (cx, toggles, _) = harness(cx, false);
        cx.update(|window, cx| window.focus_next(cx));
        cx.update(|window, cx| assert!(window.focused(cx).is_some()));

        activate_key(cx, "enter");
        activate_key(cx, "space");

        assert_eq!(toggles.get(), 2);
    }

    #[gpui::test]
    fn disabled_switch_is_inert_and_blocks_parent(cx: &mut TestAppContext) {
        let (cx, toggles, parent_clicks) = harness(cx, true);
        cx.simulate_click(point(px(10.), px(10.)), Modifiers::default());

        assert_eq!(toggles.get(), 0);
        assert_eq!(parent_clicks.get(), 0);
    }

    #[gpui::test]
    fn switch_spring_preserves_momentum_when_retargeted(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let (view, cx) = cx.add_window_view(|_, _| MotionHarness { checked: false });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let start = cx.debug_bounds("switch-thumb").unwrap().origin.x;

        view.update(cx, |this, cx| {
            this.checked = true;
            cx.notify();
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.executor().advance_clock(Duration::from_millis(20));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let before_retarget = cx.debug_bounds("switch-thumb").unwrap().origin.x;
        assert!(before_retarget > start);

        view.update(cx, |this, cx| {
            this.checked = false;
            cx.notify();
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let at_retarget = cx.debug_bounds("switch-thumb").unwrap().origin.x;
        assert_eq!(at_retarget, before_retarget);
        cx.executor().advance_clock(Duration::from_millis(2));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let after_retarget = cx.debug_bounds("switch-thumb").unwrap().origin.x;

        assert!(
            after_retarget > at_retarget,
            "expected preserved forward momentum, got {at_retarget:?} then {after_retarget:?}"
        );
    }

    #[gpui::test]
    fn switch_snaps_for_each_reduced_motion_source(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let (view, cx) = cx.add_window_view(|_, _| MotionHarness { checked: false });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let unchecked = cx.debug_bounds("switch-thumb").unwrap().origin.x;

        cx.update(|_, cx| cx.set_reduce_motion(true));
        view.update(cx, |this, cx| {
            this.checked = true;
            cx.notify();
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let checked = cx.debug_bounds("switch-thumb").unwrap().origin.x;
        assert!(checked > unchecked);

        cx.update(|_, cx| {
            cx.set_reduce_motion(false);
            crate::global_state::GlobalState::global_mut(cx).set_reduced_motion(true);
        });
        view.update(cx, |this, cx| {
            this.checked = false;
            cx.notify();
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));

        assert_eq!(cx.debug_bounds("switch-thumb").unwrap().origin.x, unchecked);
    }
}
