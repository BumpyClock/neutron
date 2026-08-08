use std::rc::Rc;

use gpui::{
    AnimationExt as _, AnyElement, App, Bounds, BoxShadow, ClickEvent, Edges, ElementId,
    FocusHandle, Hsla, InteractiveElement, IntoElement, KeyBinding, MouseButton, ParentElement,
    Pixels, Point, RenderOnce, SharedString, StyleRefinement, Styled, Window, WindowControlArea,
    anchored, div, hsla, point, prelude::FluentBuilder, px, relative,
};
use rust_i18n::t;

use crate::{
    ActiveTheme as _, ClosingScope, FocusTrapElement as _, IconName, Root, Sizable as _, StyledExt,
    TITLE_BAR_HEIGHT, WindowExt as _,
    actions::{Cancel, Confirm},
    animation::{
        PresenceOptions, PresencePhase, enter_animation, exit_animation, fade_animation,
        keyed_presence, spring_animation, standard_animation,
    },
    button::{Button, ButtonVariant, ButtonVariants as _},
    global_state::GlobalState,
    h_flex,
    scroll::ScrollableElement as _,
    v_flex,
};

const CONTEXT: &str = "Dialog";
const OPEN_Y_OFFSET: f32 = 10.0;
const CLOSE_Y_OFFSET: f32 = 8.0;
pub(crate) fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("escape", Cancel, Some(CONTEXT)),
        KeyBinding::new("enter", Confirm { secondary: false }, Some(CONTEXT)),
    ]);
}

fn dialog_shadow(delta: f32) -> Vec<BoxShadow> {
    vec![
        BoxShadow {
            color: hsla(0., 0., 0., 0.1 * delta),
            offset: point(px(0.), px(20.)),
            blur_radius: px(25.),
            spread_radius: px(-5.),
            inset: false,
        },
        BoxShadow {
            color: hsla(0., 0., 0., 0.1 * delta),
            offset: point(px(0.), px(8.)),
            blur_radius: px(10.),
            spread_radius: px(-6.),
            inset: false,
        },
    ]
}

type RenderButtonFn = Box<dyn FnOnce(&mut Window, &mut App) -> AnyElement>;
type FooterFn =
    Box<dyn Fn(RenderButtonFn, RenderButtonFn, &mut Window, &mut App) -> Vec<AnyElement>>;

/// Dialog button props.
pub struct DialogButtonProps {
    ok_text: Option<SharedString>,
    ok_variant: ButtonVariant,
    cancel_text: Option<SharedString>,
    cancel_variant: ButtonVariant,
}

impl Default for DialogButtonProps {
    fn default() -> Self {
        Self {
            ok_text: None,
            ok_variant: ButtonVariant::Primary,
            cancel_text: None,
            cancel_variant: ButtonVariant::default(),
        }
    }
}

impl DialogButtonProps {
    /// Sets the text of the OK button. Default is `OK`.
    pub fn ok_text(mut self, ok_text: impl Into<SharedString>) -> Self {
        self.ok_text = Some(ok_text.into());
        self
    }

    /// Sets the variant of the OK button. Default is `ButtonVariant::Primary`.
    pub fn ok_variant(mut self, ok_variant: ButtonVariant) -> Self {
        self.ok_variant = ok_variant;
        self
    }

    /// Sets the text of the Cancel button. Default is `Cancel`.
    pub fn cancel_text(mut self, cancel_text: impl Into<SharedString>) -> Self {
        self.cancel_text = Some(cancel_text.into());
        self
    }

    /// Sets the variant of the Cancel button. Default is `ButtonVariant::default()`.
    pub fn cancel_variant(mut self, cancel_variant: ButtonVariant) -> Self {
        self.cancel_variant = cancel_variant;
        self
    }
}

/// A modal to display content in a dialog box.
#[derive(IntoElement)]
pub struct Dialog {
    style: StyleRefinement,
    title: Option<AnyElement>,
    footer: Option<FooterFn>,
    children: Vec<AnyElement>,
    width: Pixels,
    max_width: Option<Pixels>,
    margin_top: Option<Pixels>,

