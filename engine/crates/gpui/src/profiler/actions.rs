use std::time::{Duration, Instant};

use itertools::Itertools;

use crate::action::Action;

const MIN_PROFILED_ACTION_DURATION: Duration = Duration::from_micros(100);

#[doc(hidden)]
#[derive(Clone)]
pub struct ActionStatistics {
    runtime_to_beat: Duration,

    longest_runtimes: heapless::Vec<ActionTiming, 5>,
    running: Option<(&'static str, Instant)>,
}

impl std::fmt::Debug for ActionStatistics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActionStatistics")
            .field("runtime_to_beat", &self.runtime_to_beat)
            .field("longest_runtimes", &self.longest_runtimes)
            .field(
                "running",
                &self.running.map(|(id, started)| (id, started.elapsed())),
            )
            .finish()
    }
}

impl std::fmt::Display for ActionStatistics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Actions that blocked the longest\n")?;
        for action in self
            .longest_runtimes(true)
            .sorted_by_key(|action| action.runtime())
            .rev()
        {
            f.write_fmt(format_args!(
                "{:<20} - {}",
                format!("{:?}", action.runtime()), // impl dbg does not support alignment
                action.name
            ))?;
            writeln!(f)?;
        }
        Ok(())
    }
}

impl Default for ActionStatistics {
    fn default() -> Self {
        Self::new()
    }
}

impl ActionStatistics {
    const fn new() -> Self {
        Self {
            // This keeps more calls on the fast path by only tracking
            // problematic polls
            runtime_to_beat: MIN_PROFILED_ACTION_DURATION,
            longest_runtimes: heapless::Vec::new(),
            running: None,
        }
    }

    pub fn take(&mut self) -> Self {
        let taken = std::mem::take(self);
        self.running = taken.running;
        taken
    }

    pub fn is_empty(&self) -> bool {
        self.longest_runtimes.is_empty()
    }

    #[cfg(feature = "profiler")]
    pub fn update_running_action(&mut self, action: &'static str, started: Instant) {
        self.running = Some((action, started));
    }
    #[cfg(not(feature = "profiler"))]
    pub fn update_running_action(&mut self, _action: &'static str, _started: Instant) {}

    #[cfg(feature = "profiler")]
    pub fn save_action_timing(&mut self) {
        let now = Instant::now();

        let Some((action, started)) = self.running.take() else {
            // Actions are ran only on the foreground executor and therefore
            // sequentially _except_ in tests where they can run concurrently.
            //
            // When ran sequentially self.running will always be Some. When ran
            // concurrently that is no longer true. But that is fine, we do not
            // need to track action timings in tests.
            std::hint::cold_path();
            return;
        };

        let runtime = now.duration_since(started);
        if runtime >= self.runtime_to_beat {
            std::hint::cold_path(); // most actions are not the worst, optimize for that

            if self.longest_runtimes.is_full()
                && let Some(to_replace) = self
                    .longest_runtimes
                    .iter_mut()
                    .min_by_key(|action| action.runtime())
            {
                *to_replace = ActionTiming {
                    name: action,
                    start: started,
                    end: now,
                };
            } else {
                self.longest_runtimes
                    .push(ActionTiming {
                        name: action,
                        start: started,
                        end: now,
                    })
                    .expect("just checked it is not full");
            };

            self.runtime_to_beat = if self.longest_runtimes.is_full() {
                self.longest_runtimes
                    .iter()
                    .map(|action| action.runtime())
                    .min()
                    .expect("never empty")
            } else {
                MIN_PROFILED_ACTION_DURATION
            };
        }
    }
    #[cfg(not(feature = "profiler"))]
    pub fn save_action_timing(&mut self) {}

    pub fn longest_runtimes(&self, include_running: bool) -> impl Iterator<Item = ActionTiming> {
        self.longest_runtimes.iter().copied().chain(
            self.running
                .into_iter()
                .filter(move |_| include_running)
                .map(|(name, start)| ActionTiming {
                    name,
                    start,
                    end: Instant::now(),
                }),
        )
    }
}

#[doc(hidden)]
/// UNSTABLE only for use in the profiler and zed-reliability
#[derive(Copy, Clone)]
pub struct ActionTiming {
    pub name: &'static str,
    pub start: Instant,
    pub end: Instant,
}

impl core::fmt::Debug for ActionTiming {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActionTiming")
            .field("name", &self.name)
            .field("runtime", &self.runtime())
            .finish()
    }
}

