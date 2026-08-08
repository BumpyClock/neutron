//! Atomic (and optionally crash-durable) file writes.
//!
//! Two guarantees, deliberately distinct:
//!
//! - **Atomic** ([`write_atomic`]): a reader opening the target path sees either
//!   the complete previous file or the complete new file, never a torn
//!   intermediate. Achieved by writing a uniquely-named temp file in the *same*
//!   directory and then renaming it over the target. `std::fs::rename` is atomic
//!   within one filesystem on Unix, and on Windows resolves to `MoveFileEx` with
//!   `MOVEFILE_REPLACE_EXISTING`, which replaces an existing destination in a
//!   single operation. The temp file must share the target's directory so the
//!   rename stays within one filesystem.
//! - **Durable** ([`write_atomic_durable`]): additionally `fsync`s the temp file
//!   before the rename and (on Unix) `fsync`s the parent directory after, so the
//!   replacement survives a power loss / kernel panic. This costs an extra disk
//!   flush per write and is only needed for data you cannot afford to lose to an
//!   abrupt machine failure. Plain [`write_atomic`] does not promise durability.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::StorageError;

/// Monotonic per-process counter guaranteeing distinct temp names even when two
/// writes land within the same nanosecond.
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Ensure the parent directory of `path` exists, creating it if necessary.
pub(crate) fn ensure_parent(path: &Path) -> Result<(), StorageError> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|e| StorageError::io(parent, e))?;
        }
    }
    Ok(())
}

/// Compute a unique temp path in the same directory as `path`.
fn temp_path_for(path: &Path) -> PathBuf {
    let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);

    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "tmp".to_string());
    let temp_name = format!(".{file_name}.tmp.{pid}.{counter}.{nanos}");

    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.join(temp_name),
        _ => PathBuf::from(temp_name),
    }
}

/// Write `bytes` to `path` atomically (no torn reads). See the module docs for
/// the atomic-vs-durable distinction — this does **not** promise crash durability.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), StorageError> {
    write_impl(path, bytes, false)
}

/// Write `bytes` to `path` atomically *and* durably: the file is `fsync`ed
/// before the rename and (on Unix) the parent directory is `fsync`ed after, so
/// the write survives power loss. Slower than [`write_atomic`].
pub fn write_atomic_durable(path: &Path, bytes: &[u8]) -> Result<(), StorageError> {
    write_impl(path, bytes, true)
}

fn write_impl(path: &Path, bytes: &[u8], durable: bool) -> Result<(), StorageError> {
    ensure_parent(path)?;
    let temp = temp_path_for(path);

    // Write the staging file. The handle is scoped inside the closure so it is
    // closed before any rename/remove — on Windows an open handle can block
    // replacing or deleting the file.
    let staged = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|e| StorageError::io(&temp, e))?;
        file.write_all(bytes)
            .map_err(|e| StorageError::io(&temp, e))?;
        if durable {
            file.sync_all().map_err(|e| StorageError::io(&temp, e))?;
        }
        Ok(())
    })();

    if let Err(e) = staged {
        // A failed write/sync (e.g. disk full) must not leave a partial temp
        // file behind. Best-effort cleanup; the create_new failure case leaves
        // nothing to remove.
        let _ = fs::remove_file(&temp);
        return Err(e);
    }

    // Preserve the target's restrictive permissions across the replacement. The
    // staging file was created under the process umask (typically `0644`), so
    // renaming it over a `0600` settings/secrets file would silently widen it to
    // world-readable. New files get a restrictive `0600` default.
    #[cfg(unix)]
    if let Err(e) = preserve_permissions(path, &temp) {
        let _ = fs::remove_file(&temp);
        return Err(e);
    }

    if let Err(e) = fs::rename(&temp, path) {
        // Clean up the orphaned temp file; ignore any failure removing it.
        let _ = fs::remove_file(&temp);
        return Err(StorageError::io(path, e));
    }

    if durable {
        sync_parent_dir(path)?;
    }

    Ok(())
}

/// Match the staging file's permissions to the target's before the rename, so an
/// atomic replacement never widens a restrictive mode. A not-yet-existing target
/// gets a restrictive `0600` default; genuine `stat` failures are propagated.
#[cfg(unix)]
fn preserve_permissions(target: &Path, temp: &Path) -> Result<(), StorageError> {
    use std::os::unix::fs::PermissionsExt;

    let mode = match fs::metadata(target) {
        Ok(meta) => meta.permissions().mode() & 0o777,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => 0o600,
        Err(e) => return Err(StorageError::io(target, e)),
    };
    fs::set_permissions(temp, fs::Permissions::from_mode(mode))
        .map_err(|e| StorageError::io(temp, e))
}

/// `fsync` the parent directory so the rename entry itself is durable.
///
/// On Unix, failures are propagated so [`write_atomic_durable`] never reports a
/// durability guarantee it did not achieve. On non-Unix this is a no-op: Windows
/// has no directory-fsync primitive, so the parent-entry durability of the
/// rename is left to the filesystem (the file contents were still synced).
fn sync_parent_dir(path: &Path) -> Result<(), StorageError> {
    #[cfg(unix)]
    {
        let parent = path.parent().unwrap_or_else(|| Path::new(""));
        let dir = if parent.as_os_str().is_empty() {
            Path::new(".")
        } else {
            parent
        };
        let f = fs::File::open(dir).map_err(|e| StorageError::io(dir, e))?;
        f.sync_all().map_err(|e| StorageError::io(dir, e))?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn writes_and_leaves_no_temp_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("data.bin");
        write_atomic(&path, b"hello").unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"hello");
        // No sibling temp files remain.
        let leftovers: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp."))
            .collect();
        assert!(leftovers.is_empty(), "temp files leaked: {leftovers:?}");
    }

    #[test]
    fn overwrite_replaces_atomically() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("data.bin");
        write_atomic(&path, b"first").unwrap();
        write_atomic(&path, b"second").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"second");
    }

    #[test]
    fn creates_missing_parent_dirs() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("a").join("b").join("data.bin");
        write_atomic(&path, b"x").unwrap();
        assert!(path.exists());
    }

    #[test]
    fn durable_round_trips() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("durable.bin");
        write_atomic_durable(&path, b"safe").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"safe");
    }

    #[cfg(unix)]
    #[test]
    fn preserves_restrictive_permissions_on_overwrite() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().unwrap();
        let path = dir.path().join("secret.bin");

        // A brand-new file is created with a restrictive default.
        write_atomic(&path, b"first").unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );

        // Tighten further, then overwrite: the mode must survive the rename.
        fs::set_permissions(&path, fs::Permissions::from_mode(0o400)).unwrap();
        write_atomic(&path, b"second").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"second");
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o400
        );
    }
}
