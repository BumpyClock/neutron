//! Headless bootstrap test: drive the shell through the injected headless runner
//! on the process main thread and assert the lifecycle fires end-to-end.
//!
//! Uses `harness = false` (see this crate's `Cargo.toml`): GPUI panics unless
//! its `App` is constructed on the main thread, which the default test harness
//! (worker threads) cannot provide.
//!
//! Child processes request an orderly shell quit and assert that
//! `AppShellBuilder::run` returns normally. The success and fatal-startup paths
//! both assert initialized plugins shut down exactly once in reverse order after
//! `run` returns. Their post-return sentinels prevent a platform `exit(0)` from
//! passing the test.
//! A parent-side timeout only bounds a hung child regression; successful children
//! still terminate solely by returning normally from `AppShellBuilder::run`.
//!
//! This requires the Stage 1 GPUI normal-return contract and runs against the
//! canonical locked GPUI revision. Disposable sibling overrides are development
//! aids, not acceptance evidence.

use std::io::ErrorKind;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui_component_app::gpui::App;
use gpui_component_app::plugin::test_support::recording_plugin;
use gpui_component_app::prelude::*;
use gpui_component_manifest::schema::IdentityRef;
use wait_timeout::ChildExt;

const CHILD_TIMEOUT: Duration = Duration::from_secs(30);
const SUCCESS_CHILD: &str = "--success-child";
const STARTUP_FAILURE_CHILD: &str = "--startup-failure-child";
const SUCCESS_RETURNED: &str = "APPSHELL_HEADLESS_SUCCESS_RETURNED";
const STARTUP_FAILURE_RETURNED: &str = "APPSHELL_HEADLESS_STARTUP_FAILURE_RETURNED";

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

fn assert_plugin_shutdown_order(events: &Arc<Mutex<Vec<String>>>) {
    assert_eq!(
        *events.lock().expect("recording plugin events poisoned"),
        vec![
            "first:init".to_string(),
            "second:init".to_string(),
            "second:shutdown".to_string(),
            "first:shutdown".to_string(),
        ]
    );
}

fn main() {
    if std::env::args().any(|arg| arg == SUCCESS_CHILD) {
        assert_successful_run_returns();
        return;
    }
    if std::env::args().any(|arg| arg == STARTUP_FAILURE_CHILD) {
        assert_startup_failure_returns();
        return;
    }

    assert_child_returns(SUCCESS_CHILD, SUCCESS_RETURNED);
    assert_child_returns(STARTUP_FAILURE_CHILD, STARTUP_FAILURE_RETURNED);
}

fn assert_child_returns(role: &str, sentinel: &str) {
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
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.lines().any(|line| line == sentinel),
        "headless child {role} exited without reaching its post-run sentinel {sentinel}\nstdout:\n{stdout}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
}

fn assert_startup_failure_returns() {
    let plugin_events = Arc::new(Mutex::new(Vec::new()));
    let result = AppShell::builder(test_identity())
        .runner(PlatformRunner::headless())
        .initial_activation(InitialActivation::Passive)
        .exit_policy(ExitPolicy::Explicit)
        .plugin(recording_plugin("first", Arc::clone(&plugin_events)))
        .plugin(recording_plugin("second", Arc::clone(&plugin_events)))
        .start(|_, _| Err(anyhow::anyhow!("expected startup failure")))
        .run();

    assert!(matches!(result, Err(AppShellError::Startup(_))));
    assert_plugin_shutdown_order(&plugin_events);
    println!("{STARTUP_FAILURE_RETURNED}");
}

fn assert_successful_run_returns() {
    let start_completed = Arc::new(AtomicBool::new(false));
    let started_observed = Arc::new(AtomicBool::new(false));
    let will_exit_observed = Arc::new(AtomicBool::new(false));
    let runtime_error_reported = Arc::new(AtomicBool::new(false));
    let plugin_events = Arc::new(Mutex::new(Vec::new()));

    let start_completed_callback = Arc::clone(&start_completed);
    let start_completed_observer = Arc::clone(&start_completed);
    let started_observer = Arc::clone(&started_observed);
    let will_exit_observer = Arc::clone(&will_exit_observed);
    let reporter_observed = Arc::clone(&started_observed);
    let runtime_error_reporter = Arc::clone(&runtime_error_reported);
    let result = AppShell::builder(test_identity())
        .runner(PlatformRunner::headless())
        // Tray-first shape: no window, passive activation, explicit exit.
        .initial_activation(InitialActivation::Passive)
        .exit_policy(ExitPolicy::Explicit)
        .plugin(recording_plugin("first", Arc::clone(&plugin_events)))
        .plugin(recording_plugin("second", Arc::clone(&plugin_events)))
        .start(move |launch, cx: &mut App| {
            // The shell global is installed and AppInfo is reachable via the
            // extension trait with a raw &mut App.
            let info = cx.app_info();
            assert_eq!(info.app_id(), "com.example.appshelltest");
            assert_eq!(info.version(), "0.0.0");
            assert_eq!(info.paths().namespace(), "appshelltest");
            assert!(!info.capabilities().credential_store.is_supported());
            assert!(launch.cwd.is_some());

            // A liveness lease can be taken and released.
            let hold = cx.shell().hold("test");
            assert_eq!(hold.reason(), "test");
            drop(hold);

            start_completed_callback.store(true, Ordering::SeqCst);
            Ok(())
        })
        .on_event(|event, _cx| {
            if matches!(event, AppEvent::Started(_)) {
                return Err(anyhow::anyhow!("expected nonfatal lifecycle error").into());
            }
            Ok(())
        })
        .on_event(move |event, cx| {
            if matches!(event, AppEvent::Started(_)) {
                assert!(
                    start_completed_observer.load(Ordering::SeqCst),
                    "start must complete before Started is delivered"
                );
                started_observer.store(true, Ordering::SeqCst);
                cx.request_quit();
            } else if matches!(event, AppEvent::WillExit) {
                will_exit_observer.store(true, Ordering::SeqCst);
            }
            Ok(())
        })
        .on_error(move |error, _cx| {
            assert_eq!(
                error.operation(),
                gpui_component_app::error::RuntimeOperation::Lifecycle
            );
            assert_eq!(error.event(), Some("started"));
            assert!(error.continued());
            assert!(
                reporter_observed.load(Ordering::SeqCst),
                "one handler error must not prevent later handlers"
            );
            runtime_error_reporter.store(true, Ordering::SeqCst);
            println!("headless shell lifecycle: ok");
        })
        .run();

    assert!(
        result.is_ok(),
        "headless shell run returned error: {result:?}"
    );
    assert!(
        start_completed.load(Ordering::SeqCst),
        "start callback never ran"
    );
    assert!(
        started_observed.load(Ordering::SeqCst),
        "Started observer never ran"
    );
    assert!(
        runtime_error_reported.load(Ordering::SeqCst),
        "nonfatal lifecycle error was not reported"
    );
    assert!(
        will_exit_observed.load(Ordering::SeqCst),
        "orderly quit did not deliver WillExit"
    );
    assert_plugin_shutdown_order(&plugin_events);
    println!("{SUCCESS_RETURNED}");
}
