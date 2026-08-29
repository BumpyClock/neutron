//! The private integration API for the typed command/menu model (issue #11).
//!
//! [`CommandsDeclaration`] is the whole vocabulary an application declares:
//! typed commands, section providers, and one menu-bar policy. It validates
//! purely and then *lowers* onto the command registry — the
//! [`CommandRegistry`](super::CommandRegistry), [`MenuPlan`], and GPUI action
//! handlers.
//!
//! Framework-internal: `AppDeclaration` wires [`CommandsDeclaration`] in as one
//! declaration module, maps [`CommandFaults`] into `DeclarationError`
//! variants, and calls [`CommandsDeclaration::install`] from the menu module.

use std::collections::{HashMap, HashSet};

use gpui::{Action, App, MenuItem};

use super::declared::{Command, ErasedCommand};
use super::faults::{CommandFault, CommandFaults};
use super::keys::{MenuKey, MenuSectionKey};
use super::menu_model::{MenuBar, MenuNode, MenuOutline};
use super::standard::{self, DesktopPlatform};
use super::{CommandError, CommandId, MenuPlacement, MenuPlan};

/// A section provider: freshly built items for one reserved slot.
pub(crate) type SectionProvider = fn(&App) -> Vec<MenuItem>;

/// An opener for one standard surface, monomorphized at the declaration site.
///
/// A plain function pointer rather than a boxed closure: the standard command
/// handlers need `Copy` openers, and the view type is already erased by
/// monomorphization.
type StandardOpener = fn(&mut App) -> anyhow::Result<()>;

/// The standard desktop features an [`AppDeclaration`] resolved.
///
/// The declaration core owns the *policy* (is there a Settings surface? is
/// About replaced or disabled? is the theme convention on?); this is the
/// resolved answer, and it is the only thing the command model needs. Each
/// feature carries its own handler where it has one, so a feature can never be
/// half-present: an id in the menu with no command behind it, or a command with
/// no way to run.
///
/// [`AppDeclaration`]: crate::declaration::AppDeclaration
#[derive(Clone, Copy, Default)]
pub(crate) struct StandardFeatures {
    /// The Settings surface opener, present only when a Settings surface is
    /// declared. No surface means no Settings command, shortcut, or menu item.
    pub(crate) settings: Option<StandardOpener>,
    /// The About surface opener, present unless About is disabled.
    pub(crate) about: Option<StandardOpener>,
    /// Whether the Appearance section and its theme provider are installed.
    pub(crate) theme: bool,
}

impl StandardFeatures {
    /// Whether the standard Settings command is part of the vocabulary.
    pub(crate) fn has_settings(&self) -> bool {
        self.settings.is_some()
    }

    /// Whether the standard About command is part of the vocabulary.
    pub(crate) fn has_about(&self) -> bool {
        self.about.is_some()
    }
}

/// Why installing a validated declaration onto the command registry failed.
#[derive(Debug)]
pub(crate) enum InstallError {
    /// The declaration itself is invalid.
    Declaration(CommandFaults),
    /// The registry rejected a lowered command.
    Runtime(CommandError),
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Declaration(faults) => write!(f, "{faults}"),
            Self::Runtime(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for InstallError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Declaration(faults) => Some(faults),
            Self::Runtime(error) => Some(error),
        }
    }
}

/// One application's complete command and menu declaration.
pub(crate) struct CommandsDeclaration {
    commands: Vec<Box<dyn ErasedCommand>>,
    sections: Vec<(MenuSectionKey, SectionProvider)>,
    menu_bar: MenuBar,
    /// The standard features the declaration core resolved. Empty by default:
    /// this type is also used on its own, and the desktop conventions belong to
    /// `AppDeclaration::new`, not to the command model.
    features: StandardFeatures,
}

impl CommandsDeclaration {
    /// An empty declaration using the standard menu bar, matching
    /// `AppDeclaration::new`.
    pub(crate) fn new() -> Self {
        Self {
            commands: Vec::new(),
            sections: Vec::new(),
            menu_bar: MenuBar::standard(),
            features: StandardFeatures::default(),
        }
    }

    /// Record the resolved standard features.
    #[must_use]
    pub(crate) fn standard_features(mut self, features: StandardFeatures) -> Self {
        self.features = features;
        self
    }

