//! Convenience crate that re-exports GPUI's platform traits and constructors so
//! consumers don't need `#[cfg]` gating.

pub use gpui::Platform;

use std::rc::Rc;

/// Returns a background executor for the current platform.
///
/// # Panics
///
/// Panics when the headless platform cannot be initialized. Call [`try_headless`] and obtain its
/// background executor when initialization errors must be handled by the caller.
pub fn background_executor() -> gpui::BackgroundExecutor {
    current_platform(true).background_executor()
}

/// Constructs an application for the current platform.
///
/// This is the legacy infallible wrapper. New code should use [`try_application`] so platform
/// initialization errors can be handled by the caller.
///
/// # Panics
///
/// Panics when the current platform cannot be initialized. This preserves the historical API;
/// use [`try_application`] to receive the construction error instead.
pub fn application() -> gpui::Application {
    try_application()
        .expect("failed to initialize application; use try_application to handle errors")
}

/// Constructs a headless application for the current platform.
///
/// This is the legacy infallible wrapper. New code should use [`try_headless`] so platform
/// initialization errors can be handled by the caller.
///
/// # Panics
///
/// Panics when the current headless platform cannot be initialized. This preserves the historical
/// API; use [`try_headless`] to receive the construction error instead.
pub fn headless() -> gpui::Application {
    try_headless()
        .expect("failed to initialize headless application; use try_headless to handle errors")
}

/// Constructs an application for the current platform without panicking on construction failure.
///
/// # Errors
///
/// Returns an error when the platform cannot be initialized, such as when Windows OLE/DirectX or
/// Linux X11 setup fails.
pub fn try_application() -> gpui::Result<gpui::Application> {
    Ok(gpui::Application::with_platform(try_current_platform(
        false,
    )?))
}

/// Constructs a headless application for the current platform without panicking on construction
/// failure.
///
/// # Errors
///
/// Returns an error when the headless platform cannot be initialized. Headless Web applications
/// are not supported.
pub fn try_headless() -> gpui::Result<gpui::Application> {
    Ok(gpui::Application::with_platform(try_current_platform(
        true,
    )?))
}

/// Unlike [`application`], this function returns a single-threaded web application.
#[cfg(target_family = "wasm")]
pub fn single_threaded_web() -> gpui::Application {
    gpui::Application::with_platform(Rc::new(gpui_web::WebPlatform::new(false)))
}

/// Initializes panic hooks and logging for the web platform.
/// Call this before running the application in a wasm_bindgen entrypoint.
#[cfg(target_family = "wasm")]
pub fn web_init() {
    console_error_panic_hook::set_once();
    gpui_web::init_logging();
}

/// Returns the default [`Platform`] for the current OS without panicking on construction failure.
///
/// # Errors
///
/// Returns an error when the platform cannot be initialized, such as when Windows OLE/DirectX or
/// Linux X11 setup fails. Headless Web construction is unsupported and returns an error.
pub fn try_current_platform(headless: bool) -> gpui::Result<Rc<dyn Platform>> {
    #[cfg(target_os = "macos")]
    {
        Ok(Rc::new(gpui_macos::MacPlatform::new(headless)))
    }

    #[cfg(target_os = "windows")]
    {
        Ok(Rc::new(gpui_windows::WindowsPlatform::new(headless)?))
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    {
        gpui_linux::try_current_platform(headless)
    }

    #[cfg(target_family = "wasm")]
    {
        if headless {
            Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "headless web platform is not supported",
            )
            .into())
        } else {
            Ok(Rc::new(gpui_web::WebPlatform::new(true)))
        }
    }
}

/// Returns the default [`Platform`] for the current OS.
///
/// This is the legacy infallible wrapper. New code should use [`try_current_platform`] so
/// platform initialization errors can be handled by the caller.
///
/// # Panics
///
/// Panics when the current platform cannot be initialized. This preserves the historical API;
/// use [`try_current_platform`] to receive the construction error instead.
pub fn current_platform(headless: bool) -> Rc<dyn Platform> {
    try_current_platform(headless)
        .expect("failed to initialize current platform; use try_current_platform to handle errors")
}

