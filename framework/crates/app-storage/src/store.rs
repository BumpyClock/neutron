//! Debounced, backup-rotating, single-writer store.
//!
//! [`DebouncedStore`] coalesces rapid [`put`](DebouncedStore::put) calls onto a
//! background worker thread that commits at most once per debounce window. Each
//! commit rotates the previous (known-valid) generation into a `.bak` chain and
//! writes the new document atomically. A [`WriterLock`] taken at construction
//! guarantees a single writer per store.
//!
//! # Persistence guarantees
//!
//! - [`flush`](DebouncedStore::flush) and [`shutdown`](DebouncedStore::shutdown)
//!   are synchronous and **return** the commit result — use them to persist at
//!   quit.
//! - `Drop` performs a best-effort flush only and swallows errors. It is **not**
//!   a persistence API; never rely on it for durability.
//! - Worker I/O/serialization errors are surfaced: a synchronous
//!   `flush`/`shutdown` returns them directly, and background (debounce-timer)
//!   failures are retained and readable via
//!   [`last_error`](DebouncedStore::last_error).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::atomic;
use crate::envelope::{LoadOutcome, load_envelope, serialize_envelope};
use crate::error::StorageError;
use crate::lock::WriterLock;

/// Idle timeout the worker parks at when nothing is pending.
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);

/// Configuration for a [`DebouncedStore`].
#[derive(Debug, Clone)]
pub struct StoreConfig {
    /// Current schema version stamped on every write.
    pub schema_version: u32,
    /// How long to coalesce writes before committing.
    pub debounce: Duration,
    /// Number of backup generations to retain (`.bak`, `.bak.1`, ...).
    pub backup_generations: usize,
    /// Whether commits are crash-durable (`fsync`).
    pub durable: bool,
}

impl StoreConfig {
    /// Sensible defaults: 500ms debounce, 3 backup generations, non-durable.
    pub fn new(schema_version: u32) -> Self {
        Self {
            schema_version,
            debounce: Duration::from_millis(500),
            backup_generations: 3,
            durable: false,
        }
    }

    /// Set the debounce window.
    pub fn debounce(mut self, debounce: Duration) -> Self {
        self.debounce = debounce;
        self
    }

    /// Set the number of retained backup generations.
    pub fn backup_generations(mut self, generations: usize) -> Self {
        self.backup_generations = generations;
        self
    }

    /// Enable crash-durable (`fsync`) commits.
    pub fn durable(mut self, durable: bool) -> Self {
        self.durable = durable;
        self
    }
}

/// Immutable commit parameters shared with the worker.
#[derive(Clone)]
struct Backend {
    path: PathBuf,
    schema_version: u32,
    backup_generations: usize,
    durable: bool,
}

impl Backend {
    fn commit<T: Serialize>(&self, value: &T) -> Result<(), StorageError> {
        atomic::ensure_parent(&self.path)?;
        let contents = serialize_envelope(value, self.schema_version)?;
        // Rotate first so a rotation failure aborts before the primary is
        // touched. Rotation *copies* the primary into `.bak` (rather than moving
        // it), so the primary stays intact and loadable if the write below then
        // fails — corrupt-primary recovery would otherwise never trigger for a
        // primary that had simply gone missing.
        rotate_backups(&self.path, self.backup_generations)?;
        if self.durable {
            atomic::write_atomic_durable(&self.path, contents.as_bytes())
        } else {
            atomic::write_atomic(&self.path, contents.as_bytes())
        }
    }
}

enum Command {
    Save,
    Flush(Sender<Result<(), StorageError>>),
    Shutdown(Sender<Result<(), StorageError>>),
}

/// Debounced single-writer store over a schema-versioned TOML file.
pub struct DebouncedStore<T: Serialize + DeserializeOwned + Send + 'static> {
    backend: Backend,
    tx: Sender<Command>,
    pending: Arc<Mutex<Option<T>>>,
    last_error: Arc<Mutex<Option<String>>>,
    worker: Option<JoinHandle<()>>,
    // NOTE: the `WriterLock` is intentionally *not* stored here. It is owned by
    // the worker thread and released only when that thread actually exits. This
    // keeps the single-writer guarantee honest when `shutdown` times out and
    // detaches a still-running worker: a wedged worker that is mid-`commit`
    // keeps the lock (so a second process gets `WriterConflict`) until it truly
    // finishes, rather than the lock being freed the instant this handle drops.
}

