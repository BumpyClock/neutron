//! Typed command declarations and their erasure onto the command registry
//! (issue #11).
//!
//! [`Command`] is generic over its GPUI action so an app-scoped command
//! can own a *typed* fallible handler (`fn(&Action, &mut App) -> Result<()>`)
//! instead of a boxed callback that has to downcast. A window-scoped command has
//! no handler at all: its action is dispatched to the focused view.
//!
//! Genericity stops at [`ErasedCommand`]. A declaration erases itself into that
//! object-safe trait, so the aggregate declaration — and, later,
//! `AppDeclaration` — never becomes generic no matter how many action types an
//! application declares.
//!
//! Lowering targets the command registry: each typed command becomes one
//! [`RuntimeCommand`] in the [`CommandRegistry`](super::CommandRegistry),
//! and an app-scoped handler is installed through
//! [`super::set_action_handler`], which keeps exactly one GPUI `on_action`
//! listener per action type (issue #11) so a later replacement swaps behavior
//! instead of accumulating a second listener; failures route through
//! [`RuntimeError::command`](crate::error::RuntimeError::command).

use gpui::{Action, App, OsAction};

use super::binding::CommandBinding;
use super::faults::CommandFault;
use super::label::CommandLabel;
use super::standard::DesktopPlatform;
use super::{CommandId, CommandScope, MenuPlacement, RuntimeCommand};

/// A typed command declaration.
///
/// Built with [`Command::app`] (typed handler, application scope) or
/// [`Command::window`] (dispatch to the focused view), then refined with
/// the chained builders.
pub struct Command<A: Action> {
    id: CommandId,
    action: A,
    handler: Option<fn(&A, &mut App) -> anyhow::Result<()>>,
    label: CommandLabel,
    binding: CommandBinding,
    enabled: Option<fn(&App) -> bool>,
    checked: Option<fn(&App) -> bool>,
    os_action: Option<OsAction>,
}

impl<A: Action> Command<A> {
    /// An application-scoped command: `handler` receives the typed action and
    /// may fail.
    ///
    /// A handler failure is reported as a nonfatal runtime error against this
    /// command's id and never aborts the application.
    pub fn app(id: CommandId, action: A, handler: fn(&A, &mut App) -> anyhow::Result<()>) -> Self {
        Self::declare(id, action, Some(handler))
    }

    /// A window-scoped command: the action is dispatched to the focused view and
    /// the shell registers no application handler.
    pub fn window(id: CommandId, action: A) -> Self {
        Self::declare(id, action, None)
    }

    fn declare(
        id: CommandId,
        action: A,
        handler: Option<fn(&A, &mut App) -> anyhow::Result<()>>,
    ) -> Self {
        Self {
            id,
            action,
            handler,
            // Until a label is declared the id is the only honest text we have.
            label: CommandLabel::text(id.as_str()),
            binding: CommandBinding::unbound(),
            enabled: None,
            checked: None,
            os_action: None,
        }
    }

    /// Set the presentation label (static or state-derived).
    #[must_use]
    pub fn label(mut self, label: impl Into<CommandLabel>) -> Self {
        self.label = label.into();
        self
    }

    /// Set the cross-platform binding.
    #[must_use]
    pub fn binding(mut self, binding: CommandBinding) -> Self {
        self.binding = binding;
        self
    }

    /// Set the enabled predicate. Absent means always enabled.
    #[must_use]
    pub fn enabled(mut self, predicate: fn(&App) -> bool) -> Self {
        self.enabled = Some(predicate);
        self
    }

    /// Set the checked predicate. Absent means unchecked.
    #[must_use]
    pub fn checked(mut self, predicate: fn(&App) -> bool) -> Self {
        self.checked = Some(predicate);
        self
    }

    /// Associate the OS action so the platform can supply specialized behavior
    /// (the macOS standard Edit items).
    #[must_use]
    #[allow(dead_code)] // Exercised only by unit tests; no production caller yet.
    pub(crate) fn os_action(mut self, os_action: OsAction) -> Self {
        self.os_action = Some(os_action);
        self
    }

