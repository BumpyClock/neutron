//! Stable, non-localized window identity (plan §3 — "titles are not persistence
//! keys").
//!
//! A [`WindowKey`] names a window *role* (`"main"`, `"settings"`) independently
//! of its user-facing title. The manager keys singleton state and registry
//! bookkeeping on it, so a translated or numbered title never changes identity.

use std::fmt;

/// A stable, non-localized identifier for a window role.
///
/// Wraps a `&'static str` so keys are cheap to copy/compare and cannot drift
/// with locale. Distinct from the window *title*, which is user-facing and may
/// carry a numbering suffix (`"App - 2"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WindowKey(&'static str);

impl WindowKey {
    /// Create a key from a compiled-in identifier.
    pub const fn new(id: &'static str) -> Self {
        Self(id)
    }

    /// The underlying identifier.
    pub const fn as_str(&self) -> &'static str {
        self.0
    }
}

impl fmt::Display for WindowKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl From<&'static str> for WindowKey {
    fn from(id: &'static str) -> Self {
        Self::new(id)
    }
}
