//! Typed, named application settings backed by schema-versioned TOML stores.
//!
//! Settings files and their backups are **never for secrets**. Use an OS
//! credential store or another purpose-built secrets facility instead.

use std::borrow::Cow;
use std::marker::PhantomData;
use std::sync::Arc;
use std::time::Duration;

use gpui::App;
use neutron_components_storage::{
    AppPaths, DebouncedStore, Envelope, LoadOutcome, StorageError, StoreConfig,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::error::AppShellError;
use crate::handles::{AppInfo, AppProxy};
use crate::lifecycle::AppEvent;
use crate::module::RuntimeModule;

mod runtime;

pub use runtime::Settings;
use runtime::{SettingsEntry, SettingsRegistry};

// GPUI runs quit observers synchronously before applying its 100ms deadline to
// their returned futures. All settings stores share half that window.
const EXIT_FLUSH_TIMEOUT: Duration = Duration::from_millis(50);

/// A serializable application settings schema.
///
/// The schema type owns its version, its downgrade policy, its migration chain,
/// and its validation rule, so registering a store needs nothing but a
/// [`StoreKey`].
pub trait AppSettings: Serialize + DeserializeOwned + Default + Send + 'static {
    /// Current on-disk schema version.
    const SCHEMA_VERSION: u32;

    /// Behavior when disk holds a schema newer than [`Self::SCHEMA_VERSION`].
    ///
    /// [`SettingsModule::new`] derives its policy from this constant. An
    /// explicit [`SettingsModule::future_version_policy`] call overrides it.
    const FUTURE_VERSION_POLICY: FutureVersionPolicy = FutureVersionPolicy::RefuseToWrite;

    /// Migrate a raw document in place from schema `from` to schema `to`.
    ///
    /// Called **once** per load, with `from` set to the version found on disk
    /// and `to` set to the store's current version, so one implementation owns
    /// the whole chain and decides how to walk intermediate versions. The
    /// caller stamps `schema_version = to` afterwards; the implementation must
    /// leave a TOML table behind.
    ///
    /// The default refuses to migrate, which keeps a type that declares no
    /// migrations failing loudly rather than silently loading defaults over
    /// real user data.
    fn migrate(from: u32, to: u32, value: &mut toml::Value) -> Result<(), SettingsError> {
        let _ = value;
        Err(SettingsError::MissingMigration {
            found: from,
            current: to,
        })
    }

    /// Validate a value before it becomes current or is queued for persistence.
    fn validate(&self) -> Result<(), SettingsError> {
        Ok(())
    }
}

/// Name of one physical settings store.
///
/// Store identity is the resulting filename, not `(TypeId, StoreKey)`. Thus one
/// filename has exactly one owner and schema, while the same Rust type may be
/// used safely by multiple distinct keys/files.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StoreKey(Cow<'static, str>);

impl StoreKey {
    /// Conventional primary application settings store (`settings.toml`).
    pub const PRIMARY: Self = Self(Cow::Borrowed("settings"));

    /// Create a named store key.
    ///
    /// Keys may contain ASCII lowercase letters, digits, `-`, and `_` only.
    /// Uppercase is rejected because the key becomes a filename, and
    /// `Settings.toml` and `settings.toml` are the same file on the
    /// case-insensitive filesystems that macOS and Windows use by default.
    /// Allowing both spellings would let two stores claim one file while the
    /// duplicate-registration check saw distinct names.
    pub fn new(key: impl Into<Cow<'static, str>>) -> Result<Self, SettingsError> {
        let key = key.into();
        if key.is_empty()
            || !key.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
            })
        {
            return Err(SettingsError::InvalidStoreKey(key.into_owned()));
        }
        Ok(Self(key))
    }

    /// Bare key without the `.toml` suffix.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn filename(&self) -> String {
        format!("{}.toml", self.0)
    }
}

/// Behavior when disk contains a schema newer than this build understands.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum FutureVersionPolicy {
    /// Preserve the file byte-for-byte and reject every update (default).
    #[default]
    RefuseToWrite,
    /// Load defaults and allow a later explicit update to replace the file.
    Overwrite,
}