    /// The stable identity this declaration claims.
    #[must_use]
    pub fn id(&self) -> CommandId {
        self.id
    }

    /// The dispatch scope implied by the presence of a typed handler.
    pub(crate) fn scope(&self) -> CommandScope {
        if self.handler.is_some() {
            CommandScope::App
        } else {
            CommandScope::Window
        }
    }
}

/// One erased command declaration.
///
/// Object-safe on purpose: the aggregate declaration keeps commands as
/// `Box<dyn ErasedCommand>` in declaration order, validates them in that order,
/// and lowers them in that order.
pub(crate) trait ErasedCommand: 'static {
    /// The stable identity this declaration claims.
    fn id(&self) -> CommandId;

    /// Where the command dispatches.
    #[allow(dead_code)] // Exercised only by unit tests; no production caller yet.
    fn scope(&self) -> CommandScope;

    /// Append this command's pure faults.
    ///
    /// Pure: no GPUI, no filesystem, no host-platform inspection.
    fn validate(&self, faults: &mut Vec<CommandFault>);

    /// Lower to runtime *values* without touching `cx`.
    ///
    /// Nothing is registered and no handler is installed here, so an aggregate
    /// install can build every command first and only mutate the app once the
    /// whole batch is known to be installable.
    ///
    /// `placements` carries *every* surface the command was projected into, so
    /// a command declared in two menus reaches the runtime as one command with
    /// two placements.
    ///
    /// Consuming (`self: Box<Self>`) so the typed action and handler move into
    /// the runtime without cloning, while the trait stays object-safe.
    fn lower(
        self: Box<Self>,
        platform: DesktopPlatform,
        placements: &[MenuPlacement],
    ) -> LoweredCommand;
}

/// One declaration lowered to runtime values, ready to install.
///
/// Splitting the runtime command from its handler is what makes an aggregate
/// install atomic: every command can be registered (or the whole batch
/// rejected) before the irreversible `cx.on_action` step runs.
pub(crate) struct LoweredCommand {
    command: RuntimeCommand,
    handler: Option<Box<dyn FnOnce(&mut App)>>,
}

impl LoweredCommand {
    /// The runtime command to register.
    #[allow(dead_code)] // Exercised only by unit tests; no production caller yet.
    pub(crate) fn command(&self) -> &RuntimeCommand {
        &self.command
    }

    /// Split into the registrable command and its deferred handler installer.
    pub(crate) fn into_parts(self) -> (RuntimeCommand, Option<Box<dyn FnOnce(&mut App)>>) {
        (self.command, self.handler)
    }
}

impl<A: Action> ErasedCommand for Command<A> {
    fn id(&self) -> CommandId {
        self.id
    }

    fn scope(&self) -> CommandScope {
        self.scope()
    }

    fn validate(&self, faults: &mut Vec<CommandFault>) {
        self.binding.validate(self.id, faults);
    }

    fn lower(
        self: Box<Self>,
        platform: DesktopPlatform,
        placements: &[MenuPlacement],
    ) -> LoweredCommand {
        let scope = self.scope();
        let Self {
            id,
            action,
            handler,
            label,
            binding,
            enabled,
            checked,
            os_action,
        } = *self;

        // A derived label cannot be flattened here: menus re-resolve it on every
        // invalidation, so the runtime command carries the function and keeps
        // the id as its static fallback.
        let static_label = label
            .static_text()
            .cloned()
            .unwrap_or_else(|| id.as_str().into());
        let mut command = RuntimeCommand::new(id, static_label, scope, action);
        if let Some(derived) = label.derived_fn() {
            command = command.with_derived_label(derived);
        }
        if let Some(keystroke) = binding.for_platform(platform) {
            command = command.with_binding(keystroke);
            if let Some(context) = binding.context() {
                command = command.with_binding_context(context);
            }
        }
        if let Some(os_action) = os_action {
            command = command.with_os_action(os_action);
        }
        if let Some(predicate) = enabled {
            command = command.with_enabled(predicate);
        }
        if let Some(predicate) = checked {
            command = command.with_checked(predicate);
        }
        for placement in placements {
            command = command.with_placement(*placement);
        }

        let handler = handler.map(|handler| {
            let install: Box<dyn FnOnce(&mut App)> = Box::new(move |cx: &mut App| {
                super::set_action_handler::<A>(cx, id, handler);
            });
            install
        });

        LoweredCommand { command, handler }
    }
}

