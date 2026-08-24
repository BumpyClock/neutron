---
title: "Theme Motion Search Notes"
summary: "Notes on ThemeMotion tokens, default easing values, and components that consume motion helpers."
read_when: "changing theme motion tokens, animation helpers, or component motion docs"
---
# Theme Motion Search Notes

Date: 2026-08-23

## Tokens

- `ThemeMotion` tokens: four durations, two spring values, and four easing
  strings. `crates/ui/src/theme/mod.rs`
- Defaults in `crates/ui/src/theme/fluent_tokens.rs` and `crates/ui/src/theme/default-theme.json`.
- Theme overrides in `crates/ui/src/theme/schema.rs` (`ThemeMotionConfig`).
- Spring tokens in `ThemeMotion`:
  - `spring_damping_ratio` (default `0.78`)
  - `spring_frequency` (default `2.0` cycles per enter-duration window)

## Default values

- `fade_duration_ms`: `83`
- `exit_duration_ms`: `167`
- `enter_duration_ms`: `187`
- `emphasis_duration_ms`: `667`
- `spring_damping_ratio`: `0.78`
- `spring_frequency`: `2.0`
- `decelerate_easing`: `cubic-bezier(0, 0, 0, 1)`
- `standard_easing`: `cubic-bezier(0.55, 0.55, 0, 1)`
- `emphasis_easing`: `cubic-bezier(0.13, 1.62, 0, 0.92)`
- `fade_easing`: `linear`

## Usage

- Helper functions: `crates/ui/src/animation.rs` (`enter_animation`,
  `exit_animation`, `standard_animation`, `fade_animation`,
  `spring_animation`, `theme_spring_config`, `keyed_presence`).
- `Switch` uses GPUI's stateful `SpringAnimation` for thumb geometry.
- Flyouts, dialogs, accordions, sidebars, and command palette use the
  duration-based `spring_animation` for presence transforms.

## Spring forms

- `spring_animation` samples the theme spring into a fixed-duration GPUI
  `Animation`. It restarts if its target changes.
- `SpringAnimation` retains position and velocity across target changes. Use it
  for continuously mounted geometry.
- `theme_spring_config` converts theme values with `ω = 2πf/d`, `k = ω²`,
  `c = 2ζω`, and `m = 1`. Non-finite values use default tokens; frequency and
  damping ratio are bounded before conversion.
- `reduced_motion(cx)` returns true when either engine or framework reduced
  motion is enabled.

Theme files may override these fields under `motion`. Unknown legacy token names
do not change `ThemeMotion`; use the fields listed above.
