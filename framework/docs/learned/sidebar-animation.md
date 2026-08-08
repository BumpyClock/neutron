---
title: "Sidebar Animation Notes"
summary: "Notes on sidebar width animation, submenu open and close animation, and reduced-motion handling."
read_when: "changing sidebar collapse, submenu animation, or reduced-motion behavior"
---
# Sidebar Animation Notes

Date: 2026-02-10

## Goals

- Sidebar container should animate width on collapse/expand.
- Submenu sections should animate both open and close.
- Respect reduced motion.

## Implementation

- Sidebar width:
  - Added `Sidebar::width(Pixels)` as the expanded width source.
  - Animated width with theme tokens:
    - expand: `point_to_point_easing`
    - collapse: `soft_dismiss_easing`
  - Added delayed `visual_collapsed` state so internal collapsed layout (tight paddings/icon-only presentation) applies after close animation instead of snapping at frame 0.

- Submenu sections:
  - Added delayed visibility state to keep submenu mounted while closing.
  - Open animation: `fast_invoke_easing` using fast duration.
  - Close animation: `point_to_point_easing`.
  - Height + opacity animate together with shaped progress (`powf(3.0)`) to keep reflow readable.

## Caveats

- Caret icon still uses immediate 0deg/90deg toggle (no smooth rotate), because `Button` does not expose a simple transform animation hook at the wrapper level.
