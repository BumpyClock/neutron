//! Typed menus, menu-bar policy, and the pure platform outline (issue #11).
//!
//! Commands are semantic; menus are one *projection* of command identities. A
//! [`Menu`] therefore holds only ordered references — command ids,
//! separators, and section keys — and never a handler, so the same command can
//! appear in several menus without a second behavior declaration.
//!
//! [`MenuBar::outline`] is pure: given a [`DesktopPlatform`] it resolves the
//! standard layout, applies the declared edits, injects the mandatory native
//! nodes (macOS Services), and returns the same [`MenuOutline`] on every host.
//! Nothing here touches `App`, a window, or `cfg!(target_os = ...)`.

use std::collections::HashSet;

use crate::declaration::{DeclarationError, DeclarationErrors};

use super::CommandId;
use super::declaration::StandardFeatures;
use super::faults::CommandFault;
use super::keys::{MenuKey, MenuSectionKey};
use super::label::MenuLabel;
use super::standard::{self, DesktopPlatform};

/// One ordered node inside a typed menu.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MenuNode {
    /// A command, referenced by identity.
    Command(CommandId),
    /// A separator between groups.
    Separator,
    /// A reserved slot filled at runtime by the section's declared provider.
    Section(MenuSectionKey),
    /// The macOS system-managed Services menu. Framework-owned: applications
    /// cannot declare or remove it.
    Services,
}

/// One declared top-level menu.
pub struct Menu {
    key: MenuKey,
    label: MenuLabel,
    nodes: Vec<MenuNode>,
}

impl Menu {
    /// Start a menu with its key and title.
    #[must_use]
    pub fn new(key: MenuKey, label: impl Into<MenuLabel>) -> Self {
        Self {
            key,
            label: label.into(),
            nodes: Vec::new(),
        }
    }

    /// A menu titled with its own key, the common case for `Edit`/`Window`.
    #[must_use]
    pub fn keyed(key: MenuKey) -> Self {
        Self::new(key, MenuLabel::text(key.as_str()))
    }

    /// Append a command reference.
    #[must_use]
    pub fn command(mut self, id: CommandId) -> Self {
        self.nodes.push(MenuNode::Command(id));
        self
    }

    /// Append a separator.
    #[must_use]
    pub fn separator(mut self) -> Self {
        self.nodes.push(MenuNode::Separator);
        self
    }

    /// Append a reserved section slot.
    #[must_use]
    pub fn section(mut self, key: MenuSectionKey) -> Self {
        self.nodes.push(MenuNode::Section(key));
        self
    }

    /// Append the macOS Services node. Framework-internal: only the standard
    /// layout and the mandatory-node injection use it.
    #[must_use]
    fn services(mut self) -> Self {
        self.nodes.push(MenuNode::Services);
        self
    }

    /// This menu's key.
    pub fn key(&self) -> MenuKey {
        self.key
    }
}

/// The resolved, host-independent structure of one top-level menu.
#[derive(Debug)]
pub struct MenuOutlineEntry {
    key: MenuKey,
    label: MenuLabel,
    nodes: Vec<MenuNode>,
}

impl MenuOutlineEntry {
    /// The menu key.
    pub fn key(&self) -> MenuKey {
        self.key
    }

    /// The menu title, resolved against app state at projection time.
    pub fn label(&self) -> &MenuLabel {
        &self.label
    }

    /// The ordered nodes.
    pub fn nodes(&self) -> &[MenuNode] {
        &self.nodes
    }
}

/// The resolved menu bar for one platform.
///
/// Contains top-level keys, ordered command references, separators, section
/// keys, and required native nodes — and no `App`, window, or host dependency,
/// so native and in-window projections consume the same value.
#[derive(Debug)]
pub struct MenuOutline {
    menus: Vec<MenuOutlineEntry>,
}

impl MenuOutline {
    /// The ordered top-level menus.
    pub fn menus(&self) -> &[MenuOutlineEntry] {
        &self.menus
    }

    /// Every command referenced anywhere in the outline, in outline order.
    pub fn command_ids(&self) -> Vec<CommandId> {
        self.menus
            .iter()
            .flat_map(|menu| menu.nodes.iter())
            .filter_map(|node| match node {
                MenuNode::Command(id) => Some(*id),
                _ => None,
            })
            .collect()
    }

    /// Every section slot reserved anywhere in the outline, with its menu.
    pub fn section_slots(&self) -> Vec<(MenuKey, MenuSectionKey)> {
        self.menus
            .iter()
            .flat_map(|menu| {
                menu.nodes.iter().filter_map(move |node| match node {
                    MenuNode::Section(section) => Some((menu.key, *section)),
                    _ => None,
                })
            })
            .collect()
    }
}

/// Which standard menus an application may hide. The application block, Edit,
/// and Window carry framework-required behavior and are not removable.
const OPTIONAL_STANDARD_MENUS: [MenuKey; 2] = [MenuKey::VIEW, MenuKey::HELP];

/// The framework-owned standard layout plus the application's explicit edits.
#[derive(Default)]
pub struct StandardLayout {
    settings: bool,
    about: bool,
    theme_section: bool,
    hidden: Vec<MenuKey>,
    custom: Vec<Menu>,
    contributions: Vec<Menu>,
}

impl StandardLayout {
    /// The bare standard layout: no Settings surface, no About, no Appearance.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Include the standard Settings command. Present only when the application
    /// declares a Settings surface.
    #[must_use]
    #[allow(dead_code)] // Exercised only by unit tests; no production caller yet.
    pub(crate) fn with_settings(mut self) -> Self {
        self.settings = true;
        self
    }

    /// Include the standard About command.
    #[must_use]
    #[allow(dead_code)] // Exercised only by unit tests; no production caller yet.
    pub(crate) fn with_about(mut self) -> Self {
        self.about = true;
        self
    }

    /// Reserve the Appearance/Theme section in its platform-conventional menu.
    #[must_use]
    #[allow(dead_code)] // Exercised only by unit tests; no production caller yet.
    pub(crate) fn with_theme_section(mut self) -> Self {
        self.theme_section = true;
        self
    }

    /// Hide an optional standard menu. Hiding a required menu is a declaration
    /// fault, reported by [`MenuBar::outline`].
    #[must_use]
    pub(crate) fn hide(mut self, key: MenuKey) -> Self {
        self.hidden.push(key);
        self
    }

    /// Insert an application menu before the Window menu.
    #[must_use]
    pub(crate) fn insert(mut self, menu: Menu) -> Self {
        self.custom.push(menu);
        self
    }

    /// Contribute ordered nodes to the standard menu keyed by `menu.key()`.
    ///
    /// See [`MenuBar::contribute`] for the full merge behavior.
    #[must_use]
    pub(crate) fn contribute(mut self, menu: Menu) -> Self {
        self.contributions.push(menu);
        self
    }

