use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use neutron_components_manifest::schema::IdentityRef;
use neutron_components_storage::PathLayout;
use tempfile::TempDir;

use crate::capabilities::PlatformCapabilities;
use crate::error::RuntimeOperation;
use crate::handles::{self, AppInfo, PendingEvents};
use crate::liveness::{ExitPolicy, InitialActivation, Liveness};
use crate::module::RuntimeModule;

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

/// Migrates entirely through the trait hook: no declared edges are registered.
#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
struct TraitMigrated {
    value: String,
    stage: u32,
}

static TRAIT_MIGRATE_CALLS: AtomicUsize = AtomicUsize::new(0);

impl AppSettings for TraitMigrated {
    const SCHEMA_VERSION: u32 = 3;

    fn migrate(from: u32, to: u32, value: &mut toml::Value) -> Result<(), SettingsError> {
        TRAIT_MIGRATE_CALLS.fetch_add(1, Ordering::SeqCst);
        let table = value.as_table_mut().ok_or(SettingsError::Migration {
            from,
            to,
            message: "not a table".to_string(),
        })?;
        // The caller owns `schema_version`, so dropping it here must be safe.
        table.remove("schema_version");
        table.insert(
            "value".to_string(),
            toml::Value::String(format!("{from}->{to}")),
        );
        table.insert("stage".to_string(), toml::Value::Integer(i64::from(to)));
        Ok(())
    }
}

/// Declares no migration at all, so the defaulted trait hook must refuse.
#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
struct NoMigration {
    value: String,
}

impl AppSettings for NoMigration {
    const SCHEMA_VERSION: u32 = 2;
}

/// Has a trait hook *and* is used with declared migration edges, so the two paths
/// can be observed never running together.
#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
struct Precedence {
    value: String,
}

static PRECEDENCE_TRAIT_CALLS: AtomicUsize = AtomicUsize::new(0);

impl AppSettings for Precedence {
    const SCHEMA_VERSION: u32 = 3;

    fn migrate(_from: u32, _to: u32, value: &mut toml::Value) -> Result<(), SettingsError> {
        PRECEDENCE_TRAIT_CALLS.fetch_add(1, Ordering::SeqCst);
        value["value"] = toml::Value::String("trait".to_string());
        Ok(())
    }
}

fn precedence_edge_1_to_2(mut raw: toml::Value) -> Result<toml::Value, String> {
    raw["value"] = toml::Value::String("edge".to_string());
    Ok(raw)
}

fn precedence_edge_2_to_3(mut raw: toml::Value) -> Result<toml::Value, String> {
    let previous = raw["value"].as_str().unwrap_or_default().to_string();
    raw["value"] = toml::Value::String(format!("{previous}+edge"));
    Ok(raw)
}

/// Rejects out-of-range values through the trait validator.
#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
struct Validated {
    stage: u32,
}

impl AppSettings for Validated {
    const SCHEMA_VERSION: u32 = 1;

    fn validate(&self) -> Result<(), SettingsError> {
        if self.stage > 10 {
            return Err(SettingsError::Validation(format!(
                "stage {} exceeds 10",
                self.stage
            )));
        }
        Ok(())
    }
}

/// Opts into overwriting a newer on-disk schema at the type level.
#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
struct OverwriteFuture {
    value: String,
}

impl AppSettings for OverwriteFuture {
    const SCHEMA_VERSION: u32 = 1;
    const FUTURE_VERSION_POLICY: FutureVersionPolicy = FutureVersionPolicy::Overwrite;
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
        Box::new(move |error, _| {
            assert_eq!(error.operation(), RuntimeOperation::Module);
            assert_eq!(error.module_id(), Some("settings"));
            assert!(error.source_error().to_string().contains("timed out"));
            reports.fetch_add(1, Ordering::SeqCst);
        }),
        None,
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

    let entry = SettingsModule::<TestSettings>::new(StoreKey::PRIMARY)
        .migrate(1, 2, migrate_1_to_2)
        .migrate(2, 3, migrate_2_to_3)
        .open_entry(&temp.paths)
        .unwrap();

    assert_eq!(MIGRATION_STEPS.load(Ordering::SeqCst), 2);
    assert_eq!(entry.value.stage, 3);
    entry.store.flush().unwrap();
}

/// `SettingsEntry` is not `Debug`, so `expect_err` cannot be used on an open.
fn expect_open_error<T>(result: Result<T, SettingsError>, context: &str) -> SettingsError {
    match result {
        Ok(_) => panic!("{context}"),
        Err(error) => error,
    }
}

