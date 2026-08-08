use std::rc::Rc;

use gpui::{
    AnyElement, App, IntoElement, ParentElement as _, StyleRefinement, Styled, Window, div,
};

use crate::{Sizable, StyledExt, checkbox::Checkbox, setting::RenderOptions, switch::Switch};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum BoolFieldKind {
    Switch,
    Checkbox,
}

#[derive(Clone)]
pub struct BoolSettingControl {
    kind: BoolFieldKind,
    style: StyleRefinement,
    value: Rc<dyn Fn(&App) -> bool>,
    set_value: Rc<dyn Fn(bool, &mut App)>,
    default_value: Option<bool>,
}

impl BoolSettingControl {
    pub(crate) fn switch<V, S>(value: V, set_value: S) -> Self
    where
        V: Fn(&App) -> bool + 'static,
        S: Fn(bool, &mut App) + 'static,
    {
        Self::new(BoolFieldKind::Switch, value, set_value)
    }

    pub(crate) fn checkbox<V, S>(value: V, set_value: S) -> Self
    where
        V: Fn(&App) -> bool + 'static,
        S: Fn(bool, &mut App) + 'static,
    {
        Self::new(BoolFieldKind::Checkbox, value, set_value)
    }

    fn new<V, S>(kind: BoolFieldKind, value: V, set_value: S) -> Self
    where
        V: Fn(&App) -> bool + 'static,
        S: Fn(bool, &mut App) + 'static,
    {
        Self {
            kind,
            style: StyleRefinement::default(),
            value: Rc::new(value),
            set_value: Rc::new(set_value),
            default_value: None,
        }
    }

    pub fn default_value(mut self, default_value: bool) -> Self {
        self.default_value = Some(default_value);
        self
    }

    pub(crate) fn kind(&self) -> BoolFieldKind {
        self.kind
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
        let checked = (self.value)(cx);
        let set_value = self.set_value.clone();

        div()
            .refine_style(&self.style)
            .child(match self.kind {
                BoolFieldKind::Switch => Switch::new("check")
                    .checked(checked)
                    .with_size(options.size)
                    .on_click(move |checked: &bool, _, cx: &mut App| {
                        set_value(*checked, cx);
                    })
                    .into_any_element(),
                BoolFieldKind::Checkbox => Checkbox::new("check")
                    .checked(checked)
                    .with_size(options.size)
                    .on_click(move |checked: &bool, _, cx: &mut App| {
                        set_value(*checked, cx);
                    })
                    .into_any_element(),
            })
            .into_any_element()
    }

    pub(crate) fn is_resettable(&self, cx: &App) -> bool {
        self.default_value
            .is_some_and(|default_value| (self.value)(cx) != default_value)
    }

    pub(crate) fn reset(&self, _window: &mut Window, cx: &mut App) {
        let Some(default_value) = self.default_value else {
            return;
        };

        (self.set_value)(default_value, cx);
    }
}

impl Styled for BoolSettingControl {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}
