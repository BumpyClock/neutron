---
title: "App Platform Plan"
summary: "Current AppShell architecture, platform contract, evidence gates, and adopter roadmap."
read_when: "starting work on gpui-component-app, app storage/manifest, identity, or an adopter migration"
---

# App Platform Plan — making gpui-component a viable app-building platform

> Status: current architecture plan v3, 2026-07-27.

## Current plan (v3)

The workspace already contains the platform foundation:

- `gpui-component-manifest`: compiled identity and metadata verification
- `gpui-component-storage`: paths, envelopes, atomic/debounced stores, backups
- `gpui-component-app`: lifecycle, proxy, liveness, managed windows, settings,
  commands/menus, theme, capabilities, and headless runner
- `examples/app_shell`: conventional windowed-app conformance
- `examples/app_shell_background`: passive activation, zero-window liveness, and
  orderly exit conformance; it is not a tray example

The current “windowed AppShell and standard desktop controls” milestone extends
this foundation instead of adding a second framework abstraction:

1. transactional `start` after shell initialization and before readiness,
   queued-event drain, activation, and idle evaluation
2. fatal startup errors with one reverse-shutdown path; nonfatal runtime failures
   routed through an app-configurable error reporter
3. one standard command model for native macOS menus and Windows/Linux
   in-window menu bars
4. conventional conditional Settings/About actions and shortcuts, with the app
   retaining ownership of schema and UI presentation
5. honest capability reporting, managed-window identity, safe singleton reuse,
   binding validation, and ordered asset fallback

### Current reference-app reality

- **Agent Term** at audited commit
  `c533566b80014c42e831ff28369f74bf53ee2049` is Tauri 2 + React/WebView.
  Moving it to AppShell requires a native GPUI UI rewrite plus explicit
  replacements for invoke IPC, packaging, updater, sidecar bundling, and platform
  effects. It is a parity benchmark, not a current adopter. See
  [Agent Term AppShell parity audit](agent-term-appshell-parity.md).
- **Andromeda** is a native single-window GPUI app and a candidate first external
  adopter after explicit authorization, clean worktree, and legacy-session-path
  preservation.
- **Agent Limits** is the native tray/multi-window stress case. Real cross-platform
  tray support must land before adoption; a liveness hold alone is not a tray API.

### Standard desktop control contract

| Behavior | macOS | Windows | Linux |
|---|---|---|---|
| Menu surface | Native global application menu | App-owned in-window `AppMenuBar` | App-owned in-window `AppMenuBar` |
| Structure | App, Edit, custom, Window | File, Edit, optional View/custom, Window, Help | File, Edit, optional View/custom, Window, Help |
| Settings | Settings… / `cmd-,` | Settings / `ctrl-,` | Preferences / `ctrl-,` |
| Quit | `cmd-q` | `ctrl-q` | `ctrl-q` |
| Close window | `cmd-w` | `ctrl-w` | `ctrl-w` |

Optional Settings/About items, commands, and chords do not exist without a
callback. Raw `MenuPlan` stays available for exact custom ordering. Direct
`set_menus`, raw `MenuPlan`, and `StandardMenus` are separate ownership modes.

### Capability boundary

Native tray and URL-scheme registration report unsupported on every platform
until AppShell provides and verifies the complete behavior. URL delivery on one
platform is not registration. Packaging, updater, signing, single-instance,
sidecar bundling, credential storage, and a React/WebView bridge remain separate
milestones.

### Evidence and adoption order

1. Unit/headless contracts plus native macOS launch smoke in this workspace.
2. Windows/Linux compile gates and native smoke where display-capable runners
   exist; otherwise use the explicit `compiled-not-natively-validated` label and
   documented manual commands.
3. First authorized clean native adopter; Andromeda is a candidate.
4. Real tray milestone, then authorized Agent Limits adoption.
5. Agent Term only as an explicit native-UI product project, using its parity
   audit as release gate.

## Historical v2 record

