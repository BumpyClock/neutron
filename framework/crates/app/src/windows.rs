//! Window management (plan §3 "Windows", promoted from agent-term + ansible).
//!
//! [`WindowManager`] is an app-scoped GPUI [`gpui::Global`] (installed by
//! [`WindowsModule`] at init — *not* a process `OnceLock`). It opens windows and
//! overlays, wraps them in `neutron_components::Root` by policy, numbers/titles them
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

mod about;
mod key;
mod registry;
mod spec;

use std::any::Any;
use std::collections::{HashMap, HashSet};

use gpui::{
    AnyWindowHandle, App, AppContext as _, Bounds, Context, Entity, Render, Subscription, Window,
    WindowBounds, WindowHandle,
};
use neutron_components::{Root, menu::AppMenuBar};

use crate::capabilities::Capability;
use crate::commands::standard::CloseWindow;
#[cfg(target_os = "macos")]
use crate::commands::standard::{Minimize, Zoom};
use crate::commands::{AppMenusExt as _, has_projected_menus};
use crate::declaration::{
    DeclaredSurface, SurfaceCardinality, SurfaceKey, SurfaceOptions, SurfaceRole,
};
use crate::error::AppShellError;
use crate::handles::{AppInfo, AppProxy, AppShellExt as _, ShellState};
use crate::liveness::ShellHold;
use crate::module::RuntimeModule;

pub(crate) use about::{AboutWindow, default_about_surface};
pub use key::WindowKey;
use registry::SingletonMetadata;
pub(crate) use registry::{SingletonPhase, SurfaceKind, WindowRecord};
pub use spec::RawWindow;
pub use spec::{OverlaySpec, WindowSize};
pub(crate) use spec::{RootPolicy, WindowSpec};

/// The liveness reason recorded for a window's lease.
const WINDOW_HOLD_REASON: &str = "window";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MenuSurface {
    #[cfg_attr(any(target_os = "windows", target_os = "linux"), allow(dead_code))]
    NativeGlobal,
    InWindow,
}

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
    #[error("window {key} has root policy {expected}, not requested policy {actual}")]
    RootPolicyMismatch {
        /// Stable window key.
        key: WindowKey,
        /// Root policy expected by the manager or registered singleton.
        expected: String,
        /// Root policy requested by the spec.
        actual: String,
    },

    /// An open was requested for a surface the application never declared.
    #[error("surface `{id}` is not declared")]
    UndeclaredSurface {
        /// The requested surface ID.
        id: &'static str,
    },

    /// A declared surface was requested with a content or argument type other
    /// than the one it was declared with.
    ///
    /// Unreachable through a typed [`crate::declaration::SurfaceKey`] on a
    /// validated declaration; reported rather than panicked so a future caller
    /// cannot turn a lookup mistake into a crash.
    #[error("surface `{id}` was declared as {expected}, not {actual}")]
    SurfaceTypeMismatch {
        /// The requested surface ID.
        id: &'static str,
        /// `View`/`Args` the surface was declared with.
        expected: &'static str,
        /// `View`/`Args` this open requested.
        actual: &'static str,
    },

    /// The window backing a surface handle has closed.
    #[error("the surface window is no longer open")]
    WindowClosed(#[source] anyhow::Error),

    /// A declared singleton surface names a live window that the
    /// declared-surface layer did not open, so no typed content is tracked for
    /// it. Reachable when a raw window ([`crate::Shell::open_raw`]) was opened
    /// under a declared surface's key.
    #[error("surface `{id}` names a window the shell did not open as a surface")]
    UntrackedSurfaceWindow {
        /// The requested surface ID.
        id: &'static str,
    },
}

/// A freshly opened, `Root`-wrapped window.
///
/// The auto-`Root`-wrap changes the window's root type, so the handle is typed
/// `WindowHandle<Root>` while the caller's content entity is returned alongside
/// it (plan §3 — "make that explicit").
pub(crate) struct OpenedWindow<V: 'static> {
    /// The window handle (root type is `Root`).
    pub window: WindowHandle<Root>,
    /// The caller's content view, wrapped inside the window's `Root`.
    pub content: Entity<V>,
}

/// Outcome of [`WindowManager::open_singleton`].
pub(crate) enum Singleton<V: 'static> {
    /// A new window was created.
    Opened(OpenedWindow<V>),
    /// A live window already existed and was focused; nothing new created.
    Reused,
    /// A create for this key is already in flight; nothing done.
    InFlight,
}

