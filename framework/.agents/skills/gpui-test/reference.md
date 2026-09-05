# GPUI test API reference

Use the resolved engine source, not an upstream snippet, to select test APIs.
The fragments below illustrate API contracts. They are not standalone tests.
Use [repository tests](examples.md) for complete fixtures and assertions.

## Subscription lifetime

[`Context::subscribe_self`](../../../../engine/crates/gpui/src/app/context.rs)
returns a `Subscription`. Retain that handle for the observation period.
A discarded handle cancels the subscription.

For an entity with a `subscriptions: Vec<gpui::Subscription>` field:

```rust
let subscription = cx.subscribe_self(|this, event: &ValueChanged, cx| {
    this.received_value = event.new_value;
    cx.notify();
});
component.subscriptions.push(subscription);
```

Here, `component` is the entity under construction inside `cx.new`.
`ValueChanged` and the entity fields are illustrative application types.
[`Subscription::detach`](../../../../engine/crates/gpui/src/subscription.rs)
keeps the callback active until the subscribed entities are released.
Detach only when that lifetime matches the contract.

## Async entity updates

[`WeakEntity::update`](../../../../engine/crates/gpui/src/app/entity_map.rs)
returns a synchronous `Result<R>`, not a future.
Await the task from [`Context::spawn`](../../../../engine/crates/gpui/src/app/context.rs), not the update.

For a component with a `value` field:

```rust
fn update_value(&self, cx: &mut gpui::Context<Self>) -> gpui::Task<anyhow::Result<()>> {
    cx.spawn(async move |this, cx| {
        this.update(cx, |component, cx| {
            component.value = 42;
            cx.notify();
        })?;
        Ok(())
    })
}
```

Propagate the error when entity release is a failure.
If entity release is expected cancellation, handle that case explicitly.
Retain or await tasks whose completion is part of the test contract.

## Scheduler and timers

Trace the concrete context before a timer assertion:

- [`TestAppContext::run_until_parked`](../../../../engine/crates/gpui/src/app/test_context.rs)
  delegates to the background executor.
- [`BackgroundExecutor::run_until_parked`](../../../../engine/crates/gpui/src/executor.rs)
  uses the scheduler, which can advance to the next timer when no runnable tasks remain.
- `BackgroundExecutor::advance_clock` makes timers ready but does not execute tasks.

Do not assume that methods with the same name on other contexts share these semantics.
For deadline boundaries, establish the pending task and control clock progress through the resolved scheduler API.
Assert both the state before the deadline and the result after completion.
Do not use a wall-clock sleep or an arbitrary elapsed-time threshold as a scheduler oracle.

## Context selection

Use ordinary `#[test]` for pure state, geometry, validation, and builder contracts.
Use `#[gpui::test]` when the contract needs entities, subscriptions, executors, or app state.
Use `VisualTestContext` when the contract needs a window, layout, focus, or input dispatch.
Headless GPUI tests do not establish native platform conformance.
