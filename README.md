# Neutron

Neutron is a Rust-native desktop application SDK. It combines a GPU-accelerated UI engine with a reusable desktop component framework in one Cargo workspace.

## Repository map

- `engine/` owns generic UI mechanisms: application state, rendering, text, input, event loops, windows, platform adapters, accessibility, and renderer evidence.
- `framework/` owns application policy and reusable components: AppShell, lifecycle, commands, menus, settings, themes, storage, managed windows, conformance, and release tooling.
- `MIGRATION.md` records immutable source and destination snapshot facts.
- `AGENTS.md` defines repository policy. `CLAUDE.md` points to that policy.

Engine crates must not depend on framework crates. Framework crates may depend on public engine crates through root-workspace path dependencies with exact versions. The root workspace is the product workspace. The app-manifest downstream fixture and the WASM-only `hello_web` example are isolated workspaces by design.

## Workspace commands

Run commands from the repository root:

```sh
cargo fmt --all -- --check
cargo metadata --locked --format-version 1
cargo check --locked --workspace --all-targets
cargo test --locked --workspace --all-targets --features test-support
cargo clippy --locked --workspace --all-targets --all-features -- --deny warnings
```

Build documentation from `framework/docs` with `bun install --frozen-lockfile` and `bun run build`.

Use root scripts when present:

```sh
./script/bootstrap
./script/check
./script/test
./script/release-check
./script/stage1
```

Tests and evidence must match the exact source commit. Compilation, headless tests, native window creation, presentation API submission, backend acceptance, and hardware-GPU proof are separate evidence levels.

## Support maturity

The current source baselines provide development and test foundations. Platform maturity remains conservative until the final monorepo Stage 1 matrix passes. Supported, compile-only, experimental, and unsupported behavior must stay distinct in docs and metadata.

Custom-rendered controls are not native operating-system widgets. Describe Neutron as native application/runtime integration with custom-rendered controls.

## Upstream provenance

- `engine/` is a selective semantic fork of [Zed](https://github.com/zed-industries/zed). See [`engine/fork.toml`](engine/fork.toml) and [`engine/UPSTREAM.md`](engine/UPSTREAM.md).
- `framework/` preserves its [Longbridge source](https://github.com/longbridge/gpui-component) relationship and local adaptations. See framework upstream and compatibility records.
- Historical [BumpyClock/gpui](https://github.com/BumpyClock/gpui) and [BumpyClock/gpui-component](https://github.com/BumpyClock/gpui-component) links remain provenance only. New Neutron links use [BumpyClock/neutron](https://github.com/BumpyClock/neutron) after destination publication.

The source repositories remain unchanged. Neutron imports exact committed snapshots; it does not claim imported source history or tags.

## Status

This repository is in structural consolidation. Do not publish packages, create releases or tags, or begin Stage 2 feature work until the migration gates in `AGENTS.md` pass.
