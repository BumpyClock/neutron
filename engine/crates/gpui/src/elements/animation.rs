use std::{
    rc::Rc,
    time::{Duration, Instant},
};

use crate::{
    AnyElement, App, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement,
    ParentElement, SpringAnimation, SpringConfig, SpringPlayback, SpringState, SpringTarget,
    Window,
};

pub use easing::*;
use smallvec::SmallVec;

/// An animation that can be applied to an element.
#[derive(Clone)]
pub struct Animation {
    /// The amount of time for which this animation should run
    pub duration: Duration,
    /// Whether to repeat this animation when it finishes
    pub oneshot: bool,
    /// A function that takes a delta between 0 and 1 and returns a new delta
    /// between 0 and 1 based on the given easing function.
    pub easing: Rc<dyn Fn(f32) -> f32>,
    /// Bounds for the easing output. Defaults to [0, 1].
    pub easing_bounds: EasingBounds,
}

#[derive(Clone, Copy, Debug, PartialEq)]
/// Bounds behavior for easing output validation.
pub enum EasingBounds {
    /// Output must stay within [0, 1].
    Bounded,
    /// Output may be any finite value (spring/overshoot).
    Unbounded,
    /// Output must stay within the provided range.
    Range {
        /// Inclusive minimum output value.
        min: f32,
        /// Inclusive maximum output value.
        max: f32,
    },
}

impl Animation {
    /// Create a new animation with the given duration.
    /// By default the animation will only run once and will use a linear easing function.
    pub fn new(duration: Duration) -> Self {
        Self {
            duration,
            oneshot: true,
            easing: Rc::new(linear),
            easing_bounds: EasingBounds::Bounded,
        }
    }

    /// Set the animation to loop when it finishes.
    pub fn repeat(mut self) -> Self {
        self.oneshot = false;
        self
    }

    /// Set the easing function to use for this animation.
    /// The easing function will take a time delta between 0 and 1 and return a new delta
    /// between 0 and 1
    pub fn with_easing(mut self, easing: impl Fn(f32) -> f32 + 'static) -> Self {
        self.easing = Rc::new(easing);
        self.easing_bounds = EasingBounds::Bounded;
        self
    }

    /// Set an easing function that may return values outside [0, 1].
    /// Use only for transform-like properties (scale/translate/rotate).
    pub fn with_unbounded_easing(mut self, easing: impl Fn(f32) -> f32 + 'static) -> Self {
        self.easing = Rc::new(easing);
        self.easing_bounds = EasingBounds::Unbounded;
        self
    }

    /// Override easing output bounds.
    pub fn with_easing_bounds(mut self, min: f32, max: f32) -> Self {
        self.easing_bounds = EasingBounds::Range { min, max };
        self
    }
}

/// An extension trait for adding the animation wrapper to both Elements and Components
pub trait AnimationExt {
    /// Render this component or element with an animation
    fn with_animation(
        self,
        id: impl Into<ElementId>,
        animation: Animation,
        animator: impl Fn(Self, f32) -> Self + 'static,
    ) -> AnimationElement<Self>
    where
        Self: Sized,
    {
        AnimationElement {
            id: id.into(),
            element: Some(self),
            animator: Box::new(move |this, _, value| animator(this, value)),
            animations: smallvec::smallvec![animation],
        }
    }

    /// Render this component or element with a chain of animations
    fn with_animations(
        self,
        id: impl Into<ElementId>,
        animations: Vec<Animation>,
        animator: impl Fn(Self, usize, f32) -> Self + 'static,
    ) -> AnimationElement<Self>
    where
        Self: Sized,
    {
        AnimationElement {
            id: id.into(),
            element: Some(self),
            animator: Box::new(animator),
            animations: animations.into(),
        }
    }

