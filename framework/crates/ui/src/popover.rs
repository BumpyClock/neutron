use gpui::{
    AnyElement, App, Bounds, Context, Deferred, DismissEvent, Div, ElementId, EventEmitter,
    FocusHandle, Focusable, Half, InteractiveElement as _, IntoElement, KeyBinding, MouseButton,
    ParentElement, Pixels, Point, Render, RenderOnce, SharedString, Stateful, StyleRefinement,
    Styled, Subscription, Window, deferred, div, prelude::FluentBuilder as _, px,
};
use std::rc::Rc;

use crate::{
    ActiveTheme, Anchor, ElementExt, FlyoutTokens, Selectable, StyledExt as _, SurfaceContext,
    SurfacePreset,
    actions::Cancel,
    anchored,
    animation::{FlyoutSlide, PresenceOptions, flyout_motion, flyout_presence},
    flyout_primary_foreground, v_flex,
};

const CONTEXT: &str = "Popover";
pub(crate) fn init(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("escape", Cancel, Some(CONTEXT))])
}

/// A popover element that can be triggered by a button or any other element.
#[derive(IntoElement)]
pub struct Popover {
    id: ElementId,
    style: StyleRefinement,
    anchor: Anchor,
    default_open: bool,
    open: Option<bool>,
    tracked_focus_handle: Option<FocusHandle>,
    trigger: Option<Box<dyn FnOnce(bool, &Window, &App) -> AnyElement + 'static>>,
    content: Option<
        Rc<
            dyn Fn(&mut PopoverState, &mut Window, &mut Context<PopoverState>) -> AnyElement
                + 'static,
        >,
    >,
    children: Vec<AnyElement>,
    /// Style for trigger element.
    /// This is used for hotfix the trigger element style to support w_full.
    trigger_style: Option<StyleRefinement>,
    mouse_button: MouseButton,
    appearance: bool,
    overlay_closable: bool,
    on_open_change: Option<Rc<dyn Fn(&bool, &mut Window, &mut App)>>,
}

impl Popover {
    /// Create a new Popover with `view` mode.
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            style: StyleRefinement::default(),
            anchor: Anchor::TopLeft,
            trigger: None,
            trigger_style: None,
            content: None,
            tracked_focus_handle: None,
            children: vec![],
            mouse_button: MouseButton::Left,
            appearance: true,
            overlay_closable: true,
            default_open: false,
            open: None,
            on_open_change: None,
        }
    }

    /// Set the anchor corner of the popover, default is `Corner::TopLeft`.
    ///
    /// This method is kept for backward compatibility with `Corner` type.
    /// Internally, it converts `Corner` to `Anchor`.
    pub fn anchor(mut self, anchor: impl Into<Anchor>) -> Self {
        self.anchor = anchor.into();
        self
    }

    /// Set the mouse button to trigger the popover, default is `MouseButton::Left`.
    pub fn mouse_button(mut self, mouse_button: MouseButton) -> Self {
        self.mouse_button = mouse_button;
        self
    }

    /// Set the trigger element of the popover.
    pub fn trigger<T>(mut self, trigger: T) -> Self
    where
        T: Selectable + IntoElement + 'static,
    {
        self.trigger = Some(Box::new(|is_open, _, _| {
            let selected = trigger.is_selected();
            trigger.selected(selected || is_open).into_any_element()
        }));
        self
    }

    /// Set the default open state of the popover, default is `false`.
    ///
    /// This is only used to initialize the open state of the popover.
    ///
    /// And please note that if you use the `open` method, this value will be ignored.
    pub fn default_open(mut self, open: bool) -> Self {
        self.default_open = open;
        self
    }

    /// Force set the open state of the popover.
    ///
    /// If this is set, the popover will be controlled by this value.
    ///
    /// NOTE: You must be used in conjunction with `on_open_change` to handle state changes.
    pub fn open(mut self, open: bool) -> Self {
        self.open = Some(open);
        self
    }

    /// Add a callback to be called when the open state changes.
    ///
    /// The first `&bool` parameter is the **new open state**.
    ///
    /// This is useful when using the `open` method to control the popover state.
    pub fn on_open_change<F>(mut self, callback: F) -> Self
    where
        F: Fn(&bool, &mut Window, &mut App) + 'static,
    {
        self.on_open_change = Some(Rc::new(callback));
        self
    }

    /// Set the style for the trigger element.
    pub fn trigger_style(mut self, style: StyleRefinement) -> Self {
        self.trigger_style = Some(style);
        self
    }

    /// Set whether clicking outside the popover will dismiss it, default is `true`.
    pub fn overlay_closable(mut self, closable: bool) -> Self {
        self.overlay_closable = closable;
        self
    }

    /// Set the content builder for content of the Popover.
    ///
    /// This callback will called every time on render the popover.
    /// So, you should avoid creating new elements or entities in the content closure.
    pub fn content<F, E>(mut self, content: F) -> Self
    where
        E: IntoElement,
        F: Fn(&mut PopoverState, &mut Window, &mut Context<PopoverState>) -> E + 'static,
    {
        self.content = Some(Rc::new(move |state, window, cx| {
            content(state, window, cx).into_any_element()
        }));
        self
    }

    /// Set whether the popover no style, default is `false`.
    ///
    /// If no style:
    ///
    /// - The popover will not have a bg, border, shadow, or padding.
    /// - The click out of the popover will not dismiss it.
    pub fn appearance(mut self, appearance: bool) -> Self {
        self.appearance = appearance;
        self
    }

    /// Bind the focus handle to receive focus when the popover is opened.
    /// If you not set this, a new focus handle will be created for the popover to
    ///
    /// If popover is opened, the focus will be moved to the focus handle.
    pub fn track_focus(mut self, handle: &FocusHandle) -> Self {
        self.tracked_focus_handle = Some(handle.clone());
        self
    }

    fn resolved_corner(anchor: Anchor, trigger_bounds: Bounds<Pixels>) -> Point<Pixels> {
        let offset = if anchor.is_center() {
            gpui::point(trigger_bounds.size.width.half(), px(0.))
        } else {
            Point::default()
        };

        trigger_bounds.corner(anchor.swap_vertical().into())
            + offset
            + Point {
                x: px(0.),
                y: -trigger_bounds.size.height,
            }
    }
}

