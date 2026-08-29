# Stage 1 Contract

This file is the Stage 1 behavior contract and part of the canonical identity
set. It states required startup and teardown order, evidence clauses,
exact-source identity, source-blind validation, evidence levels, and
non-claims. `TESTING.md` and the
[runtime-evidence documentation](docs/docs/runtime-evidence.md) link here
instead of repeating those clauses. The clauses are requirements, not a claim
that a retained CI run already implements or evidences them.

## Startup order

```text
framework modules
→ application SetupModules
→ common start hook
→ typed LaunchSpec before_primary
→ primary surface
→ Started
```

A setup failure skips common start, `before_primary`, primary creation, and
readiness.

## Teardown order

```text
WillExit
→ application shutdown hook
→ SetupModules in reverse resolved order
→ framework modules in reverse order
```

## Pure declaration behavior

Declaration-level logic (`AppDeclaration`, `DeclarationErrors`, typed
capability and identity validation) is pure: deterministic, event-loop-free,
and covered by ordinary Rust unit tests. Pure tests establish logic and state
transitions only. They make no native, headless, or presentation claim.

## Headless behavior

The three `stage1-lifecycle-headless-*` jobs run the injected headless runner
through `cargo test --locked -p neutron-components-app --test headless
--features test-support`. They prove AppShell lifecycle order and normal
return, startup failure, and shutdown ordering without a native event loop,
window, renderer, or presentation.

## Native profile evidence