    /// Renders this component or element at the value produced by a spring.
    ///
    /// The element ID preserves position and velocity across target changes.
    /// A newly mounted spring starts at its target unless configured with
    /// [`SpringAnimation::from`].
    fn with_spring<T>(
        self,
        id: impl Into<ElementId>,
        animation: SpringAnimation<T>,
        animator: impl FnOnce(Self, T::Output) -> Self + 'static,
    ) -> SpringAnimationElement<Self>
    where
        Self: Sized,
        T: SpringTarget,
        T::Output: 'static,
    {
        let SpringAnimation {
            config,
            target,
            epsilon,
            initial,
            playback,
        } = animation;
        let scalar_target = target.target();
        SpringAnimationElement {
            id: id.into(),
            element: Some(self),
            config,
            target: scalar_target,
            epsilon,
            initial,
            playback,
            animator: Some(Box::new(move |this, value| {
                animator(this, target.resolve(value))
            })),
        }
    }
}

impl<E: IntoElement + 'static> AnimationExt for E {}

/// A GPUI element that applies an animation to another element
pub struct AnimationElement<E> {
    id: ElementId,
    element: Option<E>,
    animations: SmallVec<[Animation; 1]>,
    animator: Box<dyn Fn(E, usize, f32) -> E + 'static>,
}

/// A GPUI element driven by a stateful spring.
pub struct SpringAnimationElement<E> {
    id: ElementId,
    element: Option<E>,
    config: SpringConfig,
    target: f32,
    epsilon: f32,
    initial: Option<f32>,
    playback: SpringPlayback,
    animator: Option<Box<dyn FnOnce(E, f32) -> E + 'static>>,
}

impl<E: ParentElement> ParentElement for SpringAnimationElement<E> {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        let Some(element) = &mut self.element else {
            return;
        };

        element.extend(elements);
    }
}

impl<E> SpringAnimationElement<E> {
    /// Returns a new [`SpringAnimationElement<E>`] after applying the given function
    /// to the element being animated.
    pub fn map_element(mut self, f: impl FnOnce(E) -> E) -> SpringAnimationElement<E> {
        self.element = self.element.map(f);
        self
    }
}

impl<E: IntoElement + 'static> IntoElement for SpringAnimationElement<E> {
    type Element = SpringAnimationElement<E>;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl<E: ParentElement> ParentElement for AnimationElement<E> {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        let Some(element) = &mut self.element else {
            return;
        };

        element.extend(elements);
    }
}

impl<E> AnimationElement<E> {
    /// Returns a new [`AnimationElement<E>`] after applying the given function
    /// to the element being animated.
    pub fn map_element(mut self, f: impl FnOnce(E) -> E) -> AnimationElement<E> {
        self.element = self.element.map(f);
        self
    }
}

impl<E: IntoElement + 'static> IntoElement for AnimationElement<E> {
    type Element = AnimationElement<E>;

    fn into_element(self) -> Self::Element {
        self
    }
}

struct AnimationState {
    start: Instant,
    animation_ix: usize,
}

struct SpringElementState {
    spring: SpringState,
    target: f32,
    config: SpringConfig,
    initial: f32,
    playback: SpringPlayback,
    updated_at: Instant,
}

impl<E: IntoElement + 'static> Element for SpringAnimationElement<E> {
    type RequestLayoutState = AnyElement;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (crate::LayoutId, Self::RequestLayoutState) {
        window.with_element_state(global_id.unwrap(), |state, window| {
            let now = cx.background_executor().now();
            let initial = self.initial.unwrap_or(self.target);
            let mut state = state.unwrap_or_else(|| SpringElementState {
                spring: SpringState {
                    position: initial,
                    velocity: 0.0,
                },
                target: self.target,
                config: self.config,
                initial,
                playback: self.playback,
                updated_at: now,
            });

            let elapsed = now.duration_since(state.updated_at).as_secs_f32();
            match state.playback {
                SpringPlayback::Running => {
                    state.spring = state.config.step(state.spring, state.target, elapsed);
                }
                SpringPlayback::Paused
                | SpringPlayback::Stopped
                | SpringPlayback::Completed
                | SpringPlayback::Cancelled => {}
            }

            state.config = self.config;
            state.target = self.target;

            let done = match self.playback {
                SpringPlayback::Running => {
                    if cx.reduce_motion() {
                        state.spring = SpringState {
                            position: state.target,
                            velocity: 0.0,
                        };
                        true
                    } else {
                        let done =
                            state
                                .config
                                .is_settled(state.spring, state.target, self.epsilon);
                        if done {
                            state.spring = SpringState {
                                position: state.target,
                                velocity: 0.0,
                            };
                        }
                        done
                    }
                }
                SpringPlayback::Paused => true,
                SpringPlayback::Stopped => {
                    state.spring.velocity = 0.0;
                    true
                }
                SpringPlayback::Completed => {
                    state.spring = SpringState {
                        position: state.target,
                        velocity: 0.0,
                    };
                    true
                }
                SpringPlayback::Cancelled => {
                    state.spring = SpringState {
                        position: state.initial,
                        velocity: 0.0,
                    };
                    true
                }
            };
            state.playback = self.playback;
            state.updated_at = now;

            let element = self.element.take().expect("should only be called once");
            let animator = self.animator.take().expect("should only be called once");
            let mut element = animator(element, state.spring.position).into_any_element();

            if !done {
                window.request_animation_frame();
            }

            ((element.request_layout(window, cx), element), state)
        })
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: crate::Bounds<crate::Pixels>,
        element: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        element.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: crate::Bounds<crate::Pixels>,
        element: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        element.paint(window, cx);
    }
}