    /// Declare one typed command. The action type is erased immediately, so the
    /// declaration stays non-generic.
    #[must_use]
    pub(crate) fn command<A: Action>(mut self, command: Command<A>) -> Self {
        self.commands.push(Box::new(command));
        self
    }

    /// Declare the provider for one reserved menu section.
    #[must_use]
    pub(crate) fn section(mut self, key: MenuSectionKey, provider: SectionProvider) -> Self {
        self.sections.push((key, provider));
        self
    }

    /// Set the menu-bar policy (default [`MenuBar::standard`]).
    #[must_use]
    pub(crate) fn menu_bar(mut self, menu_bar: MenuBar) -> Self {
        self.menu_bar = menu_bar;
        self
    }

    /// Validate the whole declaration for one platform and return the resolved
    /// outline.
    ///
    /// Pure: no GPUI, no filesystem, no host-platform inspection, so the result
    /// is identical on every host for a given `platform` argument.
    ///
    /// # Errors
    ///
    /// Reports every independent fault in declaration order: duplicate command
    /// ids, unparsable bindings and key contexts on *any* platform, duplicate
    /// section providers, structural menu faults, and dangling command or
    /// section references.
    pub(crate) fn validate(&self, platform: DesktopPlatform) -> Result<MenuOutline, CommandFaults> {
        self.validate_with(platform, self.features)
    }

    /// Validate against a features set the caller resolved, without recording
    /// it.
    ///
    /// The declaration core validates before it lowers, and lowering is what
    /// moves the resolved features onto this value; this seam lets validation
    /// see the same answer without a mutation the pure path has no business
    /// making.
    pub(crate) fn validate_with(
        &self,
        platform: DesktopPlatform,
        features: StandardFeatures,
    ) -> Result<MenuOutline, CommandFaults> {
        let mut faults = Vec::new();

        // The framework vocabulary is installed by `install`, so it is in scope
        // for reference checking. Seeding it here also makes an app that
        // re-declares a framework id a `DuplicateCommand` fault rather than a
        // runtime collision.
        let mut ids: HashSet<CommandId> = standard::framework_command_ids(platform)
            .into_iter()
            .chain(standard::feature_command_ids(features))
            .collect();
        for command in &self.commands {
            if !ids.insert(command.id()) {
                faults.push(CommandFault::DuplicateCommand {
                    command: command.id(),
                });
            }
            command.validate(&mut faults);
        }

        let mut sections = HashSet::new();
        // The theme feature registers the Appearance provider at runtime rather
        // than through `section`, so seed its slot or the outline that reserves
        // it would look dangling.
        if features.theme {
            sections.insert(MenuSectionKey::THEME);
        }
        for (key, _) in &self.sections {
            if !sections.insert(*key) {
                faults.push(CommandFault::DuplicateSectionProvider { section: *key });
            }
        }

        let outline = match self.menu_bar.outline_with(platform, features) {
            Ok(outline) => Some(outline),
            Err(menu_faults) => {
                faults.extend(menu_faults);
                None
            }
        };
        if let Some(outline) = &outline {
            MenuBar::validate_references(outline, &ids, &sections, &mut faults);
        }

        match (CommandFaults::new(faults), outline) {
            (Some(faults), _) => Err(faults),
            (None, Some(outline)) => Ok(outline),
            (None, None) => unreachable!("a failed outline always contributes at least one fault"),
        }
    }

