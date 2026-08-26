use std::ops::Range;

use crate::{ActiveTheme, AxisExt, ElementExt, StyledExt, h_flex};
use gpui::{
    Along, App, AppContext as _, Axis, Background, Bounds, Context, Corners, DefiniteLength,
    DragMoveEvent, Empty, Entity, EntityId, EventEmitter, Hsla, InteractiveElement, IntoElement,
    MouseButton, MouseDownEvent, ParentElement as _, Pixels, Point, Render, RenderOnce,
    StatefulInteractiveElement as _, StyleRefinement, Styled, Window, div,
    prelude::FluentBuilder as _, px, relative,
};

#[derive(Clone)]
struct DragThumb((EntityId, bool));

impl Render for DragThumb {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

#[derive(Clone)]
struct DragSlider(EntityId);

impl Render for DragSlider {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

/// Events emitted by the [`SliderState`].
pub enum SliderEvent {
    Change(SliderValue),
}

/// The value of the slider, can be a single value or a range of values.
///
/// - Can from a f32 value, which will be treated as a single value.
/// - Or from a (f32, f32) tuple, which will be treated as a range of values.
///
/// The default value is `SliderValue::Single(0.0)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SliderValue {
    Single(f32),
    Range(f32, f32),
}

impl std::fmt::Display for SliderValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SliderValue::Single(value) => write!(f, "{}", value),
            SliderValue::Range(start, end) => write!(f, "{}..{}", start, end),
        }
    }
}

impl From<f32> for SliderValue {
    fn from(value: f32) -> Self {
        SliderValue::Single(value)
    }
}

impl From<(f32, f32)> for SliderValue {
    fn from(value: (f32, f32)) -> Self {
        SliderValue::Range(value.0, value.1)
    }
}

impl From<Range<f32>> for SliderValue {
    fn from(value: Range<f32>) -> Self {
        SliderValue::Range(value.start, value.end)
    }
}

impl Default for SliderValue {
    fn default() -> Self {
        SliderValue::Single(0.)
    }
}

impl SliderValue {
    /// Clamp the value to the given range.
    pub fn clamp(self, min: f32, max: f32) -> Self {
        match self {
            SliderValue::Single(value) => SliderValue::Single(value.clamp(min, max)),
            SliderValue::Range(start, end) => {
                SliderValue::Range(start.clamp(min, max), end.clamp(min, max))
            }
        }
    }

    /// Check if the value is a single value.
    #[inline]
    pub fn is_single(&self) -> bool {
        matches!(self, SliderValue::Single(_))
    }

    /// Check if the value is a range of values.
    #[inline]
    pub fn is_range(&self) -> bool {
        matches!(self, SliderValue::Range(_, _))
    }

    /// Get the start value.
    pub fn start(&self) -> f32 {
        match self {
            SliderValue::Single(value) => *value,
            SliderValue::Range(start, _) => *start,
        }
    }

    /// Get the end value.
    pub fn end(&self) -> f32 {
        match self {
            SliderValue::Single(value) => *value,
            SliderValue::Range(_, end) => *end,
        }
    }

    fn is_finite(&self) -> bool {
        match self {
            SliderValue::Single(value) => value.is_finite(),
            SliderValue::Range(start, end) => start.is_finite() && end.is_finite(),
        }
    }

    fn set_start(&mut self, value: f32) {
        if let SliderValue::Range(_, end) = self {
            *self = SliderValue::Range(value.min(*end), *end);
        } else {
            *self = SliderValue::Single(value);
        }
    }

    fn set_end(&mut self, value: f32) {
        if let SliderValue::Range(start, _) = self {
            *self = SliderValue::Range(*start, value.max(*start));
        } else {
            *self = SliderValue::Single(value);
        }
    }
}

