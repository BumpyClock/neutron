//! Standard desktop command vocabulary and menu configuration.

use gpui::{App, OsAction, actions};
use gpui_component::input;

use super::{
    ABOUT_COMMAND_ID, APP_MENU, AppCommandsExt, Command, CommandError, CommandId, CommandScope,
    EDIT_MENU, FILE_MENU, HELP_MENU, MenuPlan, OPEN_SETTINGS_COMMAND_ID, QUIT_COMMAND_ID,
    THEME_SECTION, VIEW_MENU, WINDOW_MENU,
};
use crate::handles::AppShellExt;

actions!(
    app,
    [
        /// Quit the application through the shell's single shutdown path.
        Quit,
        /// Open the app-provided Settings/Preferences surface.
        OpenSettings,
        /// Show the app-provided About surface.
        About,
        /// Close the focused window.
        CloseWindow,
    ]
);

#[cfg(target_os = "macos")]
actions!(
    app,
    [
        /// Hide the application (macOS).
        HideApp,
        /// Hide other applications (macOS).
        HideOthers,
        /// Show all applications (macOS).
        ShowAll,
        /// Minimize the focused window (macOS).
        Minimize,
        /// Zoom the focused window (macOS).
        Zoom,
    ]
);

type StandardCallback = Box<dyn Fn(&mut App) -> anyhow::Result<()> + 'static>;

/// Conventional cross-platform application menus.
///
/// Settings and About are present only when their callbacks are configured, so
/// the command registry never exposes inert items or shortcuts.
pub struct StandardMenus {
    settings: Option<StandardCallback>,
    about: Option<StandardCallback>,
    theme_menu: bool,
    custom_menus: Vec<&'static str>,
}

impl StandardMenus {
    /// Create standard Quit, Edit, and Window menus without optional app-owned
    /// Settings, About, or Appearance surfaces.
    pub fn new() -> Self {
        Self {
            settings: None,
            about: None,
            theme_menu: false,
            custom_menus: Vec::new(),
        }
    }

    /// Add Settings/Preferences and its conventional shortcut.
    #[must_use]
    pub fn on_settings(
        mut self,
        callback: impl Fn(&mut App) -> anyhow::Result<()> + 'static,
    ) -> Self {
        self.settings = Some(Box::new(callback));
        self
    }

    /// Add About to the platform-conventional menu.
    #[must_use]
    pub fn on_about(mut self, callback: impl Fn(&mut App) -> anyhow::Result<()> + 'static) -> Self {
        self.about = Some(Box::new(callback));
        self
    }

    /// Add the reserved Appearance/theme section.
    #[must_use]
    pub fn with_theme_menu(mut self) -> Self {
        self.theme_menu = true;
        self
    }

    /// Insert a custom top-level menu before Window/Help.
    #[must_use]
    pub fn with_menu(mut self, key: &'static str) -> Self {
        if !self.custom_menus.contains(&key) {
            self.custom_menus.push(key);
        }
        self
    }

    pub(super) fn install(self, cx: &mut App, app_name: &str) -> Result<MenuPlan, CommandError> {
        let platform = DesktopPlatform::current();
        let plan = standard_menu_plan(platform, self.theme_menu, &self.custom_menus);
        install_base(cx, app_name, platform, true)?;

        if let Some(callback) = self.settings {
            cx.register_command(
                Command::new(
                    OPEN_SETTINGS_COMMAND_ID,
                    settings_label(platform),
                    CommandScope::App,
                    OpenSettings,
                )
                .with_binding(settings_binding(platform))
                .placed(app_menu_key(platform), settings_group(platform), 0),
            )?;
            cx.on_action(move |_: &OpenSettings, cx: &mut App| {
                if let Err(error) = callback(cx) {
                    report_callback_error(cx, OPEN_SETTINGS_COMMAND_ID, error);
                }
            });
        }

        if let Some(callback) = self.about {
            cx.register_command(
                Command::new(
                    ABOUT_COMMAND_ID,
                    format!("About {app_name}"),
                    CommandScope::App,
                    About,
                )
                .placed(about_menu_key(platform), 0, 0),
            )?;
            cx.on_action(move |_: &About, cx: &mut App| {
                if let Err(error) = callback(cx) {
                    report_callback_error(cx, ABOUT_COMMAND_ID, error);
                }
            });
        }

        Ok(plan)
    }
}