    /// Validate, then lower onto the command registry.
    ///
    /// Installs the framework's standard command vocabulary first, then every
    /// application command, then each app-scoped typed handler and the section
    /// providers, and returns the [`MenuPlan`] the menu module should install.
    ///
    /// Atomic: the whole batch is lowered and checked against the live registry
    /// before anything is registered, and no action handler is installed until
    /// registration has succeeded. A rejected declaration leaves the registry,
    /// the handlers, and the projection untouched.
    ///
    /// # Errors
    ///
    /// Returns [`InstallError`] for declaration faults and runtime registration
    /// failures.
    pub(crate) fn install(
        self,
        platform: DesktopPlatform,
        cx: &mut App,
    ) -> Result<MenuPlan, InstallError> {
        let outline = self.validate(platform).map_err(InstallError::Declaration)?;
        let LoweredMenus { plan, placements } = lower_menus(&outline);
        let placements_of = |id: CommandId| placements.get(&id).map_or(&[][..], Vec::as_slice);

        // Phase 1: build every runtime value. Pure — `cx` is untouched.
        // Framework commands come first so an application command that shadows
        // one is rejected rather than silently overriding it.
        let mut commands: Vec<super::RuntimeCommand> = standard::framework_commands(platform)
            .into_iter()
            .chain(standard::feature_commands(platform, self.features))
            .map(|command| {
                placements_of(command.id())
                    .iter()
                    .fold(command, |command, placement| {
                        command.with_placement(*placement)
                    })
            })
            .collect();
        let mut handlers = Vec::new();
        for command in self.commands {
            let placements = placements_of(command.id());
            let (command, handler) = command.lower(platform, placements).into_parts();
            commands.push(command);
            handlers.extend(handler);
        }

        // Phase 2: mutate. `install_declared_commands` re-checks the whole batch
        // against the live registry and registers all or nothing, so the
        // irreversible handler installation below only runs once the commands
        // are in.
        super::install_declared_commands(cx, commands).map_err(InstallError::Runtime)?;
        standard::install_framework_handlers(cx);
        standard::install_feature_handlers(cx, self.features);
        for handler in handlers {
            handler(cx);
        }
        for (key, provider) in self.sections {
            super::register_declared_section(cx, key.as_str(), provider);
        }
        Ok(plan)
    }
}

impl Default for CommandsDeclaration {
    fn default() -> Self {
        Self::new()
    }
}

/// The command registry's view of a typed outline.
struct LoweredMenus {
    plan: MenuPlan,
    /// Every placement per command, in outline order. A command projected into
    /// several menus (or into a menu *and* the dock) contributes one entry per
    /// surface.
    placements: HashMap<CommandId, Vec<MenuPlacement>>,
}

/// Translate a typed outline into the placement-based model the current
/// executor uses: each separator opens the next group, and every node keeps its
/// order within that group.
fn lower_menus(outline: &MenuOutline) -> LoweredMenus {
    let mut plan = MenuPlan::from_keys(
        outline
            .menus()
            .iter()
            .map(super::menu_model::MenuOutlineEntry::key)
            .filter(|key| !key.is_dock())
            .map(MenuKey::as_str),
    );
    let mut placements: HashMap<CommandId, Vec<MenuPlacement>> = HashMap::new();

    // Declared titles survive lowering: the plan carries them so `build_menus`
    // renders what the declaration asked for instead of the raw key, and a
    // derived title is re-resolved on every projection.
    for menu in outline.menus() {
        plan.set_menu_label(menu.key().as_str(), menu.label().clone());
    }

    for menu in outline.menus() {
        let (mut group, mut order) = (0_u16, 0_u16);
        for node in menu.nodes() {
            match node {
                MenuNode::Separator => {
                    group = group.saturating_add(1);
                    order = 0;
                }
                MenuNode::Command(id) => {
                    placements.entry(*id).or_default().push(MenuPlacement::new(
                        menu.key().as_str(),
                        group,
                        order,
                    ));
                    order = order.saturating_add(1);
                }
                MenuNode::Section(section) => {
                    plan.reserve_section_at(menu.key().as_str(), group, order, section.as_str());
                    order = order.saturating_add(1);
                }
                MenuNode::Services => {
                    plan.reserve_services_at(menu.key().as_str(), group, order);
                    order = order.saturating_add(1);
                }
            }
        }
    }

    LoweredMenus { plan, placements }
}

#[cfg(test)]
mod tests {
    use gpui::{BorrowAppContext as _, actions};

    use super::*;
    use crate::commands::AppCommandsExt;
    use crate::commands::CommandRegistry;
    use crate::commands::binding::CommandBinding;
    use crate::commands::label::MenuLabel;
    use crate::commands::menu::MenuPlanNode as PlanNode;
    use crate::commands::menu_model::Menu;

    actions!(declaration_test, [Alpha, Beta]);

    const ALPHA: CommandId = CommandId("test.alpha");
    const BETA: CommandId = CommandId("test.beta");

    fn ok(_: &Alpha, _: &mut App) -> anyhow::Result<()> {
        Ok(())
    }

    fn empty_section(_: &App) -> Vec<MenuItem> {
        Vec::new()
    }

