use std::any::{Any, TypeId};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, mpsc};
use std::time::Instant;

use gpui::BorrowAppContext;

use super::*;

pub(super) trait ErasedSettingsEntry {
    fn settings_type_id(&self) -> TypeId;
    fn settings_type_name(&self) -> &'static str;
    fn value(&self) -> &dyn Any;
    fn value_mut(&mut self) -> &mut dyn Any;
    fn snapshot_for_update(&self) -> Result<toml::Value, SettingsError>;
    fn finish_update(&mut self, previous: toml::Value) -> Result<(), SettingsError>;
    fn queue_current_erased(&mut self) -> Result<(), SettingsError>;
    fn version(&self) -> u64;
    fn last_error(&self) -> Option<String>;
    fn flush(&mut self) -> Result<(), SettingsError>;
    fn flush_for_exit(&mut self, deadline: Instant) -> Result<(), SettingsError>;
}

#[cfg(test)]
pub(super) type ExitFlushHook = Arc<dyn Fn() -> Result<(), SettingsError> + Send + Sync>;

pub(super) struct SettingsEntry<T: AppSettings> {
    pub(super) key: String,
    pub(super) value: T,
    pub(super) store: Arc<DebouncedStore<toml::Value>>,
    pub(super) current_version: u32,
    pub(super) future_version: Option<u32>,
    pub(super) future_version_policy: FutureVersionPolicy,
    pub(super) validator: Option<SettingsValidator<T>>,
    pub(super) version: u64,
    pub(super) lifecycle_error: Option<String>,
    pub(super) exit_flushed: bool,
    #[cfg(test)]
    pub(super) exit_flush_hook: Option<ExitFlushHook>,
}

impl<T: AppSettings> SettingsEntry<T> {
    fn write_guard(&self) -> Result<(), SettingsError> {
        match (self.future_version, self.future_version_policy) {
            (Some(found), FutureVersionPolicy::RefuseToWrite) => {
                Err(SettingsError::UnsupportedFutureVersion {
                    found,
                    supported: self.current_version,
                })
            }
            _ => Ok(()),
        }
    }

    pub(super) fn queue_current(&self) -> Result<(), SettingsError> {
        self.write_guard()?;
        validate_value(&self.value, self.validator)?;
        let snapshot = toml::Value::try_from(&self.value)
            .map_err(StorageError::from)
            .map_err(SettingsError::from)?;
        self.store.put(snapshot)?;
        Ok(())
    }

    pub(super) fn snapshot_for_update(&self) -> Result<toml::Value, SettingsError> {
        self.write_guard()?;
        toml::Value::try_from(&self.value)
            .map_err(StorageError::from)
            .map_err(SettingsError::from)
    }

    pub(super) fn finish_update(&mut self, previous: toml::Value) -> Result<(), SettingsError> {
        if let Err(error) = self.queue_current_erased() {
            self.value = decode_value(previous)?;
            return Err(error);
        }
        Ok(())
    }
}

impl<T: AppSettings> ErasedSettingsEntry for SettingsEntry<T> {
    fn settings_type_id(&self) -> TypeId {
        TypeId::of::<T>()
    }

