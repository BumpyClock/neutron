//! Command registry and menu projection (plan §3 — "commands before menus").
//!
//! A [`Command`] carries a stable [`CommandId`], a GPUI action, a scope, a
//! localizable label, optional enabled/checked predicates, an optional default
//! keybinding, and an optional [`MenuPlacement`]. The [`CommandRegistry`] is the
//! canonical vocabulary; the native menu, the dock menu, and (later) tray menus
//! and keymap files are all *projections* of it.
//!
//! Contributors that own their own actions (the theme service's
//! Appearance/Theme submenu, the window manager's Move-to-Window section) feed a
//! reserved menu section through the [`MenuSection`] seam, so this module never
//! imports them.
//!
//! The [`MenusPlugin`] installs the registry, wires the standard command set,
//! registers default keybindings, and re-installs the native menus whenever a
//! contributing section signals change via [`menus_invalidate`].

mod menu;
mod plugin;
mod standard;

use std::collections::HashMap;

use gpui::{Action, App, Entity, Global, MenuItem, OsAction, SharedString, WeakEntity};
use gpui_component::menu::AppMenuBar;
use thiserror::Error;

pub use menu::{MenuNode, MenuOutline, MenuPlan, THEME_SECTION};
pub use plugin::{MenusPlugin, menus_invalidate};
pub use standard::{About, CloseWindow, OpenSettings, Quit, StandardMenus};
#[cfg(target_os = "macos")]
pub use standard::{HideApp, HideOthers, Minimize, ShowAll, Zoom};

/// The top-level menu key used for the application menu. Its rendered title is
/// the app display name; placements and outlines match on this key.
pub const APP_MENU: &str = "App";
/// The top-level menu key for the standard Edit menu.
pub const EDIT_MENU: &str = "Edit";
/// The top-level menu key for the standard Window menu.
pub const WINDOW_MENU: &str = "Window";
/// The pseudo-menu key for commands projected into the macOS dock menu.
pub const DOCK_MENU: &str = "Dock";
/// The top-level File menu used on Windows and Linux.
pub const FILE_MENU: &str = "File";
/// The optional View menu used on Windows and Linux.
pub const VIEW_MENU: &str = "View";
/// The top-level Help menu used on Windows and Linux.
pub const HELP_MENU: &str = "Help";

/// Stable id for the standard Settings/Preferences command.
pub const OPEN_SETTINGS_COMMAND_ID: CommandId = CommandId("app.settings");
/// Stable id for the standard About command.
pub const ABOUT_COMMAND_ID: CommandId = CommandId("app.about");
/// Stable id for the standard Quit command.
pub const QUIT_COMMAND_ID: CommandId = CommandId("app.quit");

/// Errors returned while registering commands.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum CommandError {
    /// A command's default keybinding could not be parsed.
    #[error("invalid default binding `{binding}` for command `{command}`")]
    InvalidBinding {
        /// Stable command id.
        command: CommandId,
        /// Invalid GPUI keystroke string.
        binding: &'static str,
        /// Parser error.
        #[source]
        source: gpui::InvalidKeystrokeError,
    },
}

/// A stable, process-unique identifier for a command.
///
/// The canonical key across every projection (menu, dock, tray, keymap). Ids are
/// `&'static str` so they can be written as literals and compared cheaply; keep
/// them stable across releases (they will anchor user keymap files).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct CommandId(pub &'static str);

impl CommandId {
    /// The underlying string.
    pub const fn as_str(&self) -> &'static str {
        self.0
    }
}

impl std::fmt::Display for CommandId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

/// Where a command's handler is dispatched.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CommandScope {
    /// Dispatched to the application via a global `on_action` handler.
    App,
    /// Dispatched to the focused window/view (e.g. the Edit block, which the
    /// focused input handles). The shell registers no global handler for these.
    Window,
}

