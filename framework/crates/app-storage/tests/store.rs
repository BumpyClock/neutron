//! Integration tests for `DebouncedStore` exercised through its public API.

use std::time::{Duration, Instant};

use gpui_component_storage::{DebouncedStore, LoadOutcome, StoreConfig};
use serde::{Deserialize, Serialize};
use tempfile::TempDir;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Doc {
    label: String,
    n: u64,
}

fn doc(label: &str, n: u64) -> Doc {
    Doc {
        label: label.to_string(),
        n,
    }
}

fn cfg() -> StoreConfig {
    StoreConfig::new(1).debounce(Duration::from_millis(50))
}

fn wait_for(mut cond: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if cond() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("condition not met within timeout");
}

fn load_value(store: &DebouncedStore<Doc>) -> Option<Doc> {
    match store.load().unwrap() {
        LoadOutcome::Loaded(v) => Some(v),
        _ => None,
    }
}

#[test]
fn put_then_flush_persists() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("store.toml");
    let store = DebouncedStore::<Doc>::open(&path, cfg()).unwrap();

    store.put(doc("a", 1)).unwrap();
    store.flush().unwrap();

    assert_eq!(load_value(&store), Some(doc("a", 1)));
}

#[test]
fn rapid_puts_coalesce_to_latest() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("store.toml");
    let store = DebouncedStore::<Doc>::open(&path, cfg()).unwrap();

    // Several puts inside one debounce window: only the last survives.
    store.put(doc("first", 1)).unwrap();
    store.put(doc("second", 2)).unwrap();
    store.put(doc("third", 3)).unwrap();
    store.flush().unwrap();

    assert_eq!(load_value(&store), Some(doc("third", 3)));
}

#[test]
fn background_debounce_writes_without_explicit_flush() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("store.toml");
    let store = DebouncedStore::<Doc>::open(&path, cfg()).unwrap();

    store.put(doc("bg", 42)).unwrap();
    // No flush: the debounce timer must commit on its own.
    wait_for(|| path.exists());
    assert_eq!(load_value(&store), Some(doc("bg", 42)));
}

#[test]
fn shutdown_flushes_pending_synchronously() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("store.toml");
    let store = DebouncedStore::<Doc>::open(&path, cfg()).unwrap();

    store.put(doc("quit", 9)).unwrap();
    store.shutdown().unwrap();

    // Re-open (the previous writer released its lock) and confirm persistence.
    let reopened = DebouncedStore::<Doc>::open(&path, cfg()).unwrap();
    assert_eq!(load_value(&reopened), Some(doc("quit", 9)));
}

#[test]
fn backup_generations_rotate() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("store.toml");
    let store = DebouncedStore::<Doc>::open(&path, cfg()).unwrap();

    for i in 0..4 {
        store.put(doc("gen", i)).unwrap();
        store.flush().unwrap();
    }

    let bak = |suffix: &str| {
        let mut s = path.as_os_str().to_os_string();
        s.push(suffix);
        std::path::PathBuf::from(s)
    };

    // Existence alone passes even if rotation duplicated or misordered
    // generations, so assert each file's actual value. The newest generation is
    // the primary and each `.bak*` holds the next-older value. Backups are read
    // by pointing a fresh store at their path (their lock is independent of the
    // still-open primary store).
    let read_backup = |p: std::path::PathBuf| {
        let store = DebouncedStore::<Doc>::open(&p, cfg()).unwrap();
        let value = load_value(&store);
        store.shutdown().unwrap();
        value
    };
    assert_eq!(load_value(&store), Some(doc("gen", 3)), "primary");
    assert_eq!(read_backup(bak(".bak")), Some(doc("gen", 2)), ".bak");
    assert_eq!(read_backup(bak(".bak.1")), Some(doc("gen", 1)), ".bak.1");
    assert_eq!(read_backup(bak(".bak.2")), Some(doc("gen", 0)), ".bak.2");
}

