//! The private declaration core: `DesktopApp`, `AppDeclaration`, pure
//! validation, advanced policies, and lowering into the runtime plan.
//!
//! This module is private; the crate root re-exports the value types that
//! make up the public declaration surface.

mod advanced;
mod errors;
mod launch;
mod lifecycle;
mod lowering;
mod module;
pub(crate) mod run;
mod settings;
mod setup;
mod surface;

use std::sync::Arc;

use gpui::{Action, AssetSource, Render};
use neutron_components_manifest::schema::IdentityRef;

use crate::commands::standard::DesktopPlatform;
use crate::commands::{
    Command, CommandFault, CommandsDeclaration, MenuBar, MenuSectionKey, SectionProvider,
    StandardFeatures,
};
use crate::liveness::{ExitPolicy, InitialActivation};

pub use advanced::AdvancedHooks;
pub use errors::{DeclarationError, DeclarationErrors};
pub(crate) use launch::{DeclaredLaunch, LaunchRuntime, PreparedLaunch};
pub use launch::{LaunchDecision, LaunchSpec, ProcessLaunch};
use launch::{PrimaryOpener, validate_launch_set};
use lifecycle::LifecycleHooks;
pub(crate) use lifecycle::{ErrorHook, EventHook, ShutdownHook, StartHook};
pub(crate) use module::DeclarationModule;
use settings::{AboutIntent, DeclaredSettingsStore, ErasedSettingsStore, ThemeIntent};
pub(crate) use setup::DeclaredSetupModule;
pub use setup::{SetupKey, SetupModule};
use surface::SurfaceModule;
pub(crate) use surface::{DeclaredSurface, SurfaceCardinality, SurfaceOptions, SurfaceRole};
pub use surface::{Surface, SurfaceKey};

/// A type-level application declaration.
///
/// The application is a *type*, not a value: the shell never creates or retains
/// a mutable application object. Mutable state lives in GPUI entities, globals,
/// and explicit handles.
pub trait DesktopApp: 'static {
    /// Return the complete, pure declaration for this application.
    fn declaration() -> AppDeclaration;
}

/// One complete, pure application declaration.
///
/// Opaque and non-generic: typed declaration modules erase themselves into an
/// ordered internal list, so adding module kinds never changes this type's
/// shape. The declaration is built with consuming methods and validated without
/// GPUI or a platform loop.
pub struct AppDeclaration {
    identity: IdentityRef,
    assets: Vec<Arc<dyn AssetSource>>,
    advanced: AdvancedHooks,
    modules: Vec<Box<dyn DeclarationModule>>,
    /// Ordered `(id, role)` index of the declared surfaces.
    ///
    /// The surfaces themselves are erased into `modules`; this index exists
    /// because duplicate IDs and a second primary are the only surface faults
    /// no single surface can see on its own.
    surfaces: Vec<(&'static str, SurfaceRole)>,
    /// Declared launch modules, in declaration order.
    ///
    /// At most one is legal. Surplus declarations are kept rather than
    /// discarded so [`AppDeclaration::validate`] can report them; the first
    /// remains the one that parses.
    launch: Vec<DeclaredLaunch>,
    /// Erased openers of every declared primary surface, whatever route
    /// declared them. Only the first can ever run; the surplus are kept so a
    /// repeated primary is reported rather than silently replacing the first.
    primary: Vec<PrimaryOpener>,
    /// Declared application setup modules, in declaration order.
    ///
    /// Kept out of `modules` because they lower as one pipeline plugin that
    /// must land after every other declaration module plugin, regardless of
    /// where `setup` was called.
    setup: Vec<DeclaredSetupModule>,
    /// The declared lifecycle hooks, with surplus singletons counted.
    lifecycle: LifecycleHooks,
    /// Declared application settings stores, in declaration order.
    ///
    /// Kept out of `modules` because lowering must install every surface before
    /// any store, regardless of where `settings_store` was called.
    settings_stores: Vec<Box<dyn ErasedSettingsStore>>,
    /// The resolved About policy: the framework surface by convention, an
    /// application replacement, or disabled.
    about: AboutIntent,
    /// The resolved theme policy: the registry source by convention, an
    /// application source, or disabled.
    theme: ThemeIntent,
    /// The Settings surface opener, set by [`AppDeclaration::settings_surface`].
    ///
    /// The single reason the standard Settings command exists: no Settings
    /// surface, no Settings command, shortcut, or menu item.
    settings_opener: Option<settings::StandardOpener>,
    /// The typed command and menu vocabulary. Always present: the standard
    /// menu bar is the conventional desktop foundation, so an application that
    /// declares nothing still gets exactly one menu owner.
    commands: CommandsDeclaration,
    /// The exit policy (default [`ExitPolicy::WhenIdle`]).
    exit_policy: ExitPolicy,
    /// The initial-activation policy (default [`InitialActivation::Regular`]).
    initial_activation: InitialActivation,
}

impl AppDeclaration {
    /// Start a declaration from the compiled-in identity, with the standard
    /// desktop foundation by convention.
    ///
    /// The conventions are the theme (registry source, framework-owned shell
    /// preferences, and the platform Appearance section) and a framework About
    /// surface with its standard command. Deliberately *not* included: an
    /// application settings store, a Settings surface, or a Settings command —
    /// the framework cannot invent an application's schema or its settings UI,
    /// and a Settings item that opens nothing is worse than none.
    #[must_use]
    pub fn new(identity: IdentityRef) -> Self {
        Self {
            identity,
            assets: Vec::new(),
            advanced: AdvancedHooks::new(),
            modules: Vec::new(),
            surfaces: Vec::new(),
            launch: Vec::new(),
            primary: Vec::new(),
            setup: Vec::new(),
            lifecycle: LifecycleHooks::new(),
            settings_stores: Vec::new(),
            about: AboutIntent::new(),
            theme: ThemeIntent::new(),
            settings_opener: None,
            commands: CommandsDeclaration::new(),
            exit_policy: ExitPolicy::WhenIdle,
            initial_activation: InitialActivation::Regular,
        }
    }

