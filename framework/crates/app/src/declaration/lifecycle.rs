//! The declaration's lifecycle hooks: common start, event observers, the one
//! runtime error reporter, and the application shutdown hook.
//!
//! Everything here is a non-capturing `fn` pointer, like the rest of the
//! declaration model: a declaration is a pure, type-level description, so a
//! hook that closed over state would need a lifetime, a lock, or a thread
//! affinity rule that the declaration cannot express.
//!
//! Cardinality is part of the declaration, not of lowering: `on_event` is
//! repeatable, while start, the reporter, and shutdown are singletons. A
//! surplus singleton is *counted*, never dropped or last-wins applied, so
//! [`AppDeclaration::validate`](super::AppDeclaration::validate) can report it
//! and an application can never silently lose lifecycle work it asked for.

use gpui::App;

use crate::error::RuntimeError;
use crate::handles::AppShutdownHook;
use crate::lifecycle::AppEvent;
use crate::module::EventHandler;
use crate::shell::{ErrorReporter, StartCallback};

use super::errors::DeclarationError;

/// Fallible application composition, run once after every framework and
/// declared module has initialized and before the launch hook, the primary
/// surface, and `Started`.
///
/// Launch-specific work belongs in
/// [`LaunchSpec::before_primary`](super::LaunchSpec::before_primary); this hook
/// is the launch-independent half, so it takes no launch value.
pub(crate) type StartHook = fn(&mut App) -> anyhow::Result<()>;

/// A lifecycle event observer. Repeatable; a failure is nonfatal and later
/// observers still run.
pub(crate) type EventHook = fn(&AppEvent, &mut App) -> anyhow::Result<()>;

/// The single observer of nonfatal runtime errors.
pub(crate) type ErrorHook = fn(&RuntimeError, &mut App);

/// Application teardown, run after `WillExit` and before framework modules tear
/// down in reverse.
pub(crate) type ShutdownHook = fn(&mut App) -> anyhow::Result<()>;

/// The declared lifecycle hooks, with their cardinality faults counted.
#[derive(Default)]
pub(crate) struct LifecycleHooks {
    start: Option<StartHook>,
    surplus_starts: usize,
    events: Vec<EventHook>,
    reporter: Option<ErrorHook>,
    surplus_reporters: usize,
    shutdown: Option<ShutdownHook>,
    surplus_shutdowns: usize,
}

impl LifecycleHooks {
    /// The conventional default: no lifecycle hooks at all.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Record the common start hook, counting a surplus declaration.
    pub(crate) fn start(&mut self, hook: StartHook) {
        match self.start {
            None => self.start = Some(hook),
            Some(_) => self.surplus_starts += 1,
        }
    }

    /// Append a lifecycle observer, preserving declaration order.
    pub(crate) fn on_event(&mut self, hook: EventHook) {
        self.events.push(hook);
    }

    /// Record the runtime error reporter, counting a surplus declaration.
    pub(crate) fn on_error(&mut self, hook: ErrorHook) {
        match self.reporter {
            None => self.reporter = Some(hook),
            Some(_) => self.surplus_reporters += 1,
        }
    }

    /// Record the application shutdown hook, counting a surplus declaration.
    ///
    /// The hook is paired with the common start phase, not with process exit:
    /// it runs for every teardown from that phase onward, and not at all when
    /// startup fails before it. See
    /// [`RuntimePlan::run`](crate::shell::RuntimePlan::run).
    pub(crate) fn shutdown(&mut self, hook: ShutdownHook) {
        match self.shutdown {
            None => self.shutdown = Some(hook),
            Some(_) => self.surplus_shutdowns += 1,
        }
    }