impl ParentElement for Popover {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl Styled for Popover {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

pub struct PopoverState {
    focus_handle: FocusHandle,
    pub(crate) tracked_focus_handle: Option<FocusHandle>,
    previous_focus_handle: Option<FocusHandle>,
    trigger_bounds: Bounds<Pixels>,
    open: bool,
    needs_initial_transition: bool,
    on_open_change: Option<Rc<dyn Fn(&bool, &mut Window, &mut App)>>,

    _dismiss_subscription: Option<Subscription>,
}

impl PopoverState {
    pub fn new(default_open: bool, cx: &mut App) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            tracked_focus_handle: None,
            previous_focus_handle: None,
            trigger_bounds: Bounds::default(),
            open: default_open,
            needs_initial_transition: default_open,
            on_open_change: None,
            _dismiss_subscription: None,
        }
    }

    /// Check if the popover is open.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Dismiss the popover if it is open.
    pub fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.open {
            self.toggle_open(window, cx);
        }
    }

    /// Open the popover if it is closed.
    pub fn show(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.open {
            self.toggle_open(window, cx);
        }
    }

    fn set_open(
        &mut self,
        open: bool,
        emit_change: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.open == open {
            return;
        }

        self.open = open;
        if open {
            self.previous_focus_handle = window.focused(cx);
            let state = cx.entity();
            let focus_handle = if let Some(tracked_focus_handle) = self.tracked_focus_handle.clone()
            {
                tracked_focus_handle
            } else {
                self.focus_handle.clone()
            };
            focus_handle.focus(window, cx);

            self._dismiss_subscription =
                Some(
                    window.subscribe(&cx.entity(), cx, move |_, _: &DismissEvent, window, cx| {
                        state.update(cx, |state, cx| {
                            state.dismiss(window, cx);
                        });
                        window.refresh();
                    }),
                );
        } else {
            self._dismiss_subscription = None;
            let popover_owns_focus = self.focus_handle.contains_focused(window, cx)
                || self
                    .tracked_focus_handle
                    .as_ref()
                    .is_some_and(|handle| handle.contains_focused(window, cx));
            if popover_owns_focus
                && let Some(previous_focus_handle) = self.previous_focus_handle.take()
            {
                previous_focus_handle.focus(window, cx);
            }
        }

        if emit_change && let Some(callback) = self.on_open_change.as_ref() {
            callback(&open, window, cx);
        }
        cx.notify();
    }

    fn toggle_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_open(!self.open, true, window, cx);
    }

    fn on_action_cancel(&mut self, _: &Cancel, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss(window, cx);
    }
}