impl ActionTiming {
    pub fn duration(&self) -> Duration {
        self.end.saturating_duration_since(self.start)
    }
}

impl ActionTiming {
    #[doc(hidden)]
    pub fn runtime(&self) -> Duration {
        self.end - self.start
    }
}

// The profiler is careful to never block when the lock is held, therefore a
// spinlock is optimal.
#[cfg(feature = "profiler")]
static ACTION_STATISTICS: spin::Mutex<ActionStatistics> =
    const { spin::Mutex::new(ActionStatistics::new()) };

#[doc(hidden)]
#[cfg(feature = "profiler")]
pub(crate) fn update_running_action(action: &(dyn Action + 'static), cx: &mut crate::App) {
    let now = Instant::now();
    let action = action.type_id();
    let action = cx.actions.try_resolve_action(&action).unwrap_or("un-named");
    ACTION_STATISTICS.lock().update_running_action(action, now);
}

#[doc(hidden)]
#[cfg(not(feature = "profiler"))]
#[inline(always)]
pub(crate) fn update_running_action(_: &(dyn Action + 'static), _: &mut crate::App) {}

#[doc(hidden)]
#[cfg(feature = "profiler")]
pub(crate) fn save_action_timing() {
    ACTION_STATISTICS.lock().save_action_timing();
}

#[doc(hidden)]
#[cfg(not(feature = "profiler"))]
#[inline(always)]
pub(crate) fn save_action_timing() {}

#[doc(hidden)]
#[cfg(feature = "profiler")]
pub fn take_action_stats() -> ActionStatistics {
    ACTION_STATISTICS.lock().take()
}

#[doc(hidden)]
#[cfg(not(feature = "profiler"))]
pub fn take_action_stats() -> ActionStatistics {
    ActionStatistics::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(feature = "profiler")]
    fn enabled_action_profiler_records_slow_actions() {
        let mut statistics = ActionStatistics::new();
        statistics.update_running_action("test-action", Instant::now() - Duration::from_millis(1));

        statistics.save_action_timing();

        let timing = statistics
            .longest_runtimes(false)
            .next()
            .expect("slow action should be recorded");
        assert_eq!(timing.name, "test-action");
    }

    #[test]
    #[cfg(feature = "profiler")]
    fn saving_without_a_running_action_is_tolerated() {
        let mut statistics = ActionStatistics::new();
        statistics.save_action_timing();
        assert!(statistics.is_empty());
    }

    #[test]
    #[cfg(feature = "profiler")]
    fn full_action_statistics_replaces_the_shortest_action() {
        let mut statistics = ActionStatistics::new();
        for (name, duration) in [
            ("one", 1),
            ("two", 2),
            ("three", 3),
            ("four", 4),
            ("five", 5),
            ("replacement", 3),
        ] {
            statistics
                .update_running_action(name, Instant::now() - Duration::from_millis(duration));
            statistics.save_action_timing();
        }

        let names = statistics
            .longest_runtimes(false)
            .map(|timing| timing.name)
            .collect::<Vec<_>>();
        assert!(!names.contains(&"one"));
        assert!(names.contains(&"five"));
        assert!(names.contains(&"replacement"));
    }

    #[test]
    #[cfg(feature = "profiler")]
    fn action_statistics_fill_all_slots_before_raising_threshold() {
        let mut statistics = ActionStatistics::new();
        for (name, duration) in [
            ("five", 5),
            ("four", 4),
            ("three", 3),
            ("two", 2),
            ("one", 1),
        ] {
            statistics
                .update_running_action(name, Instant::now() - Duration::from_millis(duration));
            statistics.save_action_timing();
        }

        assert_eq!(statistics.longest_runtimes(false).count(), 5);
    }

    #[test]
    #[cfg(not(feature = "profiler"))]
    fn disabled_action_profiler_is_a_noop() {
        let mut statistics = ActionStatistics::new();
        statistics.update_running_action("test-action", Instant::now() - Duration::from_millis(1));
        statistics.save_action_timing();
        assert!(statistics.is_empty());
    }
}
