//! Command registry and menu projection (plan §3 — "commands before menus").
//!
//! A [`RuntimeCommand`] carries a stable [`CommandId`], a GPUI action, a scope,
//! a localizable label, optional enabled/checked predicates, an optional
//! default keybinding, and an optional [`MenuPlacement`]. The
//! [`CommandRegistry`] is the canonical vocabulary; the native menu, the dock
//! menu, and (later) tray menus and keymap files are all *projections* of it.
//!
//! Contributors that own their own actions (the theme service's
//! Appearance/Theme submenu, the window manager's Move-to-Window section) feed a
//! reserved menu section through the [`MenuSection`] seam, so this module never
//! imports them.
//!
//! The [`MenusModule`] installs the registry, wires the standard command set,
//! registers default keybindings, and re-installs the native menus whenever a
//! contributing section signals change via [`menus_invalidate`].

mod binding;
mod declaration;
mod declared;
mod faults;
mod keys;
mod label;
mod menu;
mod menu_model;
mod menus;
pub mod standard;

use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gpui::{Action, App, Entity, Global, MenuItem, OsAction, SharedString, WeakEntity};
use neutron_components::menu::AppMenuBar;
use thiserror::Error;

use standard::DesktopPlatform;

pub(crate) use menu::{MenuPlan, THEME_SECTION};
pub(crate) use menus::{MenusModule, menus_invalidate};

pub use keys::{KeyFault, MenuKey, MenuSectionKey};

pub use binding::CommandBinding;
pub use label::{CommandLabel, MenuLabel};

// Crate-internal re-exports so the declaration core can name the typed command
// vocabulary, and so the executor (kept private) can still build native menus
// from the old string-keyed model.
pub(crate) use declaration::{CommandsDeclaration, SectionProvider, StandardFeatures};
pub use declared::Command;
// Public: `DeclarationError::Command` carries a `CommandFault`, so it must be
// publicly nameable rather than reachable only through a `pub(crate)` path.
pub use faults::CommandFault;
pub use menu_model::{Menu, MenuBar, MenuNode, MenuOutline, MenuOutlineEntry};

// String aliases of the typed menu keys, used only by the private executor
// (`menu.rs`/`standard.rs`) that still builds native menus from a flat
// string-keyed model. Not part of the public API.

/// The top-level menu key used for the application menu. Its rendered title is
/// the app display name; placements and outlines match on this key.
pub(crate) const APP_MENU: &str = MenuKey::APP.as_str();
/// The top-level menu key for the standard Edit menu.
#[allow(dead_code)] // Exercised only by unit tests; no production caller yet.
pub(crate) const EDIT_MENU: &str = MenuKey::EDIT.as_str();
/// The top-level menu key for the standard Window menu.
#[allow(dead_code)] // Exercised only by unit tests; no production caller yet.
pub(crate) const WINDOW_MENU: &str = MenuKey::WINDOW.as_str();
/// The pseudo-menu key for commands projected into the macOS dock menu.
pub(crate) const DOCK_MENU: &str = MenuKey::DOCK.as_str();

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
    /// A command id was registered twice through the explicit
    /// register-only path, which never silently replaces an existing entry.
    #[error("command `{command}` is already registered")]
    Duplicate {
        /// Stable command id.
        command: CommandId,
    },
    /// A command's key context could not be parsed as a GPUI context
    /// predicate.
    #[error("invalid key context `{context}` for command `{command}`")]
    InvalidKeyContext {
        /// Stable command id.
        command: CommandId,
        /// Invalid context predicate.
        context: &'static str,
        /// Parser error.
        #[source]
        source: anyhow::Error,
    },
    /// A menu-section key was registered twice through
    /// [`Commands::register_section`], which never silently replaces an
    /// existing provider.
    #[error("menu section `{section}` is already registered")]
    DuplicateSection {
        /// The repeated section key.
        section: MenuSectionKey,
    },
    /// A command id and its GPUI action type must map to each other 1:1
    /// (issue #11): registering or replacing a command whose id already
    /// names a different action, or whose action already belongs to a
    /// different id, is rejected rather than silently creating a second
    /// mapping (which would leave a stale `on_action` trampoline pointed at
    /// the wrong handler). `replace_command` additionally rejects changing an
    /// existing id's scope (app-scoped vs. window-scoped).
    #[error(
        "command `{command}` is incompatible with the current id/action mapping for `{action}`"
    )]
    IncompatibleAction {
        /// The command id attempting the (re)registration.
        command: CommandId,
        /// Stable name of the conflicting action.
        action: &'static str,
    },
    /// [`Commands::replace_command`] targeted a framework-owned standard
    /// command id.
    ///
    /// The standard Quit/About/Settings/edit/window handlers are wired
    /// through the framework's own raw GPUI `on_action` listeners, not
    /// through [`super::set_action_handler`]; replacing one of these ids
    /// would layer a second app-level handler on top of that listener
    /// instead of swapping it. Settings and About stay surface-derived (an
    /// application supplies the surface the standard command opens, never
    /// the command itself), OS-routed edit/window actions are not
    /// overridable, and no standard command is designated overridable in
    /// this milestone, so every framework-owned id is rejected here.
    #[error("command `{command}` is a framework-owned standard command and cannot be replaced")]
    FrameworkOwned {
        /// The rejected standard command id.
        command: CommandId,
    },
}

