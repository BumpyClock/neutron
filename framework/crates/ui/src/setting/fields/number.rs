use std::rc::Rc;

use gpui::{
    AnyElement, App, AppContext as _, Entity, IntoElement, SharedString, StyleRefinement, Styled,
    Subscription, Window, prelude::FluentBuilder as _,
};

use crate::{
    AxisExt, Sizable, StyledExt,
    input::{InputEvent, InputState, NumberInput, NumberInputEvent, StepAction},
    setting::RenderOptions,
};

#[derive(Clone, Debug)]
pub struct NumberFieldOptions {
    /// The minimum value for the number input, default is `f64::MIN`.
    pub min: f64,
    /// The maximum value for the number input, default is `f64::MAX`.
    pub max: f64,
    /// The step value for the number input, default is `1.0`.
    pub step: f64,
}

impl Default for NumberFieldOptions {
    fn default() -> Self {
        Self {
            min: f64::MIN,
            max: f64::MAX,
            step: 1.0,
        }
    }
}

#[derive(Clone)]
pub struct NumberInputSettingControl {
    options: NumberFieldOptions,
    style: StyleRefinement,
    value: Rc<dyn Fn(&App) -> f64>,
    set_value: Rc<dyn Fn(f64, &mut App)>,
    default_value: Option<f64>,
}

struct State {
    input: Entity<InputState>,
    initial_value: f64,
    _subscriptions: Vec<Subscription>,
}

impl NumberInputSettingControl {
    pub(crate) fn new<V, S>(options: NumberFieldOptions, value: V, set_value: S) -> Self
    where
        V: Fn(&App) -> f64 + 'static,
        S: Fn(f64, &mut App) + 'static,
    {
        Self {
            options,
            style: StyleRefinement::default(),
            value: Rc::new(value),
            set_value: Rc::new(set_value),
            default_value: None,
        }
    }

    pub fn default_value(mut self, default_value: f64) -> Self {
        self.default_value = Some(default_value);
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
        let num_options = self.options.clone();

        let state = window
            .use_keyed_state(
                SharedString::from(format!(
                    "number-state-{}-{}-{}",
                    options.page_ix, options.group_ix, options.item_ix
                )),
                cx,
                |window, cx| {
                    let input =
                        cx.new(|cx| InputState::new(window, cx).default_value(value.to_string()));
                    let _subscriptions = vec![
                        cx.subscribe_in(&input, window, {
                            let num_options = num_options.clone();
                            move |_, input, event: &NumberInputEvent, window, cx| match event {
                                NumberInputEvent::Step(action) => input.update(cx, |input, cx| {
                                    let value = input.value();
                                    if let Ok(value) = value.parse::<f64>() {
                                        let new_value = if *action == StepAction::Increment {
                                            value + num_options.step
                                        } else {
                                            value - num_options.step
                                        };
                                        input.set_value(
                                            SharedString::from(new_value.to_string()),
                                            window,
                                            cx,
                                        );
                                    }
                                }),
                            }
                        }),
                        cx.subscribe_in(&input, window, {
                            move |state: &mut State, input, event: &InputEvent, window, cx| {
                                if let InputEvent::Change = event {
                                    input.update(cx, |input, cx| {
                                        let value = input.value();
                                        if value == state.initial_value.to_string() {
                                            return;
                                        }

                                        if let Ok(value) = value.parse::<f64>() {
                                            let clamped_value =
                                                value.clamp(num_options.min, num_options.max);

                                            set_value(clamped_value, cx);
                                            state.initial_value = clamped_value;
                                            if clamped_value != value {
                                                input.set_value(
                                                    SharedString::from(clamped_value.to_string()),
                                                    window,
                                                    cx,
                                                );
                                            }
                                        }
                                    });
                                }
                            }
                        }),
                    ];

                    State {
                        input,
                        initial_value: value,
                        _subscriptions,
                    }
                },
            )
            .read(cx);

        NumberInput::new(&state.input)
            .with_size(options.size)
            .map(|this| {
                if options.layout.is_horizontal() {
                    this.w_32()
                } else {
                    this.w_full()
                }
            })
            .refine_style(&self.style)
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

impl Styled for NumberInputSettingControl {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}
