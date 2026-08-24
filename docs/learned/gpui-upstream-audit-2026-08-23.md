---
summary: Ranked audit of Zed GPUI changes after Neutron's recorded upstream cursor.
read_when:
  - Importing or evaluating upstream Zed GPUI changes.
  - Updating engine/fork.toml or engine/UPSTREAM.md.
---

# GPUI upstream audit, 2026-08-23

## Scope

This audit compares Neutron commit `7fd145ac68413c288f93b5a65f2040afad8c15e7` with Zed `main`.
Neutron records Zed commit `2c4e44704c37ee87e59ac84e3e17388178b28545` as its audited cursor.
The inspected Zed target is `d9ad6aff67e47de43abb270d22de75dd950f1b48`.
Its tree is `c56c35d41d03998657d77021b897db563f803c02`.
Its commit date is 2026-08-22T20:58:12Z.

The cursor-to-target range has 115 commits that touch `crates/gpui*`.
The broader extracted dependency scope has 129 commits and 143 changed files.
The common GPUI package trees have 180 files with byte differences.
They also have 34 local-only paths and 24 upstream-only paths.
These tree counts measure audit scale, not semantic gaps.

Evidence sources are the official Zed Git repository and the local Neutron source tree.
Patch checks used upstream GPUI hunks with `git apply --check --directory=engine`.
A clean check proves mechanical applicability against current files.
It does not prove semantic safety or complete integration.

## Fork differences that must remain

Neutron keeps retained compositor layers, element backdrop blur, and native rounded blur.
Neutron keeps overlay surfaces separate from anchored popups.
Neutron keeps stable accessibility identifiers and rollback behavior.
Neutron keeps first-presentation evidence and renderer recovery contracts.
Neutron keeps deterministic X11 event-batch delivery.
Neutron requires only `Send` for dedicated scheduler task output.

Upstream now has a separate `gpui_apple` renderer crate.
Neutron keeps its modified Metal renderer in `gpui_macos`.
Upstream uses Rust 1.97.1 and Taffy 0.13.0.
Neutron uses Rust 1.95.0 and Taffy 0.10.1.
Upstream added WebGL shaders, system notifications, springs, gestures, container queries, and profiler journals.
These additions are not corrective imports.

## Post-audit spring decision

The user requested a separate review of native spring animation after the corrective audit.
Commit `8b1497dbd22fb06f5838a7c0b84a1e54fafa71bc` is inside the audited target ancestry.
Neutron adopted it as an explicit feature, not as a P0 or P1 correction.

The engine keeps the upstream physical spring model and retargetable velocity state.
Neutron uses the background executor clock instead of wall time for deterministic tests.
The framework maps its normalized theme tokens to physical spring parameters.
The Switch thumb uses the native spring because it stays mounted and can reverse direction.

Presence transitions retain duration-based spring easing.
Dialog, menu, flyout, and accordion exits need a bounded mount lifetime.
Backdrop blur, native window blur, materials, and retained compositor layers do not use the spring.
Reduced-motion policy combines the engine signal with the framework signal.

## P0 corrective imports