impl Focusable for PopoverState {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for PopoverState {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

impl EventEmitter<DismissEvent> for PopoverState {}

impl Popover {
    pub(crate) fn render_popover<E>(
        anchor: Anchor,
        trigger_bounds: Bounds<Pixels>,
        content: E,
        _: &mut Window,
        _: &mut App,
    ) -> Deferred
    where
        E: IntoElement + 'static,
    {
        deferred(
            anchored()
                .snap_to_window_with_margin(px(8.))
                .anchor(anchor)
                .position(Self::resolved_corner(anchor, trigger_bounds))
                .child(div().relative().child(content)),
        )
        .with_priority(1)
    }

    pub(crate) fn render_popover_content(
        anchor: Anchor,
        appearance: bool,
        _: &mut Window,
        cx: &mut App,
    ) -> Stateful<Div> {
        let tokens = FlyoutTokens::new(cx);

        v_flex()
            .id("content")
            .occlude()
            .tab_group()
            .when(appearance, |this| {
                SurfacePreset::flyout()
                    .with_radius(tokens.radius)
                    .apply_material(this, cx, SurfaceContext::new(cx))
                    .text_color(flyout_primary_foreground(cx))
                    .p(tokens.content_padding)
            })
            .map(|this| match anchor {
                Anchor::TopLeft | Anchor::TopCenter | Anchor::TopRight => this.top_1(),
                Anchor::BottomLeft | Anchor::BottomCenter | Anchor::BottomRight => this.bottom_1(),
            })
    }
}

impl RenderOnce for Popover {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let force_open = self.open;
        let default_open = self.default_open;
        let tracked_focus_handle = self.tracked_focus_handle.clone();
        let state = window.use_keyed_state(self.id.clone(), cx, |_, cx| {
            PopoverState::new(default_open, cx)
        });

        state.update(cx, |state, cx| {
            if let Some(tracked_focus_handle) = tracked_focus_handle {
                state.tracked_focus_handle = Some(tracked_focus_handle);
            }
            state.on_open_change = self.on_open_change.clone();

            let mut requested_open = force_open;
            if state.needs_initial_transition {
                state.needs_initial_transition = false;
                let initial_open = state.open;
                state.open = false;
                requested_open = Some(force_open.unwrap_or(initial_open));
            }
            if let Some(force_open) = requested_open {
                state.set_open(force_open, false, window, cx);
            }
        });

        let open = state.read(cx).open;
        let focus_handle = state.read(cx).focus_handle.clone();
        let trigger_bounds = state.read(cx).trigger_bounds;

        let Some(trigger) = self.trigger else {
            return div().id("empty");
        };

        let parent_view_id = window.current_view();
        let popover_id = self.id.clone();

        let el = div()
            .id(popover_id.clone())
            .child((trigger)(open, window, cx))
            .on_mouse_down(self.mouse_button, {
                let state = state.clone();
                move |_, window, cx| {
                    cx.stop_propagation();
                    state.update(cx, |state, cx| {
                        // We force set open to false to toggle it correctly.
                        // Because if the mouse down out will toggle open first.
                        state.open = open;
                        state.toggle_open(window, cx);
                    });
                    cx.notify(parent_view_id);
                }
            })
            .on_prepaint({
                let state = state.clone();
                move |bounds, _, cx| {
                    state.update(cx, |state, _| {
                        state.trigger_bounds = bounds;
                    })
                }
            });

        let motion = cx.theme().motion.clone();
        let reduced_motion = crate::animation::reduced_motion(cx);
        let presence = flyout_presence(
            SharedString::from(format!("popover-presence-{}", popover_id)),
            open,
            PresenceOptions {
                animate_on_mount: true,
            },
            window,
            cx,
        );
        if !presence.should_render() {
            return el;
        }

        let vertical_direction = if matches!(
            self.anchor,
            Anchor::TopLeft | Anchor::TopCenter | Anchor::TopRight
        ) {
            -1.0
        } else {
            1.0
        };
        let slide = FlyoutSlide::vertical(vertical_direction).exit_distance(4.0);

        let popover_content =
            Self::render_popover_content(self.anchor, self.appearance, window, cx)
                .track_focus(&focus_handle)
                .key_context(CONTEXT)
                .on_action(window.listener_for(&state, PopoverState::on_action_cancel))
                .when_some(self.content, |this, content| {
                    this.child(state.update(cx, |state, cx| (content)(state, window, cx)))
                })
                .children(self.children)
                .when(self.overlay_closable, |this| {
                    this.on_mouse_down_out({
                        let state = state.clone();
                        move |_, window, cx| {
                            state.update(cx, |state, cx| {
                                state.dismiss(window, cx);
                            });
                            cx.notify(parent_view_id);
                        }
                    })
                })
                .refine_style(&self.style)
                .map(move |el| {
                    flyout_motion("popover", presence, slide, &motion, reduced_motion, el)
                });

