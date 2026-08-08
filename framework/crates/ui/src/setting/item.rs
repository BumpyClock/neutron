use gpui::{
    AnyElement, App, Axis, Div, InteractiveElement as _, IntoElement, ParentElement, SharedString,
    Stateful, Styled, Window, div, prelude::FluentBuilder as _,
};
use std::rc::Rc;

use crate::{
    ActiveTheme as _, AxisExt, StyledExt as _,
    label::Label,
    setting::{RenderOptions, SettingControl},
    text::Text,
    v_flex,
};

/// Setting item.
#[derive(Clone)]
pub enum SettingItem {
    /// A normal setting item with a title, description, and field.
    Item {
        title: SharedString,
        description: Option<Text>,
        layout: Axis,
        field: Box<SettingControl>,
    },
    /// A full custom element to render.
    Element {
        render: Rc<dyn Fn(&RenderOptions, &mut Window, &mut App) -> AnyElement + 'static>,
    },
}

impl SettingItem {
    /// Create a new setting item.
    pub fn new<F>(title: impl Into<SharedString>, field: F) -> Self
    where
        F: Into<SettingControl>,
    {
        SettingItem::Item {
            title: title.into(),
            description: None,
            layout: Axis::Horizontal,
            field: Box::new(field.into()),
        }
    }

    /// Create a new custom element setting item with a render closure.
    pub fn render<R, E>(render: R) -> Self
    where
        E: IntoElement,
        R: Fn(&RenderOptions, &mut Window, &mut App) -> E + 'static,
    {
        SettingItem::Element {
            render: Rc::new(move |options, window, cx| {
                render(options, window, cx).into_any_element()
            }),
        }
    }

    /// Set the description of the setting item.
    ///
    /// Only applies to [`SettingItem::Item`].
    pub fn description(mut self, description: impl Into<Text>) -> Self {
        match &mut self {
            SettingItem::Item { description: d, .. } => {
                *d = Some(description.into());
            }
            SettingItem::Element { .. } => {}
        }
        self
    }

    /// Set the layout of the setting item.
    ///
    /// Only applies to [`SettingItem::Item`].
    pub fn layout(mut self, layout: Axis) -> Self {
        match &mut self {
            SettingItem::Item { layout: l, .. } => {
                *l = layout;
            }
            SettingItem::Element { .. } => {}
        }
        self
    }

    pub(crate) fn is_match(&self, query: &str, cx: &App) -> bool {
        match self {
            SettingItem::Item {
                title, description, ..
            } => {
                title.to_lowercase().contains(&query.to_lowercase())
                    || description.as_ref().map_or(false, |d| {
                        d.get_text(cx)
                            .to_lowercase()
                            .contains(&query.to_lowercase())
                    })
            }
            // We need to show all custom elements when not searching.
            SettingItem::Element { .. } => query.is_empty(),
        }
    }

    pub(crate) fn is_resettable(&self, cx: &App) -> bool {
        match self {
            SettingItem::Item { field, .. } => field.is_resettable(cx),
            SettingItem::Element { .. } => false,
        }
    }

    pub(crate) fn reset(&self, window: &mut Window, cx: &mut App) {
        match self {
            SettingItem::Item { field, .. } => field.reset(window, cx),
            SettingItem::Element { .. } => {}
        }
    }

    pub(super) fn render_item(
        self,
        options: &RenderOptions,
        window: &mut Window,
        cx: &mut App,
    ) -> Stateful<Div> {
        div()
            .id(SharedString::from(format!("item-{}", options.item_ix)))
            .w_full()
            .child(match self {
                SettingItem::Item {
                    title,
                    description,
                    layout,
                    field,
                } => div()
                    .w_full()
                    .overflow_hidden()
                    .map(|this| {
                        if layout.is_horizontal() {
                            this.h_flex().justify_between().items_start()
                        } else {
                            this.v_flex()
                        }
                    })
                    .gap_3()
                    .child(
                        v_flex()
                            .map(|this| {
                                if layout.is_horizontal() {
                                    this.flex_1().max_w_3_5()
                                } else {
                                    this.w_full()
                                }
                            })
                            .gap_1()
                            .child(Label::new(title).text_sm())
                            .when_some(description, |this, description| {
                                this.child(
                                    div()
                                        .size_full()
                                        .text_sm()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(description),
                                )
                            }),
                    )
                    .child(div().id("field").child(field.render_control(
                        &RenderOptions { layout, ..*options },
                        window,
                        cx,
                    )))
                    .into_any_element(),
                SettingItem::Element { render } => {
                    (render)(&options, window, cx).into_any_element()
                }
            })
    }
}

