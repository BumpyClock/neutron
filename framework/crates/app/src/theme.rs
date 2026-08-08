//! Theme selection, bundled-theme synchronization, and registry integration.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use anyhow::{Context as _, Result};
use gpui::{Action, App, Global};
use gpui_component::{Theme, ThemeConfig, ThemeModePreference, ThemeRegistry};
use serde::{Deserialize, Serialize};

use crate::error::AppShellError;
use crate::plugin::{self, AppPlugin, ShellSeed};

const DEFAULT_THEME_SET: &str = "Default";
const BUNDLED_MANIFEST: &str = "themes-bundled.lst";

/// One theme file supplied by a [`ThemeAssetSource`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThemeAsset {
    /// File name relative to the application's theme directory.
    pub name: String,
    /// Complete JSON file contents.
    pub contents: Arc<[u8]>,
}

impl ThemeAsset {
    /// Construct an owned bundled theme file.
    pub fn new(name: impl Into<String>, contents: impl Into<Arc<[u8]>>) -> Self {
        Self {
            name: name.into(),
            contents: contents.into(),
        }
    }
}

/// Minimal embedded-theme abstraction.
///
/// Apps using `rust_embed` adapt its iterator/get API here. Keeping this trait
/// independent prevents `rust_embed` and its proc macro from becoming mandatory
/// dependencies of every app-shell consumer.
pub trait ThemeAssetSource: Send + Sync + 'static {
    /// Return all bundled theme files.
    fn theme_assets(&self) -> Result<Vec<ThemeAsset>>;
}

impl<F> ThemeAssetSource for F
where
    F: Fn() -> Result<Vec<ThemeAsset>> + Send + Sync + 'static,
{
    fn theme_assets(&self) -> Result<Vec<ThemeAsset>> {
        self()
    }
}

/// Source of themes managed by [`ThemePlugin`].
pub enum ThemeSource {
    /// Embedded files synchronized to `config_dir/themes`.
    ///
    /// Bundled themes are mirrored to the config directory and then loaded
    /// through the registry, so bundled edits and newly installed user themes
    /// hot-reload normally.
    ///
    /// Assets are always synced to disk: gpui-component's [`ThemeRegistry`]
    /// exposes no in-memory registration API and keeps its theme maps private,
    /// so the watched config directory is the only path that makes bundled
    /// themes visible to theme selection.
    Bundled {
        /// Application-provided embedded files.
        assets: Arc<dyn ThemeAssetSource>,
    },
    /// JSON registry rooted at the supplied directory or `config_dir/themes`.
    Registry {
        /// Override for the watched theme directory.
        dir: Option<PathBuf>,
    },
    /// Fixed palettes. No registry lookup or file watching occurs.
    Custom {
        /// Light palette.
        light: Box<ThemeConfig>,
        /// Dark palette.
        dark: Box<ThemeConfig>,
    },
}

impl ThemeSource {
    /// Construct a bundled source that syncs into the config directory.
    pub fn bundled(assets: impl ThemeAssetSource) -> Self {
        Self::Bundled {
            assets: Arc::new(assets),
        }
    }

    /// Construct a registry using `config_dir/themes`.
    pub fn registry() -> Self {
        Self::Registry { dir: None }
    }

    /// Construct fixed light and dark palettes.
    pub fn custom(light: ThemeConfig, dark: ThemeConfig) -> Self {
        Self::Custom {
            light: Box::new(light),
            dark: Box::new(dark),
        }
    }
}

/// Persistable theme choice. Storage ownership remains with the application.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemeSelection {
    /// System/light/dark preference.
    pub mode: ThemeModePreference,
    /// Preferred registry theme-set name.
    pub name: Option<String>,
}

/// Global action selecting a registry theme set.
#[derive(Action, Clone, PartialEq, Eq)]
#[action(namespace = theme, no_json)]
pub struct SwitchTheme(pub String);

/// Global action selecting system/light/dark mode.
#[derive(Action, Clone, PartialEq, Eq)]
#[action(namespace = theme, no_json)]
pub struct SwitchThemeMode(pub ThemeModePreference);