/// Placement of a command inside a projected menu.
///
/// Menus are assembled by grouping every command with a matching [`menu`](Self::menu)
/// key, sorting by `(group, order)`, and inserting a separator between adjacent
/// groups.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MenuPlacement {
    /// The top-level menu key (e.g. [`APP_MENU`], [`EDIT_MENU`]).
    pub menu: &'static str,
    /// Group index; a separator is inserted between adjacent groups.
    pub group: u16,
    /// Order within the group.
    pub order: u16,
}

impl MenuPlacement {
    /// Place a command in `menu`, `group`, at `order`.
    pub const fn new(menu: &'static str, group: u16, order: u16) -> Self {
        Self { menu, group, order }
    }
}

/// A single command: the canonical unit projected into menus and keymaps.
pub struct Command {
    id: CommandId,
    label: SharedString,
    scope: CommandScope,
    action: Box<dyn Action>,
    os_action: Option<OsAction>,
    enabled: Option<fn(&App) -> bool>,
    checked: Option<fn(&App) -> bool>,
    default_binding: Option<&'static str>,
    placement: Option<MenuPlacement>,
}

impl Command {
    /// Create a command with the required fields. Chain the `with_*` builders to
    /// add menu placement, predicates, and a default binding.
    pub fn new(
        id: CommandId,
        label: impl Into<SharedString>,
        scope: CommandScope,
        action: impl Action,
    ) -> Self {
        Self {
            id,
            label: label.into(),
            scope,
            action: Box::new(action),
            os_action: None,
            enabled: None,
            checked: None,
            default_binding: None,
            placement: None,
        }
    }

    /// Associate the OS action so the platform can supply specialized behavior
    /// (macOS standard Edit items).
    pub fn with_os_action(mut self, os_action: OsAction) -> Self {
        self.os_action = Some(os_action);
        self
    }

    /// Set the enabled predicate. Absent means always enabled.
    pub fn with_enabled(mut self, f: fn(&App) -> bool) -> Self {
        self.enabled = Some(f);
        self
    }

    /// Set the checked predicate. Absent means unchecked.
    pub fn with_checked(mut self, f: fn(&App) -> bool) -> Self {
        self.checked = Some(f);
        self
    }

    /// Set the default keybinding (component < shell < app precedence; registered
    /// in registry order so later registrations win).
    pub fn with_binding(mut self, keystroke: &'static str) -> Self {
        self.default_binding = Some(keystroke);
        self
    }

    /// Place the command in a menu.
    pub fn with_placement(mut self, placement: MenuPlacement) -> Self {
        self.placement = Some(placement);
        self
    }

    /// Convenience for [`Command::with_placement`].
    pub fn placed(self, menu: &'static str, group: u16, order: u16) -> Self {
        self.with_placement(MenuPlacement::new(menu, group, order))
    }

    /// The command's stable id.
    pub fn id(&self) -> CommandId {
        self.id
    }

    /// The command's label.
    pub fn label(&self) -> &SharedString {
        &self.label
    }

    /// The command's dispatch scope.
    pub fn scope(&self) -> CommandScope {
        self.scope
    }

    /// The command's menu placement, if any.
    pub fn placement(&self) -> Option<MenuPlacement> {
        self.placement
    }

    /// The command's default keybinding, if any.
    pub fn default_binding(&self) -> Option<&'static str> {
        self.default_binding
    }

    /// A fresh boxed clone of the command's action (for menu/keymap projection).
    pub fn boxed_action(&self) -> Box<dyn Action> {
        self.action.boxed_clone()
    }

    /// Project the command into a menu item, evaluating checked/enabled against
    /// `cx`.
    fn to_menu_item(&self, cx: &App) -> MenuItem {
        let flags = menu_flags(self.checked.map(|f| f(cx)), self.enabled.map(|f| f(cx)));
        MenuItem::Action {
            name: self.label.clone(),
            action: self.action.boxed_clone(),
            os_action: self.os_action,
            checked: flags.checked,
            disabled: flags.disabled,
        }
    }
}

/// Resolved menu-item flags. Pure so the checked/enabled → checked/disabled
/// mapping is unit-testable without a live `App`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct MenuFlags {
    checked: bool,
    disabled: bool,
}

