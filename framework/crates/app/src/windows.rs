//! Window management (plan §3 "Windows", promoted from agent-term + ansible).
//!
//! [`WindowManager`] is an app-scoped GPUI [`gpui::Global`] (installed by
//! [`WindowsPlugin`] at init — *not* a process `OnceLock`). It opens windows and
//! overlays, wraps them in `gpui_component::Root` by policy, numbers/titles them
//! (`"App"`, `"App - 2"`), keeps a stale-handle-safe singleton state machine,
//! and gives each real window a liveness lease so the exit policy sees the
//! window count correctly.
//!
//! ## Borrow discipline
//!
//! GPUI's `open_window` needs `&mut App` re-entrantly, so no method holds a
//! borrow of the [`WindowManager`] global across the open. Each method takes
//! short `global_mut` borrows around the GPUI call — the same move-out pattern
//! the shell core uses for plugin dispatch.
//!
//! ## Pure core
//!
//! Numbering, the singleton transitions, and registry bookkeeping live in the
//! GPUI-free [`registry`] module and are unit-tested there with a fake handle.
//! This module is the thin GPUI edge.

mod key;
mod registry;
mod spec;

use std::collections::{HashMap, HashSet};

use gpui::{
    AnyWindowHandle, App, AppContext as _, Bounds, Entity, Render, Subscription, Window,
    WindowBounds, WindowHandle,
};
use gpui_component::Root;

use crate::capabilities::Capability;
use crate::commands::CloseWindow;
#[cfg(target_os = "macos")]
use crate::commands::{Minimize, Zoom};
use crate::error::AppShellError;
use crate::handles::{AppShellExt as _, ShellState};
use crate::liveness::ShellHold;
use crate::plugin::{AppPlugin, ShellSeed, sealed};

pub use key::WindowKey;
use registry::SingletonMetadata;
pub use registry::{SingletonPhase, SurfaceKind, WindowRecord};
pub use spec::{OverlaySpec, RootPolicy, WindowSize, WindowSpec};

/// The liveness reason recorded for a window's lease.
const WINDOW_HOLD_REASON: &str = "window";

/// Errors from window/overlay operations.
///
/// Kept local to this module: the core [`AppShellError`] has no window variants
/// and is owned elsewhere. `open`/`open_singleton`/`open_raw`/`open_overlay`
/// therefore return [`WindowError`]; wiring this into the public error taxonomy
/// (if desired) is an integration decision — see the module's report.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum WindowError {
    /// The underlying GPUI `open_window` / `open_overlay_surface` call failed.
    #[error("failed to open window")]
    OpenFailed(#[source] anyhow::Error),

    /// The operation is not supported on this platform/session (e.g. overlay
    /// surfaces off macOS); `reason` mirrors the capability report.
    #[error("operation unsupported: {reason}")]
    Unsupported {
        /// Human-readable reason, from [`Capability::Unsupported`].
        reason: &'static str,
    },

    /// A singleton key is already live or opening with a different content
    /// view type.
    #[error("singleton window {key} has content type {expected}, not requested type {actual}")]
    ContentTypeMismatch {
        /// Stable singleton key.
        key: WindowKey,
        /// Content type registered by the live/in-flight singleton.
        expected: &'static str,
        /// Content type requested by this open attempt.
        actual: &'static str,
    },

    /// A manager method does not match the requested root policy, or a
    /// singleton key is reused with a different root policy.
    #[error("window {key} has root policy {expected:?}, not requested policy {actual:?}")]
    RootPolicyMismatch {
        /// Stable window key.
        key: WindowKey,
        /// Root policy expected by the manager or registered singleton.
        expected: RootPolicy,
        /// Root policy requested by the spec.
        actual: RootPolicy,
    },
}

/// A freshly opened, `Root`-wrapped window.
///
/// The auto-`Root`-wrap changes the window's root type, so the handle is typed
/// `WindowHandle<Root>` while the caller's content entity is returned alongside
/// it (plan §3 — "make that explicit").
pub struct OpenedWindow<V: 'static> {
    /// The window handle (root type is `Root`).
    pub window: WindowHandle<Root>,
    /// The caller's content view, wrapped inside the window's `Root`.
    pub content: Entity<V>,
}

/// Outcome of [`WindowManager::open_singleton`].
pub enum Singleton<V: 'static> {
    /// A new window was created.
    Opened(OpenedWindow<V>),
    /// A live window already existed and was focused; nothing new created.
    Reused,
    /// A create for this key is already in flight; nothing done.
    InFlight,
}

