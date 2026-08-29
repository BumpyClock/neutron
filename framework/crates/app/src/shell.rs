//! The application entry point and the direct runtime plan it executes.
//!
//! [`AppDeclaration`](crate::declaration::AppDeclaration) lowers itself into
//! one [`RuntimePlan`]: finalized identity, assets, process policies, liveness
//! and activation, the ordered runtime modules, the lifecycle
//! observers/reporter/shutdown hook, the typed [`LaunchRuntime`], and the
//! platform runner. [`RuntimePlan::run`] is the single startup implementation.
//!
//! Work that must happen before the GPUI event loop (path resolution,
//! env/logging policy, the declared `prepare` hook, module `prepare`) runs
//! first; everything after platform construction runs inside the run closure
//! with a raw `&mut gpui::App` ([`Startup::run`]).

use std::borrow::Cow;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use gpui::{App, Application, AssetSource, QuitMode, SharedString};
use neutron_components_manifest::schema::IdentityRef;
use neutron_components_storage::{AppPaths, PathLayout};

use crate::capabilities::PlatformCapabilities;
use crate::declaration::LaunchRuntime;
use crate::error::{AppShellError, RuntimeError};
use crate::handles::{self, AppInfo, AppShutdownHook, PendingEvents};
use crate::lifecycle::AppEvent;
use crate::liveness::{ExitPolicy, InitialActivation, Liveness};
use crate::module::{EventHandler, RuntimeModules};

/// The declared common start hook, boxed for the plan.
pub(crate) type StartCallback = Box<dyn FnOnce(&mut App) -> anyhow::Result<()> + 'static>;
/// The declared observer of nonfatal runtime errors, boxed for the plan.
pub(crate) type ErrorReporter = Box<dyn Fn(&RuntimeError, &mut App) + 'static>;
/// The declared preparation hook, run once [`AppInfo`] exists (identity, paths,
/// capabilities resolved) and before module `prepare`. Still no GPUI.
pub(crate) type PrepareHook = Box<dyn FnOnce(&AppInfo) -> Result<(), AppShellError> + 'static>;
/// The declared GPUI [`Application`] customization.
pub(crate) type ConfigureApplication =
    Box<dyn FnOnce(Application) -> Result<Application, AppShellError> + 'static>;

/// Process-global environment policy (plan §3 — explicit, never a silent
/// default). Applied once before the platform is constructed.
#[non_exhaustive]
pub enum EnvironmentPolicy {
    /// Inherit the launching environment unchanged (default; safe for a library).
    Inherit,
    /// Repair `PATH` from the user's login shell.
    ///
    /// GUI-launched processes on macOS (Finder/Dock/`launchd`) and Linux
    /// (desktop launchers) inherit a minimal `PATH` that omits entries a login
    /// shell would add (`/opt/homebrew/bin`, version-manager shims, …), so tools
    /// the app shells out to cannot be found. This policy runs the login shell
    /// once before the platform exists and copies its environment into the
    /// current process — the established desktop-app fix (used by Tauri and
    /// others).
    ///
    /// # Soundness precondition (the caller's obligation)
    ///
    /// The repair mutates the process environment through `std::env::set_var`,
    /// which is **Undefined Behavior on Unix if any other thread may read or
    /// write the environment concurrently**. The shell cannot prove the caller
    /// has not already spawned threads, so the obligation is yours: **select
    /// `LoginShell` only from a single-threaded `main()`, before any thread is
    /// spawned** (the typical first statements of `main`).
    ///
    /// This is the same objection that keeps a general `Custom(vars)` variant out
    /// of this enum (see the note below it). It is accepted here only because the
    /// repair is a single, vetted, widely-used operation rather than an
    /// open-ended env hook — the alternative is every app hand-rolling the same
    /// unsafe mutation.
    ///
    /// Failure to read the login shell is **non-fatal**: it is logged and the
    /// process keeps the inherited environment (behaves as
    /// [`EnvironmentPolicy::Inherit`]); it never aborts startup. On Windows there
    /// is no login-shell `PATH` to repair, so this is a documented no-op.
    LoginShell,
}

// There is deliberately no `Custom(vars)` variant: `std::env::set_var` is
// unsafe (UB with concurrent environment access on Unix), and the shell cannot
// know whether the caller already spawned threads. The one env mutation the
// shell performs — `LoginShell` above — is a single vetted repair carrying an
// explicit caller precondition, not an open-ended "set these vars" hook. Apps
// that need arbitrary environment changes must do so in their own `main()`
// before declaring the application, where the safety obligation is visibly
// theirs.

/// Process-global logging policy (plan §3). The library must not seize the
/// process logger by default.
#[non_exhaustive]
pub enum LoggingPolicy {
    /// The application (or its harness) owns logging. Default.
    External,
    /// Run an app-provided initializer with resolved paths, before the platform.
    ///
    /// The initializer is a non-capturing `fn` pointer: logging setup is a
    /// process-global side effect, so it must not smuggle in application state.
    /// A failure aborts startup as [`AppShellError::Preparation`] rather than
    /// being logged and swallowed — the logger the message would go to is
    /// exactly what failed to initialize.
    Configure(fn(&AppPaths) -> anyhow::Result<()>),
}

/// Selects the GPUI platform backend. Injected for testing; never public.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PlatformRunner {
    kind: RunnerKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RunnerKind {
    Native,
    #[cfg(feature = "test-support")]
    Headless,
    #[cfg(test)]
    Failing,
}

impl PlatformRunner {
    /// The real platform for the current OS.
    pub(crate) fn native() -> Self {
        Self {
            kind: RunnerKind::Native,
        }
    }

    /// A headless platform, for bootstrap/lifecycle tests.
    #[cfg(feature = "test-support")]
    pub(crate) fn headless() -> Self {
        Self {
            kind: RunnerKind::Headless,
        }
    }

    #[cfg(test)]
    pub(crate) fn failing() -> Self {
        Self {
            kind: RunnerKind::Failing,
        }
    }

    fn build(self) -> Result<Application, AppShellError> {
        match self.kind {
            RunnerKind::Native => gpui_platform::try_application().map_err(AppShellError::Platform),
            #[cfg(feature = "test-support")]
            RunnerKind::Headless => Ok(gpui_platform::test_application()),
            #[cfg(test)]
            RunnerKind::Failing => Err(AppShellError::Platform(anyhow::anyhow!(
                "test platform construction failure"
            ))),
        }
    }
}

impl Default for PlatformRunner {
    fn default() -> Self {
        Self::native()
    }
}

/// The application's entry point.
pub struct AppShell;

impl AppShell {
    /// Run the application declared by `A`.
    ///
    /// # Errors
    ///
    /// Returns [`AppShellError::Declaration`] for a malformed declaration,
    /// [`AppShellError::Launch`] for a launch-parse failure, and whatever the
    /// startup path reports thereafter.
    pub fn run<A: crate::declaration::DesktopApp>() -> Result<(), AppShellError> {
        crate::declaration::run::run::<A>()
    }
}

