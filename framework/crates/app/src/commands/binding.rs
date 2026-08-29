//! Host-independent cross-platform keybindings (issue #11).
//!
//! One [`CommandBinding`] carries the macOS, Windows, and Linux chords for a
//! command plus an optional GPUI key context, or the explicit unbound state.
//!
//! Validation is *pure*: [`CommandBinding::validate`] parses every platform's
//! chord on every host. A declaration that is invalid on Windows must fail on a
//! macOS developer machine too, so `cfg!(target_os = ...)` never gates parsing.
//! Only [`CommandBinding::for_platform`] — the projection step — is
//! platform-specific.

use gpui::{KeyBindingContextPredicate, Keystroke};

use super::CommandId;
use super::faults::CommandFault;
use super::standard::DesktopPlatform;

/// The macOS, Windows, and Linux chords for one command, plus the key context
/// they apply in.
///
/// The default is [`CommandBinding::unbound`]: a command has no shortcut unless
/// it declares one.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct CommandBinding {
    macos: Option<&'static str>,
    windows: Option<&'static str>,
    linux: Option<&'static str>,
    context: Option<&'static str>,
}

impl CommandBinding {
    /// The explicit unbound state: no shortcut on any platform.
    #[must_use]
    pub const fn unbound() -> Self {
        Self {
            macos: None,
            windows: None,
            linux: None,
            context: None,
        }
    }

    /// The same chord on every platform (`f1`, `ctrl-alt-d`, …).
    #[must_use]
    pub const fn same(keystroke: &'static str) -> Self {
        Self {
            macos: Some(keystroke),
            windows: Some(keystroke),
            linux: Some(keystroke),
            context: None,
        }
    }

    /// The conventional desktop split: a `cmd-` chord on macOS and a `ctrl-`
    /// chord on Windows and Linux.
    #[must_use]
    pub const fn platform(macos: &'static str, windows_linux: &'static str) -> Self {
        Self {
            macos: Some(macos),
            windows: Some(windows_linux),
            linux: Some(windows_linux),
            context: None,
        }
    }

    /// An explicitly per-platform binding; `None` leaves that platform unbound.
    #[must_use]
    pub const fn new(
        macos: Option<&'static str>,
        windows: Option<&'static str>,
        linux: Option<&'static str>,
    ) -> Self {
        Self {
            macos,
            windows,
            linux,
            context: None,
        }
    }

    /// A chord that exists only on macOS (`cmd-h`, `cmd-m`, …).
    #[must_use]
    pub const fn macos_only(keystroke: &'static str) -> Self {
        Self::new(Some(keystroke), None, None)
    }

    /// Scope the binding to a GPUI key context, so a view-local command only
    /// fires while that context is focused. Standard application commands leave
    /// this unset and bind globally.
    #[must_use]
    pub const fn key_context(mut self, context: &'static str) -> Self {
        self.context = Some(context);
        self
    }

    /// Whether no platform declares a chord.
    #[allow(dead_code)] // Exercised only by unit tests; no production caller yet.
    pub(crate) const fn is_unbound(&self) -> bool {
        self.macos.is_none() && self.windows.is_none() && self.linux.is_none()
    }

    /// The chord to install on `platform`, if any.
    pub(crate) const fn for_platform(&self, platform: DesktopPlatform) -> Option<&'static str> {
        match platform {
            DesktopPlatform::MacOs => self.macos,
            DesktopPlatform::Windows => self.windows,
            DesktopPlatform::Linux => self.linux,
        }
    }

    /// The declared key context, if any.
    pub(crate) const fn context(&self) -> Option<&'static str> {
        self.context
    }

    /// Parse every declared chord and the key context, appending one fault per
    /// independent failure.
    ///
    /// Host-independent by construction: all three platform slots are parsed
    /// regardless of where this runs.
    pub(crate) fn validate(&self, command: CommandId, faults: &mut Vec<CommandFault>) {
        for platform in DesktopPlatform::ALL {
            let Some(keystroke) = self.for_platform(platform) else {
                continue;
            };
            if parse_keystrokes(keystroke).is_err() {
                faults.push(CommandFault::InvalidBinding {
                    command,
                    platform,
                    binding: keystroke,
                });
            }
        }
        if let Some(context) = self.context
            && KeyBindingContextPredicate::parse(context).is_err()
        {
            faults.push(CommandFault::InvalidKeyContext { command, context });
        }
    }
}

