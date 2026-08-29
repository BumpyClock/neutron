//! External-consumer compile check for the AppShell public surface.
//!
//! Imports `neutron_components_app` exactly as an application crate would —
//! through the crate root and `prelude` only — and type-checks the resolved
//! public surface: final nameability of every documented type/trait, and
//! basic construction where that is possible without a live GPUI `App`
//! (declaration-time values are pure; runtime-only values are only named,
//! never constructed, since that needs a running platform). Nothing here
//! starts a platform loop or a window: it is a compile-and-construct check,
//! not a behavior test, and it runs on every target platform.
//!
//! Must not import any private module path (`neutron_components_app::...`
//! beyond the crate root, `commands`, and `prelude`) or any private runtime
//! type: the runtime plan and its modules, the platform runner, the window
//! manager, window specs and records, the internal extension traits, or any
//! registry type.

use std::ffi::OsString;
use std::path::PathBuf;

use neutron_components_app::commands::standard::DesktopPlatform;
use neutron_components_app::gpui::{App, AppContext as _, Empty, Entity, Window, actions};
use neutron_components_app::prelude::*;
use neutron_components_app::{
    AdvancedHooks, AppClosed, AppDeclaration, AppEvent, AppInfo, AppProxy, AppShellError,
    Capability, Command, CommandBinding, CommandError, CommandFault, CommandId, CommandLabel,
    Commands, DeclarationError, DeclarationErrors, DesktopApp, LaunchDecision, LaunchSpec, Menu,
    MenuBar, MenuKey, MenuLabel, MenuNode, MenuOutline, MenuOutlineEntry, MenuSectionKey,
    OpenRequest, OverlaySpec, PathLayout, PlatformCapabilities, PlatformCapability, ProcessLaunch,
    RawWindow, RuntimeError, RuntimeOperation, Settings, SetupContext, SetupKey, SetupModule,
    ShutdownReason, Surface, SurfaceHandle, SurfaceKey, SurfaceOpen, UnsupportedCapability,
    WindowKey, WindowSize,
};