The remainder records the 2026-07-16 proposal that produced the initial crates.
It is retained for rationale, not current reference-app facts, API signatures, or
adoption order. Where it conflicts with v3 above, v3 wins.

## 1. Problem statement (verified, not aspirational)

Three real apps build on gpui-component today. Each re-implements the same app shell:

| | agent-term | ansible | Andromeda |
|---|---|---|---|
| Shape | multi-window, document-ish | tray-first overlay, **no main window** | single window |
| Bootstrap | 180-LOC `run()` | 548-LOC `app.rs` | 26-LOC `run()` |
| Identity | Cargo bundle/deb/rpm/wix metadata + .desktop + About dialog | bundle metadata + Info.plist + entitlements + MSIX/Inno/Flatpak + PACKAGING.md | none (gap) |
| Settings | TOML via `dirs`, dual configs, `~/.agent-term` root | `config.rs` — dirs + atomic write + schema-version envelope, tested (best impl) | none (dead Settings UI) |
| Window/layout persistence | 766-LOC layout crate (debounced JSON, backup rotation); no bounds | none | none |
| Theme | hardcoded Rust palettes (opts out of JSON registry) | bundled-theme sync + watch_dir + settings glue | watch_dir + menu rebuild |
| Extra | Tauri updater (configured key is nonempty; authenticity unverified) | tray + tokio bridge, permission gate, singleton settings window | — |

Cross-cutting problems:

1. **Version drift** — the three apps pin *three different revs* of gpui AND of gpui-component.
2. **Identity scattered** — bundle id/name/version live in 5–8 places per app; nothing derives from one declaration.
3. **Same glue ×3** — bootstrap, App/Edit menu wiring, theme-settings glue, asset chaining.
4. **Nobody has window-bounds persistence**; layout persistence solved once (agent-term), settings solved well once (ansible).
5. gpui fork has unused app primitives the library never wraps — but some are **stubs**:
   review verified URL-scheme registration is unimplemented on Windows
   ([`BumpyClock/gpui@67c20f3ae1046aa873591ff4b44953b53df37bc4`](https://github.com/BumpyClock/gpui/blob/67c20f3ae1046aa873591ff4b44953b53df37bc4/crates/gpui_windows/src/platform.rs#L1078)) and Linux
   ([`gpui_linux/src/linux/platform.rs:723`](https://github.com/BumpyClock/gpui/blob/67c20f3ae1046aa873591ff4b44953b53df37bc4/crates/gpui_linux/src/linux/platform.rs#L723)), and X11 overlay click-through is a silent no-op
   ([`gpui_linux/src/linux/x11/window.rs:1620`](https://github.com/BumpyClock/gpui/blob/67c20f3ae1046aa873591ff4b44953b53df37bc4/crates/gpui_linux/src/linux/x11/window.rs#L1620)). "Exists in the fork" ≠ "works on 3 OSes".

## 2. Decisions

**D1 — Build the platform layer in the gpui-component workspace, not a separate repo.**
A separate repo adds a fourth independently-pinned link and worsens drift. In-workspace,
the platform inherits the workspace's single gpui rev by construction and releases in
lockstep with gpui-component.

**D2 — Three new crates (revised from two).**
- `crates/app-manifest` → **`gpui-component-manifest`** — **Phase 1, not Phase 4**
  (blocker fix, see D4): no-gpui identity schema, parsing, validation,
  target-version derivation, and build.rs codegen helper. The packaging CLI later
  reuses this same library.
- `crates/app-storage` → **`gpui-component-storage`** — foundation, no gpui dependency:
  path resolution, atomic write, schema-version envelope, debounced store with backup
  rotation. Seeded from ansible's `config.rs` and agent-term's `DebouncedStorage`
  **plus a written portability/concurrency contract** (§4a) — "copy verbatim" is the
  seed, not the spec.
- `crates/app` → **`gpui-component-app`** — the `AppShell` builder over an internal
  (sealed) plugin/phase mechanism. Services are **always-compiled modules activated at
  runtime by builder calls**; Cargo features are reserved for heavy native deps only
  (`tray`, maybe `file-logging`, `theme-watch`). Rationale: features are unioned across
  the dep graph and additive — a default-on feature matrix for cheap modules is
  cosmetic and leaks transitively. Tray may become its own crate
  (`gpui-component-tray`) if GTK/AppIndicator deps hurt build times — that's a
  legitimate crate boundary; "tray + tokio" is not.

**D3 — Naming (locked).**
- **`AppShell`** — the builder/facade. Pairs with `WindowShell` (per-window chrome vs
  app lifecycle). Kept thin: builder methods install internal plugins; explicit phase
  order is enforced centrally, not by builder-call order.
- **`AppInfo`** — immutable identity/paths/capabilities. `Clone + Send + Sync`.
- **`AppProxy`** — cross-thread dispatch only (bounded command sender that wakes the
  main loop). `Clone + Send + Sync`; dispatch returns `AppClosed` after shutdown
  begins. **Lives in core** (not the tray feature) — serves tray, hotkeys, watchers,
  audio callbacks, updater events alike.
- Main-thread shell state is a **GPUI global** accessed via an extension trait
  (`AppShellExt` on `gpui::App`: `cx.app_info()`, `cx.app_proxy()`, `cx.windows()`,
  `cx.settings::<T>(key)`) — no second TypeId DI container beside GPUI's own global
  store, and no `AppHandle` type that mixes thread affinities. Compile-time
  `assert_impl_all!`/`assert_not_impl_any!` for the auto-trait contracts.
- `configure_application(FnOnce(Application) -> Result<Application>)` (not
  "configure_platform" — GPUI's platform already exists by then).
- `PathLayout::{PlatformDefault, SingleRoot}` (not "Xdg" — wrong term on macOS/Windows).
- `ThemeSource::{Bundled, Custom}` (avoid collision with existing `theme/schema.rs`
  `ThemeConfig`).

**D4 — Identity single-source: `[package.metadata.gpui-app]` + app-local build.rs.**
⚠ v1 had a hard blocker both reviews caught: a dependency's build.rs sees *its own*
`CARGO_MANIFEST_DIR`/`OUT_DIR`, never the consuming app's. Corrected design (the Tauri
`tauri-build` pattern):

```toml
[build-dependencies]
gpui-component-manifest = { ... }
```
```rust
// app's own build.rs (2 lines)
fn main() { gpui_component_manifest::build::emit_identity().expect("invalid [package.metadata.gpui-app]"); }
```
`include_identity!()` then includes codegen from the **app's** `OUT_DIR` (with
`rerun-if-changed`). `version` is never declared in the metadata table — canonical
SemVer is `CARGO_PKG_VERSION`; packaging versions (CFBundle 3-int, MSIX 4-part) are
*derived artifacts* with deterministic per-target derivation + validation, plus an
optional CI build number. Schema separates: stable app ID, stable data namespace
(never derived from mutable display name), display name, binary name, org/publisher,
url schemes/file associations, categories, entitlements/usage strings, legacy ID/path
aliases. Test fixture: an actual *downstream* workspace build (an in-repo example can
mask the manifest-boundary bug).

**D5 — Extraction over invention** (unchanged). Core v1 = code already running in ≥2
apps or 1 app + worse hand-rolls elsewhere. Labeled exceptions stay opt-in/thin.

**D6 — Versioning: one *resolved* gpui, not "one dependency".**
v1's "apps may only depend on gpui-component-app" reverses layering — reusable UI
crates inside an app workspace legitimately depend on `gpui-component`/`gpui` directly.
Corrected invariant:
- app binary/shell crate → `gpui-component-app` (re-exports as ergonomic convenience);
- reusable UI crates → `gpui-component`; direct `gpui` where layering/macros need it
  (audit gpui proc-macros for generated `::gpui` paths before relying on re-exports);
- each app repo centralizes versions in `[workspace.dependencies]`;
- **CI lint on `cargo metadata`/`cargo tree -d`**: reject >1 resolved package
  ID/source/rev for `gpui`, `gpui_platform`, `gpui-component` — catches transitive
  duplicates a source-grep never sees.
- gpui fork gets annotated tags; `/update-gpui` is the only bump point and updates the
  shared `[workspace.package] version` (note: workspace currently has none — add it)
  + COMPATIBILITY.md.
- **Release-candidate CI builds all three app repos** against the proposed platform
  rev — an automated downstream gate, stronger than a matrix doc + policy.

**D7 — Historical updater claim corrected.**
The audited current Agent Term configuration contains a nonempty updater public
key. This review did not verify provenance or signatures. Keep the existing
updater until a replacement proves download, verification, install, restart, and
failure behavior; do not disable it based on the former “zeroed key” claim.

**D8 — wasm out of scope for v1** (unchanged). Storage goes behind a small backend
trait so a shim is possible later; nothing wasm is built now.

**D9 — Capability honesty over parity theater.** Core exposes runtime
`PlatformCapabilities { overlay_surface, tray, dock_menu, credentials, url_schemes,
precise_window_positioning, … }` and fallible APIs return typed `Unsupported` — never
silent no-ops (the fork has silent stubs today, §1.5). Capabilities are runtime, not
`cfg(target_os)` — tray depends on the desktop session; overlay differs X11 vs
Wayland. Apps can inspect capabilities before committing to a zero-window shape.
**URL-scheme registration moves out of core** until registration + delivery work
end-to-end on all three OSes.

## 3. The `AppShell` API (target shape, v2)

Happy-path `main.rs` stays ~20 lines (the `windows_subsystem` attr must stay in app
code — a library can't set a crate-level attribute; the Phase-4 scaffold templates it):

```rust
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use gpui_component_app::prelude::*;

gpui_component_app::include_identity!(); // codegen from app's build.rs (D4)

fn main() -> anyhow::Result<()> {
    AppShell::builder(APP_IDENTITY)
        .settings::<Settings>(StoreKey::PRIMARY)
        .theme(ThemeSource::bundled(THEMES))
        .menus(MenuPlan::standard().with_theme_menu())
        .on_launch(|cx| {                     // sugar over on_event(Started)
            cx.windows().open(WindowSpec::new(WindowKey::new("main")).title("MyApp"), MainView::new)?;
            Ok(())
        })
        .run()
}
```

Core contracts (each traces to a review finding):

- **Lifecycle is an event stream, not a launch callback.** `on_launch`/`on_reopen`
  sugar remains, but the real model is:
  ```rust
  #[non_exhaustive]
  pub enum AppEvent {
      Started(LaunchRequest),           // args, cwd, urls/files, launch source
      Activated, Reopened,
      OpenRequested(OpenRequest),       // urls/files — may arrive BEFORE ready
      LastWindowClosed,
      Suspend, Resume,                  // reserved seams; may report Unsupported
      ShutdownRequested(ShutdownReason),
      WillExit,
  }
  ```
  The shell registers raw platform listeners (`on_open_urls`, `on_reopen`) immediately
  after platform construction, **queues events until services are initialized**, then
  delivers `Started` + drained queue (Electron/GApplication/Zed pattern). This
  reserves single-instance `SecondInstance` delivery and deep links without a breaking
  redesign later.
- **Thread affinity is explicit** (D3): `AppInfo`/`AppProxy` are `Send+Sync`;
  main-thread state is a GPUI global reached via `AppShellExt`. Callbacks receive
  `&mut gpui::App` raw — no context wrapper, ever.
- **Pre-platform preparation is typed state, not just a hook.** ansible's
  CoreAudio-before-GPUI constraint is expressed in `main()`:
  ```rust
  let audio = AudioBootstrap::prepare()?;   // no GPUI exists yet
  AppShell::builder(APP_IDENTITY).state(audio)…
  ```
  `before_platform(f)` stays for stateless global side effects. Contract: main thread,
  no GPUI, no windows; failure aborts cleanly before the event loop starts.
- **Liveness is hold/release, not a static quit mode.**
  `let hold = cx.shell().hold("tray")` — windows, tray, background services hold
  leases; last hold dropped with no eligible windows → shell may exit (GApplication
  model). Tray acquires its hold only **after successful creation**; tray failure
  releases it and triggers a configured fallback (open window / revert to
  window-lifetime / exit with surfaced error). Kills the unkillable-invisible-process
  state. Separately configured: initial activation (`InitialActivation::Passive` for
  tray-first — **no unconditional `activate(true)`**), macOS dock policy, quit policy.
- **Persistence guarantee is honest**: settings persist continuously within a
  configured debounce window; orderly shell-mediated shutdown makes one bounded,
  best-effort final flush pass and reports failure (GPUI quit observers get ~100ms —
  verified in fork `app.rs`). A blocked store or abrupt termination may lose pending
  changes. All platform-owned quit actions route through one `request_quit()` path.
  Tests cover every *normal* quit path (menu quit, programmatic,
  last-window-close, tray quit, failed startup after changes, worker shutdown).
- **Windows**: `WindowKey` stable non-localized identity (titles are not persistence
  keys); registry is an app-scoped GPUI global (not a process `OnceLock`); singleton
  state is `Closed | Opening | Open` (prevents async double-create);
  `RootPolicy::{ComponentRoot, Raw}` (overlays aren't the only legal non-Root case);
  `open` returns `OpenedWindow<V> { window: WindowHandle<Root>, content: Entity<V> }`
  (auto-Root-wrap changes the handle type — make that explicit); pre-show option
  customization + post-native-window hook (agent-term's objc2 titlebar tweak gets a
  sanctioned seam). Zero open windows stays a first-class state.
- **Commands before menus**: a command registry (stable ID, GPUI action, scope,
  enabled/checked, localized label, default keybinding, menu placements) is the
  canonical vocabulary; native menu, tray menu, dock menu, and future keymap files are
  *projections* of it. Theme service contributes theme commands; window manager
  contributes Move-to-Window; input contributes Edit. Menu rebuild reacts to theme
  registry AND window registry AND enabled/checked state. Keybinding precedence
  defined now (component < shell < app < user overrides < explicit disable) even
  though the user keymap file ships later.
- **Process-global policies are explicit, never silent defaults**:
  - logging: library defaults to `LoggingPolicy::External` (a lib must not grab the
    process-global logger — conflicts with tests/tracing/agent-term's sink); helpers
    provide log-dir + rotation; the *scaffold* opts new apps into file logging.
  - `EnvironmentPolicy::{Inherit (default), LoginShell, Custom}` — fix_path_env is
    policy, not default (only 1 of 3 apps needs it).
  - locale: component lib initializes only its own rust-i18n strings; app localization
    is a provider hook, not an imposed framework.
- **Testability**: `AppShell::builder(...).runner(PlatformRunner::native())` with a
  headless runner (`gpui_platform::headless()`) for bootstrap-order/lifecycle tests.
  Startup-failure policy is explicit per service (required vs degradable; tray →
  fallback, theme watcher → static themes, file log → warn, `on_launch` error →
  defined behavior). Public API errors are a stable `AppShellError`, not `anyhow`
  through library callbacks.
- **Secrets rule**: settings stores + rotated backups are never for secrets (rotation
  multiplies copies). Credential storage goes through the fork's keychain primitives
  behind a thin abstraction (later, capability-gated).
- **Assets**: explicit `.assets(...)` builder input; namespaced mounts with defined
  collision precedence — no silent last-writer-wins.

### 4a. Settings & storage contracts (was underspecified in v1)

Settings API (per store; **named stores from day one** — agent-term already has
settings + MCP config + layout, a singleton type-keyed service gets outgrown
immediately):

```rust
SettingsPlugin::<Settings>::new(StoreKey::PRIMARY)
    .current_version(3)
    .migrate(1, 2, migrate_1_to_2)     // registered chain, not a raw-TOML free-for-all
    .migrate(2, 3, migrate_2_to_3)
    .validate(validate)                 // can reject before becoming current
    .future_version_policy(FutureVersionPolicy::RefuseToWrite)
// access: cx.settings::<Settings>(StoreKey::PRIMARY) / cx.update_settings(key, |s, cx| …)?
// change observation + save-error surfacing to settings UI defined.
```

Four load outcomes, distinguished (a **newer** schema is NOT corruption): older →
migrate; current → deserialize+validate; **newer → preserve untouched, return
`UnsupportedFutureVersion`** (a downgrade must never archive-and-overwrite newer
data); malformed → archive `.bak.vN` per explicit recovery policy.

Theme/shell state does **not** live in app settings types (v1's sketch was
unimplementable — the trait exposed no fields): a small platform-owned
`ShellPreferences` store holds theme mode/selected theme/locale; apps may opt into
embedding via an adapter instead.

Storage engine spec (beyond the seeded code): unique temp names in target dir;
Windows replace semantics; fsync expectations stated (atomic vs crash-durable);
per-store writer lock or generation/CAS with conflict error (two instances exist
until single-instance ships); preserve last committed file until replacement
succeeds; rotate backups only after identifying a valid committed generation; worker
I/O errors propagate to the app (Drop is best-effort only — explicit
`flush()/shutdown()` return results); **process-level** concurrency tests, not
two-objects-one-process tests; stable storage namespace distinct from display name;
legacy path migration aliases (agent-term's `~/.agent-term`).

## 4. Service tiers

| Service | Tier | Source |
|---|---|---|
| Paths (`PathLayout`), atomic write, envelope, `DebouncedStore` + backups | Core (storage crate) | ansible config.rs + agent-term storage, seeded + §4a contract |
| Identity codegen + dirs + About data | Core (manifest crate + shell) | D4 |
| Typed settings (named stores, migration chain, future-version policy) | Core | §4a |
| `ShellPreferences` (theme mode, locale) | Core | replaces v1's impossible theme↔settings coupling |
| Lifecycle events + queue-until-ready | Core | reserved seams for single-instance/deep links |
| `AppProxy` off-thread→main bridge | **Core** (moved from tray) | serves tray/hotkeys/watchers/audio |
| Liveness holds + activation policy | Core | replaces static QuitMode-only model |
| Window manager (keys, registry-as-global, singleton state machine, overlay, RootPolicy) | Core | agent-term + ansible, promoted + hardened |
| Command registry → menus/tray/dock projections | Core | supersedes v1's menu-centric MenuPlan coupling |
| Theme glue (mode persistence, watch_dir, bundled sync) | Core, opt-out `ThemeSource::custom` | ansible + Andromeda |
| Asset chaining (namespaced) | Core | all 3 apps |
| Logging helpers | Core helpers, **policy default External** | review: process-global logger is app's call |
| Capabilities matrix + typed `Unsupported` | Core | D9 |
| Tray + dock policy | Opt-in feature/crate; uses core `AppProxy`; **no bundled tokio** (app supplies handle) | ansible |
| Window bounds persistence | Opt-in, net-new; persists logical size + scale + monitor id + maximized/fullscreen + position-unavailable (Wayland) | nobody has it |
| Permission gate | App code + `.state()` preflight + gate-window-in-launch pattern | ansible |
| Updater | Agent-owned Tauri updater; replacement later and parity-gated (D7 correction) | — |
| Layout/session schema | App code on `DebouncedStore` | agent-term |
| URL schemes / single-instance / CLI / crash reporting | **Not built**; lifecycle events + capability slots reserved | fork stubs incomplete (§1.5) |

## 5. Roadmap

**Phase 0 — foundations**
1. CI matrix (macOS/Windows/Linux): build + **launch** native smoke apps (compile-green
   proves little for platform-divergent code); feature-matrix builds (none/default/tray/all).
2. `gpui-component-storage` with the §4a contract + process-level concurrency tests.
3. `gpui-component-manifest` + downstream-workspace test fixture (D4 blocker fix).
4. Tag discipline; add missing `[workspace.package] version`; `/update-gpui` owns bumps.
5. agent-term: retain its updater until an independently verified replacement exists (D7).

**Phase 1 — `gpui-component-app` MVP**
Identity include, `AppShell` phases/plugin core (sealed trait until all three
migrations exercised it), lifecycle events + queueing, `AppInfo`/`AppProxy`/globals
split (+ auto-trait compile assertions), window manager, settings plugin,
`ShellPreferences`, command registry + standard menus, theme plugin, capabilities,
headless runner. Two in-repo conformance examples: `examples/app_shell` (single-window
happy path) and **`examples/app_shell_background`** (zero windows, passive
activation, and liveness only — it does not prove native tray behavior). Phase-1
`doctor` command: parse normalized manifest and
*verify* the apps' existing Info.plist/.desktop/wix/MSIX/Inno/Flatpak/bundle metadata
against it (identity converges before the generator exists, instead of adding an
eighth copy).

**Phase 2 — migrate the apps** (each step a shippable PR deleting real code)
1. **Andromeda** — cheapest end-to-end proof of plumbing (~150 LOC deleted, gains
   identity + working settings).
2. **ansible immediately after** (not later) — the architectural validation: storage
   re-parenting (~230), singleton controller (~200), theme (~180), tray+proxy (~130),
   builder + typed preflight + holds (~400). Gate: the tray conformance example's
   behaviors all hold in the real app.
3. **agent-term** — layout crate onto `DebouncedStore` (~300), window registry (~122),
   settings incl. second named store for MCP config (~110), command/menu blocks (~70),
   builder (~140). Keeps palettes (`ThemeSource::custom`), objc2 titlebar (post-native
   hook), frozen updater.

**Phase 3 — opt-ins with demand**: bounds persistence, user keymap file (over the
already-stable command IDs), first-run/onboarding hook, credential-store abstraction.

**Phase 4 — packaging + scaffold**
`cargo gpui-app` generates *inputs* for existing bundlers from the same manifest
library; acceptance = ansible's PACKAGING.md verification commands + deterministic
`generate --check`. `create-gpui-app` scaffold (owns `windows_subsystem`, build.rs,
CI workflow, default logging policy).

## 6. Stabilization gates (API is not 1.0 until all hold)

1. Downstream fixture proves identity codegen reads the *app's* package (incl.
   workspace-inherited version).
2. Early URLs/files are buffered and delivered post-init.
3. `AppInfo`/`AppProxy`/main-thread state have distinct, compile-tested auto-trait
   contracts.
4. Storage spec covers writer conflicts, newer-schema behavior, error propagation,
   orderly-vs-abrupt termination.
5. Tray-first conformance app starts with zero windows, no unconditional activation,
   tested tray-failure fallback.
6. Shell runs through an injected/headless runner.
7. CI launches native smoke apps on 3 OSes and checks min/max feature sets.
8. Release-candidate CI builds all three real app repos against one resolved gpui stack.
9. Stable command IDs exist before menus/keybindings are generalized.
10. Agent Term keeps its existing updater until replacement signature/install
    behavior is independently verified.

## 7. Known risks

- **Fork stubs**: URL registration (Win/Linux) and X11 overlay click-through are
  incomplete in the fork — capability-gate, don't wrap (D9). Verify overlay surface +
  `QuitMode`-with-zero-windows on Win/Linux before ansible migrates.
- **Linux tray (Wayland/GNOME)** unreliable; GTK-loop coexistence with GPUI needs a
  *runtime* integration test, not a compile check.
- **App→platform tag drift** reduced, not eliminated — mitigated by the downstream RC
  CI gate (D6), which is automation, not policy.
- **Scope creep of AppShell into a monolith** — both reviews' top long-term worry.
  Guardrails: sealed plugin trait, explicit phases, commands-not-menus, policies over
  silent defaults, and the D5 extraction rule.
- **Storage generality** — core owns bytes-on-disk + debounce + backups; domain
  schemas (LayoutSnapshot) stay app-side.