    /// The standard features this declaration resolved.
    ///
    /// Derived rather than stored, so the command model and the lowering path
    /// can never disagree with the intents above about what was declared.
    fn standard_features(&self) -> StandardFeatures {
        StandardFeatures {
            settings: self.settings_opener,
            about: self.about.opener(),
            theme: self.theme.enabled(),
        }
    }

    /// Append an application asset source. Repeatable; sources are tried in
    /// declaration order and the first to resolve a path wins.
    #[must_use]
    pub fn assets(mut self, source: impl AssetSource) -> Self {
        self.assets.push(Arc::new(source));
        self
    }

    /// Replace the advanced policy set (default [`AdvancedHooks::new`]).
    #[must_use]
    pub fn advanced(mut self, advanced: AdvancedHooks) -> Self {
        self.advanced = advanced;
        self
    }

    /// Append an erased declaration module, preserving declaration order.
    pub(crate) fn module(mut self, module: impl DeclarationModule) -> Self {
        self.modules.push(Box::new(module));
        self
    }

    /// Declare the primary surface: the launch surface, restored on reopen.
    ///
    /// At most one may be declared; an application without one is a
    /// background process. This is the unit-launch convenience: a primary
    /// surface whose open arguments are the process's typed launch value
    /// must be declared through [`LaunchSpec::primary_surface`] instead, which
    /// ties the primary's argument type to the launch value at compile time.
    #[must_use]
    pub fn primary_surface<View: 'static + Render>(mut self, surface: Surface<View, ()>) -> Self {
        self.primary.push(PrimaryOpener::of::<View, ()>());
        self.declare_surface(DeclaredSurface::erase(surface, SurfaceRole::Primary))
    }

