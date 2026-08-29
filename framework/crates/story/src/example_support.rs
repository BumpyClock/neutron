//! Additive, shared helpers for the standalone `examples/` binaries
//! (tsq-11.5.5).
//!
//! Every example under `framework/crates/story/examples/` is its own
//! [`neutron_components_app::DesktopApp`] declaration and process, sharing the
//! `neutron-story` package's compiled-in identity (see
//! `neutron_components_app::include_identity!()`, called once per example).
//! These helpers keep window sizing, Linux decoration, and bundled-theme
//! behavior identical across every example and the main gallery, without
//! repeating the exact same recipe in eight separate files. Nothing here is
//! reachable from — or changes the behavior of — the main `StoryApp`
//! declaration in the binary target's own `app.rs`.
//!
//! # Concurrent story processes are unsupported
//!
//! Every example that declares [`crate::StoryUiPreferences`] under
//! [`crate::story_preferences_key`] writes to the *same* on-disk settings
//! file as the `neutron-story` binary and every other example, because they
//! all share one compiled-in identity and therefore one settings data
//! namespace. The settings store takes a single-writer OS advisory lock
//! (issue tracked upstream in `framework-app-storage`): running two story
//! processes at once is unsupported, and the second writer's update fails
//! with a reported, typed `SettingsError::Storage` conflict rather than
//! corrupting the file. Do not run the gallery and an example, or two
//! examples, concurrently; serialize any smoke test that exercises more than
//! one of these binaries.

use gpui::{App, Entity, Focusable, Pixels, Size, Window, px, size};
#[cfg(target_os = "linux")]
use gpui::{WindowBackgroundAppearance, WindowDecorations};
use neutron_components_app::{
    AppShellError, SetupContext, SetupKey, SetupModule, Surface, ThemeAsset, ThemeAssetSource,
    ThemeSource, WindowSize,
};

/// Embedded copies of `framework/themes/*.json`.
///
/// Identical in content to the main gallery's own bundled theme source (see
/// the binary target's private `app.rs`): examples get their own copy here
/// because that one is private to the `neutron-story` binary target, not the
/// library. Keeping the same theme sets available in every example is what
/// [`example_theme_source`] is for.
#[derive(rust_embed::RustEmbed)]
#[folder = "../../themes"]
pub struct ExampleThemes;

impl ThemeAssetSource for ExampleThemes {
    fn theme_assets(&self) -> anyhow::Result<Vec<ThemeAsset>> {
        Ok(Self::iter()
            .filter_map(|name| {
                Self::get(&name)
                    .map(|file| ThemeAsset::new(name.into_owned(), file.data.into_owned()))
            })
            .collect())
    }
}

/// The shared bundled theme source every example should declare, matching the
/// main gallery's `ThemeSource::bundled(..)` convention: a theme selection
/// persisted by one story binary resolves the same way in every other.
#[must_use]
pub fn example_theme_source() -> ThemeSource {
    ThemeSource::bundled(ExampleThemes)
}

fn init_http_client(cx: &mut SetupContext<'_>) -> anyhow::Result<()> {
    let client = std::sync::Arc::new(reqwest_client::ReqwestClient::user_agent(
        "neutron-story-example",
    )?);
    cx.app().set_http_client(client);
    Ok(())
}

/// Install the HTTP client that the old shared `neutron_story::init` path
/// provided to every example.
#[must_use]
pub fn example_http_client_module() -> SetupModule {
    SetupModule::new(SetupKey::new("story-example.http-client"), init_http_client)
}

/// Restore initial focus after AppShell opens an example's primary surface.
pub fn focus_example<View: Focusable + 'static>(
    view: &Entity<View>,
    window: &mut Window,
    cx: &mut App,
) {
    let focus_handle = view.read(cx).focus_handle(cx);
    window.defer(cx, move |window, cx| {
        focus_handle.focus(window, cx);
    });
}

/// Report the complete AppShell source chain and return the examples' standard
/// failure exit code.
pub fn example_failure(name: &str, error: AppShellError) -> std::process::ExitCode {
    eprintln!("{name} failed: {:#}", anyhow::Error::new(error));
    std::process::ExitCode::from(2)
}

/// The deleted `create_new_window`'s default requested primary-surface size
/// (1600x1200, before display clamping): the default for an example that
/// does not need a different one.
#[must_use]
pub fn default_example_window_size() -> Size<Pixels> {
    size(px(1600.0), px(1200.0))
}

/// Applies the shared example window recipe to a declared surface: `requested`
/// clamped to 85% of the display (see [`WindowSize::fixed_clamped`]), the
/// deleted `create_new_window[_with_size]` helpers' 480x320 minimum, and (Linux
/// only) a transparent background with client-side decorations. Matches the
/// main gallery's primary surface exactly, so every story window looks and
/// behaves the same regardless of which binary opened it.
#[must_use]
pub fn with_example_window_defaults<View: 'static, Args: 'static>(
    surface: Surface<View, Args>,
    requested: Size<Pixels>,
) -> Surface<View, Args> {
    let surface = surface
        .size(WindowSize::fixed_clamped(requested, 0.85))
        .min_size(size(px(480.0), px(320.0)));

    #[cfg(target_os = "linux")]
    let surface = surface
        .background(WindowBackgroundAppearance::Transparent)
        .decorations(WindowDecorations::Client);

    surface
}