    /// Resolve the standard menus for `platform`, appending any faults.
    fn resolve(
        &self,
        platform: DesktopPlatform,
        features: StandardFeatures,
        faults: &mut Vec<CommandFault>,
    ) -> Vec<Menu> {
        let flags = self.flags(features);
        for key in &self.hidden {
            if !OPTIONAL_STANDARD_MENUS.contains(key) {
                faults.push(CommandFault::InvalidStandardEdit {
                    menu: *key,
                    reason: "menu is required by the framework and cannot be hidden",
                });
                // A required menu is never actually hidden, so it cannot strand
                // anything: reporting both faults would blame one edit twice.
                continue;
            }
            for feature in stranded_features(*key, platform, flags) {
                faults.push(CommandFault::StrandedStandardFeature {
                    menu: *key,
                    feature,
                });
            }
        }

        let standard_keys = standard::standard_menu_keys(platform, flags.theme);
        let mut menus = Vec::new();
        for &key in &standard_keys {
            if key == MenuKey::WINDOW {
                menus.extend(self.custom_menus());
                menus.extend(self.contributed_top_level_menus(&standard_keys));
            }
            if self.hidden.contains(&key) {
                continue;
            }
            let menu = self.standard_menu(key, platform, flags);
            menus.push(self.merge_contributions(key, menu));
        }
        menus
    }

    /// Append every contribution declared against `key`, in declaration
    /// order. Each contributed block is separated from the standard content
    /// and from the previous contributed block by one separator;
    /// [`normalize`] collapses the separator away when the block it would
    /// separate from is empty, so an all-optional-features-off standard menu
    /// receiving its first contribution never gains a stray leading
    /// separator.
    fn merge_contributions(&self, key: MenuKey, mut menu: Menu) -> Menu {
        for contribution in &self.contributions {
            if contribution.key != key {
                continue;
            }
            menu = menu.separator();
            menu.nodes.extend(contribution.nodes.iter().copied());
        }
        menu
    }

    /// Contributions whose key names no standard menu on this platform (for
    /// example Help on macOS): each becomes its own top-level menu, inserted
    /// before Window in first-contributed order, with the declared label of
    /// its first contribution. Later contributions to the same absent key
    /// merge into that one menu instead of creating a duplicate top-level
    /// key, exactly as [`Self::merge_contributions`] merges into an existing
    /// standard menu.
    fn contributed_top_level_menus(&self, standard_keys: &[MenuKey]) -> Vec<Menu> {
        let mut menus: Vec<Menu> = Vec::new();
        for contribution in &self.contributions {
            if standard_keys.contains(&contribution.key) {
                continue;
            }
            match menus.iter_mut().find(|menu| menu.key == contribution.key) {
                Some(existing) => {
                    existing.nodes.push(MenuNode::Separator);
                    existing.nodes.extend(contribution.nodes.iter().copied());
                }
                None => menus.push(Menu {
                    key: contribution.key,
                    label: contribution.label.clone(),
                    nodes: contribution.nodes.clone(),
                }),
            }
        }
        menus
    }

    /// Merge the layout's explicit edits with the features the declaration
    /// resolved.
    ///
    /// A union rather than an override: `with_about()` on the layout and a
    /// declared About surface both mean "About belongs in the menu bar", and
    /// neither can cancel the other.
    fn flags(&self, features: StandardFeatures) -> LayoutFlags {
        LayoutFlags {
            settings: self.settings || features.has_settings(),
            about: self.about || features.has_about(),
            theme: self.theme_section || features.theme,
        }
    }

    /// Custom menus are re-declared per resolution because `Menu` owns
    /// its nodes; the layout keeps the declaration and hands out copies.
    fn custom_menus(&self) -> Vec<Menu> {
        self.custom
            .iter()
            .map(|menu| Menu {
                key: menu.key,
                label: menu.label.clone(),
                nodes: menu.nodes.clone(),
            })
            .collect()
    }

    fn standard_menu(&self, key: MenuKey, platform: DesktopPlatform, flags: LayoutFlags) -> Menu {
        let app_block = standard::app_menu_key(platform) == key.as_str();
        let mut menu = if key == MenuKey::APP {
            // The application menu renders the app display name on macOS.
            Menu::new(key, MenuLabel::derived(standard::app_display_name))
        } else {
            Menu::keyed(key)
        };

        if app_block && flags.about && platform == DesktopPlatform::MacOs {
            menu = menu.command(standard::ABOUT_COMMAND_ID).separator();
        }
        if app_block && flags.settings {
            menu = menu.command(standard::OPEN_SETTINGS_COMMAND_ID).separator();
        }
        if flags.theme && key == standard::theme_section_menu(platform) {
            menu = menu.section(MenuSectionKey::THEME).separator();
        }
        if platform == DesktopPlatform::MacOs && key == MenuKey::APP {
            menu = menu
                .services()
                .separator()
                .command(standard::HIDE_APP_COMMAND_ID)
                .command(standard::HIDE_OTHERS_COMMAND_ID)
                .command(standard::SHOW_ALL_COMMAND_ID)
                .separator();
        }
        if app_block {
            menu = menu.command(standard::QUIT_COMMAND_ID);
        }
        if key == MenuKey::EDIT {
            menu = menu
                .command(standard::UNDO_COMMAND_ID)
                .command(standard::REDO_COMMAND_ID)
                .separator()
                .command(standard::CUT_COMMAND_ID)
                .command(standard::COPY_COMMAND_ID)
                .command(standard::PASTE_COMMAND_ID)
                .separator()
                .command(standard::DELETE_COMMAND_ID)
                .command(standard::DELETE_PREVIOUS_WORD_COMMAND_ID)
                .command(standard::DELETE_NEXT_WORD_COMMAND_ID)
                .separator()
                .command(standard::FIND_COMMAND_ID)
                .separator()
                .command(standard::SELECT_ALL_COMMAND_ID);
        }
        if key == MenuKey::WINDOW {
            if platform == DesktopPlatform::MacOs {
                menu = menu
                    .command(standard::MINIMIZE_COMMAND_ID)
                    .command(standard::ZOOM_COMMAND_ID)
                    .separator();
            }
            menu = menu.command(standard::CLOSE_WINDOW_COMMAND_ID);
        }
        if key == MenuKey::HELP && flags.about && platform != DesktopPlatform::MacOs {
            menu = menu.command(standard::ABOUT_COMMAND_ID);
        }
        menu
    }
}