/// Settings registration, migration, validation, and persistence errors.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum SettingsError {
    /// A key could escape the config directory, or alias another key on a
    /// case-insensitive filesystem.
    #[error(
        "invalid settings store key {0:?}: use ASCII lowercase letters, digits, '-', and '_' only"
    )]
    InvalidStoreKey(String),
    /// No registered store owns this filename.
    #[error("settings store {0:?} is not registered")]
    NotRegistered(String),
    /// Another settings module already owns this filename.
    #[error("settings store filename {0:?} is already registered")]
    DuplicateStore(String),
    /// The filename is registered for a different Rust settings type.
    #[error("settings store {key:?} has type {actual}, not {expected}")]
    TypeMismatch {
        /// Requested store key.
        key: String,
        /// Requested Rust type.
        expected: &'static str,
        /// Registered Rust type.
        actual: &'static str,
    },
    /// A migration edge required to reach the current schema is absent.
    #[error("no migration registered from schema version {found} toward {current}")]
    MissingMigration {
        /// Version reached by the chain.
        found: u32,
        /// Version required by this build.
        current: u32,
    },
    /// A migration edge is invalid or its function failed.
    #[error("migration {from}->{to} failed: {message}")]
    Migration {
        /// Source schema version.
        from: u32,
        /// Destination schema version.
        to: u32,
        /// Migration failure detail.
        message: String,
    },
    /// A loaded or updated value failed validation.
    #[error("settings validation failed: {0}")]
    Validation(String),
    /// A downgrade attempted to modify a newer on-disk schema.
    #[error(
        "refusing to write settings: on-disk schema version {found} is newer than supported {supported}"
    )]
    UnsupportedFutureVersion {
        /// Version found on disk.
        found: u32,
        /// Highest version supported by this store.
        supported: u32,
    },
    /// A bounded exit flush did not finish before its deadline.
    #[error("settings flush for {key:?} timed out after {timeout:?}")]
    FlushTimedOut {
        /// Store key.
        key: String,
        /// Applied timeout.
        timeout: Duration,
    },
    /// The helper used to bound an exit flush could not be started.
    #[error("failed to spawn settings flush worker: {0}")]
    FlushWorker(String),
    /// The linked storage crate returned a load outcome this build does not understand.
    #[error("storage returned an unsupported settings load outcome")]
    UnsupportedLoadOutcome,
    /// Foundation storage failure.
    #[error(transparent)]
    Storage(#[from] StorageError),
}

/// Function called for one schema migration edge.
pub(crate) type MigrationFn = fn(toml::Value) -> Result<toml::Value, String>;
/// Additional validator installed by [`SettingsModule::validate`].
pub(crate) type SettingsValidator<T> = fn(&T) -> Result<(), String>;

struct MigrationStep {
    from: u32,
    to: u32,
    run: MigrationFn,
}

/// The runtime module registering one typed, named settings store.
pub(crate) struct SettingsModule<T: AppSettings> {
    key: StoreKey,
    current_version: u32,
    migrations: Vec<MigrationStep>,
    validator: Option<SettingsValidator<T>>,
    future_version_policy: FutureVersionPolicy,
    _marker: PhantomData<fn() -> T>,
}

impl<T: AppSettings> SettingsModule<T> {
    /// Register `T` under `key`.
    ///
    /// The current version comes from [`AppSettings::SCHEMA_VERSION`] and the
    /// downgrade policy from [`AppSettings::FUTURE_VERSION_POLICY`].
    pub fn new(key: StoreKey) -> Self {
        Self {
            key,
            current_version: T::SCHEMA_VERSION,
            migrations: Vec::new(),
            validator: None,
            future_version_policy: T::FUTURE_VERSION_POLICY,
            _marker: PhantomData,
        }
    }

    /// Override the current version stamped on disk.
    ///
    /// Normally the [`AppSettings::SCHEMA_VERSION`] default is sufficient. This
    /// builder exists for the explicit §4a registration form.
    #[must_use]
    #[allow(dead_code)] // Exercised only by unit tests; no production caller yet.
    pub fn current_version(mut self, version: u32) -> Self {
        self.current_version = version;
        self
    }

    /// Add one directed migration edge. Edges run stepwise from the version on disk.
    ///
    /// Registering any edge makes this module own the entire chain:
    /// [`AppSettings::migrate`] is then never called, so the two paths never
    /// both run and a gap in the edge set fails with
    /// [`SettingsError::MissingMigration`] instead of being skipped.
    #[must_use]
    #[allow(dead_code)] // Exercised only by unit tests; no production caller yet.
    pub fn migrate(mut self, from: u32, to: u32, run: MigrationFn) -> Self {
        self.migrations.push(MigrationStep { from, to, run });
        self
    }

    /// Add store-specific validation after [`AppSettings::validate`].
    #[must_use]
    #[allow(dead_code)] // Exercised only by unit tests; no production caller yet.
    pub fn validate(mut self, validator: SettingsValidator<T>) -> Self {
        self.validator = Some(validator);
        self
    }

