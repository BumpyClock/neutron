#![allow(
    clippy::disallowed_methods,
    reason = "process integration test must supervise synchronous child processes"
)]

//! Headless declared-application contract tests.
//!
//! Drives real applications through the declared path —
//! `declaration::run::testing::run::<A>()` / `run_with::<A>(ProcessLaunch)` —
//! all the way through `execute -> AppDeclaration::lower -> RuntimePlan::run ->
//! Startup::run`, on the process main thread via headless child processes.
//! There is no second runtime under test: this is the exact path the public
//! `AppShell::run::<A>()` calls.
//!
//! Uses `harness = false` (see this crate's `Cargo.toml`): GPUI panics unless
//! its `App` is constructed on the main thread, which the default test harness
//! (worker threads) cannot provide.
//!
//! Every child asserts its own outcome (the `Result` from `run`/`run_with` and
//! the exact flattened order recorded by non-capturing `fn` hooks into a
//! process-static log — process isolation stands in for per-scenario state
//! since declaration hooks cannot capture) and only then prints a post-return
//! sentinel, so a platform `exit(0)` or a silently swallowed panic cannot pass
//! the test. A parent-side timeout only bounds a hung child regression.
//!
//! This requires the Stage 1 GPUI normal-return contract and runs against the
//! engine source in the root workspace.

use std::ffi::OsString;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;

use neutron_components_app::gpui::{AppContext as _, Application, Empty, Entity, Window};
use neutron_components_app::prelude::*;
use neutron_components_app::testing::{run, run_with};
use neutron_components_app::{
    AdvancedHooks, AppDeclaration, LaunchDecision, LaunchSpec, ProcessLaunch, SetupContext,
    SetupKey, SetupModule, Surface, SurfaceKey,
};
use wait_timeout::ChildExt;

const CHILD_TIMEOUT: Duration = Duration::from_secs(30);

const SUCCESS_CHILD: &str = "--success-child";
const DECLARATION_FAILURE_CHILD: &str = "--declaration-failure-child";
const LAUNCH_PARSE_FAILURE_CHILD: &str = "--launch-parse-failure-child";
const EXIT_SUCCESS_CHILD: &str = "--exit-success-child";
const SETUP_INIT_FAILURE_CHILD: &str = "--setup-init-failure-child";
const COMMON_START_FAILURE_CHILD: &str = "--common-start-failure-child";
const DEFERRED_QUIT_CHILD: &str = "--deferred-quit-child";
const BEFORE_PRIMARY_QUIT_CHILD: &str = "--before-primary-quit-child";
const PREPARE_FAILURE_CHILD: &str = "--prepare-failure-child";
const CONFIGURE_APPLICATION_FAILURE_CHILD: &str = "--configure-application-failure-child";
const EXPLICIT_PROCESS_FACTS_CHILD: &str = "--explicit-process-facts-child";

const SUCCESS_RETURNED: &str = "APPSHELL_HEADLESS_SUCCESS_RETURNED";
const DECLARATION_FAILURE_RETURNED: &str = "APPSHELL_HEADLESS_DECLARATION_FAILURE_RETURNED";
const LAUNCH_PARSE_FAILURE_RETURNED: &str = "APPSHELL_HEADLESS_LAUNCH_PARSE_FAILURE_RETURNED";
const EXIT_SUCCESS_RETURNED: &str = "APPSHELL_HEADLESS_EXIT_SUCCESS_RETURNED";
const SETUP_INIT_FAILURE_RETURNED: &str = "APPSHELL_HEADLESS_SETUP_INIT_FAILURE_RETURNED";
const COMMON_START_FAILURE_RETURNED: &str = "APPSHELL_HEADLESS_COMMON_START_FAILURE_RETURNED";
const DEFERRED_QUIT_RETURNED: &str = "APPSHELL_HEADLESS_DEFERRED_QUIT_RETURNED";
const BEFORE_PRIMARY_QUIT_RETURNED: &str = "APPSHELL_HEADLESS_BEFORE_PRIMARY_QUIT_RETURNED";
const PREPARE_FAILURE_RETURNED: &str = "APPSHELL_HEADLESS_PREPARE_FAILURE_RETURNED";
const CONFIGURE_APPLICATION_FAILURE_RETURNED: &str =
    "APPSHELL_HEADLESS_CONFIGURE_APPLICATION_FAILURE_RETURNED";
