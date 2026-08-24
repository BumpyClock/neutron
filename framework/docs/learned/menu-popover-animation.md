---
title: "Menu and Popover Animation Notes"
summary: "Notes on popover, popup menu, submenu, and sidebar flyout animation constraints."
read_when: "changing popover, menu, submenu, dropdown cleanup, or collapsed sidebar flyout animation"
---
# Menu and Popover Animation Notes

Date: 2026-08-23

## Goals

- Add consistent motion to popover and popup menus.
- Animate submenu open/close instead of snapping.
- Keep reduced-motion behavior deterministic.
- Preserve interactive child controls in collapsed sidebar flyouts.

## Implementation

- Popover and other flyouts:
  - Enter uses `spring_animation` for transform only.
  - Enter opacity uses `standard_animation`.
  - Exit uses `exit_animation` for opacity and anchor-aware translation.
  - Presence uses `keyed_presence` with enter and exit duration windows.

- PopupMenu:
  - Submenu open uses `spring_animation` for transform; close remains
    duration-based and monotonic.
  - Submenu visuals use monotonic opacity and side-aware `translate_x` offset.

- Dropdown menu lifecycle:
  - Menu cache cleanup delay now matches popover fade dismiss timing.
  - Reduced-motion path clears immediately.

- Sidebar collapsed flyout:
  - Child items without interactive suffix render in `PopupMenu`.
  - Child items with interactive suffix render as live sidebar rows inside `Popover` content.

`spring_animation` is the duration-based form. It samples a spring over the
enter window and restarts if its target changes. GPUI's stateful
`SpringAnimation` is reserved for continuously mounted geometry that can
retarget without losing velocity. `Switch` uses that form for its thumb.

Theme spring conversion uses `ω = 2πf/d`, `k = ω²`, `c = 2ζω`, and `m = 1`.
Non-finite or non-positive duration and non-finite spring tokens fall back to
default theme values. Frequency and damping ratio are bounded before conversion.
Reduced motion combines engine and framework signals, so both presence motion
and stateful geometry must reach their final state immediately.

Backdrop blur, native blur, and flyout material tokens are independent of these
animation paths. Motion changes must not remove or delay surface material.

## Caveats

- Keep spring output on transform properties only.
- Keep opacity, size, and visibility monotonic via clamped progress
  (`presence.progress`).
- For transform-heavy effects, prefer bounded distances (`~6px`) to avoid blur jitter.
