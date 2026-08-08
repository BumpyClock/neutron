---
title: "Spring Preset API"
summary: "Notes on tokenized spring preset helpers and when spring motion is safe for transform-only animation."
read_when: "changing animation helpers, spring presets, or theme motion token usage"
---
# Spring Preset API

Date: 2026-02-11

## Overview

The spring preset API provides tokenized, theme-aware animation helpers for
consistent motion across all UI components. Every component should use the
shared helpers in `crates/ui/src/animation.rs` instead of constructing
`Animation::new(Duration::...)` with inline easing.

## Spring Presets

Two spring presets are available via `SpringPreset`:

| Preset   | Damping | Frequency | Duration | Use case                            |
|----------|---------|-----------|----------|-------------------------------------|
| **Mild** | 0.78    | 2.0 Hz    | 187 ms   | Subtle transform overshoot (menus)  |
| **Medium** | 0.70  | 1.6 Hz    | 240 ms   | Noticeable bounce (dialogs, popovers) |

Spring parameters are stored in `ThemeMotion` and can be overridden per-theme.

### Usage

```rust
use crate::animation::{spring_preset_animation, SpringPreset};

// Returns Option<Animation> — None when reduced_motion is true
let anim = spring_preset_animation(&motion, reduced_motion, SpringPreset::Medium);
```

## Transform-Only Spring Rule

**Springs overshoot — they produce values > 1.0.**  
This is correct for transform properties (translate, rotate, scale) where
overshoot creates a natural bounce feel. It is **incorrect** for:

- **Opacity** — values > 1.0 are clamped or undefined
- **Size / max-height** — overshoot causes layout jumps
- **Layout properties** — content may flash or overflow

### Rule

| Property type        | Allowed easing                                    |
|----------------------|---------------------------------------------------|
| Transform (translate, scale, rotate) | Spring presets, `strong_invoke_animation` |
| Opacity              | `fade_animation` (linear), `point_to_point_animation` |
| Size / layout        | `fast_invoke_animation`, `point_to_point_animation`, `soft_dismiss_animation` |
| Reveal (open/close)  | Monotonic curves only — never spring or bounce    |

### Never use `bounce()` for reveal animations

The `bounce()` easing is forward-then-reverse: at `delta=1.0` it returns ~0,
which collapses the element on the final frame. Only use `bounce()` for
repeating cosmetic effects (e.g. skeleton shimmer).

## Monotonic Animation Helpers

These helpers return `Option<Animation>` (None when `reduced_motion` is true):

| Helper                        | Duration | Easing                          | Use case                  |
|-------------------------------|----------|---------------------------------|---------------------------|
| `fast_invoke_animation`       | 187 ms   | `cubic-bezier(0, 0, 0, 1)`     | Quick open/invoke         |
| `soft_dismiss_animation`      | 167 ms   | `cubic-bezier(1, 0, 1, 1)`     | Close/dismiss             |
| `point_to_point_animation`    | 187 ms   | `cubic-bezier(0.55, 0.55, 0, 1)` | Position + opacity move |
| `fade_animation`              | 83 ms    | linear                          | Opacity-only fade         |
| `strong_invoke_animation`     | 667 ms   | overshoot cubic-bezier          | Transform-only emphasis   |

## Component Animation Map

| Component        | Open / Enter                          | Close / Exit                    |
|------------------|---------------------------------------|---------------------------------|
| Accordion        | `spring_preset` (transform) + `point_to_point` (layout) | same        |
| Dialog           | `spring_preset(Medium)` (transform) + `point_to_point` (layout) + `fade` | `soft_dismiss` + `fade` |
| Popover          | `spring_preset(Medium)` (transform) + `point_to_point` (opacity) | `point_to_point` |
| Popup Menu       | `spring_preset(Medium)` (transform) + `point_to_point` (opacity) | `point_to_point` |
| Sidebar Menu     | `spring_preset(Mild)` (transform) + `point_to_point` (layout) | same    |
| Command Palette  | `spring_invoke` (Mild)                | —                               |
| Sheet            | `fast_invoke`                         | —                               |
| Notification     | `fast_invoke`                         | `soft_dismiss`                  |
| Collapsible      | `fast_invoke`                         | `fast_invoke`                   |
| Select           | `fast_invoke`                         | —                               |
| Context Menu     | `fast_invoke`                         | —                               |
| Tooltip          | `fade`                                | —                               |
| Tab              | `fade` (on select)                    | —                               |
| Badge            | `strong_invoke` (transform bounce)    | —                               |

## Presence State Machine

For open/close animations, use `keyed_presence()` from `animation.rs`:

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
    // render with presence.progress(delta) for opacity/layout
}
```

The state machine tracks `PresencePhase`:
- `Entering` → apply open animation
- `Entered` → static open state
- `Exiting` → apply close animation (keep mounted)
- `Exited` → unmount