    /// Declare the standard Settings surface, activating the standard Settings
    /// command contract. Unit arguments and singleton only.
    ///
    /// The Settings command exists *because* of this call. An application with
    /// no settings UI gets no Settings item, shortcut, or menu placement.
    ///
    /// A second call replaces the first, exactly as a repeated surface
    /// declaration would: the surface set reports the duplicate ID, so the
    /// contradiction is named once rather than twice.
    #[must_use]
    pub fn settings_surface<View: 'static + Render>(mut self, surface: Surface<View, ()>) -> Self {
        self.settings_opener = Some(crate::windows::settings_opener::<View>());
        self.declare_surface(DeclaredSurface::erase(surface, SurfaceRole::Settings))
    }

    /// Replace the framework's About content, preserving the standard About
    /// command contract. Unit arguments and singleton only.
    ///
    /// At most one About policy may be declared. A second call — including
    /// [`AppDeclaration::without_about`] — is reported by
    /// [`AppDeclaration::validate`] rather than replacing the first.
    #[must_use]
    pub fn about_surface<View: 'static + Render>(mut self, surface: Surface<View, ()>) -> Self {
        self.about.custom(
            DeclaredSurface::erase(surface, SurfaceRole::About),
            crate::windows::about_opener::<View>(),
        );
        self
    }

    /// Drop the About surface and its standard command entirely.
    ///
    /// For an application whose About belongs somewhere else — inside a
    /// settings pane, on a website, or nowhere at all.
    #[must_use]
    pub fn without_about(mut self) -> Self {
        self.about.disable();
        self
    }

    /// Declare a typed application settings store under an explicit key.
    ///
    /// The key names the file and is validated; the Rust type name never
    /// determines file identity, so renaming a schema type cannot orphan a
    /// user's settings. Migration, validation, and future-version policy all
    /// live on the [`crate::AppSettings`] implementation.
    #[must_use]
    pub fn settings_store<T: crate::settings::AppSettings>(
        mut self,
        key: crate::settings::StoreKey,
    ) -> Self {
        self.settings_stores
            .push(Box::new(DeclaredSettingsStore::<T>::new(key)));
        self
    }

    /// Replace the framework's registry-backed theme source.
    ///
    /// The rest of the theme convention — persisted shell preferences and the
    /// platform Appearance section — is unchanged.
    ///
    /// At most one theme policy may be declared; a second is reported by
    /// [`AppDeclaration::validate`] rather than replacing the first.
    #[must_use]
    pub fn theme(mut self, source: crate::theme::ThemeSource) -> Self {
        self.theme.custom(source);
        self
    }

    /// Drop the theme convention: no theme source, no persisted shell
    /// preferences, and no Appearance section.
    #[must_use]
    pub fn without_theme(mut self) -> Self {
        self.theme.disable();
        self
    }

    /// Declare an auxiliary surface under an application-chosen stable ID.
    #[must_use]
    pub fn surface<View: 'static + Render, Args: 'static>(
        self,
        surface: Surface<View, Args>,
    ) -> Self {
        self.declare_surface(DeclaredSurface::erase(surface, SurfaceRole::Auxiliary))
    }

    /// Declare an application setup module. Repeatable.
    ///
    /// Setup modules initialize after every framework and declared module, in a
    /// deterministic order derived from [`SetupModule::after`] dependencies, and
    /// tear down in exact reverse — regardless of where in the declaration
    /// `setup` was called.
    #[must_use]
    pub fn setup<State: 'static>(mut self, module: SetupModule<State>) -> Self {
        self.setup.push(DeclaredSetupModule::erase(module));
        self
    }

    /// Declare the common start hook: fallible application composition after
    /// every framework and declared module initializes.
    ///
    /// At most one may be declared; a second is reported by
    /// [`AppDeclaration::validate`] rather than replacing the first.
    #[must_use]
    pub fn start(mut self, hook: StartHook) -> Self {
        self.lifecycle.start(hook);
        self
    }

    /// Observe every lifecycle [`crate::AppEvent`]. Repeatable.
    ///
    /// Observers run in declaration order. A failure is nonfatal: it is
    /// reported to the runtime reporter and the remaining observers still run.
    #[must_use]
    pub fn on_event(mut self, hook: EventHook) -> Self {
        self.lifecycle.on_event(hook);
        self
    }

    /// Observe nonfatal runtime errors.
    ///
    /// Exactly one reporter observes the whole process; a second is a
    /// declaration fault rather than a silent replacement.
    #[must_use]
    pub fn runtime_errors(mut self, hook: ErrorHook) -> Self {
        self.lifecycle.on_error(hook);
        self
    }

    /// Declare application teardown, run after `WillExit` and before framework
    /// modules tear down in reverse.
    ///
    /// Runs exactly once. At most one may be declared; a second is reported by
    /// [`AppDeclaration::validate`].
    #[must_use]
    pub fn shutdown(mut self, hook: ShutdownHook) -> Self {
        self.lifecycle.shutdown(hook);
        self
    }

    /// Declare one typed command. Repeatable.
    #[must_use]
    pub fn command<A: Action>(mut self, command: Command<A>) -> Self {
        self.commands = self.commands.command(command);
        self
    }

    /// Set the menu-bar policy (default the platform-conventional standard
    /// layout).
    #[must_use]
    pub fn menu_bar(mut self, menu_bar: MenuBar) -> Self {
        self.commands = self.commands.menu_bar(menu_bar);
        self
    }

    /// Declare the provider for one reserved menu section. Repeatable.
    #[must_use]
    pub fn menu_section(mut self, key: MenuSectionKey, provider: SectionProvider) -> Self {
        self.commands = self.commands.section(key, provider);
        self
    }

    /// Set the exit policy. Use [`ExitPolicy::Explicit`] for apps that outlive
    /// their windows (default [`ExitPolicy::WhenIdle`]).
    #[must_use]
    pub fn exit_policy(mut self, policy: ExitPolicy) -> Self {
        self.exit_policy = policy;
        self
    }

    /// Set the initial-activation policy. Use [`InitialActivation::Passive`]
    /// for tray-first apps (default [`InitialActivation::Regular`]).
    #[must_use]
    pub fn initial_activation(mut self, activation: InitialActivation) -> Self {
        self.initial_activation = activation;
        self
    }

    /// Index an erased surface and append it to the module list.
    fn declare_surface(mut self, surface: DeclaredSurface) -> Self {
        self.surfaces.push((surface.id(), surface.role()));
        self.module(SurfaceModule::new(surface))
    }

    /// The declared identity.
    #[allow(dead_code)] // Exercised only by unit tests; no production caller yet.
    pub(crate) fn identity(&self) -> IdentityRef {
        self.identity
    }

    /// Check every declaration invariant, reporting all independent faults in
    /// deterministic declaration order.
    ///
    /// Pure: no GPUI and no filesystem, so validation costs nothing and can run
    /// before any resource exists.
    ///
    /// Host independence holds across every rule, including command and menu
    /// rules. Identity, module, surface, launch, and setup rules never resolve
    /// against a [`DesktopPlatform`], so they produce the same faults on every
    /// target unconditionally. Command and menu rules *do* resolve against a
    /// platform — a chord, a standard-menu placement, or an outline shape can
    /// differ by host — so this entry point validates them for every
    /// [`DesktopPlatform::ALL`] rather than only [`DesktopPlatform::current()`].
    /// That way a menu-layout fault that only manifests on Windows or Linux is
    /// still reported when validation runs on a macOS developer machine. A
    /// fault that is identical across platforms (for example a duplicate
    /// command id, which does not depend on the platform argument at all) is
    /// reported once, not once per platform: faults are deduplicated by value
    /// as they accumulate, keeping the first occurrence in platform order
    /// ([`DesktopPlatform::ALL`]) and, within one platform, declaration order.
    #[must_use]
    pub fn validate(&self) -> Result<(), DeclarationErrors> {
        let mut errors: Vec<DeclarationError> = Vec::new();

        validate_identity(&self.identity, &mut errors);
        for module in &self.modules {
            module.validate(&mut errors);
        }
        // The resolved About surface is validated and indexed exactly like a
        // declared one, whether the framework materialized it or the
        // application supplied it: it is appended last because the framework
        // resolves it after every explicit declaration.
        self.about.validate(&mut errors);
        let mut surfaces = self.surfaces.clone();
        surfaces.extend(self.about.surface_entry());
        validate_surface_set(&surfaces, &mut errors);
        settings::validate_settings_stores(&self.settings_stores, &mut errors);
        self.theme.validate(&mut errors);
        validate_launch_set(&self.launch, &self.primary, &mut errors);
        if let Err(faults) = setup::plan(&self.setup) {
            errors.extend(faults);
        }
        self.lifecycle.validate(&mut errors);
        // The command model reports faults in its own vocabulary; the narrow
        // seam it left (`CommandFaults::iter`) folds them into the one
        // aggregate so an application sees a single error list. Validating
        // every platform and deduplicating by value is what makes that
        // aggregate host-independent: see the doc comment above.
        let features = self.standard_features();
        let mut command_faults: Vec<CommandFault> = Vec::new();
        for platform in DesktopPlatform::ALL {
            if let Err(faults) = self.commands.validate_with(platform, features) {
                for fault in faults.iter() {
                    if !command_faults.contains(fault) {
                        command_faults.push(*fault);
                    }
                }
            }
        }
        errors.extend(
            command_faults
                .into_iter()
                .map(|fault| DeclarationError::Command { fault }),
        );

        match DeclarationErrors::new(errors) {
            Some(errors) => Err(errors),
            None => Ok(()),
        }
    }

    /// The ordered asset sources for this declaration.
    ///
    /// Application sources keep their declaration order and precedence; the
    /// framework component assets are appended last as the fallback, so an
    /// application can always override a framework asset but never loses one.
    pub(crate) fn asset_sources(&self) -> Vec<Arc<dyn AssetSource>> {
        let mut sources = self.assets.clone();
        sources.push(Arc::new(neutron_components_assets::Assets));
        sources
    }
}