const EXPLICIT_PROCESS_FACTS_RETURNED: &str = "APPSHELL_HEADLESS_EXPLICIT_PROCESS_FACTS_RETURNED";

/// Text the `--exit-success-child`'s launch parser asks the shell to print,
/// verbatim, before any path, platform, or GPUI work.
const EXIT_SUCCESS_STDOUT: &str = "usage: probe\n";

fn main() {
    let mut args = std::env::args();
    let role = args.nth(1);
    match role.as_deref() {
        Some(role) if role == SUCCESS_CHILD => return assert_success_child(),
        Some(role) if role == DECLARATION_FAILURE_CHILD => {
            return assert_declaration_failure_child();
        }
        Some(role) if role == LAUNCH_PARSE_FAILURE_CHILD => {
            return assert_launch_parse_failure_child();
        }
        Some(role) if role == EXIT_SUCCESS_CHILD => return assert_exit_success_child(),
        Some(role) if role == SETUP_INIT_FAILURE_CHILD => {
            return assert_setup_init_failure_child();
        }
        Some(role) if role == COMMON_START_FAILURE_CHILD => {
            return assert_common_start_failure_child();
        }
        Some(role) if role == DEFERRED_QUIT_CHILD => return assert_deferred_quit_child(),
        Some(role) if role == BEFORE_PRIMARY_QUIT_CHILD => {
            return assert_before_primary_quit_child();
        }
        Some(role) if role == PREPARE_FAILURE_CHILD => return assert_prepare_failure_child(),
        Some(role) if role == CONFIGURE_APPLICATION_FAILURE_CHILD => {
            return assert_configure_application_failure_child();
        }
        Some(role) if role == EXPLICIT_PROCESS_FACTS_CHILD => {
            return assert_explicit_process_facts_child();
        }
        _ => {}
    }

    assert_child_returns(SUCCESS_CHILD, SUCCESS_RETURNED);
    assert_child_returns(DECLARATION_FAILURE_CHILD, DECLARATION_FAILURE_RETURNED);
    assert_child_returns(LAUNCH_PARSE_FAILURE_CHILD, LAUNCH_PARSE_FAILURE_RETURNED);
    let exit_success_stdout = assert_child_returns(EXIT_SUCCESS_CHILD, EXIT_SUCCESS_RETURNED);
    assert!(
        exit_success_stdout.contains(EXIT_SUCCESS_STDOUT),
        "exit-success child did not print the parser's stdout text verbatim:\n{exit_success_stdout}",
    );
    assert_child_returns(SETUP_INIT_FAILURE_CHILD, SETUP_INIT_FAILURE_RETURNED);
    assert_child_returns(COMMON_START_FAILURE_CHILD, COMMON_START_FAILURE_RETURNED);
    assert_child_returns(DEFERRED_QUIT_CHILD, DEFERRED_QUIT_RETURNED);
    assert_child_returns(BEFORE_PRIMARY_QUIT_CHILD, BEFORE_PRIMARY_QUIT_RETURNED);
    assert_child_returns(PREPARE_FAILURE_CHILD, PREPARE_FAILURE_RETURNED);
    assert_child_returns(
        CONFIGURE_APPLICATION_FAILURE_CHILD,
        CONFIGURE_APPLICATION_FAILURE_RETURNED,
    );
    assert_child_returns(
        EXPLICIT_PROCESS_FACTS_CHILD,
        EXPLICIT_PROCESS_FACTS_RETURNED,
    );
}

