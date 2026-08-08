use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use gpui_component_manifest::schema::IdentityRef;
use gpui_component_storage::PathLayout;
use tempfile::TempDir;

use crate::capabilities::PlatformCapabilities;
use crate::error::RuntimeOperation;
use crate::handles::{self, AppInfo, PendingEvents};
use crate::liveness::{ExitPolicy, InitialActivation, Liveness};
use crate::phases::PhaseTracker;
use crate::plugin::AppPlugin;

use super::runtime::{ErasedSettingsEntry, ExitFlushHook};
use super::*;

#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
struct TestSettings {
    value: String,
    stage: u32,
}

impl AppSettings for TestSettings {
    const SCHEMA_VERSION: u32 = 3;
}

fn test_identity() -> IdentityRef {
    IdentityRef {
        app_id: "com.example.settings-test",
        display_name: "Settings Test",
        data_namespace: "settings-test",
        binary_name: None,
        org: None,
        publisher: None,
        url_schemes: &[],
        categories: &[],
        macos: None,
        linux: None,
        windows: None,
        legacy_ids: &[],
        min_os: None,
        version: "0.0.0",
        cfbundle_short_version: "0.0.0",
        msix_version: "0.0.0.0",
    }
}

fn install_error_reporter(cx: &mut gpui::App, reports: Arc<AtomicUsize>) {
    handles::install(
        cx,
        AppInfo::new(
            test_identity(),
            AppPaths::new("gpui-settings-reporter", PathLayout::PlatformDefault)
                .expect("test paths resolve"),
            PlatformCapabilities::detect(),
        ),
        Liveness::new(ExitPolicy::Explicit, InitialActivation::Passive),
        Vec::new(),
        Vec::new(),
        Arc::new(PendingEvents::default()),
        HashMap::new(),
        PhaseTracker::new(),
        Box::new(move |error, _| {
            assert_eq!(error.operation(), RuntimeOperation::Service);
            assert!(error.source_error().to_string().contains("timed out"));
            reports.fetch_add(1, Ordering::SeqCst);
        }),
    );
}

struct BlockedFlush {
    attempts: Arc<AtomicUsize>,
    started: Arc<Mutex<Option<Instant>>>,
    entered_rx: mpsc::Receiver<()>,
    release_tx: mpsc::Sender<()>,
    finished_rx: mpsc::Receiver<()>,
}

impl BlockedFlush {
    fn new() -> (Self, ExitFlushHook) {
        let attempts = Arc::new(AtomicUsize::new(0));
        let started = Arc::new(Mutex::new(None));
        let (entered_tx, entered_rx) = mpsc::sync_channel(1);
        let (release_tx, release_rx) = mpsc::channel();
        let release_rx = Arc::new(Mutex::new(release_rx));
        let (finished_tx, finished_rx) = mpsc::sync_channel(1);
        let hook_attempts = Arc::clone(&attempts);
        let hook_started = Arc::clone(&started);
        let hook = Arc::new(move || {
            hook_attempts.fetch_add(1, Ordering::SeqCst);
            *hook_started.lock().expect("exit flush start time poisoned") = Some(Instant::now());
            entered_tx
                .send(())
                .expect("exit flush entry receiver dropped");
            release_rx
                .lock()
                .expect("exit flush release receiver poisoned")
                .recv()
                .expect("exit flush release sender dropped");
            finished_tx
                .send(())
                .expect("exit flush completion receiver dropped");
            Ok(())
        });
        (
            Self {
                attempts,
                started,
                entered_rx,
                release_tx,
                finished_rx,
            },
            hook,
        )
    }

    fn wait_until_entered(&self) -> Instant {
        self.entered_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("exit flush worker did not enter storage");
        self.started
            .lock()
            .expect("exit flush start time poisoned")
            .expect("exit flush worker start time missing")
    }

    fn release(&self) {
        self.release_tx.send(()).expect("release exit flush worker");
    }

    fn wait_finished(&self) {
        self.finished_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("exit flush worker did not finish after release");
    }
}

struct TempPaths {
    paths: AppPaths,
    _root: TempDir,
}

fn temp_paths() -> TempPaths {
    let probe = AppPaths::new(
        "gpui-settings-probe",
        PathLayout::SingleRoot(".gpui-settings-probe".to_string()),
    )
    .unwrap();
    let home = probe
        .config_dir()
        .parent()
        .and_then(std::path::Path::parent)
        .unwrap();
    let root = tempfile::Builder::new()
        .prefix(".gpui-settings-")
        .tempdir_in(home)
        .unwrap();
    let root_name = root.path().file_name().unwrap().to_str().unwrap();
    let paths = AppPaths::new(
        "gpui-settings-test",
        PathLayout::SingleRoot(root_name.to_string()),
    )
    .unwrap();
    assert_eq!(paths.config_dir(), root.path().join("config"));
    TempPaths { paths, _root: root }
}

