---
title: "Building an Application"
summary: "Use AppShell for identity, startup, settings, windows, menus, and desktop lifecycle."
order: -7
---

# Building an Application

`gpui-component-app` is the experimental, pre-1.0 application layer for GPUI
Component. It replaces repeated host boilerplate while keeping product services
and UI policy in the app.

Use it when building a native GPUI desktop application. Keep using raw
`gpui::Application` APIs when you need platform behavior AppShell does not yet
model.

## Try it

Run the AppShell conformance example:

```bash
cargo run -p app_shell
```

The separate non-publishable `gpui-component-conformance` executable drives
Stage 1 native scenarios and emits the JSONL evidence contract described in
[Testing and Runtime Evidence](runtime-evidence.md).

Or run `cargo run -p gpui-component-story` and select **App Shell**. The story
previews standard menus, Settings/About actions, launch and liveness policies,
runtime errors, and platform capabilities without starting a nested
`Application`.

## What AppShell owns

- compiled application identity and platform paths
- `gpui_component::init`
- ordered startup and reverse shutdown
- typed settings stores and theme preferences
- `Root`-wrapped normal windows and keyed singleton windows
- standard commands, key bindings, and menu projection
- cross-thread dispatch and zero-window liveness
- explicit capability reporting

AppShell does not own domain services, settings-page layout, updater policy,
packaging, sidecars, credentials, or a JavaScript/WebView bridge.

## Add the application crates

Until the application crates are published as stable releases, pin every
GPUI Component crate to one workspace revision. Do not mix GPUI revisions.

```toml
[dependencies]
gpui-component-app = { git = "https://github.com/BumpyClock/gpui-component", rev = "<revision>" }
gpui-component-assets = { git = "https://github.com/BumpyClock/gpui-component", rev = "<same-revision>" }
anyhow = "1"
serde = { version = "1", features = ["derive"] }

[build-dependencies]
gpui-component-manifest = { git = "https://github.com/BumpyClock/gpui-component", rev = "<same-revision>" }

[package.metadata.gpui-app]
app_id = "com.example.my-app"
display_name = "My App"
categories = ["Utility"]
```

Add an app-local `build.rs`:

```rust
fn main() {
    gpui_component_manifest::build::emit_identity()
        .expect("invalid [package.metadata.gpui-app]");
}
```

Identity is generated from the consuming app's package. `Cargo.toml` remains the
version source.

## Standard application

```rust
use gpui_component_app::gpui::*;
use gpui_component_app::prelude::*;
use gpui_component_app::{
    AppMenusExt as _, StandardMenus, WindowManager,
};
use gpui_component_app::ui::{ActiveTheme as _, v_flex};
use serde::{Deserialize, Serialize};

gpui_component_app::include_identity!();

#[derive(Default, Serialize, Deserialize)]
struct Settings {
    show_status: bool,
}

impl AppSettings for Settings {
    const SCHEMA_VERSION: u32 = 1;
}

struct MainView {
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    menu_bar: Entity<gpui_component_app::ui::menu::AppMenuBar>,
}

impl Render for MainView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .when(cfg!(any(target_os = "windows", target_os = "linux")), |view| {
                #[cfg(any(target_os = "windows", target_os = "linux"))]
                {
                    view.child(self.menu_bar.clone())
                }
                #[cfg(target_os = "macos")]
                {
                    view
                }
            })
            .child("My App")
    }
}

fn main() -> Result<(), AppShellError> {
    AppShell::builder(APP_IDENTITY)
        .assets(gpui_component_assets::Assets)
        .settings::<Settings>(StoreKey::PRIMARY)
        .theme(ThemeSource::registry())
        .standard_menus(
            StandardMenus::new()
                .with_theme_menu()
                .on_settings(open_settings)
                .on_about(open_about),
        )
        .start(|_launch, cx| {
            WindowManager::open(
                cx,
                WindowSpec::new("main").title("My App"),
                |_, cx| {
                    cx.new(|cx| MainView {
                        #[cfg(any(target_os = "windows", target_os = "linux"))]
                        menu_bar: cx.new_app_menu_bar(),
                    })
                },
            )?;
            Ok(())
        })
        .run()
}

fn open_settings(_cx: &mut App) -> anyhow::Result<()> {
    // App owns its settings schema and chooses window, sheet, dialog, or route.
    Ok(())
}

fn open_about(_cx: &mut App) -> anyhow::Result<()> {
    // App owns About presentation. Identity data is available through app_info().
    Ok(())
}
```

`start` is the application readiness transaction. It runs after component and
shell services initialize, but before `AppEvent::Started`, queued launch events,
activation, and idle-exit evaluation. It may consume prebuilt app state, create
fallible path-aware services from `cx.app_info()`, install GPUI globals/entities,
register product commands, and open initial windows.

A platform-construction failure is returned as `AppShellError::Platform` before
AppShell enters GPUI; it does not panic or create a proxy. A returned startup
error after platform construction is also fatal and produces
`AppShellError::Startup`; AppShell closes its proxy and runs initialized plugin
shutdown in reverse exactly once.

