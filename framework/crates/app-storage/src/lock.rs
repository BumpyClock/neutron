//! Single-writer guard for a store.
//!
//! # Mechanism
//!
//! A sibling lock file (`<store>.lock`) is opened and an **OS advisory lock**
//! taken on it via `std::fs::File::try_lock` (`flock(LOCK_EX|LOCK_NB)` on Unix,
//! `LockFileEx(LOCKFILE_EXCLUSIVE_LOCK|LOCKFILE_FAIL_IMMEDIATELY)` on Windows).
//! The lock is held for the lifetime of the [`WriterLock`] and released on drop
//! — including on process exit or crash, since the OS drops the lock when the
//! owning process's file handles close. This is why an advisory OS lock is used
//! in preference to a hand-rolled PID file: there is **no stale-lock problem**
//! and no liveness probing to get wrong across platforms.
//!
//! # Failure mode
//!
//! The lock is *advisory*: it only excludes other writers that also go through
//! this API (i.e. other instances of an app built on this crate). It does not
//! stop an unrelated process from clobbering the file with raw `write`. Two app
//! instances contending produce [`StorageError::WriterConflict`] for the loser,
//! never a silent last-writer-wins. The PID of the current holder is written
//! into the lock file purely for diagnostics and surfaced in the error when
//! readable.

use std::fs::{File, OpenOptions, TryLockError};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use crate::error::StorageError;

/// Held single-writer lock. Releasing happens on drop.
#[derive(Debug)]
pub struct WriterLock {
    #[allow(dead_code)]
    path: PathBuf,
    file: File,
}

impl WriterLock {
    /// Acquire the writer lock at `lock_path`, creating the file if needed.
    ///
    /// Returns [`StorageError::WriterConflict`] if another writer already holds
    /// it.
    pub fn acquire(lock_path: &Path) -> Result<Self, StorageError> {
        if let Some(parent) = lock_path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| StorageError::io(parent, e))?;
            }
        }

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)
            .map_err(|e| StorageError::io(lock_path, e))?;

        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                let owner_pid = std::fs::read_to_string(lock_path)
                    .ok()
                    .and_then(|s| s.trim().parse().ok());
                return Err(StorageError::WriterConflict {
                    path: lock_path.to_path_buf(),
                    owner_pid,
                });
            }
            Err(TryLockError::Error(e)) => return Err(StorageError::io(lock_path, e)),
        }

        // Record our PID for diagnostics; failure here is non-fatal.
        let _ = write_pid(&file);

        Ok(Self {
            path: lock_path.to_path_buf(),
            file,
        })
    }
}

impl Drop for WriterLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

fn write_pid(file: &File) -> std::io::Result<()> {
    let mut handle = file;
    handle.set_len(0)?;
    handle.seek(SeekFrom::Start(0))?;
    write!(handle, "{}", std::process::id())?;
    handle.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn second_acquire_conflicts_then_releases_on_drop() {
        let dir = TempDir::new().unwrap();
        let lock_path = dir.path().join("store.lock");

        let held = WriterLock::acquire(&lock_path).unwrap();

        match WriterLock::acquire(&lock_path) {
            Err(StorageError::WriterConflict { owner_pid, .. }) => {
                // The holder PID is a best-effort diagnostic: on Windows the
                // exclusive lock also blocks reads from other handles, so the
                // conflicting acquirer may not be able to read it at all.
                #[cfg(unix)]
                assert_eq!(owner_pid, Some(std::process::id()));
                #[cfg(windows)]
                assert!(owner_pid.is_none() || owner_pid == Some(std::process::id()));
            }
            other => panic!("expected WriterConflict, got {other:?}"),
        }

        drop(held);
        // Once released, a fresh acquisition succeeds.
        let _reacquired = WriterLock::acquire(&lock_path).unwrap();
    }
}
