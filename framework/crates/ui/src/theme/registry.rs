use crate::{
    Theme, ThemeColor, ThemeConfig, ThemeMode, ThemeModePreference, ThemeSet,
    highlighter::HighlightTheme,
};
use anyhow::Result;
use gpui::{App, Global, SharedString};
use notify::Watcher as _;
use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    rc::Rc,
    sync::{Arc, LazyLock},
};

#[derive(Debug, Clone)]
pub struct ThemeSetEntry {
    pub name: SharedString,
    pub author: Option<SharedString>,
    pub url: Option<SharedString>,
    pub light: Option<Rc<ThemeConfig>>,
    pub dark: Option<Rc<ThemeConfig>>,
}

const DEFAULT_THEME: &str = include_str!("./default-theme.json");
pub(crate) static DEFAULT_THEME_COLORS: LazyLock<
    HashMap<ThemeMode, (Arc<ThemeColor>, Arc<HighlightTheme>)>,
> = LazyLock::new(|| {
    let mut colors = HashMap::new();

    let themes: Vec<ThemeConfig> = serde_json::from_str::<ThemeSet>(DEFAULT_THEME)
        .expect("Failed to parse themes/default.json")
        .themes;

    for theme in themes {
        let mut theme_color = ThemeColor::default();
        theme_color.apply_config(&theme, &ThemeColor::default());

        let highlight_theme = HighlightTheme {
            name: theme.name.to_string(),
            appearance: theme.mode,
            style: theme.highlight.unwrap_or_default(),
        };

        colors.insert(
            theme.mode,
            (Arc::new(theme_color), Arc::new(highlight_theme)),
        );
    }

    colors
});

pub(super) fn init(cx: &mut App) {
    cx.set_global(ThemeRegistry::default());
    ThemeRegistry::global_mut(cx).init_default_themes();

    // Observe changes to the theme registry to apply changes to the active theme
    cx.observe_global::<ThemeRegistry>(|cx| {
        let mode = Theme::global(cx).mode;
        let light_theme = Theme::global(cx).light_theme.name.clone();
        let dark_theme = Theme::global(cx).dark_theme.name.clone();

        if let Some(theme) = ThemeRegistry::global(cx)
            .themes()
            .get(&light_theme)
            .cloned()
        {
            Theme::global_mut(cx).light_theme = theme;
        }
        if let Some(theme) = ThemeRegistry::global(cx).themes().get(&dark_theme).cloned() {
            Theme::global_mut(cx).dark_theme = theme;
        }

        let theme_name = if mode.is_dark() {
            dark_theme
        } else {
            light_theme
        };

        tracing::info!("Reload active theme: {:?}...", theme_name);
        Theme::change(mode, None, cx);
        cx.refresh_windows();
    })
    .detach();
}

#[derive(Default, Debug)]
pub struct ThemeRegistry {
    themes_dir: PathBuf,
    default_themes: HashMap<ThemeMode, Rc<ThemeConfig>>,
    themes: HashMap<SharedString, Rc<ThemeConfig>>,
    theme_sets: HashMap<SharedString, ThemeSetEntry>,
    has_custom_themes: bool,
    /// Theme sets loaded from disk on the last `reload`, cached so in-memory
    /// registrations can be merged back in without re-reading the filesystem.
    disk_theme_sets: Vec<ThemeSet>,
    /// Theme sets registered in-memory via [`ThemeRegistry::register_theme_set`].
    registered_theme_sets: HashMap<SharedString, ThemeSet>,
}

impl Global for ThemeRegistry {}

impl ThemeRegistry {
    pub fn global(cx: &App) -> &Self {
        cx.global::<Self>()
    }

    pub fn global_mut(cx: &mut App) -> &mut Self {
        cx.global_mut::<Self>()
    }

    /// Watch themes directory.
    ///
    /// And reload themes to trigger the `on_load` callback.
    pub fn watch_dir<F>(themes_dir: PathBuf, cx: &mut App, on_load: F) -> Result<()>
    where
        F: Fn(&mut App) + 'static,
    {
        Self::global_mut(cx).themes_dir = themes_dir.clone();

        // Load theme in the background.
        cx.spawn(async move |cx| {
            _ = cx.update(|cx| {
                if let Err(err) = Self::_watch_themes_dir(themes_dir, cx) {
                    tracing::error!("Failed to watch themes directory: {}", err);
                }

                Self::reload_themes(cx);
                on_load(cx);
            });
        })
        .detach();

        Ok(())
    }

