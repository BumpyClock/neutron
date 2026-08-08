//! [`MenuPlan`]: the declarative native-menu description, assembled by
//! projecting the [`CommandRegistry`](super::CommandRegistry).
//!
//! Projection is two-stage so the tree structure is unit-testable without a
//! live `App`:
//!
//! 1. [`MenuPlan::outline`] (pure): given the registered commands, compute the
//!    ordered [`MenuOutline`]s — which top-level menus, and the sequence of
//!    command/separator/section nodes inside each.
//! 2. [`build_menus`] (App-bound): resolve each node into a `gpui::Menu`,
//!    evaluating labels/checked/enabled and expanding section providers.

use std::collections::HashSet;

use gpui::{App, Menu, MenuItem, SystemMenuType};

use super::{APP_MENU, Command, CommandId, CommandRegistry, DOCK_MENU, EDIT_MENU, WINDOW_MENU};

/// One node in a projected menu.
#[derive(Debug, PartialEq, Eq)]
pub enum MenuNode {
    /// A command, referenced by id (resolved to a menu item at build time).
    Command(CommandId),
    /// A separator between groups or before a section.
    Separator,
    /// A reserved section slot, expanded by its registered provider at build
    /// time (skipped if no provider is registered).
    Section(&'static str),
    /// The macOS system-managed Services menu.
    Services,
}

/// The pure, ordered structure of one top-level menu.
#[derive(Debug, PartialEq, Eq)]
pub struct MenuOutline {
    /// The top-level menu key (e.g. [`APP_MENU`]); the App menu renders with the
    /// app display name.
    pub key: &'static str,
    /// The ordered nodes.
    pub nodes: Vec<MenuNode>,
}

/// One top-level menu in the plan: a key plus any reserved section slots
/// appended after its commands.
struct TopMenu {
    key: &'static str,
    fixed: Vec<FixedNode>,
}

struct FixedNode {
    group: u16,
    order: u16,
    node: MenuNode,
}

/// A declarative description of the native menu bar. Menus are projections of
/// the command registry (plan §3).
pub struct MenuPlan {
    menus: Vec<TopMenu>,
}

impl MenuPlan {
    /// The standard App + Edit + Window menu bar, projected from the registry.
    pub fn standard() -> Self {
        Self::from_keys([APP_MENU, EDIT_MENU, WINDOW_MENU])
    }

    /// Create a plan containing exactly the top-level menu keys in declaration
    /// order. Duplicate keys are ignored after their first occurrence.
    pub fn from_keys(keys: impl IntoIterator<Item = &'static str>) -> Self {
        let mut seen = HashSet::new();
        let menus = keys
            .into_iter()
            .filter(|key| seen.insert(*key))
            .map(|key| TopMenu {
                key,
                fixed: Vec::new(),
            })
            .collect();
        Self { menus }
    }

    /// Append a top-level menu. Existing keys are not duplicated.
    #[must_use]
    pub fn with_menu(mut self, key: &'static str) -> Self {
        if !self.menus.iter().any(|menu| menu.key == key) {
            self.menus.push(TopMenu {
                key,
                fixed: Vec::new(),
            });
        }
        self
    }

    pub(super) fn insert_before(&mut self, before: &'static str, key: &'static str) {
        if self.menus.iter().any(|menu| menu.key == key) {
            return;
        }
        let ix = self
            .menus
            .iter()
            .position(|menu| menu.key == before)
            .unwrap_or(self.menus.len());
        self.menus.insert(
            ix,
            TopMenu {
                key,
                fixed: Vec::new(),
            },
        );
    }

    /// Reserve an Appearance/Theme section in the App menu, fed by the
    /// [`MenuSection`](super::MenuSection) provider registered under
    /// [`THEME_SECTION`].
    pub fn with_theme_menu(mut self) -> Self {
        self.reserve_section(APP_MENU, THEME_SECTION);
        self
    }

    /// Reserve a section `slot` at the end of the menu keyed by `menu_key`. The
    /// same seam serves a future Move-to-Window section.
    pub fn reserve_section(&mut self, menu_key: &'static str, slot: &'static str) {
        self.reserve_section_at(menu_key, u16::MAX, 0, slot);
    }

    /// Compute the pure menu outline from the registered `commands`.
    pub fn outline(&self, commands: &[Command]) -> Vec<MenuOutline> {
        self.warn_unknown_placements(commands);
        self.menus
            .iter()
            .map(|menu| MenuOutline {
                key: menu.key,
                nodes: outline_nodes(menu, commands),
            })
            .collect()
    }

