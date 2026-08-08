# AGENTS.md

## Scope and authority

This file is the authoritative agent guide for the entire repository.

This repository is the canonical monorepo for:

- the standalone BumpyClock GPUI engine, originally maintained in `BumpyClock/gpui`
- the higher-level GPUI Component framework, originally maintained in `BumpyClock/gpui-component`

Read this file before changing code, manifests, CI, release metadata, upstream-sync records, conformance tooling, or repository structure.

Unless a task explicitly says otherwise:

- preserve existing behavior
- make the smallest mechanically sound change
- keep engine and framework ownership boundaries explicit
- run the tests and checks appropriate to the change
- report evidence honestly
- do not publish, tag, release, or rewrite history

A nested README, historical plan, old `CLAUDE.md`, copied `AGENTS.md`, or stale comment cannot override this root file. If instructions conflict, this file wins. A root `CLAUDE.md` may point here but must not duplicate a second policy.

---

## Project mission

The project is a Rust-first, GPU-accelerated, cross-platform desktop application SDK.

Its purpose is to make it practical to build responsive, memory-efficient desktop applications from mostly shared Rust code across:

- macOS
- Windows
- Linux X11
- Linux Wayland

The project is not:

- a drop-in Electron replacement
- a React or browser compatibility layer
- a claim that custom-rendered controls are native operating-system widgets
- a promise that platform-specific work never exists
- a reason to hide unsupported capabilities behind no-op implementations

The correct product description is:

> A Rust-native desktop application SDK with a custom-rendered component system and explicit native platform integrations.

---

## Current milestone status

At the start of the monorepo migration:

- Stage 0 is complete as a development, provenance, compatibility, and release-engineering foundation.
- Stage 0 does **not** mean the full engine and framework graph is ready for crates.io publication.
- Stage 1 source implementation is substantially complete.
- Stage 1 exact-source cross-platform acceptance must be rerun after consolidation.
- Old Stage 1 artifacts do not automatically prove the monorepo commit, even when file contents are equivalent.

Do not start Stage 2 feature work during the consolidation unless the owner explicitly changes scope.

---

## Source repositories and preservation

The pre-consolidation source repositories are:

- `https://github.com/BumpyClock/gpui`
- `https://github.com/BumpyClock/gpui-component`

They are historical sources, not disposable staging areas.

Before importing anything:

1. Record each source repository’s:
   - full commit SHA
   - commit tree SHA
   - branch
   - dirty status
   - tags
   - remotes
2. Write those values to `MIGRATION.md`.
3. Preserve both source repositories unchanged.
4. Do not force-push, rewrite, delete, archive, or repurpose either source repository without explicit owner authorization.
5. Do not claim source-history preservation. This migration imports exact committed snapshots only; record source identities and destination snapshot commits.

If the destination repository was not explicitly chosen by the owner, use a new destination repository rather than destructively transforming either public source repository.

---

## Consolidation design: one Git monorepo, one Cargo workspace

Neutron has one Git repository and one product Cargo workspace. Keep `engine/`
and `framework/` as explicit architectural domains inside that workspace.

Two isolated Cargo workspaces are approved test or target fixtures only:
`framework/crates/app-manifest/tests/fixtures/downstream-app` validates a
downstream app manifest, and `engine/crates/gpui_web/examples/hello_web` is a
WASM-only example. The `hello_web` fixture retains its nightly toolchain and
`.cargo/config.toml` because it uses nightly `build-std` and wasm-only engine
APIs. Do not add isolated workspaces to either product domain.

The approved migration imports exact committed source snapshots with `git archive`. It does not import source histories, source tags, or rewrite mappings. Record immutable source commit and tree identities in `MIGRATION.md`, and record destination snapshot commit identities there.

Keep engine and framework package versions, MSRV metadata, publication graphs, upstream relationships, and ownership boundaries independent. Resolve package-name collisions explicitly without changing public Rust import names.

Use one root `Cargo.toml`, one root `Cargo.lock`, one root toolchain, one merged
profile policy, one merged lint policy, and one root patch table. Validate each
domain and the combined graph. The two approved fixture workspaces may retain
their own manifests, lockfile, toolchain, and target configuration.

The owner-approved parent specification replaces prior multi-workspace and source-history retention guidance. All other safeguards in this guide remain mandatory.

