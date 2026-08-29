//! Living reference documentation for the resolved `neutron-components-app`
//! public interface: `DesktopApp`, `AppDeclaration`, `AppShell::run`, and the
//! runtime seams a declared application reaches through `&mut App`.
//!
//! This page does not simulate a second application. The outer
//! `neutron-story` process *is* the real `AppShell`, declared once by
//! `StoryApp` (see the binary's `app` module) and run through the one real
//! `AppShell::run::<StoryApp>()` call in `main`. This gallery is that
//! declaration's primary surface, so every fact below describes the shell
//! actually hosting this window — there is nothing here to launch, and no
//! local key binding or standard action to intercept.

use gpui::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, Styled as _, Window, div, prelude::FluentBuilder as _,
    px,
};
use neutron_components::{ActiveTheme as _, h_flex, label::Label, text::TextView, v_flex};
use neutron_components_app::{Capability, PlatformCapabilities};

use crate::section;

/// No story-local key bindings: the real `OpenSettings`/`About` shortcuts and
/// standard menu belong to this process's own `AppShell` declaration, not to
/// this gallery page. Kept only so `stories::init` has a stable call site.
pub fn init(_cx: &mut App) {}

/// Title of the topic whose card also renders the live capability grid.
const PLATFORM_CAPABILITIES_TITLE: &str = "Platform capabilities";

/// One documented seam of the resolved `AppDeclaration`/`AppShell` public
/// interface, rendered as one story section.
///
/// Pure data — no GPUI — so the canonical topic set and its order are
/// unit-tested directly (see `tests` below) instead of only being checked by
/// eyeballing the rendered gallery page. `body` is illustrative: it is
/// written against real crate types so it stays literally correct, but it is
/// displayed text, not compiled code — the actual declaration this process
/// runs lives in the `neutron-story` binary's own `app`/`launch`/`setup`/
/// `commands` modules, not in this story crate.
struct ReferenceTopic {
    /// Card title.
    title: &'static str,
    /// Card sub-title: the one-line summary of the seam.
    summary: &'static str,
    /// Markdown body: prose, fenced code, and tables as the topic needs.
    body: &'static str,
}