static MIGRATION_STEPS: AtomicUsize = AtomicUsize::new(0);

fn migrate_1_to_2(mut raw: toml::Value) -> Result<toml::Value, String> {
    assert_eq!(raw["value"].as_str(), Some("one"));
    raw["stage"] = toml::Value::Integer(2);
    MIGRATION_STEPS.fetch_add(1, Ordering::SeqCst);
    Ok(raw)
}

fn migrate_2_to_3(mut raw: toml::Value) -> Result<toml::Value, String> {
    assert_eq!(raw["stage"].as_integer(), Some(2));
    raw["stage"] = toml::Value::Integer(3);
    MIGRATION_STEPS.fetch_add(1, Ordering::SeqCst);
    Ok(raw)
}

#[test]
fn migration_chain_runs_each_intermediate_step() {
    MIGRATION_STEPS.store(0, Ordering::SeqCst);
    let temp = temp_paths();
    std::fs::create_dir_all(temp.paths.config_dir()).unwrap();
    std::fs::write(
        temp.paths.config_dir().join("settings.toml"),
        "schema_version = 1\nvalue = \"one\"\nstage = 1\n",
    )
    .unwrap();

    let entry = SettingsPlugin::<TestSettings>::new(StoreKey::PRIMARY)
        .migrate(1, 2, migrate_1_to_2)
        .migrate(2, 3, migrate_2_to_3)
        .open_entry(&temp.paths)
        .unwrap();

    assert_eq!(MIGRATION_STEPS.load(Ordering::SeqCst), 2);
    assert_eq!(entry.value.stage, 3);
    entry.store.flush().unwrap();
}

#[test]
fn future_version_refuses_update_and_preserves_bytes() {
    let temp = temp_paths();
    std::fs::create_dir_all(temp.paths.config_dir()).unwrap();
    let path = temp.paths.config_dir().join("settings.toml");
    let bytes = b"schema_version = 99\nvalue = \"future\"\nstage = 99\n";
    std::fs::write(&path, bytes).unwrap();

    let mut entry = SettingsPlugin::<TestSettings>::new(StoreKey::PRIMARY)
        .open_entry(&temp.paths)
        .unwrap();
    let closure_ran = std::cell::Cell::new(false);
    let result = entry.snapshot_for_update().and_then(|previous| {
        closure_ran.set(true);
        entry.value.value = "changed".to_string();
        entry.finish_update(previous)
    });
    assert!(matches!(
        result,
        Err(SettingsError::UnsupportedFutureVersion {
            found: 99,
            supported: 3
        })
    ));
    assert!(
        !closure_ran.get(),
        "refused update must not invoke callback"
    );
    assert_eq!(entry.value, TestSettings::default());
    assert_eq!(std::fs::read(path).unwrap(), bytes);
}

#[test]
fn same_type_in_two_named_stores_has_separate_files_and_state() {
    let temp = temp_paths();
    let first_key = StoreKey::new("first").unwrap();
    let second_key = StoreKey::new("second").unwrap();
    let mut first = SettingsPlugin::<TestSettings>::new(first_key.clone())
        .open_entry(&temp.paths)
        .unwrap();
    let mut second = SettingsPlugin::<TestSettings>::new(second_key.clone())
        .open_entry(&temp.paths)
        .unwrap();
    first.value.value = "first".to_string();
    second.value.value = "second".to_string();
    first.queue_current().unwrap();
    second.queue_current().unwrap();
    first.store.flush().unwrap();
    second.store.flush().unwrap();

    assert_ne!(first.value, second.value);
    assert!(temp.paths.config_dir().join(first_key.filename()).exists());
    assert!(temp.paths.config_dir().join(second_key.filename()).exists());
}

#[test]
fn corrupt_file_loads_default() {
    let temp = temp_paths();
    std::fs::create_dir_all(temp.paths.config_dir()).unwrap();
    let path = temp.paths.config_dir().join("settings.toml");
    std::fs::write(&path, "not = valid = toml").unwrap();

    let entry = SettingsPlugin::<TestSettings>::new(StoreKey::PRIMARY)
        .open_entry(&temp.paths)
        .unwrap();

    assert_eq!(entry.value, TestSettings::default());
    assert!(!path.exists(), "storage must archive corrupt primary");
}

