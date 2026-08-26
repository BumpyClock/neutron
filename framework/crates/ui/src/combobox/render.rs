use gpui::{
    AnyElement, App, Bounds, ClickEvent, Edges, Entity, Hsla, InteractiveElement, IntoElement,
    Length, MouseDownEvent, ParentElement, Pixels, RenderOnce, SharedString,
    StatefulInteractiveElement, StyleRefinement, Styled, Window, anchored, div,
    prelude::FluentBuilder, px,
};

use crate::{
    ActiveTheme, ElementExt as _, FlyoutTokens, Icon, IconName, Sizable, Size, StyleSized,
    StyledExt, SurfaceContext, SurfacePreset,
    animation::{FlyoutSlide, PresenceTransition, flyout_motion},
    flyout_primary_foreground, h_flex,
    list::{List, ListState},
    searchable_list::{SearchableListAdapter, SearchableListDelegate},
    v_flex,
};

#[derive(IntoElement)]
pub struct Caret {
    size: Size,
    color: Option<Hsla>,
}

impl Caret {
    pub fn new(size: Size) -> Self {
        Self { size, color: None }
    }

    pub fn text_color(mut self, color: Hsla) -> Self {
        self.color = Some(color);
        self
    }
}

impl RenderOnce for Caret {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        Icon::new(IconName::ChevronDown)
            .with_size(match self.size {
                Size::XSmall => Size::XSmall,
                Size::Small => Size::Small,
                _ => Size::Medium,
            })
            .when_some(self.color, |this, color| this.text_color(color))
    }
}

pub(super) fn input_style(disabled: bool, cx: &App) -> (Hsla, Hsla) {
    if disabled {
        (cx.theme().muted, cx.theme().muted_foreground)
    } else {
        (cx.theme().background, cx.theme().foreground)
    }
}

/// Renders the styled trigger container.
#[allow(clippy::too_many_arguments)]
pub(super) fn render_trigger_container(
    disabled: bool,
    appearance: bool,
    size: Size,
    style: &StyleRefinement,
    bg: Hsla,
    fg: Hsla,
    outline_visible: bool,
    allow_open: bool,
    trigger_body: AnyElement,
    trailing: AnyElement,
    toggle_handler: Option<Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>>,
    prepaint_handler: Box<dyn Fn(Bounds<Pixels>, &mut Window, &mut App) + 'static>,
    cx: &mut App,
) -> impl IntoElement {
    div()
        .id("input")
        .relative()
        .flex()
        .items_center()
        .justify_between()
        .border_1()
        .border_color(cx.theme().transparent)
        .when(appearance, |this| {
            this.bg(bg)
                .text_color(fg)
                .when(disabled, |this| this.opacity(0.5))
                .border_color(cx.theme().input)
                .rounded(cx.theme().radius)
        })
        .overflow_hidden()
        .input_size(size)
        .input_text_size(size)
        .refine_style(style)
        .when(outline_visible, |this| this.focused_border(cx))
        .when(allow_open, |this| {
            this.when_some(toggle_handler, |this, handler| this.on_click(handler))
        })
        .child(
            h_flex()
                .id("inner")
                .w_full()
                .items_center()
                .justify_between()
                .gap_1()
                .child(trigger_body)
                .child(trailing),
        )
        .on_prepaint(prepaint_handler)
}

/// Renders the deferred anchored popup shell containing the searchable list and optional footer.
#[allow(clippy::too_many_arguments)]
pub(super) fn render_popup_shell<D: SearchableListDelegate + 'static>(
    list: &Entity<ListState<SearchableListAdapter<D>>>,
    menu_width: Length,
    search_placeholder: Option<SharedString>,
    size: Size,
    menu_max_h: Length,
    bounds: Bounds<Pixels>,
    footer_el: Option<AnyElement>,
    dismiss_handler: Box<dyn Fn(&MouseDownEvent, &mut Window, &mut App) + 'static>,
    presence: PresenceTransition,
    motion: &crate::theme::ThemeMotion,
    reduced_motion: bool,
    cx: &mut App,
) -> AnyElement {
    let has_footer = footer_el.is_some();
    let tokens = FlyoutTokens::sized(size, cx);
    let surface = SurfacePreset::flyout().with_radius(tokens.radius);

    anchored()
        .snap_to_window_with_margin(px(8.))
        .child(
            div()
                .occlude()
                .map(|this| match menu_width {
                    Length::Auto => this.w(bounds.size.width + px(2.)),
                    Length::Definite(w) => this.w(w),
                })
                .child(
                    surface
                        .apply_material(
                            v_flex()
                                .occlude()
                                .mt_1p5()
                                .text_color(flyout_primary_foreground(cx))
                                .child(
                                    List::new(list)
                                        .when_some(search_placeholder, |this, placeholder| {
                                            this.search_placeholder(placeholder)
                                        })
                                        .with_size(size)
                                        .max_h(menu_max_h)
                                        .paddings(Edges::all(tokens.inset)),
                                )
                                .when(has_footer, |this| {
                                    this.child(
                                        div()
                                            .border_t_1()
                                            .border_color(cx.theme().border)
                                            .p(tokens.inset)
                                            .when_some(footer_el, |this, el| this.child(el)),
                                    )
                                }),
                            cx,
                            SurfaceContext::new(cx),
                        )
                        .map(|el| {
                            flyout_motion(
                                "combobox",
                                presence,
                                FlyoutSlide::vertical(-1.0),
                                motion,
                                reduced_motion,
                                el,
                            )
                        }),
                )
                .on_mouse_down_out(dismiss_handler),
        )
        .into_any_element()
}
