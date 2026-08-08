# Engine domain

`engine/` owns the generic GPUI mechanisms: application state, rendering, text,
input, event loops, windows, platform adapters, accessibility, and renderer
evidence. Engine crates must not depend on framework crates.

GPUI is a selective semantic fork of [Zed](https://github.com/zed-industries/zed).
Use `engine/fork.toml` as the machine-readable provenance record and
`engine/UPSTREAM.md` for sync rules. For an upstream review, use a read-only
checkout in `/tmp/zed`, compare the affected GPUI code, preserve fork contracts,
and run the root engine and framework checks.

Use root-relative paths and exact package versions for local engine consumers.
Keep package identities, license notices, and publication blockers unchanged
unless the root migration task names them.
