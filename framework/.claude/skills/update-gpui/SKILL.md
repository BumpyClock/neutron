---
name: update-gpui
description: Update all immutable GPUI Git pins and compatibility metadata. Use when asked to update, bump, or upgrade GPUI.
user_invocable: true
---

## Instructions

Update GPUI only from a reviewed, supplied full 40-character commit hash. Do
not select a moving ref.

### Steps

1. Confirm target hash is exactly 40 lowercase hexadecimal characters and
   record current hash and package versions from root `Cargo.toml`.

2. Update every `https://github.com/BumpyClock/gpui` dependency declaration in
   committed manifests. Keep each declaration's package alias and features, and
   set both its `rev` and exact `version = "=X.Y.Z"`. This currently includes
   `gpui` (`package = "bumpyclock-gpui"`), `gpui_platform`, `gpui-macros`
   (`package = "gpui_macros"`), and `sum-tree` (`package = "sum_tree"`).
   Search all `Cargo.toml` files; do not leave a GPUI-family package on a
   previous revision.

3. Update `[gpui]` and every `[[gpui.packages]]` entry in
   `compatibility.toml` with same revision and exact package versions.
   Regenerate and verify derived document:

   ```bash
   cargo xtask compatibility generate
   cargo xtask compatibility check
   ```

4. For coordinated source testing only, a sibling GPUI checkout may be wired
   through an uncommitted `.cargo/config.toml` patch. Patch the actual packages
   `bumpyclock-gpui`, `gpui_platform`, `gpui_macros`, and `sum_tree`; add
   further resolved GPUI packages when needed. The committed manifest's
   package identity must already match the checkout. Remove the override
   before release checks. Never place it in a committed manifest or config.

5. Build, test, and check release plan:

   ```bash
   cargo fmt --all -- --check
   cargo metadata --locked
   cargo check --workspace --all-targets --locked
   cargo test --workspace --all-targets --locked
   ./script/clippy --locked
   cargo xtask publish-plan
   cargo xtask release-check
   ```

6. Report old/new revision, package versions, compatibility result, and
   validation results. Version selection, tag creation, and publication need
   separate owner authorization.

### Notes

- Committed GPUI dependencies use only canonical Git URL, full immutable
  revision, and exact version.
- Repository has no GPUI submodule. Local sibling source is a temporary,
  uncommitted development override only.
