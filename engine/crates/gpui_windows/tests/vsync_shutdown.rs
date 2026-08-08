#[path = "../src/vsync_shutdown.rs"]
mod vsync_shutdown;

use vsync_shutdown::{VSyncShutdown, VSyncShutdownAction};

#[test]
fn shutdown_accepts_worker_acknowledgement_before_the_queued_quit() {
    let mut shutdown = VSyncShutdown::default();
    shutdown.worker_started();

    assert_eq!(
        shutdown.request_quit(),
        VSyncShutdownAction::CancelWorkerAndPostQuit
    );
    let acknowledgement = shutdown.worker_stopped().unwrap();
    assert_eq!(
        shutdown.acknowledge_worker_stop(acknowledgement.wrapping_add(1)),
        VSyncShutdownAction::None
    );
    assert_eq!(
        shutdown.acknowledge_worker_stop(acknowledgement),
        VSyncShutdownAction::None
    );
    assert_eq!(
        shutdown.should_exit_after_quit_message(),
        VSyncShutdownAction::Exit
    );
    assert_eq!(
        shutdown.should_exit_after_quit_message(),
        VSyncShutdownAction::None
    );
}

#[test]
fn queued_quit_starts_bounded_shutdown_without_waiting_for_acknowledgement() {
    let mut shutdown = VSyncShutdown::default();
    shutdown.worker_started();

    assert_eq!(
        shutdown.request_quit(),
        VSyncShutdownAction::CancelWorkerAndPostQuit
    );
    assert_eq!(
        shutdown.should_exit_after_quit_message(),
        VSyncShutdownAction::WaitForWorker
    );
    assert_eq!(shutdown.worker_stopped(), None);
    assert_eq!(
        shutdown.complete_direct_shutdown(),
        VSyncShutdownAction::Exit
    );
}

#[test]
fn window_proc_panic_quit_waits_for_bounded_worker_shutdown() {
    let mut shutdown = VSyncShutdown::default();
    shutdown.worker_started();

    // The catch-unwind boundary posts WM_QUIT; the outer loop must still wait for VSync.
    assert_eq!(
        shutdown.should_exit_after_quit_message(),
        VSyncShutdownAction::CancelWorker
    );
    assert_eq!(shutdown.worker_stopped(), None);
    assert_eq!(
        shutdown.complete_direct_shutdown(),
        VSyncShutdownAction::Exit
    );
}

#[test]
fn shutdown_posts_quit_once_after_worker_stops() {
    let mut shutdown = VSyncShutdown::default();
    shutdown.worker_started();

    assert_eq!(shutdown.worker_stopped(), None);
    assert_eq!(shutdown.request_quit(), VSyncShutdownAction::PostQuit);
    assert_eq!(shutdown.request_quit(), VSyncShutdownAction::None);
    assert_eq!(
        shutdown.should_exit_after_quit_message(),
        VSyncShutdownAction::Exit
    );
    assert_eq!(
        shutdown.should_exit_after_quit_message(),
        VSyncShutdownAction::None
    );
}

#[test]
fn shutdown_without_worker_posts_quit_once() {
    let mut shutdown = VSyncShutdown::default();

    assert_eq!(shutdown.request_quit(), VSyncShutdownAction::PostQuit);
    assert_eq!(shutdown.request_quit(), VSyncShutdownAction::None);
    assert_eq!(
        shutdown.should_exit_after_quit_message(),
        VSyncShutdownAction::Exit
    );
    assert_eq!(
        shutdown.should_exit_after_quit_message(),
        VSyncShutdownAction::None
    );
}

#[test]
fn terminal_message_error_cancels_and_exits_once_without_queued_acknowledgement() {
    let mut shutdown = VSyncShutdown::default();
    shutdown.worker_started();

    assert_eq!(
        shutdown.request_terminal_shutdown(),
        VSyncShutdownAction::CancelWorker
    );
    assert_eq!(
        shutdown.request_terminal_shutdown(),
        VSyncShutdownAction::None
    );

    assert_eq!(shutdown.worker_stopped(), None);
    assert_eq!(
        shutdown.acknowledge_worker_stop(1),
        VSyncShutdownAction::None
    );
    assert_eq!(
        shutdown.complete_direct_shutdown(),
        VSyncShutdownAction::Exit
    );
    assert_eq!(
        shutdown.complete_direct_shutdown(),
        VSyncShutdownAction::None
    );
    assert_eq!(
        shutdown.request_terminal_shutdown(),
        VSyncShutdownAction::None
    );
}

#[test]
fn terminal_message_error_promotes_an_existing_stop_request_without_recancelling() {
    let mut shutdown = VSyncShutdown::default();
    shutdown.worker_started();

    assert_eq!(
        shutdown.request_quit(),
        VSyncShutdownAction::CancelWorkerAndPostQuit
    );
    assert_eq!(
        shutdown.request_terminal_shutdown(),
        VSyncShutdownAction::WaitForWorker
    );

    assert_eq!(shutdown.worker_stopped(), None);
    assert_eq!(
        shutdown.complete_direct_shutdown(),
        VSyncShutdownAction::Exit
    );
}

#[test]
fn terminal_message_error_rejects_an_acknowledgement_already_in_flight() {
    let mut shutdown = VSyncShutdown::default();
    shutdown.worker_started();

    assert_eq!(
        shutdown.request_quit(),
        VSyncShutdownAction::CancelWorkerAndPostQuit
    );
    let acknowledgement = shutdown.worker_stopped().unwrap();
    assert_eq!(
        shutdown.request_terminal_shutdown(),
        VSyncShutdownAction::WaitForWorker
    );
    assert_eq!(
        shutdown.acknowledge_worker_stop(acknowledgement),
        VSyncShutdownAction::None
    );
    assert_eq!(
        shutdown.complete_direct_shutdown(),
        VSyncShutdownAction::Exit
    );
}

#[test]
fn terminal_message_error_deadline_exits_once_without_late_acknowledgement() {
    let mut shutdown = VSyncShutdown::default();
    shutdown.worker_started();

    assert_eq!(
        shutdown.request_terminal_shutdown(),
        VSyncShutdownAction::CancelWorker
    );
    assert_eq!(
        shutdown.abandon_direct_shutdown(),
        VSyncShutdownAction::Exit
    );
    assert_eq!(shutdown.worker_stopped(), None);
    assert_eq!(
        shutdown.acknowledge_worker_stop(1),
        VSyncShutdownAction::None
    );
    assert_eq!(
        shutdown.abandon_direct_shutdown(),
        VSyncShutdownAction::None
    );
}
