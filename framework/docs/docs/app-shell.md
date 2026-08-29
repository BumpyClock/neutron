---
title: "Building an application"
summary: "Declare a DesktopApp, build an AppDeclaration, and run it with AppShell::run."
order: -7
---

# Building an application

`neutron-components-app` is the experimental, pre-1.0 application layer for
Neutron Components. An application is a type that implements `DesktopApp`. That
type returns an opaque `AppDeclaration`. `AppShell::run::<A>()` validates the
declaration, parses launch input, and starts the process.

Use it when you build a native GPUI desktop application. Keep using raw
`gpui::Application` APIs when you need platform behavior AppShell does not yet
model.

The `app_shell` example and the `neutron-story` gallery implement `DesktopApp`.
The `neutron-components-conformance` runner validates native scenario traces
and `story-smoke` JSONL. It is not a public application API. This page and
`framework/crates/app/tests/public_api.rs` describe the current public surface.

## What AppShell owns

- compiled application identity and platform paths
- `neutron_components::init`
- ordered startup and reverse shutdown
- typed settings stores when you declare them
- registry theme, shell preferences, and Appearance, unless you disable them
- `Root`-wrapped declared surfaces
- standard commands, key bindings, and menu projection
- cross-thread dispatch and zero-window liveness
- explicit capability reporting

AppShell does not own domain services, settings-page layout, updater policy,
packaging, sidecars, credentials, or a JavaScript/WebView bridge.

## Add the application crates

Until the application crates are published as stable releases, use one Neutron
workspace checkout. Do not mix engine revisions or add a sibling checkout.

```toml
[dependencies]
neutron-components-app = { path = "framework/crates/app", version = "=0.7.0" }
neutron-components-assets = { path = "framework/crates/assets", version = "=0.7.0" }
anyhow = "1"
serde = { version = "1", features = ["derive"] }

[build-dependencies]
neutron-components-manifest = { path = "framework/crates/app-manifest", version = "=0.7.0" }

[package.metadata.gpui-app]
app_id = "com.example.my-app"
display_name = "My App"
categories = ["Utility"]
```

Add an app-local `build.rs`:

```rust
fn main() {
    neutron_components_manifest::build::emit_identity()
        .expect("invalid [package.metadata.gpui-app]");
}
```

Identity is generated from the consuming app's package. `Cargo.toml` remains the
version source.

## Declare and run

```rust
use neutron_components_app::gpui::*;
use neutron_components_app::prelude::*;
use neutron_components_app::{AppDeclaration, Surface, SurfaceKey};
use neutron_components_app::ui::{ActiveTheme as _, v_flex};

neutron_components_app::include_identity!();

struct MainView;

impl Render for MainView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child("My App")
    }
}

fn build_main(_args: &(), _window: &mut Window, cx: &mut App) -> Entity<MainView> {
    cx.new(|_| MainView)
}

struct MyApp;

impl DesktopApp for MyApp {
    fn declaration() -> AppDeclaration {
        AppDeclaration::new(APP_IDENTITY)
            .primary_surface(
                Surface::new(SurfaceKey::<MainView>::primary(), build_main).title("My App"),
            )
    }
}

fn main() -> Result<(), AppShellError> {
    AppShell::run::<MyApp>()
}
```

`DesktopApp::declaration` is pure. It must not touch the filesystem, GPUI, or
the process environment. `AppShell::run::<A>()` is the one process entry point.
It validates first. A malformed declaration returns
`AppShellError::Declaration` before paths, the platform, or GPUI exist. A
launch-parse failure returns `AppShellError::Launch` equally early.
`LaunchDecision::ExitSuccess` writes optional stdout and returns success with
no platform. After that, a platform-construction failure is
`AppShellError::Platform`. A later startup failure is `AppShellError::Startup`.

The application is a type. The shell never creates a mutable application object.
Mutable state lives in GPUI entities, globals, and explicit handles.

## Declaration defaults

`AppDeclaration::new` installs the conventional desktop foundation:

- registry theme source
- framework-owned shell-preferences store for theme mode and selected theme
- platform Appearance section in the menu bar
- framework About surface and the standard About command
- standard menu bar

It does not invent an application settings store, a Settings surface, or a
Settings command. Declare those yourself when the product has a schema and a
Settings UI. A Settings item that opens nothing is worse than none.

