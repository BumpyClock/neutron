# Upstream synchronization

This repository is a standalone, selective hard fork of GPUI from
[`zed-industries/zed`](https://github.com/zed-industries/zed). It preserves APIs and renderer and
platform behavior that do not exist upstream while periodically importing compatible GPUI work.

`fork.toml` is the machine-readable source of truth for upstream URL, exact audited base commit,
synchronization date, fork URL, and maintained patch-cluster statuses. Run `cargo run --locked -p
engine-xtask -- fork validate` after a change. Do not duplicate these values in downstream release
metadata.

## Current provenance

- Historical README marker: `24f62484e936aa355c72f2009313bbe2898a9fd5` (2026-04-29).
- Current audited target and synchronization date: see `fork.toml`.
- The 2026-07 audit covered 1,493 Zed commits. It included 117 GPUI-family commits and 126
  GPUI-family files.
- The 2026-08 cursor-to-target audit covered 115 GPUI-family commits. Its broader extracted scope
  covered 129 commits and 143 changed files.
- Fork implementation range: commits after local baseline `510dedc3b44fcf80c791ab17e59530ae66e80558`.

The historical marker was only a lower-bound provenance note. Earlier fork commits `b373e0b` and
`8164718` had already imported some later upstream work. Neither upstream SHA is a byte-identical
fork baseline or a suitable Git merge base for this fork; the current target is an audit cursor.
The fork repository does not retain the Zed commit objects, so a Git merge-base cannot be computed
locally; the full audited target in `fork.toml` is the strongest verifiable provenance record.
The 2026-07 sync compared behavior and paths, then imported or adapted changes in reviewable local
commits. It did not merge the Zed repository or copy its root manifests and lockfile.

The 2026-08 sync also compared semantics and paths. It accepted 21 corrective commits through the
audited target. It also adopted one audited feature commit for native spring animation. It did not
merge the Zed repository or replace fork files with upstream files.

## Imported and reconciled areas

| Area | Local commits | Upstream references and result |
| --- | --- | --- |
| Preservation contracts | `800c73a` | Locked overlay, rounded blur, element backdrop blur, retained-layer, accessibility, scheduler, and device-recovery behavior before importing changes. |
| Approved local pending fixes | `e8bceee` | Ported reviewed semantic fixes from the fork-local, previously unmerged `ed805ad`; this is not attributed to the audited upstream range. |
| Standalone foundation and dependencies | `6ad4ac9`, `6cc040c` | Rust 1.95 foundation, standalone assets, dependency updates, `gpui_util` boundary, and crates.io WGPU 29.0.4. |
| Runtime and scheduling | `e4fc606` | Reconciled `b14229f1a0`, `74798c68d5`, `ee571d3c69`, and `2c4e44704c`: embedded application handles, system wake, lazy test scheduling, and shutdown fixes while retaining fork panic and `Send`-only output contracts. |
| View model | `2186631` | Ported `74b5207744` View/ViewElement semantics and macro support while retaining fork accessibility and retained-layer invalidation. |
| Profiling and benchmarks | `bb2ea30`, `256ff6c` | Reconciled `aa6f03bedd`, `297c4a4d78`, `39f7849a0f`, and `48511e0b9c`; profiling remains default-off and benchmarks cover CPU scene construction, backdrop blur, and retained layers. |
| Accessibility | `38ee1a8` | Reconciled `777e16dd1f`, `83d4847462`, and follow-ups: current roles/properties, synthetic nodes, active descendants, root titles, and wasm example while retaining stable IDs, rollback, mutation-safe listeners, and main-thread macOS ownership. |
| Rendering and GPU resources | `92e3e78`, `a2ea0ed`, `1120c02` | Reconciled `34cd17ff5e`, `0d8a4d4292`, `f281770034`, `fbd911ed3e`, `f791aa57d7`, and `56009f39b`: padded GPU booleans, atlas reclamation, SVG caps, box-shadow builders, WGPU recovery, and presented-frame reporting. |
| Remaining platform-neutral GPUI | `075969c` through `7737056` | Reconciled color, text wrapping, tooltip lifecycle, list scrolling, flex APIs, animation parenting, drag/hover cleanup, TypeId collections, prompt labels, and bounds/mouse refresh through the audited target. |
| Popups and input regions | `e9e548d`, `2a8b7b2`, `a551412`, `66835b0`, `5d9a7a7`, `cd0a58f`, `434dd20` | Ported `546a16d64f`, `a29c0d41f4`, `35eaeb94a7`, and `737f55a1a1`: the core `AnchoredPopup`/typed-error contract, generic input regions, initial app IDs, IME candidate bounds, Wayland implementation, and concrete unsupported-platform rejections. Anchored popups remain distinct from overlays. |
| Linux | `66835b0` | Reconciled 26 audited Linux commits, including `923f315f26` presented-frame commits, Wayland popup lifecycle/grabs, `a29c0d41f4` input regions, `bda5ac3` clipboard timeout, `35eaeb94a7` app IDs, `7c3160b` activation tokens, `f4364d8` headless windows, and `961f4f2` X11 mouse refresh. |
| macOS | `7c107e9`, `5d9a7a7` | Reconciled traffic lights (`137eeea67b`), font fallback/PostScript fixes (`20a93f6195`, `61ad9ebfcd`, `486cf9ef3c`, `aa16a3bf9d`), and per-window cursor rectangles (`1eba1ca72e`, `96165ec626`, `a03729b6c0`) across both windows and panels. |
| Windows | `cd0a58f` | Reconciled `23c0080d1d`, `7194f987a7`, `5aa6e8a0b3`, `ee571d3c69`, `9ac117693b`, `5c0b33f72e`, `7d19e89988`, and related current behavior: composition safety, controls, credentials, wake, hit testing, priority, and device recovery while retaining fork HostBackdrop and overlays. |
| Web and examples | `434dd20` | Reconciled `546a16d64f`, `a03729b6c0`, and `74b5207744`: explicit popup degradation and the View/a11y examples while retaining standalone web cursor and font behavior. |
| Verification and CI | `2e589cf`, `a207892`, `0330be6`, `55a253c` | Added strict quality gates plus macOS, Windows, isolated Wayland/X11, and nightly wasm jobs. |

## 2026-08 corrective sync

The prior cursor was `2c4e44704c37ee87e59ac84e3e17388178b28545`. The accepted target is
`d9ad6aff67e47de43abb270d22de75dd950f1b48`. The target tree is
`c56c35d41d03998657d77021b897db563f803c02`.

| Area | Upstream commit | Result |
| --- | --- | --- |
| Windows DirectWrite and DirectX safety | `89e8a4b9ec7e0fa8a49987a3c1dbf29343dc6999` | Imported bounds, null-pointer, staging-texture, and raw-pointer checks. |
| Element arena nested-draw safety | `ec3d887507f272119d9fe146c685f0a941d0e798` | Adapted deferred clears to Neutron's per-App arena scopes. |
| macOS display-link lifetime | `96ce8f2a05f8912851e5d20d808fe21f4134bd45` | Adapted per-display link ownership to current window callbacks. |
| macOS pasteboard lifetime | `914e1c9873b6b85bfeedc5b58e8270885bdd532e` | Retained pasteboard objects and returned owned byte buffers. |
| Wayland Fcitx5 cursor cache | `5079b33d657593ffd8a2d2978534988a8b40867e` | Cached cursor rectangles before protocol commits. |
| X11 close callback safety | `d9ad6aff67e47de43abb270d22de75dd950f1b48` | Released client state before the close callback. |
| X11 expose repaint | `ae99a867d7a24682435bd1821c66b4e172a10768` | Requested presentation after each deterministic drained event batch. |
| WGPU mixed-direction text | `c214057e086517920b214800725c5d16294ddf0d` | Adapted paragraph shaping to Cosmic Text 0.19.0. |
| Nested deferred popovers | `5e982c6bdc315a6d2bb68b3edefae00ceedd35f4` | Kept stable deferred indices and preserved accessibility reuse checks. |
| TestWindow raw handles | `7bddd16a09cf0084cefb3d98468b178343b9f1e2` | Returned `HandleError::NotSupported` instead of a panic. |
| XKB context validation | `c43e2d9734800bef0dd216ea9184fdb99bc60625` | Rejected null XKB contexts in both Linux backends. |
| Profiler lock order | `c16d19c94dbc1c62752b736186ab8fd6f1cd25c6` | Released the global lock before handle upgrades and added bounded thread-exit stress coverage. |
| Pending keys before IME input | `dc1e815e47835095748cbb036541992abd9ac826` | Completed focused pending bindings before IME text input. |
| Multi-modifier synthesis | `a8cae3bd77f6e1c1dde98bb1dcebba0d254dbd71` | Suppressed standalone modifier synthesis after multi-modifier input. |
| Inactive Wayland IME filtering | `655ed1385b00c01386bfbd9b7edea439eb1eec64` | Ignored cursor updates from inactive windows. |
| Web keyboard defaults | `5d447d4223f8455db4811ed332c4ad744bb13e40` | Prevented browser defaults only for handled input. |
| macOS appearance reentry | `a11083f9a79495e9c7ddee0c5782f22d07695c31` | Deferred appearance delivery through the foreground executor. |
| Image aspect ratio | `59b2ebf10351b5c0b5cd4403f01ed0460eeec06d` | Preserved an explicit element aspect ratio. |
| Rounded image cover | `58df5a14ce946fa27b14194d9d2fce8f87b1c331` | Cropped atlas tiles to destination bounds before rounded clipping. |
| Hover reconciliation | `38ca9106c5306ef93e52c35643df015a27f15b72` | Reconciled hover listeners after layout changes. |
| Prompt click isolation | `058f01fa93503491a735bfded53e77bfaa276148` | Blocked prompt clicks from mouse-down-out listeners behind prompts. |
| Native spring animation | `8b1497dbd22fb06f5838a7c0b84a1e54fafa71bc` | Adopted stateful, retargetable springs. Neutron uses the executor clock for deterministic tests and keeps duration-based presence motion. |

Neutron also coordinates Windows draws across windows. This adaptation prevents reentrant arena
borrows and preserves forced redraws after renderer recovery.

The spring feature does not replace retained compositor layers, backdrop blur, or native window
blur. The framework uses the native spring for the continuously mounted Switch thumb. Presence,
dialog, menu, and flyout transitions keep bounded duration-based animation because their exit
lifetime is part of the component contract.

## Dependency decisions

- The standalone toolchain is Rust 1.95.0 with rustfmt, Clippy, rust-analyzer, rust-src, and wasm
  targets recorded in `rust-toolchain.toml`.
- WGPU moved from the fork's git revision to crates.io `29.0.4`; custom backdrop-blur, retained-layer,
  recovery, and COPY_SRC paths were revalidated against that release.
- The sync adopted `ashpd` 0.13, `wayland-backend` 0.3.15, `resvg`/`usvg` 0.46,
  `accesskit_windows` 0.33.1, and `zed-font-kit` revision
  `94b0f28166665e8fd2f53ff6d268a14955c82269` where the standalone extraction uses them.
- Dependency declarations were migrated individually and the fork lockfile was regenerated locally.
  No upstream root manifest or lockfile was copied.

## Fork invariants

Future syncs must preserve these unless an explicit project decision changes them:

- `OverlaySurfaceOptions`, `OverlayInputMode`, and `App::open_overlay_surface` are display-oriented
  overlay contracts. They are not aliases for parent-relative `WindowKind::AnchoredPopup`.
- `WindowBackgroundAppearance::Blurred { corner_radius }` controls native compositor blur geometry.
  A zero radius is rectangular; a positive radius clips native blur and must survive resize, scale,
  and device recovery.
- `Styled::backdrop_blur` is an element effect that samples already-painted GPUI content. It is
  separate from native window blur.
- Retained compositor layers, retained-layer exclusions around backdrop blur, transform-aware
  hitboxes, easing bounds, public shadow painting, and inset shadows remain available.
- Overlay windows retain their platform-specific level, nonactivation, click-through, and
  all-spaces behavior. On Windows, zero-radius blur remains acrylic and positive-radius blur uses
  HostBackdrop. On macOS, overlays remain `NSPanel` instances.
- Fork accessibility keeps stable IDs, snapshot rollback, live-node sweeping, listener precedence
  and mutation safety, semantic keyboard click behavior, and macOS main-thread adapter ownership.
- Dedicated scheduler output only needs `Send`; thread and child panics must remain observable and
  must not stop later dedicated work.

## Adaptations and exclusions

- Native anchored popups are implemented on Wayland. X11, macOS, Windows, and web return the typed
  `PopupNotSupportedError` so callers can choose an in-window fallback.
- Wayland's generic input region was adapted into persistent, explicit precedence state so it can
  coexist with `OverlayInputMode`; copying upstream's transient setter would let later window
  updates erase the fork's overlay policy.
- The upstream Metal headless renderer was not copied into the fork renderer because it does not
  model the fork's retained-layer and backdrop-blur state. `current_headless_renderer` therefore
  reports `None`, and the benchmark harness uses deterministic CPU scene construction.
- Upstream async fallback `952119de41` was rejected because the fork deliberately panics when an
  `AsyncWindowContext` is bound to a closed window. That contract has focused tests.
- `f482f9e18c` and `2301e61d2a` depend on an unextracted proptest feature; `ccf4058b7a` is a
  Zed-specific keymap test; the relevant GPUI hunk of `84b753cb51` is whitespace-only;
  `2882636c06` was reverted upstream.
- The 2026-08 sync deferred Apple renderer extraction, the broad renderer resource rewrite, and
  Wayland demand-driven rendering.
- The sync excluded the Rust 1.97 and Taffy upgrades, legacy macOS blur removal, system
  notifications, window attention, and the Git-pinned `wasm_thread` workaround.
- The sync did not add unsupported silent no-op platform APIs.
- Zed application features and changes outside GPUI and the standalone dependency graph are out of
  scope. No Zed root `Cargo.toml`, `Cargo.lock`, workflow, or product assets are copied wholesale.
- Rounded native blur on pre-macOS 12 uses the legacy CGS path, which cannot apply the modern mask
  API and therefore degrades to rectangular blur. Wayland compositors without the KDE blur protocol
  and X11/web degrade to transparency rather than emulating native blur.

## Verification for the 2026-07 target

The completed local matrix passed:

- `cargo fmt --all -- --check`
- `cargo metadata --locked --format-version 1`
- `cargo check --locked --workspace --all-targets`
- `cargo clippy --locked --workspace --all-targets --all-features -- --deny warnings`
- `cargo test --locked -p scheduler` (36 tests)
- `cargo test --locked -p bumpyclock-gpui --features test-support` (253 unit and 3 integration tests)
- all GPUI examples, GPUI benches, and GPUI WGPU benches
- macOS platform tests (8 tests)
- isolated Linux no-default, Wayland, X11, and combined checks with warnings denied
- nightly wasm build-std checks for `gpui_platform`, `a11y`, and `view_example`

Rounded native window blur and animated element backdrop-blur cards were visually confirmed on
macOS. The workflow is configured to run native Windows compilation/HLSL validation and repeat the
cross-platform matrix, but this branch has not been pushed and no remote CI result exists yet.
Live Wayland/X11 popup, input-region and compositor blur behavior; Windows
HostBackdrop/device-loss; macOS legacy-CGS blur, Retina/non-Retina resizing and retained/backdrop
Metal behavior; and multi-GPU recovery remain hardware/session-only validation risks. Stable wasm
is not the supported gate because `wasm_thread` requires nightly; CI uses nightly build-std with
atomics enabled.

## Verification for the 2026-08 target

The current macOS worktree passed:

- `cargo fmt --all -- --check`
- `cargo metadata --locked --format-version 1`
- `cargo check --locked --workspace --all-targets`
- `cargo clippy --locked --workspace --all-targets --all-features -- --deny warnings`
- `cargo test --locked -p scheduler` (36 tests)
- `cargo test --locked -p bumpyclock-gpui --features test-support` (335 unit and 3 integration tests)
- `cargo test --locked -p bumpyclock-gpui --features test-support,profiler` (341 unit and 3 integration tests)
- `cargo test --locked -p gpui_wgpu --lib` (43 tests)
- `cargo test --locked -p gpui_macos` (12 tests)
- `cargo test --locked --workspace --all-targets --features test-support`
- `cargo test --locked --workspace --doc`
- `cargo test --locked -p gpui-component` (288 tests; 290 with all workspace features)
- `cargo test --locked -p gpui-component-app` (99 tests)
- `./script/check`
- `./script/test`
- `./script/stage1`
- `bun install --frozen-lockfile && bun run build` in `framework/docs`
- `cargo run --locked -p framework-xtask -- release-check`

This worktree has no immutable commit identity. Exact-source Stage 1 evidence cannot refer to it.
The component story app built and launched from the validated binary. macOS window inventory
reported its 1324 by 856 AppKit window on screen. Native conformance recorded backend-accepted
presentation through the Apple M5 Max Metal hardware adapter. All six native macOS scenarios passed
source-blind validation. The interaction scenario produced 20 valid JSONL records. The external
`pbpaste` clipboard handshake also passed.

Native Windows, X11, Wayland, and browser sessions did not run. Stable WASM remains blocked by
`wasm_thread` 0.3.3. Linux musl checks remain blocked because the required C and C++ compilers are
not installed. Publication plans report documented registry identity, unpublished prerequisite,
and root-patch blockers. Platform support metadata remains conservative.

## Next sync procedure

1. Clone or update Zed in `/tmp/zed`; never overwrite tracked fork files with an upstream tree.
2. Read `BASE` from `fork.toml` and set the new audited `TARGET`; do not duplicate the base SHA in
   procedure text.
3. Inventory `BASE..TARGET` for `crates/gpui*`, `crates/scheduler`, required extracted utilities,
   shaders, examples, and platform manifests. Also compare the target trees semantically so an old
   README marker cannot hide a pre-existing gap.
4. Classify every relevant change as already integrated, fork-modified, missing, competing, or
   excluded. Record exact paths and upstream SHAs before editing.
5. Run the preservation tests first. Import compatible behavior in dependency-ordered, reviewable
   commits; adapt conflicts around the invariants above instead of replacing whole files.
6. Run focused tests after each slice, then the full CI matrix and available native visual smokes.
7. Update `fork.toml`, this file, framework compatibility metadata, and generated documentation only
   after the selective sync and independent review are complete.
