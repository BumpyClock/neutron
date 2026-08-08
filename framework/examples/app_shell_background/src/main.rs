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
//!   (`open_singleton`), then releases the background hold so the window's own
//!   lease keeps the app alive. Closing that window then drops to zero holds / zero
//!   windows and the shell exits — demonstrating the hold/release exit contract.
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

#[cfg(any(target_os = "windows", target_os = "linux"))]
use gpui_component_app::commands::AppMenusExt as _;
use gpui_component_app::commands::StandardMenus;
use gpui_component_app::gpui::*;
use gpui_component_app::prelude::*;
use gpui_component_app::ui::{ActiveTheme as _, v_flex};

gpui_component_app::include_identity!();

/// How long the app stays window-less before the background task opens its window.
/// Kept below [`SMOKE_LIFETIME`] so a `--smoke` run still exercises the open path.
const BACKGROUND_OPEN_DELAY: Duration = Duration::from_secs(1);
/// How long a `--smoke` run stays up before requesting a clean quit.
const SMOKE_LIFETIME: Duration = Duration::from_secs(3);
static WINDOW_READY: AtomicBool = AtomicBool::new(false);
/// Window opened after background work; themed so the theme service is visibly live.
struct BackgroundWindow {
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    menu_bar: Entity<gpui_component_app::ui::menu::AppMenuBar>,
}

impl BackgroundWindow {
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    fn with_menu_bar(menu_bar: Entity<gpui_component_app::ui::menu::AppMenuBar>) -> Self {
        Self { menu_bar }
    }

    #[cfg(target_os = "macos")]
    fn new() -> Self {
        Self {}
    }
}

impl Render for BackgroundWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = v_flex()
            .flex_1()
            .items_center()
            .justify_center()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child("Opened after background work");
        #[cfg(any(target_os = "windows", target_os = "linux"))]
        return v_flex()
            .size_full()
            .child(self.menu_bar.clone())
            .child(content);
        #[cfg(target_os = "macos")]
        content
    }
}

fn main() -> Result<(), AppShellError> {
    let smoke = std::env::args().any(|arg| arg == "--smoke");

    AppShell::builder(APP_IDENTITY)
        .initial_activation(InitialActivation::Passive)
        .exit_policy(ExitPolicy::WhenIdle)
        .theme(ThemeSource::registry())
        .standard_menus(StandardMenus::new().with_theme_menu())
        .start(move |_launch, cx| {
            // Background work takes a liveness lease: with zero windows and this hold
            // outstanding, `ExitPolicy::WhenIdle` keeps the app alive.
            let hold = cx.shell().hold("background-service");
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

            if smoke {
                schedule_smoke_quit(cx, cx.shell().hold("smoke-verifier"));
            }
            Ok(())
        })
        .run()
}

/// Open the singleton main window, consuming the background hold.
fn open_main_window(cx: &mut App, hold: ShellHold) -> anyhow::Result<()> {
    WindowManager::open_singleton(
        cx,
        WindowSpec::new("main").title("App Shell Background Example"),
        |_, cx| {
            #[cfg(any(target_os = "windows", target_os = "linux"))]
            let menu_bar = cx.new_app_menu_bar();
            cx.new(|_| {
                #[cfg(any(target_os = "windows", target_os = "linux"))]
                return BackgroundWindow::with_menu_bar(menu_bar);
                #[cfg(target_os = "macos")]
                BackgroundWindow::new()
            })
        },
    )?;
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