/// App-scoped window registry + liveness leases. A GPUI [`gpui::Global`].
pub(crate) struct WindowManager {
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
        spec: WindowSpec,
        build: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
    ) -> Result<OpenedWindow<V>, WindowError> {
        Self::open_for_menu_surface(cx, spec, build, managed_menu_surface())
    }

    fn open_for_menu_surface<V: 'static + Render>(
        cx: &mut App,
        mut spec: WindowSpec,
        build: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
        menu_surface: MenuSurface,
    ) -> Result<OpenedWindow<V>, WindowError> {
        validate_root_policy(&spec, RootPolicy::ComponentRoot)?;
        let key = spec.key();
        let base = base_title(cx, &spec);
        let scope = numbering_scope(&spec, &base);
        let (number, title) = cx
            .global_mut::<WindowManager>()
            .registry
            .allocate(&scope, &base);

        let options = window_options(cx, &mut spec, &title);

        let post_open = spec.post_open;
        let menu_bar_allowed = spec.menu_bar;
        let mut content_slot: Option<Entity<V>> = None;
        let opened = cx.open_window(options, |window, cx| {
            if let Some(post_open) = post_open {
                post_open(window, cx);
            }
            let app_menu_bar =
                should_attach_app_menu_bar(menu_surface, has_projected_menus(cx), menu_bar_allowed)
                    .then(|| cx.new_app_menu_bar());
            let content = build(window, cx);
            content_slot = Some(content.clone());
            cx.new(|cx| compose_managed_root(content, window, cx, app_menu_bar))
        });

        let window = match opened {
            Ok(window) => window,
            Err(err) => {
                cx.global_mut::<WindowManager>()
                    .registry
                    .release(&scope, number);
                return Err(WindowError::OpenFailed(err));
            }
        };

        let content = content_slot.expect("build_root_view ran");
        let handle: AnyWindowHandle = window.into();
        register_window(cx, handle, key, base, scope, number, title);

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
        Self::open_singleton_for_menu_surface(cx, spec, build, managed_menu_surface())
    }

    fn open_singleton_for_menu_surface<V: 'static + Render>(
        cx: &mut App,
        spec: WindowSpec,
        build: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
        menu_surface: MenuSurface,
    ) -> Result<Singleton<V>, WindowError> {
        validate_root_policy(&spec, RootPolicy::ComponentRoot)?;
        let key = spec.key();
        let phase = cx.global::<WindowManager>().registry.singleton_phase(key);

        // Probe liveness without focusing: a mismatched reuse must fail before
        // it raises somebody else's window. Focus is applied below, once the
        // registered contract has been checked.
        let alive = match phase {
            SingletonPhase::Open(handle) => handle.update(cx, |_, _, _| ()).is_ok(),
            _ => false,
        };

        match registry::plan_singleton(phase, alive) {
            registry::SingletonAction::InFlight => {
                validate_singleton_metadata::<V>(cx, key, spec.declared_root_policy())?;
                return Ok(Singleton::InFlight);
            }
            registry::SingletonAction::Reuse(handle) => {
                validate_singleton_metadata::<V>(cx, key, spec.declared_root_policy())?;
                // Reuse *is* focus: the window already exists, so raising it is
                // the whole operation.
                let _ = handle.update(cx, |_, window, _| window.activate_window());
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

        match Self::open_for_menu_surface(cx, spec, build, menu_surface) {
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
        let scope = numbering_scope(&spec, &base);
        let (number, title) = cx
            .global_mut::<WindowManager>()
            .registry
            .allocate(&scope, &base);

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
                    .release(&scope, number);
                return Err(WindowError::OpenFailed(err));
            }
        };

        let handle: AnyWindowHandle = window.into();
        register_window(cx, handle, key, base, scope, number, title);

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
                numbering_scope: key.as_str().to_string(),
                number: 0,
                title: key.as_str().to_string(),
                kind: SurfaceKind::Overlay,
            },
        );

        Ok(window)
    }

    /// Number of open real windows tracked by the manager (excludes overlays).
    #[allow(dead_code)] // Exercised only by unit tests; no production caller yet.
    pub fn window_count(&self) -> usize {
        self.registry.window_count()
    }

    /// Number of open overlays.
    #[allow(dead_code)] // Exercised only by unit tests; no production caller yet.
    pub fn overlay_count(&self) -> usize {
        self.registry.overlay_count()
    }

    /// A monotonic version bumped on every registration change. Menu rebuilds
    /// (Move-to-Window) observe this to know when to re-project the window list
    /// — the minimal observation seam (no callback registry).
    #[allow(dead_code)] // Exercised only by unit tests; no production caller yet.
    pub fn version(&self) -> u64 {
        self.registry.version()
    }

    /// Snapshot of open real windows as `(handle, key, number, title)`, sorted
    /// by number — the Move-to-Window menu projection.
    #[allow(dead_code)] // Exercised only by unit tests; no production caller yet.
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
#[allow(dead_code)] // Exercised only by unit tests; no production caller yet.
pub(crate) trait AppWindowsExt {
    /// The installed window manager, if [`WindowsModule`] initialized.
    fn window_manager(&self) -> &WindowManager;
}

impl AppWindowsExt for App {
    fn window_manager(&self) -> &WindowManager {
        self.global::<WindowManager>()
    }
}

/// A live declared-surface window: its content, its window, and its instance.
///
/// Cheap to clone regardless of `View`; the shell keeps one per open declared
/// window so a singleton reuse can hand the same typed content back.
pub struct SurfaceHandle<View: 'static> {
    window: WindowHandle<Root>,
    content: Entity<View>,
    instance: u32,
}

// Manual impl: deriving would require `View: Clone`.
impl<View: 'static> Clone for SurfaceHandle<View> {
    fn clone(&self) -> Self {
        Self {
            window: self.window,
            content: self.content.clone(),
            instance: self.instance,
        }
    }
}

impl<View: 'static> SurfaceHandle<View> {
    /// The content view, wrapped inside the window's `Root`.
    pub fn content(&self) -> &Entity<View> {
        &self.content
    }

    /// The window handle. `Root` is the window root, not `View`.
    pub fn window(&self) -> WindowHandle<Root> {
        self.window
    }

    /// The 1-based instance number that produced this window's title suffix.
    pub fn instance(&self) -> u32 {
        self.instance
    }

    /// Focus and raise this window.
    pub fn focus(&self, cx: &mut App) -> Result<(), WindowError> {
        self.window
            .update(cx, |_, window, _| window.activate_window())
            .map_err(WindowError::WindowClosed)
    }

    /// Ask the platform to close this window. Deregistration follows from the
    /// actual close, through the manager's window-closed reconciliation.
    pub fn close(&self, cx: &mut App) -> Result<(), WindowError> {
        self.window
            .update(cx, |_, window, _| window.remove_window())
            .map_err(WindowError::WindowClosed)
    }
}

/// The outcome of opening a declared surface.
pub enum SurfaceOpen<View: 'static> {
    /// A new window was created.
    Created(SurfaceHandle<View>),
    /// A live singleton already existed; it was focused and reused.
    Reused(SurfaceHandle<View>),
    /// A create for this singleton is already in flight; nothing was done.
    InFlight,
}

// Manual impls: deriving would require `View: Debug`, which no content view has
// to be. Both print the surface's identity, not its content.
impl<View: 'static> std::fmt::Debug for SurfaceHandle<View> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SurfaceHandle")
            .field("view", &std::any::type_name::<View>())
            .field("instance", &self.instance)
            .finish_non_exhaustive()
    }
}

impl<View: 'static> std::fmt::Debug for SurfaceOpen<View> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Created(handle) => f.debug_tuple("Created").field(handle).finish(),
            Self::Reused(handle) => f.debug_tuple("Reused").field(handle).finish(),
            Self::InFlight => f.write_str("InFlight"),
        }
    }
}

/// The declared surfaces of the running application, installed by lowering.
///
/// App-scoped GPUI [`gpui::Global`]. Private on purpose: [`crate::Shell::open_surface`]
/// is the public entry point; [`open_surface`] is the internal seam it wraps.
pub(crate) struct DeclaredSurfaces {
    by_id: HashMap<&'static str, DeclaredSurface>,
    /// Typed [`SurfaceHandle`] per open declared window, erased. Cleared with
    /// the window record so a closed window cannot be reused.
    live: HashMap<AnyWindowHandle, Box<dyn Any>>,
}

impl gpui::Global for DeclaredSurfaces {}

impl DeclaredSurfaces {
    fn new() -> Self {
        Self {
            by_id: HashMap::new(),
            live: HashMap::new(),
        }
    }

    /// The declaration for `id`, if the application declared it.
    pub(crate) fn get(&self, id: &str) -> Option<&DeclaredSurface> {
        self.by_id.get(id)
    }

    /// Number of declared surfaces.
    #[allow(dead_code)] // Exercised only by unit tests; no production caller yet.
    pub(crate) fn len(&self) -> usize {
        self.by_id.len()
    }

    /// Install one declared surface, refusing to shadow an existing ID.
    ///
    /// [`crate::declaration::AppDeclaration::validate`] is the primary check and
    /// reports duplicate IDs as declaration faults before anything is lowered;
    /// this is the defensive backstop, so a surface installed by some other path
    /// can never silently replace a live surface's factory and hooks.
    fn install(&mut self, surface: DeclaredSurface) -> Result<(), AppShellError> {
        let id = surface.id();
        if self.by_id.contains_key(id) {
            return Err(AppShellError::Module {
                module: "surface",
                source: anyhow::anyhow!("surface `{id}` is already declared"),
            });
        }
        self.by_id.insert(id, surface);
        Ok(())
    }
}

