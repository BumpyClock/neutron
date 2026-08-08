---
title: "Accordion Animation Notes"
summary: "Notes on accordion enter and exit animation patterns using keyed presence and theme motion tokens."
read_when: "changing accordion animation, keyed presence, or size reveal behavior"
---
# Accordion Animation Notes

Date: 2026-02-10

## Problem

- Content reveal animated.
- Container/sibling reflow not perceived as animated.
- Collapse path removed content immediately (`when(self.open, ...)`), so no exit/layout transition.

## Fix Pattern

- Use `animation::keyed_presence` to manage Entering/Entered/Exiting/Exited.
- Keep content mounted during close via `PresenceTransition::should_render`.
- Gate `with_animation` on `presence.transition_active()`.
- Progress: `presence.progress(delta)` for open/close.

## Token Alignment

- Source: `theme.motion` defaults (Fluent-aligned).
-- Applied curve: `fast_invoke_easing` (open), `point_to_point_easing` (close).
- Duration used: `fast_duration_ms`.

## Notes

- Avoid `bounce(...)` for reveal/size/opacity; it reverses at the end and causes a collapse flash.
- Keep height shaping (`powf(3.0)`) so sibling reflow does not finish too early with large max-height caps.

## Why It Works

- Height shrinks/expands over time, so parent box and following accordion items reflow continuously.
- Exit animation now visible before unmount.