The four native jobs (`stage1-native-macos-metal`, `stage1-native-windows-warp`,
`stage1-native-linux-x11-lavapipe`, `stage1-native-linux-wayland-lavapipe`) run
an actual platform event loop and validate a schema-versioned JSONL scenario
trace against an explicit profile. Each job establishes only the evidence
level and adapter classification stated for its profile in the
[runtime-evidence profile table](docs/docs/runtime-evidence.md#platform-specific-automated-scope).
A native window or event-loop return does not by itself prove presentation,
backend acceptance, or display scanout.

## Story smoke

Each of the four native jobs runs `neutron-story --smoke` in that job's
existing platform environment. Stage 1 sets `GPUI_STAGE1_STORY_EVIDENCE_PATH`
to a file in the same job artifact directory. That variable is the only
opt-in. With it set, `--smoke` writes one schema-versioned JSONL stream. The
job then validates the stream with:

```text
neutron-components-conformance --validate story-smoke --profile <profile>
```

The validator requires exactly these ten records, in this order:

```text
story_started
primary_opened
menu_projected
themes_loaded
first_presented
quit_requested
shutdown_requested
will_exit
run_returned
terminal
```

This stream proves the gallery's typed `DesktopApp` declaration opened its
primary Gallery surface, observed first presentation once, resolved the
declared platform menu model, loaded a nonzero bundled theme catalog,
requested quit after that observation, shut down through AppShell, and
returned `Ok`. Profile validation pins only the platform family recorded on
`menu_projected`. The stream must not carry native window, display, renderer,
or presentation-backend records. Extra, missing, duplicated, or reordered
records fail.

Story smoke does not prove OS menu pixels or activation, display scanout,
broad rendering, input, clipboard, accessibility, arbitrary themes, or
settings edits.

Ordinary Framework CI StoryApp CLI gates (`--help`, `--version`,
`--asset-smoke`, `--fail-start`, and a macOS-only evidence-free `--smoke`)
are regression coverage. They are not retained Stage 1 evidence. An ordinary
`--smoke` run without `GPUI_STAGE1_STORY_EVIDENCE_PATH` writes no evidence
file.

Story-smoke JSONL stays in the four native job artifacts. It does not add an
eighth upload. Aggregate acceptance still requires exactly seven source
manifests and seven verifications.

## Exact-source identity

The Stage 1 workflow supports `workflow_dispatch` so acceptance can bind a real
candidate branch SHA. `workflow_dispatch` of a candidate branch is available
only once this workflow version, the one containing the `workflow_dispatch`
trigger, exists on the repository's default branch. GitHub does not offer
manual dispatch for a workflow version that exists only on a non-default
branch. Until then, and for any run that is not an accepted
`workflow_dispatch` candidate, pull-request runs bind ephemeral merge commits
and remain rehearsal evidence only.

Every Stage 1 job records `source-manifest.json` before build or execution.
The manifest binds the checked-out commit, tree, GitHub workflow identity, a
clean-checkout requirement, and the committed-Git-blob SHA-256 digest of each
file in the canonical identity set below. An `always()` step repeats the
snapshot after execution and writes `source-verification.json`. A changed or
dirty source tree fails the job. The aggregate `stage1-acceptance` job
requires exactly seven uploaded source manifests and seven verifications, each
matching the accepted commit, tree, clean status, and workflow identity, and
each reporting the same canonical digest map computed from the accepted
commit. The jobs hash committed Git blobs so checkout line-ending filters
cannot create platform-specific source identities.

Canonical identity set (same order and content in
`framework/tooling/stage1_source_manifest.py` `IDENTITY_FILES`, the
`stage1-acceptance` aggregate `identity_paths`, and this list):

1. `Cargo.toml`
2. `Cargo.lock`
3. `engine/fork.toml`
4. `framework/compatibility.toml`
5. `.github/workflows/stage1.yml`
6. `framework/crates/app/Cargo.toml`
7. `framework/crates/story/Cargo.toml`
8. `framework/STAGE1-CONTRACT.md`
9. `framework/tooling/stage1_source_manifest.py`
10. `.github/actions/stage1-source-identity/action.yml`

A retained run counts as exact-source evidence only when both files report
the same identity and verification passed.

## Source-blind validation

After an accepted seven-job matrix and aggregate pass on one candidate SHA,
source-blind validation runs a prewritten contract against binaries built
from that SHA, without source access, on macOS, Windows, Linux X11, and
Linux Wayland. It requires clause-by-clause reports and an aggregate GO. A
NO-GO blocks acceptance. Any code fix requires a new SHA and repeats Stage 1
rehearsal and acceptance.

## Evidence levels

| Level | What it establishes | What it does not establish |
|---|---|---|
| Pure proof | Isolated declaration logic and deterministic state transitions. | An event loop, native window, or graphics API call. |
| Headless integration proof | AppShell lifecycle behavior through the injected headless runner. | A native OS event loop, native window, clipboard, renderer, or presentation. |
| Native event-loop proof | Normal return after an actual platform event loop processes the scenario. | That a native window or renderer was selected unless separate records prove it. |
| Native window proof | Matching pointer-free raw window and display classification for the target platform. | Pointer values, a visible desktop frame, rendering, presentation, or display scanout. |
| Presentation API submission proof | The renderer reached its platform-specific first presentation API observation. | Backend acceptance, compositor presentation, display scanout, or user-visible pixels. |
| Backend-acceptance proof | The platform-specific conformance path reports backend acceptance or scheduling. | Display scanout, presentation completion, or general GPU correctness. |
| Software/hardware-GPU proof | The selected adapter's classification (software or hardware) matches recorded evidence. | Performance, thermal behavior, or broad device compatibility. |
| Story-smoke proof | A ten-record `story-smoke` JSONL stream from `neutron-story --smoke` with `GPUI_STAGE1_STORY_EVIDENCE_PATH` set, validated against the native profile. Proves typed `DesktopApp` declaration, primary opening, first-presentation observation, declared menu model, nonzero bundled theme catalog, shutdown ordering, and clean `Ok` return. | OS menu pixels or activation, display scanout, broad rendering, input, clipboard, accessibility, arbitrary themes, or settings edits. Native handle, renderer, and presentation-backend groups are rejected. |
| Exact-source proof | A retained artifact's manifest and verification both match the accepted commit, tree, and clean status. | Behavior correctness independent of the recorded test outcome. |
| Source-blind proof | An external, source-blind contract run reports GO on the accepted SHA. | Anything beyond the prewritten contract's stated clauses. |
| Manual-only proof | A capability needs a human or dedicated external accessibility/input evaluation. | Automated certification. |

## Explicit non-claims

- Retained Stage 1 evidence covers only the jobs and profiles defined in this
  contract; a skipped job is not a passed evidence job.
- Native window construction and matching display-handle classification are
  not presentation evidence.
- First-presentation evidence is not display scanout evidence.
- Software-adapter profiles (WARP, lavapipe) are not hardware GPU evidence.
- Story smoke is not OS menu pixel or activation proof, display scanout
  proof, broad rendering proof, or input, clipboard, accessibility, arbitrary
  theme, or settings-edit certification.
- The automated suite does not certify comprehensive IME behavior or
  VoiceOver, Narrator, Orca, or any other screen-reader integration.
- Previous exact-source Stage 1 evidence is historical for a new interface
  or commit; no later source commit may claim an already-accepted run's
  evidence.
