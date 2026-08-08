use gpui::{
    Animation, AnimationExt as _, AnyElement, App, Axis, IntoElement, ParentElement as _,
    SharedString, Styled as _, Window, div, px, spring,
};
use std::time::Duration;

use crate::{ActiveTheme as _, ThemeMotion, global_state::GlobalState};

/// Returns whether motion should be reduced for the current component context.
pub fn reduced_motion(cx: &App) -> bool {
    GlobalState::global(cx).reduced_motion()
}

/// A cubic bezier function like CSS `cubic-bezier`.
///
/// Builder:
///
/// https://cubic-bezier.com
pub fn cubic_bezier(x1: f32, y1: f32, x2: f32, y2: f32) -> impl Fn(f32) -> f32 {
    move |t: f32| {
        if !t.is_finite() {
            return 0.0;
        }
        let t = t.clamp(0.0, 1.0);
        let one_t = 1.0 - t;
        let one_t2 = one_t * one_t;
        let t2 = t * t;
        let t3 = t2 * t;

        // The Bezier curve function for x and y, where x0 = 0, y0 = 0, x3 = 1, y3 = 1
        let _x = 3.0 * x1 * one_t2 * t + 3.0 * x2 * one_t * t2 + t3;
        let y = 3.0 * y1 * one_t2 * t + 3.0 * y2 * one_t * t2 + t3;

        if y.is_finite() {
            y.clamp(0.0, 1.0)
        } else {
            0.0
        }
    }
}

/// A cubic bezier function without clamping the output.
pub fn cubic_bezier_unbounded(x1: f32, y1: f32, x2: f32, y2: f32) -> impl Fn(f32) -> f32 {
    move |t: f32| {
        if !t.is_finite() {
            return 0.0;
        }
        let t = t.clamp(0.0, 1.0);
        let one_t = 1.0 - t;
        let one_t2 = one_t * one_t;
        let t2 = t * t;
        let t3 = t2 * t;

        let _x = 3.0 * x1 * one_t2 * t + 3.0 * x2 * one_t * t2 + t3;
        let y = 3.0 * y1 * one_t2 * t + 3.0 * y2 * one_t * t2 + t3;
        if y.is_finite() { y } else { 0.0 }
    }
}

/// Parse a CSS cubic-bezier string into (x1, y1, x2, y2).
pub fn parse_cubic_bezier_easing(value: &str) -> Option<(f32, f32, f32, f32)> {
    let trimmed = value.trim();
    let body = trimmed
        .strip_prefix("cubic-bezier(")?
        .strip_suffix(')')?
        .trim();
    let mut parts = body.split(',').map(str::trim);
    let x1 = parts.next()?.parse::<f32>().ok()?;
    let y1 = parts.next()?.parse::<f32>().ok()?;
    let x2 = parts.next()?.parse::<f32>().ok()?;
    let y2 = parts.next()?.parse::<f32>().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((x1, y1, x2, y2))
}

/// Apply a theme easing string to an Animation.
pub fn animation_with_theme_easing(animation: Animation, easing: &str) -> Animation {
    if easing.trim().eq_ignore_ascii_case("linear") {
        return animation.with_easing(|delta: f32| delta);
    }
    if let Some((x1, y1, x2, y2)) = parse_cubic_bezier_easing(easing) {
        let overshoot = y1 < 0.0 || y1 > 1.0 || y2 < 0.0 || y2 > 1.0;
        if overshoot {
            return animation.with_unbounded_easing(cubic_bezier_unbounded(x1, y1, x2, y2));
        }
        return animation.with_easing(cubic_bezier(x1, y1, x2, y2));
    }
    animation
}

/// Create a theme animation with the given duration and easing. Returns None if reduced_motion.
pub fn theme_animation(duration_ms: u16, easing: &str, reduced_motion: bool) -> Option<Animation> {
    if reduced_motion {
        return None;
    }
    let anim = Animation::new(Duration::from_millis(duration_ms as u64));
    Some(animation_with_theme_easing(anim, easing))
}

/// Enter animation: `enter` duration on the decelerate curve. For opacity/reveal
/// of surfaces that are appearing.
pub fn enter_animation(motion: &ThemeMotion, reduced_motion: bool) -> Option<Animation> {
    theme_animation(
        motion.enter_duration_ms,
        &motion.decelerate_easing,
        reduced_motion,
    )
}

