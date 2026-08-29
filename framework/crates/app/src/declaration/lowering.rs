//! Lowering a validated declaration into the runtime plan it runs as.
//!
//! One direction, no intermediate builder: [`AppDeclaration::lower`] consumes
//! the declaration and produces the [`RuntimePlan`] the shell executes. Every
//! dedicated declaration concern (surfaces, settings stores, the About and
//! theme conventions, commands and menus, setup, lifecycle hooks) contributes
//! its own finalized runtime modules and values here, in the fixed order that
//! makes the conventional desktop work.

use crate::commands::MenusModule;
use crate::commands::standard::DesktopPlatform;
use crate::declaration::LaunchRuntime;
use crate::error::AppShellError;
use crate::module::RuntimeModules;
use crate::setup::SetupPipelineModule;
use crate::shell::{PlatformRunner, RuntimePlan};

use super::AppDeclaration;

/// Map an advanced-hook failure into the run-error category that covers process
/// preparation and application configuration.
///
/// The failure is returned, never logged and swallowed: a hook that could not
/// prepare the process must stop startup.
fn preparation<T>(result: anyhow::Result<T>) -> Result<T, AppShellError> {
    result.map_err(AppShellError::Preparation)
}

impl AppDeclaration {
    /// Lower this declaration into the plan that runs it.
    ///
    /// Call [`AppDeclaration::validate`] first: lowering assumes the
    /// declaration is already known to be well formed, and
    /// [`AppDeclaration::prepare_launch`] second, since `launch` is the
    /// prepared typed launch runtime.
    pub(crate) fn lower(self, launch: LaunchRuntime, runner: PlatformRunner) -> RuntimePlan {
        let assets = self.asset_sources();
        let setup_order = super::setup::plan(&self.setup)
            .expect("lowering runs only on a declaration that already validated");
        let features = self.standard_features();
        let Self {
            identity,
            assets: _,
            advanced,
            modules: declared,
            surfaces: _,
            launch: _,
            primary: _,
            mut setup,
            lifecycle,
            settings_stores,
            about,
            theme,
            settings_opener: _,
            commands,
            exit_policy,
            initial_activation,
        } = self;

        // Window management is the only always-present service, and it must
        // initialize before any surface: the surfaces register against the
        // window manager it installs.
        let mut modules: RuntimeModules = vec![Box::new(crate::windows::WindowsModule::new())];
        for module in declared {
            module.install(&mut modules);
        }
        // The resolved About surface installs with the declared ones, after
        // them: it is framework-resolved, so it takes the last surface slot
        // whether the framework materialized it or the application replaced it.
        if let Some(surface) = about.into_surface() {
            modules.push(Box::new(crate::windows::declared_surface_module(surface)));
        }
        // Settings stores after every surface: a surface may read a store while
        // it initializes, and modules initialize in install order.
        for store in settings_stores {
            store.install(&mut modules);
        }
        // The whole theme convention, or none of it: shell preferences, the
        // theme module with the framework's read/write mapping, and the
        // Appearance menu bridge. `without_theme` installs none of them.
        theme.install(&mut modules);
        // Exactly one menu owner, installing the typed framework, feature, and
        // application vocabulary in one pass.
        modules.push(Box::new(MenusModule::declared(
            commands.standard_features(features),
            DesktopPlatform::current(),
        )));
        // Strictly last, whatever order `setup` was called in: application setup
        // must initialize after every framework and declared module and tear
        // down before them.
        if !setup.is_empty() {
            let mut slots: Vec<Option<_>> = setup.drain(..).map(Some).collect();
            let resolved = setup_order
                .into_iter()
                .map(|index| slots[index].take().expect("each module is placed once"))
                .collect();
            modules.push(Box::new(SetupPipelineModule::new(resolved)));
        }

        let lifecycle = lifecycle.into_runtime();
        // The application's own `advanced` set is authoritative by
        // construction: a declaration module contributes runtime modules only,
        // so nothing lowered above can reach these policies.
        RuntimePlan {
            identity,
            assets,
            path_layout: advanced.path_layout,
            environment: advanced.environment,
            logging: advanced.logging,
            initial_activation,
            exit_policy,
            prepare: advanced
                .prepare
                .map(|hook| Box::new(move |info: &_| preparation(hook(info))) as _),
            configure_application: advanced
                .configure_application
                .map(|hook| Box::new(move |application| preparation(hook(application))) as _),
            modules,
            observers: lifecycle.observers,
            start: lifecycle.start,
            error_reporter: lifecycle.reporter,
            app_shutdown: lifecycle.shutdown,
            launch,
            runner,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use neutron_components_storage::PathLayout;

    use gpui::Empty;

    use super::super::advanced::AdvancedHooks;
    use super::super::module::test_support::RecordingModule;
    use super::super::setup::tests::init_unit;
    use super::super::surface::tests::build_empty;
    use super::super::tests::{ProbeAssets, identity};
    use super::super::{SetupKey, SetupModule, Surface, SurfaceKey};
    use super::*;
    use crate::handles::AppInfo;
    use crate::settings::{AppSettings, StoreKey};
    use crate::shell::{EnvironmentPolicy, LoggingPolicy};
    use crate::theme::ThemeSource;
    use std::error::Error as _;

    /// A minimal application schema, so a declared store lowers a real typed
    /// settings module rather than a stand-in.
    #[derive(Default, serde::Serialize, serde::Deserialize)]
    struct Prefs {
        greeting: String,
    }

    impl AppSettings for Prefs {
        const SCHEMA_VERSION: u32 = 1;
    }

    fn prepare_hook(_info: &AppInfo) -> anyhow::Result<()> {
        Ok(())
    }

    fn configure_hook(application: gpui::Application) -> anyhow::Result<gpui::Application> {
        Ok(application)
    }

    fn logging_hook(_paths: &neutron_components_storage::AppPaths) -> anyhow::Result<()> {
        Ok(())
    }

    /// Lower a declaration the way [`super::super::run::execute`] does, with a
    /// unit launch runtime and the native runner, so every test below asserts
    /// the exact plan the shell would run.
    fn lower(declaration: AppDeclaration) -> RuntimePlan {
        declaration.lower(LaunchRuntime::unit(None), PlatformRunner::native())
    }

    fn module_names(declaration: AppDeclaration) -> Vec<&'static str> {
        lower(declaration).module_names()
    }

    #[test]
    fn lowering_applies_identity_assets_and_advanced_policies() {
        let plan = lower(
            AppDeclaration::new(identity())
                .assets(ProbeAssets {
                    hit: "probe",
                    bytes: b"app",
                })
                .advanced(
                    AdvancedHooks::new()
                        .path_layout(PathLayout::SingleRoot(".neutron".into()))
                        .environment(EnvironmentPolicy::LoginShell)
                        .logging(LoggingPolicy::Configure(logging_hook))
                        .prepare(prepare_hook)
                        .configure_application(configure_hook),
                ),
        );

        assert_eq!(plan.identity, identity());
        assert_eq!(plan.assets.len(), 2, "app source plus framework");
        assert_eq!(plan.path_layout, PathLayout::SingleRoot(".neutron".into()));
        assert_eq!(plan.environment_name(), "login-shell");
        assert_eq!(plan.logging_name(), "configure");
        assert!(plan.prepare.is_some());
        assert!(plan.configure_application.is_some());
    }

    #[test]
    fn lowering_defaults_leave_the_plan_at_its_conventional_state() {
        let plan = lower(AppDeclaration::new(identity()));

        assert_eq!(plan.assets.len(), 1, "framework fallback only");
        assert_eq!(plan.path_layout, PathLayout::PlatformDefault);
        assert_eq!(plan.environment_name(), "inherit");
        assert_eq!(plan.logging_name(), "external");
        assert!(plan.prepare.is_none());
        assert!(plan.configure_application.is_none());
        assert!(
            plan.start.is_none() && plan.observers.is_empty() && plan.app_shutdown.is_none(),
            "an application that declares no lifecycle hooks gets none",
        );
    }

    #[test]
    fn lowered_assets_keep_first_match_wins_with_framework_fallback() {
        let plan = lower(
            AppDeclaration::new(identity())
                .assets(ProbeAssets {
                    hit: "probe",
                    bytes: b"first",
                })
                .assets(ProbeAssets {
                    hit: "probe",
                    bytes: b"second",
                }),
        );

        assert_eq!(
            plan.load_asset("probe")
                .expect("probe load succeeds")
                .as_deref(),
            Some(&b"first"[..]),
        );
        assert!(
            plan.load_asset("icons/bell.svg")
                .expect("framework load succeeds")
                .is_some(),
            "framework assets remain reachable behind the app sources",
        );
    }

    #[test]
    fn declared_lifecycle_hooks_reach_the_plan() {
        fn start(_cx: &mut gpui::App) -> anyhow::Result<()> {
            Ok(())
        }
        fn observe(_event: &crate::lifecycle::AppEvent, _cx: &mut gpui::App) -> anyhow::Result<()> {
            Ok(())
        }
        fn report(_error: &crate::error::RuntimeError, _cx: &mut gpui::App) {}
        fn teardown(_cx: &mut gpui::App) -> anyhow::Result<()> {
            Ok(())
        }

        let plan = lower(
            AppDeclaration::new(identity())
                .start(start)
                .on_event(observe)
                .on_event(observe)
                .runtime_errors(report)
                .shutdown(teardown),
        );

        assert!(plan.start.is_some());
        assert_eq!(plan.observers.len(), 2, "on_event is repeatable");
        assert!(plan.error_reporter.is_some());
        assert!(plan.app_shutdown.is_some());
    }

    #[test]
    fn modules_install_in_declaration_order() {
        let log = Arc::new(Mutex::new(Vec::new()));

        let _plan = lower(
            AppDeclaration::new(identity())
                .module(RecordingModule::new("first", Arc::clone(&log)))
                .module(RecordingModule::new("second", Arc::clone(&log))),
        );

        assert_eq!(
            log.lock().expect("log poisoned").as_slice(),
            ["first:install", "second:install"],
        );
    }

    #[test]
    fn the_setup_pipeline_is_the_last_runtime_module() {
        let names = module_names(
            AppDeclaration::new(identity())
                // Declared before the surface on purpose: setup must still
                // install last, so it initializes after every framework and
                // declared module and tears down before them.
                .setup(SetupModule::new(SetupKey::new("app.setup"), init_unit))
                .surface(Surface::new(SurfaceKey::<Empty>::new("panel"), build_empty)),
        );

        assert_eq!(
            names.last().copied(),
            Some("neutron_components_app::setup::SetupPipelineModule"),
            "the setup pipeline must be the last runtime module: {names:?}",
        );
        assert!(
            names.len() >= 3,
            "the framework and surface modules are still installed before it: {names:?}",
        );
    }

    #[test]
    fn a_declaration_without_setup_modules_installs_no_pipeline() {
        let names = module_names(AppDeclaration::new(identity()));

        assert!(
            !names
                .iter()
                .any(|name| name.ends_with("SetupPipelineModule")),
            "an empty setup list must not install a no-op module: {names:?}",
        );
    }

    /// The whole conventional desktop foundation, in the one order that makes
    /// it work: windows before any surface, the resolved About with the
    /// surfaces, settings stores after every surface, the theme convention
    /// after the stores, exactly one menu owner, and setup last.
    #[test]
    fn the_default_declaration_installs_the_conventional_desktop_order() {
        let names = module_names(AppDeclaration::new(identity()));

        assert_eq!(
            names,
            vec![
                "neutron_components_app::windows::WindowsModule",
                "neutron_components_app::windows::DeclaredSurfaceModule",
                "neutron_components_app::settings::ShellPreferencesModule",
                "neutron_components_app::theme::ThemeModule",
                "neutron_components_app::declaration::settings::ThemeMenuModule",
                "neutron_components_app::commands::menus::MenusModule",
            ],
        );
    }

    /// The whole declared vocabulary at once: every dedicated concern
    /// contributes its own runtime modules, and the framework/application
    /// order is fixed by lowering rather than by declaration order. Bypassing
    /// one declared module, or reordering framework work against application
    /// setup, changes this list.
    #[test]
    fn every_declared_concern_contributes_its_runtime_module_in_the_fixed_order() {
        let names = module_names(
            AppDeclaration::new(identity())
                // Deliberately declared in an order that contradicts the
                // resolved one: setup first, then a store, then the surfaces.
                .setup(SetupModule::new(SetupKey::new("app.setup"), init_unit))
                .settings_store::<Prefs>(StoreKey::PRIMARY)
                .surface(Surface::new(SurfaceKey::<Empty>::new("panel"), build_empty))
                .primary_surface(Surface::new(SurfaceKey::<Empty>::primary(), build_empty))
                .settings_surface(Surface::new(SurfaceKey::<Empty>::settings(), build_empty)),
        );

        assert_eq!(
            names,
            vec![
                "neutron_components_app::windows::WindowsModule",
                // The three declared surfaces, in declaration order…
                "neutron_components_app::windows::DeclaredSurfaceModule",
                "neutron_components_app::windows::DeclaredSurfaceModule",
                "neutron_components_app::windows::DeclaredSurfaceModule",
                // …then the framework-resolved About surface.
                "neutron_components_app::windows::DeclaredSurfaceModule",
                "neutron_components_app::settings::SettingsModule<neutron_components_app::declaration::lowering::tests::Prefs>",
                "neutron_components_app::settings::ShellPreferencesModule",
                "neutron_components_app::theme::ThemeModule",
                "neutron_components_app::declaration::settings::ThemeMenuModule",
                "neutron_components_app::commands::menus::MenusModule",
                "neutron_components_app::setup::SetupPipelineModule",
            ],
        );
    }

    #[test]
    fn declared_stores_install_after_every_surface_and_before_the_theme() {
        let names = module_names(
            AppDeclaration::new(identity())
                // Declared before the surface on purpose: a store must still
                // install after it, so a surface can read settings as it
                // initializes.
                .settings_store::<Prefs>(StoreKey::PRIMARY)
                .surface(Surface::new(SurfaceKey::<Empty>::new("panel"), build_empty)),
        );

        let last_surface = names
            .iter()
            .rposition(|name| name.ends_with("DeclaredSurfaceModule"))
            .unwrap_or_else(|| panic!("both surfaces are installed: {names:?}"));
        let position = |needle: &str| {
            names
                .iter()
                .position(|name| name.contains(needle))
                .unwrap_or_else(|| panic!("{needle} is installed: {names:?}"))
        };
        assert!(last_surface < position("SettingsModule<"));
        assert!(position("SettingsModule<") < position("ShellPreferencesModule"));
        assert!(position("ThemeMenuModule") < position("MenusModule"));
    }

    #[test]
    fn without_theme_installs_no_preferences_source_or_appearance_bridge() {
        let names = module_names(AppDeclaration::new(identity()).without_theme());

        assert_eq!(
            names,
            vec![
                "neutron_components_app::windows::WindowsModule",
                "neutron_components_app::windows::DeclaredSurfaceModule",
                "neutron_components_app::commands::menus::MenusModule",
            ],
            "the theme convention is all three modules or none of them",
        );
    }

    #[test]
    fn a_custom_theme_source_keeps_the_rest_of_the_convention() {
        let names = module_names(AppDeclaration::new(identity()).theme(ThemeSource::registry()));

        assert_eq!(
            names,
            module_names(AppDeclaration::new(identity())),
            "replacing the source replaces nothing else",
        );
    }

    #[test]
    fn without_about_installs_no_about_surface() {
        let names = module_names(AppDeclaration::new(identity()).without_about());

        assert!(
            !names.iter().any(|name| name.ends_with("SurfaceModule")),
            "a declaration with no surfaces and no About installs none: {names:?}",
        );
    }

    #[test]
    fn a_custom_about_replaces_the_framework_surface_rather_than_joining_it() {
        let names = module_names(
            AppDeclaration::new(identity())
                .about_surface(Surface::new(SurfaceKey::<Empty>::about(), build_empty)),
        );

        assert_eq!(
            names
                .iter()
                .filter(|name| name.ends_with("DeclaredSurfaceModule"))
                .count(),
            1,
            "two About surfaces would collide on the reserved id: {names:?}",
        );
    }

    #[test]
    fn exactly_one_menus_module_owns_the_declared_vocabulary() {
        let names = module_names(AppDeclaration::new(identity()));

        assert_eq!(
            names
                .iter()
                .filter(|name| name.ends_with("MenusModule"))
                .count(),
            1,
            "a second menus owner would install the framework vocabulary twice: {names:?}",
        );
    }

    #[test]
    fn the_setup_pipeline_installs_after_the_declared_menus_module() {
        let names = module_names(
            AppDeclaration::new(identity())
                .setup(SetupModule::new(SetupKey::new("app"), init_unit)),
        );

        let menus = names
            .iter()
            .position(|name| name.ends_with("MenusModule"))
            .expect("the declared menus module is installed");
        let pipeline = names
            .iter()
            .position(|name| name.ends_with("SetupPipelineModule"))
            .expect("the setup pipeline is installed");
        assert!(
            menus < pipeline,
            "application setup initializes after every framework module: {names:?}",
        );
    }

    #[test]
    fn a_failing_advanced_hook_surfaces_as_a_preparation_error() {
        let error = super::preparation(Err::<(), _>(anyhow::anyhow!("prepare failed")))
            .expect_err("the hook fails");

        assert!(
            matches!(error, AppShellError::Preparation(_)),
            "preparation failures must not be logged and swallowed: {error}",
        );
        assert_eq!(
            error.source().expect("the cause is preserved").to_string(),
            "prepare failed",
        );
    }
}