    pub(super) fn reserve_section_at(
        &mut self,
        menu_key: &'static str,
        group: u16,
        order: u16,
        slot: &'static str,
    ) {
        let Some(menu) = self.menus.iter_mut().find(|menu| menu.key == menu_key) else {
            log::warn!("menu section `{slot}` targets missing top-level menu `{menu_key}`");
            return;
        };
        menu.fixed.push(FixedNode {
            group,
            order,
            node: MenuNode::Section(slot),
        });
    }

    pub(super) fn reserve_services_at(&mut self, menu_key: &'static str, group: u16, order: u16) {
        let Some(menu) = self.menus.iter_mut().find(|menu| menu.key == menu_key) else {
            log::warn!("Services targets missing top-level menu `{menu_key}`");
            return;
        };
        menu.fixed.push(FixedNode {
            group,
            order,
            node: MenuNode::Services,
        });
    }

    fn warn_unknown_placements(&self, commands: &[Command]) {
        for placement in commands.iter().filter_map(Command::placement) {
            if placement.menu != DOCK_MENU
                && !self.menus.iter().any(|menu| menu.key == placement.menu)
            {
                log::warn!(
                    "command targets missing top-level menu `{}`",
                    placement.menu
                );
            }
        }
    }
}

/// The section slot fed by the theme service's Appearance/Theme submenu.
pub const THEME_SECTION: &str = "appearance";

/// Build the ordered nodes for one top-level menu: commands grouped and
/// separated by `(group, order)`, then each reserved section (preceded by a
/// separator when the menu already has items).
fn outline_nodes(menu: &TopMenu, commands: &[Command]) -> Vec<MenuNode> {
    enum PlacedNode<'a> {
        Command(CommandId),
        Fixed(&'a MenuNode),
    }

    let mut placed: Vec<(u16, u16, usize, PlacedNode<'_>)> = commands
        .iter()
        .enumerate()
        .filter_map(|c| {
            c.1.placement()
                .filter(|p| p.menu == menu.key)
                .map(|p| (p.group, p.order, c.0, PlacedNode::Command(c.1.id())))
        })
        .collect();
    placed.extend(menu.fixed.iter().enumerate().map(|(ix, fixed)| {
        (
            fixed.group,
            fixed.order,
            commands.len() + ix,
            PlacedNode::Fixed(&fixed.node),
        )
    }));
    placed.sort_by_key(|(group, order, insertion, _)| (*group, *order, *insertion));

    let mut nodes = Vec::new();
    let mut last_group: Option<u16> = None;
    for (group, _, _, node) in placed {
        if let Some(prev) = last_group {
            if prev != group {
                nodes.push(MenuNode::Separator);
            }
        }
        last_group = Some(group);
        match node {
            PlacedNode::Command(id) => nodes.push(MenuNode::Command(id)),
            PlacedNode::Fixed(MenuNode::Section(slot)) => nodes.push(MenuNode::Section(slot)),
            PlacedNode::Fixed(MenuNode::Services) => nodes.push(MenuNode::Services),
            PlacedNode::Fixed(_) => unreachable!("fixed nodes are sections or Services"),
        }
    }
    nodes
}

/// Resolve the plan into native `gpui::Menu`s against the current app state.
///
/// Returns `None` when no plan is installed. Empty menus (no placed commands and
/// no populated sections) are dropped so the bar stays clean.
pub(super) fn build_menus(cx: &App, registry: &CommandRegistry) -> Option<Vec<Menu>> {
    let plan = registry.plan()?;
    let app_title = app_menu_title(cx);

    let mut menus = Vec::new();
    for outline in plan.outline(registry.commands()) {
        let mut items = Vec::new();
        for node in &outline.nodes {
            match node {
                MenuNode::Command(id) => {
                    if let Some(cmd) = registry.get(*id) {
                        items.push(cmd.to_menu_item(cx));
                    }
                }
                // Never emit a leading or doubled separator: a preceding
                // reserved section may have resolved to zero items.
                MenuNode::Separator => {
                    if !items.is_empty() && !matches!(items.last(), Some(MenuItem::Separator)) {
                        items.push(MenuItem::Separator);
                    }
                }
                MenuNode::Section(slot) => {
                    if let Some(section) = registry.section(slot) {
                        // Drop a dangling separator if the section is empty.
                        let section_items = section.items(cx);
                        if section_items.is_empty() {
                            if matches!(items.last(), Some(MenuItem::Separator)) {
                                items.pop();
                            }
                        } else {
                            items.extend(section_items);
                        }
                    } else if matches!(items.last(), Some(MenuItem::Separator)) {
                        items.pop();
                    }
                }
                MenuNode::Services => {
                    items.push(MenuItem::os_submenu("Services", SystemMenuType::Services));
                }
            }
        }
        if items.is_empty() {
            continue;
        }
        let name = if outline.key == APP_MENU {
            app_title.clone()
        } else {
            outline.key.into()
        };
        menus.push(Menu {
            name,
            items,
            disabled: false,
        });
    }
    Some(menus)
}

/// Project the dock-menu commands (keyed [`DOCK_MENU`](super::DOCK_MENU)) into a
/// flat item list for `set_dock_menu`.
#[cfg(target_os = "macos")]
pub(super) fn build_dock_items(cx: &App, registry: &CommandRegistry) -> Vec<MenuItem> {
    let mut placed: Vec<(u16, u16, &Command)> = registry
        .commands()
        .iter()
        .filter_map(|c| {
            c.placement()
                .filter(|p| p.menu == super::DOCK_MENU)
                .map(|p| (p.group, p.order, c))
        })
        .collect();
    placed.sort_by_key(|(group, order, _)| (*group, *order));

    let mut items = Vec::new();
    let mut last_group: Option<u16> = None;
    for (group, _, cmd) in placed {
        if let Some(prev) = last_group {
            if prev != group {
                items.push(MenuItem::Separator);
            }
        }
        last_group = Some(group);
        items.push(cmd.to_menu_item(cx));
    }
    items
}

/// The App menu title: the app display name when the shell is installed, else a
/// neutral fallback (keeps pure/early builds working).
fn app_menu_title(cx: &App) -> gpui::SharedString {
    use crate::handles::{AppShellExt, ShellState};
    if cx.has_global::<ShellState>() {
        cx.app_info().display_name().to_string().into()
    } else {
        APP_MENU.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{CommandScope, MenuPlacement};
    use gpui::actions;

    actions!(menu_test, [Noop]);

    fn cmd(id: &'static str, placement: MenuPlacement) -> Command {
        Command::new(CommandId(id), id, CommandScope::App, Noop).with_placement(placement)
    }

    #[test]
    fn outline_groups_with_separators_and_sorts() {
        let commands = vec![
            cmd("quit", MenuPlacement::new(APP_MENU, 9, 0)),
            cmd("about", MenuPlacement::new(APP_MENU, 0, 0)),
            cmd("copy", MenuPlacement::new(EDIT_MENU, 0, 1)),
            cmd("undo", MenuPlacement::new(EDIT_MENU, 0, 0)),
        ];
        let plan = MenuPlan::standard();
        let outline = plan.outline(&commands);

        // App menu: about (group 0), separator, quit (group 9).
        assert_eq!(outline[0].key, APP_MENU);
        assert_eq!(
            outline[0].nodes,
            vec![
                MenuNode::Command(CommandId("about")),
                MenuNode::Separator,
                MenuNode::Command(CommandId("quit")),
            ]
        );
        // Edit menu: undo then copy (order within group), no separator.
        assert_eq!(
            outline[1].nodes,
            vec![
                MenuNode::Command(CommandId("undo")),
                MenuNode::Command(CommandId("copy")),
            ]
        );
        // Window menu: empty.
        assert_eq!(outline[2].key, WINDOW_MENU);
        assert!(outline[2].nodes.is_empty());
    }

    #[test]
    fn theme_menu_reserves_section_after_separator() {
        let commands = vec![cmd("about", MenuPlacement::new(APP_MENU, 0, 0))];
        let plan = MenuPlan::standard().with_theme_menu();
        let outline = plan.outline(&commands);
        assert_eq!(
            outline[0].nodes,
            vec![
                MenuNode::Command(CommandId("about")),
                MenuNode::Separator,
                MenuNode::Section(THEME_SECTION),
            ]
        );
    }

    #[test]
    fn theme_section_without_commands_has_no_leading_separator() {
        let plan = MenuPlan::standard().with_theme_menu();
        let outline = plan.outline(&[]);
        assert_eq!(outline[0].nodes, vec![MenuNode::Section(THEME_SECTION)]);
    }

    #[test]
    fn exact_and_appended_menu_keys_preserve_order() {
        let plan = MenuPlan::from_keys([EDIT_MENU, "Tools"]).with_menu(WINDOW_MENU);
        let outline = plan.outline(&[]);
        assert_eq!(
            outline.iter().map(|menu| menu.key).collect::<Vec<_>>(),
            vec![EDIT_MENU, "Tools", WINDOW_MENU]
        );
    }

    #[test]
    fn duplicate_appended_menu_key_is_ignored() {
        let plan = MenuPlan::from_keys([EDIT_MENU]).with_menu(EDIT_MENU);
        assert_eq!(plan.outline(&[]).len(), 1);
    }
}
