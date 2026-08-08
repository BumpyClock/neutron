//! Platform path resolution.
//!
//! [`AppPaths`] resolves the standard set of per-app directories from a stable
//! `namespace` (never a display name — the namespace is what user data is keyed
//! on and must not change when the product is renamed).
//!
//! Two layouts are supported:
//!
//! - [`PathLayout::PlatformDefault`] follows OS conventions via the `dirs`
//!   crate (Application Support on macOS, XDG on Linux, `%APPDATA%` /
//!   `%LOCALAPPDATA%` on Windows).
//! - [`PathLayout::SingleRoot`] puts everything under one home-relative
//!   directory (e.g. `~/.agent-term`), matching apps that keep a single opaque
//!   dotfile tree.
//!
//! Resolution never creates directories; writers create them lazily via
//! [`AppPaths::ensure`]. This keeps a read-only `AppPaths` cheap and side-effect
//! free.

use std::path::{Component, Path, PathBuf};

use crate::error::StorageError;

/// Reject values that are empty, absolute, or contain traversal (`..`)/`.`
/// components. Such values would escape the platform base directory via
/// `PathBuf::join` (which discards the left side on an absolute right side and
/// walks upward on `..`).
fn validate_relative(kind: &'static str, namespace: &str, value: &str) -> Result<(), StorageError> {
    let mut components = 0usize;
    for component in Path::new(value).components() {
        match component {
            Component::Normal(_) => components += 1,
            _ => {
                return Err(StorageError::PathResolution {
                    kind,
                    namespace: namespace.to_string(),
                });
            }
        }
    }
    if components == 0 {
        // Empty, or only separators/current-dir markers.
        return Err(StorageError::PathResolution {
            kind,
            namespace: namespace.to_string(),
        });
    }
    Ok(())
}

/// How the app's directories map onto the filesystem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathLayout {
    /// OS-native locations resolved through the `dirs` crate.
    PlatformDefault,
    /// Everything under a single home-relative directory, e.g. `".agent-term"`.
    SingleRoot(String),
}

/// Selects one of the resolved base directories, for use with
/// [`AppPaths::sub`] and [`AppPaths::ensure`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BaseDir {
    /// User configuration.
    Config,
    /// Roaming / primary application data.
    Data,
    /// Machine-local application data (distinct from `Data` only on Windows).
    LocalData,
    /// Disposable cache data.
    Cache,
    /// Log files.
    Log,
    /// State that should persist but is not user config (e.g. window layout).
    State,
    /// Scratch/temporary files scoped to this app.
    Temp,
}

/// Resolved per-app directory set. Cheap to clone; holds no OS handles.
#[derive(Debug, Clone)]
pub struct AppPaths {
    namespace: String,
    layout: PathLayout,
    config_dir: PathBuf,
    data_dir: PathBuf,
    local_data_dir: PathBuf,
    cache_dir: PathBuf,
    log_dir: PathBuf,
    state_dir: PathBuf,
    temp_dir: PathBuf,
}

fn resolve(
    kind: &'static str,
    namespace: &str,
    base: Option<PathBuf>,
) -> Result<PathBuf, StorageError> {
    base.map(|b| b.join(namespace))
        .ok_or_else(|| StorageError::PathResolution {
            kind,
            namespace: namespace.to_string(),
        })
}

impl AppPaths {
    /// Resolve directories for `namespace` under the given `layout`.
    ///
    /// `namespace` is a stable identifier (e.g. `"Ansible"` or `"agent-term"`),
    /// never a localized/display name. No directories are created here.
    pub fn new(namespace: &str, layout: PathLayout) -> Result<Self, StorageError> {
        validate_relative("namespace", namespace, namespace)?;
        match &layout {
            PathLayout::PlatformDefault => Self::platform_default(namespace, layout.clone()),
            PathLayout::SingleRoot(root) => {
                validate_relative("root", namespace, root)?;
                Self::single_root(namespace, root.clone(), layout.clone())
            }
        }
    }

