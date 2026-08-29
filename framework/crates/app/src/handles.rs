//! Thread-affinity-explicit handles and the main-thread shell global (plan D3).
//!
//! Two handle kinds with *distinct, compile-tested* auto-trait contracts:
//!
//! - [`AppInfo`] and [`AppProxy`] are `Clone + Send + Sync` — they cross threads
//!   (tray, watchers, audio callbacks).
//! - [`ShellState`] is a main-thread-only GPUI [`gpui::Global`], reached through
//!   the [`AppShellExt`] extension trait. It is intentionally **not** `Send`.
//!
//! Callbacks always receive a raw `&mut gpui::App`; there is no context wrapper.

use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use gpui::{App, MainThreadPoster};
use neutron_components_manifest::schema::IdentityRef;
use neutron_components_storage::AppPaths;

use crate::capabilities::PlatformCapabilities;
use crate::declaration::LaunchRuntime;
use crate::error::{AppClosed, RuntimeError};
use crate::lifecycle::{AppEvent, OpenRequest, ShutdownReason};
use crate::liveness::{Liveness, ShellHold};
use crate::module::{EventHandler, RuntimeModules};

/// Immutable application identity, resolved paths, and capability snapshot.
///
/// `Clone + Send + Sync` (compile-asserted below) so background threads can read
/// identity/paths/capabilities without touching the main-thread global.
#[derive(Clone)]
pub struct AppInfo {
    inner: Arc<AppInfoInner>,
}

struct AppInfoInner {
    identity: IdentityRef,
    paths: AppPaths,
    capabilities: PlatformCapabilities,
}

impl AppInfo {
    pub(crate) fn new(
        identity: IdentityRef,
        paths: AppPaths,
        capabilities: PlatformCapabilities,
    ) -> Self {
        Self {
            inner: Arc::new(AppInfoInner {
                identity,
                paths,
                capabilities,
            }),
        }
    }

    /// The stable application id (e.g. `com.example.app`).
    pub fn app_id(&self) -> &str {
        self.inner.identity.app_id
    }

    /// The user-facing display name.
    pub fn display_name(&self) -> &str {
        self.inner.identity.display_name
    }

    /// The canonical SemVer version (`CARGO_PKG_VERSION` of the app).
    pub fn version(&self) -> &str {
        self.inner.identity.version
    }

    /// The compiled-in borrowed identity.
    pub fn identity(&self) -> IdentityRef {
        self.inner.identity
    }

    /// Resolved per-app directories.
    pub fn paths(&self) -> &AppPaths {
        &self.inner.paths
    }

    /// Snapshot of platform capabilities.
    pub fn capabilities(&self) -> &PlatformCapabilities {
        &self.inner.capabilities
    }
}

/// Cross-thread dispatch handle: schedules work onto the main thread.
///
/// `Clone + Send + Sync`. Serves tray/hotkey/watcher/audio callbacks alike.
/// After shutdown begins, [`AppProxy::dispatch`] returns [`AppClosed`].
///
/// Backed by the gpui fork's [`MainThreadPoster`]: posting wakes the main run
/// loop through the platform dispatcher, so an idle app stays parked (no poll
/// loop). The `closed` flag is the shell-requested shutdown boundary. A
/// platform-initiated quit can close GPUI's poster first; either boundary makes
/// [`AppProxy::dispatch`] return [`AppClosed`].
#[derive(Clone)]
pub struct AppProxy {
    inner: Arc<ProxyInner>,
}

struct ProxyInner {
    poster: MainThreadPoster,
    /// Serializes a dispatch's final closed check and enqueue with shutdown.
    /// `close` acquires the gate before publishing the boundary.
    admission: Mutex<()>,
    /// Set at the `ShutdownRequested` boundary. `dispatch` rejects new work once
    /// it is set, and every posted closure re-checks it on the main thread before
    /// running so work queued before shutdown is discarded mid-teardown.
    closed: AtomicBool,
}

impl AppProxy {
    /// Create a proxy backed by the app's [`MainThreadPoster`]. Cloned to
    /// background consumers; posts run on the main thread via the app's pump.
    fn new(cx: &mut App) -> Self {
        Self {
            inner: Arc::new(ProxyInner {
                poster: cx.main_thread_poster(),
                admission: Mutex::new(()),
                closed: AtomicBool::new(false),
            }),
        }
    }

    /// Schedule `f` to run on the main thread with `&mut App`.
    ///
    /// Returns [`AppClosed`] once shutdown has begun. Shell-requested shutdown
    /// closes the proxy at `ShutdownRequested`, before teardown. Admission is
    /// serialized with `close()`: a dispatch either queues work before that
    /// boundary or observes it and returns [`AppClosed`]. Posted work re-checks
    /// `closed` on the main thread before running, so work queued before shutdown
    /// is discarded if teardown wins before the pump drains it.
    ///
    /// A platform-initiated quit may close GPUI's poster before the shell's
    /// `on_app_quit` hook closes this proxy. A `false` from that poster also maps
    /// to [`AppClosed`].
    pub fn dispatch(&self, f: impl FnOnce(&mut App) + Send + 'static) -> Result<(), AppClosed> {
        self.dispatch_inner(f, || {})
    }

    fn dispatch_inner(
        &self,
        f: impl FnOnce(&mut App) + Send + 'static,
        after_initial_check: impl FnOnce(),
    ) -> Result<(), AppClosed> {
        if self.inner.closed.load(Ordering::Acquire) {
            return Err(AppClosed);
        }
        after_initial_check();
        let _admission = self
            .inner
            .admission
            .lock()
            .expect("proxy admission gate poisoned");
        if self.inner.closed.load(Ordering::Acquire) {
            return Err(AppClosed);
        }
        let inner = Arc::clone(&self.inner);
        let posted = self.inner.poster.post(move |app| {
            if !inner.closed.load(Ordering::Acquire) {
                f(app);
            }
        });
        if posted { Ok(()) } else { Err(AppClosed) }
    }

