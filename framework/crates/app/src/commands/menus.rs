//! [`MenusModule`]: installs the standard command set, registers default
//! keybindings, and keeps the native menu bar in sync with the registry.
//!
//! Reactive rebuild: the module observes the [`neutron_components::Theme`] global so
//! checked state (appearance mode, active theme) refreshes automatically, and
//! exposes [`menus_invalidate`] for other modules (theme registry watcher, window
//! manager) to call when their contributing sections change.
//!
//! Keybinding precedence (component < shell < app < user): the component tier is
//! bound by `neutron_components::init` before this module runs; default bindings are
//! then registered from the registry in registration order, and GPUI resolves
//! later-registered bindings first — so shell/app defaults override component
//! defaults, and a future user keymap (registered last) will override both.

use gpui::{
    Action, App, DummyKeyboardMapper, KeyBinding, KeyBindingContextPredicate, KeyBindingMetaIndex,
    Subscription,
};

use super::declaration::{CommandsDeclaration, InstallError};
use super::standard::DesktopPlatform;
use super::{CommandError, CommandId, CommandRegistry, RuntimeCommand, menu};
use crate::error::AppShellError;
use crate::handles::{AppInfo, AppProxy};
use crate::module::RuntimeModule;

/// Meta tag stamped on every keybinding this module installs, so a rebuild can
/// tell registry-owned bindings apart from the component tier (bound by
/// `neutron_components::init`) and any future user-keymap tier — neither of which we
/// own — and drop only ours before re-adding the current registry.
const COMMANDS_BINDING_META: KeyBindingMetaIndex = KeyBindingMetaIndex(0xC0FFEE);

/// Installs and maintains the native menu bar from the command registry.
pub(crate) struct MenusModule {
    declared: Option<(CommandsDeclaration, DesktopPlatform)>,
    theme_observer: Option<Subscription>,
}

impl MenusModule {
    /// A module driving one typed [`CommandsDeclaration`].
    ///
    /// The declaration installs the framework vocabulary itself, so it is the
    /// only menu owner: the framework commands are registered exactly once.
    /// Internal seam for declaration lowering.
    pub(crate) fn declared(declaration: CommandsDeclaration, platform: DesktopPlatform) -> Self {
        Self {
            declared: Some((declaration, platform)),
            theme_observer: None,
        }
    }
}

impl RuntimeModule for MenusModule {
    fn id(&self) -> &'static str {
        "menus"
    }

    fn init(
        &mut self,
        cx: &mut App,
        _info: &AppInfo,
        _proxy: &AppProxy,
    ) -> Result<(), AppShellError> {
        ensure_menu_owner_available(cx)?;

        let (declaration, platform) = self
            .declared
            .take()
            .expect("MenusModule::init runs at most once");
        let plan = declaration
            .install(platform, cx)
            .map_err(declared_startup_error)?;

        // Shell/app-tier default bindings, in registration order (component tier
        // was bound by neutron_components::init already).
        install_bindings(cx).map_err(command_startup_error)?;

        cx.global_mut::<CommandRegistry>().set_plan(plan);

        // Initial projection, then rebuild whenever the theme changes.
        menus_invalidate(cx);

        // Mark the registry live: past this point apps can only register commands
        // from `on_launch` (or later), so those registrations must bind and
        // re-project themselves — see `AppCommandsExt::register_command`.
        cx.global_mut::<CommandRegistry>().activate();

        self.theme_observer =
            Some(cx.observe_global::<neutron_components::Theme>(|cx| menus_invalidate(cx)));
        Ok(())
    }
}

fn ensure_menu_owner_available(cx: &App) -> Result<(), AppShellError> {
    if cx.has_global::<CommandRegistry>() && cx.global::<CommandRegistry>().is_active() {
        return Err(AppShellError::Module {
            module: "menus",
            source: anyhow::anyhow!("an application menu owner is already initialized"),
        });
    }
    Ok(())
}

fn declared_startup_error(source: InstallError) -> AppShellError {
    AppShellError::Module {
        module: "menus",
        source: anyhow::Error::new(source),
    }
}