Orderly native and headless shutdown returns from `AppShellBuilder::run`. First
quit admission closes cross-thread proxy dispatch, invokes quit observers once,
and runs initialized plugin teardown once in reverse order. Repeated quit is a
no-op. GPUI gives all shutdown futures one shared 100 ms completion window;
settings stores claim that same absolute deadline for one bounded best-effort
flush. Launch-time quit, posted/cross-thread quit, zero-window launch,
final-window close, and supported OS quit requests converge on this path.
AppShell does not turn Windows `WM_QUERYENDSESSION` or `WM_ENDSESSION` into an
orderly quit yet. Web and caller-owned embedded lifecycle remain GPUI host
contracts rather than AppShell native-run modes.

The normal-return behavior requires the Stage 1 GPUI lifecycle contract. Until
this workspace pins that GPUI revision, validate it only through the
[documented disposable local override](../learned/gpui-submodule.md).
External resources created during `start` should therefore use RAII ownership;
the transaction does not roll back arbitrary external side effects.

## Standard menus and settings

`StandardMenus` registers only actions that work:

- Settings/Preferences appears only when `on_settings` exists.
- About appears only when `on_about` exists.
- Quit, Edit, Close Window, and platform window actions are shell-owned.
- `app.settings` and `app.about` are stable command IDs shared by menu,
  keybinding, direct action dispatch, and future projections.

Defaults follow host conventions:

| Behavior | macOS | Windows | Linux |
|---|---|---|---|
| Surface | Native global menu | In-window `AppMenuBar` | In-window `AppMenuBar` |
| Menus | App, Edit, optional custom, Window | File, Edit, optional View/custom, Window, Help | File, Edit, optional View/custom, Window, Help |
| Settings | Settings… / `cmd-,` | Settings / `ctrl-,` | Preferences / `ctrl-,` |
| Quit | Quit `<App>` / `cmd-q` | Quit / `ctrl-q` | Quit / `ctrl-q` |
| Close window | `cmd-w` | `ctrl-w` | `ctrl-w` |
| About | App menu | Help menu | Help menu |

macOS installs the menu globally. Windows and Linux apps place
`cx.new_app_menu_bar()` in each window's own title bar or chrome. AppShell keeps
every registered bar synchronized when commands or theme state change.

Use `menus(MenuPlan)` for exact custom ordering. Do not combine
`standard_menus`, raw `menus`, and direct `cx.set_menus` ownership in one app.

## Settings persistence

Register each schema under a stable `StoreKey`. Settings are stored below the
identity-derived config directory with schema envelopes, migrations, validation,
debounced atomic writes, and a bounded best-effort shutdown flush attempt. A
blocked store or elapsed shutdown budget can leave pending changes unwritten.

Settings stores are not secret stores. Backups deliberately create more copies;
use an OS credential service for tokens and passwords.

Calling `.theme(...)` automatically opts into the shell-preferences store for
theme mode and selected theme. Apps without a theme or explicit
`.shell_preferences()` consumer create no shell-preferences file or writer lock.

## Window identity and singletons

`WindowManager::{open, open_singleton, open_raw}` applies manifest `app_id` to
every managed normal window. `WindowSpec::app_id` is the explicit override.
Direct `cx.open_window` calls remain app-owned and receive no such guarantee.

Singleton keys are runtime contracts. Reusing a live or opening key with a
different content type or `RootPolicy` returns a typed error instead of
downcasting or relying on debug assertions.

## Activation and background work

- `InitialActivation::Regular` calls `activate(false)`.
- `InitialActivation::Forced` calls `activate(true)`; use only when foreground
  behavior is required.
- `InitialActivation::Passive` does not activate the app.

Use a `ShellHold` while useful background work keeps a zero-window app alive.
This is not tray support. `examples/app_shell_background` demonstrates only
passive activation, liveness holds, and zero-window exit behavior.

## Error policy

Startup and dynamic registration failures are returned to the caller. Runtime
lifecycle and menu callback failures continue by default and are logged.
Install `.on_error(...)` to route those nonfatal failures to app diagnostics.

`on_launch` remains compatibility sugar for the single transactional `start`
slot. It now runs before `Started` observers and propagates errors. Calling
`start`/`on_launch` more than once or mixing them returns a typed configuration
error before GPUI platform construction.

## Current limitations

| Capability | Status |
|---|---|
| Native tray | Unsupported on macOS, Windows, and Linux |
| URL-scheme registration | Unsupported until packaging registration and launch delivery are both proven |
| Single-instance forwarding | Not implemented |
| Packaging/updater/signing | App-owned; manifest tooling verifies metadata but does not produce installers |
| Typed singleton content access | Not implemented; type/root reuse is guarded |
| Binding contexts and user keymaps | Not implemented |
| Existing React/WebView apps | Require a native UI rewrite for full Linux parity |
| Windows session end | `WM_QUERYENDSESSION` / `WM_ENDSESSION` are unsupported by the orderly lifecycle path |

See the [generated compatibility matrix](../COMPATIBILITY.md),
[Testing and Runtime Evidence](runtime-evidence.md), and the
[Agent Term parity audit](../learned/agent-term-appshell-parity.md) before making
replacement claims.