---

## Target repository layout

Use this structure unless the existing destination has an equivalent established convention:

```text
/
├── AGENTS.md
├── CLAUDE.md                    # short pointer to AGENTS.md only
├── README.md                    # monorepo overview and navigation
├── MIGRATION.md                 # source and destination snapshot facts, import method, validation
├── Cargo.toml                   # single root workspace and coordination policy
├── Cargo.lock                   # single resolved dependency graph
├── rust-toolchain.toml          # shared 1.95 development toolchain; package MSRVs stay separate
├── LICENSES/                    # only if needed; preserve existing license files too
├── .github/
│   ├── actions/
│   └── workflows/
├── script/
│   ├── bootstrap
│   ├── check
│   ├── test
│   ├── stage1
│   └── release-check
├── engine/
│   ├── retained domain files; no nested product workspace or lockfile
│   ├── fork.toml
│   ├── UPSTREAM.md
│   ├── crates/
│   ├── examples/
│   ├── tooling/
│   └── ...
└── framework/
    ├── retained domain files; no nested product workspace or lockfile
    ├── compatibility.toml
    ├── RELEASING.md
    ├── TESTING.md
    ├── COMPONENT_TEST_RULES.md
    ├── crates/
    ├── conformance/
    ├── examples/
    ├── docs/
    ├── themes/
    ├── tooling/
    ├── xtask/
    └── ...
```

Do not flatten all crates into one `crates/` directory during the initial import.

### Workspace boundary

The root manifest is the only product workspace. Check nested workspace roots
with:

```bash
find engine framework -name Cargo.toml -print0 \
  | xargs -0 rg -l '^\[workspace\]' \
  | sort
```

The only allowed results are the downstream app-manifest fixture and the
WASM-only `hello_web` example. Check nested lockfiles with:

```bash
find engine framework -name Cargo.lock -print | sort
```

Only the downstream app-manifest fixture may retain a tracked lockfile. The
WASM example ignores its generated lockfile and keeps its nightly config.

Do not rename public crates merely to make paths look uniform.

Do not deduplicate same-named packages until their code, API, dependency role, publication status, and downstream consumers have been audited. In particular, treat the two existing `reqwest_client` packages as distinct until proven otherwise.

---

## Architectural ownership

### Engine ownership

`engine/` owns generic UI-engine and platform mechanisms:

- application core and entity model
- renderer and scene construction
- text system
- event loops
- windows, overlays, popups, and platform surfaces
- input and IME primitives
- focus primitives
- clipboard primitives
- drag and drop primitives
- accessibility primitives and platform adapters
- platform dispatch and main-thread wakeup
- renderer selection and adapter evidence
- first-presentation evidence
- engine test platforms and renderer tests
- macOS, Windows, Linux X11, Linux Wayland, and Web backends

The engine must not depend on the framework.

The engine must not contain:

- AppShell policy
- framework settings schema
- framework command or menu policy
- framework theme policy
- framework package generation
- product-specific lifecycle decisions
- product-specific storage layouts
- framework component behavior
- updater, tray, or notification policy owned by the SDK

When a framework bug exposes a missing generic primitive, add the smallest reusable primitive to `engine/`; do not leak framework concepts into the engine.

### Framework ownership

`framework/` owns application-SDK policy and reusable desktop components:

- `gpui-component`
- AppShell
- application identity and manifest code generation
- application paths and storage
- lifecycle phases and plugin ordering
- `AppInfo`, `AppProxy`, and shell liveness
- managed windows
- commands and menu projections
- settings and theme integration
- component library and design system
- Root, dialogs, sheets, notifications, focus traps, input, dock, tables, trees, editor, and related controls
- examples, story gallery, documentation, and templates
- native conformance application
- Stage 1 process supervision and evidence tooling
- framework compatibility, packaging, and release checks

Framework crates may depend on engine crates. Engine crates may never depend on framework crates.

### Conformance ownership

The native conformance application belongs in `framework/conformance/`.

It may consume:

- public framework APIs
- public engine APIs
- narrowly feature-gated engine test/conformance hooks

It must not turn test-only hooks into production policy.

A conformance result must never claim more than its actual evidence level.

---

## Dependency direction

The intended dependency direction is:

```text
root coordination tooling
        ↓
framework tooling and conformance
        ↓
framework app/components/services
        ↓
engine public crates
        ↓
engine platform/renderer/internal crates
```

Forbidden directions include:

```text
engine → framework
engine platform backend → AppShell
engine renderer → framework theme/component policy
framework component → private engine implementation by file/path coupling
```

Use public crate interfaces. Do not create file-level cross-subtree includes or symlink source files between `engine/` and `framework/`.

---

## Cargo dependency model inside the monorepo

After import, framework dependencies on engine crates must use **path plus exact version**.

The root workspace should have declarations equivalent to:

```toml
[workspace.dependencies]
gpui = {
    package = "bumpyclock-gpui",
    path = "engine/crates/gpui",
    version = "=0.1.0",
    features = ["font-kit"]
}

gpui_platform = {
    path = "engine/crates/gpui_platform",
    version = "=0.1.0",
    features = ["font-kit"]
}

gpui-macros = {
    package = "gpui_macros",
    path = "engine/crates/gpui_macros",
    version = "=0.1.0"
}

sum-tree = {
    package = "sum_tree",
    path = "engine/crates/sum_tree",
    version = "=0.1.0"
}
```

Use the actual versions and features present at migration time.

Rules:

1. Remove the old `git = "https://github.com/BumpyClock/gpui"` and `rev = ...` fields from framework-to-engine dependencies.
2. Use root-relative path dependencies with exact engine versions.
3. Keep exact registry versions for packages that affect or appear in public framework API.
4. Preserve package aliases and Rust import names.
5. Do not add a second copy of an engine crate from crates.io or another Git source.
6. The root lockfile must resolve one local engine source identity.
7. A packaged framework manifest must discard `path` and retain the exact registry package/version.
8. A Git consumer of the framework may resolve the in-repository path dependency from the same checkout.
9. Do not store the monorepo's own current commit SHA in a tracked compatibility file. A tracked file cannot safely pin the commit that contains itself.
10. Exact source identity belongs in generated CI evidence: repository, HEAD, tree, workflow run, and relevant file digests.

### Cargo patches

Cargo `[patch]` sections apply from the single root workspace manifest.

- Preserve the engine patch table at the root.
- Validate its effect on engine and framework package graphs.
- Do not copy patches without proving their graph effect.
- Publication tooling must report root-patch limitations honestly.

### Versions and MSRV

Engine and framework versions remain independent.

At migration time the expected version families are approximately:

- engine public packages: `0.1.x`
- framework public packages: `0.7.x`

Verify actual versions before editing.

Do not force a single workspace version.

Do not silently raise the framework’s declared MSRV to the engine’s MSRV merely because the pinned development toolchain is shared.

Use one root development toolchain pinned to the current shared 1.95.0 toolchain, with the engine's required components and targets. Remove nested toolchain files only after proving the root file is a functional superset. Package `rust-version` contracts remain explicit and must be validated independently.

---

## Compatibility metadata after consolidation

The old two-repository compatibility model pinned a framework source tree to an external GPUI Git revision. That model must change.

`framework/compatibility.toml` or its replacement should record:

- framework version
- engine package names and exact versions
- engine path within the monorepo
- Zed upstream repository and audited upstream base
- framework MSRV and pinned toolchain
- registry status and publication blockers
- platform evidence and maturity

It should not record a self-referential monorepo HEAD SHA.

The Stage 1 source manifest must record the exact monorepo commit and tree used for runtime evidence.

Update compatibility tooling to validate:

- required engine paths exist
- package names and aliases match
- path dependencies use exact versions
- every framework engine dependency belongs to `engine/`
- no old BumpyClock/gpui Git dependency remains
- no floating engine dependency exists
- no duplicate engine package identity is resolved
- framework package normalization retains registry versions and removes paths
- generated documentation is current

---

## Snapshot import rules

Import exact committed source snapshots with a documented, reviewable method.

1. Record each source repository, commit, tree, branch, dirty status, tags, and remotes.
2. Export each selected commit with `git archive` from a temporary read-only source checkout.
3. Place the engine snapshot under `engine/` and the framework snapshot under `framework/`.
4. Exclude source Git metadata, source histories, source tags, untracked files, and build output.
5. Commit each domain snapshot separately in Neutron.
6. Record source and destination snapshot identities plus validation evidence in `MIGRATION.md`.
7. Keep both source repositories unchanged.