| Change | Class | Local evidence | Required validation |
| --- | --- | --- | --- |
| [Windows DirectWrite and DirectX safety](https://github.com/zed-industries/zed/commit/89e8a4b9ec7e0fa8a49987a3c1dbf29343dc6999) | compatible import | Local code still uses mutable data through `&raw const`, nullable `from_raw_parts`, an unbounded atlas upload, and a mapped staging texture. The upstream patch applies cleanly. | Windows tests, D3D11 WARP text and emoji, device-loss conformance |
| [Element arena nested-draw safety](https://github.com/zed-industries/zed/commit/ec3d887507f272119d9fe146c685f0a941d0e798) | fork-modified adaptation | Local `Chunk::allocate` performs pointer addition before the bounds check. Local arena clears do not track nested draw scopes. | Upstream arena tests, GPUI tests, native Windows Stage 1 |
| [macOS display-link lifetime](https://github.com/zed-industries/zed/commit/96ce8f2a05f8912851e5d20d808fe21f4134bd45) | fork-modified adaptation | Local code uses per-window callback contexts and leaks stopped links with `mem::forget`. | macOS tests, repeated window close, monitor moves, native Metal Stage 1 |
| [macOS pasteboard lifetime](https://github.com/zed-industries/zed/commit/914e1c9873b6b85bfeedc5b58e8270885bdd532e) | fork-modified adaptation | Local `Pasteboard` stores autoreleased objects as raw `id` values. Local reads return borrowed `NSData` slices. | Autorelease-pool test and external clipboard Stage 1 |
| [Wayland Fcitx5 feedback loop](https://github.com/zed-industries/zed/commit/5079b33d657593ffd8a2d2978534988a8b40867e) | compatible adaptation | Local code has no last-cursor cache. It commits unchanged cursor rectangles and can trigger unbounded KWin preedit replay. | Helper tests, Weston Stage 1, manual KWin and Fcitx5 test |
| [X11 close callback borrow panic](https://github.com/zed-industries/zed/commit/d9ad6aff67e47de43abb270d22de75dd950f1b48) | compatible import | Local code reacquires the client-state lease after `window.close()`. The upstream patch applies cleanly. | Native X11 close callback with client-state access |
| [X11 expose repaint](https://github.com/zed-industries/zed/commit/ae99a867d7a24682435bd1821c66b4e172a10768) | fork-modified adaptation | Local code waits for a periodic refresh tick after `Expose`. A fully obscured window can have no active refresh loop. | Xvfb cover test, real-WM uncover test, presentation evidence |
| [WGPU mixed-direction BiDi crash](https://github.com/zed-industries/zed/commit/c214057e086517920b214800725c5d16294ddf0d) | compatible adaptation | Local uses `cosmic-text` 0.19.0 and shapes mixed-direction paragraphs as one line. | WGPU unit tests and mixed RTL/LTR render test |
| [Nested deferred popover crash](https://github.com/zed-industries/zed/commit/5e982c6bdc315a6d2bb68b3edefae00ceedd35f4) | fork-modified adaptation | Local code removes deferred rounds with `mem::take`. Cached subtree ranges can reference shifted entries. | Upstream regression plus accessibility-active variant |

## P1 corrective imports

| Change | Class | Local evidence | Required validation |
| --- | --- | --- | --- |
| [TestWindow raw-handle error](https://github.com/zed-industries/zed/commit/7bddd16a09cf0084cefb3d98468b178343b9f1e2) | compatible import | Local methods still call `unimplemented!()`. They must return `HandleError::NotSupported`. | GPUI regression and framework headless tests |
| [XKB context validation](https://github.com/zed-industries/zed/commit/c43e2d9734800bef0dd216ea9184fdb99bc60625) | compatible adaptation | Both Linux backends accept a wrapper with a null XKB pointer. | Null-context unit test and both Linux startup profiles |
| [Profiler lock-order deadlock](https://github.com/zed-industries/zed/commit/c16d19c94dbc1c62752b736186ab8fd6f1cd25c6) | fork-modified adaptation | Local collectors process upgraded handles while they hold the global spin mutex. | Profiler tests and concurrent thread-exit stress test |
| [Pending key binding before IME](https://github.com/zed-industries/zed/commit/dc1e815e47835095748cbb036541992abd9ac826) | compatible import | The exact upstream patch applies cleanly. | Key-dispatch tests and macOS Japanese IME test |
| [Multi-modifier synthesis](https://github.com/zed-industries/zed/commit/a8cae3bd77f6e1c1dde98bb1dcebba0d254dbd71) | compatible import | Local state tracks only `saw_keystroke`. The exact patch applies cleanly. | Focused modifier regression and keyboard conformance |
| [Inactive Wayland IME cursor](https://github.com/zed-industries/zed/commit/655ed1385b00c01386bfbd9b7edea439eb1eec64) | compatible import | Local forwards cursor bounds from inactive windows. The patch applies cleanly. | Two-window Wayland and Fcitx5 test |
| [Web keyboard defaults](https://github.com/zed-industries/zed/commit/5d447d4223f8455db4811ed332c4ad744bb13e40) | compatible import | Local prevents every non-modifier browser key event. The patch applies cleanly. | Bound and unbound WASM keyboard tests |
| [macOS appearance reentry](https://github.com/zed-industries/zed/commit/a11083f9a79495e9c7ddee0c5782f22d07695c31) | compatible adaptation | Local invokes `handle.update` during a synchronous AppKit callback. | Upstream regression and framework theme tests |
| [Image aspect-ratio preservation](https://github.com/zed-industries/zed/commit/59b2ebf10351b5c0b5cd4403f01ed0460eeec06d) | compatible adaptation | Local image layout overwrites an explicit aspect ratio. | Image layout regression |
| [Rounded `ObjectFit::Cover`](https://github.com/zed-industries/zed/commit/58df5a14ce946fa27b14194d9d2fce8f87b1c331) | fork-modified adaptation | Local clips at image bounds instead of the destination bounds. | Sprite-bound tests and renderer tests |
| [Hover reconciliation](https://github.com/zed-industries/zed/commit/38ca9106c5306ef93e52c35643df015a27f15b72) | fork-modified adaptation | Local hover callbacks change only after pointer events. Layout can move a new element below a stationary pointer. | Hover tests with overlay hit testing |
| [Prompt click isolation](https://github.com/zed-industries/zed/commit/058f01fa93503491a735bfded53e77bfaa276148) | fork-modified adaptation | In-window prompt clicks can reach a popover behind the prompt. | Prompt and mouse-down-out tests |

## Integrated or partial changes

[X11 buffered-event draining](https://github.com/zed-industries/zed/commit/f4178619acd0d47ea1f76a2025c42962c6d6638c) is integrated with a stronger local drain.
Local code drains before loop entry and after every calloop dispatch.
The forced `SetInputFocus` workaround remains and should be removed separately.

[Wayland serial filtering](https://github.com/zed-industries/zed/commit/dc2a339d5d043da448a3f7ddc7c0a85c63864aad) is mostly integrated.
Local code excludes IME serials and handles wrapping order.
It still uses zero when no eligible selection serial exists.
Adopt an optional typed serial and skip unauthorized selection calls.

[Windows inactive popups](https://github.com/zed-industries/zed/commit/826f28eb8ffa661a0d1cdf639104d4c9131e8aa4) is partly integrated.
Local code has the topmost style.
It lacks the focus-aware show, IME, and mouse-activation changes.

[Raw-window-handle unification](https://github.com/zed-industries/zed/commit/f1280b64a4146519d389c7d2f1c4817893d1d1e3) is integrated.
The lockfile resolves one `raw-window-handle` 0.6.2 identity.

## Deferred or excluded series

Defer [Apple renderer extraction](https://github.com/zed-industries/zed/commit/52b2418110ee6c7a67c52398980357b2c15609e9).
It is structural and overlaps the fork renderer.

Defer [renderer resource management](https://github.com/zed-industries/zed/commit/be8c6f9fb356dcd40a7ff06149568753e64ee171).
It overlaps retained layers, backdrop blur, and renderer recovery.

Defer [Wayland demand-driven rendering](https://github.com/zed-industries/zed/commit/eb354c8d504071bdb79110a7a5c9d374c2864113).
It changes frame callbacks and presentation evidence.

Exclude [legacy macOS blur removal](https://github.com/zed-industries/zed/commit/06b6160d46ae8a9074cd367ed64f742b47beca64).
Neutron explicitly keeps the pre-macOS 12 degradation path.

Exclude the Taffy upgrade chain beginning with [91fdd558](https://github.com/zed-industries/zed/commit/91fdd55889ab286b3e0712b44e784cbb852b9c0b).
It is a broad dependency and layout change.

Exclude the [Rust 1.97 upgrade](https://github.com/zed-industries/zed/commit/1271f8b0e8f3278eed5dd3fc12ad4bd30dce2c5d).
The monorepo pins Rust 1.95.0.

Exclude [system notifications](https://github.com/zed-industries/zed/commit/de827bce2ff1cbb40040bea0ef57c8ac56afd726).
Notifications are a migration non-goal.

Exclude [window attention](https://github.com/zed-industries/zed/commit/905e955a702707cd81a2e5bae9b381a7a9c7f614) from the fix batch.
Its Wayland implementation is a silent no-op.

Do not import the [Git-pinned wasm_thread fix](https://github.com/zed-industries/zed/commit/424a68244aa9b8ac9d4766e51b0824d2b2174bd7).
A Git normal dependency conflicts with Neutron publication rules.

## Recommended import batches

1. Import Windows text safety, X11 close safety, and TestWindow raw-handle behavior.
2. Adapt macOS display-link and pasteboard lifetime fixes.
3. Adapt Wayland IME caching, inactive-window filtering, and XKB validation.
4. Adapt arena, deferred draw, X11 expose, WGPU BiDi, and profiler fixes.
5. Import input, web keyboard, image, hover, and prompt behavior fixes.

Keep each upstream SHA in the local commit body.
Add focused regression tests before each behavior change.
Run engine tests before framework checks.
Run each affected native Stage 1 profile.
Move the audited cursor only after all accepted batches pass.

## Evidence limits

This audit did not change product source.
It did not run native Windows, X11, Wayland, or browser sessions.
Upstream runtime claims remain upstream evidence until Neutron reruns them.
Patch applicability was tested in isolation for 115 GPUI-family commits.
Nineteen patches applied mechanically, and 96 conflicted.
Mechanical conflicts often result from fork structure, not invalid fixes.
