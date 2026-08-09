# Releasing the framework domain

Neutron is one Git monorepo and one Cargo workspace. Framework crates consume
engine crates from `engine/` through exact local path and package-version
dependencies. Engine and framework versions remain independent.

The root `.github/workflows/release-validation.yml` runs for `engine-v*` and
`framework-v*` tags. It runs release checks and uploads a report. It does not
publish crates, create a GitHub release, or move a tag automatically.

## Release authorization

An owner must authorize the framework version, registry package identities,
publication, tag creation, and remote push. Do not publish while any owner or
license blocker remains unresolved.

The bump script requires cargo-edit's `cargo set-version` command. We validated
cargo-edit 0.13.13. Verify the command before a release:

```bash
cargo set-version --help
```

## Prepare a framework release

1. Select a new framework version without changing a published version or
   moving an existing tag. Keep engine package versions independent. Obtain
   owner authorization for the version and registry package identities. Create
   the version commit from the repository root:

   ```bash
   ./framework/script/bump-version.sh X.Y.Z
   ```

   The script updates framework versions and compatibility files. It creates
   one local commit. It does not create a tag, push, or publish.

2. Confirm local exact engine dependencies and fork provenance:

   ```bash
   if rg -n 'git\s*=\s*"https://github\.com/BumpyClock/gpui|github\.com/BumpyClock/gpui' --glob 'Cargo.toml'; then
     echo "error: old BumpyClock/gpui Git pin remains"
     exit 1
   fi
   cargo run --locked -p engine-xtask -- fork validate
   cargo run --locked -p framework-xtask -- compatibility generate
   cargo run --locked -p framework-xtask -- compatibility check
   ```

   Framework engine dependencies must resolve to `engine/crates/...` with an
   exact `version = "=X.Y.Z"`. `engine/fork.toml` must record the audited Zed
   base and maintained fork patch clusters. `framework/compatibility.toml`
   must record `engine_path = "engine"`, exact engine package versions, and
   crate paths. Do not record the current Neutron HEAD in tracked metadata.

3. Run the package and release gates from the repository root:

   ```bash
   cargo run --locked -p engine-xtask -- publish-plan
   cargo run --locked -p framework-xtask -- publish-plan
   cargo run --locked -p framework-xtask -- release-check
   ./script/check
   ./script/test
   ```

4. Push the version commit after owner authorization. Run the complete Stage 1
   matrix on that final source commit when release evidence is required.
   Keep all seven job artifacts. Acceptance must verify
   `BumpyClock/neutron`, the workflow `github.sha`, repository `HEAD`,
   `HEAD^{tree}`, and each `source-manifest.json` and
   `source-verification.json`. Source manifests must retain repository, commit,
   tree, workflow-run, and required file-digest evidence. Old Stage 1 artifacts
   do not prove the current monorepo commit.

5. Make no source commit after accepted Stage 1 evidence. If the source changes,
   repeat Stage 1 before release. After acceptance, create the immutable
   `framework-vX.Y.Z` tag. Push it only with separate owner authorization. Wait
   for tag-triggered Stage 1 and release validation before publication.

## Publish order

Use the package plan as the source of truth. Publish approved engine packages
first, then approved framework support and facade packages. Require registry
checks only after the owner confirms package ownership and publication access:

```bash
cargo run --locked -p framework-xtask -- publish-plan --require-registry
cargo run --locked -p framework-xtask -- release-check --require-registry
```

An authorized owner performs each publication manually after the gates pass.
The root tag workflow validates readiness only. It does not publish crates or
create a GitHub release automatically.

## Local coordinated development

Use the root workspace. Keep framework-to-engine dependencies on exact local
paths and exact engine versions. Do not add sibling checkouts, Git revisions,
branches, pull-request refs, mutable engine tags, or committed Cargo patches.

## Finalize

Create the immutable `framework-vX.Y.Z` tag only after all gates and exact-source
Stage 1 acceptance pass. The tag must point to the accepted final source commit.
Use these owner-approved commands:

```bash
git tag framework-vX.Y.Z
git push origin main framework-vX.Y.Z
```

Release notes must record the framework version, exact engine package versions,
`engine/fork.toml` upstream base, Stage 1 evidence, support limits, and open
publication blockers. Never replace a published version or move a release tag.