impl Default for StandardMenus {
    fn default() -> Self {
        Self::new()
    }
}

/// Install the legacy raw-plan standard vocabulary. About remains absent until
/// an app explicitly registers it, avoiding the previous inert menu item.
pub(super) fn install_raw(cx: &mut App, app_name: &str) -> Result<(), CommandError> {
    install_base(cx, app_name, DesktopPlatform::current(), false)
}

fn install_base(
    cx: &mut App,
    app_name: &str,
    platform: DesktopPlatform,
    platform_plan: bool,
) -> Result<(), CommandError> {
    register_app_block(cx, app_name, platform, platform_plan)?;
    register_edit_block(cx)?;
    register_window_block(cx, platform)?;
    register_handlers(cx);
    Ok(())
}

fn register_app_block(
    cx: &mut App,
    app_name: &str,
    platform: DesktopPlatform,
    platform_plan: bool,
) -> Result<(), CommandError> {
    #[cfg(target_os = "macos")]
    if platform == DesktopPlatform::MacOs {
        cx.register_command(
            Command::new(
                CommandId("app.hide"),
                format!("Hide {app_name}"),
                CommandScope::App,
                HideApp,
            )
            .with_binding("cmd-h")
            .placed(APP_MENU, 4, 0),
        )?;
        cx.register_command(
            Command::new(
                CommandId("app.hide_others"),
                "Hide Others",
                CommandScope::App,
                HideOthers,
            )
            .placed(APP_MENU, 4, 1),
        )?;
        cx.register_command(
            Command::new(
                CommandId("app.show_all"),
                "Show All",
                CommandScope::App,
                ShowAll,
            )
            .placed(APP_MENU, 4, 2),
        )?;
    }

    let menu = if platform_plan {
        app_menu_key(platform)
    } else {
        APP_MENU
    };
    let label = if platform == DesktopPlatform::MacOs {
        format!("Quit {app_name}")
    } else {
        "Quit".to_string()
    };
    cx.register_command(
        Command::new(QUIT_COMMAND_ID, label, CommandScope::App, Quit)
            .with_binding(quit_binding(platform))
            .placed(menu, 9, 0),
    )?;
    Ok(())
}

fn register_edit_block(cx: &mut App) -> Result<(), CommandError> {
    cx.register_command(edit(
        CommandId("edit.undo"),
        "Undo",
        input::Undo,
        OsAction::Undo,
        0,
        0,
    ))?;
    cx.register_command(edit(
        CommandId("edit.redo"),
        "Redo",
        input::Redo,
        OsAction::Redo,
        0,
        1,
    ))?;
    cx.register_command(edit(
        CommandId("edit.cut"),
        "Cut",
        input::Cut,
        OsAction::Cut,
        1,
        0,
    ))?;
    cx.register_command(edit(
        CommandId("edit.copy"),
        "Copy",
        input::Copy,
        OsAction::Copy,
        1,
        1,
    ))?;
    cx.register_command(edit(
        CommandId("edit.paste"),
        "Paste",
        input::Paste,
        OsAction::Paste,
        1,
        2,
    ))?;
    cx.register_command(edit(
        CommandId("edit.select_all"),
        "Select All",
        input::SelectAll,
        OsAction::SelectAll,
        2,
        0,
    ))?;
    Ok(())
}

fn edit(
    id: CommandId,
    label: &'static str,
    action: impl gpui::Action,
    os_action: OsAction,
    group: u16,
    order: u16,
) -> Command {
    Command::new(id, label, CommandScope::Window, action)
        .with_os_action(os_action)
        .placed(EDIT_MENU, group, order)
}

fn register_window_block(cx: &mut App, platform: DesktopPlatform) -> Result<(), CommandError> {
    #[cfg(target_os = "macos")]
    if platform == DesktopPlatform::MacOs {
        cx.register_command(
            Command::new(
                CommandId("window.minimize"),
                "Minimize",
                CommandScope::Window,
                Minimize,
            )
            .with_binding("cmd-m")
            .placed(WINDOW_MENU, 0, 0),
        )?;
        cx.register_command(
            Command::new(CommandId("window.zoom"), "Zoom", CommandScope::Window, Zoom).placed(
                WINDOW_MENU,
                0,
                1,
            ),
        )?;
    }

    cx.register_command(
        Command::new(
            CommandId("window.close"),
            "Close Window",
            CommandScope::Window,
            CloseWindow,
        )
        .with_binding(close_binding(platform))
        .placed(WINDOW_MENU, 1, 0),
    )?;
    Ok(())
}