/// One validated declaration, finalized into everything startup needs.
///
/// Private and inert: it holds resolved values, not intent, and
/// [`AppDeclaration::lower`](crate::declaration::AppDeclaration) is its only
/// constructor. There is no generic module-injection seam on it — an
/// application contributes runtime modules only through the declaration
/// vocabulary that owns them.
pub(crate) struct RuntimePlan {
    pub(crate) identity: IdentityRef,
    /// Asset sources in resolution order; the first to resolve a path wins.
    pub(crate) assets: Vec<Arc<dyn AssetSource>>,
    pub(crate) path_layout: PathLayout,
    pub(crate) environment: EnvironmentPolicy,
    pub(crate) logging: LoggingPolicy,
    pub(crate) initial_activation: InitialActivation,
    pub(crate) exit_policy: ExitPolicy,
    pub(crate) prepare: Option<PrepareHook>,
    pub(crate) configure_application: Option<ConfigureApplication>,
    /// Runtime modules in init order; shutdown runs them in reverse.
    pub(crate) modules: RuntimeModules,
    /// Declared lifecycle observers, delivered after every module.
    pub(crate) observers: Vec<EventHandler>,
    pub(crate) start: Option<StartCallback>,
    pub(crate) error_reporter: Option<ErrorReporter>,
    pub(crate) app_shutdown: Option<AppShutdownHook>,
    /// The prepared typed launch: `before_primary` and the primary opener.
    pub(crate) launch: LaunchRuntime,
    pub(crate) runner: PlatformRunner,
}

impl RuntimePlan {
    /// Execute the plan: process policies, the platform, then the GPUI loop.
    pub(crate) fn run(self) -> Result<(), AppShellError> {
        let Self {
            identity,
            assets,
            path_layout,
            environment,
            logging,
            initial_activation,
            exit_policy,
            prepare,
            configure_application,
            mut modules,
            observers,
            start,
            error_reporter,
            app_shutdown,
            launch,
            runner,
        } = self;

        // ---- Before the platform (no GPUI) ----
        validate_identity(&identity)?;
        let paths =
            AppPaths::new(identity.data_namespace, path_layout).map_err(AppShellError::Paths)?;
        apply_environment(environment);
        apply_logging(logging, &paths)?;
        let capabilities = PlatformCapabilities::detect();
        let app_info = AppInfo::new(identity, paths, capabilities);

        // The declared preparation hook sees resolved identity/paths/
        // capabilities; module `prepare` below observes any process state it
        // established.
        if let Some(prepare) = prepare {
            prepare(&app_info)?;
        }
        for module in &mut modules {
            module.prepare(&app_info);
        }

        // ---- The GPUI application ----
        let mut application = runner.build()?.with_assets(ChainedAssets::new(assets));
        if let Some(configure) = configure_application {
            application = configure(application)?;
        }
        // Quit mode is shell-owned and not customizable: liveness is the single
        // quit authority, so every quit routes through `request_quit`. Applied
        // AFTER `configure_application` so the app callback cannot re-enable
        // platform auto-quit and bypass the lifecycle/teardown path.
        application = application.with_quit_mode(QuitMode::Explicit);

        // ---- Early platform listeners ----
        let pending = Arc::new(PendingEvents::default());
        {
            let pending = Arc::clone(&pending);
            application.on_open_urls(move |urls| {
                let _ = pending.push(handles::open_event(urls));
            });
        }
        {
            // Route early reopens through the same queue-until-ready buffer as
            // URLs; delivering directly here would be a no-op before the shell
            // global is installed, silently losing a startup-time reopen.
            let pending = Arc::clone(&pending);
            application.on_reopen(move |_cx| {
                let _ = pending.push(AppEvent::Reopened);
            });
        }

        // ---- The main-thread startup sequence ----
        let error_cell: Arc<Mutex<Option<AppShellError>>> = Arc::new(Mutex::new(None));
        let liveness = Liveness::new(exit_policy, initial_activation);
        // Retained for the running shell's lifetime (issues #3/#6/#29):
        // `Startup` uses it for the initial primary open, and a clone is
        // stashed on `ShellState` (see `handles::set_launch_runtime`) so a
        // later `Reopened` can restore the primary from the same immutable
        // typed launch value, without ever publishing it through a public
        // global.
        let startup = Startup {
            app_info,
            liveness,
            initial_activation,
            modules,
            observers,
            pending,
            launch: Rc::new(launch),
            start,
            app_shutdown,
            error_reporter: error_reporter.unwrap_or_else(|| {
                Box::new(|error, _cx| {
                    log::error!("{error}");
                })
            }),
        };
        let error_slot = Arc::clone(&error_cell);
        application.run(move |cx| {
            if let Err(err) = startup.run(cx) {
                // Startup already completed fatal teardown. Retain its error
                // until the application loop returns, then surface it to the
                // caller.
                log::error!("app shell startup failed: {err}");
                *error_slot.lock().expect("error cell poisoned") = Some(err);
                cx.quit();
            }
        });

        match error_cell.lock().expect("error cell poisoned").take() {
            Some(err) => Err(err),
            None => Ok(()),
        }
    }
}

/// Everything moved into the run closure to execute the post-platform startup
/// sequence on the main thread.
struct Startup {
    app_info: AppInfo,
    liveness: Liveness,
    initial_activation: InitialActivation,
    modules: RuntimeModules,
    observers: Vec<EventHandler>,
    pending: Arc<PendingEvents>,
    launch: Rc<LaunchRuntime>,
    start: Option<StartCallback>,
    app_shutdown: Option<AppShutdownHook>,
    error_reporter: ErrorReporter,
}