Do not:

- run history-rewrite tools against source repositories
- claim source history preservation
- import source tags or generated commit mappings
- mix feature changes into snapshot commits
- remove license or attribution files during path moves

Use `.git-blame-ignore-revs` only for a genuinely mechanical formatting commit, not to hide semantic changes.

---

## Upstream relationships

### Zed → engine

The engine is a selective hard fork of GPUI from:

- `https://github.com/zed-industries/zed`

`engine/fork.toml` is the machine-readable source of truth for:

- upstream URL
- audited upstream base/cursor
- synchronization date
- maintained patch clusters
- provisional registry identities

`engine/UPSTREAM.md` explains the semantic sync process and fork invariants.

When syncing Zed:

1. clone or update Zed in `/tmp/zed`
2. inventory relevant GPUI-family changes from the recorded base to the proposed target
3. classify each change as:
   - already integrated
   - compatible import
   - fork-modified adaptation
   - competing implementation
   - excluded
4. preserve fork-specific renderer, overlay, accessibility, scheduler, lifecycle, and platform contracts
5. import in dependency order
6. run engine tests
7. run downstream framework checks
8. update provenance only after validation

Never overwrite `engine/` with an upstream tree.

### Longbridge → framework

The component system originated from:

- `https://github.com/longbridge/gpui-component`

Preserve Longbridge attribution and existing license notices.

Record the framework upstream relationship in a durable `framework/UPSTREAM.md` or equivalent machine-readable/human-readable pair. Do not leave the only provenance note in an old README paragraph.

When importing Longbridge changes:

- compare semantics, not only file names
- preserve local AppShell, component, motion, material, accessibility, and platform-parity contracts
- avoid wholesale replacement
- document conflicts and adaptations
- run framework and native conformance checks

The two upstream streams are independent. A Zed sync must not silently overwrite framework behavior, and a Longbridge sync must not directly patch private engine internals.

---

## Stage 0 invariants to preserve

Stage 0 established the project’s release-engineering foundation.

The monorepo migration must preserve:

- exact package identities
- exact version requirements
- deterministic package graph analysis
- generated or mechanically checked compatibility docs
- fork provenance validation
- explicit crates.io blockers
- immutable release/tag policy
- unit, headless, native, packaging, and compile-only test categories
- honest platform support language
- no hidden Git-only normal dependency in a publishable package
- engine packages published before dependent framework packages
- normalized manifest inspection

The migration may change how source identity is represented, but it must not weaken the checks.

### Known publication blockers

Do not “solve” these implicitly during consolidation:

- owner approval/control of engine registry package identities
- conflicts with existing registry package names
- framework ownership/control of historical Longbridge package identities
- Git-only normal dependencies such as `fix-path-env`
- engine root patches that do not transfer into packaged consumers
- bundled asset/icon provenance and license review
- provisional registry-name collisions, including distinct `gpui_util` and `util` crates

Do not publish anything during consolidation.

---

## Stage 1 invariants to preserve

Stage 1 established or implemented contracts for:

- caller-owned native and headless lifecycle return
- transactional AppShell startup
- typed startup failure
- exactly-once shutdown
- reverse plugin teardown
- closed cross-thread admission after shutdown begins
- queued startup-event delivery stopping at shutdown
- terminal platform-loop failure not becoming success
- deterministic X11 buffered-event delivery
- renderer/adapter evidence
- first-presentation evidence
- WGPU recovery adapter policy
- external native clipboard verification
- bounded process-tree supervision
- Windows pre-start Job Object membership
- focus/text, composition, scale, and accessibility interaction contracts
- exact-source evidence manifests

Do not regress these contracts while moving files.

### Evidence reset after migration

The monorepo changes source paths, repository identity, commit identity, manifests, lockfiles, and workflows. Therefore:

- previous exact-source Stage 1 evidence is historical
- do not mark the monorepo verified from old artifacts alone
- keep platform statuses conservative until the monorepo matrix passes
- rerun Stage 1 on one exact final monorepo commit
- retain source manifests, validation output, renderer data, service logs, watchdog logs, and aggregate acceptance evidence
- make no source commit after the final accepted evidence run without rerunning acceptance

