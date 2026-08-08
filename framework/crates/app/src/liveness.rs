//! Liveness: hold/release leases and exit policy (plan §3).
//!
//! Application lifetime is governed by *leases*, not a static quit mode. Windows,
//! trays, and background services each take a [`ShellHold`]; when the last hold
//! is released and no windows remain, an app with [`ExitPolicy::WhenIdle`] may
//! exit (the GApplication model). There is no unconditional `activate(true)`:
//! initial activation follows [`InitialActivation`], so a tray-first app can
//! launch passively with zero windows.
//!
//! The exit *decision* is a pure function ([`should_exit`]) and is unit-tested
//! without any platform.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::handles::AppProxy;

/// How the app presents itself at launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum InitialActivation {
    /// Normal foreground activation (default).
    #[default]
    Regular,
    /// Force foreground activation.
    ///
    /// The distinction from [`InitialActivation::Regular`] is observable on
    /// macOS. Current Windows and Linux backends ignore the force flag.
    Forced,
    /// Do not steal focus / show in the foreground. For tray-first apps that
    /// launch with no window (plan §3 — "no unconditional `activate(true)`").
    Passive,
}

/// When the shell is allowed to exit on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExitPolicy {
    /// Exit once there are no windows and no holds (last-window-close model).
    #[default]
    WhenIdle,
    /// Never exit automatically; quit only via an explicit `request_quit`
    /// (tray-first apps that intentionally outlive their windows).
    Explicit,
}

/// Pure exit decision.
///
/// Returns `true` when the shell should exit given the policy, the number of
/// outstanding holds, and the number of open windows.
pub fn should_exit(policy: ExitPolicy, holds: usize, windows: usize) -> bool {
    match policy {
        ExitPolicy::WhenIdle => holds == 0 && windows == 0,
        ExitPolicy::Explicit => false,
    }
}

/// Main-thread liveness state, stored in the shell global.
#[derive(Debug)]
pub struct Liveness {
    holds: Arc<AtomicUsize>,
    exit_policy: ExitPolicy,
    initial_activation: InitialActivation,
    activated: bool,
}

impl Liveness {
    /// New liveness state with the given policies and zero holds.
    pub fn new(exit_policy: ExitPolicy, initial_activation: InitialActivation) -> Self {
        Self {
            holds: Arc::new(AtomicUsize::new(0)),
            exit_policy,
            initial_activation,
            activated: false,
        }
    }

    /// The configured exit policy.
    pub fn exit_policy(&self) -> ExitPolicy {
        self.exit_policy
    }

    /// The configured initial-activation policy.
    pub fn initial_activation(&self) -> InitialActivation {
        self.initial_activation
    }

    /// Whether initial activation has been applied.
    pub fn activated(&self) -> bool {
        self.activated
    }

    /// Record that initial activation has been applied.
    pub fn mark_activated(&mut self) {
        self.activated = true;
    }

    /// Current number of outstanding holds.
    pub fn hold_count(&self) -> usize {
        self.holds.load(Ordering::SeqCst)
    }

    /// The shared hold counter, cloned into [`crate::ShellHandle`] so leases can
    /// be taken without borrowing the main-thread global.
    pub fn holds_arc(&self) -> Arc<AtomicUsize> {
        Arc::clone(&self.holds)
    }
}

/// Acquire a lease against a shared hold counter.
///
/// Used by [`crate::ShellHandle::hold`]. The returned [`ShellHold`] releases the
/// lease on drop and asks the shell to re-evaluate exit; `proxy` schedules that
/// re-evaluation on the main thread (holds may be dropped off-thread).
pub(crate) fn acquire_hold(
    holds: Arc<AtomicUsize>,
    proxy: AppProxy,
    reason: &'static str,
) -> ShellHold {
    holds.fetch_add(1, Ordering::SeqCst);
    ShellHold {
        holds,
        proxy,
        reason,
    }
}

/// An RAII liveness lease. Dropping it releases the lease and triggers an exit
/// re-evaluation on the main thread.
pub struct ShellHold {
    holds: Arc<AtomicUsize>,
    proxy: AppProxy,
    reason: &'static str,
}

impl ShellHold {
    /// The reason this hold was taken (for diagnostics).
    pub fn reason(&self) -> &'static str {
        self.reason
    }
}

impl std::fmt::Debug for ShellHold {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShellHold")
            .field("reason", &self.reason)
            .field("remaining", &self.holds.load(Ordering::SeqCst))
            .finish()
    }
}

impl Drop for ShellHold {
    fn drop(&mut self) {
        self.holds.fetch_sub(1, Ordering::SeqCst);
        // Ask the main thread to reconsider exit. If the app has already shut
        // down, the dispatch is a no-op — that's the correct outcome. The exit
        // reason is taken from any pending window-close record, else `Requested`.
        let _ = self.proxy.dispatch(crate::handles::evaluate_exit);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn when_idle_exits_only_with_no_windows_and_no_holds() {
        assert!(should_exit(ExitPolicy::WhenIdle, 0, 0));
        assert!(!should_exit(ExitPolicy::WhenIdle, 1, 0));
        assert!(!should_exit(ExitPolicy::WhenIdle, 0, 1));
        assert!(!should_exit(ExitPolicy::WhenIdle, 2, 3));
    }

    #[test]
    fn explicit_never_auto_exits() {
        assert!(!should_exit(ExitPolicy::Explicit, 0, 0));
        assert!(!should_exit(ExitPolicy::Explicit, 5, 5));
    }

    #[test]
    fn startup_idle_whenidle_app_is_eligible_to_exit() {
        // The stable-state evaluation the shell runs after activation: a
        // WhenIdle app that launched with no window and no hold must be eligible
        // to exit (otherwise it lives forever under QuitMode::Explicit)...
        assert!(should_exit(ExitPolicy::WhenIdle, 0, 0));
        // ...an app that opened a window during `Started` stays alive...
        assert!(!should_exit(ExitPolicy::WhenIdle, 0, 1));
        // ...a hold (e.g. tray) keeps it alive...
        assert!(!should_exit(ExitPolicy::WhenIdle, 1, 0));
        // ...and a tray-first Explicit app never auto-exits at startup.
        assert!(!should_exit(ExitPolicy::Explicit, 0, 0));
    }
}