impl Startup {
    fn run(self, cx: &mut App) -> Result<(), AppShellError> {
        let Self {
            app_info,
            liveness,
            initial_activation,
            mut modules,
            observers,
            pending,
            launch,
            start,
            app_shutdown,
            error_reporter,
        } = self;

        // Component library globals, exactly once.
        neutron_components::init(cx);

        // Core services: install the global (moves modules/observers in) and
        // start the cross-thread drain loop.
        let proxy = handles::install(
            cx,
            app_info.clone(),
            liveness,
            std::mem::take(&mut modules),
            observers,
            Arc::clone(&pending),
            error_reporter,
            app_shutdown,
        );
        // Retain the launch runtime on the shell global before any observer
        // can run, so a `Reopened` delivered from this point on can already
        // restore the primary (issues #3/#6/#29).
        handles::set_launch_runtime(cx, Rc::clone(&launch));

        // Install lifecycle observers (incl. the `on_app_quit` teardown hook)
        // immediately, BEFORE any application-controlled handler (module init,
        // `Started`/`start`) can run. Otherwise a `request_quit()` from a
        // `start` handler would terminate before the quit observer exists,
        // skipping WillExit, reverse module shutdown, proxy close, and flush.
        handles::register_observers(cx);

        // Initialize the runtime modules in declaration order. A failure here
        // is fatal (required service); it aborts startup, unwinding the
        // already-initialized prefix in reverse (the documented shutdown
        // contract).
        let mut installed = cx.global_mut::<crate::handles::ShellState>().take_modules();
        let mut initialized = 0usize;
        let mut init_error = None;
        for module in &mut installed {
            match module.init(cx, &app_info, &proxy) {
                Ok(()) => initialized += 1,
                Err(err) => {
                    init_error = Some(err);
                    break;
                }
            }
        }
        if let Some(err) = init_error {
            // The failing module never completed init and is not shut down; the
            // successfully-initialized prefix is torn down in reverse order.
            installed.truncate(initialized);
            cx.global_mut::<crate::handles::ShellState>()
                .restore_modules(installed);
            handles::fail_startup(cx);
            return Err(err);
        }
        cx.global_mut::<crate::handles::ShellState>()
            .restore_modules(installed);

        // The one fatal application-owned composition transaction. The
        // transaction is entered before the hook runs — and even when none is
        // declared — so every teardown from here on runs the application
        // shutdown hook, including a failure inside `start` itself.
        handles::enter_app_start(cx);
        if let Some(start) = start
            && let Err(source) = start(cx)
        {
            handles::fail_startup(cx);
            return Err(AppShellError::Startup(source));
        }

        // The typed launch hook and the primary surface, between the common
        // start hook and readiness. Both are skipped when a quit was already
        // deferred by module `init` or by `start` — asked through the
        // read-only `deferred_quit`, because `finish_start` would publish
        // readiness before any surface exists.
        if handles::deferred_quit(cx).is_none() {
            launch
                .before_primary(cx)
                .inspect_err(|_| handles::fail_startup(cx))?;
            // The launch hook itself may have requested quit.
            if handles::deferred_quit(cx).is_none() {
                launch
                    .open_primary(cx)
                    .map_err(AppShellError::Startup)
                    .inspect_err(|_| handles::fail_startup(cx))?;
            }
        }

        // A quit requested during module init/start is deferred so a later
        // startup error wins. Successful startup now performs normal shutdown,
        // without publishing readiness events or activating.
        if let Some(reason) = handles::finish_start(cx) {
            pending.close();
            handles::request_quit_with(cx, reason);
            return Ok(());
        }

        // Deliver Started, enable post-ready delivery, drain the buffer.
        handles::deliver_event(cx, &AppEvent::Started);
        if cx
            .global::<crate::handles::ShellState>()
            .is_shutdown_requested()
        {
            // A Started handler may request quit synchronously. Do not publish
            // queued launch events or activate after that shutdown boundary.
            pending.close();
            return Ok(());
        }
        let _ = pending.proxy.set(proxy);
        if handles::drain_pending(cx) == handles::PendingDrain::ShutdownRequested
            || cx
                .global::<crate::handles::ShellState>()
                .is_shutdown_requested()
        {
            return Ok(());
        }

        // Activation.
        if cx
            .global::<crate::handles::ShellState>()
            .is_shutdown_requested()
        {
            return Ok(());
        }
        if let Some(force) = activation_force(initial_activation) {
            cx.activate(force);
        }
        if cx
            .global::<crate::handles::ShellState>()
            .is_shutdown_requested()
        {
            return Ok(());
        }

        // Startup has reached its stable state. Evaluate exit once so an app
        // that launched genuinely idle (no windows opened during `Started`, no
        // holds) exits under `ExitPolicy::WhenIdle` instead of living forever
        // under the shell-owned `QuitMode::Explicit`. Otherwise nothing would
        // ever trigger evaluation (which only fires on window-close/hold-drop).
        // No window closed here, so no pending reason is set → attributed to
        // `Requested`.
        if !cx
            .global::<crate::handles::ShellState>()
            .is_shutdown_requested()
        {
            handles::evaluate_exit(cx);
        }
        Ok(())
    }
}

fn activation_force(activation: InitialActivation) -> Option<bool> {
    match activation {
        InitialActivation::Regular => Some(false),
        InitialActivation::Forced => Some(true),
        InitialActivation::Passive => None,
    }
}

fn validate_identity(identity: &IdentityRef) -> Result<(), AppShellError> {
    if identity.app_id.is_empty() {
        return Err(AppShellError::Preparation(anyhow::anyhow!(
            "app_id is empty"
        )));
    }
    if identity.data_namespace.is_empty() {
        return Err(AppShellError::Preparation(anyhow::anyhow!(
            "data_namespace is empty"
        )));
    }
    Ok(())
}

fn apply_environment(policy: EnvironmentPolicy) {
    match policy {
        EnvironmentPolicy::Inherit => {}
        EnvironmentPolicy::LoginShell => apply_login_shell(),
    }
}

/// Repair `PATH` from the login shell. Soundness rests on the caller precondition
/// documented on [`EnvironmentPolicy::LoginShell`] (no other threads yet).
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn apply_login_shell() {
    // `fix_path_env::fix()` copies the login shell's environment into this
    // process via `std::env::set_var`; the caller precondition (single-threaded
    // `main`) is what makes that sound. A failed repair is non-fatal — keep the
    // inherited environment rather than aborting startup.
    if let Err(err) = fix_path_env::fix() {
        log::warn!(
            "EnvironmentPolicy::LoginShell: could not repair PATH from the login \
             shell; keeping the inherited environment: {err}"
        );
    }
}

/// No login-shell `PATH` to repair off Unix; documented no-op (see the variant).
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn apply_login_shell() {
    log::debug!(
        "EnvironmentPolicy::LoginShell is a no-op on this platform; keeping the \
         inherited environment"
    );
}

fn apply_logging(policy: LoggingPolicy, paths: &AppPaths) -> Result<(), AppShellError> {
    match policy {
        LoggingPolicy::External => Ok(()),
        LoggingPolicy::Configure(init) => init(paths).map_err(AppShellError::Preparation),
    }
}

/// Composes asset sources with first-match-wins load and unioned listings.
struct ChainedAssets {
    sources: Vec<Arc<dyn AssetSource>>,
}

impl ChainedAssets {
    fn new(sources: Vec<Arc<dyn AssetSource>>) -> Self {
        Self { sources }
    }
}

impl AssetSource for ChainedAssets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        let mut first_error = None;
        for source in &self.sources {
            match source.load(path) {
                Ok(Some(bytes)) => return Ok(Some(bytes)),
                Ok(None) => {}
                Err(error) => {
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                }
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(None),
        }
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
        let mut out = Vec::new();
        for source in &self.sources {
            out.extend(source.list(path)?);
        }
        out.sort();
        out.dedup();
        Ok(out)
    }
}