/// The scale mode of the slider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SliderScale {
    /// Linear scale where values change uniformly across the slider range.
    /// This is the default mode.
    #[default]
    Linear,
    /// Logarithmic scale where the distance between values increases exponentially.
    ///
    /// This is useful for parameters that have a large range of values where smaller
    /// changes are more significant at lower values. Common examples include:
    ///
    /// - Volume controls (human hearing perception is logarithmic)
    /// - Frequency controls (musical notes follow a logarithmic scale)
    /// - Zoom levels
    /// - Any parameter where you want finer control at lower values
    ///
    /// # For example
    ///
    /// ```
    /// use neutron_components::slider::{SliderState, SliderScale};
    ///
    /// let slider = SliderState::new()
    ///     .min(1.0)    // Must be > 0 for logarithmic scale
    ///     .max(1000.0)
    ///     .scale(SliderScale::Logarithmic);
    /// ```
    ///
    /// - Moving the slider 1/3 of the way will yield ~10
    /// - Moving it 2/3 of the way will yield ~100
    /// - The full range covers 3 orders of magnitude evenly
    Logarithmic,
}

impl SliderScale {
    #[inline]
    pub fn is_linear(&self) -> bool {
        matches!(self, SliderScale::Linear)
    }

    #[inline]
    pub fn is_logarithmic(&self) -> bool {
        matches!(self, SliderScale::Logarithmic)
    }
}

/// State of the [`Slider`].
pub struct SliderState {
    min: f32,
    max: f32,
    step: f32,
    value: SliderValue,
    /// When is single value mode, only `end` is used, the start is always 0.0.
    percentage: Range<f32>,
    /// The bounds of the slider after rendered.
    bounds: Bounds<Pixels>,
    scale: SliderScale,
}

impl SliderState {
    /// Create a new [`SliderState`].
    pub fn new() -> Self {
        Self {
            min: 0.0,
            max: 100.0,
            step: 1.0,
            value: SliderValue::default(),
            percentage: (0.0..0.0),
            bounds: Bounds::default(),
            scale: SliderScale::default(),
        }
    }

    /// Set the minimum value of the slider, default: 0.0
    pub fn min(mut self, min: f32) -> Self {
        assert!(min.is_finite(), "`min` must be finite");
        self.min = min;
        self.update_thumb_pos();
        self
    }

    /// Set the maximum value of the slider, default: 100.0
    pub fn max(mut self, max: f32) -> Self {
        assert!(max.is_finite(), "`max` must be finite");
        self.max = max;
        self.update_thumb_pos();
        self
    }

    /// Set the step value of the slider, default: 1.0
    pub fn step(mut self, step: f32) -> Self {
        assert!(
            step.is_finite() && step > 0.0,
            "`step` must be finite and greater than 0"
        );
        self.step = step;
        self.update_thumb_pos();
        self
    }

    /// Set the scale of the slider, default: [`SliderScale::Linear`].
    pub fn scale(mut self, scale: SliderScale) -> Self {
        self.scale = scale;
        self.update_thumb_pos();
        self
    }

    /// Set the default value of the slider, default: 0.0
    pub fn default_value(mut self, value: impl Into<SliderValue>) -> Self {
        let value = value.into();
        if value.is_finite() {
            self.value = value;
        }
        self.update_thumb_pos();
        self
    }

    /// Set the value of the slider.
    pub fn set_value(
        &mut self,
        value: impl Into<SliderValue>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let value = value.into();
        if !value.is_finite() {
            return;
        }
        self.value = value;
        self.update_thumb_pos();
        cx.notify();
    }

    /// Get the value of the slider.
    pub fn value(&self) -> SliderValue {
        self.value
    }

    /// Converts a value between 0.0 and 1.0 to a value between the minimum and maximum value,
    /// depending on the chosen scale.
    fn percentage_to_value(&self, percentage: f32) -> f32 {
        let fallback = if self.min.is_finite() { self.min } else { 0.0 };
        if !self.has_valid_configuration() || !percentage.is_finite() {
            return fallback;
        }
        let percentage = percentage.clamp(0.0, 1.0);
        match self.scale {
            SliderScale::Linear => {
                let min = f64::from(self.min);
                let max = f64::from(self.max);
                let value = min + (max - min) * f64::from(percentage);
                if value.is_finite() {
                    value.clamp(min, max) as f32
                } else if percentage <= 0.0 {
                    self.min
                } else {
                    self.max
                }
            }
            SliderScale::Logarithmic => {
                let min = f64::from(self.min);
                let max = f64::from(self.max);
                let min_log = min.ln();
                let max_log = max.ln();
                let value = (min_log + (max_log - min_log) * f64::from(percentage)).exp();
                if value.is_finite() {
                    value.clamp(min, max) as f32
                } else {
                    self.max
                }
            }
        }
    }

