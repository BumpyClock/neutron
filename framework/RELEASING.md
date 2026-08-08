# Releasing GPUI Component

This repository currently supports Git consumption only. Framework crates must
not be published until every exact GPUI fork package in `compatibility.toml` is
published and source-equivalent to the pinned GPUI commit.

See [testing and CI](TESTING.md) for the maintained test inventory and native
runtime limits that release validation does not erase.

## Prepare a release

1. Choose the next version without changing an existing tag or published crate.
   A GPUI revision or public compatibility change requires a pre-1.0 minor bump.
2. Merge GPUI first. Record its final 40-character commit and exact engine
   package versions in this repository's manifests and `compatibility.toml`.
3. Regenerate and verify compatibility documentation:

   ```bash
   cargo xtask compatibility generate
   cargo xtask compatibility check
   ```

4. Review publication order and source/package artifacts:

   ```bash
   cargo xtask publish-plan
   cargo xtask release-check
   ```

5. After engine packages are available on crates.io, require registry evidence:

   ```bash
   cargo xtask publish-plan --require-registry
   cargo xtask release-check --require-registry
   ```

`--require-registry` is expected to fail before the GPUI engine publication
prerequisites exist. Do not weaken it or replace it with a Git dependency.

## Publish order

Use `cargo xtask publish-plan --require-registry` as the source of truth. It
derives the dependency order from manifests. Publish foundational GPUI packages,
then GPUI platform/facade packages, followed by framework support packages and
finally public framework facade packages.

An owner with verified crates.io access performs the authorized publication only
after the release gate passes. This repository's tag workflow validates releases;
it does not publish crates automatically.

## Local coordinated development

Use an uncommitted sibling-checkout patch as described in
[`docs/learned/gpui-submodule.md`](docs/learned/gpui-submodule.md). Never commit
a local path, branch, pull-request revision, or mutable GPUI tag as a release
dependency.

## Finalize

Create an immutable framework tag only after all gates pass. Release notes must
record the framework version, full GPUI commit, exact engine registry package
versions, support evidence, and any remaining native-runtime limitations.

Never replace a published crate version or move a release tag.
