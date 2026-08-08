//! The `AppShell` builder and phase-driven `run` (plan §3).
//!
//! Builder methods only *record intent*; [`AppShellBuilder::run`] executes the
//! fixed phase order from [`crate::phases`]. Work that must happen before the
//! GPUI event loop (path resolution, env/logging policy, `before_platform`,
//! plugin `configure`) runs in `Preflight`; everything else runs inside the run
//! closure with a raw `&mut gpui::App`.

use std::any::{Any, TypeId};
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use gpui::{App, Application, AssetSource, QuitMode, SharedString};
use gpui_component_manifest::schema::IdentityRef;
use gpui_component_storage::{AppPaths, PathLayout};

use crate::capabilities::PlatformCapabilities;
use crate::error::{
    AppShellError, BuilderConfigurationError, MenuConfiguration, RuntimeError, StartupHook,
};
use crate::handles::{self, AppInfo, PendingEvents};
use crate::lifecycle::{AppEvent, LaunchRequest};
use crate::liveness::{ExitPolicy, InitialActivation, Liveness};
use crate::phases::{Phase, PhaseTracker};
use crate::plugin::{AppPlugin, BuildContext, EventHandler, ShellSeed};

type StartCallback = Box<dyn FnOnce(&LaunchRequest, &mut App) -> anyhow::Result<()> + 'static>;
type ErrorReporter = Box<dyn Fn(&RuntimeError, &mut App) + 'static>;

/// Process-global environment policy (plan §3 — explicit, never a silent
/// default). Applied once in `Preflight`.
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
    /// once during `Preflight` and copies its environment into the current
    /// process — the established desktop-app fix (used by Tauri and others).
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
// unsafe (UB with concurrent environment access on Unix), and this builder
// cannot know whether the caller already spawned threads. The one env mutation
// the shell performs — `LoginShell` above — is a single vetted repair carrying an
// explicit caller precondition, not an open-ended "set these vars" hook. Apps
// that need arbitrary environment changes must do so in their own `main()` before
// constructing the shell, where the safety obligation is visibly theirs.

/// Process-global logging policy (plan §3). The library must not seize the
/// process logger by default.
#[non_exhaustive]
pub enum LoggingPolicy {
    /// The application (or its harness) owns logging. Default.
    External,
    /// Run an app-provided initializer with resolved paths during `Preflight`.
    Configure(Box<dyn FnOnce(&AppPaths)>),
}

/// Selects the GPUI platform backend. Injected for testing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlatformRunner {
    kind: RunnerKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RunnerKind {
    Native,
    Headless,
    #[cfg(test)]
    Failing,
}

impl PlatformRunner {
    /// The real platform for the current OS.
    pub fn native() -> Self {
        Self {
            kind: RunnerKind::Native,
        }
    }

    /// A headless platform, for bootstrap/lifecycle tests.
    pub fn headless() -> Self {
        Self {
            kind: RunnerKind::Headless,
        }
    }

    #[cfg(test)]
    fn failing() -> Self {
        Self {
            kind: RunnerKind::Failing,
        }
    }