#[test]
fn store_keys_reject_uppercase_that_would_alias_on_a_case_insensitive_disk() {
    for candidate in [
        "Shell-Preferences",
        "SHELL-PREFERENCES",
        "Settings",
        "myStore",
        "storeA",
    ] {
        let error = StoreKey::new(candidate)
            .expect_err("an uppercase key can alias another key's file on macOS and Windows");
        assert!(
            matches!(&error, SettingsError::InvalidStoreKey(key) if key == candidate),
            "{candidate}: {error}"
        );
        assert!(
            error.to_string().contains("ASCII lowercase"),
            "the message must say what is allowed: {error}"
        );
    }
}

#[test]
fn store_keys_still_reject_separators_and_non_ascii() {
    for candidate in ["", "..", "a/b", "a\\b", "a.b", "a b", "café", "sett\0ings"] {
        assert!(
            matches!(
                StoreKey::new(candidate.to_string()),
                Err(SettingsError::InvalidStoreKey(_))
            ),
            "{candidate:?} must stay rejected"
        );
    }
}

#[test]
fn store_keys_accept_the_lowercase_charset_including_framework_keys() {
    for candidate in [
        "settings",
        "shell-preferences",
        "my_store2",
        "a",
        "0",
        "-",
        "_",
    ] {
        let key = StoreKey::new(candidate).unwrap_or_else(|error| panic!("{candidate}: {error}"));
        assert_eq!(key.as_str(), candidate);
        assert_eq!(key.filename(), format!("{candidate}.toml"));
    }

    // The two keys the framework constructs directly, bypassing `new`, must
    // themselves satisfy the rule they impose on applications.
    for reserved in [StoreKey::PRIMARY, shell_preferences_key()] {
        let rebuilt = StoreKey::new(reserved.as_str().to_string())
            .unwrap_or_else(|error| panic!("reserved key {reserved:?}: {error}"));
        assert_eq!(rebuilt.filename(), reserved.filename());
    }
}

#[test]
fn a_lowercase_key_with_the_full_charset_round_trips_through_its_own_file() {
    let temp = temp_paths();
    let key = StoreKey::new("my_store-2").unwrap();

    let mut entry = SettingsModule::<TestSettings>::new(key.clone())
        .open_entry(&temp.paths)
        .unwrap();
    let previous = entry.snapshot_for_update().unwrap();
    entry.value.value = "kept".to_string();
    entry.finish_update(previous).unwrap();
    ErasedSettingsEntry::flush(&mut entry).unwrap();
    drop(entry);

    assert!(
        temp.paths.config_dir().join("my_store-2.toml").exists(),
        "the key names the file verbatim"
    );
    let reloaded = SettingsModule::<TestSettings>::new(key)
        .open_entry(&temp.paths)
        .unwrap();
    assert_eq!(reloaded.value.value, "kept");
}

fn seed(temp: &TempPaths, key: &StoreKey, contents: &str) -> std::path::PathBuf {
    std::fs::create_dir_all(temp.paths.config_dir()).unwrap();
    let path = temp.paths.config_dir().join(key.filename());
    std::fs::write(&path, contents).unwrap();
    path
}

#[test]
fn trait_migration_runs_once_and_stamps_the_current_version() {
    TRAIT_MIGRATE_CALLS.store(0, Ordering::SeqCst);
    let temp = temp_paths();
    seed(
        &temp,
        &StoreKey::PRIMARY,
        "schema_version = 1\nvalue = \"one\"\nstage = 1\n",
    );

    let entry = SettingsModule::<TraitMigrated>::new(StoreKey::PRIMARY)
        .open_entry(&temp.paths)
        .unwrap();

    assert_eq!(TRAIT_MIGRATE_CALLS.load(Ordering::SeqCst), 1);
    assert_eq!(
        entry.value,
        TraitMigrated {
            value: "1->3".to_string(),
            stage: 3,
        },
        "the hook receives the disk version and the current version"
    );
    entry.store.flush().unwrap();
    drop(entry);

    // Reloading must find a current-version file, proving the caller stamped
    // `schema_version` after the hook rather than leaving it at 1.
    let reloaded = SettingsModule::<TraitMigrated>::new(StoreKey::PRIMARY)
        .open_entry(&temp.paths)
        .unwrap();
    assert_eq!(TRAIT_MIGRATE_CALLS.load(Ordering::SeqCst), 1);
    assert_eq!(reloaded.value.stage, 3);
}

#[test]
fn default_trait_migration_refuses_instead_of_dropping_user_data() {
    let temp = temp_paths();
    let path = seed(
        &temp,
        &StoreKey::PRIMARY,
        "schema_version = 1\nvalue = \"kept\"\n",
    );

    let error = expect_open_error(
        SettingsModule::<NoMigration>::new(StoreKey::PRIMARY).open_entry(&temp.paths),
        "a type declaring no migration cannot load an older schema",
    );

    assert!(
        matches!(
            error,
            SettingsError::MissingMigration {
                found: 1,
                current: 2
            }
        ),
        "{error}"
    );
    assert_eq!(
        std::fs::read_to_string(path).unwrap(),
        "schema_version = 1\nvalue = \"kept\"\n",
        "a refused migration must leave the older file untouched"
    );
}

