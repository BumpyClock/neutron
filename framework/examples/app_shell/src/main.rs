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
#[cfg(any(target_os = "windows", target_os = "linux"))]
use gpui_component_app::commands::AppMenusExt as _;
use gpui_component_app::commands::StandardMenus;
use gpui_component_app::gpui::*;
use gpui_component_app::prelude::*;
use gpui_component_app::ui::{ActiveTheme as _, switch::Switch, v_flex};
use serde::{Deserialize, Serialize};

gpui_component_app::include_identity!();

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

struct MainView {
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    menu_bar: Entity<gpui_component_app::ui::menu::AppMenuBar>,
}

impl MainView {
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    fn with_menu_bar(menu_bar: Entity<gpui_component_app::ui::menu::AppMenuBar>) -> Self {
        Self { menu_bar }
    }

    #[cfg(target_os = "macos")]
    fn new() -> Self {
        Self {}
    }
}

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
        #[cfg(any(target_os = "windows", target_os = "linux"))]
        return v_flex()
            .size_full()
            .child(self.menu_bar.clone())
            .child(content);
        #[cfg(target_os = "macos")]
        content
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

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("app_shell failed: {error:#}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<(), AppShellError> {
    let smoke = std::env::args().any(|arg| arg == "--smoke");
    let asset_smoke = std::env::args().any(|arg| arg == "--asset-smoke");
    let fail_start = std::env::args().any(|arg| arg == "--fail-start");

    AppShell::builder(APP_IDENTITY)
        .assets(ExampleAssets)
        .assets(gpui_component_assets::Assets)
        .initial_activation(InitialActivation::Forced)
        .settings::<ExampleSettings>(StoreKey::PRIMARY)
        .theme(ThemeSource::registry())
        .standard_menus(
            StandardMenus::new()
                .with_theme_menu()
                .on_settings(open_settings)
                .on_about(open_about),
        )
        .start(move |_launch, cx| {
            if fail_start {
                // Startup failure wins over a quit requested during Starting:
                // AppShell returns the error and this executable maps it nonzero.
                eprintln!("APP_SHELL_FAIL_START_REACHED");
                cx.request_quit();
                bail!("requested startup failure");
            }

            if asset_smoke {
                validate_asset_chain(cx)?;
                cx.request_quit();
                return Ok(());
            }

            cx.set_global(ExampleService {
                config_dir: cx.app_info().paths().config_dir().to_path_buf(),
            });
            cx.update_settings(StoreKey::PRIMARY, |settings: &mut ExampleSettings, _| {
                settings.launch_count += 1;
            })?;
            WindowManager::open(
                cx,
                WindowSpec::new("main").title("App Shell Example"),
                |_, cx| {
                    #[cfg(any(target_os = "windows", target_os = "linux"))]
                    let menu_bar = cx.new_app_menu_bar();
                    cx.new(|_| {
                        #[cfg(any(target_os = "windows", target_os = "linux"))]
                        return MainView::with_menu_bar(menu_bar);
                        #[cfg(target_os = "macos")]
                        MainView::new()
                    })
                },
            )?;
            if smoke {
                schedule_smoke_quit(cx);
            }
            Ok(())
        })
        .run()
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

fn open_settings(cx: &mut App) -> anyhow::Result<()> {
    WindowManager::open_singleton(
        cx,
        WindowSpec::new("settings").title("Settings"),
        |_, cx| cx.new(|_| SettingsView),
    )?;
    Ok(())
}

fn open_about(cx: &mut App) -> anyhow::Result<()> {
    WindowManager::open_singleton(
        cx,
        WindowSpec::new("about").title("About App Shell Example"),
        |_, cx| cx.new(|_| AboutView),
    )?;
    Ok(())
}