/// App-scoped window registry + liveness leases. A GPUI [`gpui::Global`].
pub struct WindowManager {
    registry: registry::Registry<AnyWindowHandle>,
    /// Per-window liveness leases, dropped on deregistration so the exit policy
    /// re-evaluates. Overlays are intentionally absent (they take no lease).
    holds: HashMap<AnyWindowHandle, ShellHold>,
    /// The global window-closed observer that drives [`WindowManager::reconcile`].
    /// Held here so it lives as long as the manager.
    close_observer: Option<Subscription>,
}

impl gpui::Global for WindowManager {}

impl Default for WindowManager {
    fn default() -> Self {
        Self::new()
    }
}

impl WindowManager {
    /// A new, empty manager.
    pub fn new() -> Self {
        Self {
            registry: registry::Registry::new(),
            holds: HashMap::new(),
            close_observer: None,
        }
    }

    /// Open a `Root`-wrapped window from `spec`, building the content view with
    /// `build`. Returns the `Root` handle and the content entity.
    pub fn open<V: 'static + Render>(
        cx: &mut App,
        mut spec: WindowSpec,
        build: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
    ) -> Result<OpenedWindow<V>, WindowError> {
        validate_root_policy(&spec, RootPolicy::ComponentRoot)?;
        let key = spec.key();
        let base = base_title(cx, &spec);
        let (number, title) = cx.global_mut::<WindowManager>().registry.allocate(&base);

        let options = window_options(cx, &mut spec, &title);

        let post_open = spec.post_open;
        let mut content_slot: Option<Entity<V>> = None;
        let opened = cx.open_window(options, |window, cx| {
            if let Some(post_open) = post_open {
                post_open(window, cx);
            }
            let content = build(window, cx);
            content_slot = Some(content.clone());
            cx.new(|cx| Root::new(content, window, cx))
        });

        let window = match opened {
            Ok(window) => window,
            Err(err) => {
                cx.global_mut::<WindowManager>()
                    .registry
                    .release(&base, number);
                return Err(WindowError::OpenFailed(err));
            }
        };

        let content = content_slot.expect("build_root_view ran");
        let handle: AnyWindowHandle = window.into();
        register_window(cx, handle, key, base, number, title);

        Ok(OpenedWindow { window, content })
    }

    /// Open a keyed singleton window: reuse and focus a live one, no-op while a
    /// create is in flight, otherwise create. Stale handles (window gone without
    /// a clean deregister) are treated as closed and recreated.
    pub fn open_singleton<V: 'static + Render>(
        cx: &mut App,
        spec: WindowSpec,
        build: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
    ) -> Result<Singleton<V>, WindowError> {
        validate_root_policy(&spec, RootPolicy::ComponentRoot)?;
        let key = spec.key();
        let phase = cx.global::<WindowManager>().registry.singleton_phase(key);

        // Probe liveness only when we think a window is open (focusing it also
        // satisfies the reuse case).
        let alive = match phase {
            SingletonPhase::Open(handle) => handle
                .update(cx, |_, window, _| window.activate_window())
                .is_ok(),
            _ => false,
        };

        match registry::plan_singleton(phase, alive) {
            registry::SingletonAction::InFlight => {
                validate_singleton_metadata::<V>(cx, key, spec.declared_root_policy())?;
                return Ok(Singleton::InFlight);
            }
            registry::SingletonAction::Reuse(_) => {
                validate_singleton_metadata::<V>(cx, key, spec.declared_root_policy())?;
                return Ok(Singleton::Reused);
            }
            registry::SingletonAction::Create => {
                // Clear any stale record before recreating.
                if let SingletonPhase::Open(handle) = phase {
                    deregister(cx, handle);
                }
            }
        }

        // Enter `Opening` before the (synchronous) build so a reentrant open for
        // this key sees the in-flight state.
        cx.global_mut::<WindowManager>()
            .registry
            .begin_singleton(key, SingletonMetadata::of::<V>(spec.declared_root_policy()));

        match Self::open(cx, spec, build) {
            Ok(opened) => {
                let handle: AnyWindowHandle = opened.window.into();
                cx.global_mut::<WindowManager>()
                    .registry
                    .finish_singleton(key, handle);
                Ok(Singleton::Opened(opened))
            }
            Err(err) => {
                cx.global_mut::<WindowManager>()
                    .registry
                    .clear_singleton(key);
                Err(err)
            }
        }
    }

    /// Open a window whose content view *is* the root (no `Root` wrapper). For
    /// [`RootPolicy::Raw`] specs.
    pub fn open_raw<V: 'static + Render>(
        cx: &mut App,
        mut spec: WindowSpec,
        build: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
    ) -> Result<WindowHandle<V>, WindowError> {
        validate_root_policy(&spec, RootPolicy::Raw)?;
        let key = spec.key();
        let base = base_title(cx, &spec);
        let (number, title) = cx.global_mut::<WindowManager>().registry.allocate(&base);

        let options = window_options(cx, &mut spec, &title);

        let post_open = spec.post_open;
        let opened = cx.open_window(options, |window, cx| {
            if let Some(post_open) = post_open {
                post_open(window, cx);
            }
            build(window, cx)
        });

        let window = match opened {
            Ok(window) => window,
            Err(err) => {
                cx.global_mut::<WindowManager>()
                    .registry
                    .release(&base, number);
                return Err(WindowError::OpenFailed(err));
            }
        };

        let handle: AnyWindowHandle = window.into();
        register_window(cx, handle, key, base, number, title);

        Ok(window)
    }

    /// Open a capability-gated overlay surface (not `Root`-wrapped, not numbered,
    /// no liveness lease). Returns [`WindowError::Unsupported`] where the
    /// platform reports overlays unavailable.
    pub fn open_overlay<V: 'static + Render>(
        cx: &mut App,
        spec: OverlaySpec,
        build: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
    ) -> Result<WindowHandle<V>, WindowError> {
        if let Capability::Unsupported { reason } = cx.app_info().capabilities().overlay_surface {
            return Err(WindowError::Unsupported { reason });
        }

        let key = spec.key();
        let app_id = cx.app_info().app_id().to_string();
        let mut options = gpui::OverlaySurfaceOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None, spec.size, cx,
            ))),
            display_id: cx.primary_display().map(|display| display.id()),
            show: true,
            focus: spec.focus,
            window_background: spec.background,
            app_id: Some(app_id),
            ..Default::default()
        };
        if let Some(customize) = spec.customize {
            customize(&mut options);
        }

        let window = cx
            .open_overlay_surface(options, |window, cx| build(window, cx))
            .map_err(WindowError::OpenFailed)?;
        let handle: AnyWindowHandle = window.into();
        cx.global_mut::<WindowManager>().registry.insert(
            handle,
            WindowRecord {
                key,
                base_title: key.as_str().to_string(),
                number: 0,
                title: key.as_str().to_string(),
                kind: SurfaceKind::Overlay,
            },
        );

        Ok(window)
    }

    /// Number of open real windows tracked by the manager (excludes overlays).
    pub fn window_count(&self) -> usize {
        self.registry.window_count()
    }

    /// Number of open overlays.
    pub fn overlay_count(&self) -> usize {
        self.registry.overlay_count()
    }

    /// A monotonic version bumped on every registration change. Menu rebuilds
    /// (Move-to-Window) observe this to know when to re-project the window list
    /// — the minimal observation seam (no callback registry).
    pub fn version(&self) -> u64 {
        self.registry.version()
    }

    /// Snapshot of open real windows as `(handle, key, number, title)`, sorted
    /// by number — the Move-to-Window menu projection.
    pub fn windows(&self) -> Vec<(AnyWindowHandle, WindowKey, u32, String)> {
        let mut windows: Vec<_> = self
            .registry
            .iter()
            .filter(|(_, record)| record.kind == SurfaceKind::Window)
            .map(|(handle, record)| (*handle, record.key, record.number, record.title.clone()))
            .collect();
        windows.sort_by_key(|(_, _, number, _)| *number);
        windows
    }
}