    /// Set downgrade behavior for a newer on-disk schema.
    ///
    /// Overrides [`AppSettings::FUTURE_VERSION_POLICY`] for this store.
    #[must_use]
    #[allow(dead_code)] // Exercised only by unit tests; no production caller yet.
    pub fn future_version_policy(mut self, policy: FutureVersionPolicy) -> Self {
        self.future_version_policy = policy;
        self
    }

    fn open_entry(&self, paths: &AppPaths) -> Result<SettingsEntry<T>, SettingsError> {
        let path = paths.config_dir().join(self.key.filename());
        let store = Arc::new(DebouncedStore::open(
            path,
            StoreConfig::new(self.current_version),
        )?);
        let mut migrated = false;
        let (value, future_version) = match store.load()? {
            LoadOutcome::Loaded(raw) => (decode_value(raw)?, None),
            LoadOutcome::NeedsMigration { found, raw } => {
                migrated = true;
                (self.migrate_to_current(found, raw)?, None)
            }
            LoadOutcome::FutureVersion { found } => (T::default(), Some(found)),
            LoadOutcome::Corrupt { .. } | LoadOutcome::Missing => (T::default(), None),
            _ => return Err(SettingsError::UnsupportedLoadOutcome),
        };

        validate_value(&value, self.validator)?;
        let entry = SettingsEntry {
            key: self.key.as_str().to_string(),
            value,
            store,
            current_version: self.current_version,
            future_version,
            future_version_policy: self.future_version_policy,
            validator: self.validator,
            version: 0,
            lifecycle_error: None,
            exit_flushed: false,
            #[cfg(test)]
            exit_flush_hook: None,
        };
        if migrated {
            entry.queue_current()?;
        }
        Ok(entry)
    }

    /// Bring a raw older document up to `current_version` and decode it.
    ///
    /// Exactly one migration path runs: registered edges when this module
    /// declares any, otherwise the [`AppSettings::migrate`] hook.
    fn migrate_to_current(&self, found: u32, mut raw: toml::Value) -> Result<T, SettingsError> {
        if self.migrations.is_empty() {
            T::migrate(found, self.current_version, &mut raw)?;
            stamp_schema_version(&mut raw, found, self.current_version)?;
        } else {
            raw = self.apply_migrations(found, raw)?;
        }
        decode_envelope(raw)
    }

    fn apply_migrations(
        &self,
        mut version: u32,
        mut raw: toml::Value,
    ) -> Result<toml::Value, SettingsError> {
        while version < self.current_version {
            let migration = self
                .migrations
                .iter()
                .find(|migration| migration.from == version)
                .ok_or(SettingsError::MissingMigration {
                    found: version,
                    current: self.current_version,
                })?;
            if migration.to <= migration.from || migration.to > self.current_version {
                return Err(SettingsError::Migration {
                    from: migration.from,
                    to: migration.to,
                    message: "invalid migration edge".to_string(),
                });
            }
            raw = (migration.run)(raw).map_err(|message| SettingsError::Migration {
                from: migration.from,
                to: migration.to,
                message,
            })?;
            stamp_schema_version(&mut raw, migration.from, migration.to)?;
            version = migration.to;
        }
        Ok(raw)
    }

    fn flush_for_exit(&self, cx: &mut App) {
        let result = cx
            .global_mut::<SettingsRegistry>()
            .flush_for_exit::<T>(&self.key);
        if let Err(error) = result {
            crate::handles::report_error(
                cx,
                crate::error::RuntimeError::module("settings", anyhow::Error::new(error)),
            );
        }
    }
}

impl<T: AppSettings> RuntimeModule for SettingsModule<T> {
    fn id(&self) -> &'static str {
        "settings"
    }

    fn init(
        &mut self,
        cx: &mut App,
        info: &AppInfo,
        _proxy: &AppProxy,
    ) -> Result<(), AppShellError> {
        if !cx.has_global::<SettingsRegistry>() {
            cx.set_global(SettingsRegistry::default());
        }
        let filename = self.key.filename();
        if cx
            .global::<SettingsRegistry>()
            .entries
            .contains_key(&filename)
        {
            return Err(service_error(SettingsError::DuplicateStore(filename)));
        }
        let entry = self.open_entry(info.paths()).map_err(service_error)?;
        cx.global_mut::<SettingsRegistry>()
            .entries
            .insert(filename, Box::new(entry));
        Ok(())
    }

    fn on_event(&mut self, event: &AppEvent, cx: &mut App) -> Result<(), AppShellError> {
        if matches!(event, AppEvent::WillExit) {
            self.flush_for_exit(cx);
        }
        Ok(())
    }

    fn shutdown(&mut self, cx: &mut App) {
        self.flush_for_exit(cx);
    }
}