/// Whether any declared surface window (primary, auxiliary, Settings, or
/// About) is currently live.
///
/// Raw windows and platform overlays are runtime escapes, not declarable
/// surfaces (see the module docs), so they are never tracked in
/// [`DeclaredSurfaces::live`] and never count here. Used by the shell's
/// `Reopened` handling to decide whether restoring the primary would
/// duplicate a surface the application already has open.
pub(crate) fn any_declared_surface_live(cx: &App) -> bool {
    cx.try_global::<DeclaredSurfaces>()
        .is_some_and(|surfaces| !surfaces.live.is_empty())
}

/// The runtime module that installs one declared surface.
///
/// One module per surface keeps declaration order: modules initialize in the
/// order the declaration produced them, and the first to run creates the global.
pub(crate) fn declared_surface_module(surface: DeclaredSurface) -> impl RuntimeModule {
    DeclaredSurfaceModule {
        id: surface.id(),
        surface: Some(surface),
    }
}

struct DeclaredSurfaceModule {
    id: &'static str,
    surface: Option<DeclaredSurface>,
}

impl RuntimeModule for DeclaredSurfaceModule {
    fn id(&self) -> &'static str {
        self.id
    }

    fn init(
        &mut self,
        cx: &mut App,
        _info: &AppInfo,
        _proxy: &AppProxy,
    ) -> Result<(), AppShellError> {
        let surface = self
            .surface
            .take()
            .expect("a declared-surface module initializes once");
        if !cx.has_global::<DeclaredSurfaces>() {
            cx.set_global(DeclaredSurfaces::new());
        }
        cx.global_mut::<DeclaredSurfaces>().install(surface)
    }
}

/// Open a declared surface by its typed key.
///
/// Singleton surfaces focus and reuse a live window (running `on_reuse` with the
/// new arguments when declared), no-op while a create is in flight, and
/// otherwise create. `multiple()` surfaces always create a numbered instance.
pub(crate) fn open_surface<View: 'static + Render, Args: 'static>(
    cx: &mut App,
    key: SurfaceKey<View, Args>,
    args: &Args,
) -> Result<SurfaceOpen<View>, WindowError> {
    let id = key.id();
    let (role, options, hooks) = {
        let surfaces = cx
            .try_global::<DeclaredSurfaces>()
            .ok_or(WindowError::UndeclaredSurface { id })?;
        let declared = surfaces
            .get(id)
            .ok_or(WindowError::UndeclaredSurface { id })?;
        let hooks = declared
            .hooks::<View, Args>()
            .ok_or_else(|| surface_type_mismatch::<View, Args>(declared))?;
        (declared.role(), declared.options().clone(), hooks)
    };

    let spec = surface_spec(id, &options, role, hooks.configure_window);
    let build = hooks.build;
    if options.cardinality == SurfaceCardinality::Multiple {
        let opened = WindowManager::open(cx, spec, |window, cx| build(args, window, cx))?;
        return Ok(SurfaceOpen::Created(track_open(
            cx,
            opened,
            hooks.after_open,
        )));
    }

    // A live singleton this layer never opened has no typed content to hand
    // back. Fail before `open_singleton` focuses it, so a reuse that cannot
    // succeed does not raise a window either.
    let tracked = tracked_surface_handle::<View>(cx, id)?;

    match WindowManager::open_singleton(cx, spec, |window, cx| build(args, window, cx))? {
        Singleton::Opened(opened) => Ok(SurfaceOpen::Created(track_open(
            cx,
            opened,
            hooks.after_open,
        ))),
        Singleton::Reused => {
            let handle = tracked.ok_or(WindowError::UntrackedSurfaceWindow { id })?;
            if let Some(on_reuse) = hooks.on_reuse {
                let content = handle.content.clone();
                // Untyped update: the hook owns the content entity, which must
                // not be leased by the update that delivers it.
                AnyWindowHandle::from(handle.window)
                    .update(cx, |_, window, cx| on_reuse(&content, args, window, cx))
                    .map_err(WindowError::WindowClosed)?;
            }
            Ok(SurfaceOpen::Reused(handle))
        }
        Singleton::InFlight => Ok(SurfaceOpen::InFlight),
    }
}

/// Open one standard unit-argument surface for a framework command handler.
///
/// The typed surface openers below monomorphize this per content view, which is
/// what lets the standard About and Settings commands route to a surface whose
/// view type is only known at the declaration site.
fn open_standard_surface<View: 'static + Render>(
    cx: &mut App,
    key: SurfaceKey<View, ()>,
) -> anyhow::Result<()> {
    open_surface(cx, key, &())?;
    Ok(())
}

/// A plain-function opener for the About surface backed by `View`.
fn open_about_surface<View: 'static + Render>(cx: &mut App) -> anyhow::Result<()> {
    open_standard_surface(cx, SurfaceKey::<View>::about())
}

/// A plain-function opener for the Settings surface backed by `View`.
fn open_settings_surface<View: 'static + Render>(cx: &mut App) -> anyhow::Result<()> {
    open_standard_surface(cx, SurfaceKey::<View>::settings())
}

/// The opener the standard About command routes to for an application-supplied
/// About surface.
pub(crate) fn about_opener<View: 'static + Render>() -> fn(&mut App) -> anyhow::Result<()> {
    open_about_surface::<View>
}

/// The opener the standard About command routes to for the framework's own
/// About surface.
pub(crate) fn default_about_opener() -> fn(&mut App) -> anyhow::Result<()> {
    open_about_surface::<AboutWindow>
}

/// The opener the standard Settings command routes to.
pub(crate) fn settings_opener<View: 'static + Render>() -> fn(&mut App) -> anyhow::Result<()> {
    open_settings_surface::<View>
}

/// Open a typed raw window: the content view is the window root, with no
/// `Root` wrapper, no framework chrome, and no overlay composition.
pub(crate) fn open_raw_window<View: 'static + Render, Args: 'static>(
    cx: &mut App,
    raw: &RawWindow<View, Args>,
    args: &Args,
) -> Result<WindowHandle<View>, WindowError> {
    let build = raw.build();
    let mut content_slot = None;
    let window = WindowManager::open_raw(cx, raw.spec(), |window, cx| {
        let content = build(args, window, cx);
        content_slot = Some(content.clone());
        content
    })?;
    if let Some(after_open) = raw.after_open_hook() {
        let content = content_slot.expect("the raw window's root view was built");
        // Untyped update: a raw window's root *is* the content, so a typed
        // update would lease the very entity the hook is handed.
        AnyWindowHandle::from(window)
            .update(cx, |_, window, cx| after_open(&content, window, cx))
            .map_err(WindowError::WindowClosed)?;
    }
    Ok(window)
}