    on_close: Rc<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>,
    on_ok: Option<Rc<dyn Fn(&ClickEvent, &mut Window, &mut App) -> bool + 'static>>,
    on_cancel: Rc<dyn Fn(&ClickEvent, &mut Window, &mut App) -> bool + 'static>,
    button_props: DialogButtonProps,
    close_button: bool,
    overlay: bool,
    overlay_closable: bool,
    keyboard: bool,
    animate: bool,
    defer_close: bool,
    appearance: bool,

    /// This will be change when open the dialog, the focus handle is create when open the dialog.
    pub(crate) focus_handle: FocusHandle,
    pub(crate) id: u64,
    pub(crate) layer_ix: usize,
    pub(crate) overlay_visible: bool,
    pub(crate) closing: bool,
}

pub(crate) fn overlay_color(overlay: bool, cx: &App) -> Hsla {
    if !overlay {
        return hsla(0., 0., 0., 0.);
    }

    cx.theme().overlay
}

impl Dialog {
    /// Create a new dialog.
    pub fn new(_: &mut Window, cx: &mut App) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            style: StyleRefinement::default(),
            title: None,
            footer: None,
            children: Vec::new(),
            margin_top: None,
            width: px(480.),
            max_width: None,
            overlay: true,
            keyboard: true,
            animate: true,
            defer_close: false,
            appearance: true,
            id: 0,
            layer_ix: 0,
            overlay_visible: false,
            closing: false,
            on_close: Rc::new(|_, _, _| {}),
            on_ok: None,
            on_cancel: Rc::new(|_, _, _| true),
            button_props: DialogButtonProps::default(),
            close_button: true,
            overlay_closable: true,
        }
    }

    pub(crate) fn should_animate(&self, cx: &App) -> bool {
        self.animate && !GlobalState::global(cx).reduced_motion()
    }

    /// Whether closing should keep the dialog mounted for the exit window
    /// before unmounting: true when the chrome animates, or when the opener
    /// requested [`Dialog::defer_close`] for content-driven exits. Reduced
    /// motion always unmounts immediately.
    pub(crate) fn should_defer_close(&self, cx: &App) -> bool {
        (self.animate || self.defer_close) && !GlobalState::global(cx).reduced_motion()
    }

    /// Sets the title of the dialog.
    pub fn title(mut self, title: impl IntoElement) -> Self {
        self.title = Some(title.into_any_element());
        self
    }

    /// Set the footer of the dialog.
    ///
    /// The `footer` is a function that takes two `RenderButtonFn` and a `WindowContext` and returns a list of `AnyElement`.
    ///
    /// - First `RenderButtonFn` is the render function for the OK button.
    /// - Second `RenderButtonFn` is the render function for the CANCEL button.
    ///
    /// When you set the footer, the footer will be placed default footer buttons.
    pub fn footer<E, F>(mut self, footer: F) -> Self
    where
        E: IntoElement,
        F: Fn(RenderButtonFn, RenderButtonFn, &mut Window, &mut App) -> Vec<E> + 'static,
    {
        self.footer = Some(Box::new(move |ok, cancel, window, cx| {
            footer(ok, cancel, window, cx)
                .into_iter()
                .map(|e| e.into_any_element())
                .collect()
        }));
        self
    }

    /// Set to use confirm dialog, with OK and Cancel buttons.
    ///
    /// See also [`Self::alert`]
    pub fn confirm(self) -> Self {
        self.footer(|ok, cancel, window, cx| vec![cancel(window, cx), ok(window, cx)])
            .overlay_closable(false)
            .close_button(false)
    }

    /// Set to as a alter dialog, with OK button.
    ///
    /// See also [`Self::confirm`]
    pub fn alert(self) -> Self {
        self.footer(|ok, _, window, cx| vec![ok(window, cx)])
            .overlay_closable(false)
            .close_button(false)
    }

    /// Set the button props of the dialog.
    pub fn button_props(mut self, button_props: DialogButtonProps) -> Self {
        self.button_props = button_props;
        self
    }

    /// Sets the callback for when the dialog is closed.
    ///
    /// Called after [`Self::on_ok`] or [`Self::on_cancel`] callback.
    pub fn on_close(
        mut self,
        on_close: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_close = Rc::new(on_close);
        self
    }

    /// Sets the callback for when the dialog is has been confirmed.
    ///
    /// The callback should return `true` to close the dialog, if return `false` the dialog will not be closed.
    pub fn on_ok(
        mut self,
        on_ok: impl Fn(&ClickEvent, &mut Window, &mut App) -> bool + 'static,
    ) -> Self {
        self.on_ok = Some(Rc::new(on_ok));
        self
    }

    /// Sets the callback for when the dialog is has been canceled.
    ///
    /// The callback should return `true` to close the dialog, if return `false` the dialog will not be closed.
    pub fn on_cancel(
        mut self,
        on_cancel: impl Fn(&ClickEvent, &mut Window, &mut App) -> bool + 'static,
    ) -> Self {
        self.on_cancel = Rc::new(on_cancel);
        self
    }

    /// Sets the false to hide close icon, default: true
    pub fn close_button(mut self, close_button: bool) -> Self {
        self.close_button = close_button;
        self
    }

    /// Set the top offset of the dialog, defaults to None, will use the 1/10 of the viewport height.
    pub fn margin_top(mut self, margin_top: impl Into<Pixels>) -> Self {
        self.margin_top = Some(margin_top.into());
        self
    }

    /// Sets the width of the dialog, defaults to 480px.
    ///
    /// See also [`Self::width`]
    pub fn w(mut self, width: impl Into<Pixels>) -> Self {
        self.width = width.into();
        self
    }

    /// Sets the width of the dialog, defaults to 480px.
    pub fn width(mut self, width: impl Into<Pixels>) -> Self {
        self.width = width.into();
        self
    }

    /// Set the maximum width of the dialog, defaults to `None`.
    pub fn max_w(mut self, max_width: impl Into<Pixels>) -> Self {
        self.max_width = Some(max_width.into());
        self
    }

    /// Set the overlay of the dialog, defaults to `true`.
    pub fn overlay(mut self, overlay: bool) -> Self {
        self.overlay = overlay;
        self
    }

    /// Set the overlay closable of the dialog, defaults to `true`.
    ///
    /// When the overlay is clicked, the dialog will be closed.
    pub fn overlay_closable(mut self, overlay_closable: bool) -> Self {
        self.overlay_closable = overlay_closable;
        self
    }

    /// Set whether to support keyboard esc to close the dialog, defaults to `true`.
    pub fn keyboard(mut self, keyboard: bool) -> Self {
        self.keyboard = keyboard;
        self
    }

    /// Set whether to play enter animations, defaults to `true`.
    pub fn animate(mut self, animate: bool) -> Self {
        self.animate = animate;
        self
    }

    /// Keep the dialog mounted through the exit window while closing, so
    /// content can run its own exit animation even when the dialog chrome does
    /// not animate (`animate(false)`).
    ///
    /// Content learns the closing state via [`crate::is_layer_closing`]. The
    /// window is [`crate::animation::exit_duration`] and is also the ceiling —
    /// the dialog is torn down when it elapses whether or not the content
    /// finished; there is no completion signal to leak.
    pub fn defer_close(mut self, defer: bool) -> Self {
        self.defer_close = defer;
        self
    }

    /// Set whether the dialog renders its default background, border, radius, and shadow.
    pub fn appearance(mut self, appearance: bool) -> Self {
        self.appearance = appearance;
        self
    }

    pub(crate) fn has_overlay(&self) -> bool {
        self.overlay
    }

    fn defer_close_dialog(window: &mut Window, cx: &mut App) {
        Root::update(window, cx, |root, window, cx| {
            root.defer_close_dialog(window, cx);
        });
    }
}