    /// Converts a value between the minimum and maximum value to a value between 0.0 and 1.0,
    /// depending on the chosen scale.
    fn value_to_percentage(&self, value: f32) -> f32 {
        if !self.has_valid_configuration() || !value.is_finite() {
            return 0.0;
        }
        let percentage = match self.scale {
            SliderScale::Linear => {
                let min = f64::from(self.min);
                let max = f64::from(self.max);
                let range = max - min;
                (f64::from(value) - min) / range
            }
            SliderScale::Logarithmic => {
                let min_log = f64::from(self.min).ln();
                let max_log = f64::from(self.max).ln();
                let range = max_log - min_log;
                if !min_log.is_finite() || !max_log.is_finite() || !range.is_finite() {
                    return 0.0;
                }
                (f64::from(value).ln() - min_log) / range
            }
        };
        if percentage.is_finite() {
            percentage.clamp(0.0, 1.0) as f32
        } else {
            0.0
        }
    }

    fn has_valid_configuration(&self) -> bool {
        self.min.is_finite()
            && self.max.is_finite()
            && self.min < self.max
            && self.step.is_finite()
            && self.step > 0.0
            && (!self.scale.is_logarithmic() || self.min > 0.0)
    }

    fn quantize_value(&self, value: f32) -> Option<f32> {
        if !self.has_valid_configuration() || !value.is_finite() {
            return None;
        }
        let step = f64::from(self.step);
        let quantized = (f64::from(value) / step).round() * step;
        let quantized = if quantized.is_finite() {
            quantized
        } else if value >= self.max {
            f64::from(self.max)
        } else if value <= self.min {
            f64::from(self.min)
        } else {
            return None;
        };
        Some(quantized.clamp(f64::from(self.min), f64::from(self.max)) as f32)
    }

    fn percentage_is_valid(&self) -> bool {
        self.percentage.start.is_finite()
            && self.percentage.end.is_finite()
            && (0.0..=1.0).contains(&self.percentage.start)
            && (0.0..=1.0).contains(&self.percentage.end)
            && self.percentage.start <= self.percentage.end
    }

    fn reset_thumb_pos(&mut self) {
        self.percentage = 0.0..0.0;
    }

    fn update_thumb_pos(&mut self) {
        if !self.has_valid_configuration() || !self.value.is_finite() {
            self.reset_thumb_pos();
            return;
        }
        match self.value {
            SliderValue::Single(value) => {
                let percentage = self.value_to_percentage(value.clamp(self.min, self.max));
                self.percentage = 0.0..percentage.clamp(0.0, 1.0);
            }
            SliderValue::Range(start, end) => {
                let clamped_start = start.clamp(self.min, self.max);
                let clamped_end = end.clamp(self.min, self.max);
                let start = self.value_to_percentage(clamped_start);
                let end = self.value_to_percentage(clamped_end);
                self.percentage = start.min(end)..start.max(end);
            }
        }
    }

    fn percentage_for_position(&self, axis: Axis, position: Point<Pixels>) -> Option<f32> {
        if !self.has_valid_configuration() || !self.percentage_is_valid() {
            return None;
        }

        let bounds = self.bounds;
        let inner_pos = if axis.is_horizontal() {
            (position.x - bounds.left()).as_f32()
        } else {
            (bounds.bottom() - position.y).as_f32()
        };
        let total_size = bounds.size.along(axis).as_f32();
        if !inner_pos.is_finite() || !total_size.is_finite() || total_size <= 0.0 {
            return None;
        }

        Some((inner_pos.clamp(0.0, total_size) / total_size).clamp(0.0, 1.0))
    }

