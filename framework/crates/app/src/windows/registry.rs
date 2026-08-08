//! Pure window-registry logic: numbering, singleton state machine, and
//! bookkeeping (plan §3 "Windows").
//!
//! This module deliberately touches *no* GPUI. It is generic over an opaque
//! window handle `H` so the whole surface — number allocation, title formatting,
//! the `Closed | Opening | Open` singleton transitions, and window/overlay
//! counting — is unit-tested without opening a real window. The GPUI-facing
//! [`super::WindowManager`] instantiates it with `gpui::AnyWindowHandle` and
//! sequences the borrows around `open_window`.

use std::any::TypeId;
use std::collections::{BTreeSet, HashMap};
use std::hash::Hash;

use super::key::WindowKey;
use super::spec::RootPolicy;

/// Whether a registered surface is a real window (Root-wrappable, numbered,
/// holds a liveness lease) or an overlay surface (capability-gated, not
/// numbered, no lease).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceKind {
    /// A normal application window.
    Window,
    /// An overlay surface (click-through / popup).
    Overlay,
}

/// One registered surface.
#[derive(Debug, Clone)]
pub struct WindowRecord {
    /// Stable role identity.
    pub key: WindowKey,
    /// The un-numbered base title this window was opened with.
    pub base_title: String,
    /// The assigned window number (1-based). `0` for overlays (not numbered).
    pub number: u32,
    /// The resolved, user-facing title (`base` or `"base - N"`).
    pub title: String,
    /// Window vs overlay.
    pub kind: SurfaceKind,
}

/// The lifecycle of a keyed singleton window.
///
/// `Opening` is the async-safety state: it is set *before* the (synchronous, but
/// potentially reentrant) window build begins, so a second `open_singleton` for
/// the same key observes `Opening` and refuses to double-create.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SingletonPhase<H> {
    /// No window; a new open should proceed.
    Closed,
    /// A create is in flight; a concurrent open should no-op.
    Opening,
    /// A window is open with the given handle (subject to a liveness probe).
    Open(H),
}

/// Type and root-policy contract held while a singleton is opening or live.
///
/// The manager intentionally does not offer typed access to a singleton's
/// content yet; this metadata only prevents callers from reusing a window key
/// with an incompatible view or root policy.
#[derive(Debug, Clone, Copy)]
pub struct SingletonMetadata {
    pub(super) content_type: TypeId,
    pub(super) content_type_name: &'static str,
    pub(super) root_policy: RootPolicy,
}

impl SingletonMetadata {
    pub(super) fn of<V: 'static>(root_policy: RootPolicy) -> Self {
        Self {
            content_type: TypeId::of::<V>(),
            content_type_name: std::any::type_name::<V>(),
            root_policy,
        }
    }
}

/// Current singleton state plus the contract registered at creation time.
#[derive(Debug, Clone, Copy)]
pub struct SingletonRecord<H> {
    pub(super) phase: SingletonPhase<H>,
    pub(super) metadata: SingletonMetadata,
}

/// What an `open_singleton` call should do, given the current phase and whether
/// the tracked handle is still alive. Pure; see [`plan_singleton`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SingletonAction<H> {
    /// Proceed to create a new window.
    Create,
    /// A create is already in flight; do nothing.
    InFlight,
    /// Focus and reuse the still-live window with this handle.
    Reuse(H),
}

/// Decide what `open_singleton` should do.
///
/// `alive` is only meaningful when `phase` is [`SingletonPhase::Open`]; callers
/// probe the real handle (e.g. by attempting to focus it) and pass the result.
pub fn plan_singleton<H: Copy>(phase: SingletonPhase<H>, alive: bool) -> SingletonAction<H> {
    match phase {
        SingletonPhase::Closed => SingletonAction::Create,
        SingletonPhase::Opening => SingletonAction::InFlight,
        SingletonPhase::Open(handle) => {
            if alive {
                SingletonAction::Reuse(handle)
            } else {
                // Stale handle: the window is gone but the phase never reset.
                SingletonAction::Create
            }
        }
    }
}