fn test_identity() -> neutron_components_app::IdentityRef {
    neutron_components_app::IdentityRef {
        app_id: "com.example.publicapitest",
        display_name: "Public API Test",
        data_namespace: "publicapitest",
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

fn build_empty(_args: &(), _window: &mut Window, cx: &mut App) -> Entity<Empty> {
    cx.new(|_| Empty)
}

fn parse_unit(_process: &ProcessLaunch) -> anyhow::Result<LaunchDecision<()>> {
    Ok(LaunchDecision::Run(()))
}

fn before_primary(_value: &(), _cx: &mut App) -> anyhow::Result<()> {
    Ok(())
}

fn init_setup(_cx: &mut SetupContext<'_>) -> anyhow::Result<()> {
    Ok(())
}

fn teardown_setup(_state: (), _cx: &mut SetupContext<'_>) -> anyhow::Result<()> {
    Ok(())
}

fn start_hook(_cx: &mut App) -> anyhow::Result<()> {
    Ok(())
}

fn on_event(_event: &AppEvent, _cx: &mut App) -> anyhow::Result<()> {
    Ok(())
}

fn on_runtime_error(_error: &RuntimeError, _cx: &mut App) {}

fn shutdown_hook(_cx: &mut App) -> anyhow::Result<()> {
    Ok(())
}

actions!(public_api_test, [Probe, OpenRepository]);

fn probe_handler(_action: &Probe, _cx: &mut App) -> anyhow::Result<()> {
    Ok(())
}

fn open_repository_handler(_action: &OpenRepository, _cx: &mut App) -> anyhow::Result<()> {
    Ok(())
}

/// A minimal application declaration exercising most declaration-time
/// builders in one place. `DesktopApp::declaration` is the one required entry
/// point; a real application's `main` calls [`AppShell::run`].
struct PublicApiTestApp;

impl DesktopApp for PublicApiTestApp {
    fn declaration() -> AppDeclaration {
        AppDeclaration::new(test_identity())
            .advanced(
                AdvancedHooks::new()
                    .path_layout(PathLayout::PlatformDefault)
                    .environment(EnvironmentPolicy::Inherit)
                    .logging(LoggingPolicy::External),
            )
            .primary_surface(Surface::new(SurfaceKey::<Empty>::primary(), build_empty))
            .launch(LaunchSpec::new(parse_unit).before_primary(before_primary))
            .setup(
                SetupModule::new(SetupKey::new("public_api.probe"), init_setup)
                    .shutdown(teardown_setup),
            )
            .start(start_hook)
            .on_event(on_event)
            .runtime_errors(on_runtime_error)
            .shutdown(shutdown_hook)
            .command(
                Command::app(CommandId::new("public_api.probe"), Probe, probe_handler)
                    .label("Probe"),
            )
            // The app-owned command a real consumer contributes into Help
            // (tsq-11.5.8), alongside the standard content `MenuBar::insert`
            // could not add there without duplicating the Help key.
            .command(
                Command::app(
                    CommandId::new("public_api.open_repository"),
                    OpenRepository,
                    open_repository_handler,
                )
                .label("Open Repository"),
            )
            // Platform-neutral on purpose: About is enabled by convention (the
            // framework default), and Help hosts it on Windows/Linux while the
            // application menu hosts it on macOS. Hiding a menu that would
            // strand a standard feature is a declaration fault by design (see
            // `commands::menu_model`'s stranded-feature validation) and its
            // explicit-platform coverage lives in that module's own tests, not
            // here — this declaration must validate identically on every host.
            // `contribute` merges "Open Repository" into Help's standard
            // content where Help exists (Windows/Linux) and inserts a new
            // Help menu before Window where it does not (macOS), never
            // duplicating the Help key either way.
            .menu_bar(MenuBar::standard().contribute(
                Menu::keyed(MenuKey::HELP).command(CommandId::new("public_api.open_repository")),
            ))
    }
}

#[test]
fn app_declaration_and_desktop_app_construct_and_validate() {
    let declaration = PublicApiTestApp::declaration();
    assert!(
        declaration.validate().is_ok(),
        "the resolved public builders produce a valid declaration",
    );

    // `AppShell::run` is the one process entry point; naming its function
    // pointer type-checks the resolved signature without starting a platform
    // loop.
    let _run: fn() -> Result<(), AppShellError> = AppShell::run::<PublicApiTestApp>;
}

#[test]
fn advanced_hooks_policies_are_public_consuming_builders() {
    fn prepare_hook(_info: &AppInfo) -> anyhow::Result<()> {
        Ok(())
    }
    fn configure_hook(
        app: neutron_components_app::gpui::Application,
    ) -> anyhow::Result<neutron_components_app::gpui::Application> {
        Ok(app)
    }

    let _hooks = AdvancedHooks::new()
        .path_layout(PathLayout::SingleRoot(".public-api-test".into()))
        .environment(EnvironmentPolicy::LoginShell)
        .logging(LoggingPolicy::External)
        .prepare(prepare_hook)
        .configure_application(configure_hook);
}

/// An `OsString` that is not valid UTF-8 on this platform, to prove
/// `ProcessLaunch::args` preserves non-UTF-8 arguments verbatim rather than
/// lossily converting them.
#[cfg(unix)]
fn non_utf8_arg() -> OsString {
    use std::os::unix::ffi::OsStringExt as _;
    // A lone 0xFF byte is never valid UTF-8; `from_vec` accepts any byte
    // sequence on Unix, so this round-trips without lossy replacement.
    OsString::from_vec(vec![0xFF, b'x'])
}

/// See the Unix definition above.
#[cfg(windows)]
fn non_utf8_arg() -> OsString {
    use std::os::windows::ffi::OsStringExt as _;
    // An unpaired UTF-16 surrogate is never valid UTF-16/UTF-8 text;
    // `from_wide` accepts any u16 sequence on Windows, so this round-trips
    // without lossy replacement.
    OsString::from_wide(&[0xD800, u16::from(b'x')])
}

#[test]
fn process_launch_and_launch_spec_are_public() {
    let empty = ProcessLaunch::empty();
    assert!(empty.args().is_empty());
    assert!(empty.cwd().is_none());

    let explicit = ProcessLaunch::new(Vec::new(), None);
    assert_eq!(explicit, ProcessLaunch::empty());

    // Both fields are public (issue #29): an external caller can construct and
    // destructure `ProcessLaunch` directly, with the exact `OsString`/`PathBuf`
    // types, and a non-UTF-8 argument survives verbatim.
    let non_utf8 = non_utf8_arg();
    let launch = ProcessLaunch {
        args: vec![OsString::from("--flag"), non_utf8.clone()],
        cwd: Some(PathBuf::from("public-api-test-cwd")),
    };
    let ProcessLaunch { args, cwd } = launch;
    assert_eq!(args, vec![OsString::from("--flag"), non_utf8]);
    assert_eq!(cwd, Some(PathBuf::from("public-api-test-cwd")));

    let _spec = LaunchSpec::new(parse_unit)
        .before_primary(before_primary)
        .primary_surface(Surface::new(
            SurfaceKey::<Empty, ()>::primary(),
            build_empty,
        ));

    match parse_unit(&empty).expect("the parser is infallible") {
        LaunchDecision::Run(()) => {}
        LaunchDecision::ExitSuccess { .. } => panic!("the unit parser always runs"),
    }
}

#[test]
fn setup_key_module_and_context_signatures_are_public() {
    let module = SetupModule::new(SetupKey::new("public_api.setup"), init_setup)
        .after(SetupKey::new("public_api.dependency"))
        .shutdown(teardown_setup);
    let _module = module;

    // `SetupContext` is handed to setup hooks by the shell; its accessors are
    // exercised through the function-pointer signature above. Name it
    // directly here too, so a removed accessor breaks this test.
    fn _uses_setup_context(cx: &mut SetupContext<'_>) {
        let _info: &AppInfo = cx.app_info();
        let _proxy: AppProxy = cx.app_proxy();
        let _app: &mut App = cx.app();
    }
}

#[test]
fn surface_and_window_types_are_public() {
    let _surface = Surface::new(SurfaceKey::<Empty>::primary(), build_empty)
        .title("Public API Test")
        .size(WindowSize::DisplayFraction(0.8))
        .menu_bar(true)
        .multiple();

    let _raw =
        RawWindow::<Empty, ()>::new(WindowKey::new("public_api.raw"), build_empty).title("Raw");

    let _overlay = OverlaySpec::new("public_api.overlay", 320.0, 240.0);

    // Runtime-only handles are named, not constructed: they only come from a
    // live `Shell::open_surface`/`open_raw` call.
    fn _uses_surface_handle(handle: &SurfaceHandle<Empty>) -> Entity<Empty> {
        handle.content().clone()
    }
    fn _uses_surface_open(open: SurfaceOpen<Empty>) {
        match open {
            SurfaceOpen::Created(_) | SurfaceOpen::Reused(_) | SurfaceOpen::InFlight => {}
        }
    }
}

#[test]
fn shell_commands_and_settings_traits_are_implemented_for_app() {
    fn requires_shell<T: Shell>() {}
    fn requires_commands<T: Commands>() {}
    fn requires_settings<T: Settings>() {}
    requires_shell::<App>();
    requires_commands::<App>();
    requires_settings::<App>();
}

#[test]
fn typed_commands_bindings_and_labels_are_public() {
    let _command = Command::app(CommandId::new("public_api.probe"), Probe, probe_handler)
        .label(CommandLabel::text("Probe"))
        .binding(CommandBinding::platform("cmd-p", "ctrl-p").key_context("PublicApi"))
        .enabled(|_| true)
        .checked(|_| false);

    let _window_command = Command::window(CommandId::new("public_api.window_probe"), Probe);

    let _label = MenuLabel::text("Public API");
    let _derived_label = MenuLabel::derived(|_cx| "Derived".into());

    // Nameable error path for a rejected registration.
    fn _uses_command_error(error: &CommandError) -> bool {
        matches!(error, CommandError::Duplicate { .. })
    }
}

#[test]
fn menu_bar_keys_and_public_outline_are_usable() {
    let outline = MenuBar::standard()
        .hide(MenuKey::VIEW)
        .insert(Menu::new(
            MenuKey::new("Tools").expect("valid key"),
            "Tools",
        ))
        .outline(DesktopPlatform::MacOs)
        .expect("the standard layout with a hidden optional menu is valid");

    assert!(!outline.menus().is_empty());

    fn _uses_menu_types(
        outline: &MenuOutline,
        entry: &MenuOutlineEntry,
        node: &MenuNode,
        section: MenuSectionKey,
    ) -> MenuKey {
        let _ = (outline.menus(), entry.label(), node, section);
        entry.key()
    }

    // A structural fault is reported through the public `DeclarationError`
    // vocabulary, not a private fault type.
    let faults = MenuBar::custom(vec![
        Menu::keyed(MenuKey::EDIT).command(CommandId::new("a")),
    ])
    .hide(MenuKey::VIEW)
    .outline(DesktopPlatform::Linux)
    .expect_err("hide does not apply to a custom menu bar");
    assert!(faults.iter().any(|error| matches!(
        error,
        DeclarationError::Command {
            fault: CommandFault::InvalidStandardEdit { .. }
        }
    )));
    let _errors: &DeclarationErrors = &faults;
}

#[test]
fn contribute_merges_into_help_and_app_declaration_validates_declared_references() {
    let open_repository = CommandId::new("public_api.open_repository");

    // External nameability: `MenuBar::contribute` is reachable through the
    // crate root import above, and merges into Help on every platform
    // without ever duplicating the Help key. `MenuBar::outline` alone only
    // validates structure, so it accepts a reference to an undeclared
    // command here.
    for platform in [
        DesktopPlatform::MacOs,
        DesktopPlatform::Windows,
        DesktopPlatform::Linux,
    ] {
        let outline = MenuBar::standard()
            .contribute(Menu::keyed(MenuKey::HELP).command(open_repository))
            .outline(platform)
            .expect("contribute is structurally valid even for an undeclared command");
        assert_eq!(
            outline
                .menus()
                .iter()
                .filter(|menu| menu.key() == MenuKey::HELP)
                .count(),
            1,
            "{platform:?} must not gain a duplicate Help key",
        );
        assert!(outline.command_ids().contains(&open_repository));
    }

    // `AppDeclaration::validate` goes further than `MenuBar::outline`: the
    // full declaration below declares `open_repository` and contributes it
    // to Help, and validates cleanly on every platform.
    PublicApiTestApp::declaration()
        .validate()
        .expect("a declared command contributed to Help validates");

    // Swap in a Help contribution referencing a command the declaration
    // never declared: `AppDeclaration::validate` rejects the dangling
    // reference that `MenuBar::outline` alone would not have caught.
    let undeclared = CommandId::new("public_api.never_declared");
    let errors = PublicApiTestApp::declaration()
        .menu_bar(MenuBar::standard().contribute(Menu::keyed(MenuKey::HELP).command(undeclared)))
        .validate()
        .expect_err("a Help contribution referencing an undeclared command must fail validation");
    assert!(errors.iter().any(|error| matches!(
        error,
        DeclarationError::Command {
            fault: CommandFault::UnknownCommand { menu, command }
        } if *menu == MenuKey::HELP && *command == undeclared
    )));
}

#[test]
fn lifecycle_error_and_capability_types_are_public() {
    let started = AppEvent::Started;
    assert_eq!(started.name(), "started");
    let _reopened = AppEvent::Reopened;
    let _open_requested = AppEvent::OpenRequested(OpenRequest::default());
    let _last_window_closed = AppEvent::LastWindowClosed;
    let _shutdown_requested = AppEvent::ShutdownRequested(ShutdownReason::Requested);
    let _will_exit = AppEvent::WillExit;

    let runtime_error = RuntimeError::lifecycle(started, anyhow::anyhow!("probe"));
    assert_eq!(runtime_error.operation(), RuntimeOperation::Lifecycle);
    assert!(matches!(runtime_error.event(), Some(AppEvent::Started)));

    let command_error =
        RuntimeError::command(CommandId::new("public_api.probe"), anyhow::anyhow!("probe"));
    assert_eq!(
        command_error.command_id(),
        Some(CommandId::new("public_api.probe")),
    );

    fn _uses_app_shell_error(error: &AppShellError) -> bool {
        matches!(error, AppShellError::Declaration(_))
    }
    fn _uses_app_closed(_: AppClosed) {}

    let capabilities = PlatformCapabilities::detect();
    let _capability: Capability = capabilities.get(PlatformCapability::Tray);
    fn _uses_unsupported_capability(_: &UnsupportedCapability) {}
}
