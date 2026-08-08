use std::rc::Rc;

use gpui::{
    AnyElement, App, IntoElement, ParentElement as _, StyleRefinement, Styled, Window, div,
};

use crate::{StyledExt as _, setting::RenderOptions};

/// A trait for rendering custom setting field elements.
///
/// For [`crate::setting::SettingControl::element`].
pub trait SettingFieldElement {
    type Element: IntoElement + 'static;

    fn render_field(
        &self,
        options: &RenderOptions,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::Element;
}

impl<F, E> SettingFieldElement for F
where
    E: IntoElement + 'static,
    F: Fn(&RenderOptions, &mut Window, &mut App) -> E,
{
    type Element = E;

    fn render_field(
        &self,
        options: &RenderOptions,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::Element {
        (self)(options, window, cx)
    }
}

struct AnySettingFieldElement<T>(T);

impl<T> SettingFieldElement for AnySettingFieldElement<T>
where
    T: SettingFieldElement,
{
    type Element = AnyElement;

    fn render_field(
        &self,
        options: &RenderOptions,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::Element {
        self.0.render_field(options, window, cx).into_any_element()
    }
}

impl SettingFieldElement for Rc<dyn SettingFieldElement<Element = AnyElement>> {
    type Element = AnyElement;

    fn render_field(
        &self,
        options: &RenderOptions,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::Element {
        self.as_ref().render_field(options, window, cx)
    }
}

#[derive(Clone)]
pub struct ElementSettingControl {
    style: StyleRefinement,
    element_render: Rc<dyn SettingFieldElement<Element = AnyElement>>,
}

impl ElementSettingControl {
    pub(crate) fn new<E>(element_render: E) -> Self
    where
        E: SettingFieldElement + 'static,
    {
        Self {
            style: StyleRefinement::default(),
            element_render: Rc::new(AnySettingFieldElement(element_render)),
        }
    }

    pub(crate) fn style_mut(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }

    pub(crate) fn render(
        &self,
        options: &RenderOptions,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        div()
            .refine_style(&self.style)
            .child(self.element_render.render_field(options, window, cx))
            .into_any_element()
    }

    pub(crate) fn is_resettable(&self, _cx: &App) -> bool {
        false
    }

    pub(crate) fn reset(&self, _window: &mut Window, _cx: &mut App) {}
}

impl Styled for ElementSettingControl {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}