fn service_error(error: SettingsError) -> AppShellError {
    AppShellError::Module {
        module: "settings",
        source: anyhow::Error::new(error),
    }
}

fn decode_value<T: AppSettings>(raw: toml::Value) -> Result<T, SettingsError> {
    raw.try_into()
        .map_err(StorageError::from)
        .map_err(SettingsError::from)
}

/// Decode a migrated document, whose table still carries `schema_version`.
fn decode_envelope<T: AppSettings>(raw: toml::Value) -> Result<T, SettingsError> {
    let envelope: Envelope<T> = raw
        .try_into()
        .map_err(StorageError::from)
        .map_err(SettingsError::from)?;
    Ok(envelope.inner)
}

/// Stamp the version a migration step just produced, rejecting a non-table.
fn stamp_schema_version(raw: &mut toml::Value, from: u32, to: u32) -> Result<(), SettingsError> {
    raw.as_table_mut()
        .ok_or_else(|| SettingsError::Migration {
            from,
            to,
            message: "migration returned a non-table TOML value".to_string(),
        })?
        .insert(
            "schema_version".to_string(),
            toml::Value::Integer(i64::from(to)),
        );
    Ok(())
}

fn validate_value<T: AppSettings>(
    value: &T,
    validator: Option<SettingsValidator<T>>,
) -> Result<(), SettingsError> {
    value.validate()?;
    if let Some(validator) = validator {
        validator(value).map_err(SettingsError::Validation)?;
    }
    Ok(())
}

/// Persisted shell theme preference.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ThemeMode {
    /// Follow the operating-system appearance.
    #[default]
    System,
    /// Force light appearance.
    Light,
    /// Force dark appearance.
    Dark,
}

/// Platform-owned theme and locale preferences.
///
/// This settings file and its backups are **never for secrets**.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ShellPreferences {
    /// System/light/dark appearance mode.
    pub theme_mode: ThemeMode,
    /// Selected named theme, if any.
    pub theme_name: Option<String>,
    /// Selected locale, if any.
    pub locale: Option<String>,
}

impl AppSettings for ShellPreferences {
    const SCHEMA_VERSION: u32 = 1;
}

/// Filename stem of the platform-owned shell-preferences store.
///
/// Reserved: an application store may not claim it.
pub(crate) const SHELL_PREFERENCES_KEY: &str = "shell-preferences";

pub(crate) fn shell_preferences_key() -> StoreKey {
    StoreKey(Cow::Borrowed(SHELL_PREFERENCES_KEY))
}

/// Dedicated module for the automatically registered shell-preferences store.
pub(crate) struct ShellPreferencesModule(SettingsModule<ShellPreferences>);

impl ShellPreferencesModule {
    /// Create the shell-preferences module.
    pub fn new() -> Self {
        Self(SettingsModule::new(shell_preferences_key()))
    }
}

impl Default for ShellPreferencesModule {
    fn default() -> Self {
        Self::new()
    }
}

impl RuntimeModule for ShellPreferencesModule {
    fn id(&self) -> &'static str {
        "shell-preferences"
    }

    fn init(
        &mut self,
        cx: &mut App,
        info: &AppInfo,
        proxy: &AppProxy,
    ) -> Result<(), AppShellError> {
        self.0.init(cx, info, proxy)
    }

    fn on_event(&mut self, event: &AppEvent, cx: &mut App) -> Result<(), AppShellError> {
        self.0.on_event(event, cx)
    }

    fn shutdown(&mut self, cx: &mut App) {
        self.0.shutdown(cx);
    }
}

/// Clone the platform-owned shell preferences without exposing registry internals.
pub fn shell_preferences(cx: &App) -> ShellPreferences {
    cx.settings::<ShellPreferences>(shell_preferences_key())
        .clone()
}

/// Update and debounce-save platform-owned shell preferences.
pub fn update_shell_preferences(
    cx: &mut App,
    update: impl FnOnce(&mut ShellPreferences),
) -> Result<(), SettingsError> {
    cx.update_settings(shell_preferences_key(), |preferences, _cx| {
        update(preferences)
    })
}

#[cfg(test)]
#[path = "settings/tests.rs"]
mod tests;
