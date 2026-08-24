---
title: "Spring animation policy"
summary: "Notes on tokenized spring helpers and when to use stateful or duration-based motion."
read_when: "changing animation helpers, spring animation, or theme motion token usage"
---
# Spring animation policy

Date: 2026-08-23

The motion layer has two spring forms. Use GPUI's stateful
`SpringAnimation` for continuously mounted geometry that can change target.
Use the framework's duration-based helpers for presence transforms and other
animations with a fixed lifetime.

## Stateful spring

GPUI provides `SpringConfig`, `SpringAnimation`, `SpringTarget`, and
`SpringPlayback`. `with_spring` renders an element from the current spring
state. Its element ID stores position and velocity, so a target change does not
restart the motion.

`Switch` is the current framework consumer. Its thumb is continuously mounted
and can retarget when the controlled value changes. Do not use this form for a
surface that unmounts at the end of its exit transition.

## Theme conversion

```rust
use crate::animation::theme_spring_config;

let config = theme_spring_config(&cx.theme().motion);
```

`ThemeMotion` stores frequency in cycles per enter-duration window. The
framework converts it to GPUI's physical parameters with unit mass:

```text
ω = 2πf / d
k = ω²
c = 2ζω
m = 1
```

Here `d` is `enter_duration_ms / 1000`, `f` is `spring_frequency`, and `ζ` is
`spring_damping_ratio`. Non-finite or non-positive duration and non-finite
spring values use default theme tokens. Frequency is clamped to a small
positive value before conversion; damping ratio is clamped to `0..=1`.

## Duration-based spring

`animation::spring_animation` uses the same theme values to sample a spring
into a duration-based `Animation` over the enter window. It is used for fixed
presence transforms such as flyouts, dialogs, accordions, and command palette
reveals. Retargeting this form restarts the sample. `gpui::sampled_easing` has
the same restart behavior.

`enter_animation`, `exit_animation`, `standard_animation`, and `fade_animation`
remain duration-based. Exit presence uses `exit_duration` so an element stays
mounted until its close animation completes.

## Overshoot rule

Springs can produce values outside `0..=1`. Use them for transform or bounded
point-to-point geometry. Do not use spring output for:

- Opacity. Values above `1.0` are not a stable opacity contract.
- Size or max-height. Overshoot can cause layout jumps.
- Layout properties. Content can flash or overflow.

Keep opacity, size, and visibility on monotonic duration-based curves.

## Component map

| Component | Open / enter | Close / exit |
| --- | --- | --- |
| Accordion | Duration spring for transform plus enter animation | Exit animation |
| Dialog | Duration spring for transform plus enter animation | Exit animation |
| Flyout / popover | Duration spring plus standard fade | Exit animation |
| Popup menu | Duration spring plus standard fade | Exit animation |
| Command palette | Duration spring plus standard fade | Exit animation |
| Switch | Stateful spring for thumb geometry | Stateful spring |
| Sheet | Enter animation | Exit animation |
| Notification | Enter animation | Exit animation |
| Tooltip | Fade animation | — |

## Presence state machine

For open and close animations, use `keyed_presence()` from `animation.rs`:

```rust
let presence = keyed_presence(
    "my-component".into(),
    target_open,
    animate,
    open_duration,
    close_duration,
    PresenceOptions { animate_on_mount: true },
    window,
    cx,
);

if presence.should_render() {
    // Render with presence.progress(delta) for opacity or layout.
}
```

The state machine tracks `PresencePhase`:

- `Entering`: apply open animation.
- `Entered`: render the static open state.
- `Exiting`: apply close animation while the element stays mounted.
- `Exited`: unmount the element.

Reduced motion combines the GPUI engine signal with the framework signal. Use
`animation::reduced_motion(cx)` and skip both stateful and duration-based motion
when it returns `true`.
