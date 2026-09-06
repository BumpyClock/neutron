# AGENTS.md

Neutron is a Rust-native desktop SDK with one product Cargo workspace and two architectural domains.
This file owns repository policy. Subtree guides supply domain-specific contracts.

## Scope and safety

- Preserve unrelated user changes and existing behavior outside the requested scope.
- Require explicit authorization for publication, tags, releases, destructive operations, and shared-history rewrites.
- Preserve source repositories, licenses, attribution, and publication blockers.
- Keep Stage 2 feature work outside consolidation unless the owner changes scope.
- Report only evidence from the checked source and environment.
- Continue safe local work through affected validation without repeated approval.
- Stop at a blocker that requires unavailable evidence or new authority.

## Architecture

`engine/` owns generic UI mechanisms, platform backends, rendering, input, accessibility, and event loops.
`framework/` owns AppShell, application policy, reusable components, and native conformance.
Framework crates may depend on public engine crates. Engine crates must not depend on the framework.
Use public crate interfaces rather than cross-domain source includes or symlinks.

Keep one root workspace, lockfile, development toolchain, profile policy, lint policy, and patch table.
Retain only these isolated workspace exceptions:

- `framework/crates/app-manifest/tests/fixtures/downstream-app`
- `engine/crates/gpui_web/examples/hello_web`

Preserve their target-specific configuration. The downstream fixture may retain its tracked lockfile.
The WASM example retains its nightly toolchain and ignores its generated lockfile.
Keep engine and framework versions, package MSRVs, publication graphs, and upstream relationships independent.
Use framework-to-engine path dependencies with exact versions and preserved public package aliases.
Keep registry versions in normalized package manifests.
Do not introduce duplicate engine source identities or self-referential monorepo commit pins.

## Task references

Read only the material relevant to the affected contract:

| Task | Reference |
| --- | --- |
| Engine mechanisms or platform changes | [Engine guide](engine/AGENTS.md) |
| Components or AppShell policy | [Framework guide](framework/AGENTS.md) |
| Engine upstream synchronization | [Upstream rules](engine/UPSTREAM.md) and `engine/fork.toml` |
| Compatibility or package changes | `framework/compatibility.toml` and [compatibility documentation](framework/docs/COMPATIBILITY.md) |
| Framework test selection | [Testing guide](framework/TESTING.md) |
| Release or publication readiness | [Release guide](framework/RELEASING.md) |
| Snapshot imports or consolidation acceptance | [Consolidation specification](docs/CONSOLIDATION.md) and [source records](MIGRATION.md) |

For affected lifecycle, shutdown, conformance, or evidence contracts, consult the corresponding sections of the consolidation specification.
Its import itinerary and final-report template do not apply to unrelated routine edits.

## Validation

Select checks by the affected contract. Reuse valid results when source, inputs, command, and environment remain applicable.
Routine documentation changes need link and content checks, not the native platform matrix.

| Change | Validation |
| --- | --- |
| Focused engine or framework behavior | Affected tests and domain checks |
| Engine behavior | Engine tests and downstream framework checks |
| Engine lifecycle, platform, renderer, input, accessibility, or presentation | Applicable Stage 1 profiles and downstream checks |
| Framework-only component | Affected framework tests plus engine resolution and compatibility checks |
| Root manifests, dependencies, lockfile, CI, or shared tooling | Both domains |
| Package metadata or release readiness | `./script/release-check` and applicable release requirements |
| Framework documentation site | The documentation build from `framework/docs` |
| Final consolidation acceptance | The full exact-source matrix and gates in the consolidation specification |

Use `./script/check`, `./script/test`, and `./script/stage1` for their relevant checks.
Use focused package tests when they observe the affected contract sufficiently.
Preserve required integration checks. Do not replace native evidence with headless tests.

## Evidence and provenance

Keep compile-only, headless, native, software-GPU, hardware-GPU, and manual evidence distinct.
Do not infer runtime support from compilation or a native window handle.
Preserve accessibility, reduced motion, and typed capability errors instead of silent platform no-ops.
Describe the controls as custom-rendered, not native operating-system widgets.

Record exact source commits, trees, manifests, and acceptance artifacts for Stage 1 claims.
Historical pre-consolidation evidence does not establish current monorepo acceptance.
Revalidate exact-source acceptance after a source change that invalidates it.
Do not claim consolidation completion without every required consolidation gate.

Record immutable snapshot provenance in `MIGRATION.md`.
Do not claim imported source history or tags. Preserve both historical source repositories unchanged.
Keep the Zed and Longbridge upstream streams independent.
Leave unresolved registry ownership, asset provenance, and package blockers explicit.