const REFERENCE_TOPICS: &[ReferenceTopic] = &[
    ReferenceTopic {
        title: "Entry point",
        summary: "One declaration type, run through the real shell.",
        body: r#"`DesktopApp` is a type, not a value: `AppShell` never creates or retains a mutable application object. `AppDeclaration` is opaque and non-generic — every typed builder call below erases into an ordered internal list, so its shape never depends on how many surfaces, commands, or setup modules an app declares.

```rust
struct MyApp;

impl DesktopApp for MyApp {
    fn declaration() -> AppDeclaration {
        AppDeclaration::new(identity())
            .primary_surface(Surface::new(SurfaceKey::primary(), MainWindow::build))
    }
}

fn main() -> Result<(), AppShellError> {
    AppShell::run::<MyApp>()
}
```

This gallery is not a simulation of that pattern: it *is* one instance of it. You are reading the primary surface declared by `neutron-story`'s own `StoryApp`, run through the one real `AppShell::run` call in its `main`."#,
    },
    ReferenceTopic {
        title: "Convention defaults",
        summary: "Every piece of the desktop convention can be replaced or dropped explicitly.",
        body: r#"`AppDeclaration::new` starts every app from one desktop convention. Nothing about it is silently assumed — each piece is replaced or dropped by its own explicit call:

| Feature | Convention default | Explicit override |
|---|---|---|
| Theme | Registry source, persisted `ShellPreferences`, platform Appearance section | `.theme(source)` / `.without_theme()` |
| About | Framework surface, standard `About` command | `.about_surface(surface)` / `.without_about()` |
| Settings | None — no surface, no command, no shortcut | `.settings_surface(surface)` |
| Exit policy | `ExitPolicy::WhenIdle` | `.exit_policy(policy)` |
| Initial activation | `InitialActivation::Regular` | `.initial_activation(activation)` |

A Settings command exists *because* a Settings surface was declared: the framework cannot invent an application's settings schema or its UI, and a menu item that opens nothing would be worse than none. This gallery's own outer declaration overrides three of the five rows above — its Settings surface is real, and `InitialActivation::Forced` replaces the convention default."#,
    },
    ReferenceTopic {
        title: "Surfaces",
        summary: "Typed primary/Settings/About/auxiliary windows, plus two runtime escapes.",
        body: r#"A surface is a normal managed window: `AppShell` wraps its content in `neutron_components::Root`, numbers and titles it, and takes a liveness hold. `SurfaceKey<View, Args>` binds a stable ID to the content and argument types, so a declared surface can never be opened with the wrong content.

```rust
AppDeclaration::new(identity())
    .primary_surface(Surface::new(SurfaceKey::primary(), MainWindow::build))
    .settings_surface(Surface::new(SurfaceKey::settings(), SettingsWindow::build))
    .surface(Surface::new(SurfaceKey::new("logs"), LogsWindow::build))
```

| Role | Key | Cardinality |
|---|---|---|
| Primary | `SurfaceKey::primary()` | One; restored on reopen |
| Settings | `SurfaceKey::settings()` | One; activates `OpenSettings` |
| About | `SurfaceKey::about()` | One; activates `About` |
| Auxiliary | `SurfaceKey::new(id)` | Singleton, or `.multiple()` |

`RawWindow<View, Args>` and a capability-gated overlay are the runtime escapes from that composition — opened explicitly at runtime, never declared:

```rust
let raw = RawWindow::<LogPanel>::new("log.panel", LogPanel::build);
cx.open_raw(&raw, &())?;

cx.open_overlay(OverlaySpec::new("hud", 320.0, 96.0), |window, cx| {
    cx.new(|cx| Hud::new(window, cx))
})?;
```"#,
    },
    ReferenceTopic {
        title: "Launch",
        summary: "A typed parser, an optional pre-primary hook, and the primary's argument type — tied together at compile time.",
        body: r#"`LaunchSpec<T>` ties the parser, the optional `before_primary` hook, and the typed primary surface's argument type to one `T` at compile time.

```rust
fn parse(process: &ProcessLaunch) -> anyhow::Result<LaunchDecision<OpenPath>> {
    match process.args() {
        [path] => Ok(LaunchDecision::Run(OpenPath::from(path))),
        [] => Ok(LaunchDecision::Run(OpenPath::default())),
        _ => Ok(LaunchDecision::ExitSuccess { stdout: Some(USAGE.into()) }),
    }
}

AppDeclaration::new(identity()).launch(
    LaunchSpec::new(parse)
        .before_primary(|launch, cx| restore_recent_files(launch, cx))
        .primary_surface(Surface::new(SurfaceKey::primary(), MainWindow::build)),
)
```

`ProcessLaunch` carries the raw process facts (`args`, `cwd`); the parser is a non-capturing `fn`, run once before paths, the platform, or GPUI exist, so an `ExitSuccess` decision (`--help`, `--version`) costs nothing. `before_primary` runs after the common `start` hook and before the primary surface opens — this gallery's own launch spec uses exactly that hook to detect `--fail-start`/`--asset-smoke` and request a quit before any window appears."#,
    },
    ReferenceTopic {
        title: "Setup modules",
        summary: "Keyed, dependency-ordered state the shell owns for the process's whole lifetime.",
        body: r#"`SetupModule<State>` registers state the shell owns for the process's whole lifetime. Every declared module initializes after every framework module, in an order resolved from `.after(..)` dependencies, and tears down in exact reverse — regardless of where in the declaration `.setup(..)` was called.

```rust
const WATCHER: SetupKey = SetupKey::new("theme_watcher");
const INDEX: SetupKey = SetupKey::new("search_index");

AppDeclaration::new(identity())
    .setup(SetupModule::new(WATCHER, init_watcher).shutdown(teardown_watcher))
    .setup(
        SetupModule::<Index>::new(INDEX, init_index)
            .after(WATCHER)
            .shutdown(teardown_index),
    )
```

`init`/`shutdown` are non-capturing: `fn(&mut SetupContext<'_>) -> anyhow::Result<State>` and `fn(State, &mut SetupContext<'_>) -> anyhow::Result<()>`. `SetupContext` exposes `app_info()`, `app_proxy()`, and `app()` as the one escape to `&mut App`. A module whose `init` fails rolls back only the prefix that already initialized; a failing teardown is reported and the remaining reverse teardown still runs regardless."#,
    },
    ReferenceTopic {
        title: "Commands and menus",
        summary: "Typed, fallible commands; cross-platform bindings; menus projected from one registry.",
        body: r#"`Command<A: Action>` keeps a typed, fallible handler instead of a boxed callback. `CommandBinding` carries the macOS/Windows/Linux chords plus an optional key context; `MenuBar` projects the command registry into the native menu or an in-window `AppMenuBar`.

```rust
actions!(my_app, [ToggleSidebar]);

AppDeclaration::new(identity())
    .command(
        Command::app(CommandId::new("view.toggle_sidebar"), ToggleSidebar, toggle_sidebar)
            .label("Toggle Sidebar")
            .binding(CommandBinding::platform("cmd-k", "ctrl-k")),
    )
    .menu_bar(MenuBar::standard().contribute(
        Menu::keyed(MenuKey::WINDOW).command(CommandId::new("view.toggle_sidebar")),
    ))
```

`Command::window(id, action)` dispatches to the focused view instead of an app handler, scoped with `CommandBinding::same(chord).key_context(name)` — the shape this gallery's own "Toggle Search" shortcut uses, and the shape this story's previous revision faked for `OpenSettings` with a raw `KeyBinding`, before this rewrite removed it. `MenuBar::standard()` is the platform-conventional layout; `.hide`/`.insert`/`.contribute` edit it, `MenuBar::custom(..)`/`MenuBar::none()` replace it outright. Settings, About, Quit, Edit, and Window are added automatically wherever the declaration resolved the matching feature — never something a story page adds or removes."#,
    },
    ReferenceTopic {
        title: "Lifecycle",
        summary: "AppEvent is a queued stream with one deferred-quit path and two liveness policies.",
        body: r#"`AppEvent` is a queued stream, not one launch callback: raw platform listeners register before any service exists, and events that arrive early are buffered and delivered FIFO once the shell is ready.

**Started → Reopened / OpenRequested (may precede Started, then queued) → LastWindowClosed → ShutdownRequested(reason) → WillExit**

```rust
AppDeclaration::new(identity())
    .start(|cx| { restore_window_layout(cx); Ok(()) })
    .on_event(|event, cx| { log_lifecycle(event, cx); Ok(()) })
    .shutdown(|cx| { flush_recent_files(cx); Ok(()) })
    .exit_policy(ExitPolicy::WhenIdle)
    .initial_activation(InitialActivation::Regular)
```

`ShutdownRequested` fires exactly once however quit was requested (menu, `request_quit`, tray, last window); a quit requested while an event is already being delivered is deferred, not dropped, and runs after the current delivery pass finishes. `ExitPolicy::Explicit` keeps a tray-first app alive with no windows; `InitialActivation::Passive` launches it without stealing focus."#,
    },
    ReferenceTopic {
        title: "Runtime traits",
        summary: "Shell, Commands, and Settings — reached identically from any &mut App, with nonfatal errors routed through one reporter.",
        body: r#"`Shell`, `Commands`, and `Settings` are extension traits on `gpui::App`, reached identically from a command handler, a window callback, or a `SetupContext::app()`.

```rust
fn toggle_sidebar(_: &ToggleSidebar, cx: &mut App) -> anyhow::Result<()> {
    cx.open_surface(SurfaceKey::<Sidebar, ()>::new("sidebar"), &())?;
    Ok(())
}

cx.register_command(Command::app(id, action, handler))?;
let settings = cx.settings::<EditorSettings>(key);
cx.update_settings::<EditorSettings, _>(key, |settings, _| settings.zoom += 1)?;
```

A failing `Command::app` handler is never fatal: it is caught and reported as a `RuntimeError::command`, observed by the one reporter a declaration installs with `.runtime_errors(hook)` — `fn(&RuntimeError, &mut App)`. Module and shutdown failures are reported the same way, as `RuntimeError::module`/`RuntimeError::shutdown`; nothing here aborts the process."#,
    },
    ReferenceTopic {
        title: PLATFORM_CAPABILITIES_TITLE,
        summary: "Runtime values, not cfg! constants — a fork stub is reported, never faked.",
        body: r#"`PlatformCapabilities` are runtime values, not `cfg!` constants, so a fork stub (an unimplemented tray, an unverified overlay) is reported honestly instead of silently pretending to work.

```rust
match capabilities.get(PlatformCapability::Tray) {
    Capability::Supported => install_tray(cx),
    Capability::Unsupported { reason } => tracing::warn!("tray unavailable: {reason}"),
}

capabilities.require(PlatformCapability::OverlaySurface)?; // Err(UnsupportedCapability)
```

The grid below is `PlatformCapabilities::detect()` for the process actually running this gallery — reading it dispatches no action and changes nothing about the running window."#,
    },
    ReferenceTopic {
        title: "Root-owned layers",
        summary: "The menu bar, sheet, dialog, and notification layers belong to Root, not to a surface's own content.",
        body: r#"`neutron_components::Root` wraps every declared surface's content. It — not the application, and not this gallery page — owns the in-window menu bar, the sheet, dialog, and notification layers, so they compose consistently across every surface without each screen reimplementing them.

```rust
window.open_dialog(cx, |dialog, _, _| dialog.child("Discard changes?"));
window.open_sheet_at(Placement::Right, cx, |sheet, _, _| sheet.child(Inspector));
window.push_notification("Saved", cx);
```

`RawWindow` and overlay surfaces opt out of this composition entirely: their content view *is* the window root, so an application that opens one owns all of its chrome itself."#,
    },
];

