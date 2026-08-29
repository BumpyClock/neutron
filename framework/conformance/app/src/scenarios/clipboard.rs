use std::io::Write as _;
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use anyhow::Context as _;
#[cfg(feature = "wayland-conformance")]
use neutron_components_app::gpui::KeyDownEvent;
use neutron_components_app::gpui::{App, Entity, Global, Window};
use neutron_components_app::{
    AppDeclaration, AppEvent, AppProxy, DesktopApp, ExitPolicy, LaunchDecision, LaunchSpec,
    ProcessLaunch, Shell as _, Surface, SurfaceKey,
};
use serde_json::json;

use crate::cli::Scenario;
#[cfg(not(feature = "wayland-conformance"))]
use crate::native_window::build_conformance_view;
#[cfg(feature = "wayland-conformance")]
use crate::native_window::build_conformance_view_with_key_down;
use crate::native_window::{ConformanceView, observe_native_window};
use crate::protocol::{CLIPBOARD_EXPECTED_PAYLOAD, Protocol};
use crate::scenarios::{
    self, Handoff, ScenarioOutcome, ScenarioState, catch_run, emit_after_run, lock_or_recover,
    observe_app_event, panic_message,
};

const CLIPBOARD_ACKNOWLEDGEMENT: &[u8] = b"verified\n";
const CLIPBOARD_CANCELLATION: &[u8] = b"cancel\n";

/// Recovered by [`run`] after `AppShell::run` returns: this run's canonical
/// `Protocol`/`ScenarioState` plus the network handles `run` alone needs
/// afterward, to unblock a worker that never received its ready/cancel signal
/// and then to join it. Installed by [`parse`].
static TAIL: Handoff<ClipboardTail> = Handoff::new();
/// The worker `JoinHandle`, installed by `after_open` once it spawns the
/// worker, joined by [`run`] after `AppShell::run` returns.
static WORKER_TAIL: Handoff<JoinHandle<ClipboardWorkerReport>> = Handoff::new();

struct ClipboardTail {
    protocol: Protocol,
    state: ScenarioState,
    worker_signal: mpsc::Sender<ClipboardWorkerSignal>,
    cancellation: Arc<ClipboardCancellation>,
    acknowledgement_address: SocketAddr,
}

/// This run's launch value: the conformance protocol/state plus the network
/// setup the parser constructed, before any platform or GPUI code runs.
/// `networking` is drained exactly once, by `before_primary` — a `Mutex`
/// because `before_primary` only ever sees `&ClipboardLaunch`, never an owned
/// value it could destructure directly.
struct ClipboardLaunch {
    protocol: Protocol,
    state: ScenarioState,
    networking: Mutex<Option<ClipboardSetup>>,
}

struct ClipboardSetup {
    listener: TcpListener,
    worker_waiter: mpsc::Receiver<ClipboardWorkerSignal>,
    worker_signal: mpsc::Sender<ClipboardWorkerSignal>,
    cancellation: Arc<ClipboardCancellation>,
    acknowledgement_address: SocketAddr,
}

struct ClipboardApp;

impl DesktopApp for ClipboardApp {
    fn declaration() -> AppDeclaration {
        AppDeclaration::new(crate::APP_IDENTITY)
            .exit_policy(ExitPolicy::Explicit)
            .on_event(on_event)
            .launch(
                LaunchSpec::new(parse)
                    .before_primary(before_primary)
                    .primary_surface(primary_surface()),
            )
    }
}

#[cfg(not(feature = "wayland-conformance"))]
fn primary_surface() -> Surface<ConformanceView, ClipboardLaunch> {
    Surface::new(SurfaceKey::primary(), build_conformance_view)
        .title("Clipboard")
        .after_open(after_open)
}

#[cfg(feature = "wayland-conformance")]
fn primary_surface() -> Surface<ConformanceView, ClipboardLaunch> {
    Surface::new(SurfaceKey::primary(), build_wayland_view)
        .title("Clipboard")
        .after_open(after_open)
}

#[cfg(feature = "wayland-conformance")]
fn build_wayland_view(
    _args: &ClipboardLaunch,
    window: &mut Window,
    cx: &mut App,
) -> Entity<ConformanceView> {
    build_conformance_view_with_key_down(window, cx, on_key_down)
}