/// Extension trait for read access to the window manager on a raw `App`.
///
/// Opening windows is done through the `WindowManager::open*` associated
/// functions (they need `&mut App` re-entrantly, which an `&WindowManager`
/// borrow cannot provide).
pub trait AppWindowsExt {
    /// The installed window manager, if [`WindowsPlugin`] initialized.
    fn window_manager(&self) -> &WindowManager;
}

impl AppWindowsExt for App {
    fn window_manager(&self) -> &WindowManager {
        self.global::<WindowManager>()
    }
}

/// Register a freshly opened real window: insert its record and take a liveness
/// lease keyed by the handle.
fn register_window(
    cx: &mut App,
    handle: AnyWindowHandle,
    key: WindowKey,
    base_title: String,
    number: u32,
    title: String,
) {
    let hold = cx.shell().hold(WINDOW_HOLD_REASON);
    let manager = cx.global_mut::<WindowManager>();
    manager.registry.insert(
        handle,
        WindowRecord {
            key,
            base_title,
            number,
            title,
            kind: SurfaceKind::Window,
        },
    );
    manager.holds.insert(handle, hold);
}

/// Reconcile the registry against the live window set — the primary
/// deregistration trigger, driven by the shell's global `on_window_closed`
/// observer. Any registered surface no longer present in [`App::windows`] has
/// closed (regardless of whether the app still retains its root/content entity)
/// and is deregistered. This is what makes cleanup robust for `open_raw`, where
/// the root *is* the app's view and an entity-release hook would never fire.
fn reconcile(cx: &mut App) {
    if !cx.has_global::<WindowManager>() {
        return;
    }
    let live: HashSet<AnyWindowHandle> = cx.windows().into_iter().collect();
    let closed = cx
        .global::<WindowManager>()
        .registry
        .closed_handles(|handle| live.contains(handle));
    for handle in closed {
        deregister(cx, handle);
    }
}