The source manifest should digest at least:

```text
Cargo.toml
Cargo.lock
engine/fork.toml
framework/compatibility.toml
.github/workflows/stage1.yml
```

It must also record the root monorepo HEAD and tree.

---

## Platform parity and evidence honesty

Platform parity is a required design discipline, not permission to fake parity.

For every platform-sensitive change, consider separately:

- macOS
- Windows
- Linux X11
- Linux Wayland
- Web only where the affected engine API supports it

A feature may be:

- supported and tested
- supported but not yet runtime-tested
- experimental
- compile-only
- explicitly unsupported

Unsupported behavior must be:

- represented by a typed error or capability result
- documented
- covered by a test where practical

Do not implement silent no-ops to make APIs appear cross-platform.

Do not infer runtime support from:

- source code existing
- successful compilation
- a headless test
- a native window handle alone
- a presentation API call alone
- software-GPU CI alone

Maintain these evidence distinctions:

- unit proof
- headless integration proof
- native event-loop proof
- native window proof
- presentation API submission
- backend acceptance
- software-GPU proof
- hardware-GPU proof
- manual-only proof

Custom-rendered controls are not native OS widgets. Say “native application/runtime integration with custom-rendered controls.”

---

## Framework architecture essentials

### Component initialization

AppShell owns `gpui_component::init(cx)` during its component-initialization phase.

Do not call it again in an AppShell `start` callback.

Applications that bootstrap GPUI manually must call `gpui_component::init(cx)` before using framework components.

### Root

The first framework view in a normal window should be `Root`, which owns framework overlays such as:

- sheets
- dialogs
- notifications
- keyboard traversal behavior

Do not bypass `Root` accidentally when building ordinary framework windows or conformance fixtures.

### Component principles

- Prefer stateless `RenderOnce` components where practical.
- Follow existing size, style, and builder conventions.
- Desktop buttons use the default cursor; link-like controls may use a pointer cursor.
- Preserve accessibility roles, names, values, states, actions, and focus semantics.
- Preserve reduced-motion behavior.
- Do not add a visual feature to one renderer/backend and leave the others silently different.
- Test complex branching, state transitions, geometry, lifecycle, accessibility, and builder contracts.
- Do not add tests whose only value is restating trivial field assignment.

### AppShell

AppShell owns application policy, not the engine.

Preserve:

- explicit startup phases
- immutable app identity
- event queueing before readiness
- startup as one fatal composition transaction
- nonfatal runtime error reporting
- liveness holds
- managed-window identity
- one command vocabulary projected into menus and command surfaces
- reverse shutdown
- product-specific state remaining outside the generic framework unless explicitly designed otherwise

---

## CI design in the monorepo

Root workflows should coordinate both domains through the single root workspace and explicit package selections.

Recommended workflow ownership:

- `engine-ci`: engine format, metadata, fork validation, package plan, tests, platform builds, WASM
- `framework-ci`: framework format, lint, unit, doctest, headless, compatibility, packaging, examples
- `stage1`: exact-source headless and native runtime matrix
- `monorepo-integration`: path-dependency, source-identity, repository-link, and cross-workspace checks

Rules:

1. Pin third-party Actions by immutable commit.
2. Use least-privilege permissions.
3. Do not use `continue-on-error` for mandatory gates.
4. Upload evidence on failure.
5. Keep job names stable for branch protection.
6. Engine changes must run downstream framework checks.
7. Engine lifecycle, platform, renderer, input, accessibility, or presentation changes must run the applicable Stage 1 matrix.
8. Framework-only component changes need not rerun unrelated engine-only tests when safe, but must not skip engine resolution and compatibility checks.
9. Root, dependency, lockfile, workflow, or shared-tooling changes run both sides.
10. Path filters must not let an engine change merge without downstream framework validation.
11. A skipped job is not a passed evidence job.
12. Preserve exact-source manifests after the repository move.

---

## Canonical commands

Create thin root scripts that invoke existing workspace-native commands. Do not duplicate test logic in shell when an xtask already owns it.

### Root coordination

Expected root entry points:

```bash
./script/bootstrap
./script/check
./script/test
./script/release-check
./script/stage1
```

