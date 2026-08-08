#[cfg(any(feature = "wayland", feature = "x11"))]
mod accesskit_shims;
mod dispatcher;
mod headless;
mod keyboard;
mod platform;
#[cfg(any(feature = "wayland", feature = "x11"))]
mod text_system;
#[cfg(feature = "wayland")]
mod wayland;
#[cfg(feature = "x11")]
mod x11;

#[cfg(any(feature = "wayland", feature = "x11"))]
mod xdg_desktop_portal;

pub use dispatcher::*;
pub(crate) use headless::*;
pub(crate) use keyboard::*;
pub(crate) use platform::*;
#[cfg(any(feature = "wayland", feature = "x11"))]
pub(crate) use text_system::*;
#[cfg(feature = "wayland")]
pub(crate) use wayland::*;
#[cfg(feature = "x11")]
pub(crate) use x11::*;

use std::rc::Rc;

const XDG_ACTIVATION_TOKEN_ENV_VAR: &str = "XDG_ACTIVATION_TOKEN";

fn startup_activation_token_from_environment() -> Option<String> {
    std::env::var(XDG_ACTIVATION_TOKEN_ENV_VAR)
        .ok()
        .filter(|token| !token.is_empty())
}

/// Removes and returns the Wayland startup activation token from the process environment.
///
/// Pass the result to [`current_platform_with_startup_activation_token`] so the first Wayland
/// window can consume it without mutating the environment during platform construction.
///
/// # Safety
///
/// The caller must ensure that no other thread can read or write the process environment and that
/// no other thread can spawn a child process for the duration of this call. This normally means
/// calling it during single-threaded process startup, before initializing worker threads or an
/// embedded host runtime.
pub unsafe fn take_startup_activation_token_from_environment() -> Option<String> {
    let token = startup_activation_token_from_environment();
    // SAFETY: The function's contract requires the caller to exclude concurrent environment access
    // and child-process creation for the duration of this call.
    unsafe { std::env::remove_var(XDG_ACTIVATION_TOKEN_ENV_VAR) };
    token
}

/// Returns the default platform implementation for the current OS.
///
/// This compatibility constructor reads `XDG_ACTIVATION_TOKEN` without removing it because it
/// cannot prove that process environment mutation is safe at the call site. Applications that can
/// capture the token during single-threaded startup should call
/// [`take_startup_activation_token_from_environment`] and pass its result to
/// [`current_platform_with_startup_activation_token`].
///
/// # Panics
///
/// Panics when platform initialization fails. Use [`try_current_platform`] to handle the error.
pub fn current_platform(headless: bool) -> Rc<dyn gpui::Platform> {
    try_current_platform(headless).expect("failed to initialize Linux platform")
}

/// Tries to construct the default platform implementation for the current OS.
///
/// # Errors
///
/// Returns an error when the selected backend cannot be initialized.
pub fn try_current_platform(headless: bool) -> anyhow::Result<Rc<dyn gpui::Platform>> {
    try_current_platform_with_startup_activation_token(
        headless,
        startup_activation_token_from_environment(),
    )
}

/// Returns the default platform implementation with an explicitly captured Wayland startup token.
///
/// Passing the token explicitly lets embedders remove it at a startup boundary where process
/// environment mutation is known to be safe. On non-Wayland backends the token is ignored.
///
/// # Panics
///
/// Panics when platform initialization fails. Use
/// [`try_current_platform_with_startup_activation_token`] to handle the error.
pub fn current_platform_with_startup_activation_token(
    headless: bool,
    startup_activation_token: Option<String>,
) -> Rc<dyn gpui::Platform> {
    try_current_platform_with_startup_activation_token(headless, startup_activation_token)
        .expect("failed to initialize Linux platform")
}

/// Tries to construct the default platform implementation with an explicitly captured Wayland
/// startup token.
///
/// # Errors
///
/// Returns an error when the selected backend cannot be initialized.
pub fn try_current_platform_with_startup_activation_token(
    headless: bool,
    startup_activation_token: Option<String>,
) -> anyhow::Result<Rc<dyn gpui::Platform>> {
    #[cfg(feature = "x11")]
    use anyhow::Context as _;

    #[cfg(not(feature = "wayland"))]
    let _ = startup_activation_token;

    if headless {
        return Ok(Rc::new(LinuxPlatform {
            inner: HeadlessClient::new()?,
        }));
    }

    match gpui::guess_compositor() {
        #[cfg(feature = "wayland")]
        "Wayland" => Ok(Rc::new(LinuxPlatform {
            inner: WaylandClient::new(startup_activation_token)?,
        })),

        #[cfg(feature = "x11")]
        "X11" => Ok(Rc::new(LinuxPlatform {
            inner: X11Client::new().context("failed to initialize X11 client")?,
        })),

        "Headless" => anyhow::bail!(
            "no graphical Linux compositor detected; pass headless=true to initialize a headless platform"
        ),
        _ => anyhow::bail!(
            r#"at least one of the "wayland" or "x11" features must be enabled on gpui_linux or gpui_platform"#
        ),
    }
}
