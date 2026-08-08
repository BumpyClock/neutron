//! Schema-versioned TOML envelope with explicit load outcomes.
//!
//! Every persisted document is wrapped in an [`Envelope`] that stamps a
//! `schema_version` alongside the inner payload (flattened, so the on-disk TOML
//! stays flat). On load, the version drives one of five explicit outcomes
//! ([`LoadOutcome`]) so callers can migrate, refuse, or recover deliberately —
//! in particular a *newer* on-disk version is never treated as corruption and
//! is never overwritten.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::atomic::{write_atomic, write_atomic_durable};
use crate::error::StorageError;

/// Owned envelope used when deserializing a current-version document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope<T> {
    /// On-disk schema version of the payload.
    pub schema_version: u32,
    /// The wrapped payload, flattened into the same TOML table.
    #[serde(flatten)]
    pub inner: T,
}

/// Borrowing envelope used for serialization so we never clone the payload.
#[derive(Serialize)]
struct EnvelopeRef<'a, T> {
    schema_version: u32,
    #[serde(flatten)]
    inner: &'a T,
}

/// The five distinct results of loading a versioned document.
#[derive(Debug)]
#[non_exhaustive]
pub enum LoadOutcome<T> {
    /// File is present, current-version, and deserialized cleanly.
    Loaded(T),
    /// File is present but an *older* version. Carries the raw TOML so the
    /// caller can run a migration chain.
    NeedsMigration {
        /// Version found on disk (`< current`).
        found: u32,
        /// The raw parsed document, for migration.
        raw: toml::Value,
    },
    /// File is present but a *newer* version. The file is left untouched — the
    /// caller must refuse to write so newer data is never clobbered.
    FutureVersion {
        /// Version found on disk (`> current`).
        found: u32,
    },
    /// File is present but malformed (unparsable or missing version). It has
    /// been archived aside per recovery policy.
    Corrupt {
        /// Where the malformed file was moved, if archiving succeeded.
        archived_to: Option<PathBuf>,
    },
    /// No file exists at the path.
    Missing,
}

/// Serialize `value` into a versioned TOML string.
pub fn serialize_envelope<T: Serialize>(
    value: &T,
    schema_version: u32,
) -> Result<String, StorageError> {
    let envelope = EnvelopeRef {
        schema_version,
        inner: value,
    };
    Ok(toml::to_string_pretty(&envelope)?)
}

/// Serialize and atomically write `value` (versioned) to `path`.
pub fn save_envelope<T: Serialize>(
    path: &Path,
    value: &T,
    schema_version: u32,
) -> Result<(), StorageError> {
    let contents = serialize_envelope(value, schema_version)?;
    write_atomic(path, contents.as_bytes())
}

/// Like [`save_envelope`] but crash-durable (`fsync`).
pub fn save_envelope_durable<T: Serialize>(
    path: &Path,
    value: &T,
    schema_version: u32,
) -> Result<(), StorageError> {
    let contents = serialize_envelope(value, schema_version)?;
    write_atomic_durable(path, contents.as_bytes())
}

/// Load and classify the document at `path` against `current_version`.
///
/// Genuine filesystem failures (other than "not found") are returned as `Err`;
/// everything else maps onto a [`LoadOutcome`]. A malformed file — unparsable
/// TOML *or* invalid UTF-8 bytes — is archived to `<path>.bak.v<version>` (or a
/// timestamped `.bak.corrupt.<nanos>` when the version is unknown) before
/// returning [`LoadOutcome::Corrupt`], so it stays backup-recoverable.
pub fn load_envelope<T: DeserializeOwned>(
    path: &Path,
    current_version: u32,
) -> Result<LoadOutcome<T>, StorageError> {
    // Read bytes rather than `read_to_string`: the latter reports invalid UTF-8
    // as `ErrorKind::InvalidData`, which would surface as an `Err` and bypass
    // the corrupt-archive/recovery path. A bad byte sequence is corruption, not
    // a filesystem error.
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(LoadOutcome::Missing),
        Err(e) => return Err(StorageError::io(path, e)),
    };

    let contents = match String::from_utf8(bytes) {
        Ok(s) => s,
        Err(_) => {
            let archived_to = archive_corrupt(path, None);
            return Ok(LoadOutcome::Corrupt { archived_to });
        }
    };

    let value: toml::Value = match toml::from_str(&contents) {
        Ok(v) => v,
        Err(_) => {
            let archived_to = archive_corrupt(path, None);
            return Ok(LoadOutcome::Corrupt { archived_to });
        }
    };

    let found = match value
        .get("schema_version")
        .and_then(toml::Value::as_integer)
    {
        // Values above `u32::MAX` cannot be smaller than any real (u32) current
        // version, so they are unambiguously "future": refuse to write, never
        // archive. We report `u32::MAX` rather than truncating the raw integer,
        // which would otherwise wrap to a small number and be misclassified as a
        // migration candidate.
        Some(n) if n > u32::MAX as i64 => {
            return Ok(LoadOutcome::FutureVersion { found: u32::MAX });
        }
        Some(n) => match u32::try_from(n) {
            Ok(v) => v,
            // Negative version: malformed, not a real schema.
            Err(_) => {
                let archived_to = archive_corrupt(path, None);
                return Ok(LoadOutcome::Corrupt { archived_to });
            }
        },
        None => {
            // No usable version field — treat as malformed.
            let archived_to = archive_corrupt(path, None);
            return Ok(LoadOutcome::Corrupt { archived_to });
        }
    };

    if found > current_version {
        return Ok(LoadOutcome::FutureVersion { found });
    }
    if found < current_version {
        return Ok(LoadOutcome::NeedsMigration { found, raw: value });
    }

    // Current version: deserialize the payload.
    match value.try_into::<Envelope<T>>() {
        Ok(envelope) => Ok(LoadOutcome::Loaded(envelope.inner)),
        Err(_) => {
            let archived_to = archive_corrupt(path, Some(found));
            Ok(LoadOutcome::Corrupt { archived_to })
        }
    }
}