impl ParentElement for Dialog {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl Styled for Dialog {
    fn style(&mut self) -> &mut gpui::StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for Dialog {
    fn render(self, window: &mut Window, cx: &mut App) -> impl gpui::IntoElement {
        let layer_ix = self.layer_ix;
        let dialog_id = self.id;
        let on_close = self.on_close.clone();
        let on_ok = self.on_ok.clone();
        let on_cancel = self.on_cancel.clone();
        let has_title = self.title.is_some();
        let reduced_motion = GlobalState::global(cx).reduced_motion();
        let should_animate = self.should_animate(cx);
        // The presence runs whenever the close is deferred — including
        // content-driven exits with a non-animating chrome — so the Exiting
        // phase exists for the whole deferral window.
        let presence_active = self.should_defer_close(cx);
        let closing = self.closing;
        let target_open = !self.closing;
        let appearance = self.appearance;

        let render_ok: RenderButtonFn = Box::new({
            let on_ok = on_ok.clone();
            let on_close = on_close.clone();
            let ok_text = self
                .button_props
                .ok_text
                .unwrap_or_else(|| t!("Dialog.ok").into());
            let ok_variant = self.button_props.ok_variant;
            move |_, _| {
                Button::new("ok")
                    .label(ok_text)
                    .with_variant(ok_variant)
                    .on_click({
                        let on_ok = on_ok.clone();
                        let on_close = on_close.clone();

                        move |_, window, cx| {
                            if let Some(on_ok) = &on_ok {
                                if !on_ok(&ClickEvent::default(), window, cx) {
                                    return;
                                }
                            }

                            window.close_dialog(cx);
                            on_close(&ClickEvent::default(), window, cx);
                        }
                    })
                    .into_any_element()
            }
        });
        let render_cancel: RenderButtonFn = Box::new({
            let on_cancel = on_cancel.clone();
            let on_close = on_close.clone();
            let cancel_text = self
                .button_props
                .cancel_text
                .unwrap_or_else(|| t!("Dialog.cancel").into());
            let cancel_variant = self.button_props.cancel_variant;
            move |_, _| {
                Button::new("cancel")
                    .label(cancel_text)
                    .with_variant(cancel_variant)
                    .on_click({
                        let on_cancel = on_cancel.clone();
                        let on_close = on_close.clone();
                        move |_, window, cx| {
                            if !on_cancel(&ClickEvent::default(), window, cx) {
                                return;
                            }

                            window.close_dialog(cx);
                            on_close(&ClickEvent::default(), window, cx);
                        }
                    })
                    .into_any_element()
            }
        });

        let window_paddings = crate::window_border::window_paddings(window);
        let view_size = window.viewport_size()
            - gpui::size(
                window_paddings.left + window_paddings.right,
                window_paddings.top + window_paddings.bottom,
            );
        let bounds = Bounds {
            origin: Point::default(),
            size: view_size,
        };
        let offset_top = px(layer_ix as f32 * 16.);
        let y = self.margin_top.unwrap_or(view_size.height / 10.) + offset_top;
        let x = bounds.center().x - self.width / 2.;

        let base_size = window.text_style().font_size;
        let rem_size = window.rem_size();

        let mut paddings = Edges::all(px(24.));
        if let Some(pl) = self.style.padding.left {
            paddings.left = pl.to_pixels(base_size, rem_size);
        }
        if let Some(pr) = self.style.padding.right {
            paddings.right = pr.to_pixels(base_size, rem_size);
        }
        if let Some(pt) = self.style.padding.top {
            paddings.top = pt.to_pixels(base_size, rem_size);
        }
        if let Some(pb) = self.style.padding.bottom {
            paddings.bottom = pb.to_pixels(base_size, rem_size);
        }

        if !has_title {
            // When no title, reduce the top padding to fix line-height effect.
            paddings.top -= px(6.);
        }

        let open_duration = crate::animation::enter_duration(&cx.theme().motion);
        let close_duration = crate::animation::exit_duration(&cx.theme().motion);

        let presence = keyed_presence(
            SharedString::from(format!("dialog-{}-presence", dialog_id)),
            target_open,
            presence_active,
            open_duration,
            close_duration,
            PresenceOptions {
                animate_on_mount: true,
            },
            window,
            cx,
        );
        let transition_active = presence.transition_active();

        let motion = &cx.theme().motion;
        let open_panel_layout_animation = standard_animation(motion, reduced_motion)
            .or_else(|| enter_animation(motion, reduced_motion))
            .unwrap_or_else(|| {
                gpui::Animation::new(std::time::Duration::from_millis(u64::from(
                    motion.enter_duration_ms,
                )))
            });
        let open_panel_transform_animation = spring_animation(motion, reduced_motion);
        let close_panel_animation = exit_animation(motion, reduced_motion).unwrap_or_else(|| {
            gpui::Animation::new(std::time::Duration::from_millis(u64::from(
                motion.exit_duration_ms,
            )))
        });
        let fade_in_animation = fade_animation(motion, reduced_motion).unwrap_or_else(|| {
            gpui::Animation::new(std::time::Duration::from_millis(u64::from(
                motion.fade_duration_ms,
            )))
        });
        let fade_out_animation = fade_animation(motion, reduced_motion).unwrap_or_else(|| {
            gpui::Animation::new(std::time::Duration::from_millis(u64::from(
                motion.fade_duration_ms,
            )))
        });

        anchored()
            .position(point(window_paddings.left, window_paddings.top))
            .snap_to_window()
            .child(
                div()
                    .id(ElementId::NamedInteger("dialog".into(), layer_ix as u64))
                    .occlude()
                    .w(view_size.width)
                    .h(view_size.height)
                    .when(self.overlay_visible, |this| {
                        this.bg(overlay_color(self.overlay, cx))
                    })
                    .when(self.overlay, |this| {
                        // Only the last dialog owns the `mouse down - close dialog` event.
                        if (self.layer_ix + 1) != Root::read(window, cx).active_dialogs.len() {
                            return this;
                        }

                        this.window_control_area(WindowControlArea::Drag)
                            .on_any_mouse_down({
                                let on_cancel = on_cancel.clone();
                                let on_close = on_close.clone();
                                move |event, window, cx| {
                                    if event.position.y < TITLE_BAR_HEIGHT {
                                        return;
                                    }

                                    cx.stop_propagation();
                                    if self.overlay_closable && event.button == MouseButton::Left {
                                        on_cancel(&ClickEvent::default(), window, cx);
                                        on_close(&ClickEvent::default(), window, cx);
                                        window.close_dialog(cx);
                                    }
                                }
                            })
                    })
                    .child(
                        v_flex()
                            .id(layer_ix)
                            .track_focus(&self.focus_handle)
                            .focus_trap(format!("dialog-{}", layer_ix), &self.focus_handle)
                            .when(appearance, |this| {
                                this.bg(cx.theme().popover)
                                    .border_1()
                                    .border_color(cx.theme().border)
                                    .rounded(cx.theme().radius_lg)
                                    .min_h_24()
                            })
                            .pt(paddings.top)
                            .pb(paddings.bottom)
                            .gap(paddings.top.min(px(16.)))
                            .refine_style(&self.style)
                            .px_0()
                            .key_context(CONTEXT)
                            .when(self.keyboard, |this| {
                                this.on_action({
                                    let on_cancel = on_cancel.clone();
                                    let on_close = on_close.clone();
                                    move |_: &Cancel, window, cx| {
                                        window.close_dialog(cx);
                                        // FIXME:
                                        //
                                        // Here some Dialog have no focus_handle, so it will not work will Escape key.
                                        // But by now, we `cx.close_dialog()` going to close the last active model, so the Escape is unexpected to work.
                                        on_cancel(&ClickEvent::default(), window, cx);
                                        on_close(&ClickEvent::default(), window, cx);
                                    }
                                })
                                .on_action({
                                    let on_ok = on_ok.clone();
                                    let on_close = on_close.clone();
                                    let has_footer = self.footer.is_some();
                                    move |_: &Confirm, window, cx| {
                                        if let Some(on_ok) = &on_ok {
                                            if on_ok(&ClickEvent::default(), window, cx) {
                                                Self::defer_close_dialog(window, cx);
                                                on_close(&ClickEvent::default(), window, cx);
                                            }
                                        } else if has_footer {
                                            Self::defer_close_dialog(window, cx);
                                            on_close(&ClickEvent::default(), window, cx);
                                        }
                                    }
                                })
                            })
                            // There style is high priority, can't be overridden.
                            .absolute()
                            .occlude()
                            .relative()
                            .left(x)
                            .top(y)
                            .w(self.width)
                            .when_some(self.max_width, |this, w| this.max_w(w))
                            .when_some(self.title, |this, title| {
                                this.child(
                                    div()
                                        .pl(paddings.left)
                                        .pr(paddings.right)
                                        .line_height(relative(1.))
                                        .font_semibold()
                                        .child(title),
                                )
                            })
                            .children(self.close_button.then(|| {
                                let top = (paddings.top - px(10.)).max(px(8.));
                                let right = (paddings.right - px(10.)).max(px(8.));

                                Button::new("close")
                                    .absolute()
                                    .top(top)
                                    .right(right)
                                    .small()
                                    .ghost()
                                    .icon(IconName::Close)
                                    .on_click({
                                        let on_cancel = self.on_cancel.clone();
                                        let on_close = self.on_close.clone();
                                        move |_, window, cx| {
                                            window.close_dialog(cx);
                                            on_cancel(&ClickEvent::default(), window, cx);
                                            on_close(&ClickEvent::default(), window, cx);
                                        }
                                    })
                            }))
                            .child(
                                div().flex_1().overflow_hidden().child(
                                    // Body. ClosingScope exposes the closing
                                    // state so content can run its own exit
                                    // animation during the deferral window
                                    // (see `Dialog::defer_close`).
                                    ClosingScope::new(
                                        closing,
                                        v_flex()
                                            .size_full()
                                            .overflow_y_scrollbar()
                                            .pl(paddings.left)
                                            .pr(paddings.right)
                                            .children(self.children),
                                    ),
                                ),
                            )
                            .when_some(self.footer, |this, footer| {
                                this.child(
                                    h_flex()
                                        .gap_2()
                                        .pl(paddings.left)
                                        .pr(paddings.right)
                                        .line_height(relative(1.))
                                        .justify_end()
                                        .children(footer(render_ok, render_cancel, window, cx)),
                                )
                            })
                            .on_any_mouse_down({
                                |_, _, cx| {
                                    cx.stop_propagation();
                                }
                            })
                            .map(move |this| {
                                if !should_animate || !transition_active {
                                    // A non-animating chrome stays fully present
                                    // while a content-driven deferred close plays
                                    // out; the content owns the exit visuals.
                                    let progress = if !should_animate
                                        && matches!(presence.phase, PresencePhase::Exiting)
                                    {
                                        1.0
                                    } else {
                                        presence.progress(1.0)
                                    };
                                    this.when(appearance, |this| {
                                        this.shadow(dialog_shadow(progress))
                                    })
                                    .opacity(progress)
                                    .into_any_element()
                                } else {
                                    let panel_layout_animation =
                                        if matches!(presence.phase, PresencePhase::Entering) {
                                            open_panel_layout_animation
                                        } else {
                                            close_panel_animation
                                        };
                                    let layout_animated = this
                                        .with_animation(
                                            SharedString::from(format!(
                                                "dialog-panel-layout-{}",
                                                u8::from(matches!(
                                                    presence.phase,
                                                    PresencePhase::Entering
                                                ))
                                            )),
                                            panel_layout_animation,
                                            move |this, delta| {
                                                let progress =
                                                    presence.progress(delta).clamp(0.0, 1.0);
                                                let this = if matches!(
                                                    presence.phase,
                                                    PresencePhase::Exiting
                                                ) {
                                                    let offset =
                                                        px(CLOSE_Y_OFFSET * (1.0 - progress));
                                                    this.translate_y(offset)
                                                } else {
                                                    this
                                                };
                                                this.opacity(progress).when(appearance, |this| {
                                                    this.shadow(dialog_shadow(progress))
                                                })
                                            },
                                        )
                                        .into_any_element();
                                    if matches!(presence.phase, PresencePhase::Entering) {
                                        if let Some(transform_animation) =
                                            open_panel_transform_animation
                                        {
                                            return div()
                                                .child(layout_animated)
                                                .with_animation(
                                                    SharedString::from(
                                                        "dialog-panel-open-transform",
                                                    ),
                                                    transform_animation,
                                                    move |this, delta| {
                                                        this.translate_y(px(
                                                            OPEN_Y_OFFSET * (1.0 - delta)
                                                        ))
                                                    },
                                                )
                                                .into_any_element();
                                        }
                                    }
                                    layout_animated
                                }
                            }),
                    )
                    .map(move |this| {
                        if !should_animate || !transition_active {
                            // Hold the layer visible through a content-driven
                            // deferred close (see `Dialog::defer_close`).
                            let progress = if !should_animate
                                && matches!(presence.phase, PresencePhase::Exiting)
                            {
                                1.0
                            } else {
                                presence.progress(1.0)
                            };
                            this.opacity(progress).into_any_element()
                        } else {
                            let fade_animation =
                                if matches!(presence.phase, PresencePhase::Entering) {
                                    fade_in_animation
                                } else {
                                    fade_out_animation
                                };
                            this.with_animation(
                                SharedString::from(format!(
                                    "dialog-fade-motion-{}",
                                    u8::from(matches!(presence.phase, PresencePhase::Entering))
                                )),
                                fade_animation,
                                move |this, delta| {
                                    let opacity = presence.progress(delta);
                                    this.opacity(opacity.clamp(0.0, 1.0))
                                },
                            )
                            .into_any_element()
                        }
                    }),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{AppContext as _, Context, Render, TestAppContext, div};

    struct TestRoot;
    impl Render for TestRoot {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
        }
    }

    /// Runs `f` inside a real window, since `Dialog::new` needs both a Window
    /// and an App.
    fn in_window<R: 'static>(
        cx: &mut TestAppContext,
        reduced_motion: bool,
        f: impl FnOnce(&mut Window, &mut App) -> R + 'static,
    ) -> R {
        let window = cx.update(|cx| {
            crate::init(cx);
            GlobalState::global_mut(cx).set_reduced_motion(reduced_motion);
            cx.open_window(Default::default(), |_, cx| cx.new(|_| TestRoot))
                .unwrap()
        });

        window.update(cx, |_, window, cx| f(window, cx)).unwrap()
    }

    #[gpui::test]
    fn test_dialog_builder(cx: &mut TestAppContext) {
        in_window(cx, false, |window, cx| {
            let dialog = Dialog::new(window, cx);
            assert!(dialog.animate, "chrome animates by default");
            assert!(!dialog.defer_close, "deferred close is opt-in");

            let configured = Dialog::new(window, cx).animate(false).defer_close(true);
            assert!(!configured.animate);
            assert!(configured.defer_close);
        });
    }

    /// An animating dialog already stays mounted for its exit; `defer_close` is
    /// for hosts that opt out of chrome animation and drive their own exit.
    #[gpui::test]
    fn test_should_defer_close_covers_both_opt_ins(cx: &mut TestAppContext) {
        in_window(cx, false, |window, cx| {
            assert!(
                Dialog::new(window, cx)
                    .animate(true)
                    .defer_close(false)
                    .should_defer_close(cx)
            );
            assert!(
                Dialog::new(window, cx)
                    .animate(false)
                    .defer_close(true)
                    .should_defer_close(cx),
                "defer_close must hold a non-animating dialog mounted"
            );
            assert!(
                !Dialog::new(window, cx)
                    .animate(false)
                    .defer_close(false)
                    .should_defer_close(cx),
                "hosts opting into neither keep the immediate-unmount path"
            );
        });
    }

    /// Reduced motion unmounts immediately regardless of either opt-in, so a
    /// deferred window can never hold a dialog past its dismissal.
    #[gpui::test]
    fn test_reduced_motion_never_defers(cx: &mut TestAppContext) {
        in_window(cx, true, |window, cx| {
            let dialog = Dialog::new(window, cx).animate(true).defer_close(true);
            assert!(!dialog.should_defer_close(cx));
            assert!(!dialog.should_animate(cx));
        });
    }
}
