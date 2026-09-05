# Repository test examples

Use these production test modules as fixture and assertion references.
These links identify source tests, not evidence that a command passed in this session.

| Contract | Source and test |
| --- | --- |
| Pure undo/redo state | [`history.rs`](../../../crates/ui/src/history.rs): `test_history`, `test_unique_history` |
| Actual input mutation and restoration | [`input/state.rs`](../../../crates/ui/src/input/state.rs): `test_input_undo_redo_restores_multibyte_replacement` |
| Masked accessibility values | [`input/state.rs`](../../../crates/ui/src/input/state.rs): `test_input_a11y_value_omits_masked_value` |
| Disabled control event propagation | [`button/button.rs`](../../../crates/ui/src/button/button.rs): `test_disabled_and_loading_buttons_stop_parent_clicks` |
| Hover after layout changes | [`button/button.rs`](../../../crates/ui/src/button/button.rs): `test_button_hover_reconciles_after_layout_change` |
| Async prompt completion and cancellation | [`app/test_context.rs`](../../../../engine/crates/gpui/src/app/test_context.rs): `test_simulate_path_prompt_response`, `test_simulate_path_prompt_cancellation` |

## Assertion quality

Exercise the production operation rather than assign the expected state directly.
Assert an exact result where the contract defines one.
For bounded state, test progress, the limit, and the rejected operation.
A range assertion alone can accept an implementation that does nothing.
For undo/redo, assert the changed value and each restored value.
For negative behavior, establish the event path or successful control case in the fixture.

Use the existing benchmark infrastructure for performance claims.
Do not add arbitrary `Instant` thresholds to correctness tests.
Machine load and build mode are not part of a component correctness contract.

## Test execution

Select commands and required integration checks from [TESTING.md](../../../TESTING.md).
Select the affected package and test filter before a broad workspace run.
Use existing feature and platform configuration rather than copy a generic CI workflow.
`--test-threads=1` controls Rust test concurrency, not GPUI property-test iterations.

Use [API contracts](reference.md) for subscription lifetime, async updates, and scheduler behavior.
