use std::rc::Rc;

use gpui::{
    AbsoluteLength, AnimationExt as _, AnyElement, App, Bounds, ClickEvent, Decorations,
    DefiniteLength, DismissEvent, Edges, EventEmitter, FocusHandle, InteractiveElement as _,
    IntoElement, KeyBinding, MouseButton, ParentElement, Pixels, RenderOnce, SharedString,
    StyleRefinement, Styled, Window, WindowControlArea, anchored, div, point,
    prelude::FluentBuilder as _, px,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    ActiveTheme, FocusTrapElement as _, IconName, Placement, Sizable, StyledExt as _,
    WindowExt as _,
    actions::Cancel,
    animation::{enter_animation, exit_animation},
    button::{Button, ButtonVariants as _},
    dialog::overlay_color,
    h_flex,
    scroll::ScrollableElement as _,
    title_bar::TITLE_BAR_HEIGHT,
    v_flex,
};

const CONTEXT: &str = "Sheet";

fn sheet_content_bounds(
    viewport: gpui::Size<Pixels>,
    decorations: Decorations,
    mut insets: Edges<Pixels>,
) -> Bounds<Pixels> {
    match decorations {
        Decorations::Server => insets = Edges::all(px(0.)),
        Decorations::Client { tiling } => {
            // WindowBorder adds a one-pixel border only on non-tiled client edges.
            for (inset, tiled) in [
                (&mut insets.top, tiling.top),
                (&mut insets.right, tiling.right),
                (&mut insets.bottom, tiling.bottom),
                (&mut insets.left, tiling.left),
            ] {
                if tiled {
                    *inset = px(0.);
                } else {
                    *inset += px(1.);
                }
            }
        }
    }
    Bounds::new(
        point(insets.left, insets.top),
        gpui::size(
            (viewport.width - insets.left - insets.right).max(px(0.)),
            (viewport.height - insets.top - insets.bottom).max(px(0.)),
        ),
    )
}

pub(crate) fn init(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("escape", Cancel, Some(CONTEXT))])
}

/// The settings for sheets.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SheetSettings {
    /// The margin top for the sheet, default is [`TITLE_BAR_HEIGHT`].
    pub margin_top: Pixels,
}

impl Default for SheetSettings {
    fn default() -> Self {
        Self {
            margin_top: TITLE_BAR_HEIGHT,
        }
    }
}

/// Sheet component that slides in from the side of the window.
#[derive(IntoElement)]
pub struct Sheet {
    pub(crate) focus_handle: FocusHandle,
    pub(crate) placement: Placement,
    pub(crate) size: DefiniteLength,
    resizable: bool,
    on_close: Rc<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>,
    title: Option<AnyElement>,
    footer: Option<AnyElement>,
    style: StyleRefinement,
    children: Vec<AnyElement>,
    overlay: bool,
    overlay_closable: bool,
    pub(crate) closing: bool,
}

impl Sheet {
    /// Creates a new Sheet.
    pub fn new(_: &mut Window, cx: &mut App) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            placement: Placement::Right,
            size: DefiniteLength::Absolute(px(350.).into()),
            resizable: true,
            title: None,
            footer: None,
            style: StyleRefinement::default(),
            children: Vec::new(),
            overlay: true,
            overlay_closable: true,
            closing: false,
            on_close: Rc::new(|_, _, _| {}),
        }
    }

    /// Sets the title of the sheet.
    pub fn title(mut self, title: impl IntoElement) -> Self {
        self.title = Some(title.into_any_element());
        self
    }

    /// Set the footer of the sheet.
    pub fn footer(mut self, footer: impl IntoElement) -> Self {
        self.footer = Some(footer.into_any_element());
        self
    }

    /// Sets the size of the sheet, default is 350px.
    pub fn size(mut self, size: impl Into<DefiniteLength>) -> Self {
        self.size = size.into();
        self
    }

    /// Sets whether the sheet is resizable, default is `true`.
    pub fn resizable(mut self, resizable: bool) -> Self {
        self.resizable = resizable;
        self
    }

    /// Set whether the sheet should have an overlay, default is `true`.
    pub fn overlay(mut self, overlay: bool) -> Self {
        self.overlay = overlay;
        self
    }

    /// Set whether the sheet should be closable by clicking the overlay, default is `true`.
    pub fn overlay_closable(mut self, overlay_closable: bool) -> Self {
        self.overlay_closable = overlay_closable;
        self
    }

    /// Listen to the close event of the sheet.
    pub fn on_close(
        mut self,
        on_close: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_close = Rc::new(on_close);
        self
    }
}