/// Format a window title from its base and 1-based number.
///
/// Number `1` (the first/only window of that base) keeps the bare title; later
/// windows get a `" - N"` suffix (`"App"`, `"App - 2"`, …).
fn format_title(base: &str, number: u32) -> String {
    if number <= 1 {
        base.to_string()
    } else {
        format!("{base} - {number}")
    }
}

/// The pure registry: numbers, records, and singleton phases.
#[derive(Debug)]
pub struct Registry<H> {
    records: HashMap<H, WindowRecord>,
    /// In-use window numbers per base title. Reserved at `allocate`, freed at
    /// `remove`/`release`, so numbering survives a build that reentrantly opens
    /// another window of the same base.
    numbers: HashMap<String, BTreeSet<u32>>,
    singletons: HashMap<WindowKey, SingletonRecord<H>>,
    version: u64,
}

impl<H> Default for Registry<H> {
    fn default() -> Self {
        Self {
            records: HashMap::new(),
            numbers: HashMap::new(),
            singletons: HashMap::new(),
            version: 0,
        }
    }
}

impl<H: Copy + Eq + Hash> Registry<H> {
    /// A fresh, empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Reserve the lowest free number for `base_title` and return it with the
    /// formatted title. The number stays reserved until [`Registry::remove`] or
    /// [`Registry::release`].
    pub fn allocate(&mut self, base_title: &str) -> (u32, String) {
        let set = self.numbers.entry(base_title.to_string()).or_default();
        let mut number = 1;
        while set.contains(&number) {
            number += 1;
        }
        set.insert(number);
        (number, format_title(base_title, number))
    }

    /// Release a reserved number that never became — or is no longer — a record
    /// (e.g. `open_window` failed after [`Registry::allocate`]).
    pub fn release(&mut self, base_title: &str, number: u32) {
        if let Some(set) = self.numbers.get_mut(base_title) {
            set.remove(&number);
            if set.is_empty() {
                self.numbers.remove(base_title);
            }
        }
    }

    /// Register an opened surface under its handle. Bumps the change version.
    pub fn insert(&mut self, handle: H, record: WindowRecord) {
        self.records.insert(handle, record);
        self.version += 1;
    }

    /// Deregister a surface, freeing its window number. Bumps the change
    /// version. Returns the removed record, if any.
    pub fn remove(&mut self, handle: &H) -> Option<WindowRecord> {
        let record = self.records.remove(handle)?;
        self.release(&record.base_title, record.number);
        self.version += 1;
        Some(record)
    }

    /// Number of registered real windows (excludes overlays).
    pub fn window_count(&self) -> usize {
        self.records
            .values()
            .filter(|r| r.kind == SurfaceKind::Window)
            .count()
    }

    /// Number of registered overlays.
    pub fn overlay_count(&self) -> usize {
        self.records
            .values()
            .filter(|r| r.kind == SurfaceKind::Overlay)
            .count()
    }

    /// A monotonic counter bumped on every insert/remove — a minimal
    /// observation seam for menu rebuilds (Move-to-Window) without a callback
    /// registry.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Iterate registered `(handle, record)` pairs.
    pub fn iter(&self) -> impl Iterator<Item = (&H, &WindowRecord)> {
        self.records.iter()
    }

    /// Registered handles for which `is_live` returns `false` — i.e. surfaces
    /// that have closed and whose records should be deregistered.
    ///
    /// This is the pure core of reconciliation: the GPUI edge passes a predicate
    /// backed by the live window set, so deregistration is driven by actual
    /// window closure rather than entity lifetime (which a retained root/content
    /// entity would defeat).
    pub fn closed_handles(&self, is_live: impl Fn(&H) -> bool) -> Vec<H> {
        self.records
            .keys()
            .filter(|handle| !is_live(handle))
            .copied()
            .collect()
    }

