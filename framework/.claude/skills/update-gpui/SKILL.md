---
name: update-gpui
description: Update local engine dependencies, fork provenance, and compatibility metadata in the Neutron monorepo.
user_invocable: true
---

## Instructions

Neutron has one Git monorepo and one Cargo workspace. Use exact local engine
paths and package versions. Never add engine Git pins or sibling checkouts.

### Steps

1. Start at the repository root. Record engine package names, exact versions,
   `engine/fork.toml`, and `framework/compatibility.toml`. Use an owner-approved
   Zed commit for upstream syncs. Do not select a moving ref.

2. Update framework dependencies in the root workspace manifest. Keep aliases,
   Rust import names, and features. Use a root-relative path and exact version:

   ```toml
   gpui = { package = "bumpyclock-gpui", path = "engine/crates/gpui", version = "=0.1.0" }
   gpui_platform = { path = "engine/crates/gpui_platform", version = "=0.1.0" }
   gpui-macros = { package = "gpui_macros", path = "engine/crates/gpui_macros", version = "=0.1.0" }
   sum-tree = { package = "sum_tree", path = "engine/crates/sum_tree", version = "=0.1.0" }
   ```

   Use checkout versions. Reject old engine Git dependencies and floating
   revisions in tracked manifests.

3. Update provenance metadata. Keep the audited Zed base and patch clusters in
   `engine/fork.toml`. Keep `engine_path = "engine"` and exact engine package
   versions in `framework/compatibility.toml`. Do not store current Neutron HEAD
   in tracked compatibility metadata.

4. Run root checks after dependency or provenance changes:

   ```bash
   cargo fmt --all -- --check
   cargo metadata --locked --format-version 1
   cargo check --locked --workspace --all-targets
   cargo test --locked --workspace --all-targets --features test-support
   cargo clippy --locked --workspace --all-targets --all-features -- --deny warnings
   cargo run --locked -p engine-xtask -- fork validate
   cargo run --locked -p engine-xtask -- publish-plan
   cargo run --locked -p framework-xtask -- compatibility generate
   cargo run --locked -p framework-xtask -- compatibility check
   cargo run --locked -p framework-xtask -- publish-plan
   cargo run --locked -p framework-xtask -- release-check
   ```

5. Run Stage 1 on one exact final commit for runtime or release evidence.
   Retain every `source-manifest.json` and `source-verification.json` artifact.
   Acceptance must match `BumpyClock/neutron`, `github.sha`, `HEAD`, and
   `HEAD^{tree}`. Verify repository, workflow-run, and required file digests.
   Repeat Stage 1 after any later source commit and report versions, fork base,
   compatibility results, and evidence paths. Require owner authorization for
   versions, identities, publication, tags, and pushes because release checks do
   not publish or create releases automatically.

### Notes

- Engine upstream provenance belongs in `engine/fork.toml`.
- Stage 1 evidence identifies the exact monorepo commit and tree.
- Do not commit temporary `.cargo/config.toml` patches.