    fn platform_default(namespace: &str, layout: PathLayout) -> Result<Self, StorageError> {
        let config_dir = resolve("config", namespace, dirs::config_dir())?;
        let data_dir = resolve("data", namespace, dirs::data_dir())?;
        let local_data_dir = resolve("local data", namespace, dirs::data_local_dir())?;
        let cache_dir = resolve("cache", namespace, dirs::cache_dir())?;

        // `dirs::state_dir()` is only populated on Linux; fall back to data.
        let state_dir = match dirs::state_dir() {
            Some(base) => base.join(namespace),
            None => data_dir.clone(),
        };

        // Logs: macOS has a dedicated `~/Library/Logs`; elsewhere nest under the
        // machine-local state/data tree.
        let log_dir = {
            #[cfg(target_os = "macos")]
            {
                match dirs::home_dir() {
                    Some(home) => home.join("Library").join("Logs").join(namespace),
                    None => state_dir.join("logs"),
                }
            }
            #[cfg(target_os = "windows")]
            {
                local_data_dir.join("logs")
            }
            #[cfg(not(any(target_os = "macos", target_os = "windows")))]
            {
                state_dir.join("logs")
            }
        };

        let temp_dir = std::env::temp_dir().join(namespace);

        Ok(Self {
            namespace: namespace.to_string(),
            layout,
            config_dir,
            data_dir,
            local_data_dir,
            cache_dir,
            log_dir,
            state_dir,
            temp_dir,
        })
    }

    fn single_root(
        namespace: &str,
        root: String,
        layout: PathLayout,
    ) -> Result<Self, StorageError> {
        let base = dirs::home_dir().map(|h| h.join(&root)).ok_or_else(|| {
            StorageError::PathResolution {
                kind: "home",
                namespace: namespace.to_string(),
            }
        })?;

        let data_dir = base.join("data");
        Ok(Self {
            namespace: namespace.to_string(),
            layout,
            config_dir: base.join("config"),
            local_data_dir: data_dir.clone(),
            data_dir,
            cache_dir: base.join("cache"),
            log_dir: base.join("logs"),
            state_dir: base.join("state"),
            temp_dir: base.join("tmp"),
        })
    }

    /// The stable namespace these paths were resolved from.
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// The layout used to resolve these paths.
    pub fn layout(&self) -> &PathLayout {
        &self.layout
    }

    /// User configuration directory.
    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    /// Roaming / primary application data directory.
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// Machine-local application data directory. Equal to [`data_dir`] except on
    /// Windows, where it maps to `%LOCALAPPDATA%`.
    ///
    /// [`data_dir`]: Self::data_dir
    pub fn local_data_dir(&self) -> &Path {
        &self.local_data_dir
    }

    /// Disposable cache directory.
    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    /// Log directory.
    pub fn log_dir(&self) -> &Path {
        &self.log_dir
    }

    /// Persistent (non-config) state directory.
    pub fn state_dir(&self) -> &Path {
        &self.state_dir
    }

    /// App-scoped temporary directory.
    pub fn temp_dir(&self) -> &Path {
        &self.temp_dir
    }

    fn base(&self, base: BaseDir) -> &Path {
        match base {
            BaseDir::Config => &self.config_dir,
            BaseDir::Data => &self.data_dir,
            BaseDir::LocalData => &self.local_data_dir,
            BaseDir::Cache => &self.cache_dir,
            BaseDir::Log => &self.log_dir,
            BaseDir::State => &self.state_dir,
            BaseDir::Temp => &self.temp_dir,
        }
    }

    /// Join `name` onto one of the resolved base directories without creating
    /// anything.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::PathResolution`] if `name` is empty, absolute, or
    /// contains `..` — anything that could discard or escape the base directory.
    pub fn sub(&self, base: BaseDir, name: &str) -> Result<PathBuf, StorageError> {
        validate_relative("sub-path", &self.namespace, name)?;
        Ok(self.base(base).join(name))
    }