fn command_startup_error(source: CommandError) -> AppShellError {
    AppShellError::Module {
        module: "menus",
        source: anyhow::Error::new(source),
    }
}

/// Bind a *newly added* command's default keybinding (if any) and re-project the
/// menus.
///
/// Append-only fast path for a brand-new command id registered after the initial
/// pass (the common `on_launch` case). The binding lands after the shell defaults
/// and below any future user overrides — preserving precedence. For *replacing*
/// an existing id, use [`rebuild_bindings_and_reproject`], which also removes the
/// replaced command's now-stale chord.
pub(super) fn bind_and_reproject(cx: &mut App, id: CommandId) -> Result<(), CommandError> {
    if let Some(binding) = binding_for(cx, id)? {
        cx.bind_keys([binding]);
    }
    menus_invalidate(cx);
    Ok(())
}

/// Rebuild the registry's keybindings from scratch and re-project the menus.
///
/// Used when an already-registered id is replaced after the initial pass: the
/// old command's binding is still live in the keymap, and the fork exposes no
/// per-binding removal — only `clear_key_bindings` (all) plus `bind_keys`. So we
/// snapshot the whole keymap, drop the bindings we own (tagged), and restore the
/// foreign ones (component tier, any user tier) *in place* around a freshly
/// rebuilt registry: those installed before our tier stay before it, those
/// installed after stay after (see [`split_around_registry`]). Net effect: stale
/// registry chords disappear while every foreign tier keeps its bindings, order,
/// and precedence relative to ours.
pub(super) fn rebuild_bindings_and_reproject(cx: &mut App) -> Result<(), CommandError> {
    let snapshot: Vec<KeyBinding> = {
        let keymap = cx.key_bindings();
        let keymap = keymap.borrow();
        keymap.bindings().cloned().collect()
    };
    let (before, after) = split_around_registry(snapshot);
    cx.clear_key_bindings();
    cx.bind_keys(before);
    install_bindings(cx)?;
    cx.bind_keys(after);
    menus_invalidate(cx);
    Ok(())
}

/// Split a keymap snapshot around the registry tier, dropping the registry-owned
/// (tagged) bindings so they can be rebuilt fresh.
///
/// Foreign bindings that appeared *before* the first registry-owned binding are
/// returned in `before`; every other foreign binding — including any interleaved
/// with or following the registry tier — is returned in `after`. Rebuilding as
/// `before + registry + after` preserves each foreign binding's position, and
/// thus precedence, relative to our tier.
fn split_around_registry(snapshot: Vec<KeyBinding>) -> (Vec<KeyBinding>, Vec<KeyBinding>) {
    let mut before = Vec::new();
    let mut after = Vec::new();
    let mut seen_registry = false;
    for binding in snapshot {
        if binding.meta() == Some(COMMANDS_BINDING_META) {
            seen_registry = true;
            continue;
        }
        if seen_registry {
            after.push(binding);
        } else {
            before.push(binding);
        }
    }
    (before, after)
}

/// Build the [`KeyBinding`] for a registered command's default binding, if it has
/// one.
fn binding_for(cx: &App, id: CommandId) -> Result<Option<KeyBinding>, CommandError> {
    let registry = cx.global::<CommandRegistry>();
    let Some(command) = registry.get(id) else {
        return Ok(None);
    };
    binding_of(command)
}

/// Build the [`KeyBinding`] for `command`'s default binding, if it has one,
/// scoped to its key context when one is declared.
fn binding_of(command: &RuntimeCommand) -> Result<Option<KeyBinding>, CommandError> {
    let Some(keystroke) = command.default_binding() else {
        return Ok(None);
    };
    load_binding(
        command.id(),
        keystroke,
        command.binding_context(),
        command.boxed_action(),
    )
    .map(Some)
}

