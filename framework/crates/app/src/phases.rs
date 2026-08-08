//! Central, explicit startup/shutdown phase ordering (plan §3, D3).
//!
//! Ordering is defined here, *not* by the order in which builder methods were
//! called — a builder method only records intent; this module fixes the
//! sequence. Shutdown runs the completed phases in reverse. The logic is pure
//! (no gpui) so ordering and reverse-shutdown are unit-testable without any
//! platform.

/// Ordered startup phases. Discriminants are the canonical order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Phase {
    /// Validate identity, resolve paths, apply process-global policies, run
    /// `before_platform` hooks and prepare typed state. No GPUI yet.
    Preflight,
    /// Construct the GPUI [`gpui::Application`] and apply `configure_application`.
    ConfigureApp,
    /// Register raw platform listeners (`on_open_urls`, `on_reopen`) so early
    /// events are captured before services exist.
    EarlyListeners,
    /// `gpui_component::init` — component library globals.
    ComponentInit,
    /// Build core services: [`crate::AppInfo`], [`crate::AppProxy`], shell global.
    CoreServices,
    /// Initialize plugins in registration order.
    PluginInit,
    /// Run the application-owned transactional startup callback.
    Start,
    /// Deliver `Started` and drain any events queued before readiness.
    DrainQueue,
    /// Apply the initial-activation policy.
    Activation,
}

/// The canonical startup order.
pub const STARTUP_PHASES: [Phase; 9] = [
    Phase::Preflight,
    Phase::ConfigureApp,
    Phase::EarlyListeners,
    Phase::ComponentInit,
    Phase::CoreServices,
    Phase::PluginInit,
    Phase::Start,
    Phase::DrainQueue,
    Phase::Activation,
];

/// Records which phases completed so shutdown can unwind them in reverse.
///
/// Only phases that actually completed are unwound — if startup aborts midway,
/// shutdown touches exactly the subsystems that were brought up.
#[derive(Debug, Default, Clone)]
pub struct PhaseTracker {
    completed: Vec<Phase>,
}

impl PhaseTracker {
    /// A tracker with no phases completed.
    pub fn new() -> Self {
        Self::default()
    }

    /// Mark `phase` completed. Phases must be completed in [`STARTUP_PHASES`]
    /// order; out-of-order completion is a shell bug and panics in debug.
    pub fn complete(&mut self, phase: Phase) {
        if let Some(&last) = self.completed.last() {
            debug_assert!(
                phase > last,
                "phases must complete in order: {phase:?} after {last:?}"
            );
        }
        self.completed.push(phase);
    }

    /// Whether `phase` has completed.
    pub fn is_complete(&self, phase: Phase) -> bool {
        self.completed.contains(&phase)
    }

    /// The furthest phase reached, if any.
    pub fn last(&self) -> Option<Phase> {
        self.completed.last().copied()
    }

    /// Completed phases in reverse order — the shutdown sequence.
    pub fn shutdown_order(&self) -> Vec<Phase> {
        self.completed.iter().rev().copied().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_phases_are_strictly_increasing() {
        for pair in STARTUP_PHASES.windows(2) {
            assert!(pair[0] < pair[1], "not ordered: {:?}", pair);
        }
    }

    #[test]
    fn shutdown_is_reverse_of_completed() {
        let mut t = PhaseTracker::new();
        t.complete(Phase::Preflight);
        t.complete(Phase::ConfigureApp);
        t.complete(Phase::EarlyListeners);
        assert_eq!(
            t.shutdown_order(),
            vec![Phase::EarlyListeners, Phase::ConfigureApp, Phase::Preflight]
        );
    }

    #[test]
    fn partial_startup_only_unwinds_reached_phases() {
        let mut t = PhaseTracker::new();
        t.complete(Phase::Preflight);
        t.complete(Phase::ConfigureApp);
        // Startup aborted before services came up.
        assert!(!t.is_complete(Phase::CoreServices));
        assert_eq!(
            t.shutdown_order(),
            vec![Phase::ConfigureApp, Phase::Preflight]
        );
        assert_eq!(t.last(), Some(Phase::ConfigureApp));
    }

    #[test]
    fn full_startup_unwinds_everything() {
        let mut t = PhaseTracker::new();
        for phase in STARTUP_PHASES {
            t.complete(phase);
        }
        let order = t.shutdown_order();
        assert_eq!(order.len(), STARTUP_PHASES.len());
        assert_eq!(order.first().copied(), Some(Phase::Activation));
        assert_eq!(order.last().copied(), Some(Phase::Preflight));
    }
}