    /// The current singleton phase for `key` ([`SingletonPhase::Closed`] if the
    /// key was never opened as a singleton).
    pub fn singleton_phase(&self, key: WindowKey) -> SingletonPhase<H> {
        self.singletons
            .get(&key)
            .map(|record| record.phase)
            .unwrap_or(SingletonPhase::Closed)
    }

    /// The contract for a live or in-flight singleton.
    pub fn singleton_metadata(&self, key: WindowKey) -> Option<SingletonMetadata> {
        self.singletons.get(&key).map(|record| record.metadata)
    }

    /// Register a singleton before its window build begins.
    pub fn begin_singleton(&mut self, key: WindowKey, metadata: SingletonMetadata) {
        self.singletons.insert(
            key,
            SingletonRecord {
                phase: SingletonPhase::Opening,
                metadata,
            },
        );
    }

    /// Mark an in-flight singleton as open, retaining its original contract.
    pub fn finish_singleton(&mut self, key: WindowKey, handle: H) {
        if let Some(record) = self.singletons.get_mut(&key) {
            record.phase = SingletonPhase::Open(handle);
        }
    }

    /// Remove singleton state after failure or window closure.
    pub fn clear_singleton(&mut self, key: WindowKey) {
        self.singletons.remove(&key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAIN: WindowKey = WindowKey::new("main");
    const SETTINGS: WindowKey = WindowKey::new("settings");

    fn record(number: u32, title: &str, kind: SurfaceKind) -> WindowRecord {
        WindowRecord {
            key: MAIN,
            base_title: "App".to_string(),
            number,
            title: title.to_string(),
            kind,
        }
    }

    #[test]
    fn first_window_keeps_bare_title_later_are_suffixed() {
        let mut reg: Registry<u64> = Registry::new();
        assert_eq!(reg.allocate("App"), (1, "App".to_string()));
        assert_eq!(reg.allocate("App"), (2, "App - 2".to_string()));
        assert_eq!(reg.allocate("App"), (3, "App - 3".to_string()));
    }

    #[test]
    fn numbering_reuses_lowest_free_after_release() {
        let mut reg: Registry<u64> = Registry::new();
        let (n1, _) = reg.allocate("App"); // 1
        let (n2, _) = reg.allocate("App"); // 2
        let (n3, _) = reg.allocate("App"); // 3
        assert_eq!((n1, n2, n3), (1, 2, 3));

        // Freeing #2 makes 2 the lowest free again.
        reg.release("App", 2);
        assert_eq!(reg.allocate("App"), (2, "App - 2".to_string()));
        // 4 is next after 1,2,3.
        assert_eq!(reg.allocate("App"), (4, "App - 4".to_string()));
    }

    #[test]
    fn numbering_is_per_base_title() {
        let mut reg: Registry<u64> = Registry::new();
        assert_eq!(reg.allocate("App"), (1, "App".to_string()));
        assert_eq!(reg.allocate("Inspector"), (1, "Inspector".to_string()));
        assert_eq!(reg.allocate("App"), (2, "App - 2".to_string()));
    }

    #[test]
    fn remove_frees_the_number_for_reuse() {
        let mut reg: Registry<u64> = Registry::new();
        let (n, title) = reg.allocate("App");
        reg.insert(10, record(n, &title, SurfaceKind::Window));
        let (n2, _) = reg.allocate("App");
        assert_eq!(n2, 2);

        reg.remove(&10);
        // #1 is free again.
        assert_eq!(reg.allocate("App"), (1, "App".to_string()));
    }

    #[test]
    fn counts_distinguish_windows_from_overlays() {
        let mut reg: Registry<u64> = Registry::new();
        reg.insert(1, record(1, "App", SurfaceKind::Window));
        reg.insert(2, record(2, "App - 2", SurfaceKind::Window));
        reg.insert(3, record(0, "Overlay", SurfaceKind::Overlay));
        assert_eq!(reg.window_count(), 2);
        assert_eq!(reg.overlay_count(), 1);
    }

    #[test]
    fn version_bumps_on_insert_and_remove_only() {
        let mut reg: Registry<u64> = Registry::new();
        assert_eq!(reg.version(), 0);
        // allocate/release alone do not bump (no registered change yet).
        let _ = reg.allocate("App");
        assert_eq!(reg.version(), 0);

        reg.insert(1, record(1, "App", SurfaceKind::Window));
        assert_eq!(reg.version(), 1);
        reg.remove(&1);
        assert_eq!(reg.version(), 2);
        reg.remove(&1); // no-op
        assert_eq!(reg.version(), 2);
    }

    #[test]
    fn closed_handles_returns_registered_surfaces_absent_from_live_set() {
        let mut reg: Registry<u64> = Registry::new();
        reg.insert(1, record(1, "App", SurfaceKind::Window));
        reg.insert(2, record(2, "App - 2", SurfaceKind::Window));
        reg.insert(3, record(0, "Overlay", SurfaceKind::Overlay));

        // Windows 2 and 3 are gone; 1 is still live.
        let live = [1u64];
        let mut closed = reg.closed_handles(|h| live.contains(h));
        closed.sort();
        assert_eq!(closed, vec![2, 3]);

        // All live: nothing to deregister.
        let all = [1u64, 2, 3];
        assert!(reg.closed_handles(|h| all.contains(h)).is_empty());

        // None live: everything is closed.
        let mut all_closed = reg.closed_handles(|_| false);
        all_closed.sort();
        assert_eq!(all_closed, vec![1, 2, 3]);
    }

    #[test]
    fn singleton_phase_defaults_to_closed_and_clears() {
        let mut reg: Registry<u64> = Registry::new();
        assert_eq!(reg.singleton_phase(SETTINGS), SingletonPhase::Closed);

        let metadata = SingletonMetadata::of::<u64>(RootPolicy::ComponentRoot);
        reg.begin_singleton(SETTINGS, metadata);
        assert_eq!(reg.singleton_phase(SETTINGS), SingletonPhase::Opening);

        reg.finish_singleton(SETTINGS, 42);
        assert_eq!(reg.singleton_phase(SETTINGS), SingletonPhase::Open(42));

        reg.clear_singleton(SETTINGS);
        assert_eq!(reg.singleton_phase(SETTINGS), SingletonPhase::Closed);
        // Keys are independent.
        assert_eq!(reg.singleton_phase(MAIN), SingletonPhase::Closed);
    }

    #[test]
    fn singleton_metadata_is_retained_for_opening_and_live_windows() {
        let mut reg: Registry<u64> = Registry::new();
        let metadata = SingletonMetadata::of::<String>(RootPolicy::ComponentRoot);
        reg.begin_singleton(SETTINGS, metadata);

        let opening = reg.singleton_metadata(SETTINGS).unwrap();
        assert_eq!(opening.content_type, TypeId::of::<String>());
        assert_eq!(opening.content_type_name, std::any::type_name::<String>());
        assert_eq!(opening.root_policy, RootPolicy::ComponentRoot);

        reg.finish_singleton(SETTINGS, 42);
        let open = reg.singleton_metadata(SETTINGS).unwrap();
        assert_eq!(open.content_type, TypeId::of::<String>());
        assert_eq!(open.root_policy, RootPolicy::ComponentRoot);
    }

    #[test]
    fn plan_singleton_closed_creates() {
        assert_eq!(
            plan_singleton::<u64>(SingletonPhase::Closed, false),
            SingletonAction::Create
        );
    }

    #[test]
    fn plan_singleton_opening_is_in_flight() {
        assert_eq!(
            plan_singleton::<u64>(SingletonPhase::Opening, true),
            SingletonAction::InFlight
        );
    }

    #[test]
    fn plan_singleton_open_reuses_when_alive_else_recreates() {
        assert_eq!(
            plan_singleton(SingletonPhase::Open(7u64), true),
            SingletonAction::Reuse(7)
        );
        assert_eq!(
            plan_singleton(SingletonPhase::Open(7u64), false),
            SingletonAction::Create
        );
    }
}
