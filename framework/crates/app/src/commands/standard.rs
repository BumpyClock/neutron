//! Standard desktop command vocabulary and menu configuration.

use gpui::{App, OsAction, actions};
use neutron_components::input;

use super::declaration::StandardFeatures;
use super::keys::MenuKey;
use super::{CommandId, CommandScope, RuntimeCommand};
use crate::handles::AppShellExt;

pub use super::{ABOUT_COMMAND_ID, OPEN_SETTINGS_COMMAND_ID, QUIT_COMMAND_ID};

/// Stable ids for the standard commands that have no public constant yet.
///
/// The typed menu layout ([`super::menu_model`]) and the current registration
/// path both reference these, so a rename cannot desynchronize the two.
pub const HIDE_APP_COMMAND_ID: CommandId = CommandId("app.hide");
/// Stable id for the macOS Hide Others command.
pub const HIDE_OTHERS_COMMAND_ID: CommandId = CommandId("app.hide_others");
/// Stable id for the macOS Show All command.
pub const SHOW_ALL_COMMAND_ID: CommandId = CommandId("app.show_all");
/// Stable id for the macOS Minimize command.
pub const MINIMIZE_COMMAND_ID: CommandId = CommandId("window.minimize");
/// Stable id for the macOS Zoom command.
pub const ZOOM_COMMAND_ID: CommandId = CommandId("window.zoom");
/// Stable id for the Close Window command.
pub const CLOSE_WINDOW_COMMAND_ID: CommandId = CommandId("window.close");
/// Stable id for the standard Undo command.
pub const UNDO_COMMAND_ID: CommandId = CommandId("edit.undo");
/// Stable id for the standard Redo command.
pub const REDO_COMMAND_ID: CommandId = CommandId("edit.redo");
/// Stable id for the standard Cut command.
pub const CUT_COMMAND_ID: CommandId = CommandId("edit.cut");
/// Stable id for the standard Copy command.
pub const COPY_COMMAND_ID: CommandId = CommandId("edit.copy");
/// Stable id for the standard Paste command.
pub const PASTE_COMMAND_ID: CommandId = CommandId("edit.paste");
/// Stable id for the standard Select All command.
pub const SELECT_ALL_COMMAND_ID: CommandId = CommandId("edit.select_all");
/// Stable id for the standard Delete command.
pub const DELETE_COMMAND_ID: CommandId = CommandId("edit.delete");
/// Stable id for the standard Delete Previous Word command.
pub const DELETE_PREVIOUS_WORD_COMMAND_ID: CommandId = CommandId("edit.delete_previous_word");
/// Stable id for the standard Delete Next Word command.
pub const DELETE_NEXT_WORD_COMMAND_ID: CommandId = CommandId("edit.delete_next_word");
/// Stable id for the standard Find command.
pub const FIND_COMMAND_ID: CommandId = CommandId("edit.find");

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
    crate::handles::report_error(cx, crate::error::RuntimeError::command(command, error));
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DesktopPlatform {
    MacOs,
    Windows,
    Linux,
}

impl DesktopPlatform {
    /// Every desktop platform, in the order pure validation reports faults.
    ///
    /// Declaration validation iterates this list on *every* host, so a binding
    /// that is invalid on Windows fails on a macOS developer machine too.
    pub(crate) const ALL: [Self; 3] = [Self::MacOs, Self::Windows, Self::Linux];

    /// The current desktop platform.
    #[must_use]
    pub fn current() -> Self {
        #[cfg(target_os = "macos")]
        return Self::MacOs;
        #[cfg(target_os = "windows")]
        return Self::Windows;
        #[cfg(target_os = "linux")]
        return Self::Linux;
    }

    /// A stable name for diagnostics.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MacOs => "macOS",
            Self::Windows => "Windows",
            Self::Linux => "Linux",
        }
    }
}

/// The conventional top-level menus for `platform`, in bar order.
///
/// Shared with the typed menu layout so the two models cannot drift.
pub(super) fn standard_menu_keys(platform: DesktopPlatform, theme_menu: bool) -> Vec<MenuKey> {
    match platform {
        DesktopPlatform::MacOs => vec![MenuKey::APP, MenuKey::EDIT, MenuKey::WINDOW],
        DesktopPlatform::Windows | DesktopPlatform::Linux => {
            let mut keys = vec![MenuKey::FILE, MenuKey::EDIT];
            if theme_menu {
                keys.push(MenuKey::VIEW);
            }
            keys.extend([MenuKey::WINDOW, MenuKey::HELP]);
            keys
        }
    }
}