/// Construct a keybinding from a keystroke, optional key-context predicate, and
/// boxed action, tagged as registry-owned so a rebuild can identify and replace
/// it.
///
/// A command without a context binds globally, preserving the pre-typed-model
/// behavior.
fn load_binding(
    id: CommandId,
    keystroke: &'static str,
    context: Option<&'static str>,
    action: Box<dyn Action>,
) -> Result<KeyBinding, CommandError> {
    let predicate = match context {
        Some(context) => Some(std::rc::Rc::new(
            KeyBindingContextPredicate::parse(context).map_err(|source| {
                CommandError::InvalidKeyContext {
                    command: id,
                    context,
                    source,
                }
            })?,
        )),
        None => None,
    };
    KeyBinding::load(
        keystroke,
        action,
        predicate,
        false,
        None,
        &DummyKeyboardMapper,
    )
    .map(|binding| binding.with_meta(COMMANDS_BINDING_META))
    .map_err(|source| CommandError::InvalidBinding {
        command: id,
        binding: keystroke,
        source,
    })
}

pub(super) fn validate_command_binding(command: &RuntimeCommand) -> Result<(), CommandError> {
    binding_of(command).map(drop)
}

/// Re-project the registry into the native menu bar (and the macOS dock menu).
///
/// Call this from any module whose contributing section or command state changed
/// (theme registry updates, window list changes, enabled/checked transitions).
/// A no-op before the registry exists.
pub(crate) fn menus_invalidate(cx: &mut App) {
    if !cx.has_global::<CommandRegistry>() {
        return;
    }
    let menus = {
        let registry = cx.global::<CommandRegistry>();
        menu::build_menus(cx, registry)
    };
    if let Some(menus) = menus {
        cx.set_menus(menus);
    }
    reload_menu_bars(cx);

    #[cfg(target_os = "macos")]
    {
        // Set unconditionally: an empty projection must clear a previously
        // installed dock menu (e.g. the last dock-placed command was moved off
        // the dock), otherwise the stale native item stays live.
        let items = {
            let registry = cx.global::<CommandRegistry>();
            menu::build_dock_items(cx, registry)
        };
        cx.set_dock_menu(items);
    }
}

/// Register default keybindings from the registry, in registration order. GPUI
/// resolves bindings in reverse insertion order, so later registrations win.
fn install_bindings(cx: &mut App) -> Result<(), CommandError> {
    let bindings: Result<Vec<KeyBinding>, CommandError> = {
        let registry = cx.global::<CommandRegistry>();
        registry
            .commands()
            .iter()
            .filter_map(|command| binding_of(command).transpose())
            .collect()
    };
    cx.bind_keys(bindings?);
    Ok(())
}