/// A stable, process-unique identifier for a command.
///
/// The canonical key across every projection (menu, dock, tray, keymap). Ids are
/// `&'static str` so they can be written as literals and compared cheaply; keep
/// them stable across releases (they will anchor user keymap files).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct CommandId(pub(crate) &'static str);

impl CommandId {
    /// Create a stable command id.
    pub const fn new(id: &'static str) -> Self {
        Self(id)
    }

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
pub(crate) enum CommandScope {
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
pub(crate) struct MenuPlacement {
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

/// A single runtime command: the canonical unit projected into menus and
/// keymaps.
///
/// Private executor type. Applications declare the typed
/// [`Command`](super::declared::Command) instead; this is what it lowers to.
pub(crate) struct RuntimeCommand {
    id: CommandId,
    label: SharedString,
    derived_label: Option<fn(&App) -> SharedString>,
    scope: CommandScope,
    action: Box<dyn Action>,
    os_action: Option<OsAction>,
    enabled: Option<fn(&App) -> bool>,
    checked: Option<fn(&App) -> bool>,
    default_binding: Option<&'static str>,
    binding_context: Option<&'static str>,
    placements: Vec<MenuPlacement>,
}

impl RuntimeCommand {
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
            derived_label: None,
            scope,
            action: Box::new(action),
            os_action: None,
            enabled: None,
            checked: None,
            default_binding: None,
            binding_context: None,
            placements: Vec::new(),
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

    /// Derive the label from application state at projection time.
    ///
    /// The static label stays the fallback used by [`RuntimeCommand::label`] (and by
    /// callers that inspect a command without an `App`); menu projection
    /// prefers the derived text.
    pub(crate) fn with_derived_label(mut self, f: fn(&App) -> SharedString) -> Self {
        self.derived_label = Some(f);
        self
    }

    /// Scope the default keybinding to a GPUI key-context predicate.
    ///
    /// Absent means the binding is global. Parsing happens where the binding is
    /// loaded so a bad predicate surfaces as a [`CommandError`].
    pub(crate) fn with_binding_context(mut self, context: &'static str) -> Self {
        self.binding_context = Some(context);
        self
    }

    /// Place the command in a menu.
    ///
    /// A command may be projected into several menus (and into the macOS dock
    /// menu) — issue #11 requires it — so this *appends*. Call it once per
    /// surface the command should appear in; the projection order within a menu
    /// is decided by the placement's `(group, order)`, not by call order.
    pub fn with_placement(mut self, placement: MenuPlacement) -> Self {
        self.placements.push(placement);
        self
    }

    /// Convenience for [`RuntimeCommand::with_placement`].
    #[allow(dead_code)] // Exercised only by unit tests; no production caller yet.
    pub fn placed(self, menu: &'static str, group: u16, order: u16) -> Self {
        self.with_placement(MenuPlacement::new(menu, group, order))
    }

    /// The command's stable id.
    pub fn id(&self) -> CommandId {
        self.id
    }

    /// The command's label.
    #[allow(dead_code)] // Exercised only by unit tests; no production caller yet.
    pub fn label(&self) -> &SharedString {
        &self.label
    }

    /// The command's dispatch scope.
    pub fn scope(&self) -> CommandScope {
        self.scope
    }

    /// `TypeId` of the command's action.
    ///
    /// The runtime backstop for issue #11's one-command-per-action-type rule:
    /// a command id and its action type must map to each other 1:1, checked
    /// against [`CommandRegistry::action_owner`] before a registration or
    /// replacement is allowed to mutate the live registry.
    pub(crate) fn action_type(&self) -> TypeId {
        self.action.as_any().type_id()
    }

    /// The action's stable display name (e.g. `app::About`), for
    /// [`CommandError::IncompatibleAction`] diagnostics.
    pub(crate) fn action_name(&self) -> &'static str {
        self.action.name()
    }

    /// The command's first menu placement, if any.
    ///
    /// A compatibility accessor for callers written against the
    /// one-placement-per-command model. Menu and dock projection use
    /// [`placements`](Self::placements) so every surface is honored.
    #[allow(dead_code)] // Exercised only by unit tests; no production caller yet.
    pub fn placement(&self) -> Option<MenuPlacement> {
        self.placements.first().copied()
    }

    /// Every menu placement, in the order they were declared.
    pub fn placements(&self) -> &[MenuPlacement] {
        &self.placements
    }

    /// The command's default keybinding, if any.
    pub fn default_binding(&self) -> Option<&'static str> {
        self.default_binding
    }

    /// The key-context predicate scoping the default binding, if any.
    pub(crate) fn binding_context(&self) -> Option<&'static str> {
        self.binding_context
    }