fn lock(mutex: &Mutex<Option<String>>) -> std::sync::MutexGuard<'_, Option<String>> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

fn pending_lock<T>(mutex: &Mutex<Option<T>>) -> std::sync::MutexGuard<'_, Option<T>> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

impl<T: Serialize + DeserializeOwned + Send + 'static> DebouncedStore<T> {
    /// Open a store at `path`, acquiring its writer lock and spawning the
    /// worker.
    ///
    /// Returns [`StorageError::WriterConflict`] if another writer already owns
    /// this store.
    pub fn open(path: impl AsRef<Path>, config: StoreConfig) -> Result<Self, StorageError> {
        let path = path.as_ref().to_path_buf();
        let lock_path = append_suffix(&path, ".lock");
        let writer_lock = WriterLock::acquire(&lock_path)?;

        let backend = Backend {
            path,
            schema_version: config.schema_version,
            backup_generations: config.backup_generations,
            durable: config.durable,
        };

        let (tx, rx) = mpsc::channel();
        let pending: Arc<Mutex<Option<T>>> = Arc::new(Mutex::new(None));
        let last_error: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));

        let worker_backend = backend.clone();
        let worker_pending = Arc::clone(&pending);
        let worker_last_error = Arc::clone(&last_error);
        let debounce = config.debounce;

        // Move lock ownership into the worker: it is released only when the
        // worker thread exits. If `spawn` fails, the closure (and with it the
        // lock) is dropped here, releasing the lock.
        let worker = std::thread::Builder::new()
            .name("gpui-storage".to_string())
            .spawn(move || {
                worker_loop(
                    rx,
                    worker_backend,
                    worker_pending,
                    worker_last_error,
                    debounce,
                    writer_lock,
                );
            })
            .map_err(|e| StorageError::io(&lock_path, e))?;

        Ok(Self {
            backend,
            tx,
            pending,
            last_error,
            worker: Some(worker),
        })
    }

    /// Load and classify the current on-disk document.
    ///
    /// If the primary file is [`Corrupt`](LoadOutcome::Corrupt), backups are
    /// probed newest-first and the **first usable** generation is surfaced —
    /// [`Loaded`](LoadOutcome::Loaded), [`NeedsMigration`](LoadOutcome::NeedsMigration)
    /// (a backup written under an older schema, common right after an upgrade),
    /// or [`FutureVersion`](LoadOutcome::FutureVersion) (a newer generation left
    /// after a downgrade). `Missing`/`Corrupt` backups are skipped. This ensures
    /// a corrupt primary never causes a caller to initialize defaults over a
    /// migratable or preserved newer generation. A non-corrupt primary passes
    /// through untouched.
    pub fn load(&self) -> Result<LoadOutcome<T>, StorageError> {
        match load_envelope::<T>(&self.backend.path, self.backend.schema_version)? {
            LoadOutcome::Corrupt { archived_to } => {
                for backup in backup_paths(&self.backend.path, self.backend.backup_generations) {
                    let usable = match load_envelope::<T>(&backup, self.backend.schema_version) {
                        Ok(LoadOutcome::Loaded(value)) => LoadOutcome::Loaded(value),
                        Ok(LoadOutcome::NeedsMigration { found, raw }) => {
                            LoadOutcome::NeedsMigration { found, raw }
                        }
                        Ok(LoadOutcome::FutureVersion { found }) => {
                            LoadOutcome::FutureVersion { found }
                        }
                        // A Missing or Corrupt backup is genuinely unusable: skip
                        // it and give the next generation a chance.
                        Ok(LoadOutcome::Missing | LoadOutcome::Corrupt { .. }) => continue,
                        // An I/O error (permissions, device) is NOT evidence the
                        // backup is absent. Swallowing it here would let the
                        // caller initialize defaults over still-recoverable data,
                        // so surface it instead.
                        Err(error) => return Err(error),
                    };
                    log::warn!(
                        "recovered {} from backup {}",
                        self.backend.path.display(),
                        backup.display()
                    );
                    return Ok(usable);
                }
                Ok(LoadOutcome::Corrupt { archived_to })
            }
            other => Ok(other),
        }
    }

    /// Schedule a debounced write of `value`. The latest value wins if several
    /// `put`s land within one debounce window.
    pub fn put(&self, value: T) -> Result<(), StorageError> {
        *pending_lock(&self.pending) = Some(value);
        self.tx
            .send(Command::Save)
            .map_err(|_| StorageError::WorkerStopped)
    }

    /// Synchronously commit any pending value now, returning the commit result.
    pub fn flush(&self) -> Result<(), StorageError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.tx
            .send(Command::Flush(reply_tx))
            .map_err(|_| StorageError::WorkerStopped)?;
        reply_rx.recv().map_err(|_| StorageError::WorkerStopped)?
    }

    /// Flush and shut down the worker, joining its thread. Bounded: the final
    /// commit reply is awaited for up to 5s. If the worker does not reply in
    /// time it is **detached** (not joined) and [`StorageError::WorkerStopped`]
    /// is returned, so a wedged worker can never hang the caller indefinitely.
    ///
    /// Because the writer lock is owned by the worker thread, a detached worker
    /// keeps the lock until it actually exits — a second process opening the
    /// same store observes [`StorageError::WriterConflict`] rather than racing a
    /// still-running writer. On a normal (non-timed-out) shutdown the worker
    /// exits and releases the lock before this returns.
    pub fn shutdown(mut self) -> Result<(), StorageError> {
        self.shutdown_inner()
    }

    /// The most recent background (debounce-timer) commit error, if any. Cleared
    /// on the next successful commit. Synchronous `flush`/`shutdown` errors are
    /// returned directly rather than only stored here.
    pub fn last_error(&self) -> Option<String> {
        lock(&self.last_error).clone()
    }

    fn shutdown_inner(&mut self) -> Result<(), StorageError> {
        let worker = match self.worker.take() {
            Some(w) => w,
            None => return Ok(()),
        };

        let (reply_tx, reply_rx) = mpsc::channel();

        match self.tx.send(Command::Shutdown(reply_tx)) {
            Ok(()) => match reply_rx.recv_timeout(Duration::from_secs(5)) {
                Ok(result) => {
                    // Worker acknowledged; the join is now bounded (it is exiting).
                    let _ = worker.join();
                    result
                }
                Err(_) => {
                    // Worker is unresponsive. Detach rather than block on an
                    // unbounded join — dropping the handle leaves it running.
                    Err(StorageError::WorkerStopped)
                }
            },
            // Channel already closed: the worker has stopped, so this join is
            // bounded.
            Err(_) => {
                let _ = worker.join();
                Err(StorageError::WorkerStopped)
            }
        }
    }
}