/// The documented topics, in canonical declaration order.
fn reference_topics() -> &'static [ReferenceTopic] {
    REFERENCE_TOPICS
}

pub struct AppShellStory {
    focus_handle: FocusHandle,
}

impl super::Story for AppShellStory {
    fn title() -> &'static str {
        "App Shell"
    }

    fn description() -> &'static str {
        "The resolved DesktopApp / AppDeclaration / AppShell public interface: declaration flow, surfaces, commands, lifecycle, and runtime seams."
    }

    fn new_view(window: &mut Window, cx: &mut App) -> Entity<impl Render> {
        Self::view(window, cx)
    }
}

impl AppShellStory {
    pub fn view(window: &mut Window, cx: &mut App) -> Entity<Self> {
        cx.new(|cx| Self::new(window, cx))
    }

    fn new(_: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
        }
    }
}

impl Focusable for AppShellStory {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for AppShellStory {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // A plain shared reference: every helper below only reads theme state,
        // and reading `PlatformCapabilities::detect()` here dispatches no
        // action and mutates nothing about the outer, real, running app.
        let cx: &Context<Self> = cx;
        let capabilities = PlatformCapabilities::detect();

        v_flex()
            .id("app-shell-story")
            .track_focus(&self.focus_handle)
            .gap_6()
            .children(reference_topics().iter().enumerate().map(|(index, topic)| {
                section(topic.title)
                    .sub_title(topic.summary)
                    .child(topic_card(topic, index, cx))
                    .when(topic.title == PLATFORM_CAPABILITIES_TITLE, |section| {
                        section.child(capability_grid(capabilities, cx))
                    })
            }))
    }
}

