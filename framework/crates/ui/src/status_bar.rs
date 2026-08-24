use gpui::{
    AnyElement, App, IntoElement, ParentElement, RenderOnce, StyleRefinement, Styled, Window,
    prelude::FluentBuilder as _,
};
use smallvec::SmallVec;

use crate::{ActiveTheme, StyledExt, h_flex};

/// A horizontal status bar, usually placed at the bottom of a window or pane.
///
/// It is split into three regions: `left`, `center`, and `right`. This mirrors
/// native status bars while keeping each region open to custom GPUI elements.
///
/// `left` and `right` pin items to each end. `child` and `children` add items
/// to the center region, whose alignment follows the pinned ends: centered with
/// both ends, end-aligned with only `left`, and start-aligned otherwise.
#[derive(IntoElement)]
pub struct StatusBar {
    style: StyleRefinement,
    left: SmallVec<[AnyElement; 1]>,
    right: SmallVec<[AnyElement; 1]>,
    children: SmallVec<[AnyElement; 1]>,
}

impl StatusBar {
    /// Create a new, empty status bar.
    pub fn new() -> Self {
        Self {
            style: StyleRefinement::default(),
            left: SmallVec::new(),
            right: SmallVec::new(),
            children: SmallVec::new(),
        }
    }

    /// Append an element to the left region.
    pub fn left(mut self, child: impl IntoElement) -> Self {
        self.left.push(child.into_any_element());
        self
    }

    /// Append an element to the right region.
    pub fn right(mut self, child: impl IntoElement) -> Self {
        self.right.push(child.into_any_element());
        self
    }
}

impl ParentElement for StatusBar {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl Styled for StatusBar {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for StatusBar {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let has_left = !self.left.is_empty();
        let has_right = !self.right.is_empty();
        let region = || h_flex().overflow_hidden().items_center().gap_2();

        h_flex()
            .items_center()
            .flex_shrink_0()
            .gap_2()
            .py_1()
            .px_2()
            .border_t_1()
            .border_color(cx.theme().status_bar_border)
            .bg(cx.theme().status_bar)
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .refine_style(&self.style)
            .when(has_left, |this| this.child(region().children(self.left)))
            .child(
                region()
                    .flex_1()
                    .when(has_left && has_right, |this| this.justify_center())
                    .when(has_left && !has_right, |this| this.justify_end())
                    .children(self.children),
            )
            .when(has_right, |this| this.child(region().children(self.right)))
    }
}
