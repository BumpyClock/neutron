---
name: gpui-test
description: Writing tests for GPUI applications. Use when testing components, async operations, or UI behavior.
---

# GPUI tests

Use this skill for GPUI entity, async, input, and component test changes.
Select tests by the affected contract, not a component quota.

- Use ordinary `#[test]` when the contract does not need a GPUI context.
- Use `#[gpui::test]` for entities, subscriptions, executors, and app state.
- Use `VisualTestContext` for window, layout, focus, and input behavior.
- Initialize framework components only when the fixture needs their app services.
- Assert observable results that reject a no-op or incorrect transition.

For API-specific fragments and timer constraints, read [reference.md](reference.md).
For complete production test fixtures, read [examples.md](examples.md).
For commands and integration requirements, read [TESTING.md](../../../TESTING.md).

Complete the change when affected checks pass, or report the exact blocker and absent evidence.
Keep headless and native platform evidence separate.