fn parse(process: &ProcessLaunch) -> anyhow::Result<LaunchDecision<ClipboardLaunch>> {
    scenarios::expect_scenario(process, Scenario::Clipboard)?;
    let (protocol, state) = scenarios::parse_core(Scenario::Clipboard, "explicit")?;

    let listener =
        TcpListener::bind("127.0.0.1:0").context("bind clipboard acknowledgement listener")?;
    let acknowledgement_address = listener
        .local_addr()
        .context("obtain clipboard acknowledgement listener address")?;
    let (worker_signal, worker_waiter) = mpsc::channel();
    let cancellation = Arc::new(ClipboardCancellation::default());

    TAIL.install(ClipboardTail {
        protocol: protocol.clone(),
        state: state.clone(),
        worker_signal: worker_signal.clone(),
        cancellation: Arc::clone(&cancellation),
        acknowledgement_address,
    })?;

    Ok(LaunchDecision::Run(ClipboardLaunch {
        protocol,
        state,
        networking: Mutex::new(Some(ClipboardSetup {
            listener,
            worker_waiter,
            worker_signal,
            cancellation,
            acknowledgement_address,
        })),
    }))
}

/// The global every clipboard hook reads: the conformance protocol/state plus
/// the networking values `before_primary` moved out of the launch value's
/// `networking`, and the `remainder` `after_open` still needs to spawn the
/// worker (`after_open` has no `&T`, so it cannot reach the launch value
/// directly).
#[derive(Clone)]
struct ClipboardGlobal {
    protocol: Protocol,
    state: ScenarioState,
    worker_signal: mpsc::Sender<ClipboardWorkerSignal>,
    acknowledgement_address: SocketAddr,
    remainder: Arc<Mutex<Option<ClipboardRemainder>>>,
}

impl Global for ClipboardGlobal {}

/// The part of [`ClipboardSetup`] `before_primary` leaves in the global for
/// `after_open` to take.
struct ClipboardRemainder {
    listener: TcpListener,
    worker_waiter: mpsc::Receiver<ClipboardWorkerSignal>,
    cancellation: Arc<ClipboardCancellation>,
}

fn before_primary(value: &ClipboardLaunch, cx: &mut App) -> anyhow::Result<()> {
    value
        .state
        .emit(&value.protocol, "startup_transaction_started", json!({}));

    // Only the worker's signal/address are needed before the primary surface
    // opens; the listener/waiter/cancellation stay in the remainder for
    // `after_open` to claim once `window_opened` has actually been emitted
    // (the worker must start after that, per the protocol contract).
    let setup = lock_or_recover(&value.networking)
        .take()
        .expect("the launch parser installs clipboard networking before AppShell::run");

    cx.set_global(ClipboardGlobal {
        protocol: value.protocol.clone(),
        state: value.state.clone(),
        worker_signal: setup.worker_signal,
        acknowledgement_address: setup.acknowledgement_address,
        remainder: Arc::new(Mutex::new(Some(ClipboardRemainder {
            listener: setup.listener,
            worker_waiter: setup.worker_waiter,
            cancellation: setup.cancellation,
        }))),
    });
    Ok(())
}

fn on_event(event: &AppEvent, cx: &mut App) -> anyhow::Result<()> {
    // `try_global`, not `global`: a framework failure before `before_primary`
    // ran can still dispatch a shutdown event through this hook, and that
    // earlier failure must not compound into a panic here.
    let Some(global) = cx.try_global::<ClipboardGlobal>().cloned() else {
        return Ok(());
    };
    observe_app_event(&global.protocol, &global.state, event);
    Ok(())
}

fn after_open(_content: &Entity<ConformanceView>, window: &mut Window, cx: &mut App) {
    let global = cx.global::<ClipboardGlobal>().clone();
    observe_native_window(
        window,
        cx,
        &global.protocol,
        &global.state,
        "clipboard",
        "Clipboard",
        after_first_presentation,
    );

    // The worker must start after `window_opened` (just emitted, synchronously,
    // above) and before the acknowledgement it waits for.
    let remainder = lock_or_recover(&global.remainder).take().expect(
        "before_primary installs the clipboard remainder before AppShell opens the primary surface",
    );
    let proxy = cx.app_proxy();
    let worker_protocol = global.protocol.clone();
    let worker_state = global.state.clone();
    let worker = thread::spawn(move || {
        run_clipboard_worker(
            remainder.listener,
            remainder.worker_waiter,
            proxy,
            remainder.cancellation,
            worker_protocol,
            worker_state,
        )
    });
    if let Err(error) = WORKER_TAIL.install(worker) {
        // A singleton primary surface only opens once; a second worker would
        // mean `after_open` ran twice, which the framework does not do.
        fail_clipboard(
            cx,
            &global.protocol,
            &global.state,
            &format!("clipboard worker was already installed: {error:#}"),
        );
    }
}