#[test]
fn registered_edges_own_the_chain_and_suppress_the_trait_hook() {
    let temp = temp_paths();
    seed(
        &temp,
        &StoreKey::PRIMARY,
        "schema_version = 1\nvalue = \"one\"\n",
    );

    let entry = SettingsModule::<Precedence>::new(StoreKey::PRIMARY)
        .migrate(1, 2, precedence_edge_1_to_2)
        .migrate(2, 3, precedence_edge_2_to_3)
        .open_entry(&temp.paths)
        .unwrap();

    assert_eq!(entry.value.value, "edge+edge");
    assert_eq!(
        PRECEDENCE_TRAIT_CALLS.load(Ordering::SeqCst),
        0,
        "the trait hook must not also run when edges are registered"
    );
    entry.store.flush().unwrap();
}

#[test]
fn a_gap_in_registered_edges_fails_without_falling_back_to_the_trait_hook() {
    let temp = temp_paths();
    seed(
        &temp,
        &StoreKey::PRIMARY,
        "schema_version = 1\nvalue = \"one\"\n",
    );

    let error = expect_open_error(
        SettingsModule::<Precedence>::new(StoreKey::PRIMARY)
            .migrate(1, 2, precedence_edge_1_to_2)
            .open_entry(&temp.paths),
        "the chain cannot reach version 3",
    );

    assert!(
        matches!(
            error,
            SettingsError::MissingMigration {
                found: 2,
                current: 3
            }
        ),
        "{error}"
    );
    assert_eq!(
        PRECEDENCE_TRAIT_CALLS.load(Ordering::SeqCst),
        0,
        "a missing edge must not silently hand the rest of the chain to the trait"
    );
}

#[test]
fn trait_validation_rejects_a_loaded_value() {
    let temp = temp_paths();
    seed(
        &temp,
        &StoreKey::PRIMARY,
        "schema_version = 1\nstage = 99\n",
    );

    let error = expect_open_error(
        SettingsModule::<Validated>::new(StoreKey::PRIMARY).open_entry(&temp.paths),
        "99 is out of range",
    );

    assert!(
        matches!(&error, SettingsError::Validation(message) if message == "stage 99 exceeds 10"),
        "{error}"
    );
}

#[test]
fn trait_validation_rolls_back_a_rejected_update() {
    let temp = temp_paths();
    let mut entry = SettingsModule::<Validated>::new(StoreKey::PRIMARY)
        .open_entry(&temp.paths)
        .unwrap();

    let previous = entry.snapshot_for_update().unwrap();
    entry.value.stage = 11;
    let error = entry
        .finish_update(previous)
        .expect_err("11 is out of range");

    assert!(matches!(error, SettingsError::Validation(_)), "{error}");
    assert_eq!(entry.value, Validated { stage: 0 });
}

#[test]
fn the_future_version_policy_is_derived_from_the_settings_type() {
    assert_eq!(
        SettingsModule::<TestSettings>::new(StoreKey::PRIMARY).future_version_policy,
        FutureVersionPolicy::RefuseToWrite,
    );
    assert_eq!(
        SettingsModule::<OverwriteFuture>::new(StoreKey::PRIMARY).future_version_policy,
        FutureVersionPolicy::Overwrite,
    );
}

#[test]
fn a_type_level_overwrite_policy_allows_replacing_a_future_file() {
    let temp = temp_paths();
    let path = seed(
        &temp,
        &StoreKey::PRIMARY,
        "schema_version = 99\nvalue = \"future\"\n",
    );

    let mut entry = SettingsModule::<OverwriteFuture>::new(StoreKey::PRIMARY)
        .open_entry(&temp.paths)
        .unwrap();
    let previous = entry.snapshot_for_update().unwrap();
    entry.value.value = "replaced".to_string();
    entry.finish_update(previous).unwrap();
    ErasedSettingsEntry::flush(&mut entry).unwrap();

    let written = std::fs::read_to_string(path).unwrap();
    assert!(written.contains("replaced"), "{written}");
    assert!(written.contains("schema_version = 1"), "{written}");
}