/// Logical section for a [`ThemeMenuItem`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeMenuGroup {
    /// System/light/dark radio group.
    Mode,
    /// Registry theme-set radio group.
    Theme,
}

/// Command-neutral theme menu contribution.
pub struct ThemeMenuItem {
    /// Display label.
    pub label: String,
    /// GPUI action dispatched when selected.
    pub action: Box<dyn Action>,
    /// Whether this radio item is active.
    pub checked: bool,
    /// Menu section/radio group.
    pub group: ThemeMenuGroup,
}

type ReadPreference = fn(&App) -> Option<ThemeSelection>;
type WritePreference = fn(&mut App, &ThemeSelection);

#[derive(Clone, Copy, Default)]
struct PreferenceHooks {
    read: Option<ReadPreference>,
    write: Option<WritePreference>,
}

impl PreferenceHooks {
    fn read(self, cx: &App) -> Option<ThemeSelection> {
        call_read_hook(self.read, cx)
    }

    fn write(self, cx: &mut App, selection: &ThemeSelection) {
        call_write_hook(self.write, cx, selection);
    }
}

fn call_read_hook<C>(
    hook: Option<fn(&C) -> Option<ThemeSelection>>,
    cx: &C,
) -> Option<ThemeSelection> {
    hook.and_then(|read| read(cx))
}

fn call_write_hook<C>(
    hook: Option<fn(&mut C, &ThemeSelection)>,
    cx: &mut C,
    selection: &ThemeSelection,
) {
    if let Some(write) = hook {
        write(cx, selection);
    }
}

struct ThemeState {
    selection: ThemeSelection,
    hooks: PreferenceHooks,
    custom: Option<(Rc<ThemeConfig>, Rc<ThemeConfig>)>,
}

impl Global for ThemeState {}

#[derive(Default)]
struct ThemeRegistryVersion(u64);

impl Global for ThemeRegistryVersion {}

/// Register a callback for registry loads and hot reloads.
///
/// Register after `ThemePlugin` initialization. The callback runs after the
/// current selection has been re-applied, so command/menu integrations can
/// rebuild labels and checked state from [`theme_menu_items`]. Retain the
/// returned subscription, or call `detach()` for an app-lifetime listener.
pub fn on_theme_registry_changed(
    cx: &mut App,
    f: impl FnMut(&mut App) + 'static,
) -> gpui::Subscription {
    cx.observe_global::<ThemeRegistryVersion>(f)
}

/// Return mode and theme-set radio items for a command/menu projection.
pub fn theme_menu_items(cx: &App) -> Vec<ThemeMenuItem> {
    let active_mode = Theme::global(cx).mode_preference;
    let mut items = [
        ("System", ThemeModePreference::System),
        ("Light", ThemeModePreference::Light),
        ("Dark", ThemeModePreference::Dark),
    ]
    .into_iter()
    .map(|(label, mode)| ThemeMenuItem {
        label: label.to_owned(),
        action: Box::new(SwitchThemeMode(mode)),
        checked: active_mode == mode,
        group: ThemeMenuGroup::Mode,
    })
    .collect::<Vec<_>>();

    let uses_registry = cx
        .try_global::<ThemeState>()
        .is_none_or(|state| state.custom.is_none());
    if uses_registry {
        let active_name = Theme::global(cx).theme_set_name.clone();
        items.extend(
            ThemeRegistry::global(cx)
                .sorted_theme_sets()
                .into_iter()
                .map(|set| ThemeMenuItem {
                    label: set.name.to_string(),
                    action: Box::new(SwitchTheme(set.name.to_string())),
                    checked: active_name == set.name,
                    group: ThemeMenuGroup::Theme,
                }),
        );
    }
    items
}

/// App-shell plugin providing theme selection and registry lifecycle.
pub struct ThemePlugin {
    source: Option<ThemeSource>,
    hooks: PreferenceHooks,
}

impl ThemePlugin {
    /// Construct a theme plugin.
    pub fn new(source: ThemeSource) -> Self {
        Self {
            source: Some(source),
            hooks: PreferenceHooks::default(),
        }
    }