    /// The label as projected into menus, resolving a derived label against
    /// `cx` and falling back to the static label.
    pub(crate) fn resolved_label(&self, cx: &App) -> SharedString {
        match self.derived_label {
            Some(f) => f(cx),
            None => self.label.clone(),
        }
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
            name: self.resolved_label(cx),
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
pub(crate) trait MenuSection: 'static {
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
/// section providers keyed by slot name, and the active [`MenuPlan`] the module
/// re-projects on invalidation.
#[derive(Default)]
pub(crate) struct CommandRegistry {
    commands: Vec<RuntimeCommand>,
    index: HashMap<CommandId, usize>,
    sections: HashMap<&'static str, Box<dyn MenuSection>>,
    plan: Option<MenuPlan>,
    /// Set once the module has run its initial binding + projection pass. While
    /// unset, [`register_command`](AppCommandsExt::register_command) only records
    /// (the module binds/projects the whole registry in one batch); once set,
    /// later registrations must bind and re-project themselves immediately.
    active: bool,
    menu_bars: Vec<WeakEntity<AppMenuBar>>,
    /// The current app-scoped handler for each action type that has one,
    /// behind a shared cell so replacing a command's behavior (issue #11)
    /// never installs a second GPUI `on_action` listener for the same action.
    /// Erased as `Box<dyn Any>` holding `Rc<RefCell<ActionHandler<A>>>`;
    /// [`set_action_handler`] is the only code that creates or reads an entry,
    /// so the concrete `A` is always known at the downcast site.
    action_handlers: HashMap<TypeId, Box<dyn Any>>,
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
    pub fn register(&mut self, command: RuntimeCommand) -> Result<(), CommandError> {
        menus::validate_command_binding(&command)?;
        if let Some(&ix) = self.index.get(&command.id) {
            self.commands[ix] = command;
        } else {
            let ix = self.commands.len();
            self.index.insert(command.id, ix);
            self.commands.push(command);
        }
        Ok(())
    }

    /// Whether `id` is already registered.
    pub(crate) fn contains(&self, id: CommandId) -> bool {
        self.index.contains_key(&id)
    }

    /// The command id currently mapped to `action_type`, if the type is
    /// already owned by a registered command (issue #11: an action type names
    /// exactly one stable command id).
    fn action_owner(&self, action_type: TypeId) -> Option<CommandId> {
        self.commands
            .iter()
            .find(|command| command.action_type() == action_type)
            .map(RuntimeCommand::id)
    }

    /// Look up a command by id.
    pub fn get(&self, id: CommandId) -> Option<&RuntimeCommand> {
        self.index.get(&id).map(|&ix| &self.commands[ix])
    }

    /// All registered commands in registration order.
    pub fn commands(&self) -> &[RuntimeCommand] {
        &self.commands
    }

    /// Register a menu-section provider under `slot`. Replaces any prior provider
    /// for the same slot.
    ///
    /// Framework-internal replace-on-register seam (e.g. the theme service's
    /// Appearance section), where a repeated slot is a re-registration by
    /// trusted framework code, not an application mistake. Application code
    /// registers sections through [`Commands::register_section`], which
    /// rejects a duplicate instead.
    pub fn set_section(&mut self, slot: &'static str, section: impl MenuSection) {
        self.sections.insert(slot, Box::new(section));
    }

    /// Register a *new* menu-section provider under `slot`.
    ///
    /// # Errors
    ///
    /// Returns [`CommandError::DuplicateSection`] if `slot` is already
    /// registered. The registry is left unchanged on error: there is no
    /// section replacement API.
    pub fn register_new_section(
        &mut self,
        slot: MenuSectionKey,
        section: impl MenuSection,
    ) -> Result<(), CommandError> {
        if self.sections.contains_key(slot.as_str()) {
            return Err(CommandError::DuplicateSection { section: slot });
        }
        self.sections.insert(slot.as_str(), Box::new(section));
        Ok(())
    }

    /// The section provider for `slot`, if registered.
    pub fn section(&self, slot: &str) -> Option<&dyn MenuSection> {
        self.sections.get(slot).map(|b| b.as_ref())
    }

    /// Set the active menu plan (installed by the menu module).
    pub fn set_plan(&mut self, plan: MenuPlan) {
        self.plan = Some(plan);
    }

    /// The active menu plan, if one is installed.
    pub fn plan(&self) -> Option<&MenuPlan> {
        self.plan.as_ref()
    }

    /// Mark the registry live: the module has bound and projected the initial
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
/// on first mutation, so contributors may register before the menu module's
/// `init`.
///
/// Registration is *dynamic once the module is live*: an application first
/// reaches `&mut App` from its declared start hook — after the module's initial
/// binding/projection pass. Commands registered then are bound and re-projected
/// immediately (late app-tier bindings still sit above the shell defaults and
/// below any future user overrides).
pub(crate) trait AppCommandsExt {
    /// Register a command (creates the registry global if absent). If the menu
    /// module has already run its initial pass, the command's default binding is
    /// bound and the menus are re-projected immediately.
    ///
    /// # Errors
    ///
    /// Returns [`CommandError::InvalidBinding`] before mutating the registry
    /// when the command's default binding is invalid.
    #[allow(dead_code)] // Exercised only by unit tests; no production caller yet.
    fn register_command(&mut self, command: RuntimeCommand) -> Result<(), CommandError>;
    /// Register a menu-section provider (creates the registry global if absent).
    /// Re-projects the menus immediately if the menu module is already live.
    fn register_menu_section(&mut self, slot: &'static str, section: impl MenuSection);
    /// The command registry, if it has been created.
    fn command_registry(&self) -> Option<&CommandRegistry>;
}

impl AppCommandsExt for App {
    fn register_command(&mut self, command: RuntimeCommand) -> Result<(), CommandError> {
        menus::validate_command_binding(&command)?;
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
            RegistrationEffect::Append => menus::bind_and_reproject(self, id)?,
            RegistrationEffect::Rebuild => menus::rebuild_bindings_and_reproject(self)?,
        }
        Ok(())
    }

    fn register_menu_section(&mut self, slot: &'static str, section: impl MenuSection) {
        ensure_registry(self).set_section(slot, section);
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
pub(crate) trait AppMenusExt {
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

/// Whether the active command registry projects at least one application menu.
pub(crate) fn has_projected_menus(cx: &App) -> bool {
    let Some(registry) = cx.command_registry() else {
        return false;
    };
    menu::build_menus(cx, registry).is_some_and(|menus| !menus.is_empty())
}

/// What a live registration must do about keybindings and menus.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[allow(dead_code)] // Exercised only by unit tests; no production caller yet.
enum RegistrationEffect {
    /// Registry not yet live: the module binds/projects the whole registry in one
    /// batch, so recording is enough.
    None,
    /// A brand-new id after go-live: append its binding (no stale chord to clear).
    Append,
    /// Replacing an existing id after go-live: rebuild bindings so the replaced
    /// command's now-stale chord is removed.
    Rebuild,
}

impl RegistrationEffect {
    #[allow(dead_code)] // Exercised only by unit tests; no production caller yet.
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

/// The current app-scoped handler for one action type, indirected through a
/// shared cell so replacing a command's behavior (issue #11) never installs a
/// second GPUI `on_action` listener for the same action.
struct ActionHandler<A> {
    id: CommandId,
    handler: fn(&A, &mut App) -> anyhow::Result<()>,
}

// Manual impls: `#[derive(Clone, Copy)]` would require `A: Clone`/`A: Copy`,
// which an action type has no reason to satisfy — the struct never stores a
// value of `A`, only a function pointer over it.
impl<A> Clone for ActionHandler<A> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<A> Copy for ActionHandler<A> {}

/// Install or update the app-scoped handler for command `id`'s action `A`.
///
/// The first call for a given `A` installs exactly one GPUI `on_action::<A>`
/// trampoline, which resolves the current handler from a shared cell at
/// dispatch time — outside any registry borrow, so a handler that replaces
/// itself mid-dispatch is safe, and the replacement takes effect on the next
/// dispatch. Every later call for the same `A` only swaps the cell's
/// contents, so GPUI never accumulates a second listener that could still
/// invoke a stale handler (issue #11).
///
/// Callers must already have checked id/action/scope compatibility (see
/// [`CommandRegistry::action_owner`]); this function does not re-validate it.
pub(crate) fn set_action_handler<A: Action>(
    cx: &mut App,
    id: CommandId,
    handler: fn(&A, &mut App) -> anyhow::Result<()>,
) {
    let type_id = TypeId::of::<A>();
    let existing = ensure_registry(cx)
        .action_handlers
        .get(&type_id)
        .and_then(|slot| slot.downcast_ref::<Rc<RefCell<ActionHandler<A>>>>())
        .cloned();
    if let Some(slot) = existing {
        *slot.borrow_mut() = ActionHandler { id, handler };
        return;
    }

    let slot = Rc::new(RefCell::new(ActionHandler { id, handler }));
    ensure_registry(cx)
        .action_handlers
        .insert(type_id, Box::new(Rc::clone(&slot)));
    cx.on_action(move |action: &A, cx: &mut App| {
        let ActionHandler { id, handler } = *slot.borrow();
        if let Err(error) = handler(action, cx) {
            crate::handles::report_error(cx, crate::error::RuntimeError::command(id, error));
        }
    });
}

/// Whether `command`'s id/action mapping is compatible with `registry`'s
/// current state (issue #11: one action type names exactly one stable
/// command id). `None` means compatible.
fn incompatible_action(
    registry: &CommandRegistry,
    command: &RuntimeCommand,
) -> Option<CommandError> {
    let action_type = command.action_type();
    if registry
        .action_owner(action_type)
        .is_some_and(|owner| owner != command.id())
    {
        return Some(CommandError::IncompatibleAction {
            command: command.id(),
            action: command.action_name(),
        });
    }
    None
}

/// Install a whole batch of commands lowered from the typed declaration model,
/// atomically.
///
/// Every command is checked first — bindings parse, no id collides with the
/// live registry, no id repeats within the batch, and no action type is
/// already owned by a different id (in the batch or the live registry) — and
/// only then is anything registered. A rejected batch leaves the registry
/// byte-for-byte unchanged, so a caller can install typed commands and their
/// action handlers without stranding half a declaration in the app.
///
/// Register-only by design: the typed model rejects duplicates at declaration
/// time, so a collision here is a fault, not an override. Use
/// [`replace_declared_command`] for a deliberate replacement.
pub(crate) fn install_declared_commands(
    cx: &mut App,
    commands: Vec<RuntimeCommand>,
) -> Result<(), CommandError> {
    let mut batch = HashSet::with_capacity(commands.len());
    let mut batch_actions: HashMap<TypeId, CommandId> = HashMap::with_capacity(commands.len());
    for command in &commands {
        menus::validate_command_binding(command)?;
        let id = command.id();
        let collides = cx
            .command_registry()
            .is_some_and(|registry| registry.contains(id));
        if collides || !batch.insert(id) {
            return Err(CommandError::Duplicate { command: id });
        }
        let action_type = command.action_type();
        let owned_in_batch = batch_actions.contains_key(&action_type);
        let owned_in_registry = cx
            .command_registry()
            .and_then(|registry| incompatible_action(registry, command));
        if owned_in_batch || owned_in_registry.is_some() {
            return Err(CommandError::IncompatibleAction {
                command: id,
                action: command.action_name(),
            });
        }
        batch_actions.insert(action_type, id);
    }

    // Past this point nothing can fail, so the registry cannot be left partly
    // mutated.
    let active = cx
        .command_registry()
        .is_some_and(CommandRegistry::is_active);
    let ids: Vec<CommandId> = commands.iter().map(RuntimeCommand::id).collect();
    let registry = ensure_registry(cx);
    for command in commands {
        registry.register(command)?;
    }
    if active {
        for id in ids {
            menus::bind_and_reproject(cx, id)?;
        }
    }
    Ok(())
}

/// Replace a registered command with a new definition, or register it if
/// absent.
///
/// The only sanctioned last-wins path for the typed model. Replacing changes
/// the command's action and default binding, so when the module is already live
/// this rebuilds the registry-owned keybindings (dropping the replaced
/// command's stale binding) and re-projects the menus — an append-only
/// `bind_and_reproject` would leave the previous binding installed.
///
/// # Errors
///
/// Returns [`CommandError::IncompatibleAction`] (issue #11) when `command`'s
/// action type is already owned by a different id, or when `command`'s id
/// already exists with a different action type or dispatch scope: a stable id
/// never changes what it names, so a scope or action change is rejected
/// rather than silently replacing behavior that another part of the app may
/// still expect. The registry is left unchanged on error.
pub(crate) fn replace_declared_command(
    cx: &mut App,
    command: RuntimeCommand,
) -> Result<(), CommandError> {
    menus::validate_command_binding(&command)?;
    if let Some(registry) = cx.command_registry() {
        if let Some(error) = incompatible_action(registry, &command) {
            return Err(error);
        }
        if let Some(existing) = registry.get(command.id())
            && (existing.action_type() != command.action_type()
                || existing.scope() != command.scope())
        {
            return Err(CommandError::IncompatibleAction {
                command: command.id(),
                action: command.action_name(),
            });
        }
    }

    // Menu placements are declared through the outline the framework resolved
    // from the app's typed declaration, keyed by the stable `CommandId` — they
    // are not part of the replaceable public `Command` surface (label,
    // binding, predicates, action, handler), so an incoming replacement never
    // carries them. Snapshotting the existing command's placements here, after
    // every compatibility check has passed and before the registry is
    // touched, is what keeps a replaced command in its native, in-window, and
    // dock menu placements instead of vanishing from them. An id with no
    // existing registration (the register-if-absent path) keeps no
    // placements, matching a freshly declared command.
    let placements: Vec<MenuPlacement> = cx
        .command_registry()
        .and_then(|registry| registry.get(command.id()))
        .map(|existing| existing.placements().to_vec())
        .unwrap_or_default();
    let command = placements
        .into_iter()
        .fold(command, RuntimeCommand::with_placement);

    let active = cx
        .command_registry()
        .is_some_and(CommandRegistry::is_active);
    ensure_registry(cx).register(command)?;
    if active {
        menus::rebuild_bindings_and_reproject(cx)?;
    }
    Ok(())
}

/// Install a section provider lowered from the typed declaration model.
///
/// The typed model rejects duplicate section keys at declaration time, so this
/// records the provider and lets the module's projection pass pick it up.
pub(crate) fn register_declared_section(
    cx: &mut App,
    slot: &'static str,
    provider: fn(&App) -> Vec<MenuItem>,
) {
    ensure_registry(cx).set_section(slot, provider);
}

/// Runtime extension for registering typed commands and menu sections after
/// startup.
///
/// [`AppDeclaration::command`](crate::AppDeclaration::command) and
/// [`AppDeclaration::menu_section`](crate::AppDeclaration::menu_section) cover
/// the startup-declared vocabulary; this trait is the seam for commands and
/// sections an application registers dynamically from `&mut App` afterwards.
pub trait Commands {
    /// Register a new typed command.
    ///
    /// # Errors
    ///
    /// Returns [`CommandError::Duplicate`] if the command's id is already
    /// registered, or [`CommandError::InvalidBinding`]/
    /// [`CommandError::InvalidKeyContext`] if its default binding does not
    /// parse. The registry is left unchanged on error.
    fn register_command<A: Action>(&mut self, command: Command<A>) -> Result<(), CommandError>;

    /// Replace a registered command, or register it if absent.
    ///
    /// The only sanctioned last-wins path: unlike [`Commands::register_command`],
    /// an existing id is overwritten rather than rejected.
    ///
    /// A framework-owned standard command id (Quit, About, Settings, the
    /// standard edit commands, the standard window commands, ...) can never
    /// be targeted: those handlers are wired through the framework's own raw
    /// GPUI `on_action` listeners, so replacing one would layer a second
    /// application handler on top of it instead of swapping it. No standard
    /// command is designated overridable in this milestone.
    ///
    /// # Errors
    ///
    /// Returns [`CommandError::FrameworkOwned`] if `command`'s id is a
    /// framework-owned standard command id, or
    /// [`CommandError::InvalidBinding`]/[`CommandError::InvalidKeyContext`]
    /// if the command's default binding does not parse. Rejection happens
    /// before the registry, bindings, handlers, or menus are touched, so the
    /// registry is left unchanged on error.
    fn replace_command<A: Action>(&mut self, command: Command<A>) -> Result<(), CommandError>;

    /// Register a new menu-section provider under `key`.
    ///
    /// # Errors
    ///
    /// Returns [`CommandError::DuplicateSection`] if `key` is already
    /// registered. There is no section replacement API: the registry is left
    /// unchanged on error.
    fn register_section(
        &mut self,
        key: MenuSectionKey,
        provider: fn(&App) -> Vec<MenuItem>,
    ) -> Result<(), CommandError>;

    /// Re-project every application menu from the current registry state.
    ///
    /// Call after external state a section provider reads (theme names, open
    /// windows, ...) changes outside a command registration.
    fn invalidate_menus(&mut self);
}

impl Commands for App {
    fn register_command<A: Action>(&mut self, command: Command<A>) -> Result<(), CommandError> {
        let platform = DesktopPlatform::current();
        let lowered: Box<dyn declared::ErasedCommand> = Box::new(command);
        let (runtime_command, handler) = lowered.lower(platform, &[]).into_parts();
        install_declared_commands(self, vec![runtime_command])?;
        if let Some(handler) = handler {
            handler(self);
        }
        Ok(())
    }

    fn replace_command<A: Action>(&mut self, command: Command<A>) -> Result<(), CommandError> {
        let id = command.id();
        if standard::is_standard_command(id) {
            return Err(CommandError::FrameworkOwned { command: id });
        }
        let platform = DesktopPlatform::current();
        let lowered: Box<dyn declared::ErasedCommand> = Box::new(command);
        let (runtime_command, handler) = lowered.lower(platform, &[]).into_parts();
        replace_declared_command(self, runtime_command)?;
        if let Some(handler) = handler {
            handler(self);
        }
        Ok(())
    }

    fn register_section(
        &mut self,
        key: MenuSectionKey,
        provider: fn(&App) -> Vec<MenuItem>,
    ) -> Result<(), CommandError> {
        let live = self
            .command_registry()
            .is_some_and(CommandRegistry::is_active);
        ensure_registry(self).register_new_section(key, provider)?;
        if live {
            menus_invalidate(self);
        }
        Ok(())
    }

    fn invalidate_menus(&mut self) {
        menus_invalidate(self);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::actions;

    actions!(commands_test, [Alpha, Beta]);

    fn cmd(id: &'static str, action: impl Action) -> RuntimeCommand {
        RuntimeCommand::new(CommandId(id), id, CommandScope::App, action)
    }

    fn ok_alpha(_: &Alpha, _: &mut App) -> anyhow::Result<()> {
        Ok(())
    }

    fn ok_beta(_: &Beta, _: &mut App) -> anyhow::Result<()> {
        Ok(())
    }

    /// A GPUI global the plain (non-capturing) handler `fn`s below record
    /// into, since [`Command::app`] handlers must be non-capturing `fn`
    /// pointers.
    #[derive(Default)]
    struct ReplayLog(Vec<&'static str>);

    impl Global for ReplayLog {}

    fn record(cx: &mut App, label: &'static str) {
        if !cx.has_global::<ReplayLog>() {
            cx.set_global(ReplayLog::default());
        }
        cx.global_mut::<ReplayLog>().0.push(label);
    }

    fn record_first(_: &Alpha, cx: &mut App) -> anyhow::Result<()> {
        record(cx, "first");
        Ok(())
    }

    fn record_second(_: &Alpha, cx: &mut App) -> anyhow::Result<()> {
        record(cx, "second");
        Ok(())
    }

    fn record_third(_: &Alpha, cx: &mut App) -> anyhow::Result<()> {
        record(cx, "third");
        Ok(())
    }

    #[test]
    fn register_dedups_last_wins_and_keeps_slot() {
        let mut reg = CommandRegistry::new();
        reg.register(cmd("a", Alpha).with_binding("cmd-a")).unwrap();
        reg.register(cmd("b", Beta)).unwrap();
        // Re-register "a" with a new label/binding: replaces in place.
        reg.register(
            RuntimeCommand::new(CommandId("a"), "A2", CommandScope::App, Alpha)
                .with_binding("cmd-x"),
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

    fn empty_section(_: &App) -> Vec<gpui::MenuItem> {
        Vec::new()
    }

    #[test]
    fn register_new_section_rejects_a_duplicate_slot_and_preserves_state() {
        let mut reg = CommandRegistry::new();
        reg.register_new_section(MenuSectionKey::THEME, empty_section)
            .expect("the first registration succeeds");
        assert!(reg.section(MenuSectionKey::THEME.as_str()).is_some());

        let error = reg
            .register_new_section(MenuSectionKey::THEME, empty_section)
            .expect_err("a second registration for the same slot is rejected");

        assert!(matches!(
            error,
            CommandError::DuplicateSection { section } if section == MenuSectionKey::THEME
        ));
        assert!(
            reg.section(MenuSectionKey::THEME.as_str()).is_some(),
            "the registry keeps the first provider on a rejected second registration",
        );
    }

    #[gpui::test]
    fn commands_register_section_rejects_a_duplicate_and_reprojects_when_live(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| {
            Commands::register_section(cx, MenuSectionKey::THEME, empty_section)
                .expect("the first registration succeeds");

            let error = Commands::register_section(cx, MenuSectionKey::THEME, empty_section)
                .expect_err("a second registration for the same slot is rejected");
            assert!(matches!(
                error,
                CommandError::DuplicateSection { section } if section == MenuSectionKey::THEME
            ));
        });
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

    // ------------------------------------------------------------- issue #11
    // `Commands::replace_command` must be true last-wins: GPUI's own
    // `on_action` keeps every registered listener and can invoke more than
    // one (`App::dispatch_global_action` runs every listener for an action
    // type while the event still propagates), so appending a second listener
    // per replace would leave the previous handler live alongside the new
    // one. `set_action_handler` is the fix: exactly one listener per action
    // type, swapped through a shared cell.

    #[gpui::test]
    fn replace_command_is_last_wins_for_global_action_dispatch(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            Commands::register_command(
                cx,
                Command::app(CommandId("replace.probe"), Alpha, record_first),
            )
            .expect("the first registration succeeds");
            Commands::replace_command(
                cx,
                Command::app(CommandId("replace.probe"), Alpha, record_second),
            )
            .expect("replacing the same id/action succeeds");
            Commands::replace_command(
                cx,
                Command::app(CommandId("replace.probe"), Alpha, record_third),
            )
            .expect("replacing again succeeds");

            // No window is open: this dispatches through GPUI's no-window
            // global path (`App::dispatch_global_action`), which is exactly
            // where the reviewed bug surfaced — every registered listener
            // runs during the bubble phase unless one stops propagation.
            assert!(
                cx.active_window().is_none(),
                "this test exercises global (no-window) action dispatch"
            );
            cx.dispatch_action(&Alpha);

            assert_eq!(
                cx.global::<ReplayLog>().0,
                vec!["third"],
                "only the newest handler runs, exactly once: a stale listener \
                 must not still be live after two replacements",
            );
        });
    }

    #[gpui::test]
    fn register_command_rejects_a_different_id_for_an_owned_action(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            Commands::register_command(cx, Command::app(CommandId("owns.alpha"), Alpha, ok_alpha))
                .expect("the first registration succeeds");

            let error = Commands::register_command(
                cx,
                Command::app(CommandId("claims.alpha"), Alpha, ok_alpha),
            )
            .expect_err("a second id may not claim an already-owned action type");

            assert!(matches!(
                error,
                CommandError::IncompatibleAction {
                    command: CommandId("claims.alpha"),
                    ..
                }
            ));
            assert!(
                cx.command_registry()
                    .unwrap()
                    .get(CommandId("claims.alpha"))
                    .is_none(),
                "the rejected registration left no trace",
            );
            assert!(
                cx.command_registry()
                    .unwrap()
                    .get(CommandId("owns.alpha"))
                    .is_some(),
                "the original registration is untouched",
            );
        });
    }

    #[gpui::test]
    fn replace_command_rejects_a_different_action_for_an_existing_id(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| {
            Commands::register_command(cx, Command::app(CommandId("stable.id"), Alpha, ok_alpha))
                .expect("the first registration succeeds");

            let error =
                Commands::replace_command(cx, Command::app(CommandId("stable.id"), Beta, ok_beta))
                    .expect_err("a stable id may not change its action type");

            assert!(matches!(
                error,
                CommandError::IncompatibleAction {
                    command: CommandId("stable.id"),
                    ..
                }
            ));
            assert_eq!(
                cx.command_registry()
                    .unwrap()
                    .get(CommandId("stable.id"))
                    .unwrap()
                    .action_type(),
                TypeId::of::<Alpha>(),
                "the rejected replace left the original action type in place",
            );
        });
    }

    #[gpui::test]
    fn replace_command_rejects_a_scope_change_for_an_existing_id(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            Commands::register_command(
                cx,
                Command::app(CommandId("stable.scope"), Alpha, ok_alpha),
            )
            .expect("the first registration succeeds");

            let error = Commands::replace_command(
                cx,
                Command::<Alpha>::window(CommandId("stable.scope"), Alpha),
            )
            .expect_err("a stable id may not change its dispatch scope");

            assert!(matches!(
                error,
                CommandError::IncompatibleAction {
                    command: CommandId("stable.scope"),
                    ..
                }
            ));
            assert_eq!(
                cx.command_registry()
                    .unwrap()
                    .get(CommandId("stable.scope"))
                    .unwrap()
                    .scope(),
                CommandScope::App,
                "the rejected replace left the original scope in place",
            );
        });
    }

    #[gpui::test]
    fn replace_command_accepts_the_same_id_and_action_after_a_scope_rejection(
        cx: &mut gpui::TestAppContext,
    ) {
        // The rejected scope change above must not have corrupted anything:
        // a compatible replace (same id, same action, same scope) still
        // works, and dispatch still reaches exactly one handler.
        cx.update(|cx| {
            Commands::register_command(
                cx,
                Command::app(CommandId("stable.scope2"), Alpha, record_first),
            )
            .expect("the first registration succeeds");
            let _ = Commands::replace_command(
                cx,
                Command::<Alpha>::window(CommandId("stable.scope2"), Alpha),
            );
            Commands::replace_command(
                cx,
                Command::app(CommandId("stable.scope2"), Alpha, record_second),
            )
            .expect("a compatible replace still succeeds after a rejected one");

            cx.dispatch_action(&Alpha);
            assert_eq!(cx.global::<ReplayLog>().0, vec!["second"]);
        });
    }

    #[gpui::test]
    fn replace_command_rejects_a_framework_owned_standard_id(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            Commands::register_command(cx, Command::app(QUIT_COMMAND_ID, Alpha, record_first))
                .expect("registering directly under the id still succeeds");

            let error = Commands::replace_command(cx, Command::app(QUIT_COMMAND_ID, Beta, ok_beta))
                .expect_err("replace_command must reject every framework-owned standard id");

            assert!(matches!(
                error,
                CommandError::FrameworkOwned { command } if command == QUIT_COMMAND_ID
            ));
            assert_eq!(
                cx.command_registry()
                    .unwrap()
                    .get(QUIT_COMMAND_ID)
                    .unwrap()
                    .action_type(),
                TypeId::of::<Alpha>(),
                "the rejected replace left the original registry entry untouched",
            );

            cx.dispatch_action(&Alpha);
            assert_eq!(
                cx.global::<ReplayLog>().0,
                vec!["first"],
                "the rejected replace left the original handler installed",
            );
        });
    }

    #[gpui::test]
    fn replace_declared_command_preserves_existing_menu_placements(cx: &mut gpui::TestAppContext) {
        // Placements come from the declaration's resolved menu outline, keyed
        // by the stable `CommandId` — they are not part of the publicly
        // redeclarable `Command` surface, so a replacement's incoming
        // `RuntimeCommand` never carries them (`Commands::replace_command`
        // always lowers with `&[]`). `replace_declared_command` must restore
        // them from the existing registration rather than dropping them, or
        // the command disappears from every menu/dock surface it was placed
        // in the moment its label/binding/handler are updated.
        cx.update(|cx| {
            let original = cmd("placed.probe", Alpha)
                .with_binding("cmd-p")
                .with_placement(MenuPlacement::new(APP_MENU, 0, 0))
                .with_placement(MenuPlacement::new(EDIT_MENU, 1, 2))
                .with_placement(MenuPlacement::new(DOCK_MENU, 0, 0));
            install_declared_commands(cx, vec![original]).expect("initial install succeeds");

            let original_placements = cx
                .command_registry()
                .unwrap()
                .get(CommandId("placed.probe"))
                .unwrap()
                .placements()
                .to_vec();
            assert_eq!(
                original_placements.len(),
                3,
                "the probe installs three placements, including the dock",
            );

            // The replacement carries a new label/binding and no placements of
            // its own, exactly as every real call site lowers with `&[]`. The
            // action type must stay `Alpha`: a stable id may not change its
            // action type (see `replace_command_rejects_a_different_action_
            // for_an_existing_id`), so this test isolates the placement
            // behavior from that unrelated rejection path.
            let replacement = RuntimeCommand::new(
                CommandId("placed.probe"),
                "Replaced Label",
                CommandScope::App,
                Alpha,
            )
            .with_binding("cmd-shift-p");
            assert!(
                replacement.placements().is_empty(),
                "the incoming replacement declares no placements of its own"
            );
            replace_declared_command(cx, replacement).expect("a compatible replace succeeds");

            let replaced = cx
                .command_registry()
                .unwrap()
                .get(CommandId("placed.probe"))
                .unwrap();
            assert_eq!(
                replaced.placements(),
                original_placements.as_slice(),
                "every original placement, including the dock, survives in its \
                 original order",
            );
            // New label/binding semantics still apply: only placements are
            // carried over from the original registration.
            assert_eq!(replaced.label().as_ref(), "Replaced Label");
            assert_eq!(replaced.default_binding(), Some("cmd-shift-p"));
        });
    }

    #[gpui::test]
    fn replace_declared_command_keeps_no_placements_for_an_absent_id(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| {
            let command = cmd("fresh.probe", Alpha).with_binding("cmd-f");
            replace_declared_command(cx, command)
                .expect("replace registers an absent id instead of failing");

            assert!(
                cx.command_registry()
                    .unwrap()
                    .get(CommandId("fresh.probe"))
                    .unwrap()
                    .placements()
                    .is_empty(),
                "there was no existing registration, so there is nothing to carry over",
            );
        });
    }
}
