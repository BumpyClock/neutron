//! Stable public error type for the application shell.
//!
//! Library callbacks and shell APIs return [`AppShellError`] rather than
//! `anyhow::Error` so downstream apps can match on failure modes across
//! releases (plan §3, "public API errors are a stable `AppShellError`").
//! `anyhow` is still accepted *into* the shell (via `From`) so application
//! callbacks may use `?` freely.

use thiserror::Error;

/// Builder callbacks that occupy the single transactional startup slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum StartupHook {
    /// [`crate::AppShellBuilder::start`].
    Start,
    /// [`crate::AppShellBuilder::on_launch`].
    OnLaunch,
}

impl std::fmt::Display for StartupHook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Start => f.write_str("start"),
            Self::OnLaunch => f.write_str("on_launch"),
        }
    }
}

/// Builder APIs that claim ownership of application menu projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum MenuConfiguration {
    /// [`crate::AppShellBuilder::menus`] with an explicit [`crate::MenuPlan`].
    MenuPlan,
    /// [`crate::AppShellBuilder::standard_menus`] desktop conventions.
    StandardMenus,
}

impl std::fmt::Display for MenuConfiguration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MenuPlan => f.write_str("menus"),
            Self::StandardMenus => f.write_str("standard_menus"),
        }
    }
}

/// Invalid combinations recorded by [`crate::AppShellBuilder`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum BuilderConfigurationError {
    /// Both callbacks would occupy the one transactional startup slot.
    #[error("startup callback `{first}` is already registered; cannot also register `{second}`")]
    DuplicateStartup {
        /// Callback registered first.
        first: StartupHook,
        /// Duplicate or mixed callback registered later.
        second: StartupHook,
    },
    /// More than one builder API claimed the application menu model.
    #[error("menu configuration `{first}` is already registered; cannot also register `{second}`")]
    DuplicateMenus {
        /// Menu API registered first.
        first: MenuConfiguration,
        /// Duplicate or mixed menu API registered later.
        second: MenuConfiguration,
    },
    /// A second [`crate::MenusPlugin`] attempted to own menu projection.
    #[error("an application menu owner is already initialized")]
    DuplicateMenuOwner,
}

/// Errors surfaced by the application shell.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum AppShellError {
    /// The builder contains an invalid callback or service combination.
    #[error("invalid application shell configuration: {0}")]
    Configuration(#[source] BuilderConfigurationError),

    /// The compiled-in [`crate::AppIdentity`] failed validation.
    #[error("invalid app identity: {0}")]
    Identity(String),

    /// Per-app directories could not be resolved from the identity namespace.
    #[error("failed to resolve application paths")]
    Paths(#[source] gpui_component_storage::StorageError),

    /// A required startup service failed and startup cannot continue.
    ///
    /// Degradable services (theme watcher, file logging) must not use this;
    /// they log and continue. Only services declared *required* abort startup.
    #[error("required startup service `{service}` failed")]
    Service {
        /// Stable identifier of the failing service.
        service: &'static str,
        /// Underlying cause.
        #[source]
        source: anyhow::Error,
    },

    /// The GPUI platform could not be constructed.
    #[error("failed to initialize application platform")]
    Platform(#[source] anyhow::Error),

    /// The application startup transaction failed.
    #[error("application startup failed")]
    Startup(#[source] anyhow::Error),

    /// A cross-thread dispatch was attempted after shutdown began.
    #[error(transparent)]
    Closed(#[from] AppClosed),

    /// An application-supplied callback returned an error.
    ///
    /// This is the `?`-ergonomic bridge: `anyhow::Error` from a user closure
    /// converts here automatically.
    #[error("application callback failed")]
    Callback(#[source] anyhow::Error),
}

impl From<anyhow::Error> for AppShellError {
    fn from(source: anyhow::Error) -> Self {
        AppShellError::Callback(source)
    }
}

/// Kind of nonfatal operation reported through [`crate::AppShellBuilder::on_error`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RuntimeOperation {
    /// Lifecycle plugin or app-event delivery.
    Lifecycle,
    /// Command or action execution.
    Command,
    /// Best-effort shell service work.
    Service,
}

/// A nonfatal runtime error observed by the application shell.
///
/// Startup and directly returned API errors do not pass through this type.
#[derive(Debug)]
pub struct RuntimeError {
    operation: RuntimeOperation,
    command_id: Option<String>,
    event: Option<&'static str>,
    source: anyhow::Error,
    continued: bool,
}

impl RuntimeError {
    /// Build a lifecycle delivery error.
    pub fn lifecycle(event: &'static str, source: impl Into<anyhow::Error>) -> Self {
        Self {
            operation: RuntimeOperation::Lifecycle,
            command_id: None,
            event: Some(event),
            source: source.into(),
            continued: true,
        }
    }

    /// Build a command execution error.
    pub fn command(command_id: impl Into<String>, source: impl Into<anyhow::Error>) -> Self {
        Self {
            operation: RuntimeOperation::Command,
            command_id: Some(command_id.into()),
            event: None,
            source: source.into(),
            continued: true,
        }
    }

    /// Build a best-effort service error.
    pub fn service(source: impl Into<anyhow::Error>) -> Self {
        Self {
            operation: RuntimeOperation::Service,
            command_id: None,
            event: None,
            source: source.into(),
            continued: true,
        }
    }

    /// Operation that failed.
    pub fn operation(&self) -> RuntimeOperation {
        self.operation
    }

    /// Stable command id, for command failures.
    pub fn command_id(&self) -> Option<&str> {
        self.command_id.as_deref()
    }

    /// Stable lifecycle event name, for lifecycle failures.
    pub fn event(&self) -> Option<&'static str> {
        self.event
    }

    /// Original error, including its source chain.
    pub fn source_error(&self) -> &anyhow::Error {
        &self.source
    }

    /// Whether execution continued after this error.
    pub fn continued(&self) -> bool {
        self.continued
    }
}

impl std::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (self.event, self.command_id.as_deref()) {
            (Some(event), _) => write!(f, "lifecycle operation `{event}` failed: {}", self.source),
            (_, Some(command_id)) => {
                write!(f, "command `{command_id}` failed: {}", self.source)
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

    #[test]
    fn runtime_error_preserves_operation_context_and_source() {
        let error = RuntimeError::command("app.settings", anyhow::anyhow!("settings failed"));
        assert_eq!(error.operation(), RuntimeOperation::Command);
        assert_eq!(error.command_id(), Some("app.settings"));
        assert_eq!(error.event(), None);
        assert!(error.continued());
        assert_eq!(error.source_error().to_string(), "settings failed");
    }
}