fn register_handlers(cx: &mut App) {
    cx.on_action(|_: &Quit, cx: &mut App| cx.request_quit());

    #[cfg(target_os = "macos")]
    {
        cx.on_action(|_: &HideApp, cx: &mut App| cx.hide());
        cx.on_action(|_: &HideOthers, cx: &mut App| cx.hide_other_apps());
        cx.on_action(|_: &ShowAll, cx: &mut App| cx.unhide_other_apps());
    }
}

fn report_callback_error(cx: &mut App, command: CommandId, error: anyhow::Error) {
    crate::handles::report_error(
        cx,
        crate::error::RuntimeError::command(command.as_str(), error),
    );
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub(super) enum DesktopPlatform {
    MacOs,
    Windows,
    Linux,
}

impl DesktopPlatform {
    fn current() -> Self {
        #[cfg(target_os = "macos")]
        return Self::MacOs;
        #[cfg(target_os = "windows")]
        return Self::Windows;
        #[cfg(target_os = "linux")]
        return Self::Linux;
    }
}

fn standard_menu_plan(
    platform: DesktopPlatform,
    theme_menu: bool,
    custom_menus: &[&'static str],
) -> MenuPlan {
    let mut plan = MenuPlan::from_keys(standard_menu_keys(platform, theme_menu));
    let insertion_key = WINDOW_MENU;
    for key in custom_menus {
        plan.insert_before(insertion_key, key);
    }
    if theme_menu {
        let (menu, group) = match platform {
            DesktopPlatform::MacOs => (APP_MENU, 2),
            DesktopPlatform::Windows | DesktopPlatform::Linux => (VIEW_MENU, 0),
        };
        plan.reserve_section_at(menu, group, 0, THEME_SECTION);
    }
    if platform == DesktopPlatform::MacOs {
        plan.reserve_services_at(APP_MENU, 3, 0);
    }
    plan
}

fn standard_menu_keys(platform: DesktopPlatform, theme_menu: bool) -> Vec<&'static str> {
    match platform {
        DesktopPlatform::MacOs => vec![APP_MENU, EDIT_MENU, WINDOW_MENU],
        DesktopPlatform::Windows | DesktopPlatform::Linux => {
            let mut keys = vec![FILE_MENU, EDIT_MENU];
            if theme_menu {
                keys.push(VIEW_MENU);
            }
            keys.extend([WINDOW_MENU, HELP_MENU]);
            keys
        }
    }
}

fn app_menu_key(platform: DesktopPlatform) -> &'static str {
    match platform {
        DesktopPlatform::MacOs => APP_MENU,
        DesktopPlatform::Windows | DesktopPlatform::Linux => FILE_MENU,
    }
}

fn about_menu_key(platform: DesktopPlatform) -> &'static str {
    match platform {
        DesktopPlatform::MacOs => APP_MENU,
        DesktopPlatform::Windows | DesktopPlatform::Linux => HELP_MENU,
    }
}

fn settings_group(platform: DesktopPlatform) -> u16 {
    match platform {
        DesktopPlatform::MacOs => 1,
        DesktopPlatform::Windows | DesktopPlatform::Linux => 0,
    }
}

fn settings_label(platform: DesktopPlatform) -> &'static str {
    match platform {
        DesktopPlatform::MacOs => "Settings…",
        DesktopPlatform::Windows => "Settings",
        DesktopPlatform::Linux => "Preferences",
    }
}

fn settings_binding(platform: DesktopPlatform) -> &'static str {
    match platform {
        DesktopPlatform::MacOs => "cmd-,",
        DesktopPlatform::Windows | DesktopPlatform::Linux => "ctrl-,",
    }
}

fn quit_binding(platform: DesktopPlatform) -> &'static str {
    match platform {
        DesktopPlatform::MacOs => "cmd-q",
        DesktopPlatform::Windows | DesktopPlatform::Linux => "ctrl-q",
    }
}