    /// Whether the proxy has been closed.
    pub fn is_closed(&self) -> bool {
        self.inner.closed.load(Ordering::Acquire)
    }

    /// Close the proxy: reject future dispatches and cause already-queued posts
    /// to discard their payload when the pump reaches them. Idempotent.
    fn close(&self) {
        self.close_inner(|| {});
    }

    fn close_inner(&self, before_boundary: impl FnOnce()) {
        let _admission = self
            .inner
            .admission
            .lock()
            .expect("proxy admission gate poisoned");
        before_boundary();
        self.inner.closed.store(true, Ordering::Release);
    }
}

/// Raw platform events captured before services exist. Shared between the early
/// listeners (registered pre-`run`) and the main-thread drain.
#[derive(Default)]
struct PendingQueue {
    events: VecDeque<AppEvent>,
    closed: bool,
}

#[derive(Default)]
pub(crate) struct PendingEvents {
    queue: Mutex<PendingQueue>,
    /// Set only once the shell is ready; its presence tells post-ready listeners
    /// they may dispatch a drain instead of relying on the startup drain.
    pub(crate) proxy: OnceLock<AppProxy>,
}

impl PendingEvents {
    pub(crate) fn push(&self, event: AppEvent) -> Result<(), AppClosed> {
        {
            let mut queue = self.queue.lock().expect("pending queue poisoned");
            if queue.closed {
                return Err(AppClosed);
            }
            queue.events.push_back(event);
        }

        if let Some(proxy) = self.proxy.get()
            && let Err(error) = proxy.dispatch(|cx| {
                let _ = drain_pending(cx);
            })
        {
            self.close();
            return Err(error);
        }
        Ok(())
    }

    fn pop_front(&self) -> Result<Option<AppEvent>, AppClosed> {
        let mut queue = self.queue.lock().expect("pending queue poisoned");
        if queue.closed {
            return Err(AppClosed);
        }
        Ok(queue.events.pop_front())
    }

    pub(crate) fn close(&self) {
        let mut queue = self.queue.lock().expect("pending queue poisoned");
        queue.closed = true;
        queue.events.clear();
    }

