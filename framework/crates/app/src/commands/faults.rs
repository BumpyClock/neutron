//! Private declaration faults for the typed command and menu model.
//!
//! The aggregated public fault type (`declaration::DeclarationErrors`) is owned
//! by the declaration core and does not yet carry command variants. Rather than
//! reach across that boundary, this module reports faults in its own vocabulary
//! and leaves one narrow seam — [`CommandFaults::iter`] plus each fault's
//! `Display` — for the integration owner to map into `DeclarationError` when the
//! command module is wired into `AppDeclaration`.
//!
//! Every fault is `Copy` and carries only `&'static` identities, so collecting
//! them is allocation-light and comparing them in tests is exact.

use std::fmt;

use super::CommandId;
use super::keys::{KeyFault, MenuKey, MenuSectionKey};
use super::standard::DesktopPlatform;

/// One pure fault in a typed command or menu declaration.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum CommandFault {
    /// Two declared commands share one [`CommandId`].
    DuplicateCommand {
        /// The repeated id.
        command: CommandId,
    },
    /// A declared chord could not be parsed for one platform.
    InvalidBinding {
        /// The command that declared it.
        command: CommandId,
        /// The platform whose chord failed to parse.
        platform: DesktopPlatform,
        /// The unparsable chord.
        binding: &'static str,
    },
    /// A declared key context is not a valid GPUI context predicate.
    InvalidKeyContext {
        /// The command that declared it.
        command: CommandId,
        /// The unparsable predicate.
        context: &'static str,
    },
    /// A menu key is unusable as an identity.
    InvalidMenuKey {
        /// The rejected text.
        raw: &'static str,
        /// Why it was rejected.
        fault: KeyFault,
    },
    /// Two top-level menus share one [`MenuKey`].
    DuplicateMenuKey {
        /// The repeated key.
        menu: MenuKey,
    },
    /// A menu references a command that was never declared.
    UnknownCommand {
        /// The menu holding the reference.
        menu: MenuKey,
        /// The undeclared command.
        command: CommandId,
    },
    /// A menu reserves a section with no declared provider.
    MissingSectionProvider {
        /// The menu holding the slot.
        menu: MenuKey,
        /// The unprovided section.
        section: MenuSectionKey,
    },
    /// Two providers were declared for one section key.
    DuplicateSectionProvider {
        /// The repeated section.
        section: MenuSectionKey,
    },
    /// One section key appears twice in the resolved menu bar, so a single
    /// provider would have to fill two slots.
    DuplicateSectionSlot {
        /// The repeated section.
        section: MenuSectionKey,
    },
    /// A dynamic section was reserved inside the macOS dock menu. The dock is a
    /// flat command list built by a separate projection that has no section
    /// machinery, so the slot could never be filled.
    SectionInDock {
        /// The section that targeted the dock.
        section: MenuSectionKey,
    },
    /// One command is referenced twice inside a single menu, which would render
    /// the same entry twice. Referencing a command from *several* menus is
    /// valid: one command may legitimately project into many surfaces.
    RepeatedCommandInMenu {
        /// The menu containing the repeat.
        menu: MenuKey,
        /// The repeated command.
        command: CommandId,
    },
    /// Hiding an optional standard menu would strand an enabled standard
    /// feature: the resolved platform places that feature in the hidden menu,
    /// so the declaration asks for the feature and removes its only home in the
    /// same breath.
    ///
    /// Platform-resolved, because placement is: About lives in the application
    /// menu on macOS and in Help elsewhere, and the Appearance section lives in
    /// the application menu on macOS and in View elsewhere.
    StrandedStandardFeature {
        /// The menu the declaration hid.
        menu: MenuKey,
        /// The framework-owned name of the feature that lost its placement.
        feature: &'static str,
    },
    /// A standard-menu edit is not permitted.
    InvalidStandardEdit {
        /// The menu the edit targeted.
        menu: MenuKey,
        /// Why the edit was rejected.
        reason: &'static str,
    },
}

