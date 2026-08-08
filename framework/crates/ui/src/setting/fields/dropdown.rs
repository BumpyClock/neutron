use std::rc::Rc;

use gpui::{
    AnyElement, App, Corner, IntoElement, SharedString, StyleRefinement, Styled, Window,
    prelude::FluentBuilder as _,
};

use crate::{
    AxisExt, Sizable, StyledExt,
    button::Button,
    menu::{DropdownMenu, PopupMenu, PopupMenuItem},
    setting::RenderOptions,
};

#[derive(Clone)]
pub struct DropdownSettingControl {
    options: Vec<(SharedString, SharedString)>,
    style: StyleRefinement,
    value: Rc<dyn Fn(&App) -> SharedString>,
    set_value: Rc<dyn Fn(SharedString, &mut App)>,
    default_value: Option<SharedString>,
}

impl DropdownSettingControl {
    pub(crate) fn new<V, S>(
        options: Vec<(SharedString, SharedString)>,
        value: V,
        set_value: S,
    ) -> Self
    where
        V: Fn(&App) -> SharedString + 'static,
        S: Fn(SharedString, &mut App) + 'static,
    {
        Self {
            options,
            style: StyleRefinement::default(),
            value: Rc::new(value),
            set_value: Rc::new(set_value),
            default_value: None,
        }
    }

    pub fn default_value(mut self, default_value: impl Into<SharedString>) -> Self {
        self.default_value = Some(default_value.into());
        self
    }

    pub(crate) fn style_mut(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }

    pub(crate) fn render(
        &self,
        options: &RenderOptions,
        _window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let selected_value = (self.value)(cx);
        let set_value = self.set_value.clone();
        let dropdown_options = self.options.clone();

        let selected_label = dropdown_options
            .iter()
            .find(|(value, _)| *value == selected_value)
            .map(|(_, label)| label.clone())
            .unwrap_or_else(|| selected_value.clone());

        Button::new("btn")
            .when(options.layout.is_vertical(), |this| this.w_full())
            .label(selected_label)
            .dropdown_caret(true)
            .outline()
            .with_size(options.size)
            .refine_style(&self.style)
            .dropdown_menu_with_anchor(Corner::TopRight, move |menu: PopupMenu, _, _| {
                dropdown_options.iter().fold(menu, |menu, (value, label)| {
                    let checked = &selected_value == value;
                    menu.item(
                        PopupMenuItem::new(label.clone())
                            .checked(checked)
                            .on_click({
                                let value = value.clone();
                                let set_value = set_value.clone();
                                move |_, _, cx| {
                                    set_value(value.clone(), cx);
                                }
                            }),
                    )
                })
            })
            .into_any_element()
    }

    pub(crate) fn is_resettable(&self, cx: &App) -> bool {
        self.default_value
            .as_ref()
            .is_some_and(|default_value| (self.value)(cx) != *default_value)
    }

    pub(crate) fn reset(&self, _window: &mut Window, cx: &mut App) {
        let Some(default_value) = self.default_value.clone() else {
            return;
        };

        (self.set_value)(default_value, cx);
    }
}

impl Styled for DropdownSettingControl {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}
