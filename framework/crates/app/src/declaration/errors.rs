//! Aggregated declaration validation faults.
//!
//! Validation is pure: it never touches GPUI, the filesystem, or the host
//! platform, and it reports *every* independent fault it finds in declaration
//! order rather than stopping at the first one.

use std::fmt;

/// A single pure declaration fault.
///
/// Variants are supplied by the declaration modules that own each fault. Only
/// identity and surface validation exist at this stage; launch, setup, command,
/// and settings modules extend this enum as they land, which is why it is
/// `#[non_exhaustive]` from the start.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum DeclarationError {
    /// A required [`crate::IdentityRef`] field is unusable.
    InvalidIdentity {
        /// Identity field that failed validation, e.g. `app_id`.
        field: &'static str,
        /// Why the field is unusable, e.g. `is empty`.
        reason: &'static str,
    },

    /// A surface ID is unusable as a stable identifier.
    InvalidSurfaceId {
        /// The declared surface ID.
        id: &'static str,
        /// Why the ID is unusable, e.g. `is empty`.
        reason: &'static str,
    },

    /// Two surfaces were declared with the same ID.
    DuplicateSurfaceId {
        /// The repeated surface ID.
        id: &'static str,
    },

    /// An auxiliary surface claimed a framework-reserved ID.
    ReservedSurfaceId {
        /// The declared surface ID.
        id: &'static str,
        /// The standard role that owns the reserved ID.
        role: &'static str,
    },

    /// A standard role was declared with an ID other than its reserved one.
    SurfaceRoleId {
        /// The standard role.
        role: &'static str,
        /// The reserved ID the role requires.
        expected: &'static str,
        /// The ID the surface actually declared.
        actual: &'static str,
    },

    /// More than one primary surface was declared.
    MultiplePrimarySurfaces {
        /// The surplus primary surface's ID.
        id: &'static str,
    },

    /// A role that must stay a singleton asked for multiple instances.
    InvalidSurfaceCardinality {
        /// The declared surface ID.
        id: &'static str,
        /// The role that forbids multiple instances.
        role: &'static str,
    },

    /// A surface admitting multiple instances declared a reuse hook, which can
    /// never run: every open of such a surface creates a new window.
    UnreachableSurfaceReuse {
        /// The declared surface ID.
        id: &'static str,
    },

    /// A second launch specification was declared. At most one may be.
    MultipleLaunchSpecs {
        /// `type_name` of the surplus specification's launch value.
        launch: &'static str,
    },

    /// A launch specification declared more than one `before_primary` hook.
    /// Only the first would run, so the surplus is a mistake, not an override.
    MultipleBeforePrimaryHooks {
        /// `type_name` of the specification's launch value.
        launch: &'static str,
    },

    /// The primary surface takes arguments that no declared launch
    /// specification produces, so it could never be opened.
    PrimarySurfaceArguments {
        /// `type_name` of the arguments the primary surface requires.
        arguments: &'static str,
    },

    /// A setup key is not a well-formed stable identifier.
    InvalidSetupKey {
        /// The declared setup key.
        key: &'static str,
        /// Why it is rejected, phrased to follow the key.
        reason: &'static str,
    },

    /// The same setup key was declared more than once.
    DuplicateSetupKey {
        /// The repeated setup key.
        key: &'static str,
    },

    /// A setup module depends on a key nothing declares.
    MissingSetupDependency {
        /// The dependent setup module's key.
        key: &'static str,
        /// The undeclared dependency.
        dependency: &'static str,
    },

    /// A setup module declared more than one teardown. Only the first would
    /// run, so the surplus is a mistake, not an override.
    MultipleSetupTeardowns {
        /// The setup module's key.
        key: &'static str,
    },

    /// A setup module takes part in a dependency cycle, so no initialization
    /// order exists.
    SetupDependencyCycle {
        /// A setup key on the cycle.
        key: &'static str,
    },

    /// A second common start hook was declared. Only the first would run, so
    /// the surplus is a mistake, not an override.
    MultipleStartHooks,

    /// A second runtime error reporter was declared. The shell reports each
    /// nonfatal error to exactly one observer.
    MultipleErrorReporters,

    /// A second application shutdown hook was declared. Only the first would
    /// run, so the surplus is a mistake, not an override.
    MultipleShutdownHooks,

    /// The same application settings store key was declared more than once. A
    /// store key names a file, so two schemas sharing one key would overwrite
    /// each other.
    DuplicateSettingsStoreKey {
        /// The repeated store key. Owned because [`crate::StoreKey`] may own
        /// its string.
        key: String,
    },

    /// An application settings store claimed the key the framework uses for its
    /// own shell preferences.
    ReservedSettingsStoreKey {
        /// The reserved store key the application claimed.
        key: String,
    },

    /// A second About policy was declared. The first is authoritative, so the
    /// surplus is a mistake, not an override.
    MultipleAboutDeclarations,

    /// A second theme policy was declared. The first is authoritative, so the
    /// surplus is a mistake, not an override.
    MultipleThemeDeclarations,

    /// A typed command or menu declaration fault.
    Command {
        /// The fault, in the command model's own vocabulary.
        fault: crate::commands::CommandFault,
    },
}

