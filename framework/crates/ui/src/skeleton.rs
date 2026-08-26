use crate::{ActiveTheme, StyledExt};
use gpui::{
    Animation, AnimationExt, IntoElement, RenderOnce, StyleRefinement, Styled, bounce, div,
    ease_in_out,
};
use std::time::Duration;

/// A skeleton loading placeholder element.
#[derive(IntoElement)]
pub struct Skeleton {
    style: StyleRefinement,
    secondary: bool,
}

impl Skeleton {
    /// Create a new Skeleton element.
    pub fn new() -> Self {
        Self {
            style: StyleRefinement::default(),
            secondary: false,
        }
    }

    /// Set use secondary color.
    pub fn secondary(mut self) -> Self {
        self.secondary = true;
        self
    }
}

impl Styled for Skeleton {
    fn style(&mut self) -> &mut gpui::StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for Skeleton {
    fn render(self, _: &mut gpui::Window, cx: &mut gpui::App) -> impl IntoElement {
        let skeleton = div()
            .w_full()
            .h_4()
            .bg(if self.secondary {
                cx.theme().skeleton.opacity(0.5)
            } else {
                cx.theme().skeleton
            })
            .refine_style(&self.style);

        if crate::animation::reduced_motion(cx) {
            skeleton.into_any_element()
        } else {
            skeleton
                .with_animation(
                    "skeleton",
                    Animation::new(Duration::from_secs(2))
                        .repeat()
                        .with_easing(bounce(ease_in_out)),
                    move |this, delta| {
                        let v = 1.0 - delta * 0.5;
                        this.opacity(v)
                    },
                )
                .into_any_element()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, rc::Rc};

    use gpui::{App, Context, IntoElement, Render, RenderOnce, TestAppContext, Window};

    use super::*;

    #[derive(IntoElement)]
    struct SkeletonProbe {
        has_animation: Rc<Cell<bool>>,
    }

    impl RenderOnce for SkeletonProbe {
        fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
            let mut element = Skeleton::new().render(window, cx).into_any_element();
            self.has_animation.set(
                element
                    .downcast_mut::<gpui::AnimationElement<gpui::Div>>()
                    .is_some(),
            );
            element
        }
    }

    struct SkeletonHarness {
        reduced_motion: bool,
        has_animation: Rc<Cell<bool>>,
    }

    impl Render for SkeletonHarness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            crate::ReducedMotionScope::new(
                self.reduced_motion,
                SkeletonProbe {
                    has_animation: self.has_animation.clone(),
                },
            )
        }
    }

    #[gpui::test]
    fn skeleton_respects_framework_reduced_motion_scope(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let has_animation = Rc::new(Cell::new(false));
        let (view, cx) = cx.add_window_view({
            let has_animation = has_animation.clone();
            move |_, _| SkeletonHarness {
                reduced_motion: true,
                has_animation,
            }
        });

        assert!(!has_animation.get());

        view.update(cx, |view, cx| {
            view.reduced_motion = false;
            cx.notify();
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));

        assert!(has_animation.get());
    }
}