    fn settings_type_name(&self) -> &'static str {
        std::any::type_name::<T>()
    }

    fn value(&self) -> &dyn Any {
        &self.value
    }

    fn value_mut(&mut self) -> &mut dyn Any {
        &mut self.value
    }

    fn snapshot_for_update(&self) -> Result<toml::Value, SettingsError> {
        SettingsEntry::snapshot_for_update(self)
    }

    fn finish_update(&mut self, previous: toml::Value) -> Result<(), SettingsError> {
        SettingsEntry::finish_update(self, previous)
    }

    fn queue_current_erased(&mut self) -> Result<(), SettingsError> {
        self.queue_current()?;
        self.version = self.version.saturating_add(1);
        self.exit_flushed = false;
        Ok(())
    }

    fn version(&self) -> u64 {
        self.version
    }

    fn last_error(&self) -> Option<String> {
        self.lifecycle_error
            .clone()
            .or_else(|| self.store.last_error())
    }

    fn flush(&mut self) -> Result<(), SettingsError> {
        match self.store.flush() {
            Ok(()) => {
                self.lifecycle_error = None;
                Ok(())
            }
            Err(error) => {
                self.lifecycle_error = Some(error.to_string());
                Err(error.into())
            }
        }
    }

    fn flush_for_exit(&mut self, deadline: Instant) -> Result<(), SettingsError> {
        if self.exit_flushed {
            return Ok(());
        }
        let store = Arc::clone(&self.store);
        #[cfg(test)]
        let exit_flush_hook = self.exit_flush_hook.clone();
        let result = run_exit_flush(self.key.clone(), deadline, move || {
            #[cfg(test)]
            if let Some(hook) = exit_flush_hook {
                return hook();
            }
            store.flush().map_err(SettingsError::from)
        });
        match &result {
            // Only a genuinely completed direct entry flush marks it done. The
            // registry owns one-shot shutdown coordination and reports failures.
            Ok(()) => {
                self.exit_flushed = true;
                self.lifecycle_error = None;
            }
            Err(error) => self.lifecycle_error = Some(error.to_string()),
        }
        result
    }
}

fn run_exit_flush(
    key: String,
    deadline: Instant,
    flush: impl FnOnce() -> Result<(), SettingsError> + Send + 'static,
) -> Result<(), SettingsError> {
    let timeout = deadline.saturating_duration_since(Instant::now());
    if timeout.is_zero() {
        return Err(SettingsError::FlushTimedOut { key, timeout });
    }

    let (tx, rx) = mpsc::channel();
    if let Err(error) = std::thread::Builder::new()
        .name("gpui-settings-flush".to_string())
        .spawn(move || {
            let _ = tx.send(flush());
        })
    {
        return Err(SettingsError::FlushWorker(error.to_string()));
    }

    let timeout = deadline.saturating_duration_since(Instant::now());
    if timeout.is_zero() {
        return Err(SettingsError::FlushTimedOut { key, timeout });
    }
    match rx.recv_timeout(timeout) {
        Ok(result) => result,
        Err(_) => Err(SettingsError::FlushTimedOut { key, timeout }),
    }
}

#[derive(Default)]
struct ExitFlushCoordinator {
    deadline: Option<Instant>,
    claimed: HashSet<String>,
}

impl ExitFlushCoordinator {
    fn claim(&mut self, key: &str) -> Option<Instant> {
        if !self.claimed.insert(key.to_string()) {
            return None;
        }
        Some(
            *self
                .deadline
                .get_or_insert_with(|| Instant::now() + EXIT_FLUSH_TIMEOUT),
        )
    }

    fn expire(&mut self) {
        self.deadline = Some(Instant::now());
    }
}

#[derive(Default)]
pub(super) struct SettingsRegistry {
    pub(super) entries: HashMap<String, Box<dyn ErasedSettingsEntry>>,
    exit_flush: ExitFlushCoordinator,
}

impl gpui::Global for SettingsRegistry {}

impl SettingsRegistry {
    fn entry<T: AppSettings>(
        &self,
        key: &StoreKey,
    ) -> Result<&dyn ErasedSettingsEntry, SettingsError> {
        let entry = self
            .entries
            .get(&key.filename())
            .ok_or_else(|| SettingsError::NotRegistered(key.as_str().to_string()))?;
        ensure_type::<T>(entry.as_ref(), key)?;
        Ok(entry.as_ref())
    }

    pub(super) fn entry_mut<T: AppSettings>(
        &mut self,
        key: &StoreKey,
    ) -> Result<&mut dyn ErasedSettingsEntry, SettingsError> {
        let entry = self
            .entries
            .get_mut(&key.filename())
            .ok_or_else(|| SettingsError::NotRegistered(key.as_str().to_string()))?;
        ensure_type::<T>(entry.as_ref(), key)?;
        Ok(entry.as_mut())
    }

