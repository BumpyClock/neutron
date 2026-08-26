use std::time::Duration;

use crate::{Icon, IconName, Sizable, Size};
use gpui::{
    Animation, AnimationExt as _, AnyElement, App, Hsla, IntoElement, ParentElement, RenderOnce,
    Styled as _, Transformation, Window, div, ease_in_out, percentage, prelude::FluentBuilder as _,
};

/// A cycling loading spinner.
#[derive(IntoElement)]
pub struct Spinner {
    size: Size,
    icon: Icon,
    speed: Duration,
    color: Option<Hsla>,
}

impl Spinner {
    /// Create a new loading spinner.
    pub fn new() -> Self {
        Self {
            size: Size::Medium,
            speed: Duration::from_secs_f64(0.8),
            icon: Icon::new(IconName::Loader),
            color: None,
        }
    }

    /// Set specified icon for the spinner.
    ///
    /// Default is [`IconName::Loader`].
    ///
    /// Please ensure the icon used is suitable for a loading spinner.
    pub fn icon(mut self, icon: impl Into<Icon>) -> Self {
        self.icon = icon.into();
        self
    }

    /// Set the icon color.
    pub fn color(mut self, color: Hsla) -> Self {
        self.color = Some(color);
        self
    }
}

impl Sizable for Spinner {
    fn with_size(mut self, size: impl Into<Size>) -> Self {
        self.size = size.into();
        self
    }
}

impl Spinner {
    fn render_icon(self, cx: &mut App) -> AnyElement {
        let icon = self
            .icon
            .with_size(self.size)
            .when_some(self.color, |this, color| this.text_color(color));
        let icon = if crate::animation::reduced_motion(cx) {
            icon.into_any_element()
        } else {
            icon.with_animation(
                "circle",
                Animation::new(self.speed).repeat().with_easing(ease_in_out),
                |this, delta| this.transform(Transformation::rotate(percentage(delta))),
            )
            .into_any_element()
        };

        icon
    }
}

impl RenderOnce for Spinner {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        div().child(self.render_icon(cx)).into_element()
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, rc::Rc};

    use gpui::{App, Context, IntoElement, Render, RenderOnce, TestAppContext, Window};

    use super::*;

    #[derive(IntoElement)]
    struct SpinnerProbe {
        has_animation: Rc<Cell<bool>>,
    }

    impl RenderOnce for SpinnerProbe {
        fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
            let mut element = Spinner::new().render_icon(cx);
            self.has_animation.set(
                element
                    .downcast_mut::<gpui::AnimationElement<Icon>>()
                    .is_some(),
            );
            element
        }
    }

    struct SpinnerHarness {
        reduced_motion: bool,
        has_animation: Rc<Cell<bool>>,
    }

    impl Render for SpinnerHarness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            crate::ReducedMotionScope::new(
                self.reduced_motion,
                SpinnerProbe {
                    has_animation: self.has_animation.clone(),
                },
            )
        }
    }

    #[gpui::test]
    fn spinner_respects_framework_reduced_motion_scope(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let has_animation = Rc::new(Cell::new(false));
        let (view, cx) = cx.add_window_view({
            let has_animation = has_animation.clone();
            move |_, _| SpinnerHarness {
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