/// Map optional predicate results to menu-item flags: an absent checked
/// predicate is unchecked; an absent enabled predicate is enabled.
fn menu_flags(checked: Option<bool>, enabled: Option<bool>) -> MenuFlags {
    MenuFlags {
        checked: checked.unwrap_or(false),
        disabled: !enabled.unwrap_or(true),
    }
}

/// A provider that contributes freshly-built menu items to a reserved section.
///
/// The seam that lets the theme service (Appearance/Theme) and, later, the
/// window manager (Move-to-Window) feed a menu section without this module
/// importing them. Items are rebuilt on demand so checked/enabled state is
/// always current. A blanket impl accepts a plain `Fn(&App) -> Vec<MenuItem>`.
pub trait MenuSection: 'static {
    /// Build this section's items against the current app state.
    fn items(&self, cx: &App) -> Vec<MenuItem>;
}

impl<F: Fn(&App) -> Vec<MenuItem> + 'static> MenuSection for F {
    fn items(&self, cx: &App) -> Vec<MenuItem> {
        self(cx)
    }
}

/// The command registry: a main-thread GPUI [`Global`].
///
/// Holds commands (deduped by [`CommandId`], last-registration-wins), the
/// section providers keyed by slot name, and the active [`MenuPlan`] the plugin
/// re-projects on invalidation.
#[derive(Default)]
pub struct CommandRegistry {
    commands: Vec<Command>,
    index: HashMap<CommandId, usize>,
    sections: HashMap<&'static str, Box<dyn MenuSection>>,
    plan: Option<MenuPlan>,
    /// Set once the plugin has run its initial binding + projection pass. While
    /// unset, [`register_command`](AppCommandsExt::register_command) only records
    /// (the plugin binds/projects the whole registry in one batch); once set,
    /// later registrations must bind and re-project themselves immediately.
    active: bool,
    menu_bars: Vec<WeakEntity<AppMenuBar>>,
}

impl Global for CommandRegistry {}

impl CommandRegistry {
    /// A new, empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a command. If an id is already present its entry is replaced
    /// (last-registration-wins) so apps can override a standard command; the
    /// registration slot (and thus binding precedence order) is preserved.
    pub fn register(&mut self, command: Command) -> Result<(), CommandError> {
        plugin::validate_command_binding(&command)?;
        if let Some(&ix) = self.index.get(&command.id) {
            self.commands[ix] = command;
        } else {
            let ix = self.commands.len();
            self.index.insert(command.id, ix);
            self.commands.push(command);
        }
        Ok(())
    }

    /// Look up a command by id.
    pub fn get(&self, id: CommandId) -> Option<&Command> {
        self.index.get(&id).map(|&ix| &self.commands[ix])
    }

    /// All registered commands in registration order.
    pub fn commands(&self) -> &[Command] {
        &self.commands
    }

    /// Register a menu-section provider under `slot`. Replaces any prior provider
    /// for the same slot.
    pub fn register_section(&mut self, slot: &'static str, section: impl MenuSection) {
        self.sections.insert(slot, Box::new(section));
    }

    /// The section provider for `slot`, if registered.
    pub fn section(&self, slot: &str) -> Option<&dyn MenuSection> {
        self.sections.get(slot).map(|b| b.as_ref())
    }

    /// Set the active menu plan (installed by the plugin).
    pub fn set_plan(&mut self, plan: MenuPlan) {
        self.plan = Some(plan);
    }

    /// The active menu plan, if one is installed.
    pub fn plan(&self) -> Option<&MenuPlan> {
        self.plan.as_ref()
    }

    /// Mark the registry live: the plugin has bound and projected the initial
    /// registry, so subsequent registrations bind/project dynamically.
    pub fn activate(&mut self) {
        self.active = true;
    }

    /// Whether the initial projection pass has completed.
    pub fn is_active(&self) -> bool {
        self.active
    }