    /// How many times `id` is registered. The framework vocabulary shares the
    /// registry with app commands, so counting the whole registry would drift
    /// with the standard menus.
    fn registrations(registry: &CommandRegistry, id: CommandId) -> usize {
        registry
            .commands()
            .iter()
            .filter(|command| command.id() == id)
            .count()
    }

    fn two_commands() -> CommandsDeclaration {
        CommandsDeclaration::new()
            .command(Command::app(ALPHA, Alpha, ok).label("Alpha"))
            .command(Command::window(BETA, Beta).label("Beta"))
    }

    fn faults(declaration: &CommandsDeclaration, platform: DesktopPlatform) -> Vec<CommandFault> {
        declaration
            .validate(platform)
            .expect_err("expected declaration faults")
            .iter()
            .copied()
            .collect()
    }

    #[test]
    fn duplicate_command_ids_are_declaration_errors() {
        let declaration = CommandsDeclaration::new()
            .menu_bar(MenuBar::none())
            .command(Command::app(ALPHA, Alpha, ok))
            .command(Command::window(ALPHA, Beta));

        assert_eq!(
            faults(&declaration, DesktopPlatform::Linux),
            vec![CommandFault::DuplicateCommand { command: ALPHA }],
        );
    }

    #[test]
    fn duplicate_section_providers_are_declaration_errors() {
        let declaration = CommandsDeclaration::new()
            .menu_bar(MenuBar::none())
            .section(MenuSectionKey::THEME, empty_section)
            .section(MenuSectionKey::THEME, empty_section);

        assert_eq!(
            faults(&declaration, DesktopPlatform::Linux),
            vec![CommandFault::DuplicateSectionProvider {
                section: MenuSectionKey::THEME,
            }],
        );
    }

    #[test]
    fn every_independent_fault_is_reported_in_declaration_order() {
        let declaration = CommandsDeclaration::new()
            .command(Command::app(ALPHA, Alpha, ok).binding(CommandBinding::new(
                None,
                Some("ctrl-nope-x"),
                None,
            )))
            .menu_bar(MenuBar::custom(vec![
                Menu::keyed(MenuKey::EDIT).command(BETA),
            ]));

        assert_eq!(
            faults(&declaration, DesktopPlatform::Linux),
            vec![
                CommandFault::InvalidBinding {
                    command: ALPHA,
                    platform: DesktopPlatform::Windows,
                    binding: "ctrl-nope-x",
                },
                CommandFault::UnknownCommand {
                    menu: MenuKey::EDIT,
                    command: BETA,
                },
            ],
            "validation does not stop at the first fault",
        );
    }

    #[test]
    fn one_command_projected_into_two_menus_lowers_into_both() {
        let outline = MenuBar::custom(vec![
            Menu::keyed(MenuKey::EDIT).command(ALPHA),
            Menu::keyed(MenuKey::WINDOW).command(BETA).command(ALPHA),
        ])
        .outline(DesktopPlatform::Linux)
        .expect("structurally valid");

        let lowered = lower_menus(&outline);
        assert_eq!(
            lowered.placements[&ALPHA],
            vec![
                MenuPlacement::new("Edit", 0, 0),
                MenuPlacement::new("Window", 0, 1),
            ],
            "a repeated projection contributes one placement per menu",
        );

        // ...and the plan projects that one runtime command into both menus.
        let mut alpha = crate::commands::RuntimeCommand::new(
            ALPHA,
            "Alpha",
            crate::commands::CommandScope::App,
            Alpha,
        );
        for placement in &lowered.placements[&ALPHA] {
            alpha = alpha.with_placement(*placement);
        }
        let commands = vec![
            alpha,
            crate::commands::RuntimeCommand::new(
                BETA,
                "Beta",
                crate::commands::CommandScope::Window,
                Beta,
            )
            .with_placement(lowered.placements[&BETA][0]),
        ];
        let projected = lowered.plan.outline(&commands);
        assert_eq!(projected[0].nodes, vec![PlanNode::Command(ALPHA)]);
        assert_eq!(
            projected[1].nodes,
            vec![PlanNode::Command(BETA), PlanNode::Command(ALPHA)],
        );
    }