#[cfg(not(feature = "wayland-conformance"))]
fn after_first_presentation(_window: &mut Window, cx: &mut App) {
    publish_clipboard_ready(cx);
}

#[cfg(feature = "wayland-conformance")]
fn after_first_presentation(window: &mut Window, cx: &mut App) {
    request_wayland_clipboard_input(window, cx);
}

#[cfg(feature = "wayland-conformance")]
fn on_key_down(event: &KeyDownEvent, _window: &mut Window, cx: &mut App) {
    let global = cx.global::<ClipboardGlobal>().clone();
    if event.is_held || event.keystroke.key != "a" {
        fail_clipboard(
            cx,
            &global.protocol,
            &global.state,
            "Wayland conformance delivered an unexpected key-down event",
        );
        return;
    }
    global.state.emit(
        &global.protocol,
        "wayland_key_down_observed",
        json!({"key": "a", "source": "weston_test"}),
    );
    publish_clipboard_ready(cx);
}

#[cfg(feature = "wayland-conformance")]
fn request_wayland_clipboard_input(window: &mut Window, cx: &mut App) {
    let global = cx.global::<ClipboardGlobal>().clone();
    let input_result = window.request_wayland_conformance_key_press();
    global.state.emit(
        &global.protocol,
        "wayland_input_requested",
        json!({"protocol": "weston_test", "key": "a"}),
    );
    let proxy = cx.app_proxy();
    window
        .spawn(cx, async move |async_cx| match input_result.await {
            Ok(Ok(())) => {
                let update_protocol = global.protocol.clone();
                let update_state = global.state.clone();
                if let Err(error) = async_cx.update(move |_, _| {
                    update_state.emit(
                        &update_protocol,
                        "wayland_input_completed",
                        json!({"result": "key_press_delivered"}),
                    );
                }) {
                    global.state.record_failure(format!(
                        "could not record completed Wayland conformance input: {error:#}"
                    ));
                    let _ = proxy.dispatch(|cx| cx.request_quit());
                }
            }
            Ok(Err(error)) => {
                let message = format!("Wayland conformance input failed: {error:#}");
                if async_cx
                    .update(move |_, cx| {
                        fail_clipboard(cx, &global.protocol, &global.state, &message);
                    })
                    .is_err()
                {
                    let _ = proxy.dispatch(|cx| cx.request_quit());
                }
            }
            Err(_) => {
                if async_cx
                    .update(move |_, cx| {
                        fail_clipboard(
                            cx,
                            &global.protocol,
                            &global.state,
                            "Wayland conformance input completion was cancelled",
                        );
                    })
                    .is_err()
                {
                    let _ = proxy.dispatch(|cx| cx.request_quit());
                }
            }
        })
        .detach();
}