    fn register_menu_bar(&mut self, menu_bar: WeakEntity<AppMenuBar>) {
        self.menu_bars.push(menu_bar);
    }

    fn take_menu_bars(&mut self) -> Vec<WeakEntity<AppMenuBar>> {
        std::mem::take(&mut self.menu_bars)
    }

    fn replace_menu_bars(&mut self, menu_bars: Vec<WeakEntity<AppMenuBar>>) {
        self.menu_bars = menu_bars;
    }
}

/// Registry access on the raw `gpui::App`. The registry global is created lazily
/// on first mutation, so contributors may register before the plugin's `init`.
///
/// Registration is *dynamic once the plugin is live*: because
/// [`AppPlugin`](crate::AppPlugin) is
/// sealed, apps first reach `&mut App` from `on_launch` — after the plugin's
/// initial binding/projection pass. Commands registered then are bound and
/// re-projected immediately (late app-tier bindings still sit above the shell
/// defaults and below any future user overrides).
pub trait AppCommandsExt {
    /// Register a command (creates the registry global if absent). If the plugin
    /// has already run its initial pass, the command's default binding is bound
    /// and the menus are re-projected immediately.
    ///
    /// # Errors
    ///
    /// Returns [`CommandError::InvalidBinding`] before mutating the registry
    /// when the command's default binding is invalid.
    fn register_command(&mut self, command: Command) -> Result<(), CommandError>;
    /// Register a menu-section provider (creates the registry global if absent).
    /// Re-projects the menus immediately if the plugin is already live.
    fn register_menu_section(&mut self, slot: &'static str, section: impl MenuSection);
    /// The command registry, if it has been created.
    fn command_registry(&self) -> Option<&CommandRegistry>;
}

impl AppCommandsExt for App {
    fn register_command(&mut self, command: Command) -> Result<(), CommandError> {
        plugin::validate_command_binding(&command)?;
        let id = command.id();
        // Decide what to do about keybindings/menus *before* the registry mutates
        // (we need to know whether this id already existed).
        let effect = self
            .command_registry()
            .map_or(RegistrationEffect::None, |registry| {
                RegistrationEffect::of(registry.is_active(), registry.get(id).is_some())
            });
        ensure_registry(self).register(command)?;
        match effect {
            RegistrationEffect::None => {}
            RegistrationEffect::Append => plugin::bind_and_reproject(self, id)?,
            RegistrationEffect::Rebuild => plugin::rebuild_bindings_and_reproject(self)?,
        }
        Ok(())
    }

    fn register_menu_section(&mut self, slot: &'static str, section: impl MenuSection) {
        ensure_registry(self).register_section(slot, section);
        if self.global::<CommandRegistry>().is_active() {
            menus_invalidate(self);
        }
    }

    fn command_registry(&self) -> Option<&CommandRegistry> {
        self.has_global::<CommandRegistry>()
            .then(|| self.global::<CommandRegistry>())
    }
}

/// Per-window application menu-bar construction.
pub trait AppMenusExt {
    /// Create a Windows/Linux in-window menu bar backed by the active command
    /// registry. The shell weakly tracks the entity and reloads it whenever
    /// menus are reprojected.
    fn new_app_menu_bar(&mut self) -> Entity<AppMenuBar>;
}

impl AppMenusExt for App {
    fn new_app_menu_bar(&mut self) -> Entity<AppMenuBar> {
        let menu_bar = AppMenuBar::new(self);
        ensure_registry(self).register_menu_bar(menu_bar.downgrade());
        menu_bar
    }
}

/// What a live registration must do about keybindings and menus.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RegistrationEffect {
    /// Registry not yet live: the plugin binds/projects the whole registry in one
    /// batch, so recording is enough.
    None,
    /// A brand-new id after go-live: append its binding (no stale chord to clear).
    Append,
    /// Replacing an existing id after go-live: rebuild bindings so the replaced
    /// command's now-stale chord is removed.
    Rebuild,
}