#[test]
fn corrupt_primary_recovers_from_backup() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("store.toml");
    let store = DebouncedStore::<Doc>::open(&path, cfg()).unwrap();

    // Two committed generations: newest primary + one `.bak`.
    store.put(doc("good-old", 1)).unwrap();
    store.flush().unwrap();
    store.put(doc("good-new", 2)).unwrap();
    store.flush().unwrap();

    // Corrupt the primary in place.
    std::fs::write(&path, "@@ not toml @@").unwrap();

    // load() falls back to the newest valid backup (the previous generation).
    match store.load().unwrap() {
        LoadOutcome::Loaded(v) => assert_eq!(v, doc("good-old", 1)),
        other => panic!("expected recovery, got {other:?}"),
    }
}

#[test]
fn corrupt_primary_surfaces_migratable_backup() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("store.toml");
    // Current schema is 2; a backup left over from before the upgrade is at
    // schema 1, i.e. NeedsMigration relative to current.
    let store = DebouncedStore::<Doc>::open(&path, StoreConfig::new(2)).unwrap();

    // Hand-place an older-schema `.bak` generation.
    let bak = {
        let mut s = path.as_os_str().to_os_string();
        s.push(".bak");
        std::path::PathBuf::from(s)
    };
    std::fs::write(&bak, "schema_version = 1\nlabel = \"legacy\"\nn = 5\n").unwrap();

    // Corrupt the primary.
    std::fs::write(&path, "@@ not toml @@").unwrap();

    // Recovery must surface the migratable generation, not fall through to
    // Corrupt (which would let a caller init defaults over real data).
    match store.load().unwrap() {
        LoadOutcome::NeedsMigration { found, raw } => {
            assert_eq!(found, 1);
            assert_eq!(raw.get("label").and_then(|v| v.as_str()), Some("legacy"));
        }
        other => panic!("expected NeedsMigration from backup, got {other:?}"),
    }
}

#[cfg(unix)]
#[test]
fn worker_write_error_is_surfaced() {
    use std::os::unix::fs::PermissionsExt;

    let dir = TempDir::new().unwrap();
    let sub = dir.path().join("locked");
    std::fs::create_dir_all(&sub).unwrap();
    let path = sub.join("store.toml");
    let store = DebouncedStore::<Doc>::open(&path, cfg()).unwrap();

    // First commit succeeds.
    store.put(doc("ok", 1)).unwrap();
    store.flush().unwrap();

    // Make the containing directory read-only so the next atomic write fails.
    let mut perms = std::fs::metadata(&sub).unwrap().permissions();
    perms.set_mode(0o500);
    std::fs::set_permissions(&sub, perms).unwrap();

    store.put(doc("fails", 2)).unwrap();
    let flush_result = store.flush();

    // Restore permissions before any assertion so cleanup always works.
    let mut restore = std::fs::metadata(&sub).unwrap().permissions();
    restore.set_mode(0o700);
    std::fs::set_permissions(&sub, restore).unwrap();

    assert!(
        flush_result.is_err(),
        "flush should surface the write error"
    );
    assert!(
        store.last_error().is_some(),
        "last_error should retain the failure"
    );
}

#[cfg(unix)]
#[test]
fn failed_commit_leaves_primary_intact_and_loadable() {
    use std::os::unix::fs::PermissionsExt;

    let dir = TempDir::new().unwrap();
    let sub = dir.path().join("locked");
    std::fs::create_dir_all(&sub).unwrap();
    let path = sub.join("store.toml");
    let store = DebouncedStore::<Doc>::open(&path, cfg()).unwrap();

    // Commit a good generation.
    store.put(doc("keep", 1)).unwrap();
    store.flush().unwrap();

    // Make the directory read-only so rotation/write of the next commit fails.
    let mut perms = std::fs::metadata(&sub).unwrap().permissions();
    perms.set_mode(0o500);
    std::fs::set_permissions(&sub, perms).unwrap();

    store.put(doc("doomed", 2)).unwrap();
    let flush_result = store.flush();

    // Assert while the directory is still read-only: this guarantees the worker
    // cannot background-commit "doomed" and race the assertions below. Reads are
    // still permitted (r-x), so load() works.
    assert!(flush_result.is_err(), "the commit must fail");
    assert!(
        path.exists(),
        "primary must remain in place after a failed commit"
    );
    assert_eq!(load_value(&store), Some(doc("keep", 1)));

    // Restore permissions so TempDir cleanup and the Drop-time flush can run.
    let mut restore = std::fs::metadata(&sub).unwrap().permissions();
    restore.set_mode(0o700);
    std::fs::set_permissions(&sub, restore).unwrap();
}