Until those scripts exist, use the workspace commands below.

### Root workspace

Run these from the repository root:

```bash
cargo fmt --all -- --check
cargo metadata --locked --format-version 1
cargo check --locked --workspace --all-targets
cargo clippy --locked --workspace --all-targets --all-features -- --deny warnings
cargo test --locked --workspace --all-targets --features test-support
cargo test --locked --workspace --doc
cargo run --locked -p engine-xtask -- fork validate
cargo run --locked -p engine-xtask -- publish-plan
cargo run --locked -p framework-xtask -- compatibility check
cargo run --locked -p framework-xtask -- publish-plan
cargo run --locked -p framework-xtask -- release-check
cargo test --locked -p scheduler
cargo test --locked -p bumpyclock-gpui --features test-support
cargo test --locked -p gpui_wgpu --lib
python3 -m unittest discover -s framework/tooling -p 'test_*.py'
python3 -m unittest discover -s framework/tooling/tests -p 'test_*.py'
```

On Windows use the repository’s PowerShell/Python equivalents.

Build framework documentation from `framework/docs` with `bun install
--frozen-lockfile` and `bun run build`.

### Native Stage 1 matrix

The required profiles remain:

```text
headless macOS
headless Windows
headless Linux
native macOS Metal
native Windows D3D11 WARP
native Linux X11 + Xvfb + lavapipe
native Linux Wayland + isolated Weston + lavapipe
aggregate Stage 1 acceptance
```

Run the complete matrix after the final structural commit.

**Tests are required.** Remove any copied instruction saying tests do not need to run.

---

## Tooling and xtask collisions

Both source repositories contain a package named `xtask`.

The single root workspace must give every package a unique Cargo package name. Resolve the existing `xtask` and `reqwest_client` collisions with explicit package renames while preserving required Rust library target names.

Root scripts must select engine and framework tooling by explicit package name or manifest path. Keep tested Rust tooling as the source of business rules.

Do not move business rules out of tested Rust tooling into ad hoc shell scripts merely to create one command.

The same caution applies to same-named support crates such as `reqwest_client`.

---

## Documentation and link migration

Update repository links only after the destination repository name is final.

Audit:

- Cargo `repository` and `homepage`
- README badges
- README installation snippets
- docs links
- issue templates
- PR templates
- release documentation
- compatibility metadata
- build scripts
- workflow URLs
- source-manifest repository checks
- skills and agent instructions
- old source-repository links
- package metadata
- code comments containing immutable old paths

Preserve links to historical source commits where they provide provenance.

Replace obsolete sibling-checkout guidance such as the old GPUI local override workflow with the monorepo path-dependency workflow.

Do not leave a framework instruction telling agents to update a GPUI Git SHA after path dependencies replace that SHA.

Do not carry two full copies of agent instructions into the monorepo. Root `AGENTS.md` is authoritative; subtree documents should cover domain details, not repeat global policy.

---

## Licensing and attribution

Preserve:

- Apache license files
- Zed attribution
- Longbridge attribution
- third-party notices
- asset licenses
- source headers
- package `license` or `license-file` metadata

Moving files does not grant permission to relicense them.

Do not:

- delete upstream notices because the repository changed
- choose a new license
- claim ownership of a registry package
- resolve ambiguous asset provenance without owner/legal review

Record any ambiguity precisely and leave publication blocked.

---

## Editing discipline

Before making changes:

```bash
git status --short
git branch --show-current
git rev-parse HEAD
git remote -v
```

Rules:

- preserve uncommitted user work
- do not use `git reset --hard`
- do not use `git clean`
- do not rewrite shared history
- do not force-push
- do not move tags
- do not publish
- do not perform drive-by formatting
- do not mix feature work into structural migration
- do not silently delete tests
- do not lower assertions to make CI green
- do not add arbitrary sleeps as readiness mechanisms
- do not replace native tests with headless fakes
- do not report an unexecuted command as passing

Prefer small commits with one purpose and explicit validation.

When a change crosses engine and framework:

1. implement the generic engine change
2. test the engine
3. update the framework path consumer
4. run framework checks
5. run affected native conformance
6. commit the coherent result atomically in the monorepo

The monorepo removes the need to invent a future engine SHA. Never reintroduce a synthetic internal pin.

---