/// The window spec a declared surface lowers to.
fn surface_spec(
    id: &'static str,
    options: &SurfaceOptions,
    role: SurfaceRole,
    configure_window: Option<fn(&mut gpui::WindowOptions)>,
) -> WindowSpec {
    let (min_size, decorations) = (options.min_size, options.decorations);
    let mut spec = WindowSpec::new(WindowKey::new(id))
        .size(options.size)
        .background(options.background)
        .menu_bar(options.menu_bar_for(role))
        .numbering_scope(WindowKey::new(id))
        .customize_options(move |window_options| {
            spec::apply_window_options(window_options, min_size, decorations, configure_window);
        });
    if let Some(title) = &options.title {
        spec = spec.title(title.to_string());
    }
    spec
}

/// Record a freshly opened declared window and run its `after_open` hook.
fn track_open<View: 'static>(
    cx: &mut App,
    opened: OpenedWindow<View>,
    after_open: Option<fn(&Entity<View>, &mut Window, &mut App)>,
) -> SurfaceHandle<View> {
    let handle: AnyWindowHandle = opened.window.into();
    let instance = cx
        .global::<WindowManager>()
        .registry
        .record(&handle)
        .map_or(1, |record| record.number);
    let surface = SurfaceHandle {
        window: opened.window,
        content: opened.content,
        instance,
    };
    cx.global_mut::<DeclaredSurfaces>()
        .live
        .insert(handle, Box::new(surface.clone()));
    if let Some(after_open) = after_open {
        let content = surface.content.clone();
        // A window that vanished between opening and this hook is not an error
        // the caller can act on; the create itself succeeded. Untyped, so the
        // hook may update the content it is handed.
        let _ = AnyWindowHandle::from(surface.window)
            .update(cx, |_, window, cx| after_open(&content, window, cx));
    }
    surface
}

/// The typed handle tracked for the singleton `id`, checked before an open that
/// might reuse it.
///
/// `Ok(None)` means no window is registered for the key yet — the open will
/// create one. `Err` means a window *is* registered but this layer did not open
/// it, so no reuse of it could ever produce typed content.
fn tracked_surface_handle<View: 'static>(
    cx: &App,
    id: &'static str,
) -> Result<Option<SurfaceHandle<View>>, WindowError> {
    let SingletonPhase::Open(handle) = cx
        .global::<WindowManager>()
        .registry
        .singleton_phase(WindowKey::new(id))
    else {
        return Ok(None);
    };
    cx.try_global::<DeclaredSurfaces>()
        .and_then(|surfaces| surfaces.live.get(&handle))
        .and_then(|surface| surface.downcast_ref::<SurfaceHandle<View>>())
        .cloned()
        .map(Some)
        .ok_or(WindowError::UntrackedSurfaceWindow { id })
}

/// Which of `View`/`Args` diverged from the declaration.
fn surface_type_mismatch<View: 'static, Args: 'static>(declared: &DeclaredSurface) -> WindowError {
    let types = declared.types();
    let (expected, actual) = if types.view != std::any::TypeId::of::<View>() {
        (types.view_name, std::any::type_name::<View>())
    } else {
        (types.args_name, std::any::type_name::<Args>())
    };
    WindowError::SurfaceTypeMismatch {
        id: declared.id(),
        expected,
        actual,
    }
}

/// Register a freshly opened real window: insert its record and take a liveness
/// lease keyed by the handle.
fn register_window(
    cx: &mut App,
    handle: AnyWindowHandle,
    key: WindowKey,
    base_title: String,
    numbering_scope: String,
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
            numbering_scope,
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
    if cx.has_global::<DeclaredSurfaces>() {
        // Drop the typed content before the record, so a reuse can never hand
        // back a handle to a window that is gone.
        cx.global_mut::<DeclaredSurfaces>().live.remove(&handle);
    }
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
            expected: format!("{expected:?}"),
            actual: format!("{actual:?}"),
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
            expected: format!("{:?}", metadata.root_policy),
            actual: format!("{root_policy:?}"),
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

fn managed_menu_surface() -> MenuSurface {
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    {
        MenuSurface::InWindow
    }

    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        MenuSurface::NativeGlobal
    }
}

fn compose_managed_root<V: 'static + Render>(
    content: Entity<V>,
    window: &mut Window,
    cx: &mut Context<Root>,
    app_menu_bar: Option<Entity<AppMenuBar>>,
) -> Root {
    if let Some(app_menu_bar) = app_menu_bar {
        Root::new(content, window, cx).with_app_menu_bar(app_menu_bar)
    } else {
        Root::new(content, window, cx)
    }
}