/// Identity faults, reported in field order so the aggregate is deterministic.
fn validate_identity(identity: &IdentityRef, errors: &mut Vec<DeclarationError>) {
    if identity.app_id.is_empty() {
        errors.push(DeclarationError::InvalidIdentity {
            field: "app_id",
            reason: "is empty",
        });
    }
    if identity.data_namespace.is_empty() {
        errors.push(DeclarationError::InvalidIdentity {
            field: "data_namespace",
            reason: "is empty",
        });
    }
}

/// Cross-surface faults: a repeated ID and a second primary surface.
///
/// Reported in declaration order, and only for the surplus declaration, so the
/// first well-formed surface of a colliding pair is never blamed.
fn validate_surface_set(
    surfaces: &[(&'static str, SurfaceRole)],
    errors: &mut Vec<DeclarationError>,
) {
    let mut seen: Vec<&'static str> = Vec::new();
    let mut has_primary = false;
    for (id, role) in surfaces {
        if seen.contains(id) {
            errors.push(DeclarationError::DuplicateSurfaceId { id });
        } else {
            seen.push(id);
        }
        if *role == SurfaceRole::Primary {
            if has_primary {
                errors.push(DeclarationError::MultiplePrimarySurfaces { id });
            }
            has_primary = true;
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::borrow::Cow;
    use std::sync::{Arc, Mutex};

    use gpui::{Empty, SharedString};

    use super::module::test_support::RecordingModule;
    use super::surface::tests::build_empty;
    use super::*;

    pub(crate) fn identity() -> IdentityRef {
        IdentityRef {
            app_id: "com.example.declaration",
            display_name: "Declaration Test",
            data_namespace: "declaration-test",
            binary_name: None,
            org: None,
            publisher: None,
            url_schemes: &[],
            categories: &[],
            macos: None,
            linux: None,
            windows: None,
            legacy_ids: &[],
            min_os: None,
            version: "0.0.0",
            cfbundle_short_version: "0.0.0",
            msix_version: "0.0.0.0",
        }
    }

    /// Resolves `hit` to `bytes` and misses everything else.
    pub(crate) struct ProbeAssets {
        pub(crate) hit: &'static str,
        pub(crate) bytes: &'static [u8],
    }

    impl AssetSource for ProbeAssets {
        fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
            if path == self.hit {
                Ok(Some(Cow::Borrowed(self.bytes)))
            } else {
                Ok(None)
            }
        }

        fn list(&self, _path: &str) -> gpui::Result<Vec<SharedString>> {
            Ok(Vec::new())
        }
    }

    struct TestApp;

    impl DesktopApp for TestApp {
        fn declaration() -> AppDeclaration {
            AppDeclaration::new(identity())
        }
    }

    fn load(source: &Arc<dyn AssetSource>, path: &str) -> Option<Vec<u8>> {
        source
            .load(path)
            .ok()
            .flatten()
            .map(|bytes| bytes.into_owned())
    }

    #[test]
    fn a_valid_declaration_reports_no_faults() {
        assert!(TestApp::declaration().validate().is_ok());
    }

    #[test]
    fn identity_faults_are_reported_in_field_order() {
        let mut broken = identity();
        broken.app_id = "";
        broken.data_namespace = "";

        let errors = AppDeclaration::new(broken)
            .validate()
            .expect_err("an empty app_id and data_namespace are both faults");

        assert_eq!(
            errors.iter().cloned().collect::<Vec<_>>(),
            vec![
                DeclarationError::InvalidIdentity {
                    field: "app_id",
                    reason: "is empty",
                },
                DeclarationError::InvalidIdentity {
                    field: "data_namespace",
                    reason: "is empty",
                },
            ],
        );
    }

    #[test]
    fn identity_faults_precede_module_faults_in_declaration_order() {
        let mut broken = identity();
        broken.app_id = "";
        let log = Arc::new(Mutex::new(Vec::new()));

        let errors = AppDeclaration::new(broken)
            .module(RecordingModule::new("first", Arc::clone(&log)).with_fault("first-fault"))
            .module(RecordingModule::new("second", Arc::clone(&log)).with_fault("second-fault"))
            .validate()
            .expect_err("identity and both modules are faulty");

        assert_eq!(
            errors
                .iter()
                .map(|error| error.to_string())
                .collect::<Vec<_>>(),
            vec![
                "invalid app identity: `app_id` is empty",
                "invalid app identity: `first-fault` is empty",
                "invalid app identity: `second-fault` is empty",
            ],
        );
        assert_eq!(
            log.lock().expect("log poisoned").as_slice(),
            ["first:validate", "second:validate"],
        );
    }

    #[test]
    fn asset_sources_are_application_first_with_framework_fallback() {
        let declaration = AppDeclaration::new(identity())
            .assets(ProbeAssets {
                hit: "probe",
                bytes: b"first",
            })
            .assets(ProbeAssets {
                hit: "probe",
                bytes: b"second",
            });

        let sources = declaration.asset_sources();

        assert_eq!(sources.len(), 3, "two app sources plus framework fallback");
        assert_eq!(load(&sources[0], "probe").as_deref(), Some(&b"first"[..]));
        assert_eq!(load(&sources[1], "probe").as_deref(), Some(&b"second"[..]));
        assert!(
            load(&sources[2], "icons/bell.svg").is_some(),
            "the framework component assets are the last source",
        );
    }

    fn surface_faults(declaration: AppDeclaration) -> Vec<String> {
        declaration
            .validate()
            .expect_err("the declaration is faulty")
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    #[test]
    fn a_full_standard_surface_set_is_valid() {
        let declaration = AppDeclaration::new(identity())
            .primary_surface(Surface::new(SurfaceKey::<Empty>::primary(), build_empty))
            .settings_surface(Surface::new(SurfaceKey::<Empty>::settings(), build_empty))
            .about_surface(Surface::new(SurfaceKey::<Empty>::about(), build_empty))
            .surface(Surface::new(SurfaceKey::<Empty>::new("logs"), build_empty).multiple());

        assert!(declaration.validate().is_ok());
    }

    #[test]
    fn a_repeated_surface_id_blames_only_the_later_declaration() {
        let declaration = AppDeclaration::new(identity())
            .surface(Surface::new(SurfaceKey::<Empty>::new("logs"), build_empty))
            .surface(Surface::new(SurfaceKey::<Empty>::new("logs"), build_empty));

        assert_eq!(
            surface_faults(declaration),
            vec!["surface id `logs` is declared more than once".to_string()],
        );
    }

    #[test]
    fn only_one_primary_surface_may_be_declared() {
        let declaration = AppDeclaration::new(identity())
            .primary_surface(Surface::new(SurfaceKey::<Empty>::primary(), build_empty))
            .primary_surface(Surface::new(SurfaceKey::<Empty>::primary(), build_empty));

        assert_eq!(
            surface_faults(declaration),
            vec![
                "surface id `primary` is declared more than once".to_string(),
                "only one primary surface may be declared; `primary` is a second one".to_string(),
            ],
            "the repeated id and the surplus role are independent faults",
        );
    }

    #[test]
    fn per_surface_faults_precede_cross_surface_faults() {
        let declaration = AppDeclaration::new(identity())
            .surface(Surface::new(SurfaceKey::<Empty>::new("logs"), build_empty))
            .settings_surface(Surface::new(SurfaceKey::<Empty>::new("prefs"), build_empty))
            .surface(Surface::new(SurfaceKey::<Empty>::new("logs"), build_empty));

        assert_eq!(
            surface_faults(declaration),
            vec![
                "the settings surface must use the reserved id `settings`, not `prefs`".to_string(),
                "surface id `logs` is declared more than once".to_string(),
            ],
        );
    }

    #[test]
    fn an_application_may_declare_no_surfaces_at_all() {
        assert!(
            AppDeclaration::new(identity()).validate().is_ok(),
            "a background process declares no primary surface",
        );
    }

    #[test]
    fn setup_is_repeatable_and_its_graph_faults_reach_the_aggregate() {
        use super::setup::tests::init_unit;

        let declaration = AppDeclaration::new(identity())
            .setup(SetupModule::new(SetupKey::new("theme"), init_unit))
            .setup(SetupModule::new(SetupKey::new("theme"), init_unit))
            .setup(SetupModule::new(SetupKey::new("index"), init_unit).after(SetupKey::new("db")));

        assert_eq!(
            declaration
                .validate()
                .expect_err("the setup graph is faulty")
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            vec![
                "setup key `theme` is declared more than once".to_string(),
                "setup module `index` depends on `db`, which is not declared".to_string(),
            ],
            "setup faults join the one aggregate, in declaration order",
        );
    }

    // -------------------------------------------------- lifecycle and commands

    fn start_ok(_cx: &mut gpui::App) -> anyhow::Result<()> {
        Ok(())
    }

    fn observe(_event: &crate::lifecycle::AppEvent, _cx: &mut gpui::App) -> anyhow::Result<()> {
        Ok(())
    }

    fn report(_error: &crate::error::RuntimeError, _cx: &mut gpui::App) {}

    fn shutdown_ok(_cx: &mut gpui::App) -> anyhow::Result<()> {
        Ok(())
    }

    #[test]
    fn event_observers_are_repeatable_and_the_singletons_are_not() {
        let declaration = AppDeclaration::new(identity())
            .start(start_ok)
            .on_event(observe)
            .on_event(observe)
            .on_event(observe)
            .runtime_errors(report)
            .shutdown(shutdown_ok);

        assert!(
            declaration.validate().is_ok(),
            "any number of observers is well formed; one of each singleton is too",
        );
    }

    #[test]
    fn surplus_singleton_lifecycle_hooks_are_declaration_faults() {
        let declaration = AppDeclaration::new(identity())
            .start(start_ok)
            .start(start_ok)
            .runtime_errors(report)
            .runtime_errors(report)
            .shutdown(shutdown_ok)
            .shutdown(shutdown_ok);

        assert_eq!(
            declaration
                .validate()
                .expect_err("three surplus hooks are faults")
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            vec![
                "only one start hook may be declared; a second one was declared".to_string(),
                "only one runtime error reporter may be declared; a second one was declared"
                    .to_string(),
                "only one application shutdown hook may be declared; a second one was declared"
                    .to_string(),
            ],
        );
    }

    #[test]
    fn command_faults_reach_the_one_declaration_aggregate() {
        use crate::commands::{Command, CommandId};

        const ALPHA: CommandId = CommandId("probe.alpha");

        gpui::actions!(declaration_probe, [Alpha]);

        fn run(_action: &Alpha, _cx: &mut gpui::App) -> anyhow::Result<()> {
            Ok(())
        }

        let declaration = AppDeclaration::new(identity())
            .command(Command::app(ALPHA, Alpha, run))
            .command(Command::app(ALPHA, Alpha, run));

        let errors = declaration
            .validate()
            .expect_err("a duplicate command id is a fault");
        assert!(
            errors
                .iter()
                .any(|error| error.to_string().contains("probe.alpha")),
            "command faults are reported through the declaration aggregate: {:?}",
            errors.iter().map(ToString::to_string).collect::<Vec<_>>(),
        );
    }

    #[test]
    fn menu_faults_that_only_manifest_on_other_platforms_are_reported_from_any_host() {
        use crate::commands::CommandFault;
        use crate::commands::MenuKey;

        // The default declaration leaves the theme convention enabled, so
        // hiding View — the menu Windows and Linux place Appearance in —
        // strands that feature on those two platforms. macOS has no View menu
        // at all, so `DesktopPlatform::current()` alone (macOS in this test
        // process) would never see the fault. `validate` must still report it
        // because it checks every `DesktopPlatform::ALL` member, not just the
        // host's.
        let declaration =
            AppDeclaration::new(identity()).menu_bar(MenuBar::standard().hide(MenuKey::VIEW));

        let errors = declaration
            .validate()
            .expect_err("hiding View strands Appearance on Windows and Linux");
        let command_faults: Vec<CommandFault> = errors
            .iter()
            .filter_map(|error| match error {
                DeclarationError::Command { fault } => Some(*fault),
                _ => None,
            })
            .collect();

        // Reported exactly once, even though the underlying menu outline is
        // built once per platform and Windows and Linux share this exact
        // fault (identical menu, identical feature).
        assert_eq!(
            command_faults,
            vec![CommandFault::StrandedStandardFeature {
                menu: MenuKey::VIEW,
                feature: "Appearance",
            }],
            "an identical Windows/Linux fault must be deduplicated to one report: {:?}",
            command_faults,
        );
    }

    #[test]
    fn the_default_command_declaration_is_faultless() {
        assert!(
            AppDeclaration::new(identity()).validate().is_ok(),
            "the framework vocabulary an application never touches must validate",
        );
    }

    #[test]
    fn a_well_formed_setup_graph_leaves_the_declaration_valid() {
        use super::setup::tests::init_unit;

        let declaration = AppDeclaration::new(identity())
            .setup(SetupModule::new(SetupKey::new("index"), init_unit).after(SetupKey::new("db")))
            .setup(SetupModule::new(SetupKey::new("db"), init_unit));

        assert!(declaration.validate().is_ok());
    }

    // ---- Desktop conventions: About, theme, and settings stores.

    /// A minimal application schema for store-key validation.
    #[derive(Default, serde::Serialize, serde::Deserialize)]
    struct Prefs;

    impl crate::settings::AppSettings for Prefs {
        const SCHEMA_VERSION: u32 = 1;
    }

    #[test]
    fn the_default_declaration_resolves_the_framework_about_surface() {
        let declaration = AppDeclaration::new(identity());

        assert!(declaration.validate().is_ok());
        assert_eq!(
            declaration.about.surface_entry(),
            Some((surface::ABOUT_SURFACE_ID, SurfaceRole::About)),
            "the framework About takes part in surface validation like a declared one",
        );
        assert!(declaration.standard_features().has_about());
    }

    #[test]
    fn without_about_removes_both_the_surface_and_the_command() {
        let declaration = AppDeclaration::new(identity()).without_about();

        assert!(declaration.validate().is_ok());
        assert_eq!(declaration.about.surface_entry(), None);
        assert!(
            !declaration.standard_features().has_about(),
            "an About item that opens nothing is worse than none",
        );
    }

    #[test]
    fn a_custom_about_replaces_the_framework_surface_without_duplicating_it() {
        let declaration = AppDeclaration::new(identity())
            .about_surface(Surface::new(SurfaceKey::<Empty>::about(), build_empty));

        assert!(
            declaration.validate().is_ok(),
            "replacing About must not collide with the surface it replaced",
        );
        assert_eq!(
            declaration.about.surface_entry(),
            Some((surface::ABOUT_SURFACE_ID, SurfaceRole::About)),
        );
        assert!(declaration.standard_features().has_about());
    }

    /// The resolved About takes part in per-surface validation like a declared
    /// one: replacing the framework content does not buy an exemption from the
    /// reserved-ID and cardinality rules its role carries.
    #[test]
    fn a_custom_about_surface_is_validated_like_every_other_surface() {
        let errors = AppDeclaration::new(identity())
            .about_surface(
                Surface::new(SurfaceKey::<Empty>::new("credits"), build_empty).multiple(),
            )
            .validate()
            .expect_err("a malformed About surface is still a malformed surface");

        assert_eq!(
            errors.iter().cloned().collect::<Vec<_>>(),
            vec![
                DeclarationError::SurfaceRoleId {
                    role: "about",
                    expected: surface::ABOUT_SURFACE_ID,
                    actual: "credits",
                },
                DeclarationError::InvalidSurfaceCardinality {
                    id: "credits",
                    role: "about",
                },
            ],
        );
    }

    #[test]
    fn a_second_about_policy_is_a_fault_and_the_first_stays_authoritative() {
        let declaration = AppDeclaration::new(identity())
            .without_about()
            .about_surface(Surface::new(SurfaceKey::<Empty>::about(), build_empty));

        let errors = declaration
            .validate()
            .expect_err("two About policies contradict each other");

        assert_eq!(
            errors.iter().cloned().collect::<Vec<_>>(),
            vec![DeclarationError::MultipleAboutDeclarations],
        );
        assert_eq!(
            declaration.about.surface_entry(),
            None,
            "the first policy wins; the later one is reported, not applied",
        );
    }

    #[test]
    fn the_theme_convention_is_on_by_default_and_removable() {
        assert!(AppDeclaration::new(identity()).standard_features().theme);

        let disabled = AppDeclaration::new(identity()).without_theme();
        assert!(disabled.validate().is_ok());
        assert!(!disabled.standard_features().theme);

        let custom = AppDeclaration::new(identity()).theme(crate::ThemeSource::registry());
        assert!(custom.validate().is_ok());
        assert!(
            custom.standard_features().theme,
            "replacing the source replaces nothing else",
        );
    }

    #[test]
    fn a_second_theme_policy_is_a_fault_and_the_first_stays_authoritative() {
        let declaration = AppDeclaration::new(identity())
            .without_theme()
            .theme(crate::ThemeSource::registry());

        let errors = declaration
            .validate()
            .expect_err("two theme policies contradict each other");

        assert_eq!(
            errors.iter().cloned().collect::<Vec<_>>(),
            vec![DeclarationError::MultipleThemeDeclarations],
        );
        assert!(
            !declaration.standard_features().theme,
            "the first policy wins; the later one is reported, not applied",
        );
    }

    #[test]
    fn the_settings_feature_is_activated_only_by_a_settings_surface() {
        assert!(
            !AppDeclaration::new(identity())
                .standard_features()
                .has_settings(),
            "the framework cannot invent an application's settings UI",
        );

        let declared = AppDeclaration::new(identity())
            .settings_surface(Surface::new(SurfaceKey::<Empty>::settings(), build_empty));
        assert!(declared.validate().is_ok());
        assert!(declared.standard_features().has_settings());
    }

    #[test]
    fn a_settings_store_key_may_not_be_declared_twice() {
        let errors = AppDeclaration::new(identity())
            .settings_store::<Prefs>(crate::StoreKey::PRIMARY)
            .settings_store::<Prefs>(crate::StoreKey::PRIMARY)
            .validate()
            .expect_err("two schemas on one key would overwrite each other");

        assert_eq!(
            errors.iter().cloned().collect::<Vec<_>>(),
            vec![DeclarationError::DuplicateSettingsStoreKey {
                key: "settings".to_string(),
            }],
            "only the surplus store is blamed",
        );
    }

    #[test]
    fn the_shell_preferences_key_is_reserved_for_the_framework() {
        let errors = AppDeclaration::new(identity())
            .settings_store::<Prefs>(
                crate::StoreKey::new(crate::settings::SHELL_PREFERENCES_KEY)
                    .expect("a well-formed key"),
            )
            .validate()
            .expect_err("the framework owns its own preferences file");

        assert_eq!(
            errors.iter().cloned().collect::<Vec<_>>(),
            vec![DeclarationError::ReservedSettingsStoreKey {
                key: crate::settings::SHELL_PREFERENCES_KEY.to_string(),
            }],
        );
    }
}