    pub(super) fn flush_for_exit<T: AppSettings>(
        &mut self,
        key: &StoreKey,
    ) -> Result<(), SettingsError> {
        let Some(deadline) = self.exit_flush.claim(&key.filename()) else {
            return Ok(());
        };
        let result = self.entry_mut::<T>(key)?.flush_for_exit(deadline);
        if matches!(&result, Err(SettingsError::FlushTimedOut { .. })) {
            self.exit_flush.expire();
        }
        result
    }
}

fn ensure_type<T: AppSettings>(
    entry: &dyn ErasedSettingsEntry,
    key: &StoreKey,
) -> Result<(), SettingsError> {
    if entry.settings_type_id() == TypeId::of::<T>() {
        Ok(())
    } else {
        Err(SettingsError::TypeMismatch {
            key: key.as_str().to_string(),
            expected: std::any::type_name::<T>(),
            actual: entry.settings_type_name(),
        })
    }
}

/// Typed settings access on raw [`gpui::App`], matching [`crate::AppShellExt`].
///
/// Change observation uses a cheap per-store monotonic version counter. It
/// increments after an update is validated and accepted by `DebouncedStore`.
pub trait SettingsExt {
    /// Borrow the current value. Panics if the requested store is not registered.
    fn settings<T: AppSettings>(&self, key: StoreKey) -> &T;

    /// Mutate, validate, and queue a debounced save. Invalid changes are rolled back.
    fn update_settings<T: AppSettings, R>(
        &mut self,
        key: StoreKey,
        update: impl FnOnce(&mut T, &mut App) -> R,
    ) -> Result<R, SettingsError>;

    /// Current change-observation version.
    fn settings_version<T: AppSettings>(&self, key: StoreKey) -> Result<u64, SettingsError>;

    /// Most recent background or lifecycle flush error.
    fn settings_last_error<T: AppSettings>(
        &self,
        key: StoreKey,
    ) -> Result<Option<String>, SettingsError>;

    /// Synchronously flush pending data and return any persistence failure.
    fn flush_settings<T: AppSettings>(&mut self, key: StoreKey) -> Result<(), SettingsError>;
}

impl SettingsExt for App {
    fn settings<T: AppSettings>(&self, key: StoreKey) -> &T {
        self.global::<SettingsRegistry>()
            .entry::<T>(&key)
            .unwrap_or_else(|error| panic!("{error}"))
            .value()
            .downcast_ref::<T>()
            .expect("settings registry type check and downcast disagreed")
    }

    fn update_settings<T: AppSettings, R>(
        &mut self,
        key: StoreKey,
        update: impl FnOnce(&mut T, &mut App) -> R,
    ) -> Result<R, SettingsError> {
        self.update_global::<SettingsRegistry, _>(|registry, cx| {
            let entry = registry.entry_mut::<T>(&key)?;
            let previous = entry.snapshot_for_update()?;
            let typed = entry
                .value_mut()
                .downcast_mut::<T>()
                .expect("settings registry type check and downcast disagreed");
            let result = update(typed, cx);
            entry.finish_update(previous)?;
            Ok(result)
        })
    }

    fn settings_version<T: AppSettings>(&self, key: StoreKey) -> Result<u64, SettingsError> {
        Ok(self
            .global::<SettingsRegistry>()
            .entry::<T>(&key)?
            .version())
    }

    fn settings_last_error<T: AppSettings>(
        &self,
        key: StoreKey,
    ) -> Result<Option<String>, SettingsError> {
        Ok(self
            .global::<SettingsRegistry>()
            .entry::<T>(&key)?
            .last_error())
    }

    fn flush_settings<T: AppSettings>(&mut self, key: StoreKey) -> Result<(), SettingsError> {
        self.global_mut::<SettingsRegistry>()
            .entry_mut::<T>(&key)?
            .flush()
    }
}
