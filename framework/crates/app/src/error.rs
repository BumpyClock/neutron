//! Stable public error type for the application shell.
//!
//! Library callbacks and shell APIs return [`AppShellError`] rather than
//! `anyhow::Error` so downstream apps can match on failure modes across
//! releases (plan §3, "public API errors are a stable `AppShellError`").

use thiserror::Error;

/// Errors surfaced by the application shell.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum AppShellError {
    /// A declaration failed pure validation.
    ///
    /// Carries every independent fault in declaration order. Constructed before
    /// paths, the platform, and GPUI exist, so a malformed declaration costs
    /// nothing.
    #[error("invalid application declaration: {0}")]
    Declaration(#[source] crate::declaration::DeclarationErrors),

    /// Per-app directories could not be resolved from the identity namespace.
    #[error("failed to resolve application paths")]
    Paths(#[source] neutron_components_storage::StorageError),

    /// A pre-platform preparation step failed.
    ///
    /// Covers the advanced hooks that ready the process before the application
    /// exists — `prepare`, `configure_application`, environment setup, and a
    /// [`crate::LoggingPolicy::Configure`] initializer. The failure is
    /// returned rather than logged and swallowed: a process that could not be
    /// prepared must not continue starting, and for logging in particular the
    /// logger the message would go to is exactly what failed.
    #[error("failed to prepare the application environment")]
    Preparation(#[source] anyhow::Error),

    /// The GPUI platform could not be constructed.
    #[error("failed to initialize application platform")]
    Platform(#[source] anyhow::Error),

    /// A named module failed during startup and startup cannot continue.
    ///
    /// Fatal, unlike [`crate::RuntimeError::shutdown`]: shutdown-phase module
    /// failures are nonfatal by definition and are reported through
    /// [`crate::RuntimeError`], not through this variant.
    #[error("module `{module}` failed")]
    Module {
        /// Stable identifier of the failing module.
        module: &'static str,
        /// Underlying cause.
        #[source]
        source: anyhow::Error,
    },

    /// The application startup transaction failed.
    #[error("application startup failed")]
    Startup(#[source] anyhow::Error),

    /// A launch specification could not be parsed from the process facts.
    ///
    /// Constructed by the declaration's launch preparation, before paths, the
    /// platform, and GPUI exist.
    #[error("failed to parse launch specification")]
    Launch(#[source] anyhow::Error),
}

/// Kind of nonfatal operation reported through [`crate::AppDeclaration::runtime_errors`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RuntimeOperation {
    /// Lifecycle plugin or app-event delivery.
    Lifecycle,
    /// Command or action execution.
    Command,
    /// Best-effort work performed while the shell is shutting down.
    ///
    /// Constructed when a declared application shutdown hook fails: the failure
    /// is reported and the remaining teardown still runs.
    Shutdown,
    /// Best-effort named-module lifecycle work.
    Module,
}

/// A nonfatal runtime error observed by the application shell.
///
/// Startup and directly returned API errors do not pass through this type.
#[derive(Debug)]
pub struct RuntimeError {
    operation: RuntimeOperation,
    command_id: Option<crate::commands::CommandId>,
    event: Option<crate::lifecycle::AppEvent>,
    module: Option<&'static str>,
    source: anyhow::Error,
}

impl RuntimeError {
    /// Build a lifecycle delivery error.
    pub fn lifecycle(event: crate::lifecycle::AppEvent, source: impl Into<anyhow::Error>) -> Self {
        Self {
            operation: RuntimeOperation::Lifecycle,
            command_id: None,
            event: Some(event),
            module: None,
            source: source.into(),
        }
    }

    /// Build a command execution error.
    pub fn command(
        command_id: crate::commands::CommandId,
        source: impl Into<anyhow::Error>,
    ) -> Self {
        Self {
            operation: RuntimeOperation::Command,
            command_id: Some(command_id),
            event: None,
            module: None,
            source: source.into(),
        }
    }

    /// Build a best-effort named-module lifecycle error.
    ///
    /// Used for best-effort module-scoped work, such as a declared application
    /// setup module's teardown or a shell service failing in a way that must
    /// not abort the process.
    pub fn module(module: &'static str, source: impl Into<anyhow::Error>) -> Self {
        Self {
            operation: RuntimeOperation::Module,
            command_id: None,
            event: None,
            module: Some(module),
            source: source.into(),
        }
    }

    /// Build a best-effort shutdown-phase error.
    ///
    /// Used when a declared application shutdown hook fails: the process is
    /// already exiting, so the failure is reported and teardown continues.
    pub fn shutdown(source: impl Into<anyhow::Error>) -> Self {
        Self {
            operation: RuntimeOperation::Shutdown,
            command_id: None,
            event: None,
            module: None,
            source: source.into(),
        }
    }

    /// Operation that failed.
    pub fn operation(&self) -> RuntimeOperation {
        self.operation
    }

    /// Stable command id, for command failures.
    pub fn command_id(&self) -> Option<crate::commands::CommandId> {
        self.command_id
    }

    /// The lifecycle event, for lifecycle failures.
    pub fn event(&self) -> Option<&crate::lifecycle::AppEvent> {
        self.event.as_ref()
    }

    /// Stable module identifier, for module lifecycle failures.
    pub fn module_id(&self) -> Option<&'static str> {
        self.module
    }

    /// Original error, including its source chain.
    pub fn source_error(&self) -> &anyhow::Error {
        &self.source
    }
}

impl std::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (&self.event, self.command_id, self.module) {
            (Some(event), _, _) => {
                write!(
                    f,
                    "lifecycle operation `{}` failed: {}",
                    event.name(),
                    self.source
                )
            }
            (_, Some(command_id), _) => {
                write!(f, "command `{command_id}` failed: {}", self.source)
            }
            (_, _, Some(module)) => write!(f, "module `{module}` failed: {}", self.source),
            _ if self.operation == RuntimeOperation::Shutdown => {
                write!(f, "shutdown operation failed: {}", self.source)
            }
            _ => write!(f, "runtime service operation failed: {}", self.source),
        }
    }
}

impl std::error::Error for RuntimeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.source.as_ref())
    }
}

