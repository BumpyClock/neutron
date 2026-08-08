//! Process-level single-writer concurrency test.
//!
//! Proves that two *separate processes* contending on one store produce a
//! `WriterConflict` for the loser rather than silently clobbering each other.
//! The test re-execs its own binary in two child roles (holder / contender)
//! selected via the `STORAGE_LOCK_ROLE` env var. The role tests are `#[ignore]`d
//! so they never run as part of a normal `cargo test`; the driver invokes them
//! explicitly with `--ignored --exact`.

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use gpui_component_storage::{DebouncedStore, StoreConfig};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
struct Blob {
    n: u64,
}

const ENV_PATH: &str = "STORAGE_LOCK_PATH";
const ENV_ROLE: &str = "STORAGE_LOCK_ROLE";

fn store_path() -> PathBuf {
    PathBuf::from(std::env::var(ENV_PATH).expect("STORAGE_LOCK_PATH must be set"))
}

fn release_path(path: &std::path::Path) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(".release");
    PathBuf::from(s)
}

/// Child role: acquire the store, announce it, hold the lock until released.
#[test]
#[ignore = "child role invoked by the driver via re-exec"]
fn child_holder() {
    if std::env::var(ENV_ROLE).as_deref() != Ok("holder") {
        return;
    }
    let path = store_path();
    let store = match DebouncedStore::<Blob>::open(&path, StoreConfig::new(1)) {
        Ok(store) => store,
        Err(e) => {
            println!("HOLDER_FAILED:{e}");
            return;
        }
    };
    store.put(Blob { n: 1 }).unwrap();
    store.flush().unwrap();

    println!("HOLDER_LOCKED");
    use std::io::Write;
    std::io::stdout().flush().unwrap();

    // Hold the lock until the driver signals release (bounded so a stuck driver
    // can never leave a zombie behind).
    let release = release_path(&path);
    let deadline = Instant::now() + Duration::from_secs(30);
    while !release.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    drop(store);
}

/// Child role: try to acquire the already-held store and report the outcome.
#[test]
#[ignore = "child role invoked by the driver via re-exec"]
fn child_contender() {
    if std::env::var(ENV_ROLE).as_deref() != Ok("contender") {
        return;
    }
    let path = store_path();
    match DebouncedStore::<Blob>::open(&path, StoreConfig::new(1)) {
        Ok(_) => println!("CONTENDER_ACQUIRED"),
        Err(gpui_component_storage::StorageError::WriterConflict { .. }) => {
            println!("CONTENDER_CONFLICT")
        }
        Err(e) => println!("CONTENDER_ERR:{e}"),
    }
    use std::io::Write;
    std::io::stdout().flush().unwrap();
}

fn spawn_role(role: &str, test_name: &str, path: &std::path::Path) -> Command {
    let exe = std::env::current_exe().expect("current_exe");
    let mut cmd = Command::new(exe);
    cmd.args(["--exact", test_name, "--ignored", "--nocapture"])
        .env(ENV_ROLE, role)
        .env(ENV_PATH, path)
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    cmd
}

#[test]
fn process_level_writer_conflict() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("shared.toml");

    // Start the holder and wait until it confirms it owns the lock.
    let mut holder = spawn_role("holder", "child_holder", &path)
        .spawn()
        .expect("spawn holder");
    let holder_out = holder.stdout.take().unwrap();
    let mut reader = BufReader::new(holder_out);

    let locked = {
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut got = false;
        let mut line = String::new();
        while Instant::now() < deadline {
            line.clear();
            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                break; // holder exited without locking
            }
            if line.contains("HOLDER_LOCKED") {
                got = true;
                break;
            }
            if line.contains("HOLDER_FAILED") {
                break;
            }
        }
        got
    };
    assert!(locked, "holder failed to acquire the lock");

    // With the holder owning the lock, a second process must be refused.
    let contender = spawn_role("contender", "child_contender", &path)
        .output()
        .expect("run contender");
    let contender_stdout = String::from_utf8_lossy(&contender.stdout);

    // Signal release and reap the holder regardless of assertion outcome.
    std::fs::write(release_path(&path), b"go").unwrap();
    let _ = holder.wait();

    assert!(
        contender_stdout.contains("CONTENDER_CONFLICT"),
        "second process should hit WriterConflict, got: {contender_stdout}"
    );
}