impl<E: IntoElement + 'static> Element for AnimationElement<E> {
    type RequestLayoutState = AnyElement;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (crate::LayoutId, Self::RequestLayoutState) {
        window.with_element_state(global_id.unwrap(), |state, window| {
            let mut state = state.unwrap_or_else(|| AnimationState {
                start: Instant::now(),
                animation_ix: 0,
            });
            let animation_ix = state.animation_ix;

            let mut delta = state.start.elapsed().as_secs_f32()
                / self.animations[animation_ix].duration.as_secs_f32();

            let mut done = false;
            if delta > 1.0 {
                if self.animations[animation_ix].oneshot {
                    if animation_ix >= self.animations.len() - 1 {
                        done = true;
                    } else {
                        state.start = Instant::now();
                        state.animation_ix += 1;
                    }
                    delta = 1.0;
                } else {
                    delta %= 1.0;
                }
            }
            let delta = (self.animations[animation_ix].easing)(delta);
            match self.animations[animation_ix].easing_bounds {
                EasingBounds::Bounded => {
                    debug_assert!(
                        (0.0..=1.0).contains(&delta),
                        "delta should always be between 0 and 1"
                    );
                }
                EasingBounds::Unbounded => {
                    debug_assert!(delta.is_finite(), "delta must be finite");
                }
                EasingBounds::Range { min, max } => {
                    debug_assert!(
                        (min..=max).contains(&delta),
                        "delta should always be between {} and {}",
                        min,
                        max
                    );
                }
            }

            let element = self.element.take().expect("should only be called once");
            let mut element = (self.animator)(element, animation_ix, delta).into_any_element();

            if !done {
                window.request_animation_frame();
            }

            ((element.request_layout(window, cx), element), state)
        })
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: crate::Bounds<crate::Pixels>,
        element: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        element.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: crate::Bounds<crate::Pixels>,
        element: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        element.paint(window, cx);
    }
}

mod easing {
    use crate::{SpringConfig, SpringState};
    use std::f32::consts::PI;

    /// The linear easing function, or delta itself
    pub fn linear(delta: f32) -> f32 {
        delta
    }

    /// The quadratic easing function, delta * delta
    pub fn quadratic(delta: f32) -> f32 {
        delta * delta
    }

    /// The quadratic ease-in-out function, which starts and ends slowly but speeds up in the middle
    pub fn ease_in_out(delta: f32) -> f32 {
        if delta < 0.5 {
            2.0 * delta * delta
        } else {
            let x = -2.0 * delta + 2.0;
            1.0 - x * x / 2.0
        }
    }

    /// The Quint ease-out function, which starts quickly and decelerates to a stop
    pub fn ease_out_quint() -> impl Fn(f32) -> f32 {
        move |delta| 1.0 - (1.0 - delta).powi(5)
    }

    /// Apply the given easing function, first in the forward direction and then in the reverse direction
    pub fn bounce(easing: impl Fn(f32) -> f32) -> impl Fn(f32) -> f32 {
        move |delta| {
            if delta < 0.5 {
                easing(delta * 2.0)
            } else {
                easing((1.0 - delta) * 2.0)
            }
        }
    }