    /// Returns a reference to the map of themes (including default themes).
    pub fn themes(&self) -> &HashMap<SharedString, Rc<ThemeConfig>> {
        &self.themes
    }

    /// Returns a sorted list of themes.
    pub fn sorted_themes(&self) -> Vec<&Rc<ThemeConfig>> {
        let mut themes = self.themes.values().collect::<Vec<_>>();
        // sort by is_default true first, then light first dark later, then by name case-insensitive
        themes.sort_by(|a, b| {
            b.is_default
                .cmp(&a.is_default)
                .then(a.mode.cmp(&b.mode))
                .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        themes
    }

    /// Returns a reference to the map of default themes.
    pub fn default_themes(&self) -> &HashMap<ThemeMode, Rc<ThemeConfig>> {
        &self.default_themes
    }

    pub fn default_light_theme(&self) -> &Rc<ThemeConfig> {
        &self.default_themes[&ThemeMode::Light]
    }

    pub fn default_dark_theme(&self) -> &Rc<ThemeConfig> {
        &self.default_themes[&ThemeMode::Dark]
    }

    pub fn theme_sets(&self) -> &HashMap<SharedString, ThemeSetEntry> {
        &self.theme_sets
    }

    pub fn sorted_theme_sets(&self) -> Vec<&ThemeSetEntry> {
        let mut sets = self.theme_sets.values().collect::<Vec<_>>();
        sets.sort_by_key(|a| a.name.to_lowercase());
        sets
    }

    pub fn resolve_theme(
        set: &ThemeSetEntry,
        preference: ThemeModePreference,
        system_appearance: ThemeMode,
    ) -> Option<&Rc<ThemeConfig>> {
        match preference {
            ThemeModePreference::Light => set.light.as_ref().or(set.dark.as_ref()),
            ThemeModePreference::Dark => set.dark.as_ref().or(set.light.as_ref()),
            ThemeModePreference::System => {
                if system_appearance.is_dark() {
                    set.dark.as_ref().or(set.light.as_ref())
                } else {
                    set.light.as_ref().or(set.dark.as_ref())
                }
            }
        }
    }

    fn create_default_set(&self) -> ThemeSetEntry {
        ThemeSetEntry {
            name: "Default".into(),
            author: None,
            url: None,
            light: self.default_themes.get(&ThemeMode::Light).map(Rc::clone),
            dark: self.default_themes.get(&ThemeMode::Dark).map(Rc::clone),
        }
    }

    fn init_default_themes(&mut self) {
        let default_themes: Vec<ThemeConfig> = serde_json::from_str::<ThemeSet>(DEFAULT_THEME)
            .expect("failed to parse default theme.")
            .themes;
        for theme in default_themes.into_iter() {
            if theme.mode.is_dark() {
                self.default_themes.insert(ThemeMode::Dark, Rc::new(theme));
            } else {
                self.default_themes.insert(ThemeMode::Light, Rc::new(theme));
            }
        }
        self.themes = self
            .default_themes
            .values()
            .map(|theme| {
                let name = theme.name.clone();
                (name, Rc::clone(theme))
            })
            .collect();

        self.theme_sets
            .insert("Default".into(), self.create_default_set());
    }

    fn _watch_themes_dir(themes_dir: PathBuf, cx: &mut App) -> anyhow::Result<()> {
        if !themes_dir.exists() {
            fs::create_dir_all(&themes_dir)?;
        }

        let (tx, rx) = smol::channel::bounded(100);
        let mut watcher =
            notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                if let Ok(event) = &res {
                    match event.kind {
                        notify::EventKind::Create(_)
                        | notify::EventKind::Modify(_)
                        | notify::EventKind::Remove(_) => {
                            if let Err(err) = tx.send_blocking(res) {
                                tracing::error!("Failed to send theme event: {:?}", err);
                            }
                        }
                        _ => {}
                    }
                }
            })?;

        cx.spawn(async move |cx| {
            if let Err(err) = watcher.watch(&themes_dir, notify::RecursiveMode::Recursive) {
                tracing::error!("Failed to watch themes directory: {:?}", err);
            }

            while (rx.recv().await).is_ok() {
                tracing::info!("Reloading themes...");
                _ = cx.update(Self::reload_themes);
            }
        })
        .detach();

        Ok(())
    }