/// Whether a window gets in-window menu chrome.
///
/// Three independent conditions, all required: the platform composes menus
/// in-window rather than natively, the application actually projected a
/// non-empty menu plan (an empty projection never creates an inert bar), and the
/// surface itself allows menu chrome (Settings/About opt out by role).
fn should_attach_app_menu_bar(
    menu_surface: MenuSurface,
    menus_projected: bool,
    surface_allows: bool,
) -> bool {
    menu_surface == MenuSurface::InWindow && menus_projected && surface_allows
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use gpui::{Empty, Role, SharedString, TestAppContext, VisualTestContext, actions};

    use crate::commands::{CommandId, CommandRegistry, CommandScope, MenuPlan, RuntimeCommand};
    use crate::declaration::Surface;
    use crate::handles::{self, PendingEvents};
    use crate::liveness::{ExitPolicy, InitialActivation, Liveness};
    use crate::{AppPaths, IdentityRef, PathLayout};

    actions!(window_menu_test, [MenuAction]);

    fn identity() -> IdentityRef {
        IdentityRef {
            app_id: "com.example.window-menu-test",
            display_name: "Window Menu Test",
            data_namespace: "window-menu-test",
            binary_name: None,
            org: None,
            publisher: None,
            url_schemes: &[],
            categories: &[],
            macos: None,
            linux: None,
            windows: None,
            legacy_ids: &[],
            min_os: None,
            version: "0.0.0",
            cfbundle_short_version: "0.0.0",
            msix_version: "0.0.0.0",
        }
    }

    fn initialize_window_manager(cx: &mut App) {
        neutron_components::init(cx);
        let info = AppInfo::new(
            identity(),
            AppPaths::new(
                "window-menu-test",
                PathLayout::SingleRoot("window-menu-test".to_string()),
            )
            .expect("test paths resolve"),
            crate::PlatformCapabilities::detect(),
        );
        let proxy = handles::install(
            cx,
            info.clone(),
            Liveness::new(ExitPolicy::Explicit, InitialActivation::Passive),
            Vec::new(),
            Vec::new(),
            Arc::new(PendingEvents::default()),
            Box::new(|_, _| {}),
            None,
        );
        WindowsModule::new()
            .init(cx, &info, &proxy)
            .expect("window manager initializes");
    }

    fn configure_nonempty_menus(cx: &mut App) {
        cx.set_global(CommandRegistry::new());
        let registry = cx.global_mut::<CommandRegistry>();
        registry
            .register(
                RuntimeCommand::new(
                    CommandId("window-menu-test.action"),
                    "Menu Action",
                    CommandScope::App,
                    MenuAction,
                )
                .placed("Test", 0, 0),
            )
            .expect("test command registers");
        registry.set_plan(MenuPlan::from_keys(["Test"]));
    }

    fn configure_empty_menus(cx: &mut App) {
        cx.set_global(CommandRegistry::new());
        cx.global_mut::<CommandRegistry>()
            .set_plan(MenuPlan::from_keys(["Test"]));
    }

    fn window_has_app_menu_bar(window: AnyWindowHandle, cx: &mut TestAppContext) -> bool {
        let mut visual_cx = VisualTestContext::from_window(window, cx);
        visual_cx.update(|window, cx| {
            window.set_a11y_active_for_test(true);
            window.draw(cx).clear(cx);
            let tree = window
                .last_a11y_tree_for_test()
                .cloned()
                .expect("accessibility tree captured");
            window.set_a11y_active_for_test(false);
            tree.nodes
                .iter()
                .any(|(_, node)| node.role() == Role::MenuBar)
        })
    }

    #[test]
    fn menu_surface_policy_attaches_bars_only_for_nonempty_in_window_menus() {
        assert!(should_attach_app_menu_bar(
            MenuSurface::InWindow,
            true,
            true
        ));
        assert!(!should_attach_app_menu_bar(
            MenuSurface::InWindow,
            false,
            true
        ));
        assert!(!should_attach_app_menu_bar(
            MenuSurface::NativeGlobal,
            true,
            true
        ));
        // A surface that opts out of menu chrome (Settings/About) never gets a
        // bar, even where the platform composes menus in-window.
        assert!(!should_attach_app_menu_bar(
            MenuSurface::InWindow,
            true,
            false
        ));
    }

    #[gpui::test]
    fn managed_open_attaches_app_menu_bar_for_nonempty_projected_menus(cx: &mut TestAppContext) {
        let window = cx.update(|cx| {
            initialize_window_manager(cx);
            configure_nonempty_menus(cx);
            WindowManager::open_for_menu_surface(
                cx,
                WindowSpec::new("main"),
                |_, cx| cx.new(|_| Empty),
                MenuSurface::InWindow,
            )
            .expect("managed window opens")
            .window
        });

        assert!(window_has_app_menu_bar(window.into(), cx));
    }

    #[gpui::test]
    fn managed_open_skips_app_menu_bar_without_nonempty_projected_menus(cx: &mut TestAppContext) {
        let (without_menus, with_empty_menus) = cx.update(|cx| {
            initialize_window_manager(cx);
            let without_menus = WindowManager::open_for_menu_surface(
                cx,
                WindowSpec::new("without-menus"),
                |_, cx| cx.new(|_| Empty),
                MenuSurface::InWindow,
            )
            .expect("managed window without menus opens")
            .window;

            configure_empty_menus(cx);
            let with_empty_menus = WindowManager::open_for_menu_surface(
                cx,
                WindowSpec::new("empty-menus"),
                |_, cx| cx.new(|_| Empty),
                MenuSurface::InWindow,
            )
            .expect("managed window with empty menus opens")
            .window;
            (without_menus, with_empty_menus)
        });

        assert!(!window_has_app_menu_bar(without_menus.into(), cx));
        assert!(!window_has_app_menu_bar(with_empty_menus.into(), cx));
    }

    #[gpui::test]
    fn raw_windows_do_not_get_app_menu_bars(cx: &mut TestAppContext) {
        let window = cx.update(|cx| {
            initialize_window_manager(cx);
            configure_nonempty_menus(cx);
            WindowManager::open_raw(cx, WindowSpec::new("raw").raw(), |_, cx| cx.new(|_| Empty))
                .expect("raw window opens")
        });

        assert!(!window_has_app_menu_bar(window.into(), cx));
    }

    #[gpui::test]
    fn managed_singleton_inherits_app_menu_bar_policy(cx: &mut TestAppContext) {
        let window = cx.update(|cx| {
            initialize_window_manager(cx);
            configure_nonempty_menus(cx);
            match WindowManager::open_singleton_for_menu_surface(
                cx,
                WindowSpec::new("singleton"),
                |_, cx| cx.new(|_| Empty),
                MenuSurface::InWindow,
            )
            .expect("singleton window opens")
            {
                Singleton::Opened(opened) => opened.window,
                Singleton::Reused | Singleton::InFlight => {
                    panic!("first singleton open must create")
                }
            }
        });

        assert!(window_has_app_menu_bar(window.into(), cx));
    }

    #[test]
    fn root_policy_mismatches_are_typed_errors() {
        let error = validate_root_policy(&WindowSpec::new("main").raw(), RootPolicy::ComponentRoot)
            .unwrap_err();
        assert!(matches!(
            error,
            WindowError::RootPolicyMismatch {
                key,
                ref expected,
                ref actual,
            } if key == WindowKey::new("main") && expected == "ComponentRoot" && actual == "Raw"
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
                ref expected,
                ref actual,
            } if key == WindowKey::new("settings") && expected == "Raw" && actual == "ComponentRoot"
        ));
    }

    // --- Declared surfaces -------------------------------------------------

    /// Content with observable state, so reuse can be told apart from a rebuild.
    struct Log {
        label: SharedString,
        reopens: usize,
    }

    impl Render for Log {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl gpui::IntoElement {
            Empty
        }
    }

    /// Typed open arguments, borrowed by the factory (issue #29).
    struct Filter {
        label: &'static str,
    }

    fn build_log(args: &Filter, _: &mut Window, cx: &mut App) -> Entity<Log> {
        let label = SharedString::from(args.label);
        cx.new(|_| Log { label, reopens: 0 })
    }

    fn note_reopen(content: &Entity<Log>, args: &Filter, _: &mut Window, cx: &mut App) {
        content.update(cx, |log, _| {
            log.label = SharedString::from(args.label);
            log.reopens += 1;
        });
    }

    fn declare(cx: &mut App, surface: DeclaredSurface) {
        let info = cx.app_info().clone();
        let proxy = cx.app_proxy();
        declared_surface_module(surface)
            .init(cx, &info, &proxy)
            .expect("declared surface installs");
    }

    fn log_surface(id: &'static str) -> Surface<Log, Filter> {
        Surface::new(SurfaceKey::new(id), build_log)
    }

    #[gpui::test]
    fn lowering_installs_declared_surfaces_in_declaration_order(cx: &mut TestAppContext) {
        cx.update(|cx| {
            initialize_window_manager(cx);
            declare(
                cx,
                DeclaredSurface::erase(log_surface("logs"), SurfaceRole::Auxiliary),
            );
            declare(
                cx,
                DeclaredSurface::erase(log_surface("audit"), SurfaceRole::Auxiliary),
            );

            let surfaces = cx.global::<DeclaredSurfaces>();
            assert_eq!(surfaces.len(), 2);
            assert_eq!(
                surfaces.get("logs").map(DeclaredSurface::role),
                Some(SurfaceRole::Auxiliary)
            );
            assert!(surfaces.get("missing").is_none());
        });
    }

    #[gpui::test]
    fn a_declared_singleton_surface_reuses_its_content(cx: &mut TestAppContext) {
        cx.update(|cx| {
            initialize_window_manager(cx);
            declare(
                cx,
                DeclaredSurface::erase(
                    log_surface("logs").on_reuse(note_reopen),
                    SurfaceRole::Auxiliary,
                ),
            );
            let key = SurfaceKey::<Log, Filter>::new("logs");

            let created = open_surface(cx, key, &Filter { label: "errors" })
                .expect("first open creates the surface");
            let SurfaceOpen::Created(first) = created else {
                panic!("the first open of a singleton surface must create");
            };
            assert_eq!(first.instance(), 1);

            let reopened = open_surface(cx, key, &Filter { label: "warnings" })
                .expect("second open reuses the surface");
            let SurfaceOpen::Reused(second) = reopened else {
                panic!("the second open of a live singleton surface must reuse");
            };

            // Same content entity, not a rebuilt one, with `on_reuse` applied.
            assert_eq!(second.content().entity_id(), first.content().entity_id());
            assert_eq!(second.window(), first.window());
            let log = second.content().read(cx);
            assert_eq!(log.label, SharedString::from("warnings"));
            assert_eq!(log.reopens, 1);
        });
    }

    #[gpui::test]
    fn a_declared_multiple_surface_numbers_its_instances(cx: &mut TestAppContext) {
        cx.update(|cx| {
            initialize_window_manager(cx);
            declare(
                cx,
                DeclaredSurface::erase(
                    log_surface("logs").title("Logs").multiple(),
                    SurfaceRole::Auxiliary,
                ),
            );
            let key = SurfaceKey::<Log, Filter>::new("logs");

            let SurfaceOpen::Created(first) =
                open_surface(cx, key, &Filter { label: "errors" }).expect("first instance opens")
            else {
                panic!("a multiple surface always creates");
            };
            let SurfaceOpen::Created(second) = open_surface(cx, key, &Filter { label: "warnings" })
                .expect("second instance opens")
            else {
                panic!("a multiple surface always creates");
            };

            assert_eq!(first.instance(), 1);
            assert_eq!(second.instance(), 2);
            assert_ne!(first.window(), second.window());
            assert_ne!(first.content().entity_id(), second.content().entity_id());

            let manager = cx.global::<WindowManager>();
            let titles: Vec<String> = [first.window(), second.window()]
                .iter()
                .map(|window| {
                    manager
                        .registry
                        .record(&AnyWindowHandle::from(*window))
                        .expect("instance is registered")
                        .title
                        .clone()
                })
                .collect();
            assert_eq!(titles, vec!["Logs".to_string(), "Logs - 2".to_string()]);
        });
    }

    #[gpui::test]
    fn a_declared_surface_wraps_its_content_in_root(cx: &mut TestAppContext) {
        cx.update(|cx| {
            initialize_window_manager(cx);
            declare(
                cx,
                DeclaredSurface::erase(log_surface("logs"), SurfaceRole::Auxiliary),
            );

            let SurfaceOpen::Created(handle) = open_surface(
                cx,
                SurfaceKey::<Log, Filter>::new("logs"),
                &Filter { label: "errors" },
            )
            .expect("surface opens") else {
                panic!("the first open creates");
            };

            // The window root is `Root`, and the content lives inside it.
            let root_kind = handle
                .window()
                .update(cx, |_: &mut Root, _, _| std::any::type_name::<Root>())
                .expect("the window is open");
            assert_eq!(root_kind, std::any::type_name::<Root>());
            assert_eq!(
                handle.content().read(cx).label,
                SharedString::from("errors")
            );
        });
    }

    #[gpui::test]
    fn an_in_flight_declared_singleton_open_is_reported_not_duplicated(cx: &mut TestAppContext) {
        cx.update(|cx| {
            initialize_window_manager(cx);
            declare(
                cx,
                DeclaredSurface::erase(log_surface("logs"), SurfaceRole::Auxiliary),
            );
            // Enter the phase a reentrant open would observe.
            cx.global_mut::<WindowManager>().registry.begin_singleton(
                WindowKey::new("logs"),
                SingletonMetadata::of::<Log>(RootPolicy::ComponentRoot),
            );

            let result = open_surface(
                cx,
                SurfaceKey::<Log, Filter>::new("logs"),
                &Filter { label: "errors" },
            )
            .expect("an in-flight open is not a fault");
            assert!(matches!(result, SurfaceOpen::InFlight));
            assert_eq!(cx.global::<WindowManager>().registry.window_count(), 0);
        });
    }

    #[gpui::test]
    fn opening_an_undeclared_surface_is_a_typed_fault(cx: &mut TestAppContext) {
        cx.update(|cx| {
            initialize_window_manager(cx);

            // No declarations at all: the global is absent, not merely empty.
            let error = open_surface(
                cx,
                SurfaceKey::<Log, Filter>::new("logs"),
                &Filter { label: "errors" },
            )
            .expect_err("an undeclared surface cannot open");
            assert!(matches!(
                error,
                WindowError::UndeclaredSurface { id: "logs" }
            ));

            declare(
                cx,
                DeclaredSurface::erase(log_surface("logs"), SurfaceRole::Auxiliary),
            );
            let error = open_surface(
                cx,
                SurfaceKey::<Log, Filter>::new("audit"),
                &Filter { label: "errors" },
            )
            .expect_err("an undeclared surface cannot open");
            assert!(matches!(
                error,
                WindowError::UndeclaredSurface { id: "audit" }
            ));
        });
    }

    #[gpui::test]
    fn opening_a_declared_surface_with_other_types_is_a_typed_fault(cx: &mut TestAppContext) {
        cx.update(|cx| {
            initialize_window_manager(cx);
            declare(
                cx,
                DeclaredSurface::erase(log_surface("logs"), SurfaceRole::Auxiliary),
            );

            // Same ID, different content type: reachable only by bypassing the
            // declared key, never from a validated declaration.
            let error = open_surface(
                cx,
                SurfaceKey::<Empty, Filter>::new("logs"),
                &Filter { label: "errors" },
            )
            .expect_err("a mismatched content type cannot open");
            let WindowError::SurfaceTypeMismatch {
                id,
                expected,
                actual,
            } = error
            else {
                panic!("a type mismatch is reported as such");
            };
            assert_eq!(id, "logs");
            assert_eq!(expected, std::any::type_name::<Log>());
            assert_eq!(actual, std::any::type_name::<Empty>());

            // Same content type, different argument type.
            let error = open_surface(cx, SurfaceKey::<Log, ()>::new("logs"), &())
                .expect_err("a mismatched argument type cannot open");
            assert!(matches!(
                error,
                WindowError::SurfaceTypeMismatch { expected, actual, .. }
                    if expected == std::any::type_name::<Filter>()
                        && actual == std::any::type_name::<()>()
            ));
        });
    }

    #[gpui::test]
    fn closing_a_declared_surface_releases_its_tracked_content(cx: &mut TestAppContext) {
        let window = cx.update(|cx| {
            initialize_window_manager(cx);
            declare(
                cx,
                DeclaredSurface::erase(log_surface("logs"), SurfaceRole::Auxiliary),
            );
            let SurfaceOpen::Created(handle) = open_surface(
                cx,
                SurfaceKey::<Log, Filter>::new("logs"),
                &Filter { label: "errors" },
            )
            .expect("surface opens") else {
                panic!("the first open creates");
            };
            handle.close(cx).expect("the surface window closes");
            handle.window()
        });

        cx.update(|cx| {
            assert!(cx.global::<DeclaredSurfaces>().live.is_empty());
            assert!(
                cx.global::<WindowManager>()
                    .registry
                    .record(&AnyWindowHandle::from(window))
                    .is_none()
            );
        });
    }

    #[test]
    fn a_declared_surface_lowers_its_role_menu_policy_into_its_spec() {
        let options = SurfaceOptions::default();
        assert!(surface_spec("logs", &options, SurfaceRole::Auxiliary, None).menu_bar);
        assert!(surface_spec("primary", &options, SurfaceRole::Primary, None).menu_bar);
        // Settings and About are chromeless by role.
        assert!(!surface_spec("settings", &options, SurfaceRole::Settings, None).menu_bar);
        assert!(!surface_spec("about", &options, SurfaceRole::About, None).menu_bar);

        // An explicit override wins over the role default, both ways.
        let forced = SurfaceOptions {
            menu_bar: Some(true),
            ..SurfaceOptions::default()
        };
        assert!(surface_spec("settings", &forced, SurfaceRole::Settings, None).menu_bar);
        let suppressed = SurfaceOptions {
            menu_bar: Some(false),
            ..SurfaceOptions::default()
        };
        assert!(!surface_spec("logs", &suppressed, SurfaceRole::Auxiliary, None).menu_bar);
    }

    /// The framework's default About surface, declared and opened exactly the
    /// way the resolved About feature does at runtime.
    #[gpui::test]
    fn the_default_about_surface_opens_and_reuses_its_singleton(cx: &mut TestAppContext) {
        cx.update(|cx| {
            initialize_window_manager(cx);
            declare(cx, default_about_surface());

            default_about_opener()(cx).expect("the default About opens");
            assert_eq!(cx.global::<WindowManager>().registry.window_count(), 1);

            // Reading the content back proves About renders the declared
            // identity rather than invented text.
            let SurfaceOpen::Reused(handle) =
                open_surface(cx, SurfaceKey::<about::AboutWindow>::about(), &())
                    .expect("the About surface is declared")
            else {
                panic!("a second open reuses the singleton");
            };
            let about = handle.content().read(cx);
            assert_eq!(about.display_name, SharedString::from("Window Menu Test"));
            assert_eq!(about.version, SharedString::from("0.0.0"));
            assert_eq!(
                about.app_id,
                SharedString::from("com.example.window-menu-test")
            );
            assert!(
                about.publisher.is_none(),
                "the test identity declares no publisher",
            );
            assert_eq!(
                cx.global::<WindowManager>().registry.window_count(),
                1,
                "About is a singleton",
            );
        });
    }

    /// A post-open hook that both reads and writes the content it is handed.
    ///
    /// This is the shape that panics if the update delivering the hook still
    /// leases the content entity.
    fn stamp_reopen(content: &Entity<Log>, _: &mut Window, cx: &mut App) {
        let label = content.read(cx).label.clone();
        content.update(cx, |log, _| {
            log.label = SharedString::from(format!("{label}!"));
            log.reopens += 1;
        });
    }

    #[gpui::test]
    fn a_raw_after_open_hook_may_update_the_content_it_is_handed(cx: &mut TestAppContext) {
        cx.update(|cx| {
            initialize_window_manager(cx);
            let raw = RawWindow::new("inspector", build_log).after_open(stamp_reopen);
            let window =
                open_raw_window(cx, &raw, &Filter { label: "errors" }).expect("raw window opens");

            // A raw window's content *is* its root, so the hook must not run
            // inside an update that leases it.
            let (label, reopens) = window
                .update(cx, |log: &mut Log, _, _| (log.label.clone(), log.reopens))
                .expect("the raw window is open");
            assert_eq!(label, SharedString::from("errors!"));
            assert_eq!(reopens, 1);
        });
    }

    #[gpui::test]
    fn a_declared_after_open_hook_may_update_the_content_it_is_handed(cx: &mut TestAppContext) {
        cx.update(|cx| {
            initialize_window_manager(cx);
            declare(
                cx,
                DeclaredSurface::erase(
                    log_surface("logs").after_open(stamp_reopen),
                    SurfaceRole::Auxiliary,
                ),
            );

            let SurfaceOpen::Created(handle) = open_surface(
                cx,
                SurfaceKey::<Log, Filter>::new("logs"),
                &Filter { label: "errors" },
            )
            .expect("surface opens") else {
                panic!("the first open creates");
            };

            let log = handle.content().read(cx);
            assert_eq!(log.label, SharedString::from("errors!"));
            assert_eq!(log.reopens, 1);
        });
    }

    #[gpui::test]
    fn untitled_declared_surfaces_number_per_key_not_per_title(cx: &mut TestAppContext) {
        cx.update(|cx| {
            initialize_window_manager(cx);
            declare(
                cx,
                DeclaredSurface::erase(log_surface("logs"), SurfaceRole::Auxiliary),
            );
            declare(
                cx,
                DeclaredSurface::erase(log_surface("audit"), SurfaceRole::Auxiliary),
            );

            let SurfaceOpen::Created(logs) = open_surface(
                cx,
                SurfaceKey::<Log, Filter>::new("logs"),
                &Filter { label: "errors" },
            )
            .expect("the first surface opens") else {
                panic!("the first open creates");
            };
            let SurfaceOpen::Created(audit) = open_surface(
                cx,
                SurfaceKey::<Log, Filter>::new("audit"),
                &Filter { label: "errors" },
            )
            .expect("the second surface opens") else {
                panic!("the first open creates");
            };

            // Both default to the app display name, and neither is "- 2":
            // distinct surfaces do not share a counter.
            assert_eq!(logs.instance(), 1);
            assert_eq!(audit.instance(), 1);
            let display_name = cx.app_info().display_name().to_string();
            assert_eq!(
                surface_title(cx, &logs),
                (display_name.clone(), display_name.clone())
            );
            assert_eq!(
                surface_title(cx, &audit),
                (display_name.clone(), display_name)
            );
        });
    }

    #[gpui::test]
    fn distinct_declared_surfaces_sharing_a_title_do_not_share_a_counter(cx: &mut TestAppContext) {
        cx.update(|cx| {
            initialize_window_manager(cx);
            declare(
                cx,
                DeclaredSurface::erase(log_surface("logs").title("Report"), SurfaceRole::Auxiliary),
            );
            declare(
                cx,
                DeclaredSurface::erase(
                    log_surface("audit").title("Report"),
                    SurfaceRole::Auxiliary,
                ),
            );

            let SurfaceOpen::Created(logs) = open_surface(
                cx,
                SurfaceKey::<Log, Filter>::new("logs"),
                &Filter { label: "errors" },
            )
            .expect("the first surface opens") else {
                panic!("the first open creates");
            };
            let SurfaceOpen::Created(audit) = open_surface(
                cx,
                SurfaceKey::<Log, Filter>::new("audit"),
                &Filter { label: "errors" },
            )
            .expect("the second surface opens") else {
                panic!("the first open creates");
            };

            assert_eq!(logs.instance(), 1);
            assert_eq!(audit.instance(), 1);
            assert_eq!(
                surface_title(cx, &logs),
                ("Report".to_string(), "Report".to_string())
            );
            assert_eq!(
                surface_title(cx, &audit),
                ("Report".to_string(), "Report".to_string())
            );
        });
    }

    /// The registered `(base_title, title)` of an open surface.
    fn surface_title(cx: &App, handle: &SurfaceHandle<Log>) -> (String, String) {
        let record = cx
            .global::<WindowManager>()
            .registry
            .record(&AnyWindowHandle::from(handle.window()))
            .expect("the surface is registered");
        (record.base_title.clone(), record.title.clone())
    }

    #[gpui::test]
    fn a_live_window_this_layer_did_not_open_is_never_reused_or_focused(cx: &mut TestAppContext) {
        cx.update(|cx| {
            initialize_window_manager(cx);
            declare(
                cx,
                DeclaredSurface::erase(log_surface("logs"), SurfaceRole::Auxiliary),
            );

            // A window registered under the same key by the untyped API: live,
            // but with no typed content this layer could hand back.
            let untracked = WindowManager::open_singleton(
                cx,
                WindowSpec::new(WindowKey::new("logs")),
                |window, cx| build_log(&Filter { label: "untracked" }, window, cx),
            )
            .expect("the untyped singleton opens");
            let Singleton::Opened(untracked) = untracked else {
                panic!("the first untyped open creates");
            };
            let before = cx.active_window();

            let error = open_surface(
                cx,
                SurfaceKey::<Log, Filter>::new("logs"),
                &Filter { label: "errors" },
            )
            .expect_err("an untracked window cannot be reused");
            assert!(matches!(
                error,
                WindowError::UntrackedSurfaceWindow { id: "logs" }
            ));

            // The failure raised no window and opened none either.
            assert_eq!(cx.active_window(), before);
            assert_eq!(cx.global::<WindowManager>().registry.window_count(), 1);
            assert!(
                cx.global::<WindowManager>()
                    .registry
                    .record(&AnyWindowHandle::from(untracked.window))
                    .is_some()
            );
        });
    }

    #[gpui::test]
    fn installing_a_second_surface_under_one_id_is_refused(cx: &mut TestAppContext) {
        cx.update(|cx| {
            initialize_window_manager(cx);
            declare(
                cx,
                DeclaredSurface::erase(log_surface("logs").title("First"), SurfaceRole::Auxiliary),
            );

            let info = cx.app_info().clone();
            let proxy = cx.app_proxy();
            let error = declared_surface_module(DeclaredSurface::erase(
                log_surface("logs").title("Second"),
                SurfaceRole::Auxiliary,
            ))
            .init(cx, &info, &proxy)
            .expect_err("a duplicate surface ID is refused");
            assert!(matches!(
                error,
                AppShellError::Module {
                    module: "surface",
                    ..
                }
            ));

            // The first declaration still owns the ID.
            assert_eq!(cx.global::<DeclaredSurfaces>().len(), 1);
            let SurfaceOpen::Created(handle) = open_surface(
                cx,
                SurfaceKey::<Log, Filter>::new("logs"),
                &Filter { label: "errors" },
            )
            .expect("the surviving surface opens") else {
                panic!("the first open creates");
            };
            assert_eq!(surface_title(cx, &handle).0, "First");
        });
    }

    #[gpui::test]
    fn a_typed_raw_window_is_its_own_root(cx: &mut TestAppContext) {
        let window = cx.update(|cx| {
            initialize_window_manager(cx);
            configure_nonempty_menus(cx);
            let raw = RawWindow::new("inspector", build_log).title("Inspector");
            open_raw_window(cx, &raw, &Filter { label: "errors" }).expect("raw window opens")
        });

        // No `Root`, so no framework chrome can be injected.
        assert!(!window_has_app_menu_bar(window.into(), cx));
        cx.update(|cx| {
            assert_eq!(
                window
                    .update(cx, |log: &mut Log, _, _| log.label.clone())
                    .expect("the raw window is open"),
                SharedString::from("errors")
            );
        });
    }
}