/// Deregister a surface: drop its liveness lease (re-evaluating exit), remove its
/// record (freeing the number), and reset the singleton phase if this handle was
/// the tracked singleton. Idempotent and shutdown-safe.
fn deregister(cx: &mut App, handle: AnyWindowHandle) {
    if !cx.has_global::<WindowManager>() {
        return;
    }
    let removed = {
        let manager = cx.global_mut::<WindowManager>();
        // Dropping the hold here schedules an exit re-evaluation via the proxy.
        manager.holds.remove(&handle);
        manager.registry.remove(&handle)
    };
    if let Some(record) = removed {
        let manager = cx.global_mut::<WindowManager>();
        if let SingletonPhase::Open(open_handle) = manager.registry.singleton_phase(record.key) {
            if open_handle == handle {
                manager.registry.clear_singleton(record.key);
            }
        }
    }
}

fn validate_root_policy(spec: &WindowSpec, expected: RootPolicy) -> Result<(), WindowError> {
    let actual = spec.declared_root_policy();
    if actual == expected {
        Ok(())
    } else {
        Err(WindowError::RootPolicyMismatch {
            key: spec.key(),
            expected,
            actual,
        })
    }
}

fn validate_singleton_metadata<V: 'static>(
    cx: &App,
    key: WindowKey,
    root_policy: RootPolicy,
) -> Result<(), WindowError> {
    let metadata = cx
        .global::<WindowManager>()
        .registry
        .singleton_metadata(key)
        .expect("live or in-flight singleton has metadata");
    validate_singleton_contract::<V>(key, metadata, root_policy)
}

fn validate_singleton_contract<V: 'static>(
    key: WindowKey,
    metadata: SingletonMetadata,
    root_policy: RootPolicy,
) -> Result<(), WindowError> {
    if metadata.content_type != std::any::TypeId::of::<V>() {
        return Err(WindowError::ContentTypeMismatch {
            key,
            expected: metadata.content_type_name,
            actual: std::any::type_name::<V>(),
        });
    }
    if metadata.root_policy != root_policy {
        return Err(WindowError::RootPolicyMismatch {
            key,
            expected: metadata.root_policy,
            actual: root_policy,
        });
    }
    Ok(())
}