    /// Create (if needed) and return one of the base directories. This is the
    /// lazy-creation entry point writers use before persisting.
    pub fn ensure(&self, base: BaseDir) -> Result<PathBuf, StorageError> {
        let dir = self.base(base).to_path_buf();
        std::fs::create_dir_all(&dir).map_err(|e| StorageError::io(&dir, e))?;
        Ok(dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_default_namespaces_every_dir() {
        let paths = AppPaths::new("StorageTestApp", PathLayout::PlatformDefault).unwrap();
        for dir in [
            paths.config_dir(),
            paths.data_dir(),
            paths.local_data_dir(),
            paths.cache_dir(),
            paths.log_dir(),
            paths.state_dir(),
            paths.temp_dir(),
        ] {
            assert!(
                dir.to_string_lossy().contains("StorageTestApp"),
                "{} missing namespace",
                dir.display()
            );
        }
    }

    #[test]
    fn platform_default_creates_nothing() {
        // A per-run unique namespace: no such directory can pre-exist, so a
        // regression that creates it on resolution is actually detected (the old
        // assertion passed whenever the dir happened to exist and be readable).
        let namespace = format!(
            "StorageTestUncreated-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let paths = AppPaths::new(&namespace, PathLayout::PlatformDefault).unwrap();
        // Resolution alone must not touch the filesystem.
        assert!(
            !paths.config_dir().exists(),
            "resolution created {:?}",
            paths.config_dir()
        );
    }

    #[test]
    fn single_root_nests_under_home() {
        let home = dirs::home_dir().unwrap();
        let paths =
            AppPaths::new("app", PathLayout::SingleRoot(".storage-test".to_string())).unwrap();
        let base = home.join(".storage-test");
        assert_eq!(paths.config_dir(), base.join("config"));
        assert_eq!(paths.data_dir(), base.join("data"));
        assert_eq!(paths.log_dir(), base.join("logs"));
        // local data equals data outside Windows' roaming split.
        assert_eq!(paths.local_data_dir(), paths.data_dir());
    }

    #[test]
    fn sub_and_ensure() {
        // Use a per-run unique root so the test only ever creates and deletes a
        // directory it exclusively owns — never a pre-existing user path.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root_name = format!(".storage-ensure-{}-{}", std::process::id(), nanos);
        let paths = AppPaths::new("nested", PathLayout::SingleRoot(root_name.clone())).unwrap();

        // `sub` is pure path arithmetic, no filesystem access.
        assert_eq!(
            paths.sub(BaseDir::Data, "profiles").unwrap(),
            paths.data_dir().join("profiles")
        );

        // A name that would escape or discard the base directory is rejected.
        for bad in ["", "..", "../evil", "/etc/passwd", "."] {
            assert!(
                matches!(
                    paths.sub(BaseDir::Data, bad),
                    Err(StorageError::PathResolution { .. })
                ),
                "sub name {bad:?} should be rejected"
            );
        }

        // `ensure` creates the directory on demand.
        let created = paths.ensure(BaseDir::Cache).unwrap();
        assert!(created.is_dir());

        // Clean up only the uniquely-named root this test created.
        let root = dirs::home_dir().unwrap().join(&root_name);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_traversing_or_absolute_namespaces() {
        for bad in ["", "..", "../evil", "/etc", "."] {
            assert!(
                matches!(
                    AppPaths::new(bad, PathLayout::PlatformDefault),
                    Err(StorageError::PathResolution { .. })
                ),
                "namespace {bad:?} should be rejected"
            );
        }
        // A traversing SingleRoot value is rejected too.
        assert!(matches!(
            AppPaths::new("ok", PathLayout::SingleRoot("../escape".to_string())),
            Err(StorageError::PathResolution { .. })
        ));
        // A normal namespace still resolves.
        assert!(AppPaths::new("GoodApp", PathLayout::PlatformDefault).is_ok());
    }
}