Replace the theme source with `.theme(...)`. Drop theme, shell preferences, and
Appearance with `.without_theme()`. Replace About content with
`.about_surface(...)`. Drop About with `.without_about()`.

AppShell appends `neutron_components_assets::Assets` after every application
asset source. Application sources keep declaration order and win on first hit.
You do not need to register the bundled crate unless you want to name it in
app code.

## Launch

Process facts belong to `ProcessLaunch`. `args` excludes the executable name
and preserves non-UTF-8 values. `cwd` is the working directory when it is
resolvable. Both fields are public.

`AppEvent::Started` is payload-free. There is no `LaunchRequest`.

`LaunchSpec<T>` binds one parser, an optional `before_primary` hook, and an
optional typed primary surface to the same `T`. The parser is a non-capturing
function. It returns `LaunchDecision::Run(value)` or
`LaunchDecision::ExitSuccess`. At most one launch spec is legal.

A primary surface that takes unit arguments can use
`AppDeclaration::primary_surface`. A primary surface whose open arguments are
the launch value must use `LaunchSpec::primary_surface`. Do not declare two
primaries.

```rust
fn parse_launch(process: &ProcessLaunch) -> anyhow::Result<LaunchDecision<()>> {
    if process.args().iter().any(|arg| arg == "--version") {
        return Ok(LaunchDecision::ExitSuccess {
            stdout: Some(format!("{}\n", APP_IDENTITY.version)),
        });
    }
    Ok(LaunchDecision::Run(()))
}

fn before_primary(_value: &(), _cx: &mut App) -> anyhow::Result<()> {
    Ok(())
}

AppDeclaration::new(APP_IDENTITY)
    .launch(LaunchSpec::new(parse_launch).before_primary(before_primary))
    .primary_surface(Surface::new(SurfaceKey::<MainView>::primary(), build_main))
```

An application with no primary surface is a background process.

## Surfaces and windows

A surface is a normal managed window. AppShell wraps its content in
`neutron_components::Root`, numbers and titles it, registers it, and takes a
liveness hold. `SurfaceKey<View, Args>` binds a stable ID to the content type
and the open-argument type.

```rust
Surface::new(SurfaceKey::<MainView>::primary(), build_main)
    .title("My App")
    .size(WindowSize::DisplayFraction(0.8))
    .menu_bar(true)
```

`primary_surface` is the launch surface and is restored on reopen.
`settings_surface` is unit-argument and singleton only. It is what creates the
standard Settings command. `surface` declares an auxiliary window under an
application-chosen ID. Primary and auxiliary surfaces admit
`.multiple()` numbered instances. Settings and About do not. Settings and About
have no in-window menu chrome unless you override `.menu_bar(true)`.

Open a declared surface at runtime with `Shell::open_surface`. Singleton
surfaces focus and reuse a live window. `multiple()` surfaces always create a
numbered instance. The result is `SurfaceOpen::{Created, Reused, InFlight}`.
`SurfaceHandle` gives you the content entity. The window root type is `Root`,
not the content view.

`RawWindow` is the runtime escape from `Root` composition and menu chrome. It
is not declarable. Open it with `Shell::open_raw`. `OverlaySpec` is the
capability-gated overlay escape. Open it with `Shell::open_overlay`.
`WindowRecord` is private.

Direct `cx.open_window` calls remain app-owned. They do not receive declared
identity, `Root` wrapping, or menu chrome.

## Root overlays

`Root` owns sheet, dialog, and notification rendering for a managed surface.
It mounts those layers as a sibling of the content view. Open overlays from
content with `WindowExt` methods `open_sheet`, `open_dialog`, and
`push_notification`. Content must not call `Root::render_*_layer`.

GPUI walks an action from the focused element through its ancestors only. A
focused dialog or sheet does not deliver actions into the content view's
`on_action` handlers. Content-view actions do not leak into the overlay.

## Setup and lifecycle

`SetupModule` is one-time registration after every framework module
initializes. Keys are stable identifiers. `.after(...)` names another
application setup module that must run first. Framework modules always run
before application setup. Teardown runs in exact reverse. `State` is private
to the module. The shell owns it. `SetupContext` exposes `app_info`,
`app_proxy`, and `app`.

```rust
.setup(
    SetupModule::new(SetupKey::new("myapp.services"), init_services)
        .after(SetupKey::new("myapp.paths"))
        .shutdown(teardown_services),
)
```

Lifecycle hooks are non-capturing functions:

- `start` runs once after every module initializes and before
  `before_primary`, the primary surface, and `Started`. It takes no launch
  value. At most one may be declared.
- `on_event` observes every `AppEvent`. Repeatable. A failure is nonfatal.
  Later observers still run.
- `runtime_errors` is the one observer of nonfatal runtime errors. A second
  reporter is a declaration fault.
- `shutdown` runs after `WillExit` and before framework modules tear down in
  reverse. At most one may be declared.

`AppEvent` values are `Started`, `Reopened`, `OpenRequested`,
`LastWindowClosed`, `ShutdownRequested`, and `WillExit`. `OpenRequested` may
arrive before `Started`. The shell queues those events and delivers them after
readiness.

## Commands and menus

Declare commands on the declaration. `Command::app` owns a typed fallible
handler. `Command::window` dispatches to the focused view. A handler failure
is a nonfatal `RuntimeError` against that command id.

```rust
actions!(my_app, [Probe]);

fn probe_handler(_action: &Probe, _cx: &mut App) -> anyhow::Result<()> {
    Ok(())
}

.command(
    Command::app(CommandId::new("my_app.probe"), Probe, probe_handler)
        .label("Probe")
        .binding(CommandBinding::platform("cmd-p", "ctrl-p")),
)
.menu_bar(
    MenuBar::standard().insert(Menu::new(
        MenuKey::new("Tools").expect("valid key"),
        "Tools",
    )),
)
```

`MenuBar::standard` is the default. Hide only optional standard menus `View`
and `Help`. Hiding App/File, Edit, or Window is a declaration fault. Insert
application menus before Window. `MenuBar::custom` takes a full layout.
`MenuBar::none` projects no native or in-window menus. `hide` and `insert`
edit the standard layout only.

Menus are projections of command ids. The same command can appear in several
menus. macOS uses the global native application menu. Windows and Linux attach
an in-window `AppMenuBar` on managed surfaces that allow chrome when the
projection is non-empty. Raw windows do not receive that bar.

Defaults follow host conventions:

| Behavior | macOS | Windows | Linux |
|---|---|---|---|
| Surface | Native global menu | In-window `AppMenuBar` | In-window `AppMenuBar` |
| Menus | App, Edit, optional custom, Window | File, Edit, optional View/custom, Window, Help | File, Edit, optional View/custom, Window, Help |
| Settings | Settings… / `cmd-,` | Settings / `ctrl-,` | Preferences / `ctrl-,` |
| Quit | Quit `<App>` / `cmd-q` | Quit / `ctrl-q` | Quit / `ctrl-q` |
| Close window | `cmd-w` | `ctrl-w` | `ctrl-w` |
| About | App menu | Help menu | Help menu |

Settings appears only when you declare a Settings surface. About appears
unless you call `without_about`. Appearance appears unless you call
`without_theme`. `app.settings` and `app.about` are the stable command ids.

After startup, `Commands` on `&mut App` can `register_command`,
`replace_command`, `register_section`, and `invalidate_menus`. You cannot
replace a framework-owned standard command id.

## Settings

Declare a store under a stable `StoreKey`. The key names the file. The Rust
type name never determines file identity.

`StoreKey` permits ASCII lowercase letters, digits, hyphen, and underscore
only. Uppercase is rejected because the key becomes a filename, and
`Settings.toml` and `settings.toml` are the same file on the default macOS and
Windows filesystems.

```rust
#[derive(Default, Serialize, Deserialize)]
struct Settings {
    show_status: bool,
}

impl AppSettings for Settings {
    const SCHEMA_VERSION: u32 = 1;

    fn validate(&self) -> Result<(), SettingsError> {
        Ok(())
    }
}

AppDeclaration::new(APP_IDENTITY)
    .settings_store::<Settings>(StoreKey::PRIMARY)
    .settings_surface(Surface::new(
        SurfaceKey::<SettingsView>::settings(),
        build_settings,
    ))
```

`AppSettings::validate` returns `SettingsError`. The schema type also owns
`SCHEMA_VERSION`, `migrate`, and `FUTURE_VERSION_POLICY`. The default future
policy is `RefuseToWrite`. The default `migrate` refuses, so a type with no
migrations fails loudly instead of loading defaults over user data.

Settings files are not secret stores. Backups create more copies. Use an OS
credential service for tokens and passwords.

