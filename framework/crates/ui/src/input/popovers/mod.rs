mod code_action_menu;
mod completion_menu;
mod context_menu;
mod diagnostic_popover;
mod hover_popover;

pub(crate) use code_action_menu::*;
pub(crate) use completion_menu::*;
pub(crate) use context_menu::*;
pub(crate) use diagnostic_popover::*;
pub(crate) use hover_popover::*;

use gpui::{
    App, Div, ElementId, Entity, InteractiveElement as _, IntoElement, SharedString, Stateful,
    StyleRefinement, Styled as _, Window, div, px, rems,
};

use crate::{
    ActiveTheme, FlyoutTokens, SurfaceContext, SurfacePreset, flyout_primary_foreground,
    text::{TextView, TextViewStyle},
};

pub(crate) enum ContextMenu {
    Completion(Entity<CompletionMenu>),
    CodeAction(Entity<CodeActionMenu>),
    MouseContext(Entity<MouseContextMenu>),
}

impl ContextMenu {
    pub(crate) fn is_open(&self, cx: &App) -> bool {
        match self {
            ContextMenu::Completion(menu) => menu.read(cx).is_open(),
            ContextMenu::CodeAction(menu) => menu.read(cx).is_open(),
            ContextMenu::MouseContext(menu) => menu.read(cx).is_open(),
        }
    }

    pub(crate) fn render(&self) -> impl IntoElement {
        match self {
            ContextMenu::Completion(menu) => menu.clone().into_any_element(),
            ContextMenu::CodeAction(menu) => menu.clone().into_any_element(),
            ContextMenu::MouseContext(menu) => menu.clone().into_any_element(),
        }
    }
}

pub(super) fn render_markdown(
    id: impl Into<ElementId>,
    markdown: impl Into<SharedString>,
    _: &mut Window,
    cx: &mut App,
) -> TextView {
    TextView::markdown(id, markdown)
        .style(
            TextViewStyle::default()
                .paragraph_gap(rems(0.5))
                .heading_font_size(|level, rem_size| match level {
                    1..=3 => rem_size * 1,
                    4 => rem_size * 0.9,
                    _ => rem_size * 0.8,
                })
                .code_block(
                    StyleRefinement::default()
                        .bg(cx.theme().transparent)
                        .p_0()
                        .text_size(px(11.)),
                ),
        )
        .selectable(true)
}

/// Container for the editor's transient surfaces (completion, code actions, mouse
/// context menu, documentation).
///
/// Shares the flyout material and geometry with the rest of the library, but stays
/// on the `meta` type step: these surfaces overlay live code, so density is
/// functional rather than decorative.
pub(super) fn editor_popover(id: impl Into<ElementId>, cx: &App) -> Stateful<Div> {
    let tokens = FlyoutTokens::new(cx);

    SurfacePreset::flyout()
        .with_radius(tokens.radius)
        .apply_material(
            div().id(id).flex_none().occlude(),
            cx,
            SurfaceContext::new(cx),
        )
        .text_color(flyout_primary_foreground(cx))
        .text_size(tokens.meta_size)
        .p(tokens.inset)
}