/// The menu that hosts the Appearance/Theme section on `platform`.
pub(super) fn theme_section_menu(platform: DesktopPlatform) -> MenuKey {
    match platform {
        DesktopPlatform::MacOs => MenuKey::APP,
        DesktopPlatform::Windows | DesktopPlatform::Linux => MenuKey::VIEW,
    }
}

pub(super) fn app_menu_key(platform: DesktopPlatform) -> &'static str {
    match platform {
        DesktopPlatform::MacOs => MenuKey::APP.as_str(),
        DesktopPlatform::Windows | DesktopPlatform::Linux => MenuKey::FILE.as_str(),
    }
}

pub(super) fn about_menu_key(platform: DesktopPlatform) -> &'static str {
    match platform {
        DesktopPlatform::MacOs => MenuKey::APP.as_str(),
        DesktopPlatform::Windows | DesktopPlatform::Linux => MenuKey::HELP.as_str(),
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

/// The command ids the framework itself installs for `platform`.
///
/// Pure and host-independent: the macOS-only ids are listed whenever
/// `platform` is macOS, even on a Linux build host, so typed validation of a
/// macOS layout gives the same answer everywhere.
///
/// Deliberately excludes About and Settings. Those are *optional* standard
/// references: the standard layout only projects them when the application
/// resolved the corresponding feature, so they arrive through
/// [`feature_command_ids`] instead. A layout that asks for one without the
/// feature is an `UnknownCommand` fault, which is the intended report.
pub(super) fn framework_command_ids(platform: DesktopPlatform) -> Vec<CommandId> {
    let mut ids = vec![QUIT_COMMAND_ID];
    if platform == DesktopPlatform::MacOs {
        ids.extend([
            HIDE_APP_COMMAND_ID,
            HIDE_OTHERS_COMMAND_ID,
            SHOW_ALL_COMMAND_ID,
        ]);
    }
    ids.extend([
        UNDO_COMMAND_ID,
        REDO_COMMAND_ID,
        CUT_COMMAND_ID,
        COPY_COMMAND_ID,
        PASTE_COMMAND_ID,
        DELETE_COMMAND_ID,
        DELETE_PREVIOUS_WORD_COMMAND_ID,
        DELETE_NEXT_WORD_COMMAND_ID,
        FIND_COMMAND_ID,
        SELECT_ALL_COMMAND_ID,
    ]);
    if platform == DesktopPlatform::MacOs {
        ids.extend([MINIMIZE_COMMAND_ID, ZOOM_COMMAND_ID]);
    }
    ids.push(CLOSE_WINDOW_COMMAND_ID);
    ids
}

/// Every command id the framework itself may install, across every desktop
/// platform and every optional standard feature.
///
/// [`Commands::replace_command`](super::Commands::replace_command) rejects a
/// framework-owned id no matter which platform is currently running, so this
/// unions [`framework_command_ids`] over [`DesktopPlatform::ALL`] with both
/// optional feature ids (Settings, About) instead of just the current host's
/// set — behavior stays the same on every build host.
pub(super) fn is_standard_command(id: CommandId) -> bool {
    id == OPEN_SETTINGS_COMMAND_ID
        || id == ABOUT_COMMAND_ID
        || DesktopPlatform::ALL
            .iter()
            .any(|&platform| framework_command_ids(platform).contains(&id))
}

/// Build the framework's standard commands for `platform`, without placements.
///
/// The typed model owns menu projection, so every placement comes from the
/// resolved outline and is applied by the installer. Returned as values (no
/// `&mut App`) so the whole install can be assembled before anything mutates.
///
/// The macOS-only commands are gated on the *build* host as well as `platform`,
/// because their actions only exist under `cfg(target_os = "macos")`.
pub(super) fn framework_commands(platform: DesktopPlatform) -> Vec<RuntimeCommand> {
    let mut commands = Vec::new();

    #[cfg(target_os = "macos")]
    if platform == DesktopPlatform::MacOs {
        commands.push(
            RuntimeCommand::new(HIDE_APP_COMMAND_ID, "Hide", CommandScope::App, HideApp)
                .with_derived_label(hide_app_label)
                .with_binding("cmd-h"),
        );
        commands.push(RuntimeCommand::new(
            HIDE_OTHERS_COMMAND_ID,
            "Hide Others",
            CommandScope::App,
            HideOthers,
        ));
        commands.push(RuntimeCommand::new(
            SHOW_ALL_COMMAND_ID,
            "Show All",
            CommandScope::App,
            ShowAll,
        ));
    }

    let quit = RuntimeCommand::new(QUIT_COMMAND_ID, "Quit", CommandScope::App, Quit)
        .with_binding(quit_binding(platform));
    commands.push(if platform == DesktopPlatform::MacOs {
        quit.with_derived_label(quit_label)
    } else {
        quit
    });

    commands.extend([
        edit_command(UNDO_COMMAND_ID, "Undo", input::Undo, OsAction::Undo),
        edit_command(REDO_COMMAND_ID, "Redo", input::Redo, OsAction::Redo),
        edit_command(CUT_COMMAND_ID, "Cut", input::Cut, OsAction::Cut),
        edit_command(COPY_COMMAND_ID, "Copy", input::Copy, OsAction::Copy),
        edit_command(PASTE_COMMAND_ID, "Paste", input::Paste, OsAction::Paste),
        window_command(DELETE_COMMAND_ID, "Delete", input::Delete),
        window_command(
            DELETE_PREVIOUS_WORD_COMMAND_ID,
            "Delete Previous Word",
            input::DeleteToPreviousWordStart,
        ),
        window_command(
            DELETE_NEXT_WORD_COMMAND_ID,
            "Delete Next Word",
            input::DeleteToNextWordEnd,
        ),
        window_command(FIND_COMMAND_ID, "Find", input::Search),
        edit_command(
            SELECT_ALL_COMMAND_ID,
            "Select All",
            input::SelectAll,
            OsAction::SelectAll,
        ),
    ]);

    #[cfg(target_os = "macos")]
    if platform == DesktopPlatform::MacOs {
        commands.push(
            RuntimeCommand::new(
                MINIMIZE_COMMAND_ID,
                "Minimize",
                CommandScope::Window,
                Minimize,
            )
            .with_binding("cmd-m"),
        );
        commands.push(RuntimeCommand::new(
            ZOOM_COMMAND_ID,
            "Zoom",
            CommandScope::Window,
            Zoom,
        ));
    }

    commands.push(
        RuntimeCommand::new(
            CLOSE_WINDOW_COMMAND_ID,
            "Close Window",
            CommandScope::Window,
            CloseWindow,
        )
        .with_binding(close_binding(platform)),
    );

    commands
}

/// Install the action handlers for the framework's standard commands.
///
/// Separate from [`framework_commands`] so the installer can register every
/// command first and only then take the irreversible step of adding handlers.
pub(super) fn install_framework_handlers(cx: &mut App) {
    register_handlers(cx);
}

/// The command ids the resolved standard features contribute.
///
/// Separate from [`framework_command_ids`] because these are *conditional*: the
/// Settings id exists only when a Settings surface was declared, and the About
/// id only when About was not disabled.
pub(super) fn feature_command_ids(features: StandardFeatures) -> Vec<CommandId> {
    let mut ids = Vec::new();
    if features.has_settings() {
        ids.push(OPEN_SETTINGS_COMMAND_ID);
    }
    if features.has_about() {
        ids.push(ABOUT_COMMAND_ID);
    }
    ids
}

/// Build the commands the resolved standard features contribute, without
/// placements.
///
/// Ids, labels, and bindings stay framework-owned here: an application replaces
/// the *surface* behind a standard feature, never its vocabulary or its
/// platform conventions.
pub(super) fn feature_commands(
    platform: DesktopPlatform,
    features: StandardFeatures,
) -> Vec<RuntimeCommand> {
    let mut commands = Vec::new();
    if features.has_settings() {
        commands.push(
            RuntimeCommand::new(
                OPEN_SETTINGS_COMMAND_ID,
                settings_label(platform),
                CommandScope::App,
                OpenSettings,
            )
            .with_binding(settings_binding(platform)),
        );
    }
    if features.has_about() {
        commands.push(
            RuntimeCommand::new(ABOUT_COMMAND_ID, "About", CommandScope::App, About)
                .with_derived_label(about_label),
        );
    }
    commands
}

/// Install the action handlers routing the standard feature commands to the
/// surfaces the declaration resolved.
pub(super) fn install_feature_handlers(cx: &mut App, features: StandardFeatures) {
    if let Some(open) = features.settings {
        cx.on_action(move |_: &OpenSettings, cx: &mut App| {
            if let Err(error) = open(cx) {
                report_callback_error(cx, OPEN_SETTINGS_COMMAND_ID, error);
            }
        });
    }
    if let Some(open) = features.about {
        cx.on_action(move |_: &About, cx: &mut App| {
            if let Err(error) = open(cx) {
                report_callback_error(cx, ABOUT_COMMAND_ID, error);
            }
        });
    }
}

/// `About <App>` — the convention on every desktop platform.
fn about_label(cx: &App) -> gpui::SharedString {
    format!("About {}", app_display_name(cx)).into()
}

fn edit_command(
    id: CommandId,
    label: &'static str,
    action: impl gpui::Action,
    os_action: OsAction,
) -> RuntimeCommand {
    RuntimeCommand::new(id, label, CommandScope::Window, action).with_os_action(os_action)
}

/// A window-scoped standard Edit command with no [`OsAction`] counterpart.
///
/// [`OsAction`] only covers the handful of edits the platform menu can
/// specialize (cut/copy/paste/select all/undo/redo); Delete, the word-delete
/// pair, and Find have no matching variant, so these commands carry none.
fn window_command(id: CommandId, label: &'static str, action: impl gpui::Action) -> RuntimeCommand {
    RuntimeCommand::new(id, label, CommandScope::Window, action)
}

/// `Quit <App>` — the macOS convention.
fn quit_label(cx: &App) -> gpui::SharedString {
    format!("Quit {}", app_display_name(cx)).into()
}

/// `Hide <App>` — the macOS convention.
#[cfg(target_os = "macos")]
fn hide_app_label(cx: &App) -> gpui::SharedString {
    format!("Hide {}", app_display_name(cx)).into()
}

/// The application display name, used as the derived title of the standard
/// application menu. Falls back to the menu key before the shell is installed,
/// matching the current native projection.
pub(super) fn app_display_name(cx: &App) -> gpui::SharedString {
    super::menu::app_menu_title(cx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_plan_uses_native_desktop_conventions() {
        assert_eq!(
            standard_menu_keys(DesktopPlatform::MacOs, false),
            vec![MenuKey::APP, MenuKey::EDIT, MenuKey::WINDOW]
        );
        assert_eq!(
            standard_menu_keys(DesktopPlatform::Windows, true),
            vec![
                MenuKey::FILE,
                MenuKey::EDIT,
                MenuKey::VIEW,
                MenuKey::WINDOW,
                MenuKey::HELP
            ]
        );
        assert_eq!(
            standard_menu_keys(DesktopPlatform::Linux, false),
            vec![MenuKey::FILE, MenuKey::EDIT, MenuKey::WINDOW, MenuKey::HELP]
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
    fn standard_command_membership_is_host_independent() {
        // A mac-only id must be recognized as standard even when the build
        // host is not macOS, and vice versa for a non-mac id: validation runs
        // the same on every host regardless of `DesktopPlatform::current()`.
        assert!(is_standard_command(HIDE_APP_COMMAND_ID));
        assert!(is_standard_command(MINIMIZE_COMMAND_ID));
        assert!(is_standard_command(QUIT_COMMAND_ID));
        assert!(is_standard_command(UNDO_COMMAND_ID));
        assert!(is_standard_command(CLOSE_WINDOW_COMMAND_ID));
        assert!(is_standard_command(DELETE_COMMAND_ID));
        assert!(is_standard_command(DELETE_PREVIOUS_WORD_COMMAND_ID));
        assert!(is_standard_command(DELETE_NEXT_WORD_COMMAND_ID));
        assert!(is_standard_command(FIND_COMMAND_ID));

        // Settings/About are conditional features, not entries in
        // `framework_command_ids`, but they are still framework-owned ids.
        assert!(is_standard_command(OPEN_SETTINGS_COMMAND_ID));
        assert!(is_standard_command(ABOUT_COMMAND_ID));

        assert!(!is_standard_command(CommandId("app.not_standard")));
    }

    #[test]
    fn every_standard_binding_parses() {
        for platform in [
            DesktopPlatform::MacOs,
            DesktopPlatform::Windows,
            DesktopPlatform::Linux,
        ] {
            for command in [
                RuntimeCommand::new(
                    OPEN_SETTINGS_COMMAND_ID,
                    "Settings",
                    CommandScope::App,
                    OpenSettings,
                )
                .with_binding(settings_binding(platform)),
                RuntimeCommand::new(QUIT_COMMAND_ID, "Quit", CommandScope::App, Quit)
                    .with_binding(quit_binding(platform)),
                RuntimeCommand::new(
                    CLOSE_WINDOW_COMMAND_ID,
                    "Close Window",
                    CommandScope::Window,
                    CloseWindow,
                )
                .with_binding(close_binding(platform)),
            ] {
                super::super::menus::validate_command_binding(&command).unwrap();
            }
        }
    }
}
