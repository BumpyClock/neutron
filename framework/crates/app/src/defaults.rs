//! Builder sugar that wires the service plugins together.
//!
//! The service modules (settings, theme, commands, windows) are deliberately
//! decoupled: each exposes a narrow seam instead of importing its peers. This
//! module is the one place those seams are tied — theme persistence to the
//! platform-owned [`ShellPreferences`] store, and the theme menu section into
//! the command registry.

use gpui::{App, MenuItem};
use gpui_component::ThemeModePreference;

use crate::commands::{
    AppCommandsExt, MenuPlan, MenusPlugin, StandardMenus, THEME_SECTION, menus_invalidate,
};
use crate::error::{AppShellError, MenuConfiguration};
use crate::plugin::{AppPlugin, ShellSeed};
use crate::settings::{
    AppSettings, SettingsPlugin, StoreKey, ThemeMode, shell_preferences, update_shell_preferences,
};
use crate::shell::AppShellBuilder;
use crate::theme::{ThemeMenuGroup, ThemePlugin, ThemeSelection, ThemeSource, theme_menu_items};

impl AppShellBuilder {
    /// Install persistence for shell-owned preferences such as theme selection
    /// and locale. Repeated calls are idempotent.
    pub fn shell_preferences(self) -> Self {
        self.ensure_shell_preferences()
    }

    /// Install a typed, persisted settings store (see [`crate::settings`]).
    pub fn settings<T: AppSettings>(self, key: StoreKey) -> Self {
        self.plugin(SettingsPlugin::<T>::new(key))
    }

    /// Install the theme service with selection persisted in the platform-owned
    /// [`ShellPreferences`](crate::settings::ShellPreferences) store, and its
    /// Appearance/Theme items projected into the reserved menu section.
    pub fn theme(self, source: ThemeSource) -> Self {
        self.shell_preferences()
            .plugin(
                ThemePlugin::new(source)
                    .with_preferences(read_theme_preference, write_theme_preference),
            )
            .plugin(ThemeMenuBridge)
    }

    /// Install native menus built from the command registry.
    pub fn menus(self, plan: MenuPlan) -> Self {
        self.register_menu_configuration(MenuConfiguration::MenuPlan)
            .plugin(MenusPlugin::new(plan))
    }

    /// Install platform-conventional desktop menus and shortcuts.
    pub fn standard_menus(self, menus: StandardMenus) -> Self {
        self.register_menu_configuration(MenuConfiguration::StandardMenus)
            .plugin(MenusPlugin::standard_menus(menus))
    }
}

fn read_theme_preference(cx: &App) -> Option<ThemeSelection> {
    let prefs = shell_preferences(cx);
    Some(ThemeSelection {
        mode: match prefs.theme_mode {
            ThemeMode::System => ThemeModePreference::System,
            ThemeMode::Light => ThemeModePreference::Light,
            ThemeMode::Dark => ThemeModePreference::Dark,
        },
        name: prefs.theme_name,
    })
}

fn write_theme_preference(cx: &mut App, selection: &ThemeSelection) {
    let mode = match selection.mode {
        ThemeModePreference::System => ThemeMode::System,
        ThemeModePreference::Light => ThemeMode::Light,
        ThemeModePreference::Dark => ThemeMode::Dark,
    };
    let name = selection.name.clone();
    if let Err(e) = update_shell_preferences(cx, |prefs| {
        prefs.theme_mode = mode;
        prefs.theme_name = name;
    }) {
        crate::handles::report_error(
            cx,
            crate::error::RuntimeError::service(anyhow::Error::new(e)),
        );
    }
}

/// Glue plugin: projects [`theme_menu_items`] into the command registry's
/// reserved theme section and invalidates menus on registry hot-reloads.
struct ThemeMenuBridge;

impl crate::plugin::sealed::Sealed for ThemeMenuBridge {}

impl AppPlugin for ThemeMenuBridge {
    fn init(&mut self, cx: &mut App, _shell: &ShellSeed) -> Result<(), AppShellError> {
        cx.register_menu_section(THEME_SECTION, theme_section_items);
        crate::theme::on_theme_registry_changed(cx, |cx| menus_invalidate(cx)).detach();
        Ok(())
    }
}

fn theme_section_items(cx: &App) -> Vec<MenuItem> {
    let mut items = Vec::new();
    let mut last_group: Option<ThemeMenuGroup> = None;
    for item in theme_menu_items(cx) {
        if last_group.is_some_and(|g| g != item.group) {
            items.push(MenuItem::Separator);
        }
        last_group = Some(item.group);
        items.push(MenuItem::Action {
            name: item.label.into(),
            action: item.action,
            os_action: None,
            checked: item.checked,
            disabled: false,
        });
    }
    items
}