    fn reload_themes(cx: &mut App) {
        let registry = Self::global_mut(cx);
        match registry.reload() {
            Ok(_) => {
                tracing::info!("Themes reloaded successfully.");
            }
            Err(e) => tracing::error!("Failed to reload themes: {:?}", e),
        }
    }

    /// Registers a theme set in-memory, without touching the `themes_dir` on disk.
    ///
    /// This lets callers (e.g. an app bundling its own themes, or a future wasm
    /// build with no filesystem) supply themes the same way a theme file on disk
    /// would, without needing to write that file first.
    ///
    /// Registering a set overwrites any previous set with the same `name`, whether
    /// it was registered in-memory earlier or is currently loaded from disk. Note
    /// that precedence is by *set* name, not by individual theme name: default
    /// themes are still protected (a registered theme can't replace a built-in
    /// theme of the same name), and if two different sets both contain a theme
    /// with the same name, whichever set is merged first keeps that theme.
    ///
    /// If `watch_dir` later reloads a disk file with the same set name, the
    /// on-disk version wins on the next reload - in-memory registrations behave
    /// like a previously loaded file that disk can supersede.
    pub fn register_theme_set(&mut self, set: ThemeSet) {
        self.registered_theme_sets.insert(set.name.clone(), set);
        self.rebuild_theme_sets(self.disk_theme_sets.clone());
    }

    /// Reload themes from the `themes_dir`.
    fn reload(&mut self) -> Result<()> {
        let mut loaded_sets: Vec<ThemeSet> = vec![];

        if self.themes_dir.exists() {
            for entry in fs::read_dir(&self.themes_dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("json") {
                    let file_content = fs::read_to_string(path.clone())?;

                    match serde_json::from_str::<ThemeSet>(&file_content) {
                        Ok(theme_set) => loaded_sets.push(theme_set),
                        Err(e) => {
                            tracing::error!(
                                "ignored invalid theme file: {}, {}",
                                path.display(),
                                e
                            );
                        }
                    }
                }
            }
        }

        self.disk_theme_sets = loaded_sets.clone();
        self.rebuild_theme_sets(loaded_sets);

        Ok(())
    }