## Migration-specific non-goals

The consolidation must not also implement:

- Stage 2 component stabilization
- a project rebrand
- tray support
- updater support
- notifications
- single-instance behavior
- deep-link registration
- installer packaging
- JavaScript or TypeScript bindings
- a React reconciler
- WebView-first application architecture
- broad crate renaming
- broad dependency upgrades
- public API cleanup unrelated to moved paths
- registry publication
- release creation
- tag creation or movement

If a migration failure exposes a real pre-existing bug, reproduce it and fix the smallest root cause. Keep the fix in a separate commit from mechanical import where practical.

---

## Consolidation completion gates

The monorepo migration is complete only when all of the following are true.

### Source provenance

- exact source commit and tree identities are documented
- snapshot import method and destination commits are documented
- source repositories remain unchanged
- source license and attribution files are preserved
- no source history or source tag is claimed as imported

### Structure

- engine lives under `engine/`
- framework lives under `framework/`
- root instructions and navigation exist
- one root Cargo workspace and lockfile exist
- no nested product workspace or lockfile remains; only the approved
  downstream fixture and WASM-only `hello_web` workspace exceptions remain
- no nested Git repository or submodule remains
- no vendored copy of the old GPUI repository remains
- obsolete sibling override instructions are removed or rewritten

### Dependencies

- framework engine dependencies use path plus exact version
- no framework dependency still points to `BumpyClock/gpui` by Git SHA
- no duplicate GPUI source identity resolves
- one root lockfile is current
- framework packaged manifests retain exact registry versions
- Cargo patch behavior is explicit and tested

### Architecture

- engine has no framework dependency
- framework behavior remains above engine mechanisms
- AppShell and component initialization contracts remain correct
- no platform policy moved into the wrong subtree

### Stage 0

- fork validation passes
- compatibility checking passes
- package plans pass at their documented readiness level
- release checks pass
- publication blockers remain honest
- no package was published

### Stage 1

- lifecycle and pending-event tests pass
- process-supervision tests pass
- conformance validator tests pass
- all headless jobs pass
- all four native profiles pass
- exact-source manifests refer to the final monorepo commit
- aggregate acceptance passes
- support metadata reflects only the new evidence
- no later source commit invalidates acceptance

### Documentation

- root README explains engine versus framework
- upstream relationships are documented independently
- repository URLs are current
- issue templates request versions/commits without stale hardcoded examples
- support wording distinguishes maturity from evidence
- custom-rendered controls are not marketed as native widgets

---

## Required final report for the consolidation agent

Return:

### 1. Baseline

- source repository SHAs
- source dirty status
- tags/releases
- current versions and MSRVs
- current Stage 0 status
- current Stage 1 evidence status

### 2. Import method

- destination repository
- snapshot import method
- path mapping
- destination snapshot commit location
- source-repository preservation status

### 3. Resulting layout

Show the root tree and the single root Cargo workspace.

### 4. Dependency conversion

- old Git+rev declarations
- new path+exact-version declarations
- lockfile changes
- package normalization result
- patch behavior

### 5. Tooling and CI

- root scripts
- workflow ownership
- source-identity changes
- compatibility-tool changes
- xtask/package-name collision handling

### 6. Validation

For every command:

- exact command
- working directory
- platform
- exit status
- pass/fail
- artifact or log location
- whether a watchdog intervened

### 7. Stage 0 verdict

State separately:

- foundation preserved or not
- registry publication ready or not
- remaining blockers

### 8. Stage 1 verdict

State exactly one:

```text
STAGE 1 COMPLETE IN MONOREPO
```

or:

```text
STAGE 1 NOT COMPLETE IN MONOREPO
```

List only concrete blockers if incomplete.

### 9. Remaining work

List deferred work without implementing it.

### 10. Commits and pull requests

List actual commits and PRs. Confirm that no package, release, or tag was published or created.

---

## Final principle

The monorepo is meant to make coordinated engine/framework changes atomic.

It must not erase the distinction between engine mechanism and framework policy, weaken upstream provenance, hide publication blockers, or convert configured tests into unsupported claims.

A successful consolidation leaves one repository, one root Cargo workspace, two clear architectural domains, reproducible package graphs, immutable snapshot provenance, and exact-source evidence for the final code.