fn close_binding(platform: DesktopPlatform) -> &'static str {
    match platform {
        DesktopPlatform::MacOs => "cmd-w",
        DesktopPlatform::Windows | DesktopPlatform::Linux => "ctrl-w",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_plan_uses_native_desktop_conventions() {
        assert_eq!(
            standard_menu_keys(DesktopPlatform::MacOs, false),
            vec![APP_MENU, EDIT_MENU, WINDOW_MENU]
        );
        assert_eq!(
            standard_menu_keys(DesktopPlatform::Windows, true),
            vec!["File", EDIT_MENU, "View", WINDOW_MENU, "Help"]
        );
        assert_eq!(
            standard_menu_keys(DesktopPlatform::Linux, false),
            vec!["File", EDIT_MENU, WINDOW_MENU, "Help"]
        );
    }

    #[test]
    fn custom_menus_precede_window_and_help() {
        let plan = standard_menu_plan(DesktopPlatform::Linux, false, &["Tools", "Debug"]);
        assert_eq!(
            plan.outline(&[])
                .iter()
                .map(|menu| menu.key)
                .collect::<Vec<_>>(),
            vec![
                FILE_MENU,
                EDIT_MENU,
                "Tools",
                "Debug",
                WINDOW_MENU,
                HELP_MENU
            ]
        );
    }

    #[test]
    fn settings_labels_and_bindings_match_platform() {
        assert_eq!(settings_label(DesktopPlatform::MacOs), "Settings…");
        assert_eq!(settings_label(DesktopPlatform::Windows), "Settings");
        assert_eq!(settings_label(DesktopPlatform::Linux), "Preferences");
        assert_eq!(settings_binding(DesktopPlatform::MacOs), "cmd-,");
        assert_eq!(settings_binding(DesktopPlatform::Windows), "ctrl-,");
        assert_eq!(settings_binding(DesktopPlatform::Linux), "ctrl-,");
    }

    #[test]
    fn every_standard_binding_parses() {
        for platform in [
            DesktopPlatform::MacOs,
            DesktopPlatform::Windows,
            DesktopPlatform::Linux,
        ] {
            for command in [
                Command::new(
                    OPEN_SETTINGS_COMMAND_ID,
                    "Settings",
                    CommandScope::App,
                    OpenSettings,
                )
                .with_binding(settings_binding(platform)),
                Command::new(QUIT_COMMAND_ID, "Quit", CommandScope::App, Quit)
                    .with_binding(quit_binding(platform)),
                Command::new(
                    CommandId("window.close"),
                    "Close Window",
                    CommandScope::Window,
                    CloseWindow,
                )
                .with_binding(close_binding(platform)),
            ] {
                super::super::plugin::validate_command_binding(&command).unwrap();
            }
        }
    }

    #[test]
    fn macos_system_sections_use_conventional_groups() {
        let plan = standard_menu_plan(DesktopPlatform::MacOs, true, &[]);
        assert_eq!(
            plan.outline(&[])[0].nodes,
            vec![
                super::super::MenuNode::Section(THEME_SECTION),
                super::super::MenuNode::Separator,
                super::super::MenuNode::Services,
            ]
        );
    }

    #[gpui::test]
    fn optional_callbacks_control_commands(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            StandardMenus::new().install(cx, "Test App").unwrap();
            let registry = cx.global::<super::super::CommandRegistry>();
            assert!(registry.get(OPEN_SETTINGS_COMMAND_ID).is_none());
            assert!(registry.get(ABOUT_COMMAND_ID).is_none());
        });
    }

    #[gpui::test]
    fn configured_callbacks_register_stable_commands(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            StandardMenus::new()
                .on_settings(|_| Ok(()))
                .on_about(|_| Ok(()))
                .install(cx, "Test App")
                .unwrap();
            let registry = cx.global::<super::super::CommandRegistry>();
            assert_eq!(
                registry
                    .get(OPEN_SETTINGS_COMMAND_ID)
                    .unwrap()
                    .default_binding(),
                Some(settings_binding(DesktopPlatform::current()))
            );
            assert_eq!(
                registry.get(ABOUT_COMMAND_ID).unwrap().label().as_ref(),
                "About Test App"
            );
        });
    }
}
