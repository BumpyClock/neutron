mod bool;
mod dropdown;
mod element;
mod number;
mod string;

pub use element::SettingFieldElement;
pub use number::NumberFieldOptions;

use gpui::{AnyElement, App, IntoElement, SharedString, StyleRefinement, Styled, Window};

use crate::setting::RenderOptions;

use self::{
    bool::{BoolFieldKind, BoolSettingControl},
    dropdown::DropdownSettingControl,
    element::ElementSettingControl,
    number::NumberInputSettingControl,
    string::InputSettingControl,
};

#[derive(Clone)]
pub enum SettingControl {
    Switch(BoolSettingControl),
    Checkbox(BoolSettingControl),
    Input(InputSettingControl),
    Dropdown(DropdownSettingControl),
    NumberInput(NumberInputSettingControl),
    Element(ElementSettingControl),
}

impl SettingControl {
    pub fn switch<V, S>(value: V, set_value: S) -> BoolSettingControl
    where
        V: Fn(&App) -> bool + 'static,
        S: Fn(bool, &mut App) + 'static,
    {
        BoolSettingControl::switch(value, set_value)
    }

    pub fn checkbox<V, S>(value: V, set_value: S) -> BoolSettingControl
    where
        V: Fn(&App) -> bool + 'static,
        S: Fn(bool, &mut App) + 'static,
    {
        BoolSettingControl::checkbox(value, set_value)
    }

    pub fn input<V, S>(value: V, set_value: S) -> InputSettingControl
    where
        V: Fn(&App) -> SharedString + 'static,
        S: Fn(SharedString, &mut App) + 'static,
    {
        InputSettingControl::new(value, set_value)
    }

    pub fn dropdown<V, S>(
        options: Vec<(SharedString, SharedString)>,
        value: V,
        set_value: S,
    ) -> DropdownSettingControl
    where
        V: Fn(&App) -> SharedString + 'static,
        S: Fn(SharedString, &mut App) + 'static,
    {
        DropdownSettingControl::new(options, value, set_value)
    }

    pub fn number_input<V, S>(
        options: NumberFieldOptions,
        value: V,
        set_value: S,
    ) -> NumberInputSettingControl
    where
        V: Fn(&App) -> f64 + 'static,
        S: Fn(f64, &mut App) + 'static,
    {
        NumberInputSettingControl::new(options, value, set_value)
    }

    pub fn element<E>(element: E) -> ElementSettingControl
    where
        E: SettingFieldElement + 'static,
    {
        ElementSettingControl::new(element)
    }

    pub fn render<E, R>(element_render: R) -> ElementSettingControl
    where
        E: IntoElement + 'static,
        R: Fn(&RenderOptions, &mut Window, &mut App) -> E + 'static,
    {
        Self::element(
            move |options: &RenderOptions, window: &mut Window, cx: &mut App| {
                element_render(options, window, cx).into_any_element()
            },
        )
    }

    pub(crate) fn render_control(
        &self,
        options: &RenderOptions,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        match self {
            SettingControl::Switch(control) | SettingControl::Checkbox(control) => {
                control.render(options, window, cx)
            }
            SettingControl::Input(control) => control.render(options, window, cx),
            SettingControl::Dropdown(control) => control.render(options, window, cx),
            SettingControl::NumberInput(control) => control.render(options, window, cx),
            SettingControl::Element(control) => control.render(options, window, cx),
        }
    }

    pub(crate) fn is_resettable(&self, cx: &App) -> bool {
        match self {
            SettingControl::Switch(control) | SettingControl::Checkbox(control) => {
                control.is_resettable(cx)
            }
            SettingControl::Input(control) => control.is_resettable(cx),
            SettingControl::Dropdown(control) => control.is_resettable(cx),
            SettingControl::NumberInput(control) => control.is_resettable(cx),
            SettingControl::Element(control) => control.is_resettable(cx),
        }
    }

    pub(crate) fn reset(&self, window: &mut Window, cx: &mut App) {
        match self {
            SettingControl::Switch(control) | SettingControl::Checkbox(control) => {
                control.reset(window, cx)
            }
            SettingControl::Input(control) => control.reset(window, cx),
            SettingControl::Dropdown(control) => control.reset(window, cx),
            SettingControl::NumberInput(control) => control.reset(window, cx),
            SettingControl::Element(control) => control.reset(window, cx),
        }
    }

    fn style_mut(&mut self) -> &mut StyleRefinement {
        match self {
            SettingControl::Switch(control) | SettingControl::Checkbox(control) => {
                control.style_mut()
            }
            SettingControl::Input(control) => control.style_mut(),
            SettingControl::Dropdown(control) => control.style_mut(),
            SettingControl::NumberInput(control) => control.style_mut(),
            SettingControl::Element(control) => control.style_mut(),
        }
    }
}

impl Styled for SettingControl {
    fn style(&mut self) -> &mut StyleRefinement {
        self.style_mut()
    }
}

impl From<BoolSettingControl> for SettingControl {
    fn from(control: BoolSettingControl) -> Self {
        match control.kind() {
            BoolFieldKind::Switch => SettingControl::Switch(control),
            BoolFieldKind::Checkbox => SettingControl::Checkbox(control),
        }
    }
}

impl From<InputSettingControl> for SettingControl {
    fn from(control: InputSettingControl) -> Self {
        SettingControl::Input(control)
    }
}

impl From<DropdownSettingControl> for SettingControl {
    fn from(control: DropdownSettingControl) -> Self {
        SettingControl::Dropdown(control)
    }
}

impl From<NumberInputSettingControl> for SettingControl {
    fn from(control: NumberInputSettingControl) -> Self {
        SettingControl::NumberInput(control)
    }
}

impl From<ElementSettingControl> for SettingControl {
    fn from(control: ElementSettingControl) -> Self {
        SettingControl::Element(control)
    }
}