impl fmt::Display for DeclarationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidIdentity { field, reason } => {
                write!(f, "invalid app identity: `{field}` {reason}")
            }
            Self::InvalidSurfaceId { id, reason } => {
                write!(f, "invalid surface id: `{id}` {reason}")
            }
            Self::DuplicateSurfaceId { id } => {
                write!(f, "surface id `{id}` is declared more than once")
            }
            Self::ReservedSurfaceId { id, role } => {
                write!(
                    f,
                    "surface id `{id}` is reserved for the standard {role} surface"
                )
            }
            Self::SurfaceRoleId {
                role,
                expected,
                actual,
            } => {
                write!(
                    f,
                    "the {role} surface must use the reserved id `{expected}`, not `{actual}`"
                )
            }
            Self::MultiplePrimarySurfaces { id } => {
                write!(
                    f,
                    "only one primary surface may be declared; `{id}` is a second one"
                )
            }
            Self::InvalidSurfaceCardinality { id, role } => {
                write!(
                    f,
                    "the {role} surface `{id}` must be a singleton and cannot declare multiple \
                     instances"
                )
            }
            Self::UnreachableSurfaceReuse { id } => {
                write!(
                    f,
                    "surface `{id}` declares an on_reuse hook but admits multiple instances, so \
                     no open would ever reuse a window"
                )
            }
            Self::MultipleLaunchSpecs { launch } => {
                write!(
                    f,
                    "only one launch specification may be declared; `{launch}` is a second one"
                )
            }
            Self::MultipleBeforePrimaryHooks { launch } => {
                write!(
                    f,
                    "only one before_primary hook may be declared; the launch specification for \
                     `{launch}` declares a second one"
                )
            }
            Self::PrimarySurfaceArguments { arguments } => {
                write!(
                    f,
                    "the primary surface takes `{arguments}` but no launch specification produces \
                     it"
                )
            }
            Self::InvalidSetupKey { key, reason } => {
                write!(f, "invalid setup key: `{key}` {reason}")
            }
            Self::DuplicateSetupKey { key } => {
                write!(f, "setup key `{key}` is declared more than once")
            }
            Self::MissingSetupDependency { key, dependency } => {
                write!(
                    f,
                    "setup module `{key}` depends on `{dependency}`, which is not declared"
                )
            }
            Self::MultipleSetupTeardowns { key } => {
                write!(
                    f,
                    "only one shutdown hook may be declared; setup module `{key}` declares a \
                     second one"
                )
            }
            Self::SetupDependencyCycle { key } => {
                write!(f, "setup module `{key}` is part of a dependency cycle")
            }
            Self::MultipleStartHooks => {
                f.write_str("only one start hook may be declared; a second one was declared")
            }
            Self::MultipleErrorReporters => f.write_str(
                "only one runtime error reporter may be declared; a second one was declared",
            ),
            Self::MultipleShutdownHooks => f.write_str(
                "only one application shutdown hook may be declared; a second one was declared",
            ),
            Self::DuplicateSettingsStoreKey { key } => {
                write!(f, "settings store key `{key}` is declared more than once")
            }
            Self::ReservedSettingsStoreKey { key } => {
                write!(
                    f,
                    "settings store key `{key}` is reserved for the framework's shell preferences"
                )
            }
            Self::MultipleAboutDeclarations => {
                f.write_str("only one About policy may be declared; a second one was declared")
            }
            Self::MultipleThemeDeclarations => {
                f.write_str("only one theme policy may be declared; a second one was declared")
            }
            Self::Command { fault } => write!(f, "{fault}"),
        }
    }
}

impl std::error::Error for DeclarationError {}

/// One or more [`DeclarationError`]s, in deterministic declaration order.
///
/// Non-empty by construction: it can only be built through
/// [`DeclarationErrors::new`], which returns `None` for an empty input, so a
/// `DeclarationErrors` value always names at least one real fault.
#[derive(Debug)]
pub struct DeclarationErrors {
    errors: Vec<DeclarationError>,
}

impl DeclarationErrors {
    /// Wrap collected faults, or `None` when validation found nothing.
    pub(crate) fn new(errors: Vec<DeclarationError>) -> Option<Self> {
        if errors.is_empty() {
            None
        } else {
            Some(Self { errors })
        }
    }

    /// Number of reported faults; always at least one.
    pub fn len(&self) -> usize {
        self.errors.len()
    }

    /// Always `false`: [`DeclarationErrors::new`] returns `None` rather than
    /// an empty aggregate.
    pub fn is_empty(&self) -> bool {
        false
    }

    /// Iterate the faults in declaration order.
    pub fn iter(&self) -> std::slice::Iter<'_, DeclarationError> {
        self.errors.iter()
    }
}

impl fmt::Display for DeclarationErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} declaration error", self.errors.len())?;
        if self.errors.len() != 1 {
            f.write_str("s")?;
        }
        f.write_str(": ")?;
        for (index, error) in self.errors.iter().enumerate() {
            if index > 0 {
                f.write_str("; ")?;
            }
            write!(f, "{error}")?;
        }
        Ok(())
    }
}

impl std::error::Error for DeclarationErrors {}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity_error(field: &'static str) -> DeclarationError {
        DeclarationError::InvalidIdentity {
            field,
            reason: "is empty",
        }
    }

    #[test]
    fn empty_collection_is_not_constructible() {
        assert!(DeclarationErrors::new(Vec::new()).is_none());
    }

    #[test]
    fn collection_preserves_order_and_reports_every_fault() {
        let errors = DeclarationErrors::new(vec![
            identity_error("app_id"),
            identity_error("data_namespace"),
        ])
        .expect("non-empty input yields a collection");

        assert_eq!(errors.len(), 2);
        assert_eq!(
            errors.iter().cloned().collect::<Vec<_>>(),
            vec![identity_error("app_id"), identity_error("data_namespace")],
        );
        assert_eq!(
            errors.to_string(),
            "2 declaration errors: invalid app identity: `app_id` is empty; \
             invalid app identity: `data_namespace` is empty",
        );
    }

    #[test]
    fn single_fault_display_is_singular() {
        let errors =
            DeclarationErrors::new(vec![identity_error("app_id")]).expect("one fault is non-empty");

        assert_eq!(
            errors.to_string(),
            "1 declaration error: invalid app identity: `app_id` is empty",
        );
    }
}
