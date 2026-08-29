//! The `neutron-story` [`DesktopApp`] declaration: a zero-sized application
//! type whose `declaration()` assembles identity, theme, settings, setup
//! modules, commands/menus, and the typed launch spec.

use std::collections::BTreeMap;

use gpui::{App, px, size};
#[cfg(target_os = "linux")]
use gpui::{WindowBackgroundAppearance, WindowDecorations};
use neutron_components_app::prelude::*;
use neutron_components_app::{
    AdvancedHooks, AppDeclaration, AppEvent, AppPaths, DesktopApp, InitialActivation, LaunchSpec,
    ShutdownReason, Surface, SurfaceKey, ThemeAsset, ThemeAssetSource, ThemeSource, WindowSize,
};
use neutron_story::{StoryUiPreferences, build_settings, story_preferences_key};
use serde_json::json;

use crate::APP_IDENTITY;
use crate::commands::{help_menu_bar, open_repository_command, toggle_search_command};
use crate::evidence;
use crate::gallery::{Gallery, build_gallery};
use crate::launch::{after_primary_open, before_primary, parse_launch};
use crate::setup;

/// The primary surface's window title. Shared with `story-smoke` evidence so
/// the recorded surface identity cannot drift from the declared one.
pub(crate) const PRIMARY_TITLE: &str = "Neutron Story";

/// Embedded copies of `framework/themes/*.json`, synced into the app's config
/// directory by the theme convention's bundled source.
#[derive(rust_embed::RustEmbed)]
#[folder = "../../themes"]
struct BundledThemes;

impl ThemeAssetSource for BundledThemes {
    fn theme_assets(&self) -> anyhow::Result<Vec<ThemeAsset>> {
        Ok(Self::iter()
            .filter_map(|name| {
                Self::get(&name)
                    .map(|file| ThemeAsset::new(name.into_owned(), file.data.into_owned()))
            })
            .collect())
    }
}

/// The embedded theme files, keyed by name and sorted deterministically.
///
/// Read from the very same [`BundledThemes`] value [`declaration`] hands to
/// [`ThemeSource::bundled`], so `story-smoke` compares the installed theme
/// directory against the bytes this binary actually ships rather than against
/// a second, independently declared list.
///
/// [`declaration`]: DesktopApp::declaration
///
/// # Errors
///
/// Returns whatever the asset source reports, and rejects a duplicate asset
/// name, which would make the installed-file comparison ambiguous.
pub(crate) fn embedded_theme_assets() -> anyhow::Result<BTreeMap<String, Vec<u8>>> {
    let mut assets = BTreeMap::new();
    for asset in BundledThemes.theme_assets()? {
        let name = asset.name.clone();
        if assets
            .insert(name.clone(), asset.contents.as_ref().to_vec())
            .is_some()
        {
            anyhow::bail!("bundled theme asset {name} was embedded twice");
        }
    }
    Ok(assets)
}

fn init_logging(_paths: &AppPaths) -> anyhow::Result<()> {
    use tracing_subscriber::layer::SubscriberExt as _;
    use tracing_subscriber::util::SubscriberInitExt as _;

    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("neutron_components=trace".parse()?),
        )
        .init();
    Ok(())
}

/// The primary Gallery surface: restores the historical 1600x1200 requested
/// size (display clamping owned by `Surface`/`AppShell`) and 480x320 minimum,
/// and the Linux transparent background / client-side decoration override.
fn primary_surface() -> Surface<Gallery, crate::launch::StoryLaunch> {
    let surface = Surface::new(SurfaceKey::<Gallery, _>::primary(), build_gallery)
        .title(PRIMARY_TITLE)
        .size(WindowSize::fixed_clamped(
            size(px(1600.0), px(1200.0)),
            0.85,
        ))
        .min_size(size(px(480.0), px(320.0)))
        .after_open(after_primary_open);

    #[cfg(target_os = "linux")]
    let surface = surface
        .background(WindowBackgroundAppearance::Transparent)
        .decorations(WindowDecorations::Client);

    surface
}

/// The real, singleton Settings surface: General (locale) and Appearance
/// (theme mode, theme selection, font size, radius, scrollbar behavior, and
/// list active highlighting), built from existing Settings components.
fn settings_surface() -> Surface<neutron_story::StorySettings, ()> {
    Surface::new(SurfaceKey::settings(), build_settings)
        .title("Settings")
        .size(WindowSize::Fixed(size(px(720.0), px(560.0))))
}

/// The `neutron-story` application. Zero-sized: `AppShell` never creates or
/// retains an application object.
pub(crate) struct StoryApp;

/// Record the AppShell shutdown events `story-smoke` proves were delivered.
/// A no-op on an ordinary run: no evidence stream is installed then.
fn on_event(event: &AppEvent, _cx: &mut App) -> anyhow::Result<()> {
    let Some(evidence) = evidence::active() else {
        return Ok(());
    };
    match event {
        AppEvent::ShutdownRequested(reason) => evidence.emit(
            evidence::SHUTDOWN_REQUESTED,
            json!({"reason": shutdown_reason_name(*reason)}),
        ),
        AppEvent::WillExit => evidence.emit(evidence::WILL_EXIT, json!({})),
        _ => Ok(()),
    }
}

fn shutdown_reason_name(reason: ShutdownReason) -> &'static str {
    match reason {
        ShutdownReason::StartupFailure => "startup_failure",
        ShutdownReason::LastWindowClosed => "last_window_closed",
        ShutdownReason::Requested => "requested",
        ShutdownReason::PlatformQuit => "platform_quit",
        _ => "other",
    }
}

impl DesktopApp for StoryApp {
    fn declaration() -> AppDeclaration {
        AppDeclaration::new(APP_IDENTITY)
            .advanced(AdvancedHooks::new().logging(LoggingPolicy::Configure(init_logging)))
            .initial_activation(InitialActivation::Forced)
            .on_event(on_event)
            .theme(ThemeSource::bundled(BundledThemes))
            .settings_store::<StoryUiPreferences>(story_preferences_key())
            .settings_surface(settings_surface())
            .setup(setup::story_http_client_module())
            .setup(setup::story_app_state_module())
            .setup(setup::story_panels_module())
            .setup(neutron_story::story_preferences_module())
            .command(toggle_search_command())
            .command(open_repository_command())
            .menu_bar(help_menu_bar())
            .launch(
                LaunchSpec::new(parse_launch)
                    .before_primary(before_primary)
                    .primary_surface(primary_surface()),
            )
    }
}