#[test]
fn update_flush_reload_roundtrip() {
    let temp = temp_paths();
    let mut entry = SettingsPlugin::<TestSettings>::new(StoreKey::PRIMARY)
        .open_entry(&temp.paths)
        .unwrap();
    let previous = entry.snapshot_for_update().unwrap();
    entry.value.value = "persisted".to_string();
    entry.value.stage = 3;
    entry.finish_update(previous).unwrap();
    ErasedSettingsEntry::flush(&mut entry).unwrap();
    drop(entry);

    let reloaded = SettingsPlugin::<TestSettings>::new(StoreKey::PRIMARY)
        .open_entry(&temp.paths)
        .unwrap();
    assert_eq!(
        reloaded.value,
        TestSettings {
            value: "persisted".to_string(),
            stage: 3,
        }
    );
}

#[gpui::test]
fn blocked_exit_flush_is_bounded_and_reported_once(cx: &mut gpui::TestAppContext) {
    let temp = temp_paths();
    let (blocked, hook) = BlockedFlush::new();
    let reports = Arc::new(AtomicUsize::new(0));

    let (started, elapsed) = cx.update(|app| {
        install_error_reporter(app, Arc::clone(&reports));
        let mut entry = SettingsPlugin::<TestSettings>::new(StoreKey::PRIMARY)
            .open_entry(&temp.paths)
            .expect("open settings entry");
        entry.exit_flush_hook = Some(hook);
        let mut registry = SettingsRegistry::default();
        registry
            .entries
            .insert(StoreKey::PRIMARY.filename(), Box::new(entry));
        app.set_global(registry);

        let mut plugin = SettingsPlugin::<TestSettings>::new(StoreKey::PRIMARY);
        let started = Instant::now();
        plugin.on_event(&AppEvent::WillExit, app).unwrap();
        plugin.shutdown(app);
        (started, started.elapsed())
    });

    let worker_started = blocked.wait_until_entered();
    let worker_blocked_before_return = worker_started.saturating_duration_since(started) <= elapsed;
    blocked.release();
    blocked.wait_finished();

    assert!(
        worker_blocked_before_return,
        "exit flush returned before its worker reached blocked storage"
    );
    assert!(
        elapsed < gpui::SHUTDOWN_TIMEOUT,
        "exit flush blocked the main thread for {elapsed:?}"
    );
    assert_eq!(blocked.attempts.load(Ordering::SeqCst), 1);
    assert_eq!(reports.load(Ordering::SeqCst), 1);
}

#[test]
fn exit_flush_budget_is_shared_across_stores() {
    let temp = temp_paths();
    let first_key = StoreKey::PRIMARY;
    let second_key = StoreKey::new("second").unwrap();
    let (first_blocked, first_hook) = BlockedFlush::new();
    let (second_blocked, second_hook) = BlockedFlush::new();

    let mut first = SettingsPlugin::<TestSettings>::new(first_key.clone())
        .open_entry(&temp.paths)
        .unwrap();
    first.exit_flush_hook = Some(first_hook);
    let mut second = SettingsPlugin::<TestSettings>::new(second_key.clone())
        .open_entry(&temp.paths)
        .unwrap();
    second.exit_flush_hook = Some(second_hook);
    let mut registry = SettingsRegistry::default();
    registry
        .entries
        .insert(first_key.filename(), Box::new(first));
    registry
        .entries
        .insert(second_key.filename(), Box::new(second));

    let started = Instant::now();
    let first_result = registry.flush_for_exit::<TestSettings>(&first_key);
    let second_result = registry.flush_for_exit::<TestSettings>(&second_key);
    let elapsed = started.elapsed();

    first_blocked.wait_until_entered();
    first_blocked.release();
    second_blocked.release();
    first_blocked.wait_finished();

    assert!(matches!(
        first_result,
        Err(SettingsError::FlushTimedOut { .. })
    ));
    assert!(matches!(
        second_result,
        Err(SettingsError::FlushTimedOut { .. })
    ));
    assert!(
        elapsed < gpui::SHUTDOWN_TIMEOUT,
        "settings stores exceeded the shared exit flush budget: {elapsed:?}"
    );
    assert_eq!(first_blocked.attempts.load(Ordering::SeqCst), 1);
    assert_eq!(
        second_blocked.attempts.load(Ordering::SeqCst),
        0,
        "expired shared budget must not start a late flush worker"
    );
}