/// Parse a chord exactly the way `gpui::KeyBinding::load` does — split on
/// whitespace, then run the GPUI [`Keystroke`] parser on each stroke — without
/// needing a boxed action or a keyboard mapper.
///
/// The mapper only affects *display* of an already-parsed stroke, so skipping it
/// keeps validation pure while accepting exactly the same inputs. The parity is
/// asserted in [`tests::pure_validation_matches_the_runtime_binding_loader`].
fn parse_keystrokes(keystrokes: &str) -> Result<(), gpui::InvalidKeystrokeError> {
    for stroke in keystrokes.split_whitespace() {
        Keystroke::parse(stroke)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use gpui::{DummyKeyboardMapper, KeyBinding, actions};

    use super::*;

    actions!(binding_test, [Probe]);

    const PROBE: CommandId = CommandId("test.binding");

    fn faults(binding: CommandBinding) -> Vec<CommandFault> {
        let mut faults = Vec::new();
        binding.validate(PROBE, &mut faults);
        faults
    }

    #[test]
    fn unbound_declares_nothing_on_any_platform() {
        let binding = CommandBinding::unbound();
        assert!(binding.is_unbound());
        for platform in DesktopPlatform::ALL {
            assert_eq!(binding.for_platform(platform), None);
        }
        assert!(
            faults(binding).is_empty(),
            "nothing to parse, nothing to fault"
        );
    }

    #[test]
    fn platform_projection_selects_one_chord_per_host() {
        let binding = CommandBinding::platform("cmd-q", "ctrl-q");
        assert_eq!(binding.for_platform(DesktopPlatform::MacOs), Some("cmd-q"));
        assert_eq!(
            binding.for_platform(DesktopPlatform::Windows),
            Some("ctrl-q")
        );
        assert_eq!(binding.for_platform(DesktopPlatform::Linux), Some("ctrl-q"));
        assert!(!binding.is_unbound());

        let mac_only = CommandBinding::macos_only("cmd-h");
        assert_eq!(mac_only.for_platform(DesktopPlatform::MacOs), Some("cmd-h"));
        assert_eq!(mac_only.for_platform(DesktopPlatform::Windows), None);
        assert_eq!(mac_only.for_platform(DesktopPlatform::Linux), None);
    }

    #[test]
    fn every_platform_chord_is_parsed_on_every_host() {
        // Valid where this test runs, invalid on the other two: the fault must
        // still be reported, otherwise a macOS developer could ship a broken
        // Windows binding.
        let binding = CommandBinding::new(Some("cmd-k"), Some("ctrl-nope-k"), Some("bogus-k"));

        assert_eq!(
            faults(binding),
            vec![
                CommandFault::InvalidBinding {
                    command: PROBE,
                    platform: DesktopPlatform::Windows,
                    binding: "ctrl-nope-k",
                },
                CommandFault::InvalidBinding {
                    command: PROBE,
                    platform: DesktopPlatform::Linux,
                    binding: "bogus-k",
                },
            ],
        );
    }

    #[test]
    fn key_context_is_parsed_and_reported_separately() {
        assert!(
            faults(CommandBinding::same("ctrl-k").key_context("StoryView && !Editor")).is_empty(),
        );
        assert_eq!(
            faults(CommandBinding::same("ctrl-k").key_context("StoryView &&")),
            vec![CommandFault::InvalidKeyContext {
                command: PROBE,
                context: "StoryView &&",
            }],
        );
        assert_eq!(
            CommandBinding::same("ctrl-k").context(),
            None,
            "standard commands bind globally unless a context is declared",
        );
    }

    #[test]
    fn pure_validation_matches_the_runtime_binding_loader() {
        // The pure parser must accept exactly what the runtime loader accepts,
        // or a declaration could validate and then fail while binding.
        for keystroke in [
            "cmd-q",
            "ctrl-shift-p",
            "ctrl-k ctrl-t",
            "f1",
            "cmd-,",
            "ctrl-nope-k",
            "",
            "cmd-",
        ] {
            let loaded = KeyBinding::load(
                keystroke,
                Box::new(Probe),
                None,
                false,
                None,
                &DummyKeyboardMapper,
            )
            .is_ok();
            assert_eq!(
                parse_keystrokes(keystroke).is_ok(),
                loaded,
                "pure and runtime parsers disagree about `{keystroke}`",
            );
        }
    }
}