    #[test]
    fn the_default_declaration_is_valid_on_every_platform() {
        for platform in DesktopPlatform::ALL {
            let outline = CommandsDeclaration::new()
                .validate(platform)
                .unwrap_or_else(|faults| {
                    panic!("default declaration is invalid on {platform:?}: {faults}")
                });
            assert!(
                !outline.menus().is_empty(),
                "the default standard bar projects menus on {platform:?}",
            );
        }
    }

    /// Anti-drift guard: the framework seeds exactly the ids its own standard
    /// menus reference. If a standard menu gains a command, `framework_commands`
    /// must gain it too or the default declaration stops validating.
    #[test]
    fn the_standard_layout_references_only_framework_seeded_commands() {
        for platform in DesktopPlatform::ALL {
            let seeded: HashSet<CommandId> = standard::framework_command_ids(platform)
                .into_iter()
                .collect();
            let outline = CommandsDeclaration::new()
                .validate(platform)
                .expect("the default declaration is valid");
            for menu in outline.menus() {
                for node in menu.nodes() {
                    if let MenuNode::Command(id) = node {
                        assert!(
                            seeded.contains(id),
                            "{platform:?} standard menus reference {id:?}, which the framework never installs",
                        );
                    }
                }
            }
        }
    }

    /// About and Settings are deliberately *not* framework-seeded: the app owns
    /// the handler, so opting into those surfaces without declaring the command
    /// is a reported fault rather than a dead menu entry.
    #[test]
    fn optional_standard_references_require_an_app_declared_command() {
        let layout = crate::commands::menu_model::StandardLayout::new()
            .with_about()
            .with_settings();
        let faults = CommandsDeclaration::new()
            .menu_bar(MenuBar::from_standard_layout(layout))
            .validate(DesktopPlatform::MacOs)
            .expect_err("opting in without declaring the commands is a fault");

        let unknown: Vec<CommandId> = faults
            .iter()
            .filter_map(|fault| match fault {
                CommandFault::UnknownCommand { command, .. } => Some(*command),
                _ => None,
            })
            .collect();
        assert_eq!(
            unknown,
            vec![
                standard::ABOUT_COMMAND_ID,
                standard::OPEN_SETTINGS_COMMAND_ID,
            ],
        );
    }