#[cfg(test)]
mod tests {
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
    };

    use super::*;
    use gpui::{AppContext as _, Empty, SharedString, TestAppContext};

    fn with_window(cx: &mut TestAppContext, f: impl FnOnce(&mut Window, &mut App)) {
        let window = cx.update(|cx| {
            cx.open_window(Default::default(), |_, cx| cx.new(|_| Empty))
                .expect("test window")
        });

        cx.update(|cx| {
            window
                .update(cx, |_, window, cx| f(window, cx))
                .expect("window update");
        });
    }

    fn item_is_resettable(cx: &mut TestAppContext, item: &SettingItem) -> bool {
        cx.update(|cx| item.is_resettable(cx))
    }

    #[gpui::test]
    fn test_switch_control_resets_to_default(cx: &mut TestAppContext) {
        let value = Rc::new(Cell::new(true));
        let item = SettingItem::new(
            "Enabled",
            SettingControl::switch(
                {
                    let value = value.clone();
                    move |_| value.get()
                },
                {
                    let value = value.clone();
                    move |next, _| value.set(next)
                },
            )
            .default_value(false),
        );

        assert!(item_is_resettable(cx, &item));

        with_window(cx, |window, cx| item.reset(window, cx));

        assert!(!value.get());
        assert!(!item_is_resettable(cx, &item));
    }

    #[gpui::test]
    fn test_input_control_resets_to_default(cx: &mut TestAppContext) {
        let value = Rc::new(RefCell::new(SharedString::from("/tmp/custom")));
        let item = SettingItem::new(
            "Path",
            SettingControl::input(
                {
                    let value = value.clone();
                    move |_| value.borrow().clone()
                },
                {
                    let value = value.clone();
                    move |next, _| *value.borrow_mut() = next
                },
            )
            .default_value("/usr/local/bin/bash"),
        );

        assert!(item_is_resettable(cx, &item));

        with_window(cx, |window, cx| item.reset(window, cx));

        assert_eq!(&*value.borrow(), "/usr/local/bin/bash");
        assert!(!item_is_resettable(cx, &item));
    }

    #[gpui::test]
    fn test_dropdown_control_resets_to_default(cx: &mut TestAppContext) {
        let value = Rc::new(RefCell::new(SharedString::from("dark")));
        let item = SettingItem::new(
            "Mode",
            SettingControl::dropdown(
                vec![
                    ("system".into(), "System".into()),
                    ("light".into(), "Light".into()),
                    ("dark".into(), "Dark".into()),
                ],
                {
                    let value = value.clone();
                    move |_| value.borrow().clone()
                },
                {
                    let value = value.clone();
                    move |next, _| *value.borrow_mut() = next
                },
            )
            .default_value("system"),
        );

        assert!(item_is_resettable(cx, &item));

        with_window(cx, |window, cx| item.reset(window, cx));

        assert_eq!(&*value.borrow(), "system");
        assert!(!item_is_resettable(cx, &item));
    }

    #[gpui::test]
    fn test_number_control_resets_to_default(cx: &mut TestAppContext) {
        let value = Rc::new(Cell::new(18.0));
        let item = SettingItem::new(
            "Font Size",
            SettingControl::number_input(
                crate::setting::NumberFieldOptions {
                    min: 8.0,
                    max: 72.0,
                    step: 1.0,
                },
                {
                    let value = value.clone();
                    move |_| value.get()
                },
                {
                    let value = value.clone();
                    move |next, _| value.set(next)
                },
            )
            .default_value(14.0),
        );

        assert!(item_is_resettable(cx, &item));

        with_window(cx, |window, cx| item.reset(window, cx));

        assert_eq!(value.get(), 14.0);
        assert!(!item_is_resettable(cx, &item));
    }

    #[gpui::test]
    fn test_custom_control_is_not_resettable(cx: &mut TestAppContext) {
        let item = SettingItem::new(
            "Custom",
            SettingControl::render(|_, _, _| div().child("custom field")),
        );

        assert!(!item_is_resettable(cx, &item));
    }
}
