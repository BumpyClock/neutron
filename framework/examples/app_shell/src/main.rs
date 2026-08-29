//! `app_shell` — single-window AppShell conformance example.
//!
//! Exercises the downstream application chain in-repo: generated identity,
//! ordered app/component assets, persistent settings, standard desktop menus,
//! singleton Settings/About surfaces, and an app-owned service.
//!
//! Run it: `cargo run -p app_shell`. `--smoke` requests a clean quit after the
//! initial window opens. `--asset-smoke` validates app-first asset fallback then
//! quits without opening a window. `--fail-start` demonstrates that a startup
//! error exits nonzero through `AppShellError::Startup`.

#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

use std::borrow::Cow;
use std::path::PathBuf;
use std::process::ExitCode;
use std::thread;
use std::time::Duration;

use anyhow::bail;
use neutron_components_app::gpui::*;
use neutron_components_app::prelude::*;
use neutron_components_app::ui::{ActiveTheme as _, switch::Switch, v_flex};
use neutron_components_app::{
    AppDeclaration, DesktopApp, LaunchDecision, LaunchSpec, ProcessLaunch, Surface, SurfaceKey,
};
use serde::{Deserialize, Serialize};

neutron_components_app::include_identity!();

/// How long a `--smoke` run stays up before requesting a clean quit.
const SMOKE_LIFETIME: Duration = Duration::from_secs(3);
const APP_ASSET_PATH: &str = "app_shell/example.txt";
const COMPONENT_ASSET_PATH: &str = "surface/NoiseAsset_256.png";

/// A tiny persisted settings schema, proving the identity -> storage chain: it is
/// keyed by the app's `data_namespace`, which comes from `APP_IDENTITY`.
#[derive(Serialize, Deserialize, Default)]
struct ExampleSettings {
    show_status: bool,
    launch_count: u32,
}

impl AppSettings for ExampleSettings {
    const SCHEMA_VERSION: u32 = 1;
}

/// App-owned path-aware state. AppShell owns path resolution; product services
/// choose their own lifecycle and storage policy.
struct ExampleService {
    config_dir: PathBuf,
    /// Whether this launch requested a `--smoke` quit, read back by the
    /// primary surface's `after_open` hook.
    smoke: bool,
}

impl Global for ExampleService {}

/// First asset source in the AppShell chain. Component assets remain available
/// from the bundled source registered after this one.
struct ExampleAssets;

impl AssetSource for ExampleAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok((path == APP_ASSET_PATH)
            .then(|| Cow::Borrowed(include_bytes!("../assets/example.txt").as_slice())))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok((path.is_empty() || APP_ASSET_PATH.starts_with(path))
            .then(|| APP_ASSET_PATH.into())
            .into_iter()
            .collect())
    }
}

struct MainView;

impl Render for MainView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let config_dir = cx
            .global::<ExampleService>()
            .config_dir
            .display()
            .to_string();
        let content = v_flex()
            .flex_1()
            .items_center()
            .justify_center()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child("App Shell conformance example")
            .child(format!("Settings live in {}", config_dir));
        v_flex().size_full().child(content)
    }
}

struct SettingsView;

impl Render for SettingsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let show_status = cx
            .settings::<ExampleSettings>(StoreKey::PRIMARY)
            .show_status;
        v_flex()
            .size_full()
            .p_4()
            .gap_3()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child("Settings")
            .child(
                Switch::new("show-status")
                    .label("Show status")
                    .checked(show_status)
                    .on_click(cx.listener(|_, checked, _, cx| {
                        if let Err(error) = cx.update_settings(
                            StoreKey::PRIMARY,
                            |settings: &mut ExampleSettings, _| settings.show_status = *checked,
                        ) {
                            log::error!("app_shell settings update failed: {error}");
                        }
                        cx.notify();
                    })),
            )
    }
}

struct AboutView;

impl Render for AboutView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let info = cx.app_info();
        v_flex()
            .size_full()
            .p_4()
            .gap_2()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(format!("About {}", info.display_name()))
            .child(format!("Version {}", info.version()))
            .child(info.app_id().to_string())
    }
}

/// This example's parsed CLI launch modes.
struct AppLaunch {
    smoke: bool,
    asset_smoke: bool,
    fail_start: bool,
}