    /// Damped spring easing that may overshoot.
    /// Use with `Animation::with_unbounded_easing` for transform-like properties.
    pub fn spring(damping_ratio: f32, frequency: f32) -> impl Fn(f32) -> f32 {
        let damping_ratio = damping_ratio.clamp(0.0, 1.0);
        let frequency = frequency.max(0.01);
        let natural_frequency = frequency * 2.0 * PI;
        let config = SpringConfig::new(
            natural_frequency * natural_frequency,
            2.0 * damping_ratio * natural_frequency,
            1.0,
        );
        let initial_state = SpringState::default();
        let endpoint = config.step(initial_state, 1.0, 1.0).position;
        let degenerate = endpoint.abs() < 1e-4;

        move |delta| {
            if degenerate {
                1.0
            } else {
                let t = delta.clamp(0.0, 1.0);
                config.step(initial_state, 1.0, t).position / endpoint
            }
        }
    }

    /// A custom easing function for pulsating alpha that slows down as it approaches 0.1
    pub fn pulsating_between(min: f32, max: f32) -> impl Fn(f32) -> f32 {
        let range = max - min;

        move |delta| {
            // Use a combination of sine and cubic functions for a more natural breathing rhythm
            let t = (delta * 2.0 * PI).sin();
            let breath = (t * t * t + t) / 2.0;

            // Map the breath to our desired alpha range
            let normalized_alpha = (breath + 1.0) / 2.0;

            min + (normalized_alpha * range)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, f32::consts::PI, rc::Rc, time::Duration};

    use crate::{
        Context, InteractiveElement, ParentElement, Pixels, Render, SpringAnimation, SpringConfig,
        TestAppContext, WindowHandle, div, prelude::*, px, size,
    };

    use super::*;

    struct SpringAnimationTestView {
        target: Pixels,
        initial: Option<Pixels>,
        playback: SpringPlayback,
        rendered_values: Rc<RefCell<Vec<Pixels>>>,
    }

    impl Render for SpringAnimationTestView {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            let rendered_values = self.rendered_values.clone();
            let mut animation = SpringAnimation::new(SpringConfig::new(100.0, 2.0, 1.0))
                .to(self.target)
                .with_epsilon(0.01)
                .playback(self.playback);
            if let Some(initial) = self.initial {
                animation = animation.from(initial);
            }

            div().with_spring("spring-animation", animation, move |this, value| {
                rendered_values.borrow_mut().push(value);
                this.left(value)
            })
        }
    }

    fn draw_window<V: Render>(window: &WindowHandle<V>, cx: &mut TestAppContext) {
        cx.update_window(window.any_handle, |_, window, cx| window.draw(cx).clear(cx))
            .unwrap();
        cx.run_until_parked();
    }