    #[gpui::test]
    fn the_default_declaration_installs_the_framework_vocabulary(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            CommandsDeclaration::new()
                .install(DesktopPlatform::current(), cx)
                .expect("the default declaration installs");

            let registry = cx.global::<CommandRegistry>();
            for id in standard::framework_command_ids(DesktopPlatform::current()) {
                assert_eq!(
                    registrations(registry, id),
                    1,
                    "{id:?} is installed exactly once by the framework",
                );
            }
        });
    }

    #[gpui::test]
    fn an_app_command_may_not_shadow_a_framework_command(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let result = CommandsDeclaration::new()
                .command(Command::app(standard::QUIT_COMMAND_ID, Alpha, ok))
                .menu_bar(MenuBar::none())
                .install(DesktopPlatform::MacOs, cx);

            let Err(InstallError::Declaration(faults)) = result else {
                panic!("shadowing a framework command is a declaration fault");
            };
            assert!(faults.iter().any(|fault| matches!(
                fault,
                CommandFault::DuplicateCommand { command } if *command == standard::QUIT_COMMAND_ID
            )));
            assert!(
                cx.command_registry().is_none(),
                "a rejected declaration never creates the registry",
            );
        });
    }

    #[gpui::test]
    fn a_declared_menu_label_survives_lowering_and_projection(cx: &mut gpui::TestAppContext) {
        fn dynamic_title(_: &App) -> gpui::SharedString {
            "Resolved".into()
        }

        cx.update(|cx| {
            let plan = CommandsDeclaration::new()
                .command(Command::app(ALPHA, Alpha, ok).label("Alpha"))
                .menu_bar(MenuBar::custom(vec![
                    // Key and title differ on purpose: the key is the stable
                    // identity, the label is what the user reads.
                    Menu::new(MenuKey::EDIT, "Bearbeiten").command(ALPHA),
                    Menu::new(MenuKey::VIEW, MenuLabel::derived(dynamic_title)).command(ALPHA),
                ]))
                .install(DesktopPlatform::Linux, cx)
                .expect("a valid declaration installs");

            cx.update_global::<CommandRegistry, _>(|registry, _| registry.set_plan(plan));
            let registry = cx.global::<CommandRegistry>();
            let menus = crate::commands::menu::build_menus(cx, registry).expect("a plan projects");
            let titles: Vec<&str> = menus.iter().map(|menu| menu.name.as_ref()).collect();
            assert_eq!(
                titles,
                vec!["Bearbeiten", "Resolved"],
                "declared titles reach the projection; derived titles resolve against app state",
            );
        });
    }

    #[gpui::test]
    fn a_collision_on_a_later_command_installs_nothing(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            // A pre-registered command outside the declaration.
            cx.register_command(super::super::RuntimeCommand::new(
                BETA,
                "Existing",
                crate::commands::CommandScope::Window,
                Beta,
            ))
            .expect("the pre-existing command registers");
            let before = cx.global::<CommandRegistry>().commands().len();

            let result = two_commands()
                .menu_bar(MenuBar::none())
                .install(DesktopPlatform::Linux, cx);
            let Err(InstallError::Runtime(CommandError::Duplicate { command })) = result else {
                panic!("colliding with a live command is a runtime failure");
            };
            assert_eq!(command, BETA);

            let registry = cx.global::<CommandRegistry>();
            assert_eq!(
                registry.commands().len(),
                before,
                "the first command of a rejected batch is not registered",
            );
            assert_eq!(registrations(registry, ALPHA), 0);
            assert_eq!(
                registry.get(BETA).expect("registered").label().as_ref(),
                "Existing",
                "the live command is untouched",
            );
            assert!(
                registry.plan().is_none(),
                "a rejected install never projects",
            );

            // The rejected declaration's handler was never installed, so
            // dispatching its action is inert.
            cx.dispatch_action(&Alpha);
        });
    }

    /// Issue #11: an action type names exactly one stable command id. Two
    /// commands in the *same* batch that share an action type must be
    /// rejected together with the rest of the batch — before either reaches
    /// the registry — not silently accepted as two ids for one action.
    #[gpui::test]
    fn two_commands_sharing_one_action_type_install_nothing(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let result = CommandsDeclaration::new()
                .command(Command::app(ALPHA, Alpha, ok))
                .command(Command::app(BETA, Alpha, ok))
                .menu_bar(MenuBar::none())
                .install(DesktopPlatform::Linux, cx);

            let Err(InstallError::Runtime(CommandError::IncompatibleAction { command, .. })) =
                result
            else {
                panic!("two ids sharing one action type is a runtime failure");
            };
            assert_eq!(command, BETA, "the second command in the batch is blamed");
            assert!(
                cx.command_registry().is_none(),
                "a rejected batch never creates the registry",
            );
        });
    }

    #[gpui::test]
    fn installing_a_multi_menu_command_registers_it_once_with_every_placement(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| {
            let plan = CommandsDeclaration::new()
                .command(Command::app(ALPHA, Alpha, ok))
                .menu_bar(MenuBar::custom(vec![
                    Menu::keyed(MenuKey::EDIT).command(ALPHA),
                    Menu::keyed(MenuKey::WINDOW).command(ALPHA),
                ]))
                .install(DesktopPlatform::Linux, cx)
                .expect("a repeated projection is valid");

            let registry = cx.global::<CommandRegistry>();
            assert_eq!(
                registrations(registry, ALPHA),
                1,
                "a command projected twice is still one command",
            );
            assert_eq!(
                registry.get(ALPHA).expect("registered").placements(),
                [
                    MenuPlacement::new("Edit", 0, 0),
                    MenuPlacement::new("Window", 0, 0),
                ],
            );
            assert_eq!(
                registry.get(ALPHA).expect("registered").placement(),
                Some(MenuPlacement::new("Edit", 0, 0)),
                "the compatibility accessor still reports the first placement",
            );

            let projected = plan.outline(registry.commands());
            assert_eq!(projected[0].nodes, vec![PlanNode::Command(ALPHA)]);
            assert_eq!(projected[1].nodes, vec![PlanNode::Command(ALPHA)]);
        });
    }

    #[test]
    fn separators_become_groups_and_sections_keep_their_slot() {
        let outline = MenuBar::custom(vec![
            Menu::keyed(MenuKey::EDIT)
                .command(ALPHA)
                .separator()
                .command(BETA)
                .section(MenuSectionKey::THEME),
        ])
        .outline(DesktopPlatform::Linux)
        .expect("valid");

        let lowered = lower_menus(&outline);
        assert_eq!(
            lowered.placements[&ALPHA],
            vec![MenuPlacement::new("Edit", 0, 0)]
        );
        assert_eq!(
            lowered.placements[&BETA],
            vec![MenuPlacement::new("Edit", 1, 0)]
        );

        // The plan reserves the section in the same group as `beta`, one slot
        // later, so the projected order matches the declared order.
        let commands = vec![
            crate::commands::RuntimeCommand::new(
                ALPHA,
                "Alpha",
                crate::commands::CommandScope::App,
                Alpha,
            )
            .with_placement(lowered.placements[&ALPHA][0]),
            crate::commands::RuntimeCommand::new(
                BETA,
                "Beta",
                crate::commands::CommandScope::Window,
                Beta,
            )
            .with_placement(lowered.placements[&BETA][0]),
        ];
        assert_eq!(
            lowered.plan.outline(&commands)[0].nodes,
            vec![
                PlanNode::Command(ALPHA),
                PlanNode::Separator,
                PlanNode::Command(BETA),
                PlanNode::Section(MenuSectionKey::THEME.as_str()),
            ],
        );
    }

    #[test]
    fn the_dock_projection_is_not_a_top_level_menu() {
        let outline = MenuBar::custom(vec![
            Menu::keyed(MenuKey::EDIT).command(ALPHA),
            Menu::keyed(MenuKey::DOCK).command(BETA),
        ])
        .outline(DesktopPlatform::MacOs)
        .expect("valid");

        let lowered = lower_menus(&outline);
        assert_eq!(
            lowered
                .plan
                .outline(&[])
                .iter()
                .map(|menu| menu.key)
                .collect::<Vec<_>>(),
            vec!["Edit"],
        );
        assert_eq!(
            lowered.placements[&BETA],
            vec![MenuPlacement::new("Dock", 0, 0)],
            "dock commands still carry a placement for the dock projection",
        );
    }

    #[gpui::test]
    fn installing_lowers_commands_sections_and_the_plan(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let plan = two_commands()
                .menu_bar(MenuBar::custom(vec![
                    Menu::keyed(MenuKey::EDIT)
                        .command(ALPHA)
                        .section(MenuSectionKey::THEME),
                    Menu::keyed(MenuKey::WINDOW).command(BETA),
                ]))
                .section(MenuSectionKey::THEME, empty_section)
                .install(DesktopPlatform::Linux, cx)
                .expect("a valid declaration installs");

            let registry = cx.global::<CommandRegistry>();
            assert_eq!(registrations(registry, ALPHA), 1);
            assert_eq!(registrations(registry, BETA), 1);
            assert!(registry.section(MenuSectionKey::THEME.as_str()).is_some());
            assert_eq!(
                plan.outline(registry.commands())
                    .iter()
                    .map(|menu| menu.key)
                    .collect::<Vec<_>>(),
                vec!["Edit", "Window"],
            );
        });
    }

    #[gpui::test]
    fn installing_an_invalid_declaration_touches_nothing(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let result = CommandsDeclaration::new()
                .menu_bar(MenuBar::none())
                .command(Command::app(ALPHA, Alpha, ok))
                .command(Command::window(ALPHA, Beta))
                .install(DesktopPlatform::Linux, cx);

            // `MenuPlan` is not `Debug`, so match rather than `expect_err`.
            let Err(error) = result else {
                panic!("duplicate ids are rejected");
            };
            assert!(matches!(error, InstallError::Declaration(_)));
            assert!(
                cx.command_registry()
                    .is_none_or(|r| r.commands().is_empty()),
                "validation runs before any runtime mutation",
            );
        });
    }

    // ---- Resolved standard features.

    fn open_nothing(_: &mut App) -> anyhow::Result<()> {
        Ok(())
    }

    fn all_features() -> StandardFeatures {
        StandardFeatures {
            settings: Some(open_nothing),
            about: Some(open_nothing),
            theme: true,
        }
    }

    #[test]
    fn standard_feature_commands_appear_only_once_the_declaration_resolves_them() {
        for platform in DesktopPlatform::ALL {
            let bare = CommandsDeclaration::new()
                .validate(platform)
                .expect("the default declaration is valid");
            assert!(
                !bare.command_ids().contains(&standard::ABOUT_COMMAND_ID),
                "About is projected only when the feature resolves on {platform:?}",
            );
            assert!(
                !bare
                    .command_ids()
                    .contains(&standard::OPEN_SETTINGS_COMMAND_ID),
                "Settings is projected only when a Settings surface exists on {platform:?}",
            );

            let resolved = CommandsDeclaration::new()
                .standard_features(all_features())
                .validate(platform)
                .unwrap_or_else(|faults| {
                    panic!("resolved features must validate on {platform:?}: {faults}")
                });
            assert!(
                resolved.command_ids().contains(&standard::ABOUT_COMMAND_ID),
                "About is a desktop convention on {platform:?}",
            );
            assert!(
                resolved
                    .command_ids()
                    .contains(&standard::OPEN_SETTINGS_COMMAND_ID),
                "a declared Settings surface activates the standard command on {platform:?}",
            );
            assert!(
                resolved
                    .section_slots()
                    .iter()
                    .any(|(_, section)| *section == MenuSectionKey::THEME),
                "the theme convention reserves its Appearance slot on {platform:?}",
            );
        }
    }

    /// The Settings feature alone must not drag About in, or the other way
    /// round: each is resolved independently by the declaration.
    #[test]
    fn each_standard_feature_is_projected_independently() {
        let settings_only = StandardFeatures {
            settings: Some(open_nothing),
            ..StandardFeatures::default()
        };
        for platform in DesktopPlatform::ALL {
            let outline = CommandsDeclaration::new()
                .standard_features(settings_only)
                .validate(platform)
                .expect("a Settings-only declaration is valid");
            assert!(
                outline
                    .command_ids()
                    .contains(&standard::OPEN_SETTINGS_COMMAND_ID),
            );
            assert!(!outline.command_ids().contains(&standard::ABOUT_COMMAND_ID));
            assert!(outline.section_slots().is_empty());
        }
    }

    /// The stranding rule reads the *resolved* features, not just the layout's
    /// explicit opt-ins: a declaration that earns About from a default policy
    /// and then hides the menu holding it is as contradictory as one that
    /// called `with_about()`.
    #[test]
    fn hiding_a_menu_strands_a_feature_the_declaration_resolved() {
        for platform in DesktopPlatform::ALL {
            let layout = crate::commands::menu_model::StandardLayout::new().hide(MenuKey::HELP);
            let result = CommandsDeclaration::new()
                .standard_features(all_features())
                .menu_bar(MenuBar::from_standard_layout(layout))
                .validate(platform);

            match platform {
                DesktopPlatform::MacOs => {
                    result.unwrap_or_else(|faults| {
                        panic!("macOS does not place About in Help: {faults}")
                    });
                }
                DesktopPlatform::Windows | DesktopPlatform::Linux => {
                    let faults = result.expect_err("About loses its only placement");
                    assert_eq!(
                        faults.iter().copied().collect::<Vec<_>>(),
                        vec![CommandFault::StrandedStandardFeature {
                            menu: MenuKey::HELP,
                            feature: "About",
                        }],
                    );
                }
            }
        }
    }

    #[gpui::test]
    fn resolved_feature_commands_install_once_with_framework_owned_metadata(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| {
            CommandsDeclaration::new()
                .standard_features(all_features())
                .install(DesktopPlatform::current(), cx)
                .expect("a declaration with resolved features installs");

            let registry = cx.global::<CommandRegistry>();
            for id in standard::feature_command_ids(all_features()) {
                assert_eq!(
                    registrations(registry, id),
                    1,
                    "{id:?} is installed exactly once",
                );
            }
            let settings = registry
                .get(standard::OPEN_SETTINGS_COMMAND_ID)
                .expect("Settings is registered");
            assert!(
                settings.default_binding.is_some(),
                "the standard Settings shortcut stays framework-owned",
            );
            assert!(
                !settings.placements.is_empty(),
                "placement comes from the resolved outline, not the application",
            );
            let about = registry
                .get(standard::ABOUT_COMMAND_ID)
                .expect("About is registered");
            assert!(
                about.derived_label.is_some(),
                "About renders `About <App>` from the resolved identity",
            );
        });
    }
}