    /// Rebuilds `themes` and `theme_sets` from the default themes plus `disk_sets`,
    /// filling in any `registered_theme_sets` whose name isn't already covered by
    /// `disk_sets` (disk sets take precedence over an in-memory registration with
    /// the same set name).
    fn rebuild_theme_sets(&mut self, disk_sets: Vec<ThemeSet>) {
        let mut all_sets = disk_sets;
        for (name, set) in &self.registered_theme_sets {
            if !all_sets.iter().any(|s| &s.name == name) {
                all_sets.push(set.clone());
            }
        }

        self.themes.clear();
        for theme in self.default_themes.values() {
            self.themes
                .insert(theme.name.clone(), Rc::new((**theme).clone()));
        }

        for set in &all_sets {
            for theme in &set.themes {
                if self.themes.contains_key(&theme.name) {
                    continue;
                }

                if theme.is_default {
                    self.default_themes
                        .insert(theme.mode, Rc::new(theme.clone()));
                }

                self.has_custom_themes = true;
                self.themes
                    .insert(theme.name.clone(), Rc::new(theme.clone()));
            }
        }

        // Rebuild theme_sets
        self.theme_sets.clear();
        // Re-add default set
        self.theme_sets
            .insert("Default".into(), self.create_default_set());

        for set in &all_sets {
            let entry = self
                .theme_sets
                .entry(set.name.clone())
                .or_insert_with(|| ThemeSetEntry {
                    name: set.name.clone(),
                    author: set.author.clone(),
                    url: set.url.clone(),
                    light: None,
                    dark: None,
                });
            for config in &set.themes {
                let rc = Rc::new(config.clone());
                if config.mode.is_dark() {
                    entry.dark = Some(rc);
                } else {
                    entry.light = Some(rc);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_config(name: &str, mode: ThemeMode) -> ThemeConfig {
        ThemeConfig {
            name: name.into(),
            mode,
            ..Default::default()
        }
    }

    fn make_registry() -> ThemeRegistry {
        let mut registry = ThemeRegistry::default();
        registry.init_default_themes();
        registry
    }

    #[test]
    fn register_theme_set_is_visible_in_sorted_theme_sets() {
        let mut registry = make_registry();
        registry.register_theme_set(ThemeSet {
            name: "Acme".into(),
            author: Some("Acme Corp".into()),
            url: None,
            themes: vec![
                make_config("Acme Light", ThemeMode::Light),
                make_config("Acme Dark", ThemeMode::Dark),
            ],
        });

        let set = registry
            .sorted_theme_sets()
            .into_iter()
            .find(|set| set.name == "Acme")
            .expect("registered set should be visible in sorted_theme_sets");
        assert_eq!(set.light.as_ref().unwrap().name, "Acme Light");
        assert_eq!(set.dark.as_ref().unwrap().name, "Acme Dark");
        assert_eq!(set.author.as_deref(), Some("Acme Corp"));
    }

    #[test]
    fn register_theme_set_overwrites_by_name() {
        let mut registry = make_registry();
        registry.register_theme_set(ThemeSet {
            name: "Acme".into(),
            author: None,
            url: None,
            themes: vec![make_config("Acme Light", ThemeMode::Light)],
        });
        registry.register_theme_set(ThemeSet {
            name: "Acme".into(),
            author: None,
            url: None,
            themes: vec![make_config("Acme Light v2", ThemeMode::Light)],
        });

        let set = registry
            .theme_sets()
            .get(&SharedString::from("Acme"))
            .expect("set should still be registered under the same name");
        assert_eq!(set.light.as_ref().unwrap().name, "Acme Light v2");
        assert!(set.dark.is_none());
        assert!(
            !registry
                .themes()
                .contains_key(&SharedString::from("Acme Light")),
            "theme from the overwritten registration should no longer be reachable"
        );
    }

    #[test]
    fn register_theme_set_does_not_override_default_themes() {
        let mut registry = make_registry();
        let default_light_name = registry.default_light_theme().name.clone();
        let default_dark_name = registry.default_dark_theme().name.clone();

        let mut impersonator = make_config(default_light_name.as_ref(), ThemeMode::Light);
        impersonator.is_default = true;
        impersonator.radius = Some(99);
        registry.register_theme_set(ThemeSet {
            name: "Impersonator".into(),
            author: None,
            url: None,
            themes: vec![impersonator],
        });

        assert_eq!(registry.default_light_theme().name, default_light_name);
        assert_eq!(registry.default_dark_theme().name, default_dark_name);
        assert_eq!(
            registry.themes().get(&default_light_name).unwrap().radius,
            None,
            "the built-in default theme must not be overwritten by a same-named registration"
        );
    }

    #[test]
    fn disk_reload_overrides_registered_set_with_same_name() {
        let mut registry = make_registry();
        registry.register_theme_set(ThemeSet {
            name: "Acme".into(),
            author: None,
            url: None,
            themes: vec![make_config("Acme Light", ThemeMode::Light)],
        });

        let dir = std::env::temp_dir().join(format!(
            "gpui-component-theme-registry-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        fs::create_dir_all(&dir).expect("create temp themes dir");
        let disk_set = ThemeSet {
            name: "Acme".into(),
            author: None,
            url: None,
            themes: vec![make_config("Acme Light On Disk", ThemeMode::Light)],
        };
        fs::write(
            dir.join("acme.json"),
            serde_json::to_string(&disk_set).expect("serialize theme set"),
        )
        .expect("write theme file");

        registry.themes_dir = dir.clone();
        let result = registry.reload();
        fs::remove_dir_all(&dir).ok();
        result.expect("reload should succeed");

        let set = registry
            .theme_sets()
            .get(&SharedString::from("Acme"))
            .expect("set should still be present after reload");
        assert_eq!(set.light.as_ref().unwrap().name, "Acme Light On Disk");
    }
}