/// The enabled standard features `platform` places in `menu`, in the order the
/// standard layout would have rendered them.
///
/// Settings is deliberately absent: the platform always places it in the
/// application menu (macOS) or File (Windows/Linux), both of which are required
/// and therefore already unhideable.
fn stranded_features(
    menu: MenuKey,
    platform: DesktopPlatform,
    flags: LayoutFlags,
) -> Vec<&'static str> {
    let mut stranded = Vec::new();
    if flags.theme && standard::theme_section_menu(platform) == menu {
        stranded.push("Appearance");
    }
    if flags.about && standard::about_menu_key(platform) == menu.as_str() {
        stranded.push("About");
    }
    stranded
}

/// The standard-menu content flags, after merging the layout's explicit edits
/// with the declaration's resolved features.
#[derive(Clone, Copy)]
struct LayoutFlags {
    settings: bool,
    about: bool,
    theme: bool,
}

/// The application's menu-bar policy.
///
/// Construct with [`MenuBar::standard`], [`MenuBar::custom`], or
/// [`MenuBar::none`]. [`MenuBar::hide`], [`MenuBar::insert`], and
/// [`MenuBar::contribute`] edit the standard layout only; calling any of them
/// while the policy is [`MenuBar::custom`] or [`MenuBar::none`] does not
/// panic, but is recorded as a declaration fault that [`MenuBar::outline`]
/// reports, since there is no standard layout there to edit.
pub struct MenuBar {
    policy: MenuBarPolicy,
    /// Edits recorded against the wrong policy (see the type docs), folded
    /// into the aggregate at [`MenuBar::outline`] time.
    edit_faults: Vec<CommandFault>,
}

/// The chosen menu-bar policy, private so every construction and edit passes
/// through [`MenuBar`]'s validated seam.
enum MenuBarPolicy {
    /// The platform-conventional standard layout plus explicit edits.
    Standard(StandardLayout),
    /// An application-owned layout. AppShell still injects the mandatory native
    /// system nodes.
    Custom(Vec<Menu>),
    /// No native or in-window menu projection.
    None,
}

impl MenuBar {
    fn from_policy(policy: MenuBarPolicy) -> Self {
        Self {
            policy,
            edit_faults: Vec::new(),
        }
    }

    /// Construct directly from a resolved [`StandardLayout`]. Test-only: real
    /// declarations build edits through [`MenuBar::hide`]/[`MenuBar::insert`].
    #[cfg(test)]
    pub(crate) fn from_standard_layout(layout: StandardLayout) -> Self {
        Self::from_policy(MenuBarPolicy::Standard(layout))
    }

    /// The default policy: the standard layout with no optional surfaces.
    #[must_use]
    pub fn standard() -> Self {
        Self::from_policy(MenuBarPolicy::Standard(StandardLayout::new()))
    }

    /// An application-owned menu bar.
    #[must_use]
    pub fn custom(menus: Vec<Menu>) -> Self {
        Self::from_policy(MenuBarPolicy::Custom(menus))
    }

    /// No native or in-window menu projection.
    #[must_use]
    pub fn none() -> Self {
        Self::from_policy(MenuBarPolicy::None)
    }

    /// Hide an optional standard menu. Hiding a required menu is a declaration
    /// fault, reported by [`MenuBar::outline`]. Calling this when the policy
    /// is not [`MenuBar::standard`] is also a declaration fault: there is no
    /// standard layout here to edit.
    #[must_use]
    pub fn hide(mut self, key: MenuKey) -> Self {
        match self.policy {
            MenuBarPolicy::Standard(layout) => {
                self.policy = MenuBarPolicy::Standard(layout.hide(key));
            }
            MenuBarPolicy::Custom(_) | MenuBarPolicy::None => {
                self.edit_faults.push(CommandFault::InvalidStandardEdit {
                    menu: key,
                    reason: "hide only edits the standard menu-bar layout",
                });
            }
        }
        self
    }

    /// Insert an application menu before the Window menu, as a new top-level
    /// key. For adding to a menu the standard layout already provides (for
    /// example placing an app-owned command in Help alongside the standard
    /// About command), use [`MenuBar::contribute`] instead: inserting the
    /// same key here is a duplicate top-level menu, reported by
    /// [`MenuBar::outline`]. Same restriction as [`MenuBar::hide`].
    #[must_use]
    pub fn insert(mut self, menu: Menu) -> Self {
        match self.policy {
            MenuBarPolicy::Standard(layout) => {
                self.policy = MenuBarPolicy::Standard(layout.insert(menu));
            }
            MenuBarPolicy::Custom(_) | MenuBarPolicy::None => {
                self.edit_faults.push(CommandFault::InvalidStandardEdit {
                    menu: menu.key(),
                    reason: "insert only edits the standard menu-bar layout",
                });
            }
        }
        self
    }

    /// Contribute ordered command/separator/section nodes to a standard menu,
    /// identified by `menu.key()`.
    ///
    /// If the platform's standard layout already has a menu with that key
    /// (for example Help on Windows and Linux), the contributed nodes are
    /// appended to it, separated from the standard content by one separator;
    /// the standard menu's framework label is kept. If the platform's
    /// standard layout has no such menu (for example Help on macOS, where the
    /// standard layout never places a Help menu), the contribution is instead
    /// inserted as a new top-level menu before Window, using its own declared
    /// label — exactly like [`MenuBar::insert`], except that a later
    /// contribution to the same absent key merges into that menu instead of
    /// producing a duplicate top-level key.
    ///
    /// Several contributions to one key merge in declaration order,
    /// whichever branch applies. Same restriction as [`MenuBar::hide`]:
    /// calling this when the policy is not [`MenuBar::standard`] is a
    /// declaration fault, reported by [`MenuBar::outline`].
    #[must_use]
    pub fn contribute(mut self, menu: Menu) -> Self {
        match self.policy {
            MenuBarPolicy::Standard(layout) => {
                self.policy = MenuBarPolicy::Standard(layout.contribute(menu));
            }
            MenuBarPolicy::Custom(_) | MenuBarPolicy::None => {
                self.edit_faults.push(CommandFault::InvalidStandardEdit {
                    menu: menu.key(),
                    reason: "contribute only edits the standard menu-bar layout",
                });
            }
        }
        self
    }

    /// Resolve this policy into a pure, host-independent outline.
    ///
    /// # Errors
    ///
    /// Returns every structural fault in declaration order — rejected
    /// standard edits, invalid menu identities, duplicate top-level keys, and
    /// duplicate section slots — as [`DeclarationError::Command`]. Reference
    /// faults that need the declared command and provider sets are checked by
    /// [`validate_references`](Self::validate_references).
    pub fn outline(&self, platform: DesktopPlatform) -> Result<MenuOutline, DeclarationErrors> {
        self.outline_with(platform, StandardFeatures::default())
            .map_err(|faults| {
                DeclarationErrors::new(
                    faults
                        .into_iter()
                        .map(|fault| DeclarationError::Command { fault })
                        .collect(),
                )
                .expect("outline_with only returns Err with at least one fault")
            })
    }

