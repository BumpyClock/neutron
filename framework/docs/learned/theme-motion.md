---
title: "Theme Motion Search Notes"
summary: "Notes on ThemeMotion tokens, default easing values, and components that consume motion helpers."
read_when: "changing theme motion tokens, animation helpers, or component motion docs"
---
# Theme Motion Search Notes

Date: 2026-02-10

## Tokens

- `ThemeMotion` tokens: durations + easing strings. `crates/ui/src/theme/mod.rs`
- Defaults in `crates/ui/src/theme/fluent_tokens.rs` and `crates/ui/src/theme/default-theme.json`.
- Theme overrides in `crates/ui/src/theme/schema.rs` (`ThemeMotionConfig`).
- Spring presets in `ThemeMotion`:
  - `spring_mild_duration_ms`, `spring_mild_damping_ratio`, `spring_mild_frequency`
  - `spring_medium_duration_ms`, `spring_medium_damping_ratio`, `spring_medium_frequency`

## Fluent token values (animation.md)

- Fast Invoke: durations 187/333/500ms; easing `cubic-bezier(0, 0, 0, 1)`
- Strong Invoke: duration 667ms; easing `cubic-bezier(0.13, 1.62, 0, 0.92)`
- Fast Dismiss: durations 187/333/500ms; easing `cubic-bezier(0, 0, 0, 1)`
- Soft Dismiss: duration 167ms; easing `cubic-bezier(1, 0, 1, 1)`
- Point to Point: durations 187/333/500ms; easing `cubic-bezier(0.55, 0.55, 0, 1)`
- Fade In/Out: duration 83ms; easing `linear`
- Springs: none in tokens

## Usage

- Helper fns: `crates/ui/src/animation.rs` (`fast_invoke_animation`, `soft_dismiss_animation`, `point_to_point_animation`, `fade_animation`, `strong_invoke_animation`).
- Components using theme motion: accordion, badge, checkbox, collapsible, dialog, menu/context_menu, notification, popover, progress, select, sheet, sidebar, switch, tab, time/date_picker, tooltip, command_palette (durations only). See `crates/ui/src/*`.

## Spring-like patterns

- `gentle_spring` easing in command palette. `crates/ui/src/command_palette/view.rs`
- `strong_invoke_easing` is overshoot (cubic-bezier y>1). `crates/ui/src/theme/fluent_tokens.rs`
- `bounce(ease_in_out)` used for skeleton shimmer. `crates/ui/src/skeleton.rs`
- Preset API: `spring_preset_animation(..., SpringPreset::{Mild, Medium})`. `crates/ui/src/animation.rs`

## Doc mismatch

- `docs/docs/components/sidebar.md` says submenu expand uses `bounce(ease_in_out)`, but code uses `fast_invoke_animation`.
