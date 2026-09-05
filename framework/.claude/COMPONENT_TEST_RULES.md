# Component test contracts

Use [TESTING.md](../TESTING.md) for test selection and commands.
Use the canonical [GPUI test skill](../.agents/skills/gpui-test/SKILL.md) for context selection and API constraints.
Use its [repository examples](../.agents/skills/gpui-test/examples.md) for complete test fixtures.

Test behavior that can regress: state transitions, geometry, lifecycle, accessibility, and input propagation.
Test builder options when their interaction enforces a meaningful contract.
Do not require a builder test for every component or a fixed number of tests.
Do not mirror each setter with a field assertion.

Use ordinary Rust tests for pure builders and helpers.
Initialize GPUI only when the tested contract needs app state, entities, subscriptions, a window, or the executor.
An existing test with an unused GPUI context does not establish that requirement for new tests.

Assert exact outcomes and relevant boundary transitions.
Ensure a no-op implementation cannot satisfy a test that requires progress.
Retain subscriptions through the observation period.
Use the resolved scheduler contract for timer tests, not wall-clock thresholds.
