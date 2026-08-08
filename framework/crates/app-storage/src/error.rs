//! Error type shared across the storage crate.

use std::path::PathBuf;

use thiserror::Error;

/// Errors produced by the storage layer.
///
/// Every variant carries enough context (usually the offending path) to be
/// actionable without a separate log line, so callers can surface failures to
/// the user directly.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum StorageError {
    /// An I/O operation failed against a specific path.
    #[error("i/o error at {path}: {source}")]
    Io {
        /// Path the failing operation targeted.
        path: PathBuf,
        /// Underlying OS error.
        #[source]
        source: std::io::Error,
    },

    /// Serializing a value to TOML failed.
    #[error("failed to serialize to TOML: {0}")]
    Serialize(#[from] toml::ser::Error),

    /// Deserializing a value from TOML failed.
    #[error("failed to deserialize TOML: {0}")]
    Deserialize(#[from] toml::de::Error),

    /// A platform directory could not be resolved for the given namespace.
    #[error("could not resolve {kind} directory for namespace {namespace:?}")]
    PathResolution {
        /// Which logical directory failed to resolve (e.g. `"config"`).
        kind: &'static str,
        /// The app namespace that was being resolved.
        namespace: String,
    },

    /// Another process (or another in-process store) already holds the
    /// single-writer lock for this store.
    #[error("another writer holds the lock for {path}{}", owner_pid.map(|p| format!(" (owner pid {p})")).unwrap_or_default())]
    WriterConflict {
        /// The lock file that is already held.
        path: PathBuf,
        /// PID recorded in the lock file, if it could be read.
        owner_pid: Option<u32>,
    },

    /// The on-disk schema is newer than this build supports; the caller must
    /// refuse to write so the newer data is never clobbered.
    #[error(
        "refusing to write: on-disk schema version {found} is newer than supported {supported}"
    )]
    FutureVersion {
        /// Version found on disk.
        found: u32,
        /// Highest version this build understands.
        supported: u32,
    },

    /// The background store worker has stopped and can no longer accept work.
    #[error("storage worker has stopped")]
    WorkerStopped,
}

impl StorageError {
    /// Build an [`StorageError::Io`] tagged with the path it occurred at.
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        StorageError::Io {
            path: path.into(),
            source,
        }
    }
}
