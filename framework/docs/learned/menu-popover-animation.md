---
title: "Menu and Popover Animation Notes"
summary: "Notes on popover, popup menu, submenu, and sidebar flyout animation constraints."
read_when: "changing popover, menu, submenu, dropdown cleanup, or collapsed sidebar flyout animation"
---
# Menu and Popover Animation Notes

Date: 2026-02-10

## Goals

- Add consistent motion to popover and popup menus.
- Animate submenu open/close instead of snapping.
- Keep reduced-motion behavior deterministic.
- Preserve interactive child controls in collapsed sidebar flyouts.

## Implementation

- Popover:
  - Enter uses `spring_preset_animation(..., SpringPreset::Medium)` for transform only.
  - Exit uses `point_to_point` curve.
  - Visuals: monotonic opacity + anchor-aware `translate_y` spring offset.

- PopupMenu:
  - Submenu open uses `SpringPreset::Medium` transform; close remains monotonic.
  - Submenu visuals: monotonic opacity + side-aware `translate_x` spring offset.

- Dropdown menu lifecycle:
  - Menu cache cleanup delay now matches popover fade dismiss timing.
  - Reduced-motion path clears immediately.

- Sidebar collapsed flyout:
  - Child items without interactive suffix render in `PopupMenu`.
  - Child items with interactive suffix render as live sidebar rows inside `Popover` content.

## Caveats

- Keep overshoot/spring easing on transform properties only.
- Keep opacity/size/visibility monotonic via clamped progress (`presence.progress`).
- For transform-heavy effects, prefer bounded distances (`~6px`) to avoid blur jitter.
