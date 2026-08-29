//! Advanced process-level policies and hooks.
//!
//! Everything here is opt-in. The defaults are the conventional desktop
//! foundation: platform-default paths, the inherited environment, and
//! application-owned logging, with no process preparation and no GPUI
//! [`Application`] customization.

use gpui::Application;
use neutron_components_storage::PathLayout;

use crate::handles::AppInfo;
use crate::shell::{EnvironmentPolicy, LoggingPolicy};

/// Process preparation, run once after identity, paths, and capabilities
/// resolve and before the platform starts.
///
/// A non-capturing `fn` pointer by design: preparation is a process-global side
/// effect, so it must not carry application state that would then need a
/// lifetime, a lock, or a thread-affinity rule.
pub(crate) type PrepareHook = fn(&AppInfo) -> anyhow::Result<()>;

/// Customization of the GPUI [`Application`] before it runs. Non-capturing for
/// the same reason as [`PrepareHook`].
pub(crate) type ConfigureApplicationHook = fn(Application) -> anyhow::Result<Application>;

/// Opt-in escape hatches from the conventional desktop foundation.
pub struct AdvancedHooks {
    pub(super) path_layout: PathLayout,
    pub(super) environment: EnvironmentPolicy,
    pub(super) logging: LoggingPolicy,
    pub(super) prepare: Option<PrepareHook>,
    pub(super) configure_application: Option<ConfigureApplicationHook>,
}

impl Default for AdvancedHooks {
    fn default() -> Self {
        Self {
            path_layout: PathLayout::PlatformDefault,
            environment: EnvironmentPolicy::Inherit,
            logging: LoggingPolicy::External,
            prepare: None,
            configure_application: None,
        }
    }
}

impl AdvancedHooks {
    /// The conventional defaults.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Choose the on-disk directory layout (default
    /// [`PathLayout::PlatformDefault`]).
    #[must_use]
    pub fn path_layout(mut self, layout: PathLayout) -> Self {
        self.path_layout = layout;
        self
    }

    /// Set the environment policy (default [`EnvironmentPolicy::Inherit`]).
    #[must_use]
    pub fn environment(mut self, policy: EnvironmentPolicy) -> Self {
        self.environment = policy;
        self
    }

    /// Set the logging policy (default [`LoggingPolicy::External`]).
    #[must_use]
    pub fn logging(mut self, policy: LoggingPolicy) -> Self {
        self.logging = policy;
        self
    }

    /// Run process preparation once [`AppInfo`] is known.
    #[must_use]
    pub fn prepare(mut self, hook: PrepareHook) -> Self {
        self.prepare = Some(hook);
        self
    }

    /// Customize the GPUI [`Application`] before it runs.
    #[must_use]
    pub fn configure_application(mut self, hook: ConfigureApplicationHook) -> Self {
        self.configure_application = Some(hook);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prepare_hook(_info: &AppInfo) -> anyhow::Result<()> {
        Ok(())
    }

    fn configure_hook(application: Application) -> anyhow::Result<Application> {
        Ok(application)
    }

    fn logging_hook(_paths: &neutron_components_storage::AppPaths) -> anyhow::Result<()> {
        Ok(())
    }

    #[test]
    fn defaults_are_the_conventional_desktop_foundation() {
        let hooks = AdvancedHooks::new();

        assert_eq!(hooks.path_layout, PathLayout::PlatformDefault);
        assert!(matches!(hooks.environment, EnvironmentPolicy::Inherit));
        assert!(matches!(hooks.logging, LoggingPolicy::External));
        assert!(hooks.prepare.is_none());
        assert!(hooks.configure_application.is_none());
    }

    #[test]
    fn consuming_methods_record_non_capturing_hooks() {
        let hooks = AdvancedHooks::new()
            .path_layout(PathLayout::SingleRoot(".neutron".into()))
            .environment(EnvironmentPolicy::LoginShell)
            .logging(LoggingPolicy::Configure(logging_hook))
            .prepare(prepare_hook)
            .configure_application(configure_hook);

        assert_eq!(hooks.path_layout, PathLayout::SingleRoot(".neutron".into()));
        assert!(matches!(hooks.environment, EnvironmentPolicy::LoginShell));
        assert!(matches!(hooks.logging, LoggingPolicy::Configure(_)));
        assert!(hooks.prepare.is_some());
        assert!(hooks.configure_application.is_some());
    }
}
