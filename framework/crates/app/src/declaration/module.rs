//! The erased declaration-module seam.
//!
//! Surface, launch, setup, command, and settings declarations are typed at
//! their call site and erased into this one object-safe trait, so
//! [`super::AppDeclaration`] stays non-generic and opaque no matter how many
//! typed module kinds exist.
//!
//! The trait is `pub(crate)`: it is the shell's internal assembly seam, not an
//! extension point, so an application can neither implement it nor reach the
//! runtime modules it contributes. It deliberately does not require `Send` or
//! `Sync` — a declaration is built and consumed on the main thread.

use crate::module::RuntimeModules;

use super::errors::DeclarationError;

/// One erased piece of an application declaration.
///
/// Object-safe on purpose: [`super::AppDeclaration`] keeps modules as
/// `Box<dyn DeclarationModule>` in declaration order, validates them in that
/// order, and installs them in that order.
pub(crate) trait DeclarationModule: 'static {
    /// Stable identifier used in diagnostics.
    #[cfg(test)]
    fn key(&self) -> &'static str;

    /// Append this module's pure faults, in the module's own declaration order.
    ///
    /// Pure: no GPUI, no filesystem, no host-platform inspection.
    fn validate(&self, errors: &mut Vec<DeclarationError>);

    /// Append the runtime modules this declaration contributes, in order.
    ///
    /// The only thing a declaration module may contribute: it cannot reach the
    /// plan's identity, assets, or process policies, so no module lowered later
    /// can clobber a policy the application declared.
    ///
    /// Consuming (`self: Box<Self>`) so a module can move owned state such as
    /// surface hooks into its runtime module without cloning, while staying
    /// object-safe.
    fn install(self: Box<Self>, modules: &mut RuntimeModules);
}

#[cfg(test)]
pub(super) mod test_support {
    use std::sync::{Arc, Mutex};

    use super::DeclarationModule;
    use crate::declaration::errors::DeclarationError;
    use crate::module::RuntimeModules;

    /// Records validate/install calls so tests can assert deterministic order.
    pub(in crate::declaration) struct RecordingModule {
        key: &'static str,
        faults: Vec<DeclarationError>,
        log: Arc<Mutex<Vec<String>>>,
    }

    impl RecordingModule {
        pub(in crate::declaration) fn new(key: &'static str, log: Arc<Mutex<Vec<String>>>) -> Self {
            Self {
                key,
                faults: Vec::new(),
                log,
            }
        }

        pub(in crate::declaration) fn with_fault(mut self, field: &'static str) -> Self {
            self.faults.push(DeclarationError::InvalidIdentity {
                field,
                reason: "is empty",
            });
            self
        }
    }

    impl DeclarationModule for RecordingModule {
        fn key(&self) -> &'static str {
            self.key
        }

        fn validate(&self, errors: &mut Vec<DeclarationError>) {
            self.log
                .lock()
                .expect("recording module log poisoned")
                .push(format!("{}:validate", self.key));
            errors.extend(self.faults.iter().cloned());
        }

        fn install(self: Box<Self>, _modules: &mut RuntimeModules) {
            self.log
                .lock()
                .expect("recording module log poisoned")
                .push(format!("{}:install", self.key));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::test_support::RecordingModule;
    use super::*;

    #[test]
    fn trait_is_object_safe_and_needs_no_send_or_sync() {
        let log = Arc::new(Mutex::new(Vec::new()));
        // Compiles only if the trait is object-safe; the `Box<dyn _>` also
        // proves no `Send`/`Sync` bound leaked into the trait.
        let module: Box<dyn DeclarationModule> =
            Box::new(RecordingModule::new("test.module", Arc::clone(&log)));

        assert_eq!(module.key(), "test.module");
    }

    #[test]
    fn validate_appends_module_faults_without_clearing_earlier_ones() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let module = RecordingModule::new("test.module", Arc::clone(&log)).with_fault("app_id");
        let mut errors = vec![DeclarationError::InvalidIdentity {
            field: "earlier",
            reason: "is empty",
        }];

        module.validate(&mut errors);

        assert_eq!(errors.len(), 2);
        assert_eq!(
            errors[1],
            DeclarationError::InvalidIdentity {
                field: "app_id",
                reason: "is empty",
            }
        );
        assert_eq!(
            log.lock().expect("log poisoned").as_slice(),
            ["test.module:validate"]
        );
    }
}