impl EventEmitter<DismissEvent> for Sheet {}
impl ParentElement for Sheet {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}
impl Styled for Sheet {
    fn style(&mut self) -> &mut gpui::StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for Sheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let placement = self.placement;
        let content_bounds = sheet_content_bounds(
            window.viewport_size(),
            window.window_decorations(),
            crate::window_border::window_paddings(window),
        );
        let size = content_bounds.size;
        let top = cx.theme().sheet.margin_top;
        let reduced_motion = crate::animation::reduced_motion(cx);
        let motion = &cx.theme().motion;
        let closing = self.closing;
        // While closing, the Root keeps the sheet mounted for the exit window
        // and this reversed slide plays; the window is also the ceiling.
        let slide_animation = if closing {
            exit_animation(motion, reduced_motion)
        } else {
            enter_animation(motion, reduced_motion)
        };
        let on_close = self.on_close.clone();

        let base_size = window.text_style().font_size;
        let rem_size = window.rem_size();
        // Slide the panel's full extent along its placement axis. A fixed
        // travel shorter than the panel makes it appear part-way in and then
        // settle, rather than entering from off-screen.
        let travel = self.size.to_pixels(
            AbsoluteLength::Pixels(if placement.is_horizontal() {
                size.width
            } else {
                size.height
            }),
            rem_size,
        );
        let mut paddings = Edges::all(px(16.));
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