/// Exit animation: `exit` duration on the standard curve. The one dismiss
/// motion — presence close windows must use [`exit_duration`] so this can play
/// to completion before its element unmounts. Decelerating rather than
/// accelerating: an accelerating opacity fade dumps most of its change into the
/// final frame and reads as a snap at 60Hz (measured ~5x the per-frame delta of
/// this curve's tail).
pub fn exit_animation(motion: &ThemeMotion, reduced_motion: bool) -> Option<Animation> {
    theme_animation(
        motion.exit_duration_ms,
        &motion.standard_easing,
        reduced_motion,
    )
}

/// Point-to-point animation: `enter` duration on the standard curve. For moves
/// between two on-screen states (switch knobs, progress, widths).
pub fn standard_animation(motion: &ThemeMotion, reduced_motion: bool) -> Option<Animation> {
    theme_animation(
        motion.enter_duration_ms,
        &motion.standard_easing,
        reduced_motion,
    )
}

/// Fade animation (83ms, linear) for micro state changes.
pub fn fade_animation(motion: &ThemeMotion, reduced_motion: bool) -> Option<Animation> {
    theme_animation(motion.fade_duration_ms, &motion.fade_easing, reduced_motion)
}

/// Emphasis animation: long overshoot curve for attention accents.
pub fn emphasis_animation(motion: &ThemeMotion, reduced_motion: bool) -> Option<Animation> {
    theme_animation(
        motion.emphasis_duration_ms,
        &motion.emphasis_easing,
        reduced_motion,
    )
}

/// The enter window as a [`Duration`] — how long a presence must keep an
/// entering element mounted (also the spring settle window).
pub fn enter_duration(motion: &ThemeMotion) -> Duration {
    Duration::from_millis(u64::from(motion.enter_duration_ms))
}

/// Two frames of headroom after the exit animation before unmount: presence
/// timers and the animation clock do not start on the same frame, and without
/// slack the last frames of a fade get clipped (measured as a visible pop).
/// The extra frames render at opacity 0, so the slack is invisible.
const EXIT_UNMOUNT_GRACE_MS: u16 = 33;

/// The exit window as a [`Duration`] — how long a presence must keep an exiting
/// element mounted so [`exit_animation`] plays to completion. Slightly longer
/// than the animation itself; see [`EXIT_UNMOUNT_GRACE_MS`].
pub fn exit_duration(motion: &ThemeMotion) -> Duration {
    // Widen before adding: exit_duration_ms is deserialized from theme JSON, so
    // a value near u16::MAX would otherwise wrap to a near-zero exit window and
    // unmount the element on its first frame.
    Duration::from_millis(u64::from(motion.exit_duration_ms) + u64::from(EXIT_UNMOUNT_GRACE_MS))
}

/// The one spring, for transform-only reveal motion. Settles within the `enter`
/// window. Unbounded easing: pair it with transforms, not opacity.
pub fn spring_animation(motion: &ThemeMotion, reduced_motion: bool) -> Option<Animation> {
    if reduced_motion {
        return None;
    }

    Some(
        Animation::new(enter_duration(motion))
            .with_unbounded_easing(spring(motion.spring_damping_ratio, motion.spring_frequency)),
    )
}

/// Shared open/close durations for expand-collapse patterns.
pub fn expand_collapse_durations(motion: &ThemeMotion) -> (Duration, Duration) {
    (enter_duration(motion), exit_duration(motion))
}