    /// Whether the queue stopped accepting events at the shutdown boundary.
    #[cfg(test)]
    pub(crate) fn is_closed(&self) -> bool {
        self.queue.lock().expect("pending queue poisoned").closed
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.queue
            .lock()
            .expect("pending queue poisoned")
            .events
            .is_empty()
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.queue
            .lock()
            .expect("pending queue poisoned")
            .events
            .len()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PendingDrain {
    Completed,
    ShutdownRequested,
}

#[derive(Debug)]
enum Readiness {
    Starting {
        deferred_quit: Option<ShutdownReason>,
    },
    Ready,
    Failed,
}

impl Readiness {
    /// The quit a module or start hook deferred, without consuming it.
    ///
    /// Read-only on purpose: [`Readiness::finish_start`] publishes readiness as
    /// a side effect, so the startup sequence cannot use it to *ask* whether a
    /// quit is pending before the primary surface exists.
    fn deferred_quit(&self) -> Option<ShutdownReason> {
        match self {
            Self::Starting { deferred_quit } => *deferred_quit,
            Self::Ready | Self::Failed => None,
        }
    }

    fn defer_quit(&mut self, reason: ShutdownReason) -> bool {
        let Self::Starting { deferred_quit } = self else {
            return false;
        };
        if deferred_quit.is_none() {
            *deferred_quit = Some(reason);
        }
        true
    }

    fn finish_start(&mut self) -> Option<ShutdownReason> {
        match std::mem::replace(self, Self::Ready) {
            Self::Starting { deferred_quit } => deferred_quit,
            state => {
                *self = state;
                None
            }
        }
    }
}

type ErrorReporter = Box<dyn Fn(&RuntimeError, &mut App)>;

/// The declaration's single application shutdown hook.
///
/// `FnOnce` encodes the exactly-once contract in the type: the hook is moved out
/// of the global when it runs, so no teardown path can run it twice.
pub(crate) type AppShutdownHook = Box<dyn FnOnce(&mut App) -> anyhow::Result<()>>;

/// Main-thread shell state. A GPUI [`gpui::Global`]; intentionally not `Send`.
pub(crate) struct ShellState {
    app_info: AppInfo,
    proxy: AppProxy,
    liveness: Liveness,
    modules: RuntimeModules,
    observers: Vec<EventHandler>,
    pending: Arc<PendingEvents>,
    subscriptions: Vec<gpui::Subscription>,
    readiness: Readiness,
    error_reporter: Option<ErrorReporter>,
    reporting_error: bool,
    /// The declaration's application shutdown hook, taken when it runs.
    app_shutdown: Option<AppShutdownHook>,
    /// Whether the application startup transaction has begun.
    ///
    /// Set immediately before the common start phase runs — including when no
    /// start hook is registered — and never cleared. It is the precondition for
    /// [`run_app_shutdown`]: an application only tears down if it was given the
    /// chance to build. A framework, module, or setup failure *before*
    /// this point leaves nothing application-owned to unwind.
    app_start_entered: bool,
    /// Re-entrancy guard for `deliver_event`.
    delivery: crate::lifecycle::ReentrantQueue,
    /// The reason to attribute if the next idle evaluation triggers an exit.
    /// Set by the window-close observer (which fires before a window-manager
    /// module drops the window's hold), consumed by `evaluate_exit`.
    pending_exit_reason: Option<ShutdownReason>,
    shutdown_requested: bool,
    will_exit_done: bool,
    /// The declaration's retained launch runtime, including the immutable
    /// typed launch value and the primary opener (issues #3/#6/#29). Set once
    /// via [`set_launch_runtime`] right after this global is installed, and
    /// never published through a public accessor: [`restore_primary_on_reopen`]
    /// is its only reader. `None` in a test harness that installs this global
    /// directly without a declared launch runtime, which is the correct no-op
    /// (nothing to restore).
    launch_runtime: Option<Rc<LaunchRuntime>>,
}

impl gpui::Global for ShellState {}

impl ShellState {
    /// Whether readiness has been published (`finish_start` has run).
    #[cfg(test)]
    pub(crate) fn is_ready(&self) -> bool {
        matches!(self.readiness, Readiness::Ready)
    }

    /// The live liveness-lease counter, for asserting that a hold is held.
    #[cfg(test)]
    pub(crate) fn holds(&self) -> usize {
        self.liveness
            .holds_arc()
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    pub(crate) fn is_shutdown_requested(&self) -> bool {
        self.shutdown_requested
    }

    /// Move the runtime modules out for a `&mut App` re-entrant call (init).
    pub(crate) fn take_modules(&mut self) -> RuntimeModules {
        std::mem::take(&mut self.modules)
    }

    /// Restore modules taken by [`ShellState::take_modules`].
    pub(crate) fn restore_modules(&mut self, modules: RuntimeModules) {
        self.modules = modules;
    }
}

/// A lightweight, cloneable handle for taking liveness leases and driving quit.
#[derive(Clone)]
pub(crate) struct ShellHandle {
    proxy: AppProxy,
    holds: Arc<std::sync::atomic::AtomicUsize>,
}

impl ShellHandle {
    /// Acquire a liveness lease. The shell stays alive until it is dropped (and
    /// no windows remain, under [`crate::ExitPolicy::WhenIdle`]).
    pub fn hold(&self, reason: &'static str) -> ShellHold {
        crate::liveness::acquire_hold(Arc::clone(&self.holds), self.proxy.clone(), reason)
    }

    /// The cross-thread proxy.
    #[allow(dead_code)] // Exercised only by unit tests; no production caller yet.
    pub fn proxy(&self) -> AppProxy {
        self.proxy.clone()
    }
}

/// Extension trait exposing shell services on the raw `gpui::App`.
pub(crate) trait AppShellExt {
    /// Immutable app identity/paths/capabilities.
    fn app_info(&self) -> &AppInfo;
    /// A cross-thread dispatch proxy.
    fn app_proxy(&self) -> AppProxy;
    /// A handle for liveness leases and quit.
    fn shell(&self) -> ShellHandle;
    /// Route a quit through the single shutdown path.
    fn request_quit(&mut self);
}

impl AppShellExt for App {
    fn app_info(&self) -> &AppInfo {
        &self.global::<ShellState>().app_info
    }

    fn app_proxy(&self) -> AppProxy {
        self.global::<ShellState>().proxy.clone()
    }

    fn shell(&self) -> ShellHandle {
        let st = self.global::<ShellState>();
        ShellHandle {
            proxy: st.proxy.clone(),
            holds: st.liveness.holds_arc(),
        }
    }

    fn request_quit(&mut self) {
        request_quit(self);
    }
}

/// Install the shell global. Wires the cross-thread proxy to the app's
/// [`MainThreadPoster`]; no poll loop — posts wake the main run loop directly.
///
/// Called once core services come up, with the constructed `AppInfo` and the
/// runtime modules and lifecycle observers the declaration lowered.
#[allow(clippy::too_many_arguments)]
pub(crate) fn install(
    cx: &mut App,
    app_info: AppInfo,
    liveness: Liveness,
    modules: RuntimeModules,
    observers: Vec<EventHandler>,
    pending: Arc<PendingEvents>,
    error_reporter: ErrorReporter,
    app_shutdown: Option<AppShutdownHook>,
) -> AppProxy {
    let proxy = AppProxy::new(cx);

    cx.set_global(ShellState {
        app_info,
        proxy: proxy.clone(),
        liveness,
        modules,
        observers,
        pending,
        subscriptions: Vec::new(),
        readiness: Readiness::Starting {
            deferred_quit: None,
        },
        error_reporter: Some(error_reporter),
        reporting_error: false,
        app_shutdown,
        app_start_entered: false,
        delivery: crate::lifecycle::ReentrantQueue::new(),
        pending_exit_reason: None,
        shutdown_requested: false,
        will_exit_done: false,
        launch_runtime: None,
    });
    proxy
}

/// Retain the launch runtime for the running shell's lifetime (issues
/// #3/#6/#29): the same immutable typed launch value and primary opener stay
/// reachable for a later `Reopened` restore. Called once, immediately after
/// [`install`], before any observer can run.
pub(crate) fn set_launch_runtime(cx: &mut App, runtime: Rc<LaunchRuntime>) {
    cx.global_mut::<ShellState>().launch_runtime = Some(runtime);
}

/// Register lifecycle observers (window-closed, app-quit) after readiness.
pub(crate) fn register_observers(cx: &mut App) {
    let window_closed = cx.on_window_closed(|app| {
        if app.windows().is_empty() {
            // Record the reason now. The actual exit may only happen on a later
            // tick, after the window module's reconcile drops the closed
            // window's liveness hold — this observer runs before that. Recording
            // it lets the eventual `evaluate_exit` attribute the exit to
            // LastWindowClosed instead of the generic Requested, and avoids a
            // premature exit here while the hold is still counted.
            app.global_mut::<ShellState>().pending_exit_reason =
                Some(ShutdownReason::LastWindowClosed);
            deliver_event(app, &AppEvent::LastWindowClosed);
        }
        evaluate_exit(app);
    });
    let app_quit = cx.on_app_quit(|app| {
        run_will_exit(app);
        std::future::ready(())
    });
    let st = cx.global_mut::<ShellState>();
    st.subscriptions.push(window_closed);
    st.subscriptions.push(app_quit);
}

/// Deliver `event` to every runtime module then every declared observer.
///
/// Re-entrancy-safe: a delivery moves modules/observers out of the global for the
/// pass, so a callback that itself delivers an event (e.g. `request_quit()` from
/// a `Started` handler emitting `ShutdownRequested`) would otherwise hit empty
/// subscriber lists. Such nested events are buffered and drained after the
/// current pass, in order (see [`crate::lifecycle::ReentrantQueue`]).
///
/// This is the one delivery path for `AppEvent::Reopened`, reached both by a
/// live platform reopen and by one queued before readiness: both route through
/// [`PendingEvents`], whose drain (`drain_pending`, and the post-ready dispatch
/// in [`PendingEvents::push`]) calls this function for every event it pops.
/// [`restore_primary_on_reopen`] runs here, before `deliver_one`, so it always
/// runs before any module or observer sees `Reopened` (issues #3/#6/#29).
pub(crate) fn deliver_event(cx: &mut App, event: &AppEvent) {
    if !cx.has_global::<ShellState>() {
        return;
    }
    if !cx.global_mut::<ShellState>().delivery.try_enter(event) {
        // A delivery is already in progress; this event is now buffered and will
        // be drained by the active pass below.
        return;
    }
    let mut current = event.clone();
    loop {
        if matches!(current, AppEvent::Reopened) {
            restore_primary_on_reopen(cx);
        }
        deliver_one(cx, &current);
        match cx.global_mut::<ShellState>().delivery.take_next() {
            Some(next) => current = next,
            None => break,
        }
    }
}

/// Before any `Reopened` observer runs: if no declared surface is currently
/// live, reopen the primary with the shell's retained launch value (issues
/// #3/#6/#29). Otherwise a no-op — an application with a live primary,
/// auxiliary, Settings, or About surface already has something open, and one
/// is never created when any of them is live.
///
/// A restore failure is caught and reported nonfatally through
/// [`RuntimeError::lifecycle`] rather than propagated: this runs long after
/// startup completed, so misclassifying it as a startup failure (which would
/// tear the whole shell down) would be wrong. [`LaunchRuntime::open_primary`]
/// returns the underlying failure undecorated (no `AppShellError::Startup`
/// wrapper), so the reported error's source chain names the real cause
/// directly instead of a startup-classified wrapper that only ever applied
/// to the initial-startup caller. Reopened is still delivered to observers
/// afterward either way.
///
/// A no-op once shutdown has begun: `PendingEvents::close` already discards a
/// `Reopened` queued after that boundary, but one already buffered by the
/// [`crate::lifecycle::ReentrantQueue`] ahead of a same-pass `request_quit`
/// could still reach here after `shutdown_requested` is set, and recreating a
/// window while teardown is already underway would fight it. Observers still
/// receive `Reopened` in that case; only the restore attempt is skipped.
fn restore_primary_on_reopen(cx: &mut App) {
    if cx.global::<ShellState>().shutdown_requested {
        return;
    }
    let Some(runtime) = cx.global::<ShellState>().launch_runtime.clone() else {
        return;
    };
    if crate::windows::any_declared_surface_live(cx) {
        return;
    }
    if let Err(error) = runtime.open_primary(cx) {
        report_error(cx, RuntimeError::lifecycle(AppEvent::Reopened, error));
    }
}

/// Deliver a single event to the runtime modules then the declared observers,
/// moving them out of the global for the call (so they can receive `&mut App`)
/// and restoring them. Their errors are reported, not fatal.
fn deliver_one(cx: &mut App, event: &AppEvent) {
    let (mut modules, mut observers) = {
        let st = cx.global_mut::<ShellState>();
        (
            std::mem::take(&mut st.modules),
            std::mem::take(&mut st.observers),
        )
    };

    let mut module_errors = Vec::new();
    for module in &mut modules {
        if let Err(err) = module.on_event(event, cx) {
            module_errors.push((module.id(), err));
        }
    }
    let mut observer_errors = Vec::new();
    for observer in &mut observers {
        if let Err(err) = observer(event, cx) {
            observer_errors.push(err);
        }
    }

    let st = cx.global_mut::<ShellState>();
    st.modules = modules;
    st.observers = observers;
    for (module, error) in module_errors {
        report_error(cx, RuntimeError::module(module, error));
    }
    for error in observer_errors {
        report_error(cx, RuntimeError::lifecycle(event.clone(), error));
    }
}

/// Report a nonfatal runtime error through the configured sink.
///
/// A reporter that recursively reports falls back to logging rather than
/// re-entering itself.
pub(crate) fn report_error(cx: &mut App, error: RuntimeError) {
    if !cx.has_global::<ShellState>() {
        log::error!("{error}");
        return;
    }
    let reporter = {
        let state = cx.global_mut::<ShellState>();
        if state.reporting_error {
            log::error!("{error}");
            return;
        }
        state.reporting_error = true;
        state.error_reporter.take()
    };
    if let Some(reporter) = reporter {
        reporter(&error, cx);
        let state = cx.global_mut::<ShellState>();
        state.error_reporter = Some(reporter);
        state.reporting_error = false;
    } else {
        log::error!("{error}");
        cx.global_mut::<ShellState>().reporting_error = false;
    }
}

/// The quit a module `init` or the common start hook deferred, if any.
///
/// Non-mutating: unlike [`finish_start`], asking does not publish readiness, so
/// the startup sequence can skip the launch hook and the primary surface
/// without marking the shell ready before a surface exists.
pub(crate) fn deferred_quit(cx: &App) -> Option<ShutdownReason> {
    if !cx.has_global::<ShellState>() {
        return None;
    }
    cx.global::<ShellState>().readiness.deferred_quit()
}

/// Mark the application startup transaction as begun.
///
/// Called by the startup sequence immediately before the common start phase,
/// whether or not a start hook is registered. From this point on the
/// application owns state that a failure must unwind, so every later teardown
/// runs the declared application shutdown hook.
pub(crate) fn enter_app_start(cx: &mut App) {
    cx.global_mut::<ShellState>().app_start_entered = true;
}

/// Complete the transactional startup state and return any deferred quit.
pub(crate) fn finish_start(cx: &mut App) -> Option<ShutdownReason> {
    cx.global_mut::<ShellState>().readiness.finish_start()
}

/// Abort startup, discard queued events, and run fatal teardown exactly once.
pub(crate) fn fail_startup(cx: &mut App) {
    if !cx.has_global::<ShellState>()
        || matches!(cx.global::<ShellState>().readiness, Readiness::Failed)
    {
        return;
    }
    let pending = {
        let state = cx.global_mut::<ShellState>();
        state.readiness = Readiness::Failed;
        state.shutdown_requested = true;
        state.pending.clone()
    };
    pending.close();
    cx.global::<ShellState>().proxy.close();
    deliver_event(
        cx,
        &AppEvent::ShutdownRequested(ShutdownReason::StartupFailure),
    );
    run_will_exit(cx);
}

/// Drain events buffered by early platform listeners and deliver them.
pub(crate) fn drain_pending(cx: &mut App) -> PendingDrain {
    if !cx.has_global::<ShellState>() {
        return PendingDrain::Completed;
    }
    let pending = cx.global::<ShellState>().pending.clone();
    loop {
        if cx.global::<ShellState>().shutdown_requested {
            pending.close();
            return PendingDrain::ShutdownRequested;
        }

        let event = match pending.pop_front() {
            Ok(Some(event)) => event,
            Ok(None) => return PendingDrain::Completed,
            Err(AppClosed) => return PendingDrain::ShutdownRequested,
        };
        deliver_event(cx, &event);
    }
}

/// Re-evaluate the exit policy; quit through the single path if idle.
///
/// If an exit is triggered, it is attributed to any `pending_exit_reason`
/// recorded by the window-close observer (consumed here), else
/// [`ShutdownReason::Requested`]. This makes attribution robust to the ordering
/// between this observer and the window module's hold-drop: the reason is
/// recorded on close and consumed by whichever evaluation actually exits.
pub(crate) fn evaluate_exit(cx: &mut App) {
    if !cx.has_global::<ShellState>() {
        return;
    }
    let (policy, holds) = {
        let st = cx.global::<ShellState>();
        if st.shutdown_requested {
            return;
        }
        (st.liveness.exit_policy(), st.liveness.hold_count())
    };
    let windows = cx.windows().len();
    if crate::liveness::should_exit(policy, holds, windows) {
        let reason = cx
            .global_mut::<ShellState>()
            .pending_exit_reason
            .take()
            .unwrap_or(ShutdownReason::Requested);
        request_quit_with(cx, reason);
    } else if windows > 0 {
        // The app is alive because a window exists; clear any stale window-close
        // reason so a later idle exit (e.g. via hold-drop) is not misattributed.
        cx.global_mut::<ShellState>().pending_exit_reason = None;
    }
}

/// The single public quit path. Idempotent. Attributes
/// [`ShutdownReason::Requested`].
pub(crate) fn request_quit(cx: &mut App) {
    request_quit_with(cx, ShutdownReason::Requested);
}

/// The single quit path with an explicit reason. Idempotent.
pub(crate) fn request_quit_with(cx: &mut App, reason: ShutdownReason) {
    if !cx.has_global::<ShellState>() {
        cx.quit();
        return;
    }
    {
        let st = cx.global_mut::<ShellState>();
        if st.readiness.defer_quit(reason) {
            return;
        }
        if matches!(st.readiness, Readiness::Failed) {
            return;
        }
        if st.shutdown_requested {
            return;
        }
        st.shutdown_requested = true;
    }
    // Stop accepting cross-thread work at the shutdown boundary, before any
    // teardown runs — background producers must not enqueue callbacks that would
    // land mid-shutdown.
    let pending = cx.global::<ShellState>().pending.clone();
    pending.close();
    cx.global::<ShellState>().proxy.close();
    deliver_event(cx, &AppEvent::ShutdownRequested(reason));
    // Platform quit fires `on_app_quit`, which runs `run_will_exit`.
    cx.quit();
}

/// Final teardown, delivered for every quit cause via `on_app_quit`.
fn run_will_exit(cx: &mut App) {
    if !cx.has_global::<ShellState>() {
        return;
    }
    {
        let st = cx.global_mut::<ShellState>();
        if st.will_exit_done {
            return;
        }
        st.will_exit_done = true;
    }
    // Close the proxy before any teardown so nothing is accepted past the
    // shell boundary. Idempotent: it is already closed on `request_quit`; on a
    // platform-initiated quit this is the earliest shell hook, though GPUI may
    // already have rejected poster sends.
    let pending = cx.global::<ShellState>().pending.clone();
    pending.close();
    cx.global::<ShellState>().proxy.close();
    // A platform-initiated quit never went through `request_quit`; surface a
    // uniform `ShutdownRequested` first.
    let already_requested = cx.global::<ShellState>().shutdown_requested;
    if !already_requested {
        cx.global_mut::<ShellState>().shutdown_requested = true;
        deliver_event(
            cx,
            &AppEvent::ShutdownRequested(ShutdownReason::PlatformQuit),
        );
    }
    deliver_event(cx, &AppEvent::WillExit);
    run_app_shutdown(cx);
    shutdown_modules(cx);
}

/// Run the declaration's application shutdown hook, at most once.
///
/// Skipped entirely unless the application startup transaction began (see
/// [`enter_app_start`]). A framework, module, or setup failure *before*
/// the common start phase means the application never ran a line of its own
/// composition code, so calling its shutdown hook would ask it to tear down
/// state it never built.
///
/// Otherwise it runs between `WillExit` and reverse module shutdown: the
/// application tears down while every framework module it was built on is still
/// live, and the modules then tear down under it. A failure is nonfatal by
/// definition — the process is already exiting — so it is reported as
/// [`RuntimeError::shutdown`] and teardown continues.
fn run_app_shutdown(cx: &mut App) {
    if !cx.global::<ShellState>().app_start_entered {
        return;
    }
    let Some(hook) = cx.global_mut::<ShellState>().app_shutdown.take() else {
        return;
    };
    if let Err(error) = hook(cx) {
        report_error(cx, RuntimeError::shutdown(error));
    }
}

/// Shut the runtime modules down in reverse init order.
fn shutdown_modules(cx: &mut App) {
    let mut modules = std::mem::take(&mut cx.global_mut::<ShellState>().modules);
    for module in modules.iter_mut().rev() {
        module.shutdown(cx);
    }
    // The modules are done; they are not restored.
}

/// Convenience for early listeners that need to synthesize an `OpenRequest`.
pub(crate) fn open_event(urls: Vec<String>) -> AppEvent {
    AppEvent::OpenRequested(OpenRequest { urls })
}

// ---------------------------------------------------------------------------
// Auto-trait contract assertions (plan §3, gate 3). Hand-rolled, no new deps.
// ---------------------------------------------------------------------------

const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    fn assert_clone<T: Clone>() {}
    assert_send_sync::<AppInfo>();
    assert_send_sync::<AppProxy>();
    assert_send_sync::<ShellHandle>();
    assert_clone::<AppInfo>();
    assert_clone::<AppProxy>();
    assert_clone::<ShellHandle>();
    assert_send_sync::<PendingEvents>();
};

// Assert `ShellState` is NOT `Send` (it is a main-thread global). The two
// blanket impls below are ambiguous for any `Send` type, so the inference-driven
// reference resolves only when `ShellState: !Send`.
#[allow(dead_code)]
trait AmbiguousIfSend<A> {
    fn deconflict() {}
}
impl<T: ?Sized> AmbiguousIfSend<()> for T {}
impl<T: ?Sized + Send> AmbiguousIfSend<u8> for T {}

const _: fn() = || {
    let _ = <ShellState as AmbiguousIfSend<_>>::deconflict;
};

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::sync_channel;

    use super::*;
    use gpui::TestAppContext;

    #[test]
    fn starting_latches_only_first_quit_reason() {
        let mut readiness = Readiness::Starting {
            deferred_quit: None,
        };
        assert!(readiness.defer_quit(ShutdownReason::Requested));
        assert!(readiness.defer_quit(ShutdownReason::LastWindowClosed));
        assert_eq!(readiness.finish_start(), Some(ShutdownReason::Requested));
        assert!(matches!(readiness, Readiness::Ready));
    }

    #[test]
    fn ready_and_failed_do_not_defer_quit() {
        let mut ready = Readiness::Ready;
        assert!(!ready.defer_quit(ShutdownReason::Requested));
        let mut failed = Readiness::Failed;
        assert!(!failed.defer_quit(ShutdownReason::Requested));
    }

    /// Posted work runs on the main thread; once closed, `dispatch` rejects.
    #[gpui::test]
    fn dispatch_runs_until_closed(cx: &mut TestAppContext) {
        let proxy = cx.update(AppProxy::new);
        let log = Arc::new(Mutex::new(Vec::<u32>::new()));

        assert!(!proxy.is_closed());
        let sink = log.clone();
        proxy
            .dispatch(move |_app| sink.lock().unwrap().push(1))
            .expect("dispatch before close");
        cx.run_until_parked();
        assert_eq!(
            *log.lock().unwrap(),
            vec![1],
            "posted work ran on main thread"
        );

        proxy.close();
        assert!(proxy.is_closed());
        assert_eq!(
            proxy.dispatch(|_app| {}),
            Err(AppClosed),
            "dispatch rejected after close"
        );
    }

    /// Work accepted before the boundary is discarded if shutdown wins before
    /// the pump drains it (the run-time re-check in the posted closure).
    #[gpui::test]
    fn queued_work_discarded_after_close(cx: &mut TestAppContext) {
        let proxy = cx.update(AppProxy::new);
        let log = Arc::new(Mutex::new(Vec::<u32>::new()));

        let sink = log.clone();
        proxy
            .dispatch(move |_app| sink.lock().unwrap().push(1))
            .expect("dispatch before close");
        proxy.close();
        cx.run_until_parked();
        assert!(
            log.lock().unwrap().is_empty(),
            "payload queued before close is discarded, not run mid-teardown"
        );
    }

    /// A callback that triggers shutdown discards work queued behind it, in FIFO
    /// order — the equivalent of the old drain loop's between-callback re-check.
    #[gpui::test]
    fn callback_shutdown_discards_following_work(cx: &mut TestAppContext) {
        let proxy = cx.update(AppProxy::new);
        let log = Arc::new(Mutex::new(Vec::<char>::new()));

        let sink = log.clone();
        let closer = proxy.clone();
        let after_close = proxy.clone();
        proxy
            .dispatch(move |_app| {
                sink.lock().unwrap().push('a');
                closer.close();
                assert_eq!(after_close.dispatch(|_| {}), Err(AppClosed));
            })
            .expect("first dispatch");
        let sink = log.clone();
        proxy
            .dispatch(move |_app| sink.lock().unwrap().push('b'))
            .expect("second dispatch");

        cx.run_until_parked();
        assert_eq!(
            *log.lock().unwrap(),
            vec!['a'],
            "work behind the shutdown-triggering callback is discarded"
        );
    }

    /// `close` must hold admission before it publishes the shutdown boundary.
    /// A dispatch that sees the pre-boundary state but waits on admission must
    /// observe the boundary at its final check and return `AppClosed`.
    #[gpui::test]
    fn close_publishes_boundary_after_admission(cx: &mut TestAppContext) {
        let proxy = cx.update(AppProxy::new);
        let (admitted_sender, admitted_receiver) = sync_channel(0);
        let (release_sender, release_receiver) = sync_channel(0);

        let closer = proxy.clone();
        let close_thread = std::thread::spawn(move || {
            closer.close_inner(|| {
                admitted_sender.send(()).expect("signal close admission");
                release_receiver.recv().expect("release close boundary");
            });
        });

        admitted_receiver.recv().expect("wait for close admission");
        if proxy.is_closed() {
            release_sender.send(()).expect("release close boundary");
            close_thread.join().expect("join close");
            panic!("close published its boundary before acquiring admission");
        }

        let (initial_check_sender, initial_check_receiver) = sync_channel(0);
        let (release_initial_check_sender, release_initial_check_receiver) = sync_channel(0);
        let executed = Arc::new(AtomicBool::new(false));
        let dispatched = proxy;
        let executed_in_dispatch = Arc::clone(&executed);
        let dispatch_thread = std::thread::spawn(move || {
            dispatched.dispatch_inner(
                move |_app| {
                    executed_in_dispatch.store(true, Ordering::SeqCst);
                },
                || {
                    initial_check_sender
                        .send(())
                        .expect("signal initial dispatch check");
                    release_initial_check_receiver
                        .recv()
                        .expect("release initial dispatch check");
                },
            )
        });

        initial_check_receiver
            .recv()
            .expect("wait for initial dispatch check");
        release_initial_check_sender
            .send(())
            .expect("release initial dispatch check");
        release_sender.send(()).expect("release close boundary");
        close_thread.join().expect("join close");

        assert_eq!(
            dispatch_thread.join().expect("join dispatch"),
            Err(AppClosed),
            "dispatch that observed the pre-boundary state must be rejected"
        );
        cx.run_until_parked();
        assert!(
            !executed.load(Ordering::SeqCst),
            "rejected dispatch must not enqueue a payload"
        );
    }

    #[test]
    fn pending_events_close_clears_and_rejects_before_proxy_publication() {
        let pending = PendingEvents::default();
        pending
            .push(AppEvent::Reopened)
            .expect("pre-ready event is accepted");
        assert!(!pending.is_empty());

        pending.close();

        assert!(pending.is_empty());
        assert_eq!(pending.push(AppEvent::Reopened), Err(AppClosed));
        assert!(pending.proxy.get().is_none());
    }

    // ------------------------------------------------------- reopen restore
    // Regression coverage for the retained-`LaunchRuntime` primary-restore
    // contract (issues #3/#6/#29). `deliver_event` is the single internal
    // delivery path both a live platform `on_reopen` callback and a reopen
    // queued before readiness funnel through (via `PendingEvents`/
    // `drain_pending`); the headless test backend cannot trigger the platform
    // callback itself, so these tests call `deliver_event` directly.

    /// A typed primary content view carrying a `u32` marker value, so a
    /// recreated primary can be distinguished from a fresh/default-built one.
    struct Marker {
        value: u32,
    }

    impl gpui::Render for Marker {
        fn render(
            &mut self,
            _window: &mut gpui::Window,
            _cx: &mut gpui::Context<Self>,
        ) -> impl gpui::IntoElement {
            gpui::div()
        }
    }

    fn build_marker(value: &u32, _window: &mut gpui::Window, cx: &mut App) -> gpui::Entity<Marker> {
        use gpui::AppContext as _;
        cx.new(|_| Marker { value: *value })
    }

    fn parse_marker(
        _process: &crate::declaration::ProcessLaunch,
    ) -> anyhow::Result<crate::declaration::LaunchDecision<u32>> {
        Ok(crate::declaration::LaunchDecision::Run(42))
    }

    /// A retained runtime for a declaration with exactly one typed primary,
    /// carrying the marker value `42`.
    fn marker_runtime() -> Rc<LaunchRuntime> {
        use crate::declaration::{AppDeclaration, LaunchSpec, PreparedLaunch, Surface, SurfaceKey};

        let prepared = AppDeclaration::new(crate::declaration::tests::identity())
            .launch(LaunchSpec::new(parse_marker).primary_surface(Surface::new(
                SurfaceKey::<Marker, u32>::primary(),
                build_marker,
            )))
            .prepare_launch(&crate::declaration::ProcessLaunch::empty())
            .expect("the marker parser succeeds");
        let PreparedLaunch::Run(runtime) = prepared else {
            panic!("the marker parser always runs");
        };
        Rc::new(runtime)
    }

    /// Install the shell global and the window manager, retaining `runtime`
    /// and registering `observers` — mirroring `Startup::run`'s own sequence
    /// (`install`, then the launch runtime is retained, before any surface
    /// opens). Declares the marker primary surface unless `declare_primary`
    /// is false (used to exercise a restore failure).
    fn install_marker_shell(
        cx: &mut App,
        namespace: &'static str,
        runtime: &Rc<LaunchRuntime>,
        declare_primary: bool,
        observers: Vec<EventHandler>,
        error_reporter: Box<dyn Fn(&RuntimeError, &mut App)>,
    ) -> (AppInfo, AppProxy) {
        use crate::declaration::{DeclaredSurface, Surface, SurfaceKey, SurfaceRole};
        use crate::liveness::{ExitPolicy, InitialActivation};
        use crate::module::RuntimeModule as _;
        use crate::windows::{WindowsModule, declared_surface_module};
        use neutron_components_storage::PathLayout;

        neutron_components::init(cx);
        let info = AppInfo::new(
            crate::declaration::tests::identity(),
            AppPaths::new(namespace, PathLayout::PlatformDefault).expect("test paths resolve"),
            PlatformCapabilities::detect(),
        );
        let proxy = install(
            cx,
            info.clone(),
            Liveness::new(ExitPolicy::Explicit, InitialActivation::Passive),
            Vec::new(),
            observers,
            Arc::new(PendingEvents::default()),
            error_reporter,
            None,
        );
        set_launch_runtime(cx, Rc::clone(runtime));
        WindowsModule::new()
            .init(cx, &info, &proxy)
            .expect("the window manager initializes");
        if declare_primary {
            declared_surface_module(DeclaredSurface::erase(
                Surface::new(SurfaceKey::<Marker, u32>::primary(), build_marker),
                SurfaceRole::Primary,
            ))
            .init(cx, &info, &proxy)
            .expect("the primary surface installs");
        }
        (info, proxy)
    }

    #[gpui::test]
    fn reopen_restores_the_primary_from_the_retained_launch_value(cx: &mut TestAppContext) {
        use crate::declaration::SurfaceKey;
        use crate::windows::{SurfaceOpen, WindowManager, open_surface};

        let runtime = marker_runtime();
        let observed_window_count_on_reopen = Arc::new(Mutex::new(None));
        let observer = Arc::clone(&observed_window_count_on_reopen);
        let observers: Vec<EventHandler> = vec![Box::new(move |event, cx| {
            if matches!(event, AppEvent::Reopened) {
                *observer.lock().expect("observer log poisoned") =
                    Some(cx.global::<WindowManager>().window_count());
            }
            Ok(())
        })];

        cx.update(|cx| {
            install_marker_shell(
                cx,
                "appshell-reopen-restore-tests",
                &runtime,
                true,
                observers,
                Box::new(|_, _| {}),
            );

            let SurfaceOpen::Created(handle) =
                open_surface(cx, SurfaceKey::<Marker, u32>::primary(), &42)
                    .expect("the primary opens")
            else {
                panic!("the first open creates the primary");
            };
            assert_eq!(
                cx.global::<WindowManager>().window_count(),
                1,
                "the initial primary opens once",
            );
            handle.close(cx).expect("close the primary");
        });

        cx.update(|cx| {
            assert!(
                !crate::windows::any_declared_surface_live(cx),
                "close reconciliation completed before Reopened is delivered",
            );

            deliver_event(cx, &AppEvent::Reopened);

            assert_eq!(
                cx.global::<WindowManager>().window_count(),
                1,
                "Reopened recreates the primary",
            );
            assert_eq!(
                *observed_window_count_on_reopen
                    .lock()
                    .expect("observer log poisoned"),
                Some(1),
                "the primary already exists by the time the Reopened observer runs",
            );

            let SurfaceOpen::Reused(reopened) =
                open_surface(cx, SurfaceKey::<Marker, u32>::primary(), &0)
                    .expect("the recreated primary is live")
            else {
                panic!("this probe reuses the recreated primary");
            };
            assert_eq!(
                reopened.content().read(cx).value,
                42,
                "the recreated primary was rebuilt from the same retained launch value",
            );
        });
    }

    #[gpui::test]
    fn reopen_does_not_create_a_primary_while_another_declared_surface_is_live(
        cx: &mut TestAppContext,
    ) {
        use crate::declaration::{DeclaredSurface, Surface, SurfaceKey, SurfaceRole};
        use crate::module::RuntimeModule as _;
        use crate::windows::{SurfaceOpen, WindowManager, declared_surface_module, open_surface};

        let runtime = marker_runtime();

        cx.update(|cx| {
            let (info, proxy) = install_marker_shell(
                cx,
                "appshell-reopen-restore-aux-tests",
                &runtime,
                true,
                Vec::new(),
                Box::new(|_, _| {}),
            );
            declared_surface_module(DeclaredSurface::erase(
                Surface::new(SurfaceKey::<Marker, u32>::new("aux"), build_marker),
                SurfaceRole::Auxiliary,
            ))
            .init(cx, &info, &proxy)
            .expect("the auxiliary surface installs");

            let SurfaceOpen::Created(primary) =
                open_surface(cx, SurfaceKey::<Marker, u32>::primary(), &42)
                    .expect("the primary opens")
            else {
                panic!("the first open creates the primary");
            };
            open_surface(cx, SurfaceKey::<Marker, u32>::new("aux"), &7)
                .expect("the auxiliary surface opens");
            primary.close(cx).expect("close the primary");
        });

        cx.update(|cx| {
            assert!(
                crate::windows::any_declared_surface_live(cx),
                "the auxiliary surface is still live",
            );

            deliver_event(cx, &AppEvent::Reopened);

            assert_eq!(
                cx.global::<WindowManager>().window_count(),
                1,
                "Reopened does not create a primary while another declared \
                 surface remains live",
            );
        });
    }

    #[gpui::test]
    fn a_failed_primary_restore_is_reported_and_reopened_still_reaches_observers(
        cx: &mut TestAppContext,
    ) {
        let runtime = marker_runtime();
        let errors: Arc<Mutex<Vec<(crate::error::RuntimeOperation, Option<AppEvent>, String)>>> =
            Arc::new(Mutex::new(Vec::new()));
        let error_log = Arc::clone(&errors);
        let observed_reopened = Arc::new(Mutex::new(false));
        let observer = Arc::clone(&observed_reopened);
        let observers: Vec<EventHandler> = vec![Box::new(move |event, _cx| {
            if matches!(event, AppEvent::Reopened) {
                *observer.lock().expect("observer log poisoned") = true;
            }
            Ok(())
        })];

        cx.update(|cx| {
            // The primary surface is deliberately never declared, so the
            // retained runtime's restore attempt fails.
            install_marker_shell(
                cx,
                "appshell-reopen-restore-failure-tests",
                &runtime,
                false,
                observers,
                Box::new(move |error, _cx| {
                    error_log.lock().expect("error log poisoned").push((
                        error.operation(),
                        error.event().cloned(),
                        error.source_error().to_string(),
                    ));
                }),
            );

            deliver_event(cx, &AppEvent::Reopened);
        });

        assert!(
            *observed_reopened.lock().expect("observer log poisoned"),
            "Reopened still reaches observers after a failed restore",
        );
        let errors = errors.lock().expect("error log poisoned");
        assert_eq!(
            errors.len(),
            1,
            "the failed restore is reported exactly once: {errors:?}"
        );
        assert_eq!(errors[0].0, crate::error::RuntimeOperation::Lifecycle);
        assert!(
            matches!(errors[0].1, Some(AppEvent::Reopened)),
            "the report carries the Reopened lifecycle context: {errors:?}",
        );
        assert_eq!(
            errors[0].2, "surface `primary` is not declared",
            "the report's source is the precise undeclared-surface cause, \
             not an `AppShellError::Startup` wrapper's generic message: {errors:?}",
        );
    }
}