/// Spawn this same binary with `role`, wait bounded by [`CHILD_TIMEOUT`],
/// assert it returned normally and reached its post-run sentinel, and return
/// its captured stdout for any scenario-specific text assertion.
fn assert_child_returns(role: &str, sentinel: &str) -> String {
    let mut child = Command::new(std::env::current_exe().expect("current test binary"))
        .arg(role)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start headless child");
    let timed_out = child
        .wait_timeout(CHILD_TIMEOUT)
        .expect("wait for headless child")
        .is_none();
    if timed_out {
        if let Err(error) = child.kill()
            && error.kind() != ErrorKind::InvalidInput
        {
            panic!("kill timed-out headless child {role}: {error}");
        }
    }
    let output = child
        .wait_with_output()
        .expect("collect headless child output");
    if timed_out {
        panic!(
            "headless child {role} exceeded {CHILD_TIMEOUT:?}\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }
    assert!(
        output.status.success(),
        "headless child {role} did not return normally: {}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        stdout.lines().any(|line| line == sentinel),
        "headless child {role} exited without reaching its post-run sentinel {sentinel}\nstdout:\n{stdout}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
    stdout
}

// ---- Recording: a process-static log, since every declared hook below is a
// non-capturing `fn` pointer (see `declaration::lifecycle`/`setup`/`launch`).
// Each scenario is its own child process, so a fresh, empty log is process
// isolation, not a shared/reset fixture. ----

static LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn record(event: impl Into<String>) {
    LOG.lock()
        .expect("recording log poisoned")
        .push(event.into());
}

fn assert_log(expected: &[&str]) {
    let actual = LOG.lock().expect("recording log poisoned").clone();
    let expected: Vec<String> = expected.iter().map(|s| s.to_string()).collect();
    assert_eq!(actual, expected, "observed order did not match");
}

fn test_identity() -> IdentityRef {
    IdentityRef {
        app_id: "com.example.appshelltest",
        display_name: "App Shell Test",
        data_namespace: "appshelltest",
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

fn broken_identity() -> IdentityRef {
    let mut broken = test_identity();
    broken.app_id = "";
    broken
}

/// The typed launch value: an application-owned argument type, distinct from
/// the framework's own types, so the typed path cannot be self-satisfying.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LaunchArgs(u32);

// ---- AdvancedHooks: process preparation and GPUI `Application` customization,
// both exercised through the lowered declaration (requirement 6). ----

fn prepare_ok(_info: &AppInfo) -> anyhow::Result<()> {
    record("advanced:prepare");
    Ok(())
}

fn configure_ok(application: Application) -> anyhow::Result<Application> {
    record("advanced:configure_application");
    Ok(application)
}

fn prepare_failing(_info: &AppInfo) -> anyhow::Result<()> {
    record("advanced:prepare");
    anyhow::bail!("deliberate prepare failure")
}

fn configure_failing(_application: Application) -> anyhow::Result<Application> {
    record("advanced:configure_application");
    anyhow::bail!("deliberate configure_application failure")
}

/// Reaching this proves `execute()` did *not* stop validation/parsing before
/// it should have: `prepare` only ever runs once identity, paths, and
/// capabilities have resolved, well after declaration validation and launch
/// parsing, and strictly before the platform is constructed.
fn prepare_tripwire(_info: &AppInfo) -> anyhow::Result<()> {
    panic!("prepare must not run: the declared path must stop before the platform")
}

/// Reaching this proves a `prepare` failure aborts before `configure_application`.
fn configure_application_tripwire(_application: Application) -> anyhow::Result<Application> {
    panic!("configure_application must not run: prepare already failed")
}

// ---- Setup: two application setup modules with an explicit dependency. ----

fn init_first(_cx: &mut SetupContext<'_>) -> anyhow::Result<()> {
    record("setup:first:init");
    Ok(())
}

fn teardown_first(_state: (), _cx: &mut SetupContext<'_>) -> anyhow::Result<()> {
    record("setup:first:teardown");
    Ok(())
}

fn init_second(_cx: &mut SetupContext<'_>) -> anyhow::Result<()> {
    record("setup:second:init");
    Ok(())
}

fn teardown_second(_state: (), _cx: &mut SetupContext<'_>) -> anyhow::Result<()> {
    record("setup:second:teardown");
    Ok(())
}

fn init_second_failing(_cx: &mut SetupContext<'_>) -> anyhow::Result<()> {
    record("setup:second:init");
    anyhow::bail!("deliberate setup init failure")
}

// ---- Lifecycle: common start, observers, the nonfatal reporter, and app
// shutdown. ----

fn start_ok(_cx: &mut App) -> anyhow::Result<()> {
    record("start");
    Ok(())
}

fn start_failing(_cx: &mut App) -> anyhow::Result<()> {
    record("start");
    anyhow::bail!("deliberate common start failure")
}

fn start_requests_quit(cx: &mut App) -> anyhow::Result<()> {
    record("start");
    cx.request_quit();
    Ok(())
}

/// The first `Started` observer: a deliberate nonfatal error, reported through
/// `on_error` without stopping the remaining observers.
fn on_event_erroring(event: &AppEvent, _cx: &mut App) -> anyhow::Result<()> {
    if matches!(event, AppEvent::Started) {
        record("on_event:started:erroring");
        anyhow::bail!("deliberate nonfatal observer error")
    }
    Ok(())
}

/// The later observer: continues past the first observer's error, requests an
/// orderly quit on `Started`, and records the events it also observes.
fn on_event_continues_and_quits(event: &AppEvent, cx: &mut App) -> anyhow::Result<()> {
    match event {
        AppEvent::Started => {
            record("on_event:started:continuation_quit");
            cx.request_quit();
        }
        AppEvent::ShutdownRequested(_) => record("on_event:shutdown_requested"),
        AppEvent::WillExit => record("on_event:will_exit"),
        _ => {}
    }
    Ok(())
}

fn on_error_reported(error: &RuntimeError, _cx: &mut App) {
    assert_eq!(error.operation(), RuntimeOperation::Lifecycle);
    assert!(matches!(error.event(), Some(AppEvent::Started)));
    record("on_error:reported");
}

fn app_shutdown_ok(_cx: &mut App) -> anyhow::Result<()> {
    record("app:shutdown");
    Ok(())
}

// ---- Launch: the typed parser, the launch-specific `before_primary` hook,
// and the typed primary surface's build/open hooks. ----

fn parse_ok(_process: &ProcessLaunch) -> anyhow::Result<LaunchDecision<LaunchArgs>> {
    Ok(LaunchDecision::Run(LaunchArgs(42)))
}

fn parse_fails(_process: &ProcessLaunch) -> anyhow::Result<LaunchDecision<LaunchArgs>> {
    anyhow::bail!("unrecognized argument")
}

fn parse_help(_process: &ProcessLaunch) -> anyhow::Result<LaunchDecision<LaunchArgs>> {
    Ok(LaunchDecision::ExitSuccess {
        stdout: Some(EXIT_SUCCESS_STDOUT.to_string()),
    })
}

/// The exact args/cwd the explicit-facts scenario injects via
/// [`ProcessLaunch::new`], and that its parser reads back through
/// [`ProcessLaunch::args`]/[`ProcessLaunch::cwd`].
const EXPLICIT_PROCESS_ARGS: &[&str] = &["--seed", "7"];
const EXPLICIT_PROCESS_CWD: &str = "/tmp/appshell-headless-explicit-cwd";

fn explicit_process_launch() -> ProcessLaunch {
    ProcessLaunch::new(
        EXPLICIT_PROCESS_ARGS.iter().map(OsString::from).collect(),
        Some(PathBuf::from(EXPLICIT_PROCESS_CWD)),
    )
}

/// A parser that hard-asserts the exact injected [`ProcessLaunch`] facts
/// (not just their presence): a bug that had `execute()` silently substitute
/// `ProcessLaunch::empty()` for the real, injected value would panic here
/// instead of quietly parsing nothing.
fn parse_explicit_facts(process: &ProcessLaunch) -> anyhow::Result<LaunchDecision<LaunchArgs>> {
    let expected_args: Vec<OsString> = EXPLICIT_PROCESS_ARGS.iter().map(OsString::from).collect();
    assert_eq!(
        process.args(),
        expected_args.as_slice(),
        "execute() must forward the injected ProcessLaunch args verbatim",
    );
    assert_eq!(
        process.cwd(),
        Some(Path::new(EXPLICIT_PROCESS_CWD)),
        "execute() must forward the injected ProcessLaunch cwd verbatim",
    );
    let seed: u32 = process.args()[1]
        .to_str()
        .expect("seed argument must be valid UTF-8")
        .parse()
        .expect("seed argument must be a valid u32");
    Ok(LaunchDecision::Run(LaunchArgs(seed)))
}

fn before_primary_ok(value: &LaunchArgs, _cx: &mut App) -> anyhow::Result<()> {
    record(format!("before_primary:{}", value.0));
    Ok(())
}

fn before_primary_requests_quit(value: &LaunchArgs, cx: &mut App) -> anyhow::Result<()> {
    record(format!("before_primary:{}", value.0));
    cx.request_quit();
    Ok(())
}

fn build_primary(value: &LaunchArgs, _window: &mut Window, cx: &mut App) -> Entity<Empty> {
    record(format!("primary:build:{}", value.0));
    cx.new(|_| Empty)
}

fn primary_opened(_view: &Entity<Empty>, _window: &mut Window, _cx: &mut App) {
    record("primary:opened");
}

/// The complete typed launch declared by every scenario that needs a primary
/// surface: parser, `before_primary`, and the typed primary itself.
fn typed_launch(
    parser: fn(&ProcessLaunch) -> anyhow::Result<LaunchDecision<LaunchArgs>>,
) -> LaunchSpec<LaunchArgs> {
    LaunchSpec::new(parser)
        .before_primary(before_primary_ok)
        .primary_surface(
            Surface::new(SurfaceKey::new("primary"), build_primary).after_open(primary_opened),
        )
}

/// The two setup modules every scenario below declares, in order, wired with
/// an explicit `after` dependency so their teardown order proves reversal.
fn setup_pair(declaration: AppDeclaration) -> AppDeclaration {
    declaration
        .setup(SetupModule::new(SetupKey::new("app.first"), init_first).shutdown(teardown_first))
        .setup(
            SetupModule::new(SetupKey::new("app.second"), init_second)
                .after(SetupKey::new("app.first"))
                .shutdown(teardown_second),
        )
}

// ---- Declared applications: one type per scenario, since `DesktopApp` binds
// the whole declaration to a type, not a value. ----

struct SuccessApp;

impl DesktopApp for SuccessApp {
    fn declaration() -> AppDeclaration {
        setup_pair(
            AppDeclaration::new(test_identity())
                .advanced(
                    AdvancedHooks::new()
                        .prepare(prepare_ok)
                        .configure_application(configure_ok),
                )
                .start(start_ok)
                .on_event(on_event_erroring)
                .on_event(on_event_continues_and_quits)
                .runtime_errors(on_error_reported)
                .shutdown(app_shutdown_ok),
        )
        .launch(typed_launch(parse_ok))
    }
}

struct DeclarationFailureApp;

impl DesktopApp for DeclarationFailureApp {
    fn declaration() -> AppDeclaration {
        AppDeclaration::new(broken_identity())
            .advanced(AdvancedHooks::new().prepare(prepare_tripwire))
    }
}

struct LaunchParseFailureApp;

impl DesktopApp for LaunchParseFailureApp {
    fn declaration() -> AppDeclaration {
        AppDeclaration::new(test_identity())
            .advanced(AdvancedHooks::new().prepare(prepare_tripwire))
            .launch(LaunchSpec::new(parse_fails))
    }
}

struct ExitSuccessApp;

impl DesktopApp for ExitSuccessApp {
    fn declaration() -> AppDeclaration {
        AppDeclaration::new(test_identity())
            .advanced(AdvancedHooks::new().prepare(prepare_tripwire))
            .launch(LaunchSpec::new(parse_help))
    }
}

struct SetupInitFailureApp;

impl DesktopApp for SetupInitFailureApp {
    fn declaration() -> AppDeclaration {
        AppDeclaration::new(test_identity())
            .advanced(
                AdvancedHooks::new()
                    .prepare(prepare_ok)
                    .configure_application(configure_ok),
            )
            .setup(
                SetupModule::new(SetupKey::new("app.first"), init_first).shutdown(teardown_first),
            )
            .setup(
                SetupModule::new(SetupKey::new("app.second"), init_second_failing)
                    .after(SetupKey::new("app.first"))
                    .shutdown(teardown_second),
            )
            .on_event(on_event_continues_and_quits)
            .shutdown(app_shutdown_ok)
    }
}

struct CommonStartFailureApp;

impl DesktopApp for CommonStartFailureApp {
    fn declaration() -> AppDeclaration {
        setup_pair(
            AppDeclaration::new(test_identity())
                .advanced(
                    AdvancedHooks::new()
                        .prepare(prepare_ok)
                        .configure_application(configure_ok),
                )
                .start(start_failing)
                .on_event(on_event_continues_and_quits)
                .shutdown(app_shutdown_ok),
        )
    }
}

struct DeferredQuitApp;

impl DesktopApp for DeferredQuitApp {
    fn declaration() -> AppDeclaration {
        setup_pair(
            AppDeclaration::new(test_identity())
                .advanced(
                    AdvancedHooks::new()
                        .prepare(prepare_ok)
                        .configure_application(configure_ok),
                )
                .start(start_requests_quit)
                .on_event(on_event_continues_and_quits)
                .shutdown(app_shutdown_ok),
        )
        .launch(typed_launch(parse_ok))
    }
}

/// Proves requirement 2: `run_with` forwards a non-empty, explicitly
/// constructed [`ProcessLaunch`] all the way to the parser, and the typed
/// value it produces flows unchanged into `before_primary` and the primary's
/// build hook.
struct ExplicitProcessFactsApp;

impl DesktopApp for ExplicitProcessFactsApp {
    fn declaration() -> AppDeclaration {
        AppDeclaration::new(test_identity())
            .start(start_ok)
            .on_event(on_event_continues_and_quits)
            .launch(typed_launch(parse_explicit_facts))
    }
}

/// A quit requested from `before_primary` itself (rather than from `start`)
/// is caught by the *inner* deferred-quit guard: `before_primary` still runs
/// and is logged, but the typed primary's build/open and `Started` are all
/// suppressed.
struct BeforePrimaryQuitApp;

impl DesktopApp for BeforePrimaryQuitApp {
    fn declaration() -> AppDeclaration {
        setup_pair(
            AppDeclaration::new(test_identity())
                .advanced(
                    AdvancedHooks::new()
                        .prepare(prepare_ok)
                        .configure_application(configure_ok),
                )
                .start(start_ok)
                .on_event(on_event_continues_and_quits)
                .shutdown(app_shutdown_ok),
        )
        .launch(
            LaunchSpec::new(parse_ok)
                .before_primary(before_primary_requests_quit)
                .primary_surface(
                    Surface::new(SurfaceKey::new("primary"), build_primary)
                        .after_open(primary_opened),
                ),
        )
    }
}

struct PrepareFailureApp;

impl DesktopApp for PrepareFailureApp {
    fn declaration() -> AppDeclaration {
        AppDeclaration::new(test_identity()).advanced(
            AdvancedHooks::new()
                .prepare(prepare_failing)
                .configure_application(configure_application_tripwire),
        )
    }
}

struct ConfigureApplicationFailureApp;

impl DesktopApp for ConfigureApplicationFailureApp {
    fn declaration() -> AppDeclaration {
        AppDeclaration::new(test_identity()).advanced(
            AdvancedHooks::new()
                .prepare(prepare_ok)
                .configure_application(configure_failing),
        )
    }
}

// ---- Scenarios ----

/// The full flattened observable order: setup init, common start,
/// `before_primary`, the typed primary's build/open, `Started`, a deliberate
/// nonfatal observer error, the later observer's continuation and quit,
/// `ShutdownRequested`, `WillExit`, app shutdown, and setup teardown in exact
/// reverse — driven by `testing::run::<SuccessApp>()` (the zero-argument
/// entry point) all the way to a live declared primary surface.
fn assert_success_child() {
    let result = run::<SuccessApp>();

    assert!(
        result.is_ok(),
        "headless declared run returned error: {result:?}"
    );
    assert_log(&[
        "advanced:prepare",
        "advanced:configure_application",
        "setup:first:init",
        "setup:second:init",
        "start",
        "before_primary:42",
        "primary:build:42",
        "primary:opened",
        "on_event:started:erroring",
        "on_event:started:continuation_quit",
        "on_error:reported",
        "on_event:shutdown_requested",
        "on_event:will_exit",
        "app:shutdown",
        "setup:second:teardown",
        "setup:first:teardown",
    ]);
    println!("{SUCCESS_RETURNED}");
}

/// A malformed declaration (empty `app_id`) is reported as
/// [`AppShellError::Declaration`] before paths, the platform, or GPUI exist.
/// The `prepare` tripwire proves it: if validation ever ran after
/// preparation, the child would panic instead of returning this error.
fn assert_declaration_failure_child() {
    let result = run_with::<DeclarationFailureApp>(ProcessLaunch::empty());

    assert!(
        matches!(result, Err(AppShellError::Declaration(_))),
        "expected a declaration error, got {result:?}",
    );
    println!("{DECLARATION_FAILURE_RETURNED}");
}

/// A launch parser failure is reported as [`AppShellError::Launch`], also
/// before the platform: the same `prepare` tripwire proves it.
fn assert_launch_parse_failure_child() {
    let result = run_with::<LaunchParseFailureApp>(ProcessLaunch::empty());

    assert!(
        matches!(result, Err(AppShellError::Launch(_))),
        "expected a launch parse error, got {result:?}",
    );
    println!("{LAUNCH_PARSE_FAILURE_RETURNED}");
}

/// A `--help`-style [`LaunchDecision::ExitSuccess`] returns `Ok(())` and
/// writes `stdout` before any path, platform, or GPUI work — proved by the
/// same `prepare` tripwire never firing.
fn assert_exit_success_child() {
    let result = run_with::<ExitSuccessApp>(ProcessLaunch::empty());

    assert!(
        result.is_ok(),
        "an exit-success request must return Ok: {result:?}"
    );
    println!("{EXIT_SUCCESS_RETURNED}");
}

/// A setup module's initialization failure is fatal
/// ([`AppShellError::Module`]); the pipeline rolls its own already-initialized
/// prefix back (here, `first`) before the module ordering is even reported,
/// and the never-initialized module's teardown never runs. Because this fails
/// before the common start phase begins, the application shutdown hook never
/// runs either.
fn assert_setup_init_failure_child() {
    let result = run_with::<SetupInitFailureApp>(ProcessLaunch::empty());

    match result {
        Err(AppShellError::Module { module, .. }) => assert_eq!(module, "app.second"),
        other => panic!("expected a module init error, got {other:?}"),
    }
    assert_log(&[
        "advanced:prepare",
        "advanced:configure_application",
        "setup:first:init",
        "setup:second:init",
        "setup:first:teardown",
        "on_event:shutdown_requested",
        "on_event:will_exit",
    ]);
    println!("{SETUP_INIT_FAILURE_RETURNED}");
}

/// A common-start failure is reported as [`AppShellError::Startup`]. Unlike a
/// setup failure, the application startup transaction was already entered
/// before the hook ran, so teardown still runs the application shutdown hook
/// and both setup modules tear down in reverse.
fn assert_common_start_failure_child() {
    let result = run_with::<CommonStartFailureApp>(ProcessLaunch::empty());

    assert!(
        matches!(result, Err(AppShellError::Startup(_))),
        "expected a startup error, got {result:?}",
    );
    assert_log(&[
        "advanced:prepare",
        "advanced:configure_application",
        "setup:first:init",
        "setup:second:init",
        "start",
        "on_event:shutdown_requested",
        "on_event:will_exit",
        "app:shutdown",
        "setup:second:teardown",
        "setup:first:teardown",
    ]);
    println!("{COMMON_START_FAILURE_RETURNED}");
}

/// A quit requested from the common start hook is deferred: it suppresses the
/// declared `before_primary` hook, the typed primary's build/open, and
/// `Started` entirely (none of `"before_primary"`, `"primary:build"`,
/// `"primary:opened"`, or an `"on_event:started:*"` entry appear below), while
/// `ShutdownRequested`, `WillExit`, app shutdown, and setup teardown still run
/// normally and the run still returns `Ok(())`.
fn assert_deferred_quit_child() {
    let result = run_with::<DeferredQuitApp>(ProcessLaunch::empty());

    assert!(
        result.is_ok(),
        "a deferred quit must still return Ok: {result:?}"
    );
    assert_log(&[
        "advanced:prepare",
        "advanced:configure_application",
        "setup:first:init",
        "setup:second:init",
        "start",
        "on_event:shutdown_requested",
        "on_event:will_exit",
        "app:shutdown",
        "setup:second:teardown",
        "setup:first:teardown",
    ]);
    println!("{DEFERRED_QUIT_RETURNED}");
}

/// A quit requested from `before_primary` itself is caught by the *inner*
/// deferred-quit guard, distinct from the outer guard `assert_deferred_quit_child`
/// proves: `before_primary` still runs and is logged (`"before_primary:42"`
/// appears below), but the typed primary's build/open and `Started` are all
/// suppressed (no `"primary:build:*"`, `"primary:opened"`, or
/// `"on_event:started:*"` entry appears), while `ShutdownRequested`,
/// `WillExit`, app shutdown, and setup teardown still run and the run still
/// returns `Ok(())`.
fn assert_before_primary_quit_child() {
    let result = run_with::<BeforePrimaryQuitApp>(ProcessLaunch::empty());

    assert!(
        result.is_ok(),
        "a before_primary-requested quit must still return Ok: {result:?}"
    );
    assert_log(&[
        "advanced:prepare",
        "advanced:configure_application",
        "setup:first:init",
        "setup:second:init",
        "start",
        "before_primary:42",
        "on_event:shutdown_requested",
        "on_event:will_exit",
        "app:shutdown",
        "setup:second:teardown",
        "setup:first:teardown",
    ]);
    println!("{BEFORE_PRIMARY_QUIT_RETURNED}");
}

/// A failing `prepare` hook surfaces as [`AppShellError::Preparation`] through
/// the lowered declaration, not by calling the mapping helper directly. The
/// `configure_application` tripwire proves the failure aborts immediately.
fn assert_prepare_failure_child() {
    let result = run_with::<PrepareFailureApp>(ProcessLaunch::empty());

    assert!(
        matches!(result, Err(AppShellError::Preparation(_))),
        "expected a preparation error, got {result:?}",
    );
    assert_log(&["advanced:prepare"]);
    println!("{PREPARE_FAILURE_RETURNED}");
}

/// A failing `configure_application` hook also surfaces as
/// [`AppShellError::Preparation`] through the lowered declaration.
fn assert_configure_application_failure_child() {
    let result = run_with::<ConfigureApplicationFailureApp>(ProcessLaunch::empty());

    assert!(
        matches!(result, Err(AppShellError::Preparation(_))),
        "expected a preparation error, got {result:?}",
    );
    assert_log(&["advanced:prepare", "advanced:configure_application"]);
    println!("{CONFIGURE_APPLICATION_FAILURE_RETURNED}");
}

/// `run_with` forwards a non-empty, explicitly constructed [`ProcessLaunch`]
/// (distinct args and an explicit `cwd`, not the default-constructed
/// `ProcessLaunch::empty()` every other scenario injects) all the way to the
/// parser: `parse_explicit_facts` hard-asserts the exact values, and the
/// typed `LaunchArgs` it derives from them is visible unchanged in both
/// `before_primary` and the primary's build hook (`"before_primary:7"` /
/// `"primary:build:7"` below, not the `LaunchArgs(42)` every other scenario's
/// `parse_ok` produces).
fn assert_explicit_process_facts_child() {
    let result = run_with::<ExplicitProcessFactsApp>(explicit_process_launch());

    assert!(
        result.is_ok(),
        "headless declared run with explicit process facts returned error: {result:?}"
    );
    assert_log(&[
        "start",
        "before_primary:7",
        "primary:build:7",
        "primary:opened",
        "on_event:started:continuation_quit",
        "on_event:shutdown_requested",
        "on_event:will_exit",
    ]);
    println!("{EXPLICIT_PROCESS_FACTS_RETURNED}");
}