    /// Report one fault per surplus singleton hook, in declaration-slot order.
    pub(crate) fn validate(&self, errors: &mut Vec<DeclarationError>) {
        errors.extend(std::iter::repeat_n(
            DeclarationError::MultipleStartHooks,
            self.surplus_starts,
        ));
        errors.extend(std::iter::repeat_n(
            DeclarationError::MultipleErrorReporters,
            self.surplus_reporters,
        ));
        errors.extend(std::iter::repeat_n(
            DeclarationError::MultipleShutdownHooks,
            self.surplus_shutdowns,
        ));
    }

    /// Finalize the declared hooks into the values the runtime plan runs.
    pub(crate) fn into_runtime(self) -> LifecycleRuntime {
        let Self {
            start,
            surplus_starts: _,
            events,
            reporter,
            surplus_reporters: _,
            shutdown,
            surplus_shutdowns: _,
        } = self;

        LifecycleRuntime {
            start: start.map(|hook| Box::new(hook) as StartCallback),
            observers: events
                .into_iter()
                .map(|hook| Box::new(hook) as EventHandler)
                .collect(),
            reporter: reporter.map(|hook| Box::new(hook) as ErrorReporter),
            shutdown: shutdown.map(|hook| Box::new(hook) as AppShutdownHook),
        }
    }
}

/// The declared lifecycle hooks, finalized for the runtime plan.
pub(crate) struct LifecycleRuntime {
    /// The common start hook, if declared.
    pub(crate) start: Option<StartCallback>,
    /// Lifecycle observers in declaration order, each nonfatal.
    pub(crate) observers: Vec<EventHandler>,
    /// The single nonfatal-runtime-error reporter, if declared.
    pub(crate) reporter: Option<ErrorReporter>,
    /// The application teardown hook, if declared.
    pub(crate) shutdown: Option<AppShutdownHook>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn start_ok(_cx: &mut App) -> anyhow::Result<()> {
        Ok(())
    }

    fn observe(_event: &AppEvent, _cx: &mut App) -> anyhow::Result<()> {
        Ok(())
    }

    fn report(_error: &RuntimeError, _cx: &mut App) {}

    fn teardown(_cx: &mut App) -> anyhow::Result<()> {
        Ok(())
    }

    fn faults(hooks: &LifecycleHooks) -> Vec<String> {
        let mut errors = Vec::new();
        hooks.validate(&mut errors);
        errors.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn a_single_hook_of_each_kind_is_faultless() {
        let mut hooks = LifecycleHooks::new();
        hooks.start(start_ok);
        hooks.on_event(observe);
        hooks.on_event(observe);
        hooks.on_error(report);
        hooks.shutdown(teardown);

        assert!(faults(&hooks).is_empty(), "on_event is repeatable");
    }

    #[test]
    fn surplus_singleton_hooks_are_declaration_faults() {
        let mut hooks = LifecycleHooks::new();
        hooks.start(start_ok);
        hooks.start(start_ok);
        hooks.on_error(report);
        hooks.on_error(report);
        hooks.shutdown(teardown);
        hooks.shutdown(teardown);
        hooks.shutdown(teardown);

        assert_eq!(
            faults(&hooks),
            vec![
                "only one start hook may be declared; a second one was declared".to_string(),
                "only one runtime error reporter may be declared; a second one was declared"
                    .to_string(),
                "only one application shutdown hook may be declared; a second one was declared"
                    .to_string(),
                "only one application shutdown hook may be declared; a second one was declared"
                    .to_string(),
            ],
            "every surplus declaration is reported, so fixing one cannot hide another",
        );
    }

    #[test]
    fn the_first_declared_singleton_is_the_one_that_is_kept() {
        let mut hooks = LifecycleHooks::new();
        hooks.start(start_ok);
        hooks.start(|_cx| anyhow::bail!("the surplus hook must never run"));

        let kept: StartHook = hooks.start.expect("a start hook is recorded");
        let expected: StartHook = start_ok;
        assert!(
            std::ptr::fn_addr_eq(kept, expected),
            "a surplus hook is counted, never applied last-wins",
        );
    }
}