fn window_options(cx: &App, spec: &mut WindowSpec, title: &str) -> gpui::WindowOptions {
    let mut options = spec::base_window_options(title, spec.background);
    options.window_bounds = Some(centered_bounds(spec.size, cx));
    if let Some(pre_show) = spec.pre_show.take() {
        pre_show(&mut options);
    }
    spec::apply_app_id(&mut options, spec.resolved_app_id(cx.app_info().app_id()));
    options
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_policy_mismatches_are_typed_errors() {
        let error = validate_root_policy(&WindowSpec::new("main").raw(), RootPolicy::ComponentRoot)
            .unwrap_err();
        assert!(matches!(
            error,
            WindowError::RootPolicyMismatch {
                key,
                expected: RootPolicy::ComponentRoot,
                actual: RootPolicy::Raw,
            } if key == WindowKey::new("main")
        ));
    }

    #[test]
    fn singleton_content_type_mismatch_is_typed() {
        let error = validate_singleton_contract::<String>(
            WindowKey::new("settings"),
            SingletonMetadata::of::<u32>(RootPolicy::ComponentRoot),
            RootPolicy::ComponentRoot,
        )
        .unwrap_err();
        assert!(matches!(
            error,
            WindowError::ContentTypeMismatch { key, .. } if key == WindowKey::new("settings")
        ));
    }

    #[test]
    fn singleton_root_policy_mismatch_is_typed() {
        let error = validate_singleton_contract::<String>(
            WindowKey::new("settings"),
            SingletonMetadata::of::<String>(RootPolicy::Raw),
            RootPolicy::ComponentRoot,
        )
        .unwrap_err();
        assert!(matches!(
            error,
            WindowError::RootPolicyMismatch {
                key,
                expected: RootPolicy::Raw,
                actual: RootPolicy::ComponentRoot,
            } if key == WindowKey::new("settings")
        ));
    }
}

/// The un-numbered base title: the spec's title, or the app display name.
fn base_title(cx: &App, spec: &WindowSpec) -> String {
    spec.title
        .clone()
        .unwrap_or_else(|| cx.app_info().display_name().to_string())
}

/// Compute centered window bounds for a [`WindowSize`] on the active display.
fn centered_bounds(size: WindowSize, cx: &App) -> WindowBounds {
    let logical = match size {
        WindowSize::Fixed(size) => size,
        WindowSize::DisplayFraction(fraction) => {
            let display = cx
                .primary_display()
                .map(|display| display.bounds().size)
                .unwrap_or_else(|| gpui::size(gpui::px(1024.0), gpui::px(768.0)));
            spec::scale_size(display, fraction)
        }
    };
    WindowBounds::Windowed(Bounds::centered(None, logical, cx))
}

/// Register global handlers for the standard window-scoped actions from
/// [`crate::commands`] (the menu/keymap register these; the window manager owns
/// their behavior). Each operates on the platform-active window.
///
/// All three map to real fork window ops: `remove_window`, `minimize_window`,
/// `zoom_window`. `CloseWindow` (Cmd-W) is cross-platform; `Minimize`/`Zoom`
/// (Cmd-M / green button) are macOS-only actions.
fn register_window_action_handlers(cx: &mut App) {
    cx.on_action(|_: &CloseWindow, cx: &mut App| {
        if let Some(handle) = cx.active_window() {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        }
    });

    #[cfg(target_os = "macos")]
    {
        cx.on_action(|_: &Minimize, cx: &mut App| {
            if let Some(handle) = cx.active_window() {
                let _ = handle.update(cx, |_, window, _| window.minimize_window());
            }
        });
        cx.on_action(|_: &Zoom, cx: &mut App| {
            if let Some(handle) = cx.active_window() {
                let _ = handle.update(cx, |_, window, _| window.zoom_window());
            }
        });
    }
}

/// Installs and tears down the [`WindowManager`] (plan §3, task P1.4).
///
/// `LastWindowClosed` emission is *not* handled here: the shell core already
/// emits it from its `on_window_closed` observer (see `handles::register_observers`).
/// This plugin only owns the manager's lifecycle.
#[derive(Default)]
pub struct WindowsPlugin;

impl WindowsPlugin {
    /// A new plugin instance.
    pub fn new() -> Self {
        Self
    }
}

impl sealed::Sealed for WindowsPlugin {}

impl AppPlugin for WindowsPlugin {
    fn init(&mut self, cx: &mut App, _shell: &ShellSeed) -> Result<(), AppShellError> {
        cx.set_global(WindowManager::new());
        // Deregistration is driven by actual window closure: reconcile the
        // registry against the live window set whenever any window closes.
        let observer = cx.on_window_closed(reconcile);
        cx.global_mut::<WindowManager>().close_observer = Some(observer);
        register_window_action_handlers(cx);
        Ok(())
    }

    fn shutdown(&mut self, cx: &mut App) {
        if !cx.has_global::<ShellState>() || !cx.has_global::<WindowManager>() {
            return;
        }
        // Drop every liveness lease so a reverse shutdown does not leave the
        // exit policy thinking windows are still holding the app alive. The
        // platform tears the windows themselves down as the loop ends.
        let manager = cx.global_mut::<WindowManager>();
        manager.holds.clear();
    }
}