impl<T: Serialize + DeserializeOwned + Send + 'static> Drop for DebouncedStore<T> {
    fn drop(&mut self) {
        // Best-effort only; errors are intentionally swallowed. Callers that
        // need the result must use `flush`/`shutdown` explicitly.
        let _ = self.shutdown_inner();
    }
}

fn worker_loop<T: Serialize + Send + 'static>(
    rx: Receiver<Command>,
    backend: Backend,
    pending: Arc<Mutex<Option<T>>>,
    last_error: Arc<Mutex<Option<String>>>,
    debounce: Duration,
    // Owned for the worker's lifetime: the single-writer lock is released only
    // when this function returns (the thread exits), never before.
    _writer_lock: WriterLock,
) {
    let mut deadline: Option<Instant> = None;

    loop {
        let timeout = match deadline {
            Some(d) => d.saturating_duration_since(Instant::now()),
            None => IDLE_TIMEOUT,
        };

        match rx.recv_timeout(timeout) {
            Ok(Command::Save) => {
                deadline = Some(Instant::now() + debounce);
            }
            Ok(Command::Flush(reply)) => {
                let result = commit_pending(&backend, &pending, &last_error);
                deadline = None;
                let _ = reply.send(result);
            }
            Ok(Command::Shutdown(reply)) => {
                let result = commit_pending(&backend, &pending, &last_error);
                let _ = reply.send(result);
                break;
            }
            Err(RecvTimeoutError::Timeout) => {
                if let Some(d) = deadline {
                    if Instant::now() >= d {
                        // Background commit: no caller is waiting, so record any
                        // failure in `last_error` for later inspection.
                        let _ = commit_pending(&backend, &pending, &last_error);
                        deadline = None;
                    }
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                let _ = commit_pending(&backend, &pending, &last_error);
                break;
            }
        }
    }
}

/// Commit the pending snapshot (if any) and update `last_error`. On failure the
/// snapshot is restored to `pending` so a later `flush`/`put` can retry rather
/// than silently dropping the user's data.
fn commit_pending<T: Serialize>(
    backend: &Backend,
    pending: &Arc<Mutex<Option<T>>>,
    last_error: &Arc<Mutex<Option<String>>>,
) -> Result<(), StorageError> {
    let value = match pending_lock(pending).take() {
        Some(v) => v,
        None => return Ok(()),
    };

    match backend.commit(&value) {
        Ok(()) => {
            *lock(last_error) = None;
            Ok(())
        }
        Err(e) => {
            restore_failed_value(pending, value);
            *lock(last_error) = Some(e.to_string());
            Err(e)
        }
    }
}

/// Put a failed commit's value back into `pending` for retry — but only if no
/// newer value arrived while the commit was in flight. `take()` released the
/// lock during the (slow) write, so a concurrent `put(newer)` may have already
/// installed a fresher snapshot; overwriting it would silently lose that update.
fn restore_failed_value<T>(pending: &Arc<Mutex<Option<T>>>, value: T) {
    let mut guard = pending_lock(pending);
    if guard.is_none() {
        *guard = Some(value);
    }
    // else: a newer put() already replaced it — keep the newer value, drop this.
}

/// Append a raw suffix to a full path (unlike `with_extension`, which strips the
/// existing extension). `settings.toml` + `.lock` -> `settings.toml.lock`.
fn append_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(suffix);
    PathBuf::from(s)
}