#[cfg(test)]
impl RuntimePlan {
    /// Load `path` through the lowered asset sources, honoring first-match-wins.
    ///
    /// Lets the declaration tests assert asset precedence without a platform.
    pub(crate) fn load_asset(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        ChainedAssets::new(self.assets.clone()).load(path)
    }

    /// The concrete type names of the lowered runtime modules, in init order.
    pub(crate) fn module_names(&self) -> Vec<&'static str> {
        self.modules
            .iter()
            .map(|module| module.type_name())
            .collect()
    }

    /// The environment policy as a stable test label.
    pub(crate) fn environment_name(&self) -> &'static str {
        match self.environment {
            EnvironmentPolicy::Inherit => "inherit",
            EnvironmentPolicy::LoginShell => "login-shell",
        }
    }

    /// The logging policy as a stable test label.
    pub(crate) fn logging_name(&self) -> &'static str {
        match self.logging {
            LoggingPolicy::External => "external",
            LoggingPolicy::Configure(_) => "configure",
        }
    }
}

#[cfg(test)]
#[path = "shell_pending_tests.rs"]
mod pending_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::module::RuntimeModule;

    enum AssetOutcome {
        Bytes(&'static [u8]),
        Missing,
        Error(&'static str),
    }

    struct TestAssets(AssetOutcome);

    impl AssetSource for TestAssets {
        fn load(&self, _path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
            match self.0 {
                AssetOutcome::Bytes(bytes) => Ok(Some(Cow::Borrowed(bytes))),
                AssetOutcome::Missing => Ok(None),
                AssetOutcome::Error(message) => Err(anyhow::anyhow!(message)),
            }
        }

        fn list(&self, _path: &str) -> gpui::Result<Vec<SharedString>> {
            Ok(Vec::new())
        }
    }

    fn identity() -> IdentityRef {
        IdentityRef {
            app_id: "com.example.envtest",
            display_name: "Env Test",
            data_namespace: "envtest",
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

    struct RecordingModule {
        name: &'static str,
        fail_init: bool,
        log: Arc<Mutex<Vec<String>>>,
    }

    impl RuntimeModule for RecordingModule {
        fn id(&self) -> &'static str {
            self.name
        }

        fn prepare(&mut self, _info: &AppInfo) {
            self.log
                .lock()
                .expect("recording module log poisoned")
                .push(format!("{}:prepare", self.name));
        }

        fn init(
            &mut self,
            _cx: &mut App,
            _info: &AppInfo,
            _proxy: &crate::handles::AppProxy,
        ) -> Result<(), AppShellError> {
            self.log
                .lock()
                .expect("recording module log poisoned")
                .push(format!("{}:init", self.name));
            if self.fail_init {
                return Err(AppShellError::Module {
                    module: self.name,
                    source: anyhow::anyhow!("{} failed", self.name),
                });
            }
            Ok(())
        }

        fn on_event(&mut self, event: &AppEvent, _cx: &mut App) -> Result<(), AppShellError> {
            self.log
                .lock()
                .expect("recording module log poisoned")
                .push(format!("{}:{}", self.name, event.name()));
            Ok(())
        }

        fn shutdown(&mut self, _cx: &mut App) {
            self.log
                .lock()
                .expect("recording module log poisoned")
                .push(format!("{}:shutdown", self.name));
        }
    }

    fn recording_module(
        name: &'static str,
        fail_init: bool,
        log: Arc<Mutex<Vec<String>>>,
    ) -> Box<dyn RuntimeModule> {
        Box::new(RecordingModule {
            name,
            fail_init,
            log,
        })
    }

    fn recording_observer(log: Arc<Mutex<Vec<String>>>, quit_on_started: bool) -> EventHandler {
        Box::new(move |event, cx| {
            log.lock()
                .expect("recording observer log poisoned")
                .push(format!("observer:{}", event.name()));
            if quit_on_started && matches!(event, AppEvent::Started) {
                handles::request_quit(cx);
            }
            Ok(())
        })
    }

    fn test_startup(
        modules: RuntimeModules,
        pending: Arc<PendingEvents>,
        start: Option<StartCallback>,
        log: Arc<Mutex<Vec<String>>>,
    ) -> Startup {
        Startup {
            app_info: AppInfo::new(
                identity(),
                AppPaths::new("appshell-startup-tests", PathLayout::PlatformDefault)
                    .expect("test paths resolve"),
                PlatformCapabilities::detect(),
            ),
            liveness: Liveness::new(ExitPolicy::Explicit, InitialActivation::Passive),
            initial_activation: InitialActivation::Passive,
            modules,
            observers: vec![recording_observer(log, false)],
            pending,
            launch: Rc::new(LaunchRuntime::unit(None)),
            start,
            error_reporter: Box::new(|_, _| {}),
            app_shutdown: None,
        }
    }

    fn assert_shutdown_rejects_proxy(cx: &mut gpui::TestAppContext) {
        use crate::error::AppClosed;
        use crate::runtime::Shell;

        cx.update(|app| {
            let proxy = app.app_proxy();
            assert!(proxy.is_closed());
            assert_eq!(proxy.dispatch(|_| {}), Err(AppClosed));
        });
    }

    #[gpui::test]
    fn module_init_failure_unwinds_initialized_prefix_without_readiness(
        cx: &mut gpui::TestAppContext,
    ) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let pending = Arc::new(PendingEvents::default());
        pending
            .push(AppEvent::Reopened)
            .expect("pre-ready event is accepted");
        let startup = test_startup(
            vec![
                recording_module("first", false, Arc::clone(&log)),
                recording_module("second", false, Arc::clone(&log)),
                recording_module("broken", true, Arc::clone(&log)),
            ],
            Arc::clone(&pending),
            None,
            Arc::clone(&log),
        );

        let result = cx.update(|app| startup.run(app));

        assert!(matches!(
            result,
            Err(AppShellError::Module {
                module: "broken",
                ..
            })
        ));
        assert_eq!(
            *log.lock().expect("recording module log poisoned"),
            vec![
                "first:init",
                "second:init",
                "broken:init",
                "first:shutdown_requested",
                "second:shutdown_requested",
                "observer:shutdown_requested",
                "first:will_exit",
                "second:will_exit",
                "observer:will_exit",
                "second:shutdown",
                "first:shutdown",
            ]
        );
        assert!(
            pending.is_empty(),
            "fatal startup clears queued launch events"
        );
        assert_shutdown_rejects_proxy(cx);
    }

    #[gpui::test]
    fn start_failure_unwinds_modules_without_publishing_readiness(cx: &mut gpui::TestAppContext) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let pending = Arc::new(PendingEvents::default());
        pending
            .push(AppEvent::Reopened)
            .expect("pre-ready event is accepted");
        let start_log = Arc::clone(&log);
        let startup = test_startup(
            vec![
                recording_module("first", false, Arc::clone(&log)),
                recording_module("second", false, Arc::clone(&log)),
            ],
            Arc::clone(&pending),
            Some(Box::new(move |_| {
                start_log
                    .lock()
                    .expect("recording module log poisoned")
                    .push("start".to_string());
                Err(anyhow::anyhow!("start failed"))
            })),
            Arc::clone(&log),
        );

        let result = cx.update(|app| startup.run(app));

        assert!(matches!(result, Err(AppShellError::Startup(_))));
        assert_eq!(
            *log.lock().expect("recording module log poisoned"),
            vec![
                "first:init",
                "second:init",
                "start",
                "first:shutdown_requested",
                "second:shutdown_requested",
                "observer:shutdown_requested",
                "first:will_exit",
                "second:will_exit",
                "observer:will_exit",
                "second:shutdown",
                "first:shutdown",
            ]
        );
        assert!(
            pending.is_empty(),
            "fatal startup clears queued launch events"
        );
        assert_shutdown_rejects_proxy(cx);
    }

    #[gpui::test]
    fn successful_quit_during_start_returns_ok_without_readiness(cx: &mut gpui::TestAppContext) {
        use crate::runtime::Shell;

        let log = Arc::new(Mutex::new(Vec::new()));
        let pending = Arc::new(PendingEvents::default());
        pending
            .push(AppEvent::Reopened)
            .expect("pre-ready event is accepted");
        let startup = test_startup(
            vec![recording_module("first", false, Arc::clone(&log))],
            Arc::clone(&pending),
            Some(Box::new(|cx| {
                cx.request_quit();
                Ok(())
            })),
            Arc::clone(&log),
        );

        let result = cx.update(|app| startup.run(app));

        assert!(result.is_ok(), "a successful startup quit is not fatal");
        assert!(
            pending.is_empty(),
            "a startup quit clears queued launch events"
        );
        let events = log.lock().expect("recording module log poisoned");
        assert!(
            !events
                .iter()
                .any(|event| event.ends_with(":started") || event.ends_with(":reopened")),
            "a startup quit does not publish readiness or drain queued events: {events:?}"
        );
        drop(events);
        assert_shutdown_rejects_proxy(cx);
    }

    #[gpui::test]
    fn started_quit_discards_pending_events_and_skips_activation(cx: &mut gpui::TestAppContext) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let pending = Arc::new(PendingEvents::default());
        pending
            .push(AppEvent::Reopened)
            .expect("pre-ready event is accepted");
        let mut startup = test_startup(
            vec![recording_module("first", false, Arc::clone(&log))],
            Arc::clone(&pending),
            None,
            Arc::clone(&log),
        );
        startup.observers = vec![recording_observer(Arc::clone(&log), true)];

        let result = cx.update(|app| startup.run(app));

        assert!(result.is_ok(), "Started-triggered quit is not fatal");
        let events = log.lock().expect("recording module log poisoned");
        assert!(events.iter().any(|event| event == "first:started"));
        assert!(
            events
                .iter()
                .any(|event| event == "observer:shutdown_requested"),
            "Started-triggered quit must enter the normal shutdown path: {events:?}"
        );
        assert!(
            !events.iter().any(|event| event.ends_with(":reopened")),
            "queued events must not drain after Started requests quit: {events:?}"
        );
        drop(events);
        assert!(
            pending.is_empty(),
            "Started-triggered quit clears queued launch events"
        );
        assert!(
            pending.proxy.get().is_none(),
            "shutdown must not enable post-ready event delivery"
        );
        assert_shutdown_rejects_proxy(cx);
    }

    // ---------------------------------------------------------------- declared
    // The whole declared sequence: runtime modules, common start, deferred-quit
    // check, `before_primary`, deferred-quit check, typed primary open,
    // `finish_start`, `Started`, drain, activation.

    /// A startup on the declared path, recording every observable step in `log`.
    fn declared_startup(
        log: &Arc<Mutex<Vec<String>>>,
        launch: LaunchRuntime,
        quit_during_start: bool,
    ) -> Startup {
        let start_log = Arc::clone(log);
        let mut startup = test_startup(
            vec![recording_module("setup", false, Arc::clone(log))],
            Arc::new(PendingEvents::default()),
            Some(Box::new(move |cx| {
                start_log
                    .lock()
                    .expect("declared startup log poisoned")
                    .push("start".to_string());
                if quit_during_start {
                    handles::request_quit(cx);
                }
                Ok(())
            })),
            Arc::clone(log),
        );
        startup.launch = Rc::new(launch);
        startup
    }

    /// The launch runtime a declaration with `spec` prepares, with no primary.
    fn launch_runtime(spec: crate::declaration::LaunchSpec<()>) -> LaunchRuntime {
        let prepared = crate::declaration::AppDeclaration::new(identity())
            .launch(spec)
            .prepare_launch(&crate::declaration::ProcessLaunch::empty())
            .expect("the test parser succeeds");
        match prepared {
            crate::declaration::PreparedLaunch::Run(runtime) => runtime,
            crate::declaration::PreparedLaunch::ExitSuccess { .. } => {
                panic!("the test parser always runs")
            }
        }
    }

    thread_local! {
        /// Declared hooks are non-capturing `fn` pointers, and every test runs
        /// on its own thread, so a thread-local recorder is isolated per test.
        static DECLARED_STEPS: Mutex<Vec<String>> = const { Mutex::new(Vec::new()) };
    }

    fn record_declared(step: &str) {
        DECLARED_STEPS.with(|steps| {
            steps
                .lock()
                .expect("declared step recorder poisoned")
                .push(step.to_string());
        });
    }

    fn declared_steps() -> Vec<String> {
        DECLARED_STEPS.with(|steps| {
            steps
                .lock()
                .expect("declared step recorder poisoned")
                .clone()
        })
    }

    fn parse_unit(
        _process: &crate::declaration::ProcessLaunch,
    ) -> anyhow::Result<crate::declaration::LaunchDecision<()>> {
        Ok(crate::declaration::LaunchDecision::Run(()))
    }

    fn recording_before_primary(_value: &(), _cx: &mut App) -> anyhow::Result<()> {
        record_declared("before_primary");
        Ok(())
    }

    /// A surface that is never installed, so opening it is a typed
    /// `UndeclaredSurface` fault: the probe for "the primary open was reached".
    fn uninstalled_primary() -> crate::declaration::LaunchSpec<()> {
        crate::declaration::LaunchSpec::new(parse_unit)
            .before_primary(recording_before_primary)
            .primary_surface(crate::declaration::Surface::new(
                crate::declaration::SurfaceKey::<ProbeView, ()>::primary(),
                |_args: &(), _window: &mut gpui::Window, cx: &mut App| {
                    use gpui::AppContext as _;
                    cx.new(|_| ProbeView)
                },
            ))
    }

    struct ProbeView;

    impl gpui::Render for ProbeView {
        fn render(
            &mut self,
            _window: &mut gpui::Window,
            _cx: &mut gpui::Context<Self>,
        ) -> impl gpui::IntoElement {
            gpui::div()
        }
    }

    #[gpui::test]
    fn the_declared_path_runs_start_then_before_primary_then_started(
        cx: &mut gpui::TestAppContext,
    ) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let launch = launch_runtime(
            crate::declaration::LaunchSpec::new(parse_unit).before_primary(|_value, cx| {
                record_declared("before_primary");
                // Installed, but readiness is published only once the primary
                // surface exists: `finish_start` must not have run yet.
                assert!(
                    !cx.global::<crate::handles::ShellState>().is_ready(),
                    "`finish_start` must not publish readiness before the primary opens",
                );
                Ok(())
            }),
        );
        let mut startup = declared_startup(&log, launch, false);
        startup.observers = vec![recording_observer(Arc::clone(&log), false)];

        cx.update(|app| startup.run(app))
            .expect("the declared startup succeeds");

        let events = log.lock().expect("declared startup log poisoned");
        let order: Vec<&str> = events.iter().map(String::as_str).collect();
        let start = order.iter().position(|step| *step == "start");
        let started = order.iter().position(|step| *step == "setup:started");
        assert_eq!(order.first(), Some(&"setup:init"), "{order:?}");
        assert!(start < started, "start precedes Started: {order:?}");
        assert_eq!(declared_steps(), vec!["before_primary"]);
    }

    #[gpui::test]
    fn the_typed_primary_opens_after_before_primary_and_before_readiness(
        cx: &mut gpui::TestAppContext,
    ) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let startup = declared_startup(&log, launch_runtime(uninstalled_primary()), false);

        let error = cx
            .update(|app| startup.run(app))
            .expect_err("the primary surface is never installed");

        assert!(
            matches!(&error, AppShellError::Startup(source)
                if source.to_string().contains("primary")),
            "a failed primary open is a startup failure: {error:?}",
        );
        assert_eq!(
            declared_steps(),
            vec!["before_primary"],
            "the primary opens only after `before_primary`",
        );
        let events = log.lock().expect("declared startup log poisoned");
        assert!(
            !events.iter().any(|step| step.ends_with(":started")),
            "`finish_start` must not run before the primary opens: {events:?}",
        );
    }

    #[gpui::test]
    fn a_quit_deferred_during_start_suppresses_before_primary_and_the_primary(
        cx: &mut gpui::TestAppContext,
    ) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let startup = declared_startup(&log, launch_runtime(uninstalled_primary()), true);

        let result = cx.update(|app| startup.run(app));

        assert!(result.is_ok(), "a deferred quit is not a startup failure");
        assert!(
            declared_steps().is_empty(),
            "a deferred quit suppresses `before_primary` and the primary open",
        );
        let events = log.lock().expect("declared startup log poisoned");
        assert!(
            !events.iter().any(|step| step.ends_with(":started")),
            "a deferred quit never publishes readiness: {events:?}",
        );
    }

    #[gpui::test]
    fn a_quit_deferred_by_a_module_suppresses_before_primary_and_the_primary(
        cx: &mut gpui::TestAppContext,
    ) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut startup = declared_startup(&log, launch_runtime(uninstalled_primary()), false);
        startup.modules = vec![Box::new(QuittingModule)];

        let result = cx.update(|app| startup.run(app));

        assert!(result.is_ok(), "a deferred quit is not a startup failure");
        assert!(
            declared_steps().is_empty(),
            "a quit deferred before start still suppresses the launch steps",
        );
    }

    struct QuittingModule;

    impl RuntimeModule for QuittingModule {
        fn init(
            &mut self,
            cx: &mut App,
            _info: &AppInfo,
            _proxy: &crate::handles::AppProxy,
        ) -> Result<(), AppShellError> {
            handles::request_quit(cx);
            Ok(())
        }
    }

    #[gpui::test]
    fn a_failing_event_observer_never_stops_the_later_ones(cx: &mut gpui::TestAppContext) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut startup = declared_startup(&log, launch_runtime(uninstalled_primary()), true);
        let first = Arc::clone(&log);
        let second = Arc::clone(&log);
        // What the declared lifecycle hooks lower to: repeatable observers,
        // each nonfatal, delivered in declaration order.
        startup.observers = vec![
            Box::new(move |event, _cx| {
                first
                    .lock()
                    .expect("declared startup log poisoned")
                    .push(format!("first:{}", event.name()));
                anyhow::bail!("observer failed")
            }),
            Box::new(move |event, _cx| {
                second
                    .lock()
                    .expect("declared startup log poisoned")
                    .push(format!("second:{}", event.name()));
                Ok(())
            }),
        ];
        let reported = Arc::clone(&log);
        startup.error_reporter = Box::new(move |error, _cx| {
            assert_eq!(error.operation(), crate::error::RuntimeOperation::Lifecycle);
            assert_eq!(error.module_id(), None);
            reported
                .lock()
                .expect("declared startup log poisoned")
                .push(format!(
                    "error:{}:{}",
                    error
                        .event()
                        .map_or("unknown", crate::lifecycle::AppEvent::name),
                    error.source_error(),
                ));
        });

        cx.update(|app| startup.run(app))
            .expect("a deferred quit is not fatal");

        let events = log.lock().expect("declared startup log poisoned");
        assert!(
            events.iter().any(|step| step == "first:shutdown_requested")
                && events
                    .iter()
                    .any(|step| step == "second:shutdown_requested"),
            "a failing observer must not suppress the ones declared after it: {events:?}",
        );
        assert!(
            events
                .iter()
                .any(|step| step.starts_with("error:shutdown_requested:")),
            "the failure reaches the one runtime reporter: {events:?}",
        );
    }

    struct FailingEventModule;

    impl RuntimeModule for FailingEventModule {
        fn id(&self) -> &'static str {
            "event-probe"
        }

        fn init(
            &mut self,
            _cx: &mut App,
            _info: &AppInfo,
            _proxy: &crate::handles::AppProxy,
        ) -> Result<(), AppShellError> {
            Ok(())
        }

        fn on_event(&mut self, event: &AppEvent, _cx: &mut App) -> Result<(), AppShellError> {
            if matches!(event, AppEvent::Started) {
                return Err(AppShellError::Module {
                    module: "inner",
                    source: anyhow::anyhow!("event failed"),
                });
            }
            Ok(())
        }
    }

    #[gpui::test]
    fn a_runtime_module_event_failure_keeps_its_module_identity(cx: &mut gpui::TestAppContext) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut startup = test_startup(
            vec![Box::new(FailingEventModule)],
            Arc::new(PendingEvents::default()),
            None,
            Arc::clone(&log),
        );
        let reported = Arc::clone(&log);
        startup.error_reporter = Box::new(move |error, _cx| {
            reported
                .lock()
                .expect("declared startup log poisoned")
                .push(format!(
                    "{:?}:{}:{}",
                    error.operation(),
                    error.module_id().unwrap_or("missing"),
                    error.source_error()
                ));
        });
        startup.observers = vec![recording_observer(Arc::clone(&log), true)];

        cx.update(|app| startup.run(app))
            .expect("module event errors are nonfatal");

        assert!(
            log.lock()
                .expect("declared startup log poisoned")
                .iter()
                .any(|step| step == "Module:event-probe:module `inner` failed"),
            "the report retains the runtime module identity",
        );
    }

    #[gpui::test]
    fn the_declared_app_shutdown_runs_once_between_will_exit_and_module_teardown(
        cx: &mut gpui::TestAppContext,
    ) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut startup = declared_startup(&log, launch_runtime(uninstalled_primary()), false);
        let shutdown_log = Arc::clone(&log);
        startup.app_shutdown = Some(Box::new(move |_cx| {
            shutdown_log
                .lock()
                .expect("declared startup log poisoned")
                .push("app:shutdown".to_string());
            Ok(())
        }));

        cx.update(|app| startup.run(app))
            .expect_err("the primary surface is never installed");

        let events = log.lock().expect("declared startup log poisoned");
        let order: Vec<&str> = events.iter().map(String::as_str).collect();
        assert_eq!(
            order.iter().filter(|step| **step == "app:shutdown").count(),
            1,
            "the application shutdown hook runs exactly once: {order:?}",
        );
        let will_exit = order
            .iter()
            .position(|step| *step == "setup:will_exit")
            .expect("WillExit is delivered");
        let app_shutdown = order
            .iter()
            .position(|step| *step == "app:shutdown")
            .expect("the app shutdown hook runs");
        let module_shutdown = order
            .iter()
            .position(|step| *step == "setup:shutdown")
            .expect("modules tear down");
        assert!(
            will_exit < app_shutdown && app_shutdown < module_shutdown,
            "app shutdown runs after WillExit and before reverse module teardown: {order:?}",
        );
    }

    #[gpui::test]
    fn a_failing_app_shutdown_is_reported_and_teardown_still_completes(
        cx: &mut gpui::TestAppContext,
    ) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut startup = declared_startup(&log, launch_runtime(uninstalled_primary()), false);
        startup.app_shutdown = Some(Box::new(|_cx| anyhow::bail!("flush failed")));
        let reported = Arc::clone(&log);
        startup.error_reporter = Box::new(move |error, _cx| {
            reported
                .lock()
                .expect("declared startup log poisoned")
                .push(format!(
                    "error:{:?}:{}",
                    error.operation(),
                    error.source_error()
                ));
        });

        cx.update(|app| startup.run(app))
            .expect_err("the primary surface is never installed");

        let events = log.lock().expect("declared startup log poisoned");
        assert!(
            events
                .iter()
                .any(|step| step == "error:Shutdown:flush failed"),
            "a failing app shutdown is reported as a Shutdown runtime error: {events:?}",
        );
        assert!(
            events.iter().any(|step| step == "setup:shutdown"),
            "teardown continues after a failing app shutdown: {events:?}",
        );
    }

    // ------------------------------------------ app-shutdown transaction gate
    // The application shutdown hook is bound to the application startup
    // *transaction*, not to teardown in general. It runs for every teardown from
    // the common start phase onward — including when no start hook is
    // declared — and is skipped for framework, module, and setup failures
    // before it, which leave nothing application-owned to unwind.

    /// A declared startup whose application shutdown hook records `app:shutdown`.
    fn recording_app_shutdown(startup: &mut Startup, log: &Arc<Mutex<Vec<String>>>) {
        let shutdown_log = Arc::clone(log);
        startup.app_shutdown = Some(Box::new(move |_cx| {
            shutdown_log
                .lock()
                .expect("declared startup log poisoned")
                .push("app:shutdown".to_string());
            Ok(())
        }));
    }

    fn count_app_shutdown(log: &Arc<Mutex<Vec<String>>>) -> usize {
        log.lock()
            .expect("declared startup log poisoned")
            .iter()
            .filter(|step| step.as_str() == "app:shutdown")
            .count()
    }

    #[gpui::test]
    fn a_module_init_failure_before_start_skips_the_app_shutdown_hook(
        cx: &mut gpui::TestAppContext,
    ) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut startup = declared_startup(&log, launch_runtime(uninstalled_primary()), false);
        // A framework/setup module that fails its own init: the common start
        // phase is never reached.
        startup.modules = vec![recording_module("broken", true, Arc::clone(&log))];
        recording_app_shutdown(&mut startup, &log);

        cx.update(|app| startup.run(app))
            .expect_err("a module init failure is fatal");

        let events = log.lock().expect("declared startup log poisoned");
        assert!(
            events.iter().any(|step| step == "observer:will_exit"),
            "teardown still runs for a pre-start failure: {events:?}",
        );
        assert!(
            !events.iter().any(|step| step == "app:shutdown"),
            "the application never started, so it must not be torn down: {events:?}",
        );
    }

    #[gpui::test]
    fn a_failing_common_start_runs_the_app_shutdown_hook_once(cx: &mut gpui::TestAppContext) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut startup = declared_startup(&log, launch_runtime(uninstalled_primary()), false);
        let start_log = Arc::clone(&log);
        startup.start = Some(Box::new(move |_cx| {
            start_log
                .lock()
                .expect("declared startup log poisoned")
                .push("start".to_string());
            Err(anyhow::anyhow!("composition failed"))
        }));
        recording_app_shutdown(&mut startup, &log);

        cx.update(|app| startup.run(app))
            .expect_err("a failing start is fatal");

        assert_eq!(
            count_app_shutdown(&log),
            1,
            "the transaction was entered before `start` ran, so it unwinds once",
        );
        let events = log.lock().expect("declared startup log poisoned");
        let start = events
            .iter()
            .position(|step| step == "start")
            .expect("start ran");
        let app_shutdown = events
            .iter()
            .position(|step| step == "app:shutdown")
            .expect("the app shutdown hook ran");
        assert!(start < app_shutdown, "{events:?}");
    }

    #[gpui::test]
    fn a_failing_before_primary_runs_the_app_shutdown_hook_once(cx: &mut gpui::TestAppContext) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let launch = launch_runtime(
            crate::declaration::LaunchSpec::new(parse_unit).before_primary(|_value, _cx| {
                record_declared("before_primary");
                Err(anyhow::anyhow!("launch hook failed"))
            }),
        );
        let mut startup = declared_startup(&log, launch, false);
        recording_app_shutdown(&mut startup, &log);

        cx.update(|app| startup.run(app))
            .expect_err("a failing launch hook is fatal");

        assert_eq!(declared_steps(), vec!["before_primary"]);
        assert_eq!(count_app_shutdown(&log), 1);
    }

    #[gpui::test]
    fn a_failing_primary_open_runs_the_app_shutdown_hook_once(cx: &mut gpui::TestAppContext) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut startup = declared_startup(&log, launch_runtime(uninstalled_primary()), false);
        recording_app_shutdown(&mut startup, &log);

        cx.update(|app| startup.run(app))
            .expect_err("the primary surface is never installed");

        assert_eq!(count_app_shutdown(&log), 1);
    }

    #[gpui::test]
    fn a_normal_quit_after_a_successful_start_runs_the_app_shutdown_hook_once(
        cx: &mut gpui::TestAppContext,
    ) {
        let log = Arc::new(Mutex::new(Vec::new()));
        // No primary surface to open, so this startup succeeds all the way
        // through `Started`, drain, and activation.
        let launch = launch_runtime(crate::declaration::LaunchSpec::new(parse_unit));
        let mut startup = declared_startup(&log, launch, false);
        recording_app_shutdown(&mut startup, &log);

        cx.update(|app| startup.run(app))
            .expect("the declared startup succeeds");
        assert_eq!(
            count_app_shutdown(&log),
            0,
            "a successful start does not tear the application down",
        );
        // `Startup::run` is driven directly in this unit seam, outside
        // `Platform::run`; ending the test app invokes the registered
        // platform-quit observer, which is the normal quit path.
        cx.quit();

        assert_eq!(count_app_shutdown(&log), 1);
        let events = log.lock().expect("declared startup log poisoned");
        let will_exit = events
            .iter()
            .position(|step| step == "setup:will_exit")
            .expect("WillExit is delivered");
        let app_shutdown = events
            .iter()
            .position(|step| step == "app:shutdown")
            .expect("the app shutdown hook ran");
        let module_shutdown = events
            .iter()
            .position(|step| step == "setup:shutdown")
            .expect("modules tear down");
        assert!(
            will_exit < app_shutdown && app_shutdown < module_shutdown,
            "a normal quit keeps the documented teardown order: {events:?}",
        );
    }

    #[gpui::test]
    fn entering_the_start_transaction_does_not_require_a_start_hook(cx: &mut gpui::TestAppContext) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut startup = declared_startup(&log, launch_runtime(uninstalled_primary()), false);
        // A declaration with no common start hook still owns everything the
        // launch runtime builds after it.
        startup.start = None;
        recording_app_shutdown(&mut startup, &log);

        cx.update(|app| startup.run(app))
            .expect_err("the primary surface is never installed");

        assert_eq!(
            count_app_shutdown(&log),
            1,
            "the transaction is entered even with no start hook",
        );
    }

    #[test]
    fn inherit_environment_is_a_noop() {
        // Inherit must never shell out or mutate the environment. Applying it is a
        // pure no-op; the LoginShell path is intentionally not exercised here (it
        // would spawn the login shell and mutate process env in CI).
        apply_environment(EnvironmentPolicy::Inherit);
    }

    /// Records the pre-platform sequence into the same thread-local recorder the
    /// declared `prepare` hook uses, so both land in one ordered log.
    struct PrePlatformProbe;

    impl RuntimeModule for PrePlatformProbe {
        fn prepare(&mut self, _info: &AppInfo) {
            record_declared("module:prepare");
        }

        fn init(
            &mut self,
            _cx: &mut App,
            _info: &AppInfo,
            _proxy: &crate::handles::AppProxy,
        ) -> Result<(), AppShellError> {
            record_declared("module:init");
            Ok(())
        }
    }

    /// The declaration side of [`PrePlatformProbe`]: the only way a runtime module
    /// reaches a plan is a declaration module contributing it.
    struct PrePlatformProbeDeclaration;

    impl crate::declaration::DeclarationModule for PrePlatformProbeDeclaration {
        fn key(&self) -> &'static str {
            "pre_platform.probe"
        }

        fn validate(&self, _errors: &mut Vec<crate::declaration::DeclarationError>) {}

        fn install(self: Box<Self>, modules: &mut RuntimeModules) {
            modules.push(Box::new(PrePlatformProbe));
        }
    }

    fn recording_prepare(_info: &AppInfo) -> anyhow::Result<()> {
        record_declared("advanced:prepare");
        Ok(())
    }

    #[test]
    fn the_plan_runs_the_declared_prepare_hook_then_module_prepare_before_the_platform() {
        let plan = crate::declaration::AppDeclaration::new(identity())
            .advanced(crate::declaration::AdvancedHooks::new().prepare(recording_prepare))
            .module(PrePlatformProbeDeclaration)
            .lower(LaunchRuntime::unit(None), PlatformRunner::failing());

        let error = plan
            .run()
            .expect_err("the failing runner never builds a platform");

        assert!(
            matches!(error, AppShellError::Platform(_)),
            "preflight completed and the platform is what failed: {error:?}",
        );
        assert_eq!(
            declared_steps(),
            vec!["advanced:prepare", "module:prepare"],
            "the application's own hook prepares the process before any module reads it, \
             and no module initializes without a platform",
        );
    }

    #[test]
    fn platform_construction_failure_returns_platform_error() {
        let plan = crate::declaration::AppDeclaration::new(identity())
            .lower(LaunchRuntime::unit(None), PlatformRunner::failing());

        let result = plan.run();

        assert!(matches!(
            result,
            Err(AppShellError::Platform(error)) if error.to_string() == "test platform construction failure"
        ));
    }

    #[test]
    fn activation_policy_maps_force_flag() {
        assert_eq!(activation_force(InitialActivation::Regular), Some(false));
        assert_eq!(activation_force(InitialActivation::Forced), Some(true));
        assert_eq!(activation_force(InitialActivation::Passive), None);
    }

    #[test]
    fn asset_chain_continues_after_error_to_later_hit() {
        let chain = ChainedAssets::new(vec![
            Arc::new(TestAssets(AssetOutcome::Error("first failed"))),
            Arc::new(TestAssets(AssetOutcome::Bytes(b"found"))),
        ]);
        assert_eq!(
            chain.load("asset").expect("later source should win"),
            Some(Cow::Borrowed(b"found".as_slice()))
        );
    }

    #[test]
    fn asset_chain_returns_first_error_when_no_source_hits() {
        let chain = ChainedAssets::new(vec![
            Arc::new(TestAssets(AssetOutcome::Error("first failed"))),
            Arc::new(TestAssets(AssetOutcome::Missing)),
            Arc::new(TestAssets(AssetOutcome::Error("second failed"))),
        ]);
        let error = chain.load("asset").expect_err("first error is retained");
        assert_eq!(error.to_string(), "first failed");
    }

    #[test]
    fn asset_chain_returns_none_when_every_source_misses_cleanly() {
        let chain = ChainedAssets::new(vec![
            Arc::new(TestAssets(AssetOutcome::Missing)),
            Arc::new(TestAssets(AssetOutcome::Missing)),
        ]);
        assert!(chain.load("asset").expect("clean misses").is_none());
    }
}
