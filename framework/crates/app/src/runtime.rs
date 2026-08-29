//! The focused runtime shell interface: [`Shell`] on `gpui::App`.
//!
//! One narrow extension trait over the seams a running application actually
//! needs — identity, cross-thread dispatch, liveness, quit, and the three
//! window routes (declared surface, typed raw window, capability-gated
//! overlay). Everything is a thin, typed wrapper: no new runtime state, no new
//! error vocabulary, and no second implementation of anything the window
//! manager already owns.

use gpui::{App, Entity, Render, Window, WindowHandle};

use crate::declaration::SurfaceKey;
use crate::handles::{AppInfo, AppProxy, AppShellExt};
use crate::liveness::ShellHold;
use crate::windows::{
    OverlaySpec, RawWindow, SurfaceOpen, WindowError, WindowManager, open_raw_window, open_surface,
};

/// The running application's shell.
///
/// Reached from any `&mut App`, so a command handler, a window callback, or a
/// setup module all use the same interface.
pub trait Shell {
    /// Immutable identity, paths, and detected capabilities.
    fn app_info(&self) -> &AppInfo;

    /// A cross-thread dispatch proxy.
    fn app_proxy(&self) -> AppProxy;

    /// Acquire a liveness lease. The shell stays alive until it is dropped.
    fn hold(&self, reason: &'static str) -> ShellHold;

    /// Route a quit through the single shutdown path.
    fn request_quit(&mut self);

    /// Open a declared surface by its typed key.
    ///
    /// Singleton surfaces focus and reuse a live window; `multiple()` surfaces
    /// always create a numbered instance.
    ///
    /// # Errors
    ///
    /// Returns [`WindowError::UndeclaredSurface`] for a key the declaration
    /// never installed, and whatever the window manager reports for a failed
    /// open.
    fn open_surface<View: 'static + Render, Args: 'static>(
        &mut self,
        key: SurfaceKey<View, Args>,
        args: &Args,
    ) -> Result<SurfaceOpen<View>, WindowError>;

    /// Open a typed raw window: the content view is the window root, with no
    /// framework composition and no menu chrome.
    ///
    /// # Errors
    ///
    /// Whatever the window manager reports for a failed open.
    fn open_raw<View: 'static + Render, Args: 'static>(
        &mut self,
        raw: &RawWindow<View, Args>,
        args: &Args,
    ) -> Result<WindowHandle<View>, WindowError>;

    /// Open a capability-gated overlay surface: the escape from both `Root`
    /// composition and ordinary window management.
    ///
    /// # Errors
    ///
    /// Returns [`WindowError::Unsupported`] where the platform reports overlay
    /// surfaces unavailable, before any window is created.
    fn open_overlay<View: 'static + Render>(
        &mut self,
        spec: OverlaySpec,
        build: impl FnOnce(&mut Window, &mut App) -> Entity<View>,
    ) -> Result<WindowHandle<View>, WindowError>;
}

impl Shell for App {
    fn app_info(&self) -> &AppInfo {
        AppShellExt::app_info(self)
    }

    fn app_proxy(&self) -> AppProxy {
        AppShellExt::app_proxy(self)
    }

    fn hold(&self, reason: &'static str) -> ShellHold {
        AppShellExt::shell(self).hold(reason)
    }

    fn request_quit(&mut self) {
        AppShellExt::request_quit(self);
    }

    fn open_surface<View: 'static + Render, Args: 'static>(
        &mut self,
        key: SurfaceKey<View, Args>,
        args: &Args,
    ) -> Result<SurfaceOpen<View>, WindowError> {
        open_surface(self, key, args)
    }

    fn open_raw<View: 'static + Render, Args: 'static>(
        &mut self,
        raw: &RawWindow<View, Args>,
        args: &Args,
    ) -> Result<WindowHandle<View>, WindowError> {
        open_raw_window(self, raw, args)
    }