/// Shared layout animation for expand-collapse wrappers.
pub fn expand_collapse_layout_animation(
    motion: &ThemeMotion,
    reduced_motion: bool,
    entering: bool,
) -> Option<Animation> {
    if entering {
        enter_animation(motion, reduced_motion)
    } else {
        exit_animation(motion, reduced_motion)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresencePhase {
    Entering,
    Entered,
    Exiting,
    Exited,
}

#[derive(Clone, Copy, Debug)]
pub struct PresenceTransition {
    pub phase: PresencePhase,
}

impl PresenceTransition {
    pub fn transition_active(self) -> bool {
        matches!(self.phase, PresencePhase::Entering | PresencePhase::Exiting)
    }

    pub fn should_render(self) -> bool {
        self.phase != PresencePhase::Exited
    }

    pub fn progress(self, delta: f32) -> f32 {
        let delta = delta.clamp(0.0, 1.0);
        match self.phase {
            PresencePhase::Entering => delta,
            PresencePhase::Exiting => 1.0 - delta,
            PresencePhase::Entered => 1.0,
            PresencePhase::Exited => 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PresenceOptions {
    pub animate_on_mount: bool,
}

/// Shared mount/open/close presence state machine keyed by element id.
///
/// - `target_open=true` moves to Entering/Entered
/// - `target_open=false` moves to Exiting/Exited
/// - stale async timers are ignored via generation guard
#[allow(clippy::too_many_arguments)]
pub fn keyed_presence(
    key_base: SharedString,
    target_open: bool,
    animate: bool,
    open_duration: Duration,
    close_duration: Duration,
    options: PresenceOptions,
    window: &mut Window,
    cx: &mut App,
) -> PresenceTransition {
    let initial_open = if options.animate_on_mount && animate {
        false
    } else {
        target_open
    };
    let target_key = SharedString::from(format!("{}-presence-target", key_base));
    let phase_key = SharedString::from(format!("{}-presence-phase", key_base));
    let generation_key = SharedString::from(format!("{}-presence-generation", key_base));
    let target_state = window.use_keyed_state(target_key, cx, |_, _| initial_open);
    let phase_state = window.use_keyed_state(phase_key, cx, |_, _| {
        if initial_open {
            PresencePhase::Entered
        } else {
            PresencePhase::Exited
        }
    });
    let generation_state = window.use_keyed_state(generation_key, cx, |_, _| 0_u64);

    let previous_target = *target_state.read(cx);
    let target_changed = previous_target != target_open;
    if target_changed {
        target_state.update(cx, |state, _| *state = target_open);
        let generation = generation_state.update(cx, |state, _| {
            *state += 1;
            *state
        });

        if !animate {
            let next_phase = if target_open {
                PresencePhase::Entered
            } else {
                PresencePhase::Exited
            };
            phase_state.update(cx, |state, _| *state = next_phase);
        } else if target_open {
            phase_state.update(cx, |state, _| *state = PresencePhase::Entering);
            cx.spawn({
                let target_state = target_state.clone();
                let phase_state = phase_state.clone();
                let generation_state = generation_state.clone();
                async move |cx| {
                    cx.background_executor().timer(open_duration).await;
                    let still_latest = generation_state.update(cx, |state, _| *state == generation);
                    if !still_latest {
                        return;
                    }
                    let still_open = target_state.update(cx, |state, _| *state);
                    if still_open {
                        _ = phase_state.update(cx, |state, cx| {
                            *state = PresencePhase::Entered;
                            cx.notify();
                        });
                    }
                }
            })
            .detach();
        } else {
            phase_state.update(cx, |state, _| *state = PresencePhase::Exiting);
            cx.spawn({
                let target_state = target_state.clone();
                let phase_state = phase_state.clone();
                let generation_state = generation_state.clone();
                async move |cx| {
                    cx.background_executor().timer(close_duration).await;
                    let still_latest = generation_state.update(cx, |state, _| *state == generation);
                    if !still_latest {
                        return;
                    }
                    let still_closed = target_state.update(cx, |state, _| !*state);
                    if still_closed {
                        _ = phase_state.update(cx, |state, cx| {
                            *state = PresencePhase::Exited;
                            cx.notify();
                        });
                    }
                }
            })
            .detach();
        }
    }

    PresenceTransition {
        phase: *phase_state.read(cx),
    }
}

/// The slide a flyout travels as it enters and exits, in logical pixels.
///
/// `direction` is +1.0 when the surface opens away from its trigger along the
/// positive axis (downward / rightward) and -1.0 when flipped.
#[derive(Clone, Copy, Debug)]
pub struct FlyoutSlide {
    pub axis: Axis,
    pub direction: f32,
    pub enter_distance: f32,
    pub exit_distance: f32,
}

impl FlyoutSlide {
    /// Vertical slide with the shared flyout distances (4px in, 2px out).
    pub fn vertical(direction: f32) -> Self {
        Self {
            axis: Axis::Vertical,
            direction,
            enter_distance: 4.0,
            exit_distance: 2.0,
        }
    }

    /// Horizontal slide with the submenu distances (6px both ways).
    pub fn horizontal(direction: f32) -> Self {
        Self {
            axis: Axis::Horizontal,
            direction,
            enter_distance: 6.0,
            exit_distance: 6.0,
        }
    }

    /// Override the exit travel distance.
    pub fn exit_distance(mut self, distance: f32) -> Self {
        self.exit_distance = distance;
        self
    }

    fn offset(&self, distance: f32, progress: f32) -> gpui::Pixels {
        px(distance * (1.0 - progress) * self.direction)
    }
}

/// The shared presence for flyout surfaces: enter/exit windows come from the
/// motion tokens (so an exit always outlives its animation) and reduced motion
/// applies the final state immediately.
pub fn flyout_presence(
    key: SharedString,
    open: bool,
    options: PresenceOptions,
    window: &mut Window,
    cx: &mut App,
) -> PresenceTransition {
    let motion = cx.theme().motion.clone();
    let reduced_motion = GlobalState::global(cx).reduced_motion();
    keyed_presence(
        key,
        open,
        !reduced_motion,
        enter_duration(&motion),
        exit_duration(&motion),
        options,
        window,
        cx,
    )
}

/// The one flyout open/close motion, shared by popover, context menu, submenu,
/// select popup and hover card: enter is the spring sliding from the trigger
/// plus a standard fade, exit is a standard fade sliding back.
///
/// Wrap the surface element with this at the point where its host already
/// paints it (inside any `deferred`/`anchored` — wrappers outside `anchored`
/// have no visual effect). `id_base` scopes the animation element ids per
/// surface instance.
pub fn flyout_motion(
    id_base: impl Into<SharedString>,
    presence: PresenceTransition,
    slide: FlyoutSlide,
    motion: &ThemeMotion,
    reduced_motion: bool,
    el: impl IntoElement,
) -> AnyElement {
    if !presence.transition_active() {
        return el.into_any_element();
    }

    let id_base = id_base.into();
    if matches!(presence.phase, PresencePhase::Entering) {
        let transformed = if let Some(anim) = spring_animation(motion, reduced_motion) {
            div()
                .child(el.into_any_element())
                .with_animation(
                    SharedString::from(format!("{}-open-transform", id_base)),
                    anim,
                    move |el, delta| match slide.axis {
                        Axis::Vertical => el.translate_y(slide.offset(slide.enter_distance, delta)),
                        Axis::Horizontal => {
                            el.translate_x(slide.offset(slide.enter_distance, delta))
                        }
                    },
                )
                .into_any_element()
        } else {
            el.into_any_element()
        };
        if let Some(anim) = standard_animation(motion, reduced_motion) {
            div()
                .child(transformed)
                .with_animation(
                    SharedString::from(format!("{}-open-fade", id_base)),
                    anim,
                    move |el, delta| el.opacity(presence.progress(delta).clamp(0.0, 1.0)),
                )
                .into_any_element()
        } else {
            transformed
        }
    } else if let Some(anim) = exit_animation(motion, reduced_motion) {
        div()
            .child(el.into_any_element())
            .with_animation(
                SharedString::from(format!("{}-close", id_base)),
                anim,
                move |el, delta| {
                    let progress = presence.progress(delta).clamp(0.0, 1.0);
                    let offset = slide.offset(slide.exit_distance, progress);
                    let el = el.opacity(progress);
                    match slide.axis {
                        Axis::Vertical => el.translate_y(offset),
                        Axis::Horizontal => el.translate_x(offset),
                    }
                },
            )
            .into_any_element()
    } else {
        el.into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        cubic_bezier, cubic_bezier_unbounded, parse_cubic_bezier_easing, spring_animation,
    };
    use crate::ThemeMotion;

    #[test]
    fn strong_invoke_curve_is_bounded() {
        let easing = cubic_bezier(0.13, 1.62, 0.0, 0.92);
        for i in 0..=1_000 {
            let t = i as f32 / 1_000.0;
            let y = easing(t);
            assert!(
                (0.0..=1.0).contains(&y),
                "expected output in [0, 1], got {y} at t={t}"
            );
        }
    }

    #[test]
    fn cubic_bezier_non_finite_input_returns_zero() {
        let easing = cubic_bezier(0.0, 0.0, 1.0, 1.0);
        assert_eq!(easing(f32::NAN), 0.0);
        assert_eq!(easing(f32::INFINITY), 0.0);
        assert_eq!(easing(f32::NEG_INFINITY), 0.0);
    }

    #[test]
    fn strong_invoke_curve_is_unbounded_for_transform_use() {
        let easing = cubic_bezier_unbounded(0.13, 1.62, 0.0, 0.92);
        let mut peak = f32::MIN;
        for i in 0..=1_000 {
            let t = i as f32 / 1_000.0;
            peak = peak.max(easing(t));
        }
        assert!(
            peak > 1.0,
            "expected overshoot above 1.0 for unbounded curve, got peak={peak}"
        );
    }

    #[test]
    fn parse_cubic_bezier_validation() {
        assert_eq!(
            parse_cubic_bezier_easing("cubic-bezier(0.13, 1.62, 0, 0.92)"),
            Some((0.13, 1.62, 0.0, 0.92))
        );
        assert_eq!(parse_cubic_bezier_easing("linear"), None);
        assert_eq!(
            parse_cubic_bezier_easing("cubic-bezier(0.1, 0.2, 0.3)"),
            None
        );
    }

    #[test]
    fn spring_respects_reduced_motion() {
        let motion = ThemeMotion::default();
        assert!(spring_animation(&motion, true).is_none());
        assert!(spring_animation(&motion, false).is_some());
    }
}
