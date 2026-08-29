//! The internal runtime-module lifecycle.
//!
//! A *runtime module* is one framework service the startup plan owns: the
//! window manager, a settings store, the theme convention, the menu owner, a
//! declared surface, the application setup pipeline. Declaration lowering is
//! the only producer; the trait is `pub(crate)`, so an application can never
//! inject one.
//!
//! Lifecycle, run by [`crate::shell::RuntimePlan`] in this exact order:
//! [`prepare`](RuntimeModule::prepare) (before GPUI exists) →
//! [`init`](RuntimeModule::init) (core services up, in declaration order) →
//! [`on_event`](RuntimeModule::on_event) (per lifecycle event) →
//! [`shutdown`](RuntimeModule::shutdown) (reverse init order).
//!
//! No `Send`/`Sync` bound: modules are built and run on the main thread.

use gpui::App;

use crate::error::AppShellError;
use crate::handles::{AppInfo, AppProxy};
use crate::lifecycle::AppEvent;

/// Boxed lifecycle observer, as declared by `AppDeclaration::on_event`.
///
/// Observers are delivered after every runtime module, in declaration order.
pub(crate) type EventHandler = Box<dyn FnMut(&AppEvent, &mut App) -> anyhow::Result<()>>;

/// One internal framework service driven by the runtime plan.
pub(crate) trait RuntimeModule: 'static {
    /// Stable identifier used in runtime error reports.
    fn id(&self) -> &'static str {
        std::any::type_name::<Self>()
    }

    /// Read resolved identity, paths, and capabilities before the platform
    /// exists. Default: no-op.
    fn prepare(&mut self, _info: &AppInfo) {}

    /// Initialize once core services are up. Required.
    ///
    /// A failure is fatal: it aborts startup and unwinds the already
    /// initialized prefix in reverse.
    fn init(&mut self, cx: &mut App, info: &AppInfo, proxy: &AppProxy)
    -> Result<(), AppShellError>;

    /// Handle a lifecycle event. Default: ignore. A failure is nonfatal.
    fn on_event(&mut self, _event: &AppEvent, _cx: &mut App) -> Result<(), AppShellError> {
        Ok(())
    }

    /// Tear down. Default: no-op. Called in reverse init order.
    fn shutdown(&mut self, _cx: &mut App) {}

    /// The module's concrete type name.
    ///
    /// Test-only: declaration lowering asserts the resolved module order, and a
    /// boxed module has no other identity.
    #[cfg(test)]
    fn type_name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }
}

/// The ordered runtime modules a declaration lowered, as the plan runs them.
pub(crate) type RuntimeModules = Vec<Box<dyn RuntimeModule>>;