/// Backup path for generation index `i` (`0` -> `.bak`, `1` -> `.bak.1`, ...).
fn backup_path(path: &Path, i: usize) -> PathBuf {
    if i == 0 {
        append_suffix(path, ".bak")
    } else {
        append_suffix(path, &format!(".bak.{i}"))
    }
}

/// All backup paths, newest (`.bak`) first.
fn backup_paths(path: &Path, generations: usize) -> Vec<PathBuf> {
    (0..generations).map(|i| backup_path(path, i)).collect()
}

/// Rotate the existing primary into the `.bak` chain before it is overwritten.
///
/// The primary is only ever written by us with valid content, so `.bak` is
/// always a previously-committed valid generation. The primary is **copied**
/// (not moved) into `.bak` so it remains in place until the new primary is
/// committed — a failed write then leaves the old primary intact rather than
/// stranding the data only in a backup. A no-op when no primary exists yet
/// (first write). Errors are surfaced so the caller can abort the commit.
fn rotate_backups(path: &Path, generations: usize) -> Result<(), StorageError> {
    if generations == 0 || !path.exists() {
        return Ok(());
    }

    // Drop the oldest generation if present.
    let oldest = backup_path(path, generations - 1);
    if oldest.exists() {
        fs::remove_file(&oldest).map_err(|e| StorageError::io(&oldest, e))?;
    }
    // Shift existing backups down by one (these are already-committed copies, so
    // moving them is safe).
    for i in (1..generations).rev() {
        let src = backup_path(path, i - 1);
        if src.exists() {
            let dst = backup_path(path, i);
            fs::rename(&src, &dst).map_err(|e| StorageError::io(&dst, e))?;
        }
    }
    // Copy — not move — the current primary into `.bak`.
    let newest = backup_path(path, 0);
    fs::copy(path, &newest).map_err(|e| StorageError::io(&newest, e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restore_installs_failed_value_when_pending_empty() {
        let pending: Arc<Mutex<Option<u32>>> = Arc::new(Mutex::new(None));
        restore_failed_value(&pending, 1);
        assert_eq!(*pending_lock(&pending), Some(1));
    }

    #[test]
    fn restore_does_not_clobber_newer_pending() {
        // Models a put(2) that landed while value 1 was being (unsuccessfully)
        // committed: the newer value must survive the failed retry restore.
        let pending: Arc<Mutex<Option<u32>>> = Arc::new(Mutex::new(Some(2)));
        restore_failed_value(&pending, 1);
        assert_eq!(*pending_lock(&pending), Some(2));
    }
}