/// Move a malformed file aside. Returns the archive path on success, `None` if
/// the rename failed (archiving is best-effort — losing the ability to archive
/// must not mask the corruption itself).
fn archive_corrupt(path: &Path, version: Option<u32>) -> Option<PathBuf> {
    let suffix = match version {
        Some(v) => format!(".bak.v{v}"),
        None => {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            format!(".bak.corrupt.{nanos}")
        }
    };
    let mut target = path.as_os_str().to_os_string();
    target.push(suffix);
    let target = PathBuf::from(target);

    match std::fs::rename(path, &target) {
        Ok(()) => Some(target),
        Err(e) => {
            log::warn!(
                "failed to archive corrupt file {} to {}: {e}",
                path.display(),
                target.display()
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};
    use tempfile::TempDir;

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct Doc {
        name: String,
        count: u32,
    }

    fn doc() -> Doc {
        Doc {
            name: "widget".to_string(),
            count: 7,
        }
    }

    #[test]
    fn missing_when_no_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("cfg.toml");
        let outcome = load_envelope::<Doc>(&path, 1).unwrap();
        assert!(matches!(outcome, LoadOutcome::Missing));
    }

    #[test]
    fn loaded_current_version_round_trips() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("cfg.toml");
        save_envelope(&path, &doc(), 3).unwrap();

        match load_envelope::<Doc>(&path, 3).unwrap() {
            LoadOutcome::Loaded(value) => assert_eq!(value, doc()),
            other => panic!("expected Loaded, got {other:?}"),
        }
    }

    #[test]
    fn older_version_needs_migration_with_raw() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("cfg.toml");
        std::fs::write(&path, "schema_version = 1\nname = \"old\"\ncount = 2\n").unwrap();

        match load_envelope::<Doc>(&path, 3).unwrap() {
            LoadOutcome::NeedsMigration { found, raw } => {
                assert_eq!(found, 1);
                assert_eq!(raw.get("name").unwrap().as_str(), Some("old"));
            }
            other => panic!("expected NeedsMigration, got {other:?}"),
        }
    }

    #[test]
    fn newer_version_is_preserved_untouched() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("cfg.toml");
        let on_disk = "schema_version = 9\nname = \"future\"\ncount = 100\n";
        std::fs::write(&path, on_disk).unwrap();

        match load_envelope::<Doc>(&path, 3).unwrap() {
            LoadOutcome::FutureVersion { found } => assert_eq!(found, 9),
            other => panic!("expected FutureVersion, got {other:?}"),
        }

        // A refused load must not modify or archive the newer file.
        assert_eq!(std::fs::read_to_string(&path).unwrap(), on_disk);
        assert!(!path.with_extension("toml.bak.v9").exists());
    }

    #[test]
    fn malformed_is_archived_and_reported_corrupt() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("cfg.toml");
        std::fs::write(&path, "this is = = not valid toml").unwrap();

        match load_envelope::<Doc>(&path, 3).unwrap() {
            LoadOutcome::Corrupt { archived_to } => {
                let archived = archived_to.expect("corrupt file should be archived");
                assert!(archived.exists());
                assert!(!path.exists(), "corrupt primary moved aside");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn missing_version_field_is_corrupt() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("cfg.toml");
        std::fs::write(&path, "name = \"x\"\ncount = 1\n").unwrap();
        assert!(matches!(
            load_envelope::<Doc>(&path, 1).unwrap(),
            LoadOutcome::Corrupt { .. }
        ));
    }

    #[test]
    fn version_above_u32_max_is_future_not_migration() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("cfg.toml");
        // u32::MAX + 1: naive `as u32` would wrap to 0 and look like an ancient
        // migration candidate; it must instead be treated as a future version.
        let on_disk = "schema_version = 4294967296\nname = \"x\"\ncount = 1\n";
        std::fs::write(&path, on_disk).unwrap();

        match load_envelope::<Doc>(&path, 3).unwrap() {
            LoadOutcome::FutureVersion { found } => assert_eq!(found, u32::MAX),
            other => panic!("expected FutureVersion, got {other:?}"),
        }
        // Untouched — a future version is never archived.
        assert_eq!(std::fs::read_to_string(&path).unwrap(), on_disk);
    }

    #[test]
    fn negative_version_is_corrupt() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("cfg.toml");
        std::fs::write(&path, "schema_version = -1\nname = \"x\"\ncount = 1\n").unwrap();
        assert!(matches!(
            load_envelope::<Doc>(&path, 3).unwrap(),
            LoadOutcome::Corrupt { .. }
        ));
    }

    #[test]
    fn invalid_utf8_is_corrupt_not_io_error() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("cfg.toml");
        // Invalid UTF-8 byte sequence: `read_to_string` would surface this as an
        // I/O error; it must instead be archived as corruption.
        std::fs::write(&path, [0xff, 0xfe, 0x00, 0x80]).unwrap();

        match load_envelope::<Doc>(&path, 3).unwrap() {
            LoadOutcome::Corrupt { archived_to } => {
                let archived = archived_to.expect("invalid-UTF-8 file should be archived");
                assert!(archived.exists());
                assert!(!path.exists(), "corrupt primary moved aside");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }
}