    /// Resolve this policy for `platform` with the standard features the
    /// declaration resolved.
    ///
    /// # Errors
    ///
    /// Same faults as [`MenuBar::outline`], in the command model's own
    /// vocabulary.
    pub(crate) fn outline_with(
        &self,
        platform: DesktopPlatform,
        features: StandardFeatures,
    ) -> Result<MenuOutline, Vec<CommandFault>> {
        let mut faults = self.edit_faults.clone();
        let menus = match &self.policy {
            MenuBarPolicy::None => Vec::new(),
            MenuBarPolicy::Standard(layout) => layout.resolve(platform, features, &mut faults),
            MenuBarPolicy::Custom(menus) => {
                let mut resolved: Vec<Menu> = menus
                    .iter()
                    .map(|menu| Menu {
                        key: menu.key,
                        label: menu.label.clone(),
                        nodes: menu.nodes.clone(),
                    })
                    .collect();
                inject_mandatory_nodes(&mut resolved, platform);
                resolved
            }
        };

        let mut seen_menus = HashSet::new();
        let mut seen_sections = HashSet::new();
        let mut entries = Vec::new();
        for menu in menus {
            if !seen_menus.insert(menu.key) {
                faults.push(CommandFault::DuplicateMenuKey { menu: menu.key });
                continue;
            }
            if let Err(fault) = MenuKey::new(menu.key.as_str()) {
                faults.push(CommandFault::InvalidMenuKey {
                    raw: menu.key.as_str(),
                    fault,
                });
            }
            // Command uniqueness is scoped to the menu: the same command may
            // appear in several menus (and in the dock), but twice in one menu
            // would render the same entry twice.
            let mut seen_commands = HashSet::new();
            for node in &menu.nodes {
                match node {
                    MenuNode::Command(command) => {
                        if !seen_commands.insert(*command) {
                            faults.push(CommandFault::RepeatedCommandInMenu {
                                menu: menu.key,
                                command: *command,
                            });
                        }
                    }
                    MenuNode::Section(section) => {
                        if menu.key.is_dock() {
                            // The dock projection is a flat command list with no
                            // section machinery: a slot there could never be
                            // filled, so reject it here instead of silently
                            // dropping it.
                            faults.push(CommandFault::SectionInDock { section: *section });
                        } else if !seen_sections.insert(*section) {
                            faults.push(CommandFault::DuplicateSectionSlot { section: *section });
                        }
                    }
                    MenuNode::Separator | MenuNode::Services => {}
                }
            }
            let nodes = normalize(&menu.nodes);
            if nodes.is_empty() {
                // A statically empty menu can never gain content: the whole
                // declaration is known here.
                continue;
            }
            entries.push(MenuOutlineEntry {
                key: menu.key,
                label: menu.label,
                nodes,
            });
        }

        if faults.is_empty() {
            Ok(MenuOutline { menus: entries })
        } else {
            Err(faults)
        }
    }

    /// Check every reference in the outline against the declared command ids and
    /// section providers, appending one fault per dangling reference.
    pub(crate) fn validate_references(
        outline: &MenuOutline,
        commands: &HashSet<CommandId>,
        sections: &HashSet<MenuSectionKey>,
        faults: &mut Vec<CommandFault>,
    ) {
        for menu in &outline.menus {
            for node in &menu.nodes {
                match node {
                    MenuNode::Command(id) if !commands.contains(id) => {
                        faults.push(CommandFault::UnknownCommand {
                            menu: menu.key,
                            command: *id,
                        });
                    }
                    MenuNode::Section(section) if !sections.contains(section) => {
                        faults.push(CommandFault::MissingSectionProvider {
                            menu: menu.key,
                            section: *section,
                        });
                    }
                    _ => {}
                }
            }
        }
    }
}

/// Inject the framework-required native nodes an application layout cannot
/// declare for itself.
///
/// On macOS the *first* top-level menu is the application menu whatever the
/// application chose to key it, so Services is injected there. A layout that
/// keys its application block `App` still wins the lookup even if it is not
/// first, which keeps the standard layout and a reordered custom layout
/// agreeing. Only a completely empty menu bar has no valid home for the node.
fn inject_mandatory_nodes(menus: &mut [Menu], platform: DesktopPlatform) {
    if platform != DesktopPlatform::MacOs {
        return;
    }
    // The Services menu belongs to the application menu. When a custom layout
    // omits it, fall back to the first *non-dock* menu: the dock projection is a
    // flat command list with no Services machinery, so injecting there would
    // produce a node that can never render.
    let app_menu = match menus.iter().position(|menu| menu.key == MenuKey::APP) {
        Some(ix) => &mut menus[ix],
        None => match menus.iter().position(|menu| !menu.key.is_dock()) {
            Some(ix) => &mut menus[ix],
            // A dock-only (or empty) bar has nowhere to host Services.
            None => return,
        },
    };
    if app_menu.nodes.contains(&MenuNode::Services) {
        return;
    }
    if !app_menu.nodes.is_empty() {
        app_menu.nodes.push(MenuNode::Separator);
    }
    app_menu.nodes.push(MenuNode::Services);
}