impl fmt::Display for CommandFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateCommand { command } => {
                write!(f, "duplicate command id `{command}`")
            }
            Self::InvalidBinding {
                command,
                platform,
                binding,
            } => write!(
                f,
                "invalid {} binding `{binding}` for command `{command}`",
                platform.as_str(),
            ),
            Self::InvalidKeyContext { command, context } => {
                write!(f, "invalid key context `{context}` for command `{command}`")
            }
            Self::InvalidMenuKey { raw, fault } => {
                write!(f, "invalid menu key `{raw}`: {fault}")
            }
            Self::DuplicateMenuKey { menu } => {
                write!(f, "duplicate top-level menu `{menu}`")
            }
            Self::UnknownCommand { menu, command } => {
                write!(f, "menu `{menu}` references undeclared command `{command}`")
            }
            Self::MissingSectionProvider { menu, section } => write!(
                f,
                "menu `{menu}` reserves section `{section}` with no declared provider",
            ),
            Self::DuplicateSectionProvider { section } => {
                write!(f, "duplicate provider for menu section `{section}`")
            }
            Self::DuplicateSectionSlot { section } => {
                write!(f, "menu section `{section}` is reserved more than once")
            }
            Self::SectionInDock { section } => write!(
                f,
                "menu section `{section}` cannot be reserved in the dock menu",
            ),
            Self::RepeatedCommandInMenu { menu, command } => write!(
                f,
                "menu `{menu}` references command `{command}` more than once",
            ),
            Self::StrandedStandardFeature { menu, feature } => write!(
                f,
                "hiding menu `{menu}` strands the enabled {feature} feature, \
                 which this platform places there",
            ),
            Self::InvalidStandardEdit { menu, reason } => {
                write!(f, "invalid standard-menu edit for `{menu}`: {reason}")
            }
        }
    }
}

impl std::error::Error for CommandFault {}

/// One or more [`CommandFault`]s in deterministic declaration order.
///
/// Non-empty by construction, mirroring `DeclarationErrors`: it can only be
/// built through [`CommandFaults::new`], which returns `None` for an empty
/// input.
#[derive(Debug)]
pub(crate) struct CommandFaults {
    faults: Vec<CommandFault>,
}

impl CommandFaults {
    /// Wrap collected faults, or `None` when validation found nothing.
    pub(crate) fn new(faults: Vec<CommandFault>) -> Option<Self> {
        if faults.is_empty() {
            None
        } else {
            Some(Self { faults })
        }
    }

    /// Number of reported faults; always at least one.
    #[allow(dead_code)] // Exercised only by unit tests; no production caller yet.
    pub(crate) fn len(&self) -> usize {
        self.faults.len()
    }

    /// Iterate the faults in declaration order.
    ///
    /// The integration seam: the declaration owner maps these into
    /// `DeclarationError` variants once the command module joins
    /// `AppDeclaration`.
    pub(crate) fn iter(&self) -> std::slice::Iter<'_, CommandFault> {
        self.faults.iter()
    }
}

impl fmt::Display for CommandFaults {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} command declaration error", self.faults.len())?;
        if self.faults.len() != 1 {
            f.write_str("s")?;
        }
        f.write_str(": ")?;
        for (index, fault) in self.faults.iter().enumerate() {
            if index > 0 {
                f.write_str("; ")?;
            }
            write!(f, "{fault}")?;
        }
        Ok(())
    }
}

impl std::error::Error for CommandFaults {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_collection_is_not_constructible() {
        assert!(CommandFaults::new(Vec::new()).is_none());
    }

    #[test]
    fn collection_preserves_order_and_renders_every_fault() {
        let faults = CommandFaults::new(vec![
            CommandFault::DuplicateCommand {
                command: CommandId("app.quit"),
            },
            CommandFault::UnknownCommand {
                menu: MenuKey::EDIT,
                command: CommandId("edit.ghost"),
            },
        ])
        .expect("non-empty input yields a collection");

        assert_eq!(faults.len(), 2);
        assert_eq!(
            faults.to_string(),
            "2 command declaration errors: duplicate command id `app.quit`; \
             menu `Edit` references undeclared command `edit.ghost`",
        );
    }

    #[test]
    fn binding_faults_name_the_offending_platform() {
        let fault = CommandFault::InvalidBinding {
            command: CommandId("app.quit"),
            platform: DesktopPlatform::Windows,
            binding: "ctrl-nope-q",
        };

        assert_eq!(
            fault.to_string(),
            "invalid Windows binding `ctrl-nope-q` for command `app.quit`",
        );
    }
}