    /// Supply the only persistence seam used by this module.
    ///
    /// Without these hooks, selection remains process-local and writes are no-ops.
    pub fn with_preferences(mut self, read: ReadPreference, write: WritePreference) -> Self {
        self.hooks = PreferenceHooks {
            read: Some(read),
            write: Some(write),
        };
        self
    }
}

impl plugin::sealed::Sealed for ThemePlugin {}

impl AppPlugin for ThemePlugin {
    fn init(&mut self, cx: &mut App, shell: &ShellSeed) -> Result<(), AppShellError> {
        let source = self.source.take().expect("ThemePlugin initialized twice");
        let selection = self.hooks.read(cx).unwrap_or_default();
        let mut watch_dir = None;
        let custom = match source {
            ThemeSource::Bundled { assets } => {
                let config_dir = shell.info.paths().config_dir();
                let themes_dir = config_dir.join("themes");
                if let Err(error) = sync_bundled_themes(assets.as_ref(), config_dir) {
                    crate::handles::report_error(cx, crate::error::RuntimeError::service(error));
                }
                watch_dir = Some(themes_dir);
                None
            }
            ThemeSource::Registry { dir } => {
                watch_dir =
                    Some(dir.unwrap_or_else(|| shell.info.paths().config_dir().join("themes")));
                None
            }
            ThemeSource::Custom { light, dark } => Some((Rc::new(*light), Rc::new(*dark))),
        };

        cx.set_global(ThemeState {
            selection,
            hooks: self.hooks,
            custom,
        });
        cx.set_global(ThemeRegistryVersion::default());
        register_actions(cx);
        apply_current_selection(cx);

        if let Some(dir) = watch_dir {
            cx.observe_global::<ThemeRegistry>(registry_reloaded)
                .detach();
            if let Err(error) = ThemeRegistry::watch_dir(dir, cx, |_| {}) {
                crate::handles::report_error(cx, crate::error::RuntimeError::service(error));
            }
        }

        Ok(())
    }
}

fn register_actions(cx: &mut App) {
    cx.on_action(|action: &SwitchTheme, cx| {
        if cx.global::<ThemeState>().custom.is_some() {
            return;
        }
        let mut selection = cx.global::<ThemeState>().selection.clone();
        selection.name = Some(action.0.clone());
        update_selection(selection, cx);
    });
    cx.on_action(|action: &SwitchThemeMode, cx| {
        let mut selection = cx.global::<ThemeState>().selection.clone();
        selection.mode = action.0;
        update_selection(selection, cx);
    });
}

fn update_selection(selection: ThemeSelection, cx: &mut App) {
    let hooks = cx.global::<ThemeState>().hooks;
    cx.global_mut::<ThemeState>().selection = selection.clone();
    apply_current_selection(cx);
    hooks.write(cx, &selection);
    cx.refresh_windows();
}

fn registry_reloaded(cx: &mut App) {
    apply_current_selection(cx);
    cx.refresh_windows();
    // Unlike upstream Zed, this fork's `global_mut` pushes
    // `NotifyGlobalObservers`, so `observe_global::<ThemeRegistryVersion>`
    // subscribers (the menu bridge) fire from this bump.
    cx.global_mut::<ThemeRegistryVersion>().0 += 1;
}

fn apply_current_selection(cx: &mut App) {
    let (selection, custom) = {
        let state = cx.global::<ThemeState>();
        (state.selection.clone(), state.custom.clone())
    };
    if let Some((light, dark)) = custom {
        let theme = Theme::global_mut(cx);
        theme.light_theme = light;
        theme.dark_theme = dark;
        theme.theme_set_name = "Custom".into();
        theme.mode_preference = selection.mode;
        Theme::sync_system_appearance(None, cx);
        return;
    }

    let selected_name = resolve_theme_set_name(
        selection.name.as_deref(),
        ThemeRegistry::global(cx)
            .sorted_theme_sets()
            .into_iter()
            .map(|set| set.name.to_string()),
    );
    let selected = selected_name.and_then(|name| {
        ThemeRegistry::global(cx)
            .theme_sets()
            .get(name.as_str())
            .cloned()
    });
    if let Some(set) = selected {
        Theme::apply_theme_set(&set, selection.mode, None, cx);
    } else {
        Theme::global_mut(cx).mode_preference = selection.mode;
        Theme::sync_system_appearance(None, cx);
    }
}