    fn build(self) -> Result<Application, AppShellError> {
        match self.kind {
            RunnerKind::Native => gpui_platform::try_application().map_err(AppShellError::Platform),
            RunnerKind::Headless => gpui_platform::try_headless().map_err(AppShellError::Platform),
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

/// Entry point: `AppShell::builder(APP_IDENTITY)`.
pub struct AppShell;

impl AppShell {
    /// Start building an application shell from the compiled-in identity.
    ///
    /// `identity` is the `APP_IDENTITY` produced by `include_identity!()`.
    pub fn builder(identity: IdentityRef) -> AppShellBuilder {
        AppShellBuilder::new(identity)
    }
}

/// Accumulates configuration; [`AppShellBuilder::run`] executes it.
pub struct AppShellBuilder {
    identity: IdentityRef,
    assets: Vec<Arc<dyn AssetSource>>,
    path_layout: PathLayout,
    environment: EnvironmentPolicy,
    logging: LoggingPolicy,
    initial_activation: InitialActivation,
    exit_policy: ExitPolicy,
    configure_app: Option<Box<dyn FnOnce(Application) -> Result<Application, AppShellError>>>,
    before_platform: Vec<Box<dyn FnOnce() -> Result<(), AppShellError>>>,
    state: HashMap<TypeId, Box<dyn Any>>,
    plugins: Vec<Box<dyn AppPlugin>>,
    handlers: Vec<EventHandler>,
    start: Option<StartCallback>,
    startup_hook: Option<StartupHook>,
    configuration_error: Option<BuilderConfigurationError>,
    error_reporter: Option<ErrorReporter>,
    shell_preferences_installed: bool,
    menu_configuration: Option<MenuConfiguration>,
    runner: PlatformRunner,
}

impl AppShellBuilder {
    fn new(identity: IdentityRef) -> Self {
        Self {
            identity,
            assets: Vec::new(),
            path_layout: PathLayout::PlatformDefault,
            environment: EnvironmentPolicy::Inherit,
            logging: LoggingPolicy::External,
            initial_activation: InitialActivation::Regular,
            exit_policy: ExitPolicy::WhenIdle,
            configure_app: None,
            before_platform: Vec::new(),
            state: HashMap::new(),
            // Window management is the only always-present app service.
            // Shell preferences are installed only by consumers.
            plugins: vec![Box::new(crate::windows::WindowsPlugin::new())],
            handlers: Vec::new(),
            start: None,
            startup_hook: None,
            configuration_error: None,
            error_reporter: None,
            shell_preferences_installed: false,
            menu_configuration: None,
            runner: PlatformRunner::native(),
        }
    }

    /// Append an asset source. Sources are tried in registration order; the
    /// first to resolve a path wins (no silent last-writer-wins).
    pub fn assets(mut self, source: impl AssetSource) -> Self {
        self.assets.push(Arc::new(source));
        self
    }

    /// Choose the on-disk directory layout (default [`PathLayout::PlatformDefault`]).
    pub fn path_layout(mut self, layout: PathLayout) -> Self {
        self.path_layout = layout;
        self
    }

    /// Set the environment policy (default [`EnvironmentPolicy::Inherit`]).
    pub fn environment(mut self, policy: EnvironmentPolicy) -> Self {
        self.environment = policy;
        self
    }

    /// Set the logging policy (default [`LoggingPolicy::External`]).
    pub fn logging(mut self, policy: LoggingPolicy) -> Self {
        self.logging = policy;
        self
    }

    /// Set the initial-activation policy. Use [`InitialActivation::Passive`] for
    /// tray-first apps (default [`InitialActivation::Regular`]).
    pub fn initial_activation(mut self, activation: InitialActivation) -> Self {
        self.initial_activation = activation;
        self
    }

    /// Set the exit policy. Use [`ExitPolicy::Explicit`] for apps that outlive
    /// their windows (default [`ExitPolicy::WhenIdle`]).
    pub fn exit_policy(mut self, policy: ExitPolicy) -> Self {
        self.exit_policy = policy;
        self
    }

    /// Customize the GPUI [`Application`] before it runs (e.g. HTTP client).
    pub fn configure_application(
        mut self,
        f: impl FnOnce(Application) -> Result<Application, AppShellError> + 'static,
    ) -> Self {
        self.configure_app = Some(Box::new(f));
        self
    }

    /// Register a stateless side effect to run before the platform starts. Runs
    /// on the main thread with no GPUI and no windows.
    pub fn before_platform(
        mut self,
        f: impl FnOnce() -> Result<(), AppShellError> + 'static,
    ) -> Self {
        self.before_platform.push(Box::new(f));
        self
    }

    /// Register typed state prepared before the platform exists (e.g. audio
    /// bootstrap). Retrieve it later via `cx.app_state::<T>()`.
    pub fn state<T: 'static>(mut self, value: T) -> Self {
        self.state.insert(TypeId::of::<T>(), Box::new(value));
        self
    }

    /// Install an internal plugin. Order is preserved for init; shutdown runs in
    /// reverse.
    pub fn plugin(mut self, plugin: impl AppPlugin) -> Self {
        self.plugins.push(Box::new(plugin));
        self
    }

    pub(crate) fn ensure_shell_preferences(mut self) -> Self {
        if !self.shell_preferences_installed {
            self.plugins
                .push(Box::new(crate::settings::ShellPreferencesPlugin::new()));
            self.shell_preferences_installed = true;
        }
        self
    }

    pub(crate) fn register_menu_configuration(mut self, configuration: MenuConfiguration) -> Self {
        if let Some(first) = self.menu_configuration {
            self.configuration_error
                .get_or_insert(BuilderConfigurationError::DuplicateMenus {
                    first,
                    second: configuration,
                });
        } else {
            self.menu_configuration = Some(configuration);
        }
        self
    }

    /// Run fallible application composition after shell services initialize and
    /// before the app becomes ready.
    ///
    /// Only one `start`/`on_launch` callback may be registered. A duplicate is
    /// returned as [`AppShellError::Configuration`] from [`Self::run`] before
    /// platform construction.
    pub fn start(
        self,
        start: impl FnOnce(&LaunchRequest, &mut App) -> anyhow::Result<()> + 'static,
    ) -> Self {
        self.register_start(StartupHook::Start, Box::new(start))
    }

    fn register_start(mut self, hook: StartupHook, start: StartCallback) -> Self {
        if let Some(first) = self.startup_hook {
            self.configuration_error
                .get_or_insert(BuilderConfigurationError::DuplicateStartup {
                    first,
                    second: hook,
                });
        } else {
            self.startup_hook = Some(hook);
            self.start = Some(start);
        }
        self
    }

    /// Handle every lifecycle [`AppEvent`].
    pub fn on_event(
        mut self,
        mut handler: impl FnMut(&AppEvent, &mut App) -> Result<(), AppShellError> + 'static,
    ) -> Self {
        self.handlers
            .push(Box::new(move |event, cx| handler(event, cx)));
        self
    }

    /// Compatibility sugar for the transactional [`Self::start`] slot.
    ///
    /// Unlike the old `Started` event sugar, failures are fatal and the callback
    /// runs before `Started` observers, queued events, and activation.
    pub fn on_launch(
        self,
        mut f: impl FnMut(&mut App) -> Result<(), AppShellError> + 'static,
    ) -> Self {
        self.register_start(
            StartupHook::OnLaunch,
            Box::new(move |_launch, cx| f(cx).map_err(anyhow::Error::new)),
        )
    }

    /// Observe nonfatal runtime errors. The default reporter logs each error.
    pub fn on_error(mut self, reporter: impl Fn(&RuntimeError, &mut App) + 'static) -> Self {
        self.error_reporter = Some(Box::new(reporter));
        self
    }

    /// Sugar over `on_event(Reopened)`.
    pub fn on_reopen(
        self,
        mut f: impl FnMut(&mut App) -> Result<(), AppShellError> + 'static,
    ) -> Self {
        self.on_event(move |event, cx| {
            if matches!(event, AppEvent::Reopened) {
                f(cx)
            } else {
                Ok(())
            }
        })
    }

    /// Inject the platform runner (default native). Use
    /// [`PlatformRunner::headless`] in tests.
    pub fn runner(mut self, runner: PlatformRunner) -> Self {
        self.runner = runner;
        self
    }

    /// Execute the shell: run the fixed phase sequence, then the GPUI event loop.
    pub fn run(self) -> Result<(), AppShellError> {
        let Self {
            identity,
            assets,
            path_layout,
            environment,
            logging,
            initial_activation,
            exit_policy,
            configure_app,
            before_platform,
            state,
            mut plugins,
            handlers,
            start,
            configuration_error,
            error_reporter,
            shell_preferences_installed: _,
            menu_configuration: _,
            startup_hook: _,
            runner,
        } = self;

        // ---- Preflight (no GPUI) ----
        if let Some(error) = configuration_error {
            return Err(AppShellError::Configuration(error));
        }
        validate_identity(&identity)?;
        let paths =
            AppPaths::new(identity.data_namespace, path_layout).map_err(AppShellError::Paths)?;
        apply_environment(environment);
        apply_logging(logging, &paths);
        for hook in before_platform {
            hook()?;
        }
        let capabilities = PlatformCapabilities::detect();
        let app_info = AppInfo::new(identity, paths, capabilities);

        // Plugin configure() sees identity/paths/capabilities before GPUI exists.
        for plugin in &mut plugins {
            let mut ctx = BuildContext { info: &app_info };
            plugin.configure(&mut ctx);
        }

        let launch = LaunchRequest::from_env();

        // ---- ConfigureApp ----
        let mut application = runner.build()?.with_assets(ChainedAssets::new(assets));
        if let Some(configure) = configure_app {
            application = configure(application)?;
        }
        // Quit mode is shell-owned and not customizable: liveness is the single
        // quit authority, so every quit routes through `request_quit`. Applied
        // AFTER `configure_application` so the app callback cannot re-enable
        // platform auto-quit and bypass the lifecycle/teardown path.
        application = application.with_quit_mode(QuitMode::Explicit);

        // ---- EarlyListeners ----
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

        // ---- run: the remaining phases execute on the main thread ----
        let error_cell: Arc<Mutex<Option<AppShellError>>> = Arc::new(Mutex::new(None));
        let liveness = Liveness::new(exit_policy, initial_activation);
        let boot = Boot {
            app_info,
            liveness,
            initial_activation,
            plugins,
            handlers,
            pending,
            state,
            launch,
            start,
            error_reporter: error_reporter.unwrap_or_else(|| {
                Box::new(|error, _cx| {
                    log::error!("{error}");
                })
            }),
        };
        let error_slot = Arc::clone(&error_cell);
        application.run(move |cx| {
            if let Err(err) = boot.run(cx) {
                // Boot already completed fatal teardown. Retain its error until
                // the application loop returns, then surface it to the caller.
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

/// Everything moved into the run closure to execute the post-`ConfigureApp`
/// phases on the main thread.
struct Boot {
    app_info: AppInfo,
    liveness: Liveness,
    initial_activation: InitialActivation,
    plugins: Vec<Box<dyn AppPlugin>>,
    handlers: Vec<EventHandler>,
    pending: Arc<PendingEvents>,
    state: HashMap<TypeId, Box<dyn Any>>,
    launch: LaunchRequest,
    start: Option<StartCallback>,
    error_reporter: ErrorReporter,
}

impl Boot {
    fn run(self, cx: &mut App) -> Result<(), AppShellError> {
        let Self {
            app_info,
            liveness,
            initial_activation,
            mut plugins,
            handlers,
            pending,
            state,
            launch,
            start,
            error_reporter,
        } = self;

        let mut phases = PhaseTracker::new();
        phases.complete(Phase::Preflight);
        phases.complete(Phase::ConfigureApp);
        phases.complete(Phase::EarlyListeners);

        // ComponentInit
        gpui_component::init(cx);
        phases.complete(Phase::ComponentInit);

        // CoreServices: install the global (moves plugins/handlers/state in) and
        // start the cross-thread drain loop.
        let proxy = handles::install(
            cx,
            app_info.clone(),
            liveness,
            std::mem::take(&mut plugins),
            handlers,
            Arc::clone(&pending),
            state,
            phases,
            error_reporter,
        );
        cx.global_mut::<crate::handles::ShellState>()
            .record_phase(Phase::CoreServices);

        // Install lifecycle observers (incl. the `on_app_quit` teardown hook)
        // immediately, BEFORE any application-controlled handler (plugin init,
        // `Started`/`on_launch`) can run. Otherwise a `request_quit()` from an
        // `on_launch` handler would terminate before the quit observer exists,
        // skipping WillExit, reverse plugin shutdown, proxy close, and flush.
        handles::register_observers(cx);

        // PluginInit: initialize plugins with the shell seed. A failure here is
        // fatal (required service); it aborts startup, unwinding the already-
        // initialized prefix in reverse (the documented shutdown contract).
        let seed = ShellSeed {
            info: app_info,
            proxy: proxy.clone(),
        };
        let mut installed = cx.global_mut::<crate::handles::ShellState>().take_plugins();
        let mut initialized = 0usize;
        let mut init_error = None;
        for plugin in &mut installed {
            match plugin.init(cx, &seed) {
                Ok(()) => initialized += 1,
                Err(err) => {
                    init_error = Some(err);
                    break;
                }
            }
        }
        if let Some(err) = init_error {
            // The failing plugin never completed init and is not shut down; the
            // successfully-initialized prefix is torn down in reverse order.
            installed.truncate(initialized);
            cx.global_mut::<crate::handles::ShellState>()
                .restore_plugins(installed);
            handles::fail_startup(cx);
            return Err(err);
        }
        cx.global_mut::<crate::handles::ShellState>()
            .restore_plugins(installed);
        cx.global_mut::<crate::handles::ShellState>()
            .record_phase(Phase::PluginInit);

        // Start: the one fatal application-owned composition transaction.
        if let Some(start) = start
            && let Err(source) = start(&launch, cx)
        {
            handles::fail_startup(cx);
            return Err(AppShellError::Startup(source));
        }
        cx.global_mut::<crate::handles::ShellState>()
            .record_phase(Phase::Start);

        // A quit requested during plugin init/start is deferred so a later
        // startup error wins. Successful startup now performs normal shutdown,
        // without publishing readiness events or activating.
        if let Some(reason) = handles::finish_start(cx) {
            pending.close();
            handles::request_quit_with(cx, reason);
            return Ok(());
        }

        // DrainQueue: deliver Started, enable post-ready delivery, drain buffer.
        handles::deliver_event(cx, &AppEvent::Started(launch));
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
        cx.global_mut::<crate::handles::ShellState>()
            .record_phase(Phase::DrainQueue);

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
        cx.global_mut::<crate::handles::ShellState>()
            .record_phase(Phase::Activation);

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
        return Err(AppShellError::Identity("app_id is empty".into()));
    }
    if identity.data_namespace.is_empty() {
        return Err(AppShellError::Identity("data_namespace is empty".into()));
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

fn apply_logging(policy: LoggingPolicy, paths: &AppPaths) {
    match policy {
        LoggingPolicy::External => {}
        LoggingPolicy::Configure(init) => init(paths),
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
#[path = "shell_pending_tests.rs"]
mod pending_tests;

#[cfg(test)]
mod tests {
    use super::*;

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

    struct RecordingPlugin {
        name: &'static str,
        fail_init: bool,
        log: Arc<Mutex<Vec<String>>>,
    }

    impl crate::plugin::sealed::Sealed for RecordingPlugin {}

    impl AppPlugin for RecordingPlugin {
        fn init(&mut self, _cx: &mut App, _shell: &ShellSeed) -> Result<(), AppShellError> {
            self.log
                .lock()
                .expect("recording plugin log poisoned")
                .push(format!("{}:init", self.name));
            if self.fail_init {
                return Err(AppShellError::Service {
                    service: self.name,
                    source: anyhow::anyhow!("{} failed", self.name),
                });
            }
            Ok(())
        }

        fn on_event(&mut self, event: &AppEvent, _cx: &mut App) -> Result<(), AppShellError> {
            self.log
                .lock()
                .expect("recording plugin log poisoned")
                .push(format!("{}:{}", self.name, event.name()));
            Ok(())
        }

        fn shutdown(&mut self, _cx: &mut App) {
            self.log
                .lock()
                .expect("recording plugin log poisoned")
                .push(format!("{}:shutdown", self.name));
        }
    }

    fn recording_plugin(
        name: &'static str,
        fail_init: bool,
        log: Arc<Mutex<Vec<String>>>,
    ) -> Box<dyn AppPlugin> {
        Box::new(RecordingPlugin {
            name,
            fail_init,
            log,
        })
    }

    fn recording_handler(log: Arc<Mutex<Vec<String>>>, quit_on_started: bool) -> EventHandler {
        Box::new(move |event, cx| {
            log.lock()
                .expect("recording handler log poisoned")
                .push(format!("handler:{}", event.name()));
            if quit_on_started && matches!(event, AppEvent::Started(_)) {
                handles::request_quit(cx);
            }
            Ok(())
        })
    }

    fn test_boot(
        plugins: Vec<Box<dyn AppPlugin>>,
        pending: Arc<PendingEvents>,
        start: Option<StartCallback>,
        log: Arc<Mutex<Vec<String>>>,
    ) -> Boot {
        Boot {
            app_info: AppInfo::new(
                identity(),
                AppPaths::new("appshell-boot-tests", PathLayout::PlatformDefault)
                    .expect("test paths resolve"),
                PlatformCapabilities::detect(),
            ),
            liveness: Liveness::new(ExitPolicy::Explicit, InitialActivation::Passive),
            initial_activation: InitialActivation::Passive,
            plugins,
            handlers: vec![recording_handler(log, false)],
            pending,
            state: HashMap::new(),
            launch: LaunchRequest::default(),
            start,
            error_reporter: Box::new(|_, _| {}),
        }
    }

    fn assert_shutdown_rejects_proxy(cx: &mut gpui::TestAppContext, expected_last: Phase) {
        use crate::error::AppClosed;
        use crate::handles::{AppShellExt, ShellState};

        cx.update(|app| {
            let proxy = app.app_proxy();
            assert!(proxy.is_closed());
            assert_eq!(proxy.dispatch(|_| {}), Err(AppClosed));
            assert_eq!(
                app.global::<ShellState>().phases().last(),
                Some(expected_last)
            );
        });
    }

    #[gpui::test]
    fn plugin_init_failure_unwinds_initialized_prefix_without_readiness(
        cx: &mut gpui::TestAppContext,
    ) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let pending = Arc::new(PendingEvents::default());
        pending
            .push(AppEvent::Reopened)
            .expect("pre-ready event is accepted");
        let boot = test_boot(
            vec![
                recording_plugin("first", false, Arc::clone(&log)),
                recording_plugin("second", false, Arc::clone(&log)),
                recording_plugin("broken", true, Arc::clone(&log)),
            ],
            Arc::clone(&pending),
            None,
            Arc::clone(&log),
        );

        let result = cx.update(|app| boot.run(app));

        assert!(matches!(
            result,
            Err(AppShellError::Service {
                service: "broken",
                ..
            })
        ));
        assert_eq!(
            *log.lock().expect("recording plugin log poisoned"),
            vec![
                "first:init",
                "second:init",
                "broken:init",
                "first:shutdown_requested",
                "second:shutdown_requested",
                "handler:shutdown_requested",
                "first:will_exit",
                "second:will_exit",
                "handler:will_exit",
                "second:shutdown",
                "first:shutdown",
            ]
        );
        assert!(
            pending.is_empty(),
            "fatal startup clears queued launch events"
        );
        assert_shutdown_rejects_proxy(cx, Phase::CoreServices);
    }

    #[gpui::test]
    fn start_failure_unwinds_plugins_without_publishing_readiness(cx: &mut gpui::TestAppContext) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let pending = Arc::new(PendingEvents::default());
        pending
            .push(AppEvent::Reopened)
            .expect("pre-ready event is accepted");
        let start_log = Arc::clone(&log);
        let boot = test_boot(
            vec![
                recording_plugin("first", false, Arc::clone(&log)),
                recording_plugin("second", false, Arc::clone(&log)),
            ],
            Arc::clone(&pending),
            Some(Box::new(move |_, _| {
                start_log
                    .lock()
                    .expect("recording plugin log poisoned")
                    .push("start".to_string());
                Err(anyhow::anyhow!("start failed"))
            })),
            Arc::clone(&log),
        );

        let result = cx.update(|app| boot.run(app));

        assert!(matches!(result, Err(AppShellError::Startup(_))));
        assert_eq!(
            *log.lock().expect("recording plugin log poisoned"),
            vec![
                "first:init",
                "second:init",
                "start",
                "first:shutdown_requested",
                "second:shutdown_requested",
                "handler:shutdown_requested",
                "first:will_exit",
                "second:will_exit",
                "handler:will_exit",
                "second:shutdown",
                "first:shutdown",
            ]
        );
        assert!(
            pending.is_empty(),
            "fatal startup clears queued launch events"
        );
        assert_shutdown_rejects_proxy(cx, Phase::PluginInit);
    }

    #[gpui::test]
    fn successful_quit_during_start_returns_ok_without_readiness(cx: &mut gpui::TestAppContext) {
        use crate::handles::AppShellExt;

        let log = Arc::new(Mutex::new(Vec::new()));
        let pending = Arc::new(PendingEvents::default());
        pending
            .push(AppEvent::Reopened)
            .expect("pre-ready event is accepted");
        let boot = test_boot(
            vec![recording_plugin("first", false, Arc::clone(&log))],
            Arc::clone(&pending),
            Some(Box::new(|_, cx| {
                cx.request_quit();
                Ok(())
            })),
            Arc::clone(&log),
        );

        let result = cx.update(|app| boot.run(app));

        assert!(result.is_ok(), "a successful startup quit is not fatal");
        assert!(
            pending.is_empty(),
            "a startup quit clears queued launch events"
        );
        let events = log.lock().expect("recording plugin log poisoned");
        assert!(
            !events
                .iter()
                .any(|event| event.ends_with(":started") || event.ends_with(":reopened")),
            "a startup quit does not publish readiness or drain queued events: {events:?}"
        );
        assert_shutdown_rejects_proxy(cx, Phase::Start);
    }

    #[gpui::test]
    fn started_quit_discards_pending_events_and_skips_activation(cx: &mut gpui::TestAppContext) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let pending = Arc::new(PendingEvents::default());
        pending
            .push(AppEvent::Reopened)
            .expect("pre-ready event is accepted");
        let mut boot = test_boot(
            vec![recording_plugin("first", false, Arc::clone(&log))],
            Arc::clone(&pending),
            None,
            Arc::clone(&log),
        );
        boot.handlers = vec![recording_handler(Arc::clone(&log), true)];

        let result = cx.update(|app| boot.run(app));

        assert!(result.is_ok(), "Started-triggered quit is not fatal");
        let events = log.lock().expect("recording plugin log poisoned");
        assert!(events.iter().any(|event| event == "first:started"));
        assert!(
            events
                .iter()
                .any(|event| event == "handler:shutdown_requested"),
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
        assert_shutdown_rejects_proxy(cx, Phase::Start);
    }

    #[test]
    fn default_environment_policy_is_inherit() {
        let builder = AppShellBuilder::new(identity());
        assert!(matches!(builder.environment, EnvironmentPolicy::Inherit));
    }

    #[test]
    fn environment_setter_records_login_shell() {
        let builder = AppShellBuilder::new(identity()).environment(EnvironmentPolicy::LoginShell);
        assert!(matches!(builder.environment, EnvironmentPolicy::LoginShell));
    }

    #[test]
    fn inherit_environment_is_a_noop() {
        // Inherit must never shell out or mutate the environment. Applying it is a
        // pure no-op; the LoginShell path is intentionally not exercised here (it
        // would spawn the login shell and mutate process env in CI).
        apply_environment(EnvironmentPolicy::Inherit);
    }

    #[test]
    fn start_and_on_launch_share_one_slot() {
        let builder = AppShellBuilder::new(identity())
            .start(|_, _| Ok(()))
            .on_launch(|_| Ok(()));
        assert!(matches!(
            builder.configuration_error,
            Some(BuilderConfigurationError::DuplicateStartup {
                first: StartupHook::Start,
                second: StartupHook::OnLaunch,
            })
        ));
    }

    #[test]
    fn duplicate_start_records_configuration_error() {
        let builder = AppShellBuilder::new(identity())
            .start(|_, _| Ok(()))
            .start(|_, _| Ok(()));
        assert!(matches!(
            builder.configuration_error,
            Some(BuilderConfigurationError::DuplicateStartup {
                first: StartupHook::Start,
                second: StartupHook::Start,
            })
        ));
        assert!(matches!(
            builder.run(),
            Err(AppShellError::Configuration(
                BuilderConfigurationError::DuplicateStartup { .. }
            ))
        ));
    }

    #[test]
    fn platform_construction_failure_returns_platform_error() {
        let result = AppShell::builder(identity())
            .runner(PlatformRunner::failing())
            .run();

        assert!(matches!(
            result,
            Err(AppShellError::Platform(error)) if error.to_string() == "test platform construction failure"
        ));
    }

    #[test]
    fn shell_preferences_are_consumer_driven_and_idempotent() {
        let builder = AppShellBuilder::new(identity());
        assert!(!builder.shell_preferences_installed);
        assert_eq!(builder.plugins.len(), 1);

        let builder = builder.shell_preferences().shell_preferences();
        assert!(builder.shell_preferences_installed);
        assert_eq!(builder.plugins.len(), 2);
    }

    #[test]
    fn raw_and_standard_menus_are_mutually_exclusive() {
        let builder = AppShellBuilder::new(identity())
            .menus(crate::commands::MenuPlan::standard())
            .standard_menus(crate::commands::StandardMenus::new());
        assert!(matches!(
            builder.configuration_error,
            Some(BuilderConfigurationError::DuplicateMenus {
                first: MenuConfiguration::MenuPlan,
                second: MenuConfiguration::StandardMenus,
            })
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