/// Parse the documented `--smoke`/`--asset-smoke`/`--fail-start` flags.
/// Deterministic and infallible: unrecognized arguments are ignored.
fn parse_launch(process: &ProcessLaunch) -> anyhow::Result<LaunchDecision<AppLaunch>> {
    let has_flag = |flag: &str| {
        process
            .args()
            .iter()
            .any(|arg| arg.to_string_lossy() == flag)
    };
    Ok(LaunchDecision::Run(AppLaunch {
        smoke: has_flag("--smoke"),
        asset_smoke: has_flag("--asset-smoke"),
        fail_start: has_flag("--fail-start"),
    }))
}

/// Launch-specific work before the primary surface opens: fail-start and
/// asset-smoke both request a quit here, which suppresses the primary open;
/// the normal path registers the app-owned service and bumps `launch_count`.
fn before_primary(launch: &AppLaunch, cx: &mut App) -> anyhow::Result<()> {
    if launch.fail_start {
        // Startup failure wins over a quit requested here: AppShell returns
        // the error and this executable maps it nonzero.
        eprintln!("APP_SHELL_FAIL_START_REACHED");
        cx.request_quit();
        bail!("requested startup failure");
    }

    if launch.asset_smoke {
        validate_asset_chain(cx)?;
        cx.request_quit();
        return Ok(());
    }

    cx.set_global(ExampleService {
        config_dir: cx.app_info().paths().config_dir().to_path_buf(),
        smoke: launch.smoke,
    });
    cx.update_settings(StoreKey::PRIMARY, |settings: &mut ExampleSettings, _| {
        settings.launch_count += 1;
    })?;
    Ok(())
}

/// Schedules the `--smoke` quit once the primary surface has opened.
fn after_primary_open(_view: &Entity<MainView>, _window: &mut Window, cx: &mut App) {
    if cx.global::<ExampleService>().smoke {
        schedule_smoke_quit(cx);
    }
}

fn build_main(_launch: &AppLaunch, _window: &mut Window, cx: &mut App) -> Entity<MainView> {
    cx.new(|_| MainView)
}

fn build_settings(_args: &(), _window: &mut Window, cx: &mut App) -> Entity<SettingsView> {
    cx.new(|_| SettingsView)
}

fn build_about(_args: &(), _window: &mut Window, cx: &mut App) -> Entity<AboutView> {
    cx.new(|_| AboutView)
}

/// The AppShell declaration for this conformance example. A zero-sized type:
/// the shell never creates or retains an application object.
struct AppShellExampleApp;

impl DesktopApp for AppShellExampleApp {
    fn declaration() -> AppDeclaration {
        AppDeclaration::new(APP_IDENTITY)
            .assets(ExampleAssets)
            .assets(neutron_components_assets::Assets)
            .initial_activation(InitialActivation::Forced)
            .settings_store::<ExampleSettings>(StoreKey::PRIMARY)
            .settings_surface(
                Surface::new(SurfaceKey::settings(), build_settings).title("Settings"),
            )
            .about_surface(
                Surface::new(SurfaceKey::about(), build_about).title("About App Shell Example"),
            )
            .launch(
                LaunchSpec::new(parse_launch)
                    .before_primary(before_primary)
                    .primary_surface(
                        Surface::new(SurfaceKey::primary(), build_main)
                            .title("App Shell Example")
                            .after_open(after_primary_open),
                    ),
            )
    }
}

fn main() -> ExitCode {
    match AppShell::run::<AppShellExampleApp>() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("app_shell failed: {:#}", anyhow::Error::new(error));
            ExitCode::from(2)
        }
    }
}

fn schedule_smoke_quit(cx: &mut App) {
    let proxy = cx.app_proxy();
    thread::spawn(move || {
        thread::sleep(SMOKE_LIFETIME);
        let _ = proxy.dispatch(|cx| cx.request_quit());
    });
}

fn validate_asset_chain(cx: &App) -> anyhow::Result<()> {
    let app_asset = cx
        .asset_source()
        .load(APP_ASSET_PATH)?
        .ok_or_else(|| anyhow::anyhow!("app asset missing"))?;
    if app_asset.as_ref() != include_bytes!("../assets/example.txt") {
        bail!("app asset contents differ");
    }
    cx.asset_source()
        .load(COMPONENT_ASSET_PATH)?
        .ok_or_else(|| anyhow::anyhow!("bundled component asset missing"))?;
    Ok(())
}