        el.child(Self::render_popover(
            self.anchor,
            trigger_bounds,
            popover_content,
            window,
            cx,
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc};

    use super::*;
    use gpui::{Entity, MouseButton, TestAppContext};

    fn focus_harness(
        cx: &mut TestAppContext,
    ) -> (
        Entity<PopoverState>,
        &mut gpui::VisualTestContext,
        FocusHandle,
        FocusHandle,
        FocusHandle,
    ) {
        cx.update(crate::init);
        let handles = Rc::new(RefCell::new(None));
        let handles_for_view = handles.clone();
        let (state, cx) = cx.add_window_view(move |_, cx| {
            let original = cx.focus_handle();
            let tracked = cx.focus_handle();
            let unrelated = cx.focus_handle();
            handles_for_view.replace(Some((original, tracked.clone(), unrelated)));
            let mut state = PopoverState::new(false, cx);
            state.tracked_focus_handle = Some(tracked);
            state
        });
        let (original, tracked, unrelated) = handles.borrow_mut().take().unwrap();
        (state, cx, original, tracked, unrelated)
    }

    #[test]
    fn test_popover_builder_chaining() {
        let popover = Popover::new("test")
            .anchor(Anchor::BottomCenter)
            .mouse_button(MouseButton::Right)
            .default_open(true)
            .appearance(false)
            .overlay_closable(false);

        assert_eq!(popover.anchor, Anchor::BottomCenter);
        assert_eq!(popover.mouse_button, MouseButton::Right);
        assert!(popover.default_open);
        assert!(!popover.appearance);
        assert!(!popover.overlay_closable);
    }

    #[test]
    fn test_resolved_corner_top_positions() {
        use gpui::px;

        let bounds = Bounds {
            origin: Point {
                x: px(100.),
                y: px(100.),
            },
            size: gpui::Size {
                width: px(200.),
                height: px(50.),
            },
        };

        let pos = Popover::resolved_corner(Anchor::TopLeft, bounds);
        assert_eq!(pos.x, px(100.));
        assert_eq!(pos.y, px(100.));

        let pos = Popover::resolved_corner(Anchor::TopCenter, bounds);
        assert_eq!(pos.x, px(200.));
        assert_eq!(pos.y, px(100.));

        let pos = Popover::resolved_corner(Anchor::TopRight, bounds);
        assert_eq!(pos.x, px(300.));
        assert_eq!(pos.y, px(100.));

        let pos = Popover::resolved_corner(Anchor::BottomLeft, bounds);
        assert_eq!(pos.x, px(100.));
        assert_eq!(pos.y, px(50.));

        let pos = Popover::resolved_corner(Anchor::BottomCenter, bounds);
        assert_eq!(pos.x, px(200.));
        assert_eq!(pos.y, px(50.));

        let pos = Popover::resolved_corner(Anchor::BottomRight, bounds);
        assert_eq!(pos.x, px(300.));
        assert_eq!(pos.y, px(50.));
    }

    #[gpui::test]
    fn dismiss_restores_previous_focus(cx: &mut TestAppContext) {
        let (state, cx, original, tracked, _) = focus_harness(cx);

        cx.update(|window, cx| original.focus(window, cx));
        state.update_in(cx, |state, window, cx| state.show(window, cx));
        cx.update(|window, _| assert!(tracked.is_focused(window)));

        state.update_in(cx, |state, window, cx| state.dismiss(window, cx));
        cx.update(|window, _| assert!(original.is_focused(window)));
    }

    #[gpui::test]
    fn dismiss_preserves_focus_that_moved_elsewhere(cx: &mut TestAppContext) {
        let (state, cx, original, tracked, unrelated) = focus_harness(cx);

        cx.update(|window, cx| original.focus(window, cx));
        state.update_in(cx, |state, window, cx| state.show(window, cx));
        cx.update(|window, _| assert!(tracked.is_focused(window)));
        cx.update(|window, cx| unrelated.focus(window, cx));

        state.update_in(cx, |state, window, cx| state.dismiss(window, cx));
        cx.update(|window, _| assert!(unrelated.is_focused(window)));
    }
}