impl RegistrationEffect {
    fn of(active: bool, id_exists: bool) -> Self {
        match (active, id_exists) {
            (false, _) => Self::None,
            (true, false) => Self::Append,
            (true, true) => Self::Rebuild,
        }
    }
}

/// Create the registry global if absent, then return it mutably.
fn ensure_registry(cx: &mut App) -> &mut CommandRegistry {
    if !cx.has_global::<CommandRegistry>() {
        cx.set_global(CommandRegistry::new());
    }
    cx.global_mut::<CommandRegistry>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::actions;

    actions!(commands_test, [Alpha, Beta]);

    fn cmd(id: &'static str, action: impl Action) -> Command {
        Command::new(CommandId(id), id, CommandScope::App, action)
    }

    #[test]
    fn register_dedups_last_wins_and_keeps_slot() {
        let mut reg = CommandRegistry::new();
        reg.register(cmd("a", Alpha).with_binding("cmd-a")).unwrap();
        reg.register(cmd("b", Beta)).unwrap();
        // Re-register "a" with a new label/binding: replaces in place.
        reg.register(
            Command::new(CommandId("a"), "A2", CommandScope::App, Alpha).with_binding("cmd-x"),
        )
        .unwrap();

        assert_eq!(reg.commands().len(), 2, "dedup keeps a single entry per id");
        assert_eq!(reg.get(CommandId("a")).unwrap().label().as_ref(), "A2");
        assert_eq!(
            reg.get(CommandId("a")).unwrap().default_binding(),
            Some("cmd-x")
        );
        // Slot order preserved: "a" still precedes "b".
        assert_eq!(reg.commands()[0].id(), CommandId("a"));
        assert_eq!(reg.commands()[1].id(), CommandId("b"));
    }

    #[test]
    fn registration_effect_routes_by_liveness_and_existing_id() {
        // Before go-live: batch pass handles everything.
        assert_eq!(
            RegistrationEffect::of(false, false),
            RegistrationEffect::None
        );
        assert_eq!(
            RegistrationEffect::of(false, true),
            RegistrationEffect::None
        );
        // After go-live: new id appends, existing id rebuilds (drops stale chord).
        assert_eq!(
            RegistrationEffect::of(true, false),
            RegistrationEffect::Append
        );
        assert_eq!(
            RegistrationEffect::of(true, true),
            RegistrationEffect::Rebuild
        );
    }

    #[test]
    fn menu_flags_defaults_and_inversion() {
        // Absent predicates: unchecked, enabled.
        assert_eq!(
            menu_flags(None, None),
            MenuFlags {
                checked: false,
                disabled: false
            }
        );
        // Checked true, disabled is the inverse of enabled.
        assert_eq!(
            menu_flags(Some(true), Some(false)),
            MenuFlags {
                checked: true,
                disabled: true
            }
        );
        assert_eq!(
            menu_flags(Some(false), Some(true)),
            MenuFlags {
                checked: false,
                disabled: false
            }
        );
    }

    #[test]
    fn invalid_binding_does_not_mutate_registry() {
        let mut reg = CommandRegistry::new();
        reg.register(cmd("a", Alpha)).unwrap();

        let error = reg
            .register(cmd("b", Beta).with_binding("not-a-valid-modifier-x"))
            .unwrap_err();

        assert!(matches!(error, CommandError::InvalidBinding { .. }));
        assert!(reg.get(CommandId("a")).is_some());
        assert!(reg.get(CommandId("b")).is_none());
    }

    #[gpui::test]
    fn dead_app_menu_bars_are_pruned(cx: &mut gpui::TestAppContext) {
        let menu_bar = cx.update(|cx| {
            let menu_bar = cx.new_app_menu_bar();
            assert_eq!(cx.global::<CommandRegistry>().menu_bars.len(), 1);
            menu_bar
        });
        drop(menu_bar);
        cx.update(|cx| {
            menus_invalidate(cx);
            assert!(cx.global::<CommandRegistry>().menu_bars.is_empty());
        });
    }
}
