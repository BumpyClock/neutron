use gpui::{
    AnimationExt as _, AnyElement, App, IntoElement, ParentElement, RenderOnce, StyleRefinement,
    Styled, Window, prelude::FluentBuilder as _,
};

use crate::{
    ActiveTheme, StyledExt, animation::enter_animation, global_state::GlobalState, v_flex,
};

/// Generous max for animated height reveal. Content fully visible
/// well before delta=1 due to decelerating easing.
const COLLAPSIBLE_CONTENT_MAX_H: f32 = 1500.0;

enum CollapsibleChild {
    Element(AnyElement),
    Content(AnyElement),
}

/// An interactive element which expands/collapses.
#[derive(IntoElement)]
pub struct Collapsible {
    style: StyleRefinement,
    children: Vec<CollapsibleChild>,
    open: bool,
}

impl Collapsible {
    /// Creates a new `Collapsible` instance.
    pub fn new() -> Self {
        Self {
            style: StyleRefinement::default(),
            open: false,
            children: vec![],
        }
    }

    /// Sets whether the collapsible is open. default is false.
    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }

    /// Sets the content of the collapsible.
    ///
    /// If `open` is false, content will be hidden.
    pub fn content(mut self, content: impl IntoElement) -> Self {
        self.children
            .push(CollapsibleChild::Content(content.into_any_element()));
        self
    }
}

impl Styled for Collapsible {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl ParentElement for Collapsible {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children
            .extend(elements.into_iter().map(|el| CollapsibleChild::Element(el)));
    }
}

impl RenderOnce for Collapsible {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let motion = &cx.theme().motion;
        let reduced_motion = GlobalState::global(cx).reduced_motion();
        let anim = enter_animation(motion, reduced_motion);

        let mut non_content = Vec::new();
        let mut content_elements = Vec::new();

        for child in self.children {
            match child {
                CollapsibleChild::Element(el) => non_content.push(el),
                CollapsibleChild::Content(el) => {
                    if self.open {
                        content_elements.push(el);
                    }
                }
            }
        }

        v_flex()
            .refine_style(&self.style)
            .children(non_content)
            .when(self.open && !content_elements.is_empty(), |this| {
                let content_wrapper = gpui::div()
                    .overflow_hidden()
                    .child(gpui::div().children(content_elements));
                this.child(if let Some(anim) = anim {
                    content_wrapper
                        .with_animation("collapsible-expand", anim, |el, delta| {
                            el.max_h(gpui::px(COLLAPSIBLE_CONTENT_MAX_H * delta))
                                .opacity(delta)
                        })
                        .into_any_element()
                } else {
                    content_wrapper.into_any_element()
                })
            })
    }
}
