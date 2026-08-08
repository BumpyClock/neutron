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

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use gpui::{App, MainThreadPoster};
use gpui_component_manifest::schema::IdentityRef;
use gpui_component_storage::AppPaths;

use crate::capabilities::PlatformCapabilities;
use crate::error::{AppClosed, RuntimeError};
use crate::lifecycle::{AppEvent, OpenRequest, ShutdownReason};
use crate::liveness::{Liveness, ShellHold};
use crate::phases::PhaseTracker;
use crate::plugin::{AppPlugin, EventHandler};

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

/// Main-thread shell state. A GPUI [`gpui::Global`]; intentionally not `Send`.
pub struct ShellState {
    app_info: AppInfo,
    proxy: AppProxy,
    liveness: Liveness,
    plugins: Vec<Box<dyn AppPlugin>>,
    handlers: Vec<EventHandler>,
    phases: PhaseTracker,
    pending: Arc<PendingEvents>,
    state: HashMap<TypeId, Box<dyn Any>>,
    subscriptions: Vec<gpui::Subscription>,
    readiness: Readiness,
    error_reporter: Option<ErrorReporter>,
    reporting_error: bool,
    /// Re-entrancy guard for `deliver_event`.
    delivery: crate::lifecycle::ReentrantQueue,
    /// The reason to attribute if the next idle evaluation triggers an exit.
    /// Set by the window-close observer (which fires before a window-manager
    /// plugin drops the window's hold), consumed by `evaluate_exit`.
    pending_exit_reason: Option<ShutdownReason>,
    shutdown_requested: bool,
    will_exit_done: bool,
}

impl gpui::Global for ShellState {}

impl ShellState {
    /// Number of live plugins (for diagnostics/tests).
    pub fn plugin_count(&self) -> usize {
        self.plugins.len()
    }

    /// The recorded phase progress.
    pub fn phases(&self) -> &PhaseTracker {
        &self.phases
    }

    pub(crate) fn is_shutdown_requested(&self) -> bool {
        self.shutdown_requested
    }

    pub(crate) fn record_phase(&mut self, phase: crate::phases::Phase) {
        self.phases.complete(phase);
    }

    /// Move plugins out for a `&mut App` re-entrant call (init).
    pub(crate) fn take_plugins(&mut self) -> Vec<Box<dyn AppPlugin>> {
        std::mem::take(&mut self.plugins)
    }

    /// Restore plugins taken by [`ShellState::take_plugins`].
    pub(crate) fn restore_plugins(&mut self, plugins: Vec<Box<dyn AppPlugin>>) {
        self.plugins = plugins;
    }
}

/// A lightweight, cloneable handle for taking liveness leases and driving quit.
#[derive(Clone)]
pub struct ShellHandle {
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
    pub fn proxy(&self) -> AppProxy {
        self.proxy.clone()
    }
}

/// Extension trait exposing shell services on the raw `gpui::App`.
pub trait AppShellExt {
    /// Immutable app identity/paths/capabilities.
    fn app_info(&self) -> &AppInfo;
    /// A cross-thread dispatch proxy.
    fn app_proxy(&self) -> AppProxy;
    /// A handle for liveness leases and quit.
    fn shell(&self) -> ShellHandle;
    /// Typed pre-platform state registered via `AppShellBuilder::state`.
    fn app_state<T: 'static>(&self) -> Option<&T>;
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

    fn app_state<T: 'static>(&self) -> Option<&T> {
        self.global::<ShellState>()
            .state
            .get(&TypeId::of::<T>())
            .and_then(|b| b.downcast_ref::<T>())
    }

    fn request_quit(&mut self) {
        request_quit(self);
    }
}

/// Install the shell global. Wires the cross-thread proxy to the app's
/// [`MainThreadPoster`]; no poll loop — posts wake the main run loop directly.
///
/// Called during the `CoreServices` phase with the constructed `AppInfo` and the
/// plugins/handlers/state accumulated by the builder.
#[allow(clippy::too_many_arguments)]
pub(crate) fn install(
    cx: &mut App,
    app_info: AppInfo,
    liveness: Liveness,
    plugins: Vec<Box<dyn AppPlugin>>,
    handlers: Vec<EventHandler>,
    pending: Arc<PendingEvents>,
    state: HashMap<TypeId, Box<dyn Any>>,
    phases: PhaseTracker,
    error_reporter: ErrorReporter,
) -> AppProxy {
    let proxy = AppProxy::new(cx);

    cx.set_global(ShellState {
        app_info,
        proxy: proxy.clone(),
        liveness,
        plugins,
        handlers,
        phases,
        pending,
        state,
        subscriptions: Vec::new(),
        readiness: Readiness::Starting {
            deferred_quit: None,
        },
        error_reporter: Some(error_reporter),
        reporting_error: false,
        delivery: crate::lifecycle::ReentrantQueue::new(),
        pending_exit_reason: None,
        shutdown_requested: false,
        will_exit_done: false,
    });
    proxy
}

/// Register lifecycle observers (window-closed, app-quit) after readiness.
pub(crate) fn register_observers(cx: &mut App) {
    let window_closed = cx.on_window_closed(|app| {
        if app.windows().is_empty() {
            // Record the reason now. The actual exit may only happen on a later
            // tick, after a window-manager plugin's reconcile drops the closed
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

/// Deliver `event` to every plugin then every app event handler.
///
/// Re-entrancy-safe: a delivery moves plugins/handlers out of the global for the
/// pass, so a callback that itself delivers an event (e.g. `request_quit()` from
/// a `Started` handler emitting `ShutdownRequested`) would otherwise hit empty
/// subscriber lists. Such nested events are buffered and drained after the
/// current pass, in order (see [`crate::lifecycle::ReentrantQueue`]).
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
        deliver_one(cx, &current);
        match cx.global_mut::<ShellState>().delivery.take_next() {
            Some(next) => current = next,
            None => break,
        }
    }
}

/// Deliver a single event to plugins then handlers, moving them out of the
/// global for the call (so they can receive `&mut App`) and restoring them.
/// Handler errors are logged, not fatal.
fn deliver_one(cx: &mut App, event: &AppEvent) {
    let (mut plugins, mut handlers) = {
        let st = cx.global_mut::<ShellState>();
        (
            std::mem::take(&mut st.plugins),
            std::mem::take(&mut st.handlers),
        )
    };

    let mut errors = Vec::new();
    for plugin in &mut plugins {
        if let Err(err) = plugin.on_event(event, cx) {
            errors.push(err);
        }
    }
    for handler in &mut handlers {
        if let Err(err) = handler(event, cx) {
            errors.push(err);
        }
    }

    let st = cx.global_mut::<ShellState>();
    st.plugins = plugins;
    st.handlers = handlers;
    for error in errors {
        report_error(cx, RuntimeError::lifecycle(event.name(), error));
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
/// between this observer and a window-manager plugin's hold-drop: the reason is
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
    shutdown_plugins(cx);
}

/// Shut plugins down in reverse init order.
fn shutdown_plugins(cx: &mut App) {
    let mut plugins = std::mem::take(&mut cx.global_mut::<ShellState>().plugins);
    for plugin in plugins.iter_mut().rev() {
        plugin.shutdown(cx);
    }
    // Plugins are done; they are not restored.
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
}
