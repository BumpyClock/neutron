use std::rc::Rc;

use gpui::{
    AnyElement, App, AppContext as _, Entity, IntoElement, SharedString, StyleRefinement, Styled,
    Window, prelude::FluentBuilder as _,
};

use crate::{
    AxisExt as _, Sizable, StyledExt,
    input::{Input, InputEvent, InputState},
    setting::RenderOptions,
};

#[derive(Clone)]
pub struct InputSettingControl {
    style: StyleRefinement,
    value: Rc<dyn Fn(&App) -> SharedString>,
    set_value: Rc<dyn Fn(SharedString, &mut App)>,
    default_value: Option<SharedString>,
}

struct State {
    input: Entity<InputState>,
    _subscription: gpui::Subscription,
}

impl InputSettingControl {
    pub(crate) fn new<V, S>(value: V, set_value: S) -> Self
    where
        V: Fn(&App) -> SharedString + 'static,
        S: Fn(SharedString, &mut App) + 'static,
    {
        Self {
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
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let value = (self.value)(cx);
        let set_value = self.set_value.clone();

        let state = window
            .use_keyed_state(
                SharedString::from(format!(
                    "string-state-{}-{}-{}",
                    options.page_ix, options.group_ix, options.item_ix
                )),
                cx,
                |window, cx| {
                    let input =
                        cx.new(|cx| InputState::new(window, cx).default_value(value.clone()));
                    let _subscription = cx.subscribe(&input, {
                        move |_, input, event: &InputEvent, cx| {
                            if let InputEvent::Change = event {
                                let value = input.read(cx).value();
                                set_value(value, cx);
                            }
                        }
                    });

                    State {
                        input,
                        _subscription,
                    }
                },
            )
            .read(cx);

        Input::new(&state.input)
            .with_size(options.size)
            .map(|this| {
                if options.layout.is_horizontal() {
                    this.w_64()
                } else {
                    this.w_full()
                }
            })
            .refine_style(&self.style)
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

impl Styled for InputSettingControl {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}