#[test]
fn an_explicit_builder_policy_overrides_the_type_level_policy() {
    let temp = temp_paths();
    let overwriting_key = StoreKey::new("overwriting").unwrap();
    let refusing_key = StoreKey::new("refusing").unwrap();
    let future = "schema_version = 99\nvalue = \"future\"\n";
    seed(&temp, &overwriting_key, future);
    seed(&temp, &refusing_key, future);

    // Type says Overwrite, builder says RefuseToWrite: the builder wins.
    let refusing = SettingsModule::<OverwriteFuture>::new(refusing_key)
        .future_version_policy(FutureVersionPolicy::RefuseToWrite)
        .open_entry(&temp.paths)
        .unwrap();
    let error = refusing
        .snapshot_for_update()
        .expect_err("the builder policy refuses");
    assert!(
        matches!(
            error,
            SettingsError::UnsupportedFutureVersion {
                found: 99,
                supported: 1
            }
        ),
        "{error}"
    );

    // Type defaults to RefuseToWrite, builder says Overwrite: the builder wins.
    let overwriting = SettingsModule::<TestSettings>::new(overwriting_key)
        .future_version_policy(FutureVersionPolicy::Overwrite)
        .open_entry(&temp.paths)
        .unwrap();
    overwriting
        .snapshot_for_update()
        .expect("the builder policy permits the write");
}

fn reject_default_stage(value: &TestSettings) -> Result<(), String> {
    if value.stage == 0 {
        return Err("stage must be set".to_string());
    }
    Ok(())
}

#[test]
fn the_declared_validator_still_runs_after_trait_validation() {
    let temp = temp_paths();

    let error = expect_open_error(
        SettingsModule::<TestSettings>::new(StoreKey::PRIMARY)
            .validate(reject_default_stage)
            .open_entry(&temp.paths),
        "the declared validator rejects the default value",
    );

    assert!(
        matches!(&error, SettingsError::Validation(message) if message == "stage must be set"),
        "{error}"
    );
}

fn pin_stage_to_2(mut raw: toml::Value) -> Result<toml::Value, String> {
    raw["stage"] = toml::Value::Integer(2);
    Ok(raw)
}

#[test]
fn the_legacy_current_version_override_still_targets_its_own_version() {
    let temp = temp_paths();
    let path = seed(
        &temp,
        &StoreKey::PRIMARY,
        "schema_version = 1\nvalue = \"one\"\nstage = 1\n",
    );

    // `TestSettings::SCHEMA_VERSION` is 3, but this store pins itself to 2.
    let entry = SettingsModule::<TestSettings>::new(StoreKey::PRIMARY)
        .current_version(2)
        .migrate(1, 2, pin_stage_to_2)
        .open_entry(&temp.paths)
        .unwrap();

    assert_eq!(entry.value.stage, 2);
    entry.store.flush().unwrap();
    assert!(
        std::fs::read_to_string(path)
            .unwrap()
            .contains("schema_version = 2"),
    );
}

#[test]
fn future_version_refuses_update_and_preserves_bytes() {
    let temp = temp_paths();
    std::fs::create_dir_all(temp.paths.config_dir()).unwrap();
    let path = temp.paths.config_dir().join("settings.toml");
    let bytes = b"schema_version = 99\nvalue = \"future\"\nstage = 99\n";
    std::fs::write(&path, bytes).unwrap();

    let mut entry = SettingsModule::<TestSettings>::new(StoreKey::PRIMARY)
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
    let mut first = SettingsModule::<TestSettings>::new(first_key.clone())
        .open_entry(&temp.paths)
        .unwrap();
    let mut second = SettingsModule::<TestSettings>::new(second_key.clone())
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

    let entry = SettingsModule::<TestSettings>::new(StoreKey::PRIMARY)
        .open_entry(&temp.paths)
        .unwrap();

    assert_eq!(entry.value, TestSettings::default());
    assert!(!path.exists(), "storage must archive corrupt primary");
}

#[test]
fn update_flush_reload_roundtrip() {
    let temp = temp_paths();
    let mut entry = SettingsModule::<TestSettings>::new(StoreKey::PRIMARY)
        .open_entry(&temp.paths)
        .unwrap();
    let previous = entry.snapshot_for_update().unwrap();
    entry.value.value = "persisted".to_string();
    entry.value.stage = 3;
    entry.finish_update(previous).unwrap();
    ErasedSettingsEntry::flush(&mut entry).unwrap();
    drop(entry);

    let reloaded = SettingsModule::<TestSettings>::new(StoreKey::PRIMARY)
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
        let mut entry = SettingsModule::<TestSettings>::new(StoreKey::PRIMARY)
            .open_entry(&temp.paths)
            .expect("open settings entry");
        entry.exit_flush_hook = Some(hook);
        let mut registry = SettingsRegistry::default();
        registry
            .entries
            .insert(StoreKey::PRIMARY.filename(), Box::new(entry));
        app.set_global(registry);

        let mut module = SettingsModule::<TestSettings>::new(StoreKey::PRIMARY);
        let started = Instant::now();
        module.on_event(&AppEvent::WillExit, app).unwrap();
        module.shutdown(app);
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

    let mut first = SettingsModule::<TestSettings>::new(first_key.clone())
        .open_entry(&temp.paths)
        .unwrap();
    first.exit_flush_hook = Some(first_hook);
    let mut second = SettingsModule::<TestSettings>::new(second_key.clone())
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