fn reload_menu_bars(cx: &mut App) {
    let menu_bars = cx.global_mut::<CommandRegistry>().take_menu_bars();
    let mut live = Vec::with_capacity(menu_bars.len());
    for menu_bar in menu_bars {
        if menu_bar
            .update(cx, |menu_bar, cx| menu_bar.reload(cx))
            .is_ok()
        {
            live.push(menu_bar);
        }
    }
    cx.global_mut::<CommandRegistry>().replace_menu_bars(live);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::standard;
    use crate::commands::{
        APP_MENU, AppCommandsExt, CommandId, CommandRegistry, CommandScope, EDIT_MENU,
        RuntimeCommand,
    };
    use gpui::actions;

    actions!(menus_test, [A, B, C]);

    /// The pure ordered list of (id, keystroke) that `install_bindings` feeds to
    /// `bind_keys`, mirroring registration order (later = higher precedence).
    fn binding_order(registry: &CommandRegistry) -> Vec<(CommandId, &'static str)> {
        registry
            .commands()
            .iter()
            .filter_map(|c| c.default_binding().map(|k| (c.id(), k)))
            .collect()
    }

    /// Install the shell global and return what a module's `init` receives.
    fn services(cx: &mut App) -> (AppInfo, AppProxy) {
        use std::sync::Arc;

        use neutron_components_storage::{AppPaths, PathLayout};

        use crate::handles::PendingEvents;
        use crate::liveness::{ExitPolicy, InitialActivation, Liveness};
        use crate::{PlatformCapabilities, handles};

        let info = AppInfo::new(
            crate::declaration::tests::identity(),
            AppPaths::new("appshell-declared-menus", PathLayout::PlatformDefault)
                .expect("test paths resolve"),
            PlatformCapabilities::detect(),
        );
        let proxy = handles::install(
            cx,
            info.clone(),
            Liveness::new(ExitPolicy::Explicit, InitialActivation::Passive),
            Vec::new(),
            Vec::new(),
            Arc::new(PendingEvents::default()),
            Box::new(|_, _| {}),
            None,
        );
        (info, proxy)
    }

    #[gpui::test]
    fn the_declared_path_installs_the_framework_vocabulary_exactly_once(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| {
            let (info, proxy) = services(cx);
            MenusModule::declared(CommandsDeclaration::new(), DesktopPlatform::MacOs)
                .init(cx, &info, &proxy)
                .expect("the default declaration installs cleanly");

            let registry = cx.global::<CommandRegistry>();
            let quit = registry
                .commands()
                .iter()
                .filter(|command| command.id() == standard::QUIT_COMMAND_ID)
                .count();
            assert_eq!(
                quit, 1,
                "the declared path installs the framework vocabulary exactly once",
            );
            assert!(
                registry
                    .commands()
                    .iter()
                    .any(|command| command.id() == standard::COPY_COMMAND_ID),
                "the whole framework vocabulary is installed by the declaration",
            );
        });
    }

    #[gpui::test]
    fn a_faulty_declaration_fails_module_init_as_a_service_error(cx: &mut gpui::TestAppContext) {
        use crate::commands::Command;

        fn run(_action: &A, _cx: &mut App) -> anyhow::Result<()> {
            Ok(())
        }

        cx.update(|cx| {
            let (info, proxy) = services(cx);
            let declaration =
                CommandsDeclaration::new().command(Command::app(standard::QUIT_COMMAND_ID, A, run));

            let error = MenusModule::declared(declaration, DesktopPlatform::MacOs)
                .init(cx, &info, &proxy)
                .expect_err("shadowing a framework command is a fault");

            assert!(
                matches!(
                    error,
                    AppShellError::Module {
                        module: "menus",
                        ..
                    }
                ),
                "a declared menu fault is a menus module failure: {error:?}",
            );
        });
    }

    #[test]
    fn split_around_registry_preserves_foreign_positions() {
        // component tier (before) | two registry (tagged) | app raw bind (after).
        let snapshot = vec![
            KeyBinding::new("ctrl-a", A, None),
            load_binding(CommandId("b1"), "cmd-1", None, Box::new(B)).unwrap(),
            load_binding(CommandId("b2"), "cmd-2", None, Box::new(B)).unwrap(),
            KeyBinding::new("ctrl-b", C, None),
        ];
        let (before, after) = split_around_registry(snapshot);
        let names = |v: &[KeyBinding]| v.iter().map(|b| b.action().name()).collect::<Vec<_>>();
        // Registry-owned bindings dropped; foreign bindings keep their side.
        assert_eq!(names(&before), vec![A.name()]);
        assert_eq!(names(&after), vec![C.name()]);
    }

    #[test]
    fn bindings_follow_registration_order() {
        let mut registry = CommandRegistry::new();
        registry
            .register(
                RuntimeCommand::new(CommandId("a"), "A", CommandScope::App, A)
                    .with_binding("cmd-k")
                    .with_placement(super::super::MenuPlacement::new(APP_MENU, 0, 0)),
            )
            .unwrap();
        // No binding -> excluded.
        registry
            .register(RuntimeCommand::new(
                CommandId("b"),
                "B",
                CommandScope::App,
                B,
            ))
            .unwrap();
        registry
            .register(
                RuntimeCommand::new(CommandId("c"), "C", CommandScope::App, C)
                    .with_binding("cmd-k")
                    .with_placement(super::super::MenuPlacement::new(EDIT_MENU, 0, 0)),
            )
            .unwrap();

        // "c" comes after "a": it is registered later, so it wins the shared
        // "cmd-k" chord under GPUI's reverse-order resolution.
        assert_eq!(
            binding_order(&registry),
            vec![(CommandId("a"), "cmd-k"), (CommandId("c"), "cmd-k")]
        );
    }

    #[test]
    fn post_activation_registration_exposes_binding_and_placement() {
        use crate::commands::MenuPlacement;

        let mut registry = CommandRegistry::new();
        // Initial batch, bound by `install_bindings`.
        registry
            .register(
                RuntimeCommand::new(CommandId("std"), "Std", CommandScope::App, A)
                    .with_binding("cmd-1"),
            )
            .unwrap();
        assert!(!registry.is_active());

        // The menu module has finished its initial pass.
        registry.activate();
        assert!(registry.is_active());

        // An app registers a new command from `on_launch` (post-activation). The
        // ext path would call `bind_and_reproject`; the registry state it reads is
        // what we assert here.
        registry
            .register(
                RuntimeCommand::new(CommandId("late"), "Late", CommandScope::App, B)
                    .with_binding("cmd-2")
                    .with_placement(MenuPlacement::new(APP_MENU, 0, 0)),
            )
            .unwrap();

        // The late binding is present, ordered after the initial one (so it wins
        // a shared chord and sits in the app tier).
        assert_eq!(
            binding_order(&registry),
            vec![(CommandId("std"), "cmd-1"), (CommandId("late"), "cmd-2")]
        );
        // And the late command's placement now appears in the projected outline.
        let outline = menu::MenuPlan::standard().outline(registry.commands());
        assert!(
            outline[0]
                .nodes
                .contains(&menu::MenuPlanNode::Command(CommandId("late")))
        );
    }

    #[test]
    fn app_command_ext_creates_registry_lazily() {
        // The ext trait's lazy-create path is covered structurally: with no
        // global, command_registry is None; the mutation path is exercised by the
        // headless integration test.
        fn assert_impls<T: AppCommandsExt>() {}
        assert_impls::<App>();
    }

    /// Regression: replacing a live command must drop the previous command's
    /// keybinding. An append-only rebind would leave the old chord installed and
    /// still dispatching, so the replaced command would answer two chords.
    #[gpui::test]
    fn replacing_a_live_command_drops_its_stale_binding(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            cx.set_global(CommandRegistry::new());
            crate::commands::install_declared_commands(
                cx,
                vec![
                    RuntimeCommand::new(CommandId("a"), "A", CommandScope::App, A)
                        .with_binding("cmd-1"),
                ],
            )
            .expect("the initial command installs");
            install_bindings(cx).expect("initial binding pass succeeds");
            cx.global_mut::<CommandRegistry>().activate();

            assert_eq!(chords_bound_to(cx, &A), vec!["cmd-1"]);

            crate::commands::replace_declared_command(
                cx,
                RuntimeCommand::new(CommandId("a"), "A", CommandScope::App, A)
                    .with_binding("cmd-2"),
            )
            .expect("replacing a live command succeeds");

            assert_eq!(
                chords_bound_to(cx, &A),
                vec!["cmd-2"],
                "the replaced command's old chord is gone, not merely shadowed",
            );
        });
    }

    /// The chords currently bound to `action`, as rendered keystrokes.
    fn chords_bound_to(cx: &App, action: &dyn gpui::Action) -> Vec<String> {
        let keymap = cx.key_bindings();
        let keymap = keymap.borrow();
        let mut chords: Vec<String> = keymap
            .bindings_for_action(action)
            .map(|binding| {
                binding
                    .keystrokes()
                    .iter()
                    .map(|keystroke| keystroke.inner().unparse())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect();
        chords.sort();
        chords
    }

    #[gpui::test]
    fn active_registry_rejects_second_menu_owner(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            cx.set_global(CommandRegistry::new());
            cx.global_mut::<CommandRegistry>().activate();

            assert!(matches!(
                ensure_menu_owner_available(cx),
                Err(AppShellError::Module {
                    module: "menus",
                    ..
                })
            ));
        });
    }
}