fn resolve_theme_set_name(
    persisted: Option<&str>,
    names: impl IntoIterator<Item = String>,
) -> Option<String> {
    let names = names.into_iter().collect::<Vec<_>>();
    if let Some(persisted) = persisted
        && names.iter().any(|candidate| candidate == persisted)
    {
        return Some(persisted.to_owned());
    }
    if names.iter().any(|name| name == DEFAULT_THEME_SET) {
        return Some(DEFAULT_THEME_SET.to_owned());
    }
    names.into_iter().next()
}

#[derive(Debug, Default, PartialEq, Eq)]
struct SyncPlan {
    write: Vec<String>,
    remove: Vec<String>,
}

fn plan_bundled_sync(
    embedded: &BTreeMap<String, Vec<u8>>,
    installed: &BTreeMap<String, Vec<u8>>,
    previous_manifest: &BTreeSet<String>,
) -> SyncPlan {
    let write = embedded
        .iter()
        .filter(|(name, contents)| installed.get(*name) != Some(*contents))
        .map(|(name, _)| name.clone())
        .collect();
    let current_names = embedded.keys().cloned().collect::<BTreeSet<_>>();
    let remove = previous_manifest
        .difference(&current_names)
        .cloned()
        .collect();
    SyncPlan { write, remove }
}

fn sync_bundled_themes(source: &dyn ThemeAssetSource, config_dir: &Path) -> Result<()> {
    let themes_dir = config_dir.join("themes");
    fs::create_dir_all(&themes_dir)
        .with_context(|| format!("create theme directory {}", themes_dir.display()))?;

    let embedded = source
        .theme_assets()?
        .into_iter()
        .map(|asset| {
            validate_theme_file_name(&asset.name)?;
            Ok((asset.name, asset.contents.as_ref().to_vec()))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    let manifest_path = config_dir.join(BUNDLED_MANIFEST);
    let previous_manifest = read_manifest(&manifest_path);
    let relevant_names = embedded
        .keys()
        .chain(previous_manifest.iter())
        .cloned()
        .collect::<BTreeSet<_>>();
    let installed = relevant_names
        .into_iter()
        .filter_map(|name| {
            fs::read(themes_dir.join(&name))
                .ok()
                .map(|bytes| (name, bytes))
        })
        .collect::<BTreeMap<_, _>>();
    let plan = plan_bundled_sync(&embedded, &installed, &previous_manifest);

    for name in plan.write {
        fs::write(themes_dir.join(&name), &embedded[&name])
            .with_context(|| format!("write bundled theme {name}"))?;
    }
    for name in plan.remove {
        let path = themes_dir.join(&name);
        if let Err(error) = fs::remove_file(&path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                log::warn!("failed to prune bundled theme {}: {error}", path.display());
            }
        }
    }
    write_manifest(&manifest_path, embedded.keys())
}

fn validate_theme_file_name(name: &str) -> Result<()> {
    let path = Path::new(name);
    let valid = !name.is_empty()
        && path.file_name().and_then(|value| value.to_str()) == Some(name)
        && path.extension().and_then(|value| value.to_str()) == Some("json");
    anyhow::ensure!(valid, "invalid bundled theme file name `{name}`");
    Ok(())
}

fn read_manifest(path: &Path) -> BTreeSet<String> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|name| validate_theme_file_name(name).is_ok())
        .map(str::to_owned)
        .collect()
}