/// Drop leading, trailing, and doubled separators so the outline never encodes a
/// visually broken menu. Sections are kept even when a provider may return no
/// items; the runtime already collapses an empty section's separator.
fn normalize(nodes: &[MenuNode]) -> Vec<MenuNode> {
    let mut out: Vec<MenuNode> = Vec::with_capacity(nodes.len());
    for node in nodes {
        if *node == MenuNode::Separator
            && (out.is_empty() || out.last() == Some(&MenuNode::Separator))
        {
            continue;
        }
        out.push(*node);
    }
    while out.last() == Some(&MenuNode::Separator) {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(outline: &MenuOutline) -> Vec<&'static str> {
        outline
            .menus()
            .iter()
            .map(|menu| menu.key().as_str())
            .collect()
    }

    fn nodes(outline: &MenuOutline, key: MenuKey) -> &[MenuNode] {
        outline
            .menus()
            .iter()
            .find(|menu| menu.key() == key)
            .unwrap_or_else(|| panic!("outline contains `{key}`"))
            .nodes()
    }

    fn full_standard() -> StandardLayout {
        StandardLayout::new()
            .with_settings()
            .with_about()
            .with_theme_section()
    }

    /// Unwrap the command faults out of a public [`MenuBar::outline`] error,
    /// asserting every reported fault is a structural command/menu fault (the
    /// only kind [`MenuBar::outline`] can produce).
    fn command_faults(errors: DeclarationErrors) -> Vec<CommandFault> {
        errors
            .iter()
            .map(|error| match error {
                DeclarationError::Command { fault } => *fault,
                other => panic!("expected a command fault, got {other}"),
            })
            .collect()
    }

    #[test]
    fn macos_standard_outline_follows_platform_convention() {
        let outline = MenuBar::from_standard_layout(full_standard())
            .outline(DesktopPlatform::MacOs)
            .expect("the standard layout is valid");

        assert_eq!(keys(&outline), vec!["App", "Edit", "Window"]);
        assert_eq!(
            nodes(&outline, MenuKey::APP),
            [
                MenuNode::Command(standard::ABOUT_COMMAND_ID),
                MenuNode::Separator,
                MenuNode::Command(standard::OPEN_SETTINGS_COMMAND_ID),
                MenuNode::Separator,
                MenuNode::Section(MenuSectionKey::THEME),
                MenuNode::Separator,
                MenuNode::Services,
                MenuNode::Separator,
                MenuNode::Command(standard::HIDE_APP_COMMAND_ID),
                MenuNode::Command(standard::HIDE_OTHERS_COMMAND_ID),
                MenuNode::Command(standard::SHOW_ALL_COMMAND_ID),
                MenuNode::Separator,
                MenuNode::Command(standard::QUIT_COMMAND_ID),
            ],
        );
        assert_eq!(
            nodes(&outline, MenuKey::WINDOW),
            [
                MenuNode::Command(standard::MINIMIZE_COMMAND_ID),
                MenuNode::Command(standard::ZOOM_COMMAND_ID),
                MenuNode::Separator,
                MenuNode::Command(standard::CLOSE_WINDOW_COMMAND_ID),
            ],
        );
    }

    /// The public consuming path: `hide`/`insert` on `MenuBar::standard()`
    /// edit the wrapped standard layout exactly as building a `StandardLayout`
    /// directly would.
    #[test]
    fn hide_and_insert_edit_the_standard_layout_through_menu_bar() {
        let outline = MenuBar::standard()
            .hide(MenuKey::VIEW)
            .insert(
                Menu::new(MenuKey::new("Tools").unwrap(), "Tools").command(CommandId("tools.run")),
            )
            .outline(DesktopPlatform::Linux)
            .expect("hiding an optional menu and inserting a custom one are both valid edits");

        assert_eq!(
            keys(&outline),
            vec!["File", "Edit", "Tools", "Window"],
            "View is hidden and Tools is inserted before Window",
        );
    }

    /// The motivating case (tsq-11.5.8): an app-owned `Open Repository`
    /// command contributed to Help must land next to the standard content on
    /// every platform, without ever duplicating the Help key — merged into
    /// the existing Help menu on Windows/Linux, and as a new top-level Help
    /// menu (before Window) on macOS, where the standard layout never places
    /// one.
    #[test]
    fn contribute_places_an_app_command_in_help_on_every_platform() {
        let repository = CommandId("app.open_repository");
        for platform in DesktopPlatform::ALL {
            let outline = MenuBar::from_standard_layout(
                full_standard().contribute(Menu::keyed(MenuKey::HELP).command(repository)),
            )
            .outline(platform)
            .expect("a Help contribution is valid on every platform");

            assert_eq!(
                keys(&outline)
                    .into_iter()
                    .filter(|key| *key == "Help")
                    .count(),
                1,
                "{platform:?} must not gain a duplicate Help key",
            );
            let help = nodes(&outline, MenuKey::HELP);
            match platform {
                DesktopPlatform::MacOs => {
                    assert_eq!(
                        help,
                        [MenuNode::Command(repository)],
                        "macOS has no standard Help menu, so the contribution is the whole menu",
                    );
                    assert_eq!(
                        keys(&outline),
                        vec!["App", "Edit", "Help", "Window"],
                        "the contributed Help menu is inserted before Window",
                    );
                }
                DesktopPlatform::Windows | DesktopPlatform::Linux => {
                    assert_eq!(
                        help,
                        [
                            MenuNode::Command(standard::ABOUT_COMMAND_ID),
                            MenuNode::Separator,
                            MenuNode::Command(repository),
                        ],
                        "{platform:?} appends the contribution after standard Help content, \
                         separated by one separator",
                    );
                }
            }
        }
    }

    /// Several contributions to one key merge in declaration order and never
    /// create a duplicate top-level key, whether the target key is an
    /// existing standard menu (Linux Help) or one the platform's standard
    /// layout never provides (macOS Help).
    #[test]
    fn repeated_contributions_to_one_key_merge_in_order_without_duplicating_the_key() {
        let first = CommandId("app.open_repository");
        let second = CommandId("app.report_issue");
        let build = || {
            full_standard()
                .contribute(Menu::keyed(MenuKey::HELP).command(first))
                .contribute(Menu::keyed(MenuKey::HELP).command(second))
        };

        let linux = MenuBar::from_standard_layout(build())
            .outline(DesktopPlatform::Linux)
            .expect("valid");
        assert_eq!(
            keys(&linux)
                .into_iter()
                .filter(|key| *key == "Help")
                .count(),
            1,
        );
        assert_eq!(
            nodes(&linux, MenuKey::HELP),
            [
                MenuNode::Command(standard::ABOUT_COMMAND_ID),
                MenuNode::Separator,
                MenuNode::Command(first),
                MenuNode::Separator,
                MenuNode::Command(second),
            ],
        );

        let macos = MenuBar::from_standard_layout(build())
            .outline(DesktopPlatform::MacOs)
            .expect("valid");
        assert_eq!(
            keys(&macos)
                .into_iter()
                .filter(|key| *key == "Help")
                .count(),
            1,
            "two contributions to the same absent key must not create two Help menus",
        );
        assert_eq!(
            nodes(&macos, MenuKey::HELP),
            [
                MenuNode::Command(first),
                MenuNode::Separator,
                MenuNode::Command(second),
            ],
        );
    }

    /// `contribute` only makes sense against the standard layout: calling it
    /// against `Custom` or `None` is a declaration fault, matching
    /// `hide`/`insert`.
    #[test]
    fn contribute_on_a_non_standard_policy_is_a_declaration_fault() {
        let faults = command_faults(
            MenuBar::custom(vec![Menu::keyed(MenuKey::EDIT)])
                .contribute(Menu::keyed(MenuKey::HELP).command(CommandId("app.open_repository")))
                .outline(DesktopPlatform::Linux)
                .expect_err("contribute does not apply to a custom menu bar"),
        );
        assert_eq!(
            faults,
            vec![CommandFault::InvalidStandardEdit {
                menu: MenuKey::HELP,
                reason: "contribute only edits the standard menu-bar layout",
            }],
        );

        let faults = command_faults(
            MenuBar::none()
                .contribute(Menu::keyed(MenuKey::HELP).command(CommandId("app.open_repository")))
                .outline(DesktopPlatform::Linux)
                .expect_err("contribute does not apply to an absent menu bar"),
        );
        assert_eq!(
            faults,
            vec![CommandFault::InvalidStandardEdit {
                menu: MenuKey::HELP,
                reason: "contribute only edits the standard menu-bar layout",
            }],
        );
    }

    /// A contribution to a key nobody else touches is still just inserted
    /// before Window, exactly like `insert`, proving `contribute` is a
    /// strict superset (merge-or-insert) rather than a different placement
    /// rule.
    #[test]
    fn contribute_to_a_key_with_no_standard_or_prior_contribution_is_inserted_before_window() {
        let outline = MenuBar::from_standard_layout(full_standard().contribute(
            Menu::new(MenuKey::new("Tools").unwrap(), "Tools").command(CommandId("tools.run")),
        ))
        .outline(DesktopPlatform::Linux)
        .expect("valid");

        assert_eq!(
            keys(&outline),
            vec!["File", "Edit", "View", "Tools", "Window", "Help"],
        );
        assert_eq!(
            nodes(&outline, MenuKey::new("Tools").unwrap()),
            [MenuNode::Command(CommandId("tools.run"))],
            "with no standard menu and no prior contribution at this key, there is no \
             standard/contributed boundary to separate",
        );
    }

    #[test]
    fn windows_and_linux_standard_outlines_move_the_app_block_and_about() {
        for platform in [DesktopPlatform::Windows, DesktopPlatform::Linux] {
            let outline = MenuBar::from_standard_layout(full_standard())
                .outline(platform)
                .expect("the standard layout is valid");

            assert_eq!(
                keys(&outline),
                vec!["File", "Edit", "View", "Window", "Help"],
                "{platform:?} keeps the conventional in-window bar",
            );
            assert_eq!(
                nodes(&outline, MenuKey::FILE),
                [
                    MenuNode::Command(standard::OPEN_SETTINGS_COMMAND_ID),
                    MenuNode::Separator,
                    MenuNode::Command(standard::QUIT_COMMAND_ID),
                ],
            );
            assert_eq!(
                nodes(&outline, MenuKey::VIEW),
                [MenuNode::Section(MenuSectionKey::THEME)],
            );
            assert_eq!(
                nodes(&outline, MenuKey::HELP),
                [MenuNode::Command(standard::ABOUT_COMMAND_ID)],
            );
            assert_eq!(
                nodes(&outline, MenuKey::WINDOW),
                [MenuNode::Command(standard::CLOSE_WINDOW_COMMAND_ID)],
                "Minimize and Zoom are macOS-only",
            );
        }
    }

    /// The Edit menu's exact node order, unchanged from StoryApp's old menu,
    /// on every desktop platform: Undo, Redo, separator, Cut, Copy, Paste,
    /// separator, Delete, Delete Previous Word, Delete Next Word, separator,
    /// Find, separator, Select All.
    #[test]
    fn edit_menu_matches_the_exact_cross_platform_node_order() {
        for platform in DesktopPlatform::ALL {
            let outline = MenuBar::from_standard_layout(full_standard())
                .outline(platform)
                .expect("the standard layout is valid");

            assert_eq!(
                nodes(&outline, MenuKey::EDIT),
                [
                    MenuNode::Command(standard::UNDO_COMMAND_ID),
                    MenuNode::Command(standard::REDO_COMMAND_ID),
                    MenuNode::Separator,
                    MenuNode::Command(standard::CUT_COMMAND_ID),
                    MenuNode::Command(standard::COPY_COMMAND_ID),
                    MenuNode::Command(standard::PASTE_COMMAND_ID),
                    MenuNode::Separator,
                    MenuNode::Command(standard::DELETE_COMMAND_ID),
                    MenuNode::Command(standard::DELETE_PREVIOUS_WORD_COMMAND_ID),
                    MenuNode::Command(standard::DELETE_NEXT_WORD_COMMAND_ID),
                    MenuNode::Separator,
                    MenuNode::Command(standard::FIND_COMMAND_ID),
                    MenuNode::Separator,
                    MenuNode::Command(standard::SELECT_ALL_COMMAND_ID),
                ],
                "{platform:?} Edit menu must preserve the old menu's exact order",
            );
        }
    }

    #[test]
    fn absent_optional_surfaces_leave_no_inert_items() {
        let outline = MenuBar::standard()
            .outline(DesktopPlatform::Linux)
            .expect("valid");

        assert_eq!(
            keys(&outline),
            vec!["File", "Edit", "Window"],
            "View needs a theme section and Help needs About; both are dropped",
        );
        assert_eq!(
            nodes(&outline, MenuKey::FILE),
            [MenuNode::Command(standard::QUIT_COMMAND_ID)],
        );
    }

    #[test]
    fn custom_menus_are_inserted_before_window() {
        let outline = MenuBar::from_standard_layout(
            full_standard()
                .insert(Menu::new(MenuKey::new("Tools").unwrap(), "Tools"))
                .insert(Menu::new(MenuKey::new("Debug").unwrap(), "Debug")),
        )
        .outline(DesktopPlatform::Linux)
        .expect("valid");

        assert_eq!(
            keys(&outline),
            vec!["File", "Edit", "View", "Window", "Help"],
            "menus declared with no nodes are dropped from the outline",
        );

        let outline = MenuBar::from_standard_layout(full_standard().insert(
            Menu::new(MenuKey::new("Tools").unwrap(), "Tools").command(CommandId("tools.run")),
        ))
        .outline(DesktopPlatform::Linux)
        .expect("valid");
        assert_eq!(
            keys(&outline),
            vec!["File", "Edit", "View", "Tools", "Window", "Help"],
        );
    }

    /// Hiding an optional menu is only safe while nothing enabled lives there,
    /// and where a feature lives is a property of the resolved platform: About
    /// sits in Help on Windows and Linux but in the application menu on macOS,
    /// where Help is not a standard menu at all.
    #[test]
    fn hiding_help_strands_about_only_where_the_platform_puts_it_there() {
        for platform in DesktopPlatform::ALL {
            let result = MenuBar::from_standard_layout(
                StandardLayout::new().with_about().hide(MenuKey::HELP),
            )
            .outline(platform);

            match platform {
                DesktopPlatform::MacOs => {
                    let outline = result.unwrap_or_else(|faults| {
                        panic!("macOS keeps About in the application menu: {faults:?}")
                    });
                    assert!(
                        outline.command_ids().contains(&standard::ABOUT_COMMAND_ID),
                        "About survives on macOS because Help never held it",
                    );
                }
                DesktopPlatform::Windows | DesktopPlatform::Linux => {
                    let faults = command_faults(result.expect_err("About would have no home"));
                    assert_eq!(
                        faults,
                        vec![CommandFault::StrandedStandardFeature {
                            menu: MenuKey::HELP,
                            feature: "About",
                        }],
                        "{platform:?} places About in Help",
                    );
                }
            }
        }
    }

    #[test]
    fn hiding_view_strands_the_appearance_section_only_where_it_lives_there() {
        for platform in DesktopPlatform::ALL {
            let result = MenuBar::from_standard_layout(
                StandardLayout::new()
                    .with_theme_section()
                    .hide(MenuKey::VIEW),
            )
            .outline(platform);

            match platform {
                DesktopPlatform::MacOs => {
                    let outline = result.unwrap_or_else(|faults| {
                        panic!("macOS keeps Appearance in the application menu: {faults:?}")
                    });
                    assert!(
                        outline
                            .section_slots()
                            .iter()
                            .any(|(_, section)| *section == MenuSectionKey::THEME),
                    );
                }
                DesktopPlatform::Windows | DesktopPlatform::Linux => {
                    let faults = command_faults(result.expect_err("Appearance would have no home"));
                    assert_eq!(
                        faults,
                        vec![CommandFault::StrandedStandardFeature {
                            menu: MenuKey::VIEW,
                            feature: "Appearance",
                        }],
                        "{platform:?} places the Appearance section in View",
                    );
                }
            }
        }
    }

    /// Hiding a *required* menu is reported once, as a rejected edit. The menu
    /// is never actually removed, so nothing it hosts is stranded and blaming
    /// the same edit twice would only obscure the fix.
    #[test]
    fn hiding_a_required_menu_is_reported_as_a_rejected_edit_and_not_as_stranding() {
        for platform in DesktopPlatform::ALL {
            let hidden = standard::app_menu_key(platform);
            let faults = MenuBar::from_standard_layout(
                full_standard().hide(MenuKey::new(hidden).expect("a standard key is valid")),
            )
            .outline(platform)
            .expect_err("the application menu is framework-required");
            let faults = command_faults(faults);

            assert!(
                faults
                    .iter()
                    .all(|fault| matches!(fault, CommandFault::InvalidStandardEdit { .. })),
                "{platform:?} reports one rejected edit, not a stranding cascade: {faults:?}",
            );
        }
    }

    /// Every enabled feature the hidden menu hosted is named, in the order the
    /// standard layout would have rendered them.
    #[test]
    fn a_hidden_menu_reports_every_feature_it_would_have_stranded() {
        // Windows/Linux keep Appearance and About in different menus, so one
        // hide can only strand one of them; a layout hiding both reports both.
        let faults =
            MenuBar::from_standard_layout(full_standard().hide(MenuKey::VIEW).hide(MenuKey::HELP))
                .outline(DesktopPlatform::Linux)
                .expect_err("both optional menus host an enabled feature");
        let faults = command_faults(faults);

        assert_eq!(
            faults,
            vec![
                CommandFault::StrandedStandardFeature {
                    menu: MenuKey::VIEW,
                    feature: "Appearance",
                },
                CommandFault::StrandedStandardFeature {
                    menu: MenuKey::HELP,
                    feature: "About",
                },
            ],
        );
    }

    /// Stranding is a property of the *standard* layout only: `Custom` and
    /// `None` own their placement, so the framework has no opinion to enforce.
    #[test]
    fn custom_and_absent_menu_bars_are_untouched_by_the_stranding_rule() {
        let features = StandardFeatures::default();
        for platform in DesktopPlatform::ALL {
            assert!(
                MenuBar::custom(vec![Menu::keyed(MenuKey::EDIT)])
                    .outline_with(platform, features)
                    .is_ok(),
            );
            assert!(MenuBar::none().outline_with(platform, features).is_ok());
        }
    }

    /// `hide`/`insert` only make sense against the standard layout: calling
    /// either against `Custom` or `None` is a declaration fault, not a
    /// silent no-op.
    #[test]
    fn hide_and_insert_on_a_non_standard_policy_are_declaration_faults() {
        let faults = command_faults(
            MenuBar::custom(vec![Menu::keyed(MenuKey::EDIT)])
                .hide(MenuKey::VIEW)
                .outline(DesktopPlatform::Linux)
                .expect_err("hide does not apply to a custom menu bar"),
        );
        assert_eq!(
            faults,
            vec![CommandFault::InvalidStandardEdit {
                menu: MenuKey::VIEW,
                reason: "hide only edits the standard menu-bar layout",
            }],
        );

        let faults = command_faults(
            MenuBar::none()
                .insert(Menu::keyed(MenuKey::new("Tools").unwrap()))
                .outline(DesktopPlatform::Linux)
                .expect_err("insert does not apply to an absent menu bar"),
        );
        assert_eq!(
            faults,
            vec![CommandFault::InvalidStandardEdit {
                menu: MenuKey::new("Tools").unwrap(),
                reason: "insert only edits the standard menu-bar layout",
            }],
        );
    }

    #[test]
    fn optional_menus_hide_and_required_menus_do_not() {
        // No About, so hiding Help strands nothing: the stranding rule is
        // asserted separately.
        let outline = MenuBar::from_standard_layout(
            StandardLayout::new()
                .with_settings()
                .with_theme_section()
                .hide(MenuKey::HELP),
        )
        .outline(DesktopPlatform::Windows)
        .expect("Help is optional");
        assert_eq!(keys(&outline), vec!["File", "Edit", "View", "Window"]);

        let faults = MenuBar::from_standard_layout(full_standard().hide(MenuKey::EDIT))
            .outline(DesktopPlatform::Windows)
            .expect_err("Edit is framework-required");
        let faults = command_faults(faults);
        assert_eq!(
            faults,
            vec![CommandFault::InvalidStandardEdit {
                menu: MenuKey::EDIT,
                reason: "menu is required by the framework and cannot be hidden",
            }],
        );
    }

    #[test]
    fn custom_menu_bars_still_receive_macos_services() {
        let custom = || {
            vec![
                Menu::new(MenuKey::APP, "Custom").command(standard::QUIT_COMMAND_ID),
                Menu::keyed(MenuKey::new("Tools").unwrap()).command(CommandId("tools.run")),
            ]
        };

        let macos = MenuBar::custom(custom())
            .outline(DesktopPlatform::MacOs)
            .expect("valid");
        assert_eq!(
            nodes(&macos, MenuKey::APP),
            [
                MenuNode::Command(standard::QUIT_COMMAND_ID),
                MenuNode::Separator,
                MenuNode::Services,
            ],
            "Services is mandatory even when the application owns the layout",
        );

        let linux = MenuBar::custom(custom())
            .outline(DesktopPlatform::Linux)
            .expect("valid");
        assert_eq!(
            nodes(&linux, MenuKey::APP),
            [MenuNode::Command(standard::QUIT_COMMAND_ID)],
            "Services is a macOS node only",
        );
    }

    #[test]
    fn services_never_lands_in_the_dock_menu() {
        // A dock-first custom layout with no App menu: Services must skip the
        // dock and land in the first menu that can actually render it.
        let dock_first = MenuBar::custom(vec![
            Menu::keyed(MenuKey::DOCK).command(CommandId("dock.new")),
            Menu::keyed(MenuKey::new("Tools").unwrap()).command(CommandId("tools.run")),
        ])
        .outline(DesktopPlatform::MacOs)
        .expect("valid");
        assert_eq!(
            nodes(&dock_first, MenuKey::DOCK),
            [MenuNode::Command(CommandId("dock.new"))],
            "the dock projection has no Services machinery",
        );
        assert_eq!(
            nodes(&dock_first, MenuKey::new("Tools").unwrap()),
            [
                MenuNode::Command(CommandId("tools.run")),
                MenuNode::Separator,
                MenuNode::Services,
            ],
            "Services falls back to the first non-dock menu",
        );

        // A dock-only bar has nowhere to host Services, so nothing is injected.
        let dock_only = MenuBar::custom(vec![
            Menu::keyed(MenuKey::DOCK).command(CommandId("dock.new")),
        ])
        .outline(DesktopPlatform::MacOs)
        .expect("valid");
        assert_eq!(
            dock_only.menus().len(),
            1,
            "a dock-only bar gains no menu just to host Services",
        );
        assert_eq!(
            nodes(&dock_only, MenuKey::DOCK),
            [MenuNode::Command(CommandId("dock.new"))],
        );
    }

    #[test]
    fn a_command_may_repeat_across_menus_but_not_within_one() {
        let repeated = MenuBar::custom(vec![
            Menu::keyed(MenuKey::EDIT)
                .command(CommandId("a"))
                .separator()
                .command(CommandId("a")),
        ])
        .outline(DesktopPlatform::Linux)
        .expect_err("one menu may not render the same command twice");
        let repeated = command_faults(repeated);
        assert_eq!(
            repeated,
            vec![CommandFault::RepeatedCommandInMenu {
                menu: MenuKey::EDIT,
                command: CommandId("a"),
            }],
        );

        MenuBar::custom(vec![
            Menu::keyed(MenuKey::EDIT).command(CommandId("a")),
            Menu::keyed(MenuKey::VIEW).command(CommandId("a")),
            Menu::keyed(MenuKey::DOCK).command(CommandId("a")),
        ])
        .outline(DesktopPlatform::Linux)
        .expect("one command may project into several surfaces");
    }

    #[test]
    fn none_projects_no_menus() {
        assert!(
            MenuBar::none()
                .outline(DesktopPlatform::MacOs)
                .expect("valid")
                .menus()
                .is_empty(),
        );
    }

    #[test]
    fn duplicate_top_level_keys_and_section_slots_are_faults() {
        let faults = MenuBar::custom(vec![
            Menu::keyed(MenuKey::EDIT).command(CommandId("edit.undo")),
            Menu::keyed(MenuKey::EDIT).command(CommandId("edit.redo")),
        ])
        .outline(DesktopPlatform::Linux)
        .expect_err("Edit is declared twice");
        let faults = command_faults(faults);
        assert_eq!(
            faults,
            vec![CommandFault::DuplicateMenuKey {
                menu: MenuKey::EDIT
            }],
        );

        let faults = MenuBar::custom(vec![
            Menu::keyed(MenuKey::VIEW).section(MenuSectionKey::THEME),
            Menu::keyed(MenuKey::WINDOW).section(MenuSectionKey::THEME),
        ])
        .outline(DesktopPlatform::Linux)
        .expect_err("one provider cannot fill two slots");
        let faults = command_faults(faults);
        assert_eq!(
            faults,
            vec![CommandFault::DuplicateSectionSlot {
                section: MenuSectionKey::THEME
            }],
        );
    }

    #[test]
    fn dangling_command_and_section_references_are_faults() {
        let outline = MenuBar::custom(vec![
            Menu::keyed(MenuKey::EDIT)
                .command(CommandId("edit.undo"))
                .command(CommandId("edit.ghost"))
                .section(MenuSectionKey::THEME),
        ])
        .outline(DesktopPlatform::Linux)
        .expect("structurally valid");

        let mut faults = Vec::new();
        MenuBar::validate_references(
            &outline,
            &HashSet::from([CommandId("edit.undo")]),
            &HashSet::new(),
            &mut faults,
        );

        assert_eq!(
            faults,
            vec![
                CommandFault::UnknownCommand {
                    menu: MenuKey::EDIT,
                    command: CommandId("edit.ghost"),
                },
                CommandFault::MissingSectionProvider {
                    menu: MenuKey::EDIT,
                    section: MenuSectionKey::THEME,
                },
            ],
        );
    }

    #[test]
    fn menus_reference_commands_without_redeclaring_behavior() {
        // The same command id may appear in two menus: a menu is a projection,
        // not a second behavior declaration.
        let shared = CommandId("app.search");
        let outline = MenuBar::custom(vec![
            Menu::keyed(MenuKey::EDIT).command(shared),
            Menu::keyed(MenuKey::new("Tools").unwrap()).command(shared),
        ])
        .outline(DesktopPlatform::Linux)
        .expect("valid");

        assert_eq!(outline.command_ids(), vec![shared, shared]);

        let mut faults = Vec::new();
        MenuBar::validate_references(
            &outline,
            &HashSet::from([shared]),
            &HashSet::new(),
            &mut faults,
        );
        assert!(faults.is_empty(), "one declaration satisfies both menus");
    }

    #[test]
    fn separators_never_lead_trail_or_double() {
        let outline = MenuBar::custom(vec![
            Menu::keyed(MenuKey::EDIT)
                .separator()
                .separator()
                .command(CommandId("edit.undo"))
                .separator()
                .separator()
                .command(CommandId("edit.redo"))
                .separator(),
        ])
        .outline(DesktopPlatform::Linux)
        .expect("valid");

        assert_eq!(
            nodes(&outline, MenuKey::EDIT),
            [
                MenuNode::Command(CommandId("edit.undo")),
                MenuNode::Separator,
                MenuNode::Command(CommandId("edit.redo")),
            ],
        );
    }

    #[test]
    fn section_slots_report_their_owning_menu() {
        let outline = MenuBar::from_standard_layout(full_standard())
            .outline(DesktopPlatform::MacOs)
            .expect("valid");

        assert_eq!(
            outline.section_slots(),
            vec![(MenuKey::APP, MenuSectionKey::THEME)],
        );
    }
}