/// One topic's markdown body, in a bordered card matching this gallery's
/// other reference cards.
fn topic_card(
    topic: &ReferenceTopic,
    index: usize,
    cx: &Context<AppShellStory>,
) -> impl IntoElement {
    div()
        .w_full()
        .p_3()
        .rounded(cx.theme().radius)
        .bg(cx.theme().muted)
        .border_1()
        .border_color(cx.theme().border)
        .child(
            TextView::markdown(("app-shell-topic", index), topic.body)
                .selectable(true)
                .text_sm(),
        )
}

/// The live `PlatformCapabilities::detect()` grid for this process.
fn capability_grid(
    capabilities: PlatformCapabilities,
    cx: &Context<AppShellStory>,
) -> impl IntoElement {
    h_flex()
        .flex_wrap()
        .w_full()
        .gap_3()
        .child(capability_card(
            "Overlay surface",
            capabilities.overlay_surface,
            cx,
        ))
        .child(capability_card("Tray", capabilities.tray, cx))
        .child(capability_card(
            "Dock menu / jump list",
            capabilities.dock_menu,
            cx,
        ))
        .child(capability_card(
            "Credential store",
            capabilities.credential_store,
            cx,
        ))
        .child(capability_card("URL schemes", capabilities.url_schemes, cx))
        .child(capability_card(
            "Precise window positioning",
            capabilities.precise_window_positioning,
            cx,
        ))
}

fn capability_card(
    label: &'static str,
    capability: Capability,
    cx: &Context<AppShellStory>,
) -> impl IntoElement {
    let (state, reason, color) = match capability {
        Capability::Supported => ("Supported", None, cx.theme().green),
        Capability::Unsupported { reason } => ("Unsupported", Some(reason), cx.theme().red),
    };

    v_flex()
        .min_w(px(220.))
        .flex_1()
        .gap_1()
        .p_3()
        .rounded(cx.theme().radius)
        .border_1()
        .border_color(cx.theme().border)
        .child(
            h_flex()
                .justify_between()
                .w_full()
                .child(Label::new(label).text_sm())
                .child(div().text_sm().text_color(color).child(state)),
        )
        .when_some(reason, |this, reason| {
            this.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(reason),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exact ten seams this story is required to document, in the order
    /// they are declared above. A rewrite that drops, renames, or reorders one
    /// regresses here first, instead of only being noticed by eyeballing the
    /// rendered gallery page.
    const EXPECTED_TITLES: [&str; 10] = [
        "Entry point",
        "Convention defaults",
        "Surfaces",
        "Launch",
        "Setup modules",
        "Commands and menus",
        "Lifecycle",
        "Runtime traits",
        "Platform capabilities",
        "Root-owned layers",
    ];

    #[test]
    fn topics_cover_every_required_seam_in_canonical_order() {
        let titles: Vec<&str> = reference_topics().iter().map(|topic| topic.title).collect();
        assert_eq!(titles, EXPECTED_TITLES);
    }

    #[test]
    fn every_topic_has_a_non_empty_summary_and_body() {
        for topic in reference_topics() {
            assert!(
                !topic.summary.trim().is_empty(),
                "{} is missing its summary",
                topic.title
            );
            assert!(
                !topic.body.trim().is_empty(),
                "{} is missing its body",
                topic.title
            );
        }
    }

    #[test]
    fn the_capability_section_title_matches_the_documented_topic() {
        assert!(
            reference_topics()
                .iter()
                .any(|topic| topic.title == PLATFORM_CAPABILITIES_TITLE),
            "the live capability grid must attach to a topic that really exists",
        );
    }
}
