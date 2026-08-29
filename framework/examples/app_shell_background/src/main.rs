//! `app_shell_background` — zero-window liveness conformance example.
//!
//! A background app may launch with **no window** and must not exit while useful
//! work remains. This example proves that shell contract without claiming native
//! tray support:
//!
//! - [`InitialActivation::Passive`] — launch without stealing focus, zero windows.
//! - A background service takes a [`ShellHold`] liveness lease so the app stays
//!   alive at zero windows ([`ExitPolicy::WhenIdle`] would otherwise exit).
//! - A timer completes the background task by opening a singleton window
//!   (the declared `"main"` surface), then releases the background hold so the
//!   window's own lease keeps the app alive. Closing that window then drops to
//!   zero holds / zero windows and the shell exits — demonstrating the
//!   hold/release exit contract.
//!
//! Run it: `cargo run -p app_shell_background`. The `--smoke` flag requests a clean
//! quit after a few seconds with exit code 0, for the CI `native-launch-smoke`
//! job.
//!
//! `--smoke` only verifies clean passive launch and shutdown. It does not claim
//! tray support or replace native tray integration.

#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use neutron_components_app::gpui::*;
use neutron_components_app::prelude::*;
use neutron_components_app::ui::{ActiveTheme as _, v_flex};
use neutron_components_app::{
    AppDeclaration, DesktopApp, LaunchDecision, LaunchSpec, ProcessLaunch, Surface, SurfaceKey,
};

neutron_components_app::include_identity!();

/// How long the app stays window-less before the background task opens its window.
/// Kept below [`SMOKE_LIFETIME`] so a `--smoke` run still exercises the open path.
const BACKGROUND_OPEN_DELAY: Duration = Duration::from_secs(1);
/// How long a `--smoke` run stays up before requesting a clean quit.
const SMOKE_LIFETIME: Duration = Duration::from_secs(3);
static WINDOW_READY: AtomicBool = AtomicBool::new(false);
/// Window opened after background work; themed so the theme service is visibly live.
struct BackgroundWindow;

impl Render for BackgroundWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = v_flex()
            .flex_1()
            .items_center()
            .justify_center()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child("Opened after background work");
        v_flex().size_full().child(content)
    }
}

/// This app declares no primary surface: it launches with zero windows and
/// opens its one window later, from background work.
struct AppShellBackground;

impl DesktopApp for AppShellBackground {
    fn declaration() -> AppDeclaration {
        AppDeclaration::new(APP_IDENTITY)
            .initial_activation(InitialActivation::Passive)
            .exit_policy(ExitPolicy::WhenIdle)
            .theme(ThemeSource::registry())
            .menu_bar(MenuBar::standard())
            .surface(
                Surface::new(
                    SurfaceKey::<BackgroundWindow>::new("main"),
                    build_background_window,
                )
                .title("App Shell Background Example"),
            )
            .launch(LaunchSpec::new(parse_launch).before_primary(start_background_work))
    }
}

fn main() -> Result<(), AppShellError> {
    AppShell::run::<AppShellBackground>()
}

fn build_background_window(
    _args: &(),
    _window: &mut Window,
    cx: &mut App,
) -> Entity<BackgroundWindow> {
    cx.new(|_| BackgroundWindow)
}

/// Typed CLI parsing: the only flag this example recognizes is `--smoke`.
fn parse_launch(process: &ProcessLaunch) -> anyhow::Result<LaunchDecision<bool>> {
    let smoke = process.args().iter().any(|arg| arg == "--smoke");
    Ok(LaunchDecision::Run(smoke))
}

/// Launch-specific work: start the background task, and under `--smoke`, also
/// schedule the smoke-test's own clean-quit timer.
fn start_background_work(smoke: &bool, cx: &mut App) -> anyhow::Result<()> {
    // Background work takes a liveness lease: with zero windows and this hold
    // outstanding, `ExitPolicy::WhenIdle` keeps the app alive.
    let hold = cx.hold("background-service");
    let proxy = cx.app_proxy();

    // Finish the background task after a short delay.
    thread::spawn(move || {
        thread::sleep(BACKGROUND_OPEN_DELAY);
        let _ = proxy.dispatch(move |cx| {
            if let Err(error) = open_main_window(cx, hold) {
                log::error!("app_shell_background failed to open window: {error}");
                cx.request_quit();
            }
        });
    });

    if *smoke {
        schedule_smoke_quit(cx, cx.hold("smoke-verifier"));
    }
    Ok(())
}

/// Open the singleton main window, consuming the background hold.
fn open_main_window(cx: &mut App, hold: ShellHold) -> anyhow::Result<()> {
    cx.open_surface(SurfaceKey::<BackgroundWindow>::new("main"), &())?;
    WINDOW_READY.store(true, Ordering::SeqCst);
    // The window now keeps the app alive, so background work can release its hold.
    drop(hold);
    Ok(())
}

fn schedule_smoke_quit(cx: &mut App, hold: ShellHold) {
    let proxy = cx.app_proxy();
    thread::spawn(move || {
        thread::sleep(SMOKE_LIFETIME);
        if !WINDOW_READY.load(Ordering::SeqCst) {
            // Test-harness failure, not application startup error handling.
            eprintln!("app_shell_background smoke: window never became ready");
            std::process::exit(1);
        }
        let _ = proxy.dispatch(move |cx| {
            drop(hold);
            cx.request_quit();
        });
    });
}