    /// Update value by mouse position
    fn update_value_by_position(
        &mut self,
        axis: Axis,
        position: Point<Pixels>,
        is_start: bool,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(percentage) = self.percentage_for_position(axis, position) else {
            return;
        };

        let percentage = if is_start {
            percentage.clamp(0.0, self.percentage.end)
        } else {
            percentage.clamp(self.percentage.start, 1.0)
        };

        let value = self.percentage_to_value(percentage);
        let Some(value) = self.quantize_value(value) else {
            return;
        };

        if is_start {
            self.percentage.start = percentage;
            self.value.set_start(value);
        } else {
            self.percentage.end = percentage;
            self.value.set_end(value);
        }
        cx.emit(SliderEvent::Change(self.value));
        cx.notify();
    }
}

impl EventEmitter<SliderEvent> for SliderState {}

/// A Slider element.
#[derive(IntoElement)]
pub struct Slider {
    state: Entity<SliderState>,
    axis: Axis,
    style: StyleRefinement,
    disabled: bool,
}

impl Slider {
    /// Create a new [`Slider`] element bind to the [`SliderState`].
    pub fn new(state: &Entity<SliderState>) -> Self {
        Self {
            axis: Axis::Horizontal,
            state: state.clone(),
            style: StyleRefinement::default(),
            disabled: false,
        }
    }

    /// As a horizontal slider.
    pub fn horizontal(mut self) -> Self {
        self.axis = Axis::Horizontal;
        self
    }

    /// As a vertical slider.
    pub fn vertical(mut self) -> Self {
        self.axis = Axis::Vertical;
        self
    }

    /// Set the disabled state of the slider, default: false
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    #[allow(clippy::too_many_arguments)]
    fn render_thumb(
        &self,
        start: DefiniteLength,
        is_start: bool,
        bar_color: Background,
        thumb_color: Hsla,
        radius: Corners<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) -> impl gpui::IntoElement {
        let entity_id = self.state.entity_id();
        let axis = self.axis;
        let id = ("slider-thumb", is_start as u32);

        if self.disabled {
            return div().id(id);
        }

        div()
            .id(id)
            .absolute()
            .when(axis.is_horizontal(), |this| {
                this.top(px(-5.)).left(start).ml(-px(8.))
            })
            .when(axis.is_vertical(), |this| {
                this.bottom(start).left(px(-5.)).mb(-px(8.))
            })
            .flex()
            .items_center()
            .justify_center()
            .flex_shrink_0()
            .corner_radii(radius)
            .bg(bar_color.opacity(0.5))
            .when(cx.theme().shadow, |this| this.shadow_md())
            .size_4()
            .p(px(1.))
            .child(
                div()
                    .flex_shrink_0()
                    .size_full()
                    .corner_radii(radius)
                    .bg(thumb_color),
            )
            .on_mouse_down(MouseButton::Left, |_, _, cx| {
                cx.stop_propagation();
            })
            .on_drag(DragThumb((entity_id, is_start)), |drag, _, _, cx| {
                cx.stop_propagation();
                cx.new(|_| drag.clone())
            })
            .on_drag_move(window.listener_for(
                &self.state,
                move |view, e: &DragMoveEvent<DragThumb>, window, cx| {
                    match e.drag(cx) {
                        DragThumb((id, is_start)) => {
                            if *id != entity_id {
                                return;
                            }

                            // set value by mouse position
                            view.update_value_by_position(
                                axis,
                                e.event.position,
                                *is_start,
                                window,
                                cx,
                            )
                        }
                    }
                },
            ))
    }
}