        anchored()
            .position(content_bounds.origin)
            .snap_to_window()
            .child(
                div()
                    .debug_selector(|| "sheet-frame".into())
                    .occlude()
                    .w(size.width)
                    .h(size.height)
                    .bg(overlay_color(self.overlay, cx))
                    .when(self.overlay, |this| {
                        this.when(placement == Placement::Bottom, |this| {
                            this.window_control_area(WindowControlArea::Drag)
                        })
                        .on_any_mouse_down({
                            let on_close = self.on_close.clone();
                            move |event, window, cx| {
                                if event.position.y < content_bounds.top() + top {
                                    return;
                                }

                                cx.stop_propagation();
                                if self.overlay_closable && event.button == MouseButton::Left {
                                    window.close_sheet(cx);
                                    on_close(&ClickEvent::default(), window, cx);
                                }
                            }
                        })
                    })
                    .child(
                        v_flex()
                            .id("sheet")
                            .debug_selector(|| "sheet-surface".into())
                            .key_context(CONTEXT)
                            .track_focus(&self.focus_handle)
                            .focus_trap("sheet", &self.focus_handle)
                            .on_action({
                                let on_close = self.on_close.clone();
                                move |_: &Cancel, window, cx| {
                                    cx.propagate();

                                    window.close_sheet(cx);
                                    on_close(&ClickEvent::default(), window, cx);
                                }
                            })
                            .absolute()
                            .occlude()
                            .bg(cx.theme().background)
                            .border_color(cx.theme().border)
                            .shadow_xl()
                            .refine_style(&self.style)
                            .map(|this| {
                                // Set the size of the sheet.
                                if placement.is_horizontal() {
                                    this.w(self.size)
                                } else {
                                    this.h(self.size)
                                }
                            })
                            .map(|this| match self.placement {
                                Placement::Top => this.top(top).left_0().right_0().border_b_1(),
                                Placement::Right => this.top(top).right_0().bottom_0().border_l_1(),
                                Placement::Bottom => {
                                    this.bottom_0().left_0().right_0().border_t_1()
                                }
                                Placement::Left => this.top(top).left_0().bottom_0().border_r_1(),
                            })
                            .child(
                                // TitleBar
                                h_flex()
                                    .justify_between()
                                    .pl_4()
                                    .pr_3()
                                    .py_2()
                                    .w_full()
                                    .font_semibold()
                                    .child(self.title.unwrap_or(div().into_any_element()))
                                    .child(
                                        Button::new("close")
                                            .small()
                                            .ghost()
                                            .icon(IconName::Close)
                                            .on_click(move |_, window, cx| {
                                                window.close_sheet(cx);
                                                on_close(&ClickEvent::default(), window, cx);
                                            }),
                                    ),
                            )
                            .child(
                                div().flex_1().overflow_hidden().child(
                                    // Body
                                    v_flex()
                                        .size_full()
                                        .overflow_y_scrollbar()
                                        .pl(paddings.left)
                                        .pr(paddings.right)
                                        .children(self.children),
                                ),
                            )
                            .when_some(self.footer, |this, footer| {
                                // Footer
                                this.child(
                                    h_flex()
                                        .justify_between()
                                        .px_4()
                                        .py_3()
                                        .w_full()
                                        .child(footer),
                                )
                            })
                            .on_any_mouse_down({
                                |_, _, cx| {
                                    cx.stop_propagation();
                                }
                            })
                            .map(move |this| match slide_animation {
                                Some(anim) => this
                                    .with_animation(
                                        SharedString::from(format!("slide-{}", u8::from(closing))),
                                        anim,
                                        move |this, delta| {
                                            // Enter slides in from the edge;
                                            // closing runs the same path back out.
                                            let progress =
                                                if closing { 1.0 - delta } else { delta };
                                            let y = -travel + progress * travel;
                                            this.map(|this| match placement {
                                                Placement::Top => this.top(top + y),
                                                Placement::Right => this.right(y),
                                                Placement::Bottom => this.bottom(y),
                                                Placement::Left => this.left(y),
                                            })
                                        },
                                    )
                                    .into_any_element(),
                                None => this.into_any_element(),
                            }),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{AppContext as _, Empty, TestAppContext, Tiling, size};

    #[gpui::test]
    fn geometry_sheet_placements_stay_inside_root_frame(cx: &mut TestAppContext) {
        cx.update(|cx| {
            crate::init(cx);
            cx.set_reduce_motion(true);
        });
        let (root, cx) = cx.add_window_view(|window, cx| {
            let content = cx.new(|_| Empty);
            crate::Root::new(content, window, cx)
        });
        cx.simulate_resize(size(px(800.), px(600.)));
        let top = cx.update(|_, cx| cx.theme().sheet.margin_top);
        for placement in [
            Placement::Top,
            Placement::Right,
            Placement::Bottom,
            Placement::Left,
        ] {
            root.update_in(cx, |root, window, cx| {
                root.open_sheet_at(placement, |sheet, _, _| sheet, window, cx);
            });
            cx.update(|window, cx| window.draw(cx).clear(cx));
            assert_eq!(
                cx.debug_bounds("sheet-frame").unwrap(),
                Bounds::new(point(px(0.), px(0.)), size(px(800.), px(600.))),
            );
            let surface = cx.debug_bounds("sheet-surface").unwrap();
            let expected = match placement {
                Placement::Top => Bounds::new(point(px(0.), top), size(px(800.), px(350.))),
                Placement::Right => {
                    Bounds::new(point(px(450.), top), size(px(350.), px(600.) - top))
                }
                Placement::Bottom => Bounds::new(point(px(0.), px(250.)), size(px(800.), px(350.))),
                Placement::Left => Bounds::new(point(px(0.), top), size(px(350.), px(600.) - top)),
            };
            assert_eq!(surface, expected);
        }
    }

    #[test]
    fn geometry_sheet_matches_client_frame_for_every_tiled_edge() {
        for mask in 0..16 {
            let tiling = Tiling {
                top: mask & 1 != 0,
                right: mask & 2 != 0,
                bottom: mask & 4 != 0,
                left: mask & 8 != 0,
            };
            let bounds = sheet_content_bounds(
                size(px(800.), px(600.)),
                Decorations::Client { tiling },
                Edges::all(px(12.)),
            );
            assert_eq!(bounds.top(), if tiling.top { px(0.) } else { px(13.) });
            assert_eq!(bounds.left(), if tiling.left { px(0.) } else { px(13.) });
            assert_eq!(
                bounds.right(),
                if tiling.right { px(800.) } else { px(787.) }
            );
            assert_eq!(
                bounds.bottom(),
                if tiling.bottom { px(600.) } else { px(587.) }
            );
        }
    }

    #[test]
    fn geometry_sheet_server_frame_has_no_client_insets() {
        assert_eq!(
            sheet_content_bounds(
                size(px(800.), px(600.)),
                Decorations::Server,
                Edges::all(px(12.)),
            ),
            Bounds::new(point(px(0.), px(0.)), size(px(800.), px(600.))),
        );
    }

    #[test]
    fn geometry_sheet_client_frame_without_shadow_retains_only_border() {
        assert_eq!(
            sheet_content_bounds(
                size(px(800.), px(600.)),
                Decorations::Client {
                    tiling: Tiling::default()
                },
                Edges::all(px(0.)),
            ),
            Bounds::new(point(px(1.), px(1.)), size(px(798.), px(598.))),
        );
    }
}
