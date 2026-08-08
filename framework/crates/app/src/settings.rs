//! Typed, named application settings backed by schema-versioned TOML stores.
//!
//! Settings files and their backups are **never for secrets**. Use an OS
//! credential store or another purpose-built secrets facility instead.

use std::borrow::Cow;
use std::marker::PhantomData;
use std::sync::Arc;
use std::time::Duration;

use gpui::App;
use gpui_component_storage::{
    AppPaths, DebouncedStore, Envelope, LoadOutcome, StorageError, StoreConfig,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::error::AppShellError;
use crate::lifecycle::AppEvent;
use crate::plugin::{AppPlugin, ShellSeed};

mod runtime;

pub use runtime::SettingsExt;
use runtime::{SettingsEntry, SettingsRegistry};

// GPUI runs quit observers synchronously before applying its 100ms deadline to
// their returned futures. All settings stores share half that window.
const EXIT_FLUSH_TIMEOUT: Duration = Duration::from_millis(50);

/// A serializable application settings schema.
pub trait AppSettings: Serialize + DeserializeOwned + Default + Send + 'static {
    /// Current on-disk schema version.
    const SCHEMA_VERSION: u32;

    /// Validate a value before it becomes current or is queued for persistence.
    fn validate(&self) -> Result<(), String> {
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
    /// Keys may contain ASCII letters, digits, `-`, and `_` only.
    pub fn new(key: impl Into<Cow<'static, str>>) -> Result<Self, SettingsError> {
        let key = key.into();
        if key.is_empty()
            || !key
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
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
    /// A key could escape or alias the config directory.
    #[error("invalid settings store key {0:?}")]
    InvalidStoreKey(String),
    /// No registered store owns this filename.
    #[error("settings store {0:?} is not registered")]
    NotRegistered(String),
    /// Another settings plugin already owns this filename.
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
pub type MigrationFn = fn(toml::Value) -> Result<toml::Value, String>;
/// Additional validator installed by [`SettingsPlugin::validate`].
pub type SettingsValidator<T> = fn(&T) -> Result<(), String>;

struct MigrationStep {
    from: u32,
    to: u32,
    run: MigrationFn,
}

/// Plugin registering one typed, named settings store.
pub struct SettingsPlugin<T: AppSettings> {
    key: StoreKey,
    current_version: u32,
    migrations: Vec<MigrationStep>,
    validator: Option<SettingsValidator<T>>,
    future_version_policy: FutureVersionPolicy,
    _marker: PhantomData<fn() -> T>,
}

impl<T: AppSettings> SettingsPlugin<T> {
    /// Register `T` under `key` using [`AppSettings::SCHEMA_VERSION`].
    pub fn new(key: StoreKey) -> Self {
        Self {
            key,
            current_version: T::SCHEMA_VERSION,
            migrations: Vec::new(),
            validator: None,
            future_version_policy: FutureVersionPolicy::default(),
            _marker: PhantomData,
        }
    }

    /// Override the current version stamped on disk.
    ///
    /// Normally the [`AppSettings::SCHEMA_VERSION`] default is sufficient. This
    /// builder exists for the explicit §4a registration form.
    #[must_use]
    pub fn current_version(mut self, version: u32) -> Self {
        self.current_version = version;
        self
    }

    /// Add one directed migration edge. Edges run stepwise from the version on disk.
    #[must_use]
    pub fn migrate(mut self, from: u32, to: u32, run: MigrationFn) -> Self {
        self.migrations.push(MigrationStep { from, to, run });
        self
    }

    /// Add plugin-specific validation after [`AppSettings::validate`].
    #[must_use]
    pub fn validate(mut self, validator: SettingsValidator<T>) -> Self {
        self.validator = Some(validator);
        self
    }

    /// Set downgrade behavior for a newer on-disk schema.
    #[must_use]
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
                (self.apply_migrations(found, raw)?, None)
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

    fn apply_migrations(&self, mut version: u32, mut raw: toml::Value) -> Result<T, SettingsError> {
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
            let table = raw.as_table_mut().ok_or_else(|| SettingsError::Migration {
                from: migration.from,
                to: migration.to,
                message: "migration returned a non-table TOML value".to_string(),
            })?;
            table.insert(
                "schema_version".to_string(),
                toml::Value::Integer(i64::from(migration.to)),
            );
            version = migration.to;
        }

        let envelope: Envelope<T> = raw
            .try_into()
            .map_err(StorageError::from)
            .map_err(SettingsError::from)?;
        Ok(envelope.inner)
    }

    fn flush_for_exit(&self, cx: &mut App) {
        let result = cx
            .global_mut::<SettingsRegistry>()
            .flush_for_exit::<T>(&self.key);
        if let Err(error) = result {
            crate::handles::report_error(
                cx,
                crate::error::RuntimeError::service(anyhow::Error::new(error)),
            );
        }
    }
}

impl<T: AppSettings> crate::plugin::sealed::Sealed for SettingsPlugin<T> {}

impl<T: AppSettings> AppPlugin for SettingsPlugin<T> {
    fn init(&mut self, cx: &mut App, shell: &ShellSeed) -> Result<(), AppShellError> {
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
        let entry = self.open_entry(shell.info.paths()).map_err(service_error)?;
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
    AppShellError::Service {
        service: "settings",
        source: anyhow::Error::new(error),
    }
}

fn decode_value<T: AppSettings>(raw: toml::Value) -> Result<T, SettingsError> {
    raw.try_into()
        .map_err(StorageError::from)
        .map_err(SettingsError::from)
}

fn validate_value<T: AppSettings>(
    value: &T,
    validator: Option<SettingsValidator<T>>,
) -> Result<(), SettingsError> {
    value.validate().map_err(SettingsError::Validation)?;
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

fn shell_preferences_key() -> StoreKey {
    StoreKey(Cow::Borrowed("shell-preferences"))
}

/// Dedicated plugin for the automatically registered shell-preferences store.
pub struct ShellPreferencesPlugin(SettingsPlugin<ShellPreferences>);

impl ShellPreferencesPlugin {
    /// Create the shell-preferences plugin.
    pub fn new() -> Self {
        Self(SettingsPlugin::new(shell_preferences_key()))
    }
}

impl Default for ShellPreferencesPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl crate::plugin::sealed::Sealed for ShellPreferencesPlugin {}

impl AppPlugin for ShellPreferencesPlugin {
    fn init(&mut self, cx: &mut App, shell: &ShellSeed) -> Result<(), AppShellError> {
        self.0.init(cx, shell)
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