fn publish_clipboard_ready(cx: &mut App) {
    let global = cx.global::<ClipboardGlobal>().clone();
    cx.write_to_clipboard(neutron_components_app::gpui::ClipboardItem::new_string(
        CLIPBOARD_EXPECTED_PAYLOAD.to_owned(),
    ));
    if let Err(error) = global.protocol.emit(
        "clipboard_ready",
        json!({
            "expected_payload": CLIPBOARD_EXPECTED_PAYLOAD,
            "ack_address": global.acknowledgement_address.to_string(),
        }),
    ) {
        fail_clipboard(
            cx,
            &global.protocol,
            &global.state,
            &format!("could not write clipboard_ready protocol record: {error:#}"),
        );
        return;
    }

    global.state.mark_clipboard_ready();
    if global
        .worker_signal
        .send(ClipboardWorkerSignal::Ready)
        .is_err()
    {
        fail_clipboard(
            cx,
            &global.protocol,
            &global.state,
            "clipboard worker stopped before external verification began",
        );
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ClipboardWorkerSignal {
    Ready,
    Cancel,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ClipboardWorkerReport {
    AcknowledgementDispatched,
    DispatchRejected,
    Cancelled,
    Failed,
}

impl ClipboardWorkerReport {
    const fn as_str(self) -> &'static str {
        match self {
            Self::AcknowledgementDispatched => "acknowledgement_dispatched",
            Self::DispatchRejected => "dispatch_rejected",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
        }
    }
}

#[derive(Default)]
struct ClipboardCancellation {
    cancelled: AtomicBool,
    active_connection: Mutex<Option<TcpStream>>,
}

impl ClipboardCancellation {
    fn install_connection(&self, connection: TcpStream) {
        let mut active_connection = lock_or_recover(&self.active_connection);
        if self.cancelled.load(Ordering::Acquire) {
            let _ = connection.shutdown(Shutdown::Both);
            return;
        }
        *active_connection = Some(connection);
        if self.cancelled.load(Ordering::Acquire)
            && let Some(connection) = active_connection.as_ref()
        {
            let _ = connection.shutdown(Shutdown::Both);
        }
    }

    fn clear_connection(&self) {
        lock_or_recover(&self.active_connection).take();
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        if let Some(connection) = lock_or_recover(&self.active_connection).as_ref() {
            let _ = connection.shutdown(Shutdown::Both);
        }
    }
}

fn run_clipboard_worker(
    listener: TcpListener,
    worker_waiter: mpsc::Receiver<ClipboardWorkerSignal>,
    proxy: AppProxy,
    cancellation: Arc<ClipboardCancellation>,
    protocol: Protocol,
    state: ScenarioState,
) -> ClipboardWorkerReport {
    state.emit(&protocol, "clipboard_worker_started", json!({}));
    match worker_waiter.recv() {
        Ok(ClipboardWorkerSignal::Ready) => {}
        Ok(ClipboardWorkerSignal::Cancel) | Err(_) => return ClipboardWorkerReport::Cancelled,
    }

    let (mut connection, _) = match listener.accept() {
        Ok(connection) => connection,
        Err(error) => {
            let failure = format!("clipboard acknowledgement listener failed: {error}");
            state.record_failure(failure.clone());
            state.emit(
                &protocol,
                "clipboard_acknowledgement_rejected",
                json!({"reason": "listener_error"}),
            );
            dispatch_clipboard_failure(&proxy, &protocol, &state, failure);
            return ClipboardWorkerReport::Failed;
        }
    };
    let cancellation_connection = match connection.try_clone() {
        Ok(connection) => connection,
        Err(error) => {
            let failure = format!("clone clipboard acknowledgement connection: {error}");
            state.record_failure(failure.clone());
            state.emit(
                &protocol,
                "clipboard_acknowledgement_rejected",
                json!({"reason": "connection_clone_error"}),
            );
            dispatch_clipboard_failure(&proxy, &protocol, &state, failure);
            return ClipboardWorkerReport::Failed;
        }
    };
    cancellation.install_connection(cancellation_connection);
    let acknowledgement = read_clipboard_acknowledgement(&mut connection);
    cancellation.clear_connection();
    let acknowledgement = match acknowledgement {
        Ok(acknowledgement) => acknowledgement,
        Err(error) => {
            let failure = format!("read clipboard acknowledgement: {error}");
            state.record_failure(failure.clone());
            state.emit(
                &protocol,
                "clipboard_acknowledgement_rejected",
                json!({"reason": "read_error"}),
            );
            dispatch_clipboard_failure(&proxy, &protocol, &state, failure);
            return ClipboardWorkerReport::Failed;
        }
    };
    if is_internal_clipboard_cancellation(&acknowledgement, &cancellation) {
        return ClipboardWorkerReport::Cancelled;
    }
    if acknowledgement != CLIPBOARD_ACKNOWLEDGEMENT {
        let failure = "clipboard acknowledgement token was not exactly verified\\n".to_owned();
        state.record_failure(failure.clone());
        state.emit(
            &protocol,
            "clipboard_acknowledgement_rejected",
            json!({"reason": "unexpected_token"}),
        );
        dispatch_clipboard_failure(&proxy, &protocol, &state, failure);
        return ClipboardWorkerReport::Failed;
    }
    if !state.claim_clipboard_acknowledgement() {
        let failure =
            "clipboard acknowledgement arrived before readiness or more than once".to_owned();
        state.record_failure(failure.clone());
        state.emit(
            &protocol,
            "clipboard_acknowledgement_rejected",
            json!({"reason": "invalid_state"}),
        );
        dispatch_clipboard_failure(&proxy, &protocol, &state, failure);
        return ClipboardWorkerReport::Failed;
    }
    state.emit(
        &protocol,
        "clipboard_acknowledged",
        json!({"acknowledgement": "verified"}),
    );

    let callback_protocol = protocol;
    let callback_state = state.clone();
    match proxy.dispatch(move |cx| {
        if !callback_state.claim_clipboard_quit() {
            fail_clipboard(
                cx,
                &callback_protocol,
                &callback_state,
                "clipboard quit callback ran without a verified acknowledgement",
            );
            return;
        }
        callback_state.emit(
            &callback_protocol,
            "quit_requested",
            json!({"source": "external_clipboard_acknowledgement"}),
        );
        cx.request_quit();
    }) {
        Ok(()) => ClipboardWorkerReport::AcknowledgementDispatched,
        Err(error) => {
            state.record_failure(format!(
                "clipboard AppProxy dispatch was rejected after acknowledgement: {error}"
            ));
            ClipboardWorkerReport::DispatchRejected
        }
    }
}

fn read_clipboard_acknowledgement(mut reader: impl std::io::Read) -> std::io::Result<Vec<u8>> {
    let mut acknowledgement = Vec::with_capacity(CLIPBOARD_ACKNOWLEDGEMENT.len());
    let mut byte = [0_u8; 1];
    while acknowledgement.len() < CLIPBOARD_ACKNOWLEDGEMENT.len() {
        if reader.read(&mut byte)? == 0 {
            break;
        }
        acknowledgement.push(byte[0]);
        if byte[0] == b'\n' {
            break;
        }
    }
    Ok(acknowledgement)
}

fn is_internal_clipboard_cancellation(
    acknowledgement: &[u8],
    cancellation: &ClipboardCancellation,
) -> bool {
    acknowledgement == CLIPBOARD_CANCELLATION && cancellation.is_cancelled()
}

fn dispatch_clipboard_failure(
    proxy: &AppProxy,
    protocol: &Protocol,
    state: &ScenarioState,
    failure: String,
) {
    let callback_protocol = protocol.clone();
    let callback_state = state.clone();
    let callback_failure = failure.clone();
    if let Err(error) = proxy.dispatch(move |cx| {
        fail_clipboard(cx, &callback_protocol, &callback_state, &callback_failure);
    }) {
        state.record_failure(format!(
            "{failure}; clipboard failure quit dispatch was rejected: {error}"
        ));
    }
}

fn cancel_clipboard_worker(
    acknowledgement_address: SocketAddr,
    cancellation: &ClipboardCancellation,
) {
    cancellation.cancel();
    let Ok(mut connection) = TcpStream::connect(acknowledgement_address) else {
        return;
    };
    let _ = connection.write_all(CLIPBOARD_CANCELLATION);
    let _ = connection.shutdown(Shutdown::Write);
}

fn fail_clipboard(cx: &mut App, protocol: &Protocol, state: &ScenarioState, failure: &str) {
    state.record_failure(failure.to_owned());
    state.emit(protocol, "clipboard_failed", json!({"reason": failure}));
    cx.request_quit();
}

fn join_clipboard_worker(
    protocol: &Protocol,
    state: &ScenarioState,
) -> anyhow::Result<ClipboardWorkerReport> {
    let worker = WORKER_TAIL
        .take()
        .context("clipboard worker was never started")?;
    let report = worker
        .join()
        .map_err(|panic| anyhow::anyhow!("clipboard worker panicked: {}", panic_message(panic)))?;
    state.emit(
        protocol,
        "clipboard_worker_joined",
        json!({"result": report.as_str()}),
    );
    Ok(report)
}

pub(crate) fn run() -> anyhow::Result<ScenarioOutcome> {
    let (tail, result) = scenarios::recover_tail(&TAIL, catch_run::<ClipboardApp>())?;

    // Unblock the worker's network wait unconditionally, whether
    // `AppShell::run` returned or panicked, before either tail below joins
    // it and writes a terminal record: a panic must never leave the worker
    // thread running or its listener/socket open past this scenario's
    // terminal output.
    let _ = tail.worker_signal.send(ClipboardWorkerSignal::Cancel);
    cancel_clipboard_worker(tail.acknowledgement_address, &tail.cancellation);

    let result = match result {
        Ok(result) => result,
        Err(panic) => {
            let cleanup = join_clipboard_worker(&tail.protocol, &tail.state).map(|_| ());
            return scenarios::finish_panicked_after_cleanup(&tail.protocol, panic, cleanup);
        }
    };

    let after_run_result = emit_after_run(&tail.protocol, &result);
    let worker_report = join_clipboard_worker(&tail.protocol, &tail.state);

    let outcome = (|| -> anyhow::Result<ScenarioOutcome> {
        if let Err(error) = result {
            after_run_result?;
            if let Err(worker_error) = &worker_report {
                tail.state
                    .record_failure(format!("clipboard worker could not join: {worker_error:#}"));
            }
            anyhow::bail!("clipboard scenario returned AppShell error: {error:#}");
        }
        after_run_result?;
        let worker_report = worker_report?;
        if worker_report != ClipboardWorkerReport::AcknowledgementDispatched {
            tail.state.record_failure(format!(
                "clipboard worker did not dispatch the verified acknowledgement: {}",
                worker_report.as_str()
            ));
        }
        if !tail.state.clipboard_acknowledged() {
            tail.state.record_failure(
                "clipboard scenario ended without external acknowledgement".to_owned(),
            );
        }
        if !tail.state.clipboard_quit_requested() {
            tail.state.record_failure(
                "clipboard scenario ended before its acknowledgement quit callback".to_owned(),
            );
        }
        if let Some(failure) = tail.state.failure() {
            anyhow::bail!("clipboard scenario failed: {failure}");
        }
        Ok(ScenarioOutcome::Passed)
    })();

    scenarios::finish(&tail.protocol, outcome)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipboard_acknowledgement_does_not_wait_for_connection_close() {
        use std::time::Duration;

        let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
        let address = listener
            .local_addr()
            .expect("listener should report its address");
        let (written_sender, written_waiter) = mpsc::channel();
        let (release_sender, release_waiter) = mpsc::channel();
        let writer = thread::spawn(move || {
            let mut connection = TcpStream::connect(address).expect("writer should connect");
            connection
                .write_all(CLIPBOARD_ACKNOWLEDGEMENT)
                .expect("writer should send acknowledgement");
            written_sender
                .send(())
                .expect("test should wait for acknowledgement bytes");
            release_waiter
                .recv()
                .expect("test should release the writer connection");
        });
        let (mut connection, _) = listener.accept().expect("listener should accept writer");
        written_waiter
            .recv()
            .expect("writer should send acknowledgement before reading");
        connection
            .set_read_timeout(Some(Duration::from_secs(1)))
            .expect("connection should accept a test read timeout");
        let acknowledgement = read_clipboard_acknowledgement(&mut connection);
        release_sender
            .send(())
            .expect("writer should still be waiting with its write side open");
        writer.join().expect("writer should not panic");

        assert_eq!(
            acknowledgement.expect("reader should stop at the acknowledgement newline"),
            CLIPBOARD_ACKNOWLEDGEMENT
        );
    }

    #[test]
    fn clipboard_cancellation_interrupts_an_active_connection() {
        use std::time::Duration;

        let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
        let address = listener
            .local_addr()
            .expect("listener should report its address");
        let client = TcpStream::connect(address).expect("client should connect");
        let (mut connection, _) = listener.accept().expect("listener should accept client");
        let cancellation = Arc::new(ClipboardCancellation::default());
        cancellation.install_connection(
            connection
                .try_clone()
                .expect("connection should be cloneable for cancellation"),
        );

        let reader_cancellation = Arc::clone(&cancellation);
        let (result_sender, result_waiter) = mpsc::channel();
        let reader = thread::spawn(move || {
            let result = read_clipboard_acknowledgement(&mut connection);
            reader_cancellation.clear_connection();
            result_sender
                .send(result)
                .expect("test should receive reader completion");
        });
        cancellation.cancel();
        let completion = result_waiter.recv_timeout(Duration::from_secs(1));
        drop(client);
        reader.join().expect("reader should not panic");

        assert!(
            completion.is_ok(),
            "cancellation must interrupt a worker blocked on an accepted connection"
        );
    }

    #[test]
    fn clipboard_cancellation_token_requires_internal_cancellation() {
        let cancellation = ClipboardCancellation::default();

        assert!(!is_internal_clipboard_cancellation(
            CLIPBOARD_CANCELLATION,
            &cancellation
        ));
        cancellation.cancel();
        assert!(is_internal_clipboard_cancellation(
            CLIPBOARD_CANCELLATION,
            &cancellation
        ));
    }

    #[test]
    fn parse_rejects_a_mismatched_scenario_selection() {
        let process = ProcessLaunch::new(vec!["--scenario".into(), "lifecycle-clean".into()], None);

        assert!(parse(&process).is_err());
    }
}