    fn canonical_spring_value(damping_ratio: f32, frequency: f32, delta: f32) -> f32 {
        let damping_ratio = damping_ratio.clamp(0.0, 1.0);
        let frequency = frequency.max(0.01);
        let natural_frequency = frequency * 2.0 * PI;
        let config = SpringConfig::new(
            natural_frequency * natural_frequency,
            2.0 * damping_ratio * natural_frequency,
            1.0,
        );
        let initial_state = SpringState::default();
        let endpoint = config.step(initial_state, 1.0, 1.0).position;
        if endpoint.abs() < 1e-4 {
            1.0
        } else {
            config
                .step(initial_state, 1.0, delta.clamp(0.0, 1.0))
                .position
                / endpoint
        }
    }

    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 1e-5,
            "actual {actual} differs from expected {expected}"
        );
    }

    #[test]
    fn animated_parent_accepts_children() {
        div()
            .id("animated-parent")
            .with_animation(
                "animation",
                Animation::new(Duration::from_secs(1)),
                |el, _| el,
            )
            .child(div());
    }

    #[test]
    fn spring_animated_parent_accepts_children() {
        div()
            .id("spring-parent")
            .with_spring(
                "spring-animation",
                SpringAnimation::new(SpringConfig::new(100.0, 10.0, 1.0))
                    .to(px(10.0))
                    .from(px(0.0)),
                |element, value| element.left(value),
            )
            .child(div());
    }

    #[test]
    fn spring_easing_matches_canonical_step_when_underdamped() {
        let easing = spring(0.35, 0.75);
        for delta in [0.0, 0.125, 0.25, 0.5, 0.75, 1.0] {
            assert_close(easing(delta), canonical_spring_value(0.35, 0.75, delta));
        }

        assert_close(easing(0.25), 0.4274547);
        assert_close(easing(0.5), 1.0033011);
        assert_close(easing(0.75), 1.1593617);
        assert!(easing(0.75) > 1.0);
    }

    #[test]
    fn spring_easing_matches_canonical_step_at_critical_damping() {
        let easing = spring(1.0, 0.75);
        for delta in [0.0, 0.125, 0.25, 0.5, 0.75, 1.0] {
            assert_close(easing(delta), canonical_spring_value(1.0, 0.75, delta));
        }

        assert_close(easing(0.25), 0.34726247);
        assert_close(easing(0.5), 0.7187843);
        assert_close(easing(0.75), 0.9146271);
    }

    #[test]
    fn spring_easing_clamps_inputs_and_normalizes_endpoints() {
        let easing = spring(-1.0, 0.0);
        let clamped = spring(0.0, 0.01);
        for delta in [-1.0, 0.0, 0.5, 1.0, 2.0] {
            assert_close(easing(delta), clamped(delta));
        }
        assert_eq!(easing(-1.0), 0.0);
        assert_eq!(easing(1.0), 1.0);
        assert_eq!(easing(2.0), 1.0);

        let over_damped_input = spring(2.0, 0.75);
        let critical = spring(1.0, 0.75);
        for delta in [0.0, 0.25, 0.5, 1.0] {
            assert_close(over_damped_input(delta), critical(delta));
        }
    }

    #[test]
    fn spring_easing_uses_fallback_for_degenerate_endpoint() {
        let easing = spring(0.0, 1.0);
        for delta in [-1.0, 0.0, 0.25, 0.5, 1.0, 2.0] {
            assert_eq!(easing(delta), 1.0);
        }
    }

    #[gpui::test]
    fn spring_animation_preserves_velocity_when_retargeted(cx: &mut TestAppContext) {
        let rendered_values = Rc::new(RefCell::new(Vec::new()));
        let window = cx.open_window(size(px(100.0), px(100.0)), {
            let rendered_values = rendered_values.clone();
            move |_, _| SpringAnimationTestView {
                target: px(0.0),
                initial: None,
                playback: SpringPlayback::Running,
                rendered_values,
            }
        });
        cx.run_until_parked();
        assert_eq!(*rendered_values.borrow(), vec![px(0.0)]);

        window
            .update(cx, |view, _, cx| {
                view.target = px(100.0);
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();

        cx.executor().advance_clock(Duration::from_millis(50));
        draw_window(&window, cx);
        let value_before_retargeting = *rendered_values.borrow().last().unwrap();
        assert!(value_before_retargeting > px(0.0));
        assert!(value_before_retargeting < px(100.0));

        window
            .update(cx, |view, _, cx| {
                view.target = px(0.0);
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
        draw_window(&window, cx);
        let value_at_retargeting = *rendered_values.borrow().last().unwrap();

        cx.executor().advance_clock(Duration::from_millis(5));
        draw_window(&window, cx);
        let value_after_retargeting = *rendered_values.borrow().last().unwrap();
        assert!(value_after_retargeting > value_at_retargeting);
    }

    #[gpui::test]
    fn paused_spring_resumes_with_its_velocity(cx: &mut TestAppContext) {
        let rendered_values = Rc::new(RefCell::new(Vec::new()));
        let window = cx.open_window(size(px(100.0), px(100.0)), {
            let rendered_values = rendered_values.clone();
            move |_, _| SpringAnimationTestView {
                target: px(0.0),
                initial: None,
                playback: SpringPlayback::Running,
                rendered_values,
            }
        });
        cx.run_until_parked();

        window
            .update(cx, |view, _, cx| {
                view.target = px(100.0);
                cx.notify();
            })
            .unwrap();
        draw_window(&window, cx);
        cx.executor().advance_clock(Duration::from_millis(50));
        draw_window(&window, cx);

        window
            .update(cx, |view, _, cx| {
                view.target = px(0.0);
                view.playback = SpringPlayback::Paused;
                cx.notify();
            })
            .unwrap();
        draw_window(&window, cx);
        let paused_value = *rendered_values.borrow().last().unwrap();

        cx.executor().advance_clock(Duration::from_millis(500));
        draw_window(&window, cx);
        assert_eq!(*rendered_values.borrow().last().unwrap(), paused_value);

        window
            .update(cx, |view, _, cx| {
                view.playback = SpringPlayback::Running;
                cx.notify();
            })
            .unwrap();
        draw_window(&window, cx);
        cx.executor().advance_clock(Duration::from_millis(5));
        draw_window(&window, cx);
        assert!(*rendered_values.borrow().last().unwrap() > paused_value);
    }

    #[gpui::test]
    fn stopped_spring_resumes_without_velocity(cx: &mut TestAppContext) {
        let rendered_values = Rc::new(RefCell::new(Vec::new()));
        let window = cx.open_window(size(px(100.0), px(100.0)), {
            let rendered_values = rendered_values.clone();
            move |_, _| SpringAnimationTestView {
                target: px(0.0),
                initial: None,
                playback: SpringPlayback::Running,
                rendered_values,
            }
        });
        cx.run_until_parked();

        window
            .update(cx, |view, _, cx| {
                view.target = px(1_000_000.0);
                cx.notify();
            })
            .unwrap();
        draw_window(&window, cx);
        cx.executor().advance_clock(Duration::from_millis(50));
        draw_window(&window, cx);

        window
            .update(cx, |view, _, cx| {
                view.target = px(0.0);
                view.playback = SpringPlayback::Stopped;
                cx.notify();
            })
            .unwrap();
        draw_window(&window, cx);
        let stopped_value = *rendered_values.borrow().last().unwrap();

        cx.executor().advance_clock(Duration::from_millis(500));
        draw_window(&window, cx);
        assert_eq!(*rendered_values.borrow().last().unwrap(), stopped_value);

        window
            .update(cx, |view, _, cx| {
                view.target = stopped_value;
                view.playback = SpringPlayback::Running;
                cx.notify();
            })
            .unwrap();
        draw_window(&window, cx);
        cx.executor().advance_clock(Duration::from_millis(5));
        draw_window(&window, cx);
        assert_eq!(*rendered_values.borrow().last().unwrap(), stopped_value);
    }

    #[gpui::test]
    fn cancelled_and_completed_springs_resolve_their_endpoints(cx: &mut TestAppContext) {
        let rendered_values = Rc::new(RefCell::new(Vec::new()));
        let window = cx.open_window(size(px(100.0), px(100.0)), {
            let rendered_values = rendered_values.clone();
            move |_, _| SpringAnimationTestView {
                target: px(100.0),
                initial: Some(px(20.0)),
                playback: SpringPlayback::Running,
                rendered_values,
            }
        });
        cx.run_until_parked();
        assert_eq!(*rendered_values.borrow(), vec![px(20.0)]);

        cx.executor().advance_clock(Duration::from_millis(50));
        draw_window(&window, cx);
        assert!(*rendered_values.borrow().last().unwrap() > px(20.0));

        window
            .update(cx, |view, _, cx| {
                view.playback = SpringPlayback::Cancelled;
                cx.notify();
            })
            .unwrap();
        draw_window(&window, cx);
        assert_eq!(*rendered_values.borrow().last().unwrap(), px(20.0));

        window
            .update(cx, |view, _, cx| {
                view.playback = SpringPlayback::Completed;
                cx.notify();
            })
            .unwrap();
        draw_window(&window, cx);
        assert_eq!(*rendered_values.borrow().last().unwrap(), px(100.0));
    }

    #[gpui::test]
    fn spring_animation_respects_reduced_motion(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_reduce_motion(true));
        let rendered_values = Rc::new(RefCell::new(Vec::new()));
        let _window = cx.open_window(size(px(100.0), px(100.0)), {
            let rendered_values = rendered_values.clone();
            move |_, _| SpringAnimationTestView {
                target: px(100.0),
                initial: Some(px(0.0)),
                playback: SpringPlayback::Running,
                rendered_values,
            }
        });
        cx.run_until_parked();

        assert_eq!(*rendered_values.borrow(), vec![px(100.0)]);
    }
}