At runtime, `Settings` on `&mut App` reads with `settings`, mutates with
`update_settings`, and flushes with `flush_settings`. Invalid updates roll
back. `settings` panics if the store was never declared.

## Runtime traits

Reach these from any `&mut App`:

- `Shell`: `app_info`, `app_proxy`, `hold`, `request_quit`, `open_surface`,
  `open_raw`, `open_overlay`
- `Commands`: dynamic command and section registration
- `Settings`: typed store access

`AppInfo` and `AppProxy` are `Send + Sync`. Main-thread state is reached
through `Shell`.

## Logging and environment

Set these on `AdvancedHooks`. Defaults are platform paths, inherited
environment, and application-owned logging.

```rust
fn init_logging(_paths: &AppPaths) -> anyhow::Result<()> {
    Ok(())
}

AppDeclaration::new(APP_IDENTITY).advanced(
    AdvancedHooks::new().logging(LoggingPolicy::Configure(init_logging)),
)
```

`LoggingPolicy::Configure` takes a non-capturing `fn(&AppPaths) ->
anyhow::Result<()>`. A failure aborts startup as `AppShellError::Preparation`.
It is not logged and swallowed. The logger that would receive the message is
what failed to start.

`EnvironmentPolicy::LoginShell` repairs `PATH` from the login shell on macOS
and Linux. Select it only from a single-threaded `main`, before any thread is
spawned. Failure to read the login shell is non-fatal. The process keeps the
inherited environment. On Windows the policy is a no-op.

## Activation and liveness

- `InitialActivation::Regular` calls `activate(false)`.
- `InitialActivation::Forced` calls `activate(true)`. Use it only when
  foreground behavior is required. Current Windows and Linux backends ignore
  the force flag.
- `InitialActivation::Passive` does not activate the app.

`ExitPolicy::WhenIdle` exits when no windows and no holds remain.
`ExitPolicy::Explicit` exits only through `request_quit`. Use a `ShellHold`
while useful background work keeps a zero-window app alive. This is not tray
support. Native tray remains unsupported.

## Error policy

Declaration, launch, path, preparation, platform, module-init, and startup
failures return to `main`. Runtime lifecycle and command failures continue by
default and are logged. Install `runtime_errors` to route those nonfatal
failures to app diagnostics. A declared `shutdown` failure is also nonfatal.
Teardown continues.

## Breaking migration

This cut replaces the builder API. There is no compatibility layer.

| Removed | Use instead |
|---|---|
| `AppShell::builder` / `AppShellBuilder` | `DesktopApp` + `AppDeclaration` + `AppShell::run::<A>()` |
| Plugin, phase, and runner assembly | Declaration modules and `AdvancedHooks` |
| `start` / `on_launch` with a launch payload | `start(&mut App)` plus `LaunchSpec` / `ProcessLaunch` |
| `LaunchRequest` and a payload on `AppEvent::Started` | `ProcessLaunch` facts; `Started` is unit |
| `WindowManager` / `WindowSpec` | `Surface` / `SurfaceKey` and `Shell::open_surface` |
| `WindowManager::open_raw` | `RawWindow` and `Shell::open_raw` |
| `WindowRecord` | Private. Use `SurfaceHandle` |
| `StandardMenus` / `MenuPlan` | `MenuBar`, `Menu`, `Command` |
| `SettingsPlugin` / `ThemePlugin` | `settings_store`, `settings_surface`, `theme`, `without_theme` |

`LoggingPolicy::Configure` is now a non-capturing fallible function.
`AppSettings::validate` returns `SettingsError`. `StoreKey` rejects anything
outside lowercase ASCII, digits, hyphen, and underscore.

## Current limitations

| Capability | Status |
|---|---|
| Native tray | Unsupported on macOS, Windows, and Linux |
| URL-scheme registration | Unsupported until packaging registration and launch delivery are both proven |
| Single-instance forwarding | Not implemented |
| Packaging/updater/signing | App-owned. Manifest tooling verifies metadata but does not produce installers |
| Binding contexts and user keymaps | Not implemented |
| Existing React/WebView apps | Require a native UI rewrite for full Linux parity |
| Windows session end | `WM_QUERYENDSESSION` / `WM_ENDSESSION` are unsupported by the orderly lifecycle path |

See the [generated compatibility matrix](../COMPATIBILITY.md) and
[Testing and Runtime Evidence](runtime-evidence.md) before making replacement
claims. Native and headless results retained for the pre-cut source do not
verify this declaration candidate.
