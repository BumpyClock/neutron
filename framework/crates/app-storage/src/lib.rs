//! Foundation storage layer for gpui-component apps.
//!
//! Pure, GPUI-free building blocks for persisting application data on disk:
//!
//! - [`paths`] — resolve per-app directories from a stable namespace, under
//!   either platform-native locations or a single home-relative root.
//! - [`atomic`] — atomic file writes (no torn reads), with an opt-in
//!   crash-durable variant.
//! - [`envelope`] — schema-versioned TOML documents with five explicit load
//!   outcomes (loaded / needs-migration / future-version / corrupt / missing).
//! - [`lock`] — a single-writer OS advisory lock so two instances cannot
//!   silently clobber each other.
//! - [`store`] — [`DebouncedStore`], a debounced, backup-rotating,
//!   single-writer store built from the pieces above.
//!
//! See `docs/learned/app-platform-plan.md` §4a for the contract this implements.

pub mod atomic;
pub mod envelope;
pub mod error;
pub mod lock;
pub mod paths;
pub mod store;

pub use atomic::{write_atomic, write_atomic_durable};
pub use envelope::{Envelope, LoadOutcome};
pub use error::StorageError;
pub use lock::WriterLock;
pub use paths::{AppPaths, BaseDir, PathLayout};
pub use store::{DebouncedStore, StoreConfig};