/// Returns a new [`HeadlessRenderer`] for the current platform, if available.
#[cfg(feature = "test-support")]
pub fn current_headless_renderer() -> Option<Box<dyn gpui::PlatformHeadlessRenderer>> {
    // This standalone fork benchmarks scene construction without a native renderer. The fork's
    // Metal renderer also carries retained-layer and backdrop-blur state that the upstream
    // headless renderer does not model, so report that no compatible renderer is available.
    None
}

#[cfg(test)]
mod api_tests {
    use super::*;

    #[test]
    fn platform_constructor_signatures_remain_compatible() {
        let _: fn(bool) -> gpui::Result<Rc<dyn Platform>> = try_current_platform;
        let _: fn() -> gpui::Result<gpui::Application> = try_application;
        let _: fn() -> gpui::Result<gpui::Application> = try_headless;
        let _: fn(bool) -> Rc<dyn Platform> = current_platform;
        let _: fn() -> gpui::Application = application;
        let _: fn() -> gpui::Application = headless;
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use gpui::{AppContext, Empty, VisualTestAppContext};
    use std::cell::RefCell;
    use std::time::Duration;

    // Note: All VisualTestAppContext tests are ignored by default because they require
    // the macOS main thread. Standard Rust tests run on worker threads, which causes
    // SIGABRT when interacting with macOS AppKit/Cocoa APIs.
    //
    // To run these tests, use:
    // cargo test -p bumpyclock-gpui visual_test_context -- --ignored --test-threads=1

    #[test]
    #[ignore] // Requires macOS main thread
    fn test_foreground_tasks_run_with_run_until_parked() {
        let mut cx = VisualTestAppContext::new(current_platform(false));

        let task_ran = Rc::new(RefCell::new(false));

        // Spawn a foreground task via the App's spawn method
        // This should use our TestDispatcher, not the MacDispatcher
        {
            let task_ran = task_ran.clone();
            cx.update(|cx| {
                cx.spawn(async move |_| {
                    *task_ran.borrow_mut() = true;
                })
                .detach();
            });
        }

        // The task should not have run yet
        assert!(!*task_ran.borrow());

        // Run until parked should execute the foreground task
        cx.run_until_parked();

        // Now the task should have run
        assert!(*task_ran.borrow());
    }

    #[test]
    #[ignore] // Requires macOS main thread
    fn test_advance_clock_triggers_delayed_tasks() {
        let mut cx = VisualTestAppContext::new(current_platform(false));

        let task_ran = Rc::new(RefCell::new(false));

        // Spawn a task that waits for a timer
        {
            let task_ran = task_ran.clone();
            let executor = cx.background_executor.clone();
            cx.update(|cx| {
                cx.spawn(async move |_| {
                    executor.timer(Duration::from_millis(500)).await;
                    *task_ran.borrow_mut() = true;
                })
                .detach();
            });
        }

        // Run until parked - the task should be waiting on the timer
        cx.run_until_parked();
        assert!(!*task_ran.borrow());

        // Advance clock past the timer duration
        cx.advance_clock(Duration::from_millis(600));

        // Now the task should have completed
        assert!(*task_ran.borrow());
    }

    #[test]
    #[ignore] // Requires macOS main thread - window creation fails on test threads
    fn test_window_spawn_uses_test_dispatcher() {
        let mut cx = VisualTestAppContext::new(current_platform(false));

        let task_ran = Rc::new(RefCell::new(false));

        let window = cx
            .open_offscreen_window_default(|_, cx| cx.new(|_| Empty))
            .expect("Failed to open window");

        // Spawn a task via window.spawn - this is the critical test case
        // for tooltip behavior, as tooltips use window.spawn for delayed show
        {
            let task_ran = task_ran.clone();
            cx.update_window(window.into(), |_, window, cx| {
                window
                    .spawn(cx, async move |_| {
                        *task_ran.borrow_mut() = true;
                    })
                    .detach();
            })
            .ok();
        }

        // The task should not have run yet
        assert!(!*task_ran.borrow());

        // Run until parked should execute the foreground task spawned via window
        cx.run_until_parked();

        // Now the task should have run
        assert!(*task_ran.borrow());
    }
}
