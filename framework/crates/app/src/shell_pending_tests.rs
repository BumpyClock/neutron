use super::*;
use crate::error::AppClosed;
use crate::handles::{AppShellExt as _, ShellState};
use crate::lifecycle::OpenRequest;
use std::sync::atomic::{AtomicUsize, Ordering};

fn identity() -> IdentityRef {
    IdentityRef {
        app_id: "com.example.pending-events",
        display_name: "Pending Events",
        data_namespace: "pending-events",
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

fn pending_event(label: &str) -> AppEvent {
    AppEvent::OpenRequested(OpenRequest {
        urls: vec![label.to_owned()],
    })
}

fn pending_label(event: &AppEvent) -> Option<&str> {
    let AppEvent::OpenRequested(request) = event else {
        return None;
    };
    request.urls.first().map(String::as_str)
}

#[derive(Default)]
struct RecordingPlugin {
    log: Arc<Mutex<Vec<String>>>,
}

impl crate::plugin::sealed::Sealed for RecordingPlugin {}

impl AppPlugin for RecordingPlugin {
    fn init(&mut self, _cx: &mut App, _shell: &ShellSeed) -> Result<(), AppShellError> {
        Ok(())
    }

    fn on_event(&mut self, event: &AppEvent, _cx: &mut App) -> Result<(), AppShellError> {
        self.log
            .lock()
            .expect("pending-event test log poisoned")
            .push(format!("plugin:{}", event.name()));
        Ok(())
    }

    fn shutdown(&mut self, _cx: &mut App) {
        self.log
            .lock()
            .expect("pending-event test log poisoned")
            .push("plugin:shutdown".to_owned());
    }
}

fn test_boot(
    pending: Arc<PendingEvents>,
    log: Arc<Mutex<Vec<String>>>,
    quit_on_pending: Option<usize>,
    error_on_pending: Option<usize>,
) -> Boot {
    let pending_count = Arc::new(AtomicUsize::new(0));
    let handler_count = Arc::clone(&pending_count);
    let handler_log = Arc::clone(&log);
    let error_log = Arc::clone(&log);

    Boot {
        app_info: AppInfo::new(
            identity(),
            AppPaths::new("pending-event-tests", PathLayout::PlatformDefault)
                .expect("test paths resolve"),
            PlatformCapabilities::detect(),
        ),
        liveness: Liveness::new(ExitPolicy::Explicit, InitialActivation::Regular),
        initial_activation: InitialActivation::Regular,
        plugins: vec![Box::new(RecordingPlugin {
            log: Arc::clone(&log),
        })],
        handlers: vec![Box::new(move |event, cx| {
            let Some(label) = pending_label(event) else {
                handler_log
                    .lock()
                    .expect("pending-event test log poisoned")
                    .push(format!("handler:{}", event.name()));
                return Ok(());
            };

            let ordinal = handler_count.fetch_add(1, Ordering::SeqCst) + 1;
            handler_log
                .lock()
                .expect("pending-event test log poisoned")
                .push(format!("pending:{label}"));
            if quit_on_pending == Some(ordinal) {
                handles::request_quit(cx);
            }
            if error_on_pending == Some(ordinal) {
                return Err(anyhow::anyhow!("pending event {ordinal} failed").into());
            }
            Ok(())
        })],
        pending,
        state: HashMap::new(),
        launch: LaunchRequest::default(),
        start: None,
        error_reporter: Box::new(move |error, _cx| {
            error_log
                .lock()
                .expect("pending-event test log poisoned")
                .push(format!(
                    "error:{}",
                    error.event().unwrap_or("unknown lifecycle event")
                ));
        }),
    }
}

#[derive(Debug, PartialEq, Eq)]
struct RunResult {
    pending_events: Vec<String>,
    shutdown_requested: usize,
    will_exit: usize,
    plugin_shutdown: usize,
    errors: Vec<String>,
    queue_len: usize,
    proxy_closed: bool,
    drain_phase: bool,
    activation_phase: bool,
}

fn run_pending_events(
    cx: &mut gpui::TestAppContext,
    labels: &[&str],
    quit_on_pending: Option<usize>,
    error_on_pending: Option<usize>,
) -> (Arc<PendingEvents>, Arc<Mutex<Vec<String>>>, RunResult) {
    let pending = Arc::new(PendingEvents::default());
    for label in labels {
        pending
            .push(pending_event(label))
            .expect("pre-ready events are accepted");
    }
    let log = Arc::new(Mutex::new(Vec::new()));
    let boot = test_boot(
        Arc::clone(&pending),
        Arc::clone(&log),
        quit_on_pending,
        error_on_pending,
    );

    cx.update(|app| boot.run(app))
        .expect("pending-event shutdown is not a startup failure");

    let (shutdown_boundary, proxy_closed, drain_phase, activation_phase) = cx.update(|app| {
        let state = app.global::<ShellState>();
        (
            state.is_shutdown_requested(),
            app.app_proxy().is_closed(),
            state.phases().is_complete(Phase::DrainQueue),
            state.phases().is_complete(Phase::Activation),
        )
    });
    if shutdown_boundary {
        // Boot::run is invoked directly in this unit seam, outside Platform::run.
        // End the test app to invoke the registered platform-quit observers.
        cx.quit();
    }

    let entries = log.lock().expect("pending-event test log poisoned").clone();
    let pending_events = entries
        .iter()
        .filter_map(|entry| entry.strip_prefix("pending:").map(str::to_owned))
        .collect();
    let errors = entries
        .iter()
        .filter(|entry| entry.starts_with("error:"))
        .cloned()
        .collect();
    let queue_len = pending.len();
    let shutdown_requested = entries
        .iter()
        .filter(|entry| entry.as_str() == "handler:shutdown_requested")
        .count();
    let will_exit = entries
        .iter()
        .filter(|entry| entry.as_str() == "handler:will_exit")
        .count();
    let plugin_shutdown = entries
        .iter()
        .filter(|entry| entry.as_str() == "plugin:shutdown")
        .count();

    (
        pending,
        log,
        RunResult {
            pending_events,
            shutdown_requested,
            will_exit,
            plugin_shutdown,
            errors,
            queue_len,
            proxy_closed,
            drain_phase,
            activation_phase,
        },
    )
}

#[gpui::test]
fn first_pending_event_shutdown_stops_delivery(cx: &mut gpui::TestAppContext) {
    let (_, _, result) = run_pending_events(cx, &["first", "second"], Some(1), None);

    assert_eq!(
        result,
        RunResult {
            pending_events: vec!["first".to_owned()],
            shutdown_requested: 1,
            will_exit: 1,
            plugin_shutdown: 1,
            errors: Vec::new(),
            queue_len: 0,
            proxy_closed: true,
            drain_phase: false,
            activation_phase: false,
        }
    );
}

#[gpui::test]
fn later_pending_event_shutdown_preserves_fifo_prefix(cx: &mut gpui::TestAppContext) {
    let (_, _, result) = run_pending_events(cx, &["first", "second", "third"], Some(2), None);

    assert_eq!(result.pending_events, ["first", "second"]);
    assert_eq!(result.shutdown_requested, 1);
    assert_eq!(result.will_exit, 1);
    assert_eq!(result.plugin_shutdown, 1);
    assert_eq!(result.queue_len, 0);
    assert!(!result.drain_phase);
    assert!(!result.activation_phase);
}

#[gpui::test]
fn event_racing_after_shutdown_is_rejected_without_queueing(cx: &mut gpui::TestAppContext) {
    let (pending, log, result) = run_pending_events(cx, &["first"], Some(1), None);
    assert_eq!(result.pending_events, ["first"]);

    let late_pending = Arc::clone(&pending);
    let dispatch = std::thread::spawn(move || late_pending.push(pending_event("late")))
        .join()
        .expect("late pending-event source panicked");

    assert_eq!(dispatch, Err(AppClosed));
    assert!(pending.is_empty());
    assert!(
        !log.lock()
            .expect("pending-event test log poisoned")
            .iter()
            .any(|entry| entry == "pending:late")
    );
}

#[gpui::test]
fn event_after_started_shutdown_is_rejected_before_proxy_publication(
    cx: &mut gpui::TestAppContext,
) {
    let pending = Arc::new(PendingEvents::default());
    pending
        .push(pending_event("early"))
        .expect("pre-ready event is accepted");
    let log = Arc::new(Mutex::new(Vec::new()));
    let mut boot = test_boot(Arc::clone(&pending), Arc::clone(&log), None, None);
    let handler_log = Arc::clone(&log);
    boot.handlers = vec![Box::new(move |event, cx| {
        handler_log
            .lock()
            .expect("pending-event test log poisoned")
            .push(format!("handler:{}", event.name()));
        if matches!(event, AppEvent::Started(_)) {
            handles::request_quit(cx);
        }
        Ok(())
    })];

    cx.update(|app| boot.run(app))
        .expect("Started-triggered quit is not a startup failure");
    assert!(pending.proxy.get().is_none());
    assert!(pending.is_empty());
    assert_eq!(pending.push(pending_event("late")), Err(AppClosed));
    cx.quit();

    let entries = log.lock().expect("pending-event test log poisoned");
    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry.as_str() == "handler:shutdown_requested")
            .count(),
        1
    );
    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry.as_str() == "handler:will_exit")
            .count(),
        1
    );
    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry.as_str() == "plugin:shutdown")
            .count(),
        1
    );
    assert!(!entries.iter().any(|entry| entry == "pending:early"));
}

#[gpui::test]
fn normal_pending_drain_is_fifo_and_activates(cx: &mut gpui::TestAppContext) {
    let (_, _, result) = run_pending_events(cx, &["first", "second", "third"], None, None);

    assert_eq!(result.pending_events, ["first", "second", "third"]);
    assert_eq!(result.shutdown_requested, 0);
    assert_eq!(result.will_exit, 0);
    assert_eq!(result.plugin_shutdown, 0);
    assert_eq!(result.queue_len, 0);
    assert!(!result.proxy_closed);
    assert!(result.drain_phase);
    assert!(result.activation_phase);
}

#[gpui::test]
fn handler_error_is_reported_before_later_shutdown_stops_queue(cx: &mut gpui::TestAppContext) {
    let (_, _, result) = run_pending_events(cx, &["first", "second", "third"], Some(2), Some(1));

    assert_eq!(result.pending_events, ["first", "second"]);
    assert_eq!(result.errors, ["error:open_requested"]);
    assert_eq!(result.shutdown_requested, 1);
    assert_eq!(result.will_exit, 1);
    assert_eq!(result.plugin_shutdown, 1);
    assert_eq!(result.queue_len, 0);
    assert!(!result.drain_phase);
    assert!(!result.activation_phase);
}