// Deliberately no blanket `From<StorageError>`: only path *resolution* failures
// are `AppShellError::Paths`. A blanket conversion would misclassify write, lock,
// and corruption `StorageError`s reached via `?` as path errors, so callers map
// `AppPaths::new` explicitly at the one resolution site (`shell.rs`).

/// Returned by [`crate::AppProxy::dispatch`] once the shell has begun shutting
/// down and can no longer accept main-thread work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("the application has shut down and can no longer accept dispatched work")]
pub struct AppClosed;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::CommandId;
    use crate::lifecycle::AppEvent;

    #[test]
    fn runtime_error_preserves_operation_context_and_source() {
        let error = RuntimeError::command(
            CommandId::new("app.settings"),
            anyhow::anyhow!("settings failed"),
        );
        assert_eq!(error.operation(), RuntimeOperation::Command);
        assert_eq!(error.command_id(), Some(CommandId::new("app.settings")));
        assert!(error.event().is_none());
        assert_eq!(error.source_error().to_string(), "settings failed");
    }

    /// Regression: `RuntimeError::module` must report `RuntimeOperation::Module`
    /// and the module identifier through `module_id`, not leak into the
    /// `command_id`/`event` accessors reserved for other operations.
    #[test]
    fn module_error_preserves_module_context_and_source() {
        let error = RuntimeError::module("theme_watcher", anyhow::anyhow!("watch failed"));
        assert_eq!(error.operation(), RuntimeOperation::Module);
        assert_eq!(error.module_id(), Some("theme_watcher"));
        assert_eq!(error.command_id(), None);
        assert!(error.event().is_none());
        assert_eq!(error.source_error().to_string(), "watch failed");
        assert_eq!(
            error.to_string(),
            "module `theme_watcher` failed: watch failed"
        );
    }

    /// Regression: a lifecycle error's `event()` accessor returns the typed
    /// `AppEvent` the shell was delivering, and `Display` still names it by
    /// its stable string.
    #[test]
    fn lifecycle_error_preserves_the_typed_event_and_stable_display_name() {
        let error = RuntimeError::lifecycle(AppEvent::Started, anyhow::anyhow!("plugin failed"));
        assert_eq!(error.operation(), RuntimeOperation::Lifecycle);
        assert!(matches!(error.event(), Some(AppEvent::Started)));
        assert_eq!(error.command_id(), None);
        assert_eq!(
            error.to_string(),
            "lifecycle operation `started` failed: plugin failed"
        );
    }

    /// Regression: `RuntimeError::shutdown` must report `RuntimeOperation::Shutdown`
    /// and a shutdown-specific `Display` message.
    #[test]
    fn shutdown_error_preserves_operation_and_distinct_display() {
        let error = RuntimeError::shutdown(anyhow::anyhow!("flush failed"));
        assert_eq!(error.operation(), RuntimeOperation::Shutdown);
        assert_eq!(error.module_id(), None);
        assert_eq!(error.command_id(), None);
        assert!(error.event().is_none());
        assert_eq!(error.to_string(), "shutdown operation failed: flush failed");
    }
}