impl Styled for Slider {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for Slider {
    fn render(self, window: &mut Window, cx: &mut gpui::App) -> impl IntoElement {
        let axis = self.axis;
        let entity_id = self.state.entity_id();
        let state = self.state.read(cx);
        let is_range = state.value().is_range();
        let percentage = state.percentage.clone();
        let bar_start = relative(percentage.start);
        let bar_end = relative(1. - percentage.end);
        let rem_size = window.rem_size();

        let bar_color = self
            .style
            .background
            .clone()
            .and_then(|bg| bg.color())
            .unwrap_or(cx.theme().slider_bar.into());
        let thumb_color = self
            .style
            .text
            .color
            .unwrap_or_else(|| cx.theme().slider_thumb);
        let corner_radii = self.style.corner_radii.clone();
        let default_radius = px(999.);
        let radius = Corners {
            top_left: corner_radii
                .top_left
                .map(|v| v.to_pixels(rem_size))
                .unwrap_or(default_radius),
            top_right: corner_radii
                .top_right
                .map(|v| v.to_pixels(rem_size))
                .unwrap_or(default_radius),
            bottom_left: corner_radii
                .bottom_left
                .map(|v| v.to_pixels(rem_size))
                .unwrap_or(default_radius),
            bottom_right: corner_radii
                .bottom_right
                .map(|v| v.to_pixels(rem_size))
                .unwrap_or(default_radius),
        };

        div()
            .id(("slider", self.state.entity_id()))
            .flex()
            .flex_1()
            .items_center()
            .justify_center()
            .when(axis.is_vertical(), |this| this.h(px(120.)))
            .when(axis.is_horizontal(), |this| this.w_full())
            .refine_style(&self.style)
            .bg(cx.theme().transparent)
            .text_color(cx.theme().foreground)
            .child(
                h_flex()
                    .id("slider-bar-container")
                    .when(!self.disabled, |this| {
                        this.on_mouse_down(
                            MouseButton::Left,
                            window.listener_for(
                                &self.state,
                                move |state, e: &MouseDownEvent, window, cx| {
                                    let mut is_start = false;
                                    if is_range {
                                        let bar_size = state.bounds.size.along(axis);
                                        let inner_pos = if axis.is_horizontal() {
                                            e.position.x - state.bounds.left()
                                        } else {
                                            state.bounds.bottom() - e.position.y
                                        };
                                        let center = ((percentage.end - percentage.start) / 2.0
                                            + percentage.start)
                                            * bar_size;
                                        is_start = inner_pos < center;
                                    }

                                    state.update_value_by_position(
                                        axis, e.position, is_start, window, cx,
                                    )
                                },
                            ),
                        )
                    })
                    .when(!self.disabled && !is_range, |this| {
                        this.on_drag(DragSlider(entity_id), |drag, _, _, cx| {
                            cx.stop_propagation();
                            cx.new(|_| drag.clone())
                        })
                        .on_drag_move(window.listener_for(
                            &self.state,
                            move |view, e: &DragMoveEvent<DragSlider>, window, cx| match e.drag(cx)
                            {
                                DragSlider(id) => {
                                    if *id != entity_id {
                                        return;
                                    }

                                    view.update_value_by_position(
                                        axis,
                                        e.event.position,
                                        false,
                                        window,
                                        cx,
                                    )
                                }
                            },
                        ))
                    })
                    .when(axis.is_horizontal(), |this| {
                        this.items_center().h_6().w_full()
                    })
                    .when(axis.is_vertical(), |this| {
                        this.justify_center().w_6().h_full()
                    })
                    .flex_shrink_0()
                    .child(
                        div()
                            .id("slider-bar")
                            .relative()
                            .when(axis.is_horizontal(), |this| this.w_full().h_1p5())
                            .when(axis.is_vertical(), |this| this.h_full().w_1p5())
                            .bg(bar_color.opacity(0.2))
                            .active(|this| this.bg(bar_color.opacity(0.4)))
                            .corner_radii(radius)
                            .child(
                                div()
                                    .absolute()
                                    .when(axis.is_horizontal(), |this| {
                                        this.h_full().left(bar_start).right(bar_end)
                                    })
                                    .when(axis.is_vertical(), |this| {
                                        this.w_full().bottom(bar_start).top(bar_end)
                                    })
                                    .bg(bar_color)
                                    .rounded_full(),
                            )
                            .when(is_range, |this| {
                                this.child(self.render_thumb(
                                    relative(percentage.start),
                                    true,
                                    bar_color,
                                    thumb_color,
                                    radius,
                                    window,
                                    cx,
                                ))
                            })
                            .child(self.render_thumb(
                                relative(percentage.end),
                                false,
                                bar_color,
                                thumb_color,
                                radius,
                                window,
                                cx,
                            ))
                            .on_prepaint({
                                let state = self.state.clone();
                                move |bounds, _, cx| state.update(cx, |r, _| r.bounds = bounds)
                            }),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logarithmic_builder_accepts_scale_before_bounds() {
        let slider = SliderState::new()
            .scale(SliderScale::Logarithmic)
            .min(1.0)
            .max(100.0);

        assert!(slider.percentage.start.is_finite());
        assert!(slider.percentage.end.is_finite());
        assert!((slider.percentage_to_value(0.5) - 10.0).abs() < 0.0001);
    }

    #[test]
    #[should_panic(expected = "`min` must be finite")]
    fn min_rejects_nonfinite_value() {
        SliderState::new().min(f32::NAN);
    }

    #[test]
    #[should_panic(expected = "`max` must be finite")]
    fn max_rejects_nonfinite_value() {
        SliderState::new().max(f32::INFINITY);
    }

    #[test]
    #[should_panic(expected = "`step` must be finite and greater than 0")]
    fn step_rejects_nonpositive_value() {
        SliderState::new().step(0.0);
    }

    #[test]
    fn quantized_value_is_clamped_to_slider_bounds() {
        let slider = SliderState::new().min(0.0).max(1.0).step(0.6);

        assert_eq!(slider.quantize_value(1.0), Some(1.0));
    }

    #[test]
    fn finite_extreme_bounds_preserve_linear_conversion() {
        let slider = SliderState::new().min(-f32::MAX).max(f32::MAX);

        assert_eq!(slider.percentage_to_value(0.5), 0.0);
        assert!((slider.value_to_percentage(0.0) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn zero_or_nonfinite_slider_geometry_has_no_pointer_percentage() {
        for width in [0.0, f32::NAN, f32::INFINITY] {
            let mut slider = SliderState::new();
            slider.bounds = Bounds::new(
                gpui::point(px(0.0), px(0.0)),
                gpui::size(px(width), px(20.0)),
            );

            assert_eq!(
                slider.percentage_for_position(Axis::Horizontal, gpui::point(px(0.0), px(0.0))),
                None,
                "width {width:?} must not produce a pointer percentage"
            );
        }
    }

    #[gpui::test]
    fn invalid_slider_relation_does_not_emit_change(cx: &mut gpui::TestAppContext) {
        cx.update(crate::init);
        let window = cx.update(|cx| {
            cx.open_window(Default::default(), |_, cx| cx.new(|_| gpui::Empty))
                .unwrap()
        });
        let mut visual_cx = gpui::VisualTestContext::from_window(window.into(), cx);
        let state = visual_cx.update(|_, cx| cx.new(|_| SliderState::new().min(100.0).max(0.0)));
        let changes = std::rc::Rc::new(std::cell::Cell::new(0));
        let changes_for_subscription = changes.clone();
        let _subscription = visual_cx.update(|_, cx| {
            cx.subscribe(&state, move |_, _: &SliderEvent, _| {
                changes_for_subscription.set(changes_for_subscription.get() + 1);
            })
        });

        state.update_in(&mut visual_cx, |slider, window, cx| {
            slider.bounds =
                Bounds::new(gpui::point(px(0.0), px(0.0)), gpui::size(px(0.0), px(20.0)));
            slider.update_value_by_position(
                Axis::Horizontal,
                gpui::point(px(0.0), px(0.0)),
                false,
                window,
                cx,
            );
        });

        assert!(state.read_with(&visual_cx, |slider, _| {
            slider.percentage.start.is_finite() && slider.percentage.end.is_finite()
        }));
        assert_eq!(changes.get(), 0);
    }
}