    fn open_overlay<View: 'static + Render>(
        &mut self,
        spec: OverlaySpec,
        build: impl FnOnce(&mut Window, &mut App) -> Entity<View>,
    ) -> Result<WindowHandle<View>, WindowError> {
        WindowManager::open_overlay(self, spec, build)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use gpui::{AppContext as _, Empty, TestAppContext};
    use neutron_components_storage::{AppPaths, PathLayout};

    use super::*;
    use crate::declaration::{DeclaredSurface, Surface, SurfaceRole};
    use crate::handles::PendingEvents;
    use crate::handles::{AppInfo, AppProxy};
    use crate::liveness::{ExitPolicy, InitialActivation, Liveness};
    use crate::module::RuntimeModule;
    use crate::windows::WindowsModule;
    use crate::{PlatformCapabilities, handles};

    fn identity() -> neutron_components_manifest::schema::IdentityRef {
        crate::declaration::tests::identity()
    }

    /// Install the shell global and the window manager, as startup does.
    fn shell(cx: &mut App) -> (AppInfo, AppProxy) {
        neutron_components::init(cx);
        let info = crate::handles::AppInfo::new(
            identity(),
            AppPaths::new("appshell-runtime-tests", PathLayout::PlatformDefault)
                .expect("test paths resolve"),
            PlatformCapabilities::detect(),
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
            .expect("the window manager initializes");
        (info, proxy)
    }

    fn build_empty(_args: &(), _window: &mut Window, cx: &mut App) -> Entity<Empty> {
        cx.new(|_| Empty)
    }

    #[gpui::test]
    fn the_shell_reports_the_installed_identity_and_a_live_proxy(cx: &mut TestAppContext) {
        cx.update(|cx| {
            shell(cx);

            assert_eq!(Shell::app_info(cx).identity(), identity());
            assert!(
                !Shell::app_proxy(cx).is_closed(),
                "a running shell hands out a live proxy",
            );
        });
    }

    #[gpui::test]
    fn a_hold_keeps_the_shell_alive_until_it_is_dropped(cx: &mut TestAppContext) {
        cx.update(|cx| {
            shell(cx);

            let hold = Shell::hold(cx, "probe");
            assert_eq!(
                cx.global::<crate::handles::ShellState>().holds(),
                1,
                "the lease is registered while it is held",
            );
            drop(hold);
            assert_eq!(cx.global::<crate::handles::ShellState>().holds(), 0);
        });
    }

    #[gpui::test]
    fn opening_an_undeclared_surface_is_a_typed_fault(cx: &mut TestAppContext) {
        cx.update(|cx| {
            shell(cx);

            let error = Shell::open_surface(cx, SurfaceKey::<Empty, ()>::new("logs"), &())
                .expect_err("nothing declared `logs`");

            assert!(
                matches!(error, WindowError::UndeclaredSurface { id: "logs" }),
                "the typed wrapper preserves the window manager's own vocabulary: {error:?}",
            );
        });
    }

    #[gpui::test]
    fn the_typed_wrappers_open_a_declared_surface_and_a_raw_window(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let (info, proxy) = shell(cx);
            crate::windows::declared_surface_module(DeclaredSurface::erase(
                Surface::new(SurfaceKey::<Empty, ()>::new("logs"), build_empty),
                SurfaceRole::Auxiliary,
            ))
            .init(cx, &info, &proxy)
            .expect("the surface installs");

            let opened = Shell::open_surface(cx, SurfaceKey::<Empty, ()>::new("logs"), &())
                .expect("a declared surface opens");
            assert!(
                matches!(opened, SurfaceOpen::Created(_)),
                "the first open of a singleton creates it",
            );

            let raw = RawWindow::<Empty, ()>::new("raw.probe", build_empty);
            Shell::open_raw(cx, &raw, &()).expect("a raw window opens");

            assert_eq!(
                cx.global::<crate::windows::WindowManager>().window_count(),
                2,
                "both wrappers really opened a window",
            );
        });
    }

    #[gpui::test]
    fn an_overlay_is_refused_where_the_platform_does_not_support_it(cx: &mut TestAppContext) {
        cx.update(|cx| {
            shell(cx);

            let result = Shell::open_overlay(
                cx,
                OverlaySpec::new("overlay.probe", 320.0, 240.0),
                |_window, cx| cx.new(|_| Empty),
            );

            match result {
                Err(WindowError::Unsupported { .. }) => {}
                Err(other) => panic!("an unsupported overlay is a capability fault: {other:?}"),
                Ok(_) => assert_eq!(
                    Shell::app_info(cx).capabilities().overlay_surface,
                    crate::capabilities::Capability::Supported,
                    "an overlay may only open where the platform supports it",
                ),
            }
        });
    }
}