fn write_manifest<'a>(path: &Path, names: impl Iterator<Item = &'a String>) -> Result<()> {
    let contents = names.cloned().collect::<Vec<_>>().join("\n");
    fs::write(path, contents)
        .with_context(|| format!("write bundled-theme manifest {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn resolves_persisted_name_before_fallbacks() {
        assert_eq!(
            resolve_theme_set_name(Some("Nord"), names(&["Default", "Nord"])),
            Some("Nord".to_owned())
        );
    }

    #[test]
    fn resolves_default_then_first_sorted_name() {
        assert_eq!(
            resolve_theme_set_name(Some("Missing"), names(&["Nord", "Default"])),
            Some("Default".to_owned())
        );
        assert_eq!(
            resolve_theme_set_name(Some("Missing"), names(&["Ayu", "Zed"])),
            Some("Ayu".to_owned())
        );
    }

    #[test]
    fn resolves_empty_registry_to_none() {
        assert_eq!(resolve_theme_set_name(Some("Missing"), Vec::new()), None);
    }

    #[test]
    fn bundled_sync_writes_changes_and_prunes_only_manifest_entries() {
        let embedded = BTreeMap::from([
            ("changed.json".to_owned(), b"new".to_vec()),
            ("same.json".to_owned(), b"same".to_vec()),
            ("new.json".to_owned(), b"new".to_vec()),
        ]);
        let installed = BTreeMap::from([
            ("changed.json".to_owned(), b"old".to_vec()),
            ("same.json".to_owned(), b"same".to_vec()),
            ("removed.json".to_owned(), b"old".to_vec()),
            ("user.json".to_owned(), b"mine".to_vec()),
        ]);
        let manifest = BTreeSet::from([
            "changed.json".to_owned(),
            "same.json".to_owned(),
            "removed.json".to_owned(),
        ]);

        assert_eq!(
            plan_bundled_sync(&embedded, &installed, &manifest),
            SyncPlan {
                write: names(&["changed.json", "new.json"]),
                remove: names(&["removed.json"]),
            }
        );
    }

    #[test]
    fn bundled_sync_writes_registrable_theme_set_into_watched_dir() {
        // The registry only surfaces themes it loads from the watched directory,
        // so a bundled source must land valid, registrable `ThemeSet` JSON there
        // without any pre-existing on-disk state.
        let config_dir = tempfile::tempdir().expect("temp config dir");
        let theme_json = br#"{
            "name": "Bundled Sample",
            "themes": [
                { "name": "Bundled Sample Light", "mode": "light" },
                { "name": "Bundled Sample Dark", "mode": "dark" }
            ]
        }"#;
        let assets = {
            let theme_json = theme_json.to_vec();
            move || {
                Ok(vec![ThemeAsset::new(
                    "bundled-sample.json",
                    theme_json.clone(),
                )])
            }
        };

        sync_bundled_themes(&assets, config_dir.path()).expect("sync bundled themes");

        let written = fs::read(config_dir.path().join("themes").join("bundled-sample.json"))
            .expect("bundled theme written to watched dir");
        // Parse through the exact serde type `ThemeRegistry::reload` uses, proving
        // the synced file registers as a named theme set with both modes.
        let set: gpui_component::ThemeSet =
            serde_json::from_slice(&written).expect("synced file is a valid theme set");
        assert_eq!(set.name.as_ref(), "Bundled Sample");
        assert_eq!(set.themes.len(), 2);
    }

    #[test]
    fn default_preference_hooks_are_no_op() {
        let mut stored: Option<ThemeSelection> = None;
        let selection = ThemeSelection {
            mode: ThemeModePreference::Dark,
            name: Some("Nord".to_owned()),
        };
        assert_eq!(call_read_hook(None, &stored), None);
        call_write_hook(None, &mut stored, &selection);
        assert_eq!(stored, None);
    }

    #[test]
    fn supplied_preference_hooks_round_trip_through_function_pointers() {
        fn read(stored: &Option<ThemeSelection>) -> Option<ThemeSelection> {
            stored.clone()
        }
        fn write(stored: &mut Option<ThemeSelection>, selection: &ThemeSelection) {
            *stored = Some(selection.clone());
        }

        let selection = ThemeSelection {
            mode: ThemeModePreference::Light,
            name: Some("Ayu".to_owned()),
        };
        let mut stored = None;
        call_write_hook(Some(write), &mut stored, &selection);
        assert_eq!(call_read_hook(Some(read), &stored), Some(selection));
    }
}
