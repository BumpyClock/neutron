# Framework domain

`framework/` owns AppShell, lifecycle policy, commands and menus, settings,
themes, storage, managed windows, reusable components, conformance, and release
tooling. Framework crates may depend on public engine crates. Do not move
framework policy into `engine/`.

## Component contracts

- Call `gpui_component::init(cx)` once during component initialization.
- Use `Root` as the first view in each normal framework window.
- Preserve accessibility roles, names, values, states, actions, and focus behavior.
- Preserve reduced-motion behavior and platform-specific capability errors.
- Prefer stateless `RenderOnce` components when state is not required.
- Use the default cursor for desktop buttons. Use a pointer cursor only for link-like controls.

## Validation

Run `../script/check` for root format, metadata, compile, lint, fork, and
compatibility checks. Run `../script/test` for unit, doctest, and tooling tests.
Run `../script/release-check` for package and release gates. Build the docs site
from `docs/` with `bun install --frozen-lockfile` and `bun run build`.

Tests are required for behavior changes. Test complex state transitions,
geometry, lifecycle, accessibility, and builder contracts. Use ordinary Rust
tests for pure logic and GPUI tests for window or rendering behavior.

## Domain records

Keep framework compatibility and publication facts in `compatibility.toml`,
`docs/COMPATIBILITY.md`, `TESTING.md`, and `RELEASING.md`. Keep durable lessons
in `docs/learned/`. Use root-workspace path dependencies for engine crates and
do not add a separate engine checkout or Git revision.