#[cfg(test)]
mod tests {
    use gpui::actions;

    use super::*;
    use crate::commands::keys::MenuKey;

    actions!(declared_test, [Alpha, Beta]);

    fn ok(_: &Alpha, _: &mut App) -> anyhow::Result<()> {
        Ok(())
    }

    #[test]
    fn scope_follows_the_constructor_not_a_flag() {
        assert_eq!(
            Command::app(CommandId("a"), Alpha, ok).scope(),
            CommandScope::App,
        );
        assert_eq!(
            Command::window(CommandId("b"), Beta).scope(),
            CommandScope::Window,
        );
    }

    #[test]
    fn binding_faults_surface_through_erasure() {
        let command: Box<dyn ErasedCommand> = Box::new(
            Command::window(CommandId("b"), Beta).binding(CommandBinding::new(
                None,
                Some("ctrl-nope-b"),
                None,
            )),
        );
        let mut faults = Vec::new();
        command.validate(&mut faults);

        assert_eq!(
            faults,
            vec![CommandFault::InvalidBinding {
                command: CommandId("b"),
                platform: DesktopPlatform::Windows,
                binding: "ctrl-nope-b",
            }],
        );
        assert_eq!(command.id(), CommandId("b"));
    }

    #[test]
    fn lowering_projects_the_platform_chord_and_placement() {
        let command: Box<dyn ErasedCommand> = Box::new(
            Command::app(CommandId("a"), Alpha, ok)
                .label("Alpha")
                .binding(CommandBinding::platform("cmd-1", "ctrl-1")),
        );
        let lowered = command.lower(
            DesktopPlatform::Windows,
            &[MenuPlacement::new(MenuKey::EDIT.as_str(), 0, 0)],
        );

        let lowered = lowered.command();
        assert_eq!(lowered.label().as_ref(), "Alpha");
        assert_eq!(lowered.scope(), CommandScope::App);
        assert_eq!(
            lowered.default_binding(),
            Some("ctrl-1"),
            "the requested platform's chord is installed, not the host's",
        );
        assert_eq!(
            lowered.placement().map(|placement| placement.menu),
            Some(MenuKey::EDIT.as_str()),
        );
    }

    #[test]
    fn lowering_installs_nothing_until_the_handler_is_run() {
        let command: Box<dyn ErasedCommand> = Box::new(Command::app(CommandId("a"), Alpha, ok));
        let (_, handler) = command.lower(DesktopPlatform::current(), &[]).into_parts();
        assert!(
            handler.is_some(),
            "an app-scoped declaration defers its handler instead of installing it during lowering",
        );

        let window: Box<dyn ErasedCommand> =
            Box::new(Command::<Alpha>::window(CommandId("b"), Alpha));
        let (_, handler) = window.lower(DesktopPlatform::current(), &[]).into_parts();
        assert!(
            handler.is_none(),
            "a window-scoped declaration dispatches to the focused handler and installs none",
        );
    }

    #[gpui::test]
    fn a_failing_handler_is_reported_and_does_not_abort(cx: &mut gpui::TestAppContext) {
        fn fail(_: &Alpha, _: &mut App) -> anyhow::Result<()> {
            Err(anyhow::anyhow!("handler failed"))
        }

        cx.update(|cx| {
            let command: Box<dyn ErasedCommand> =
                Box::new(Command::app(CommandId("a"), Alpha, fail));
            let (_, handler) = command.lower(DesktopPlatform::current(), &[]).into_parts();
            handler.expect("an app-scoped declaration carries a handler")(cx);

            // No shell is installed, so `report_error` logs and returns; the
            // point of the assertion is that dispatch itself keeps running.
            cx.dispatch_action(&Alpha);
        });
    }
}