/// The counter a window draws its instance number from.
///
/// Typed surfaces and raw windows number per key, so two untitled surfaces are
/// each instance 1 instead of `"App"` and `"App - 2"`. Untyped opens keep the
/// historical base-title scope. The `surface:` prefix keeps a key from
/// colliding with a literal window title.
fn numbering_scope(spec: &WindowSpec, base: &str) -> String {
    match spec.numbering_scope {
        Some(key) => format!("surface:{key}"),
        None => base.to_string(),
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
    let display = cx.primary_display().map(|display| display.bounds().size);
    let logical = spec::resolve_window_size(size, display);
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
/// This module only owns the manager's lifecycle.
#[derive(Default)]
pub(crate) struct WindowsModule;

impl WindowsModule {
    /// A new module instance.
    pub(crate) fn new() -> Self {
        Self
    }
}

impl RuntimeModule for WindowsModule {
    fn id(&self) -> &'static str {
        "windows"
    }

    fn init(
        &mut self,
        cx: &mut App,
        _info: &AppInfo,
        _proxy: &AppProxy,
    ) -> Result<(), AppShellError> {
        cx.set_global(WindowManager::new());
        // Deregistration is driven by actual window closure: reconcile the
        // registry against the live window set whenever any window closes.
        let observer = cx.on_window_closed(reconcile);
        cx.global_mut::<WindowManager>().close_observer = Some(observer);
        register_window_action_handlers(cx);
        Ok(())
    }

    fn shutdown(&mut self, cx: &mut App) {
        if cx.has_global::<DeclaredSurfaces>() {
            // Release the content entities the declared-surface layer retained;
            // `WindowsModule` initializes first, so it tears down last.
            cx.global_mut::<DeclaredSurfaces>().live.clear();
        }
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
