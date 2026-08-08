mod interaction_contracts;

use std::io::Write as _;
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};

use anyhow::{Context as _, bail};
use gpui_component_app::gpui;
use gpui_component_app::gpui::{Action, AnyWindowHandle, App, OwnedMenuItem, actions};
use gpui_component_app::{
    AppCommandsExt as _, AppEvent, AppProxy, AppShell, AppShellError, AppShellExt as _, Command,
    CommandId, CommandScope, ExitPolicy, InitialActivation, MenuPlan, ShellHold, ShutdownReason,
};
use serde::Serialize;
use serde_json::{Value, json};

const CONFORMANCE_MENU: &str = "Conformance";
const MENU_CHECKED_COMMAND_ID: &str = "conformance.menu-checked";
const MENU_UNCHECKED_COMMAND_ID: &str = "conformance.menu-unchecked";
const MENU_DISABLED_COMMAND_ID: &str = "conformance.menu-disabled";
const MENU_CHECKED_COMMAND_LABEL: &str = "Checked Conformance Command";
const MENU_UNCHECKED_COMMAND_LABEL: &str = "Unchecked Conformance Command";
const MENU_DISABLED_COMMAND_LABEL: &str = "Disabled Conformance Command";

actions!(
    conformance,
    [
        DispatchCheckedMenuCommand,
        DispatchUncheckedMenuCommand,
        DispatchDisabledMenuCommand,
    ]
);

use crate::cli::Scenario;
use crate::native_window::open_native_window;
#[cfg(feature = "wayland-conformance")]
use crate::native_window::open_native_window_with_key_down;
use crate::protocol::{CLIPBOARD_EXPECTED_PAYLOAD, Protocol};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ScenarioOutcome {
    Passed,
    ExpectedStartupFailure,
}

/// Shared status used by callbacks that complete after AppShell startup.
#[derive(Clone, Default)]
pub(crate) struct ScenarioState {
    inner: Arc<Mutex<ScenarioStateInner>>,
}

#[derive(Default)]
struct ScenarioStateInner {
    failure: Option<String>,
    first_presentation_observed: bool,
    background_dispatch_executed: bool,
    background_quit_requested: bool,
    clipboard_ready: bool,
    clipboard_acknowledged: bool,
    clipboard_quit_requested: bool,
    menu_command_callback_count: usize,
}

impl ScenarioState {
    pub(crate) fn emit(&self, protocol: &Protocol, event: &'static str, data: Value) {
        if let Err(error) = protocol.emit(event, data) {
            self.record_failure(format!(
                "could not write {event} protocol record: {error:#}"
            ));
        }
    }

    pub(crate) fn record_failure(&self, failure: String) {
        let mut state = self.lock();
        if state.failure.is_none() {
            state.failure = Some(failure);
        }
    }

    pub(crate) fn failure(&self) -> Option<String> {
        self.lock().failure.clone()
    }

    fn mark_first_presentation(&self) {
        self.lock().first_presentation_observed = true;
    }

    fn first_presentation_observed(&self) -> bool {
        self.lock().first_presentation_observed
    }

    fn mark_background_dispatch_executed(&self) {
        self.lock().background_dispatch_executed = true;
    }

    fn claim_background_quit(&self) -> bool {
        let mut state = self.lock();
        if state.background_dispatch_executed && !state.background_quit_requested {
            state.background_quit_requested = true;
            true
        } else {
            false
        }
    }

    fn background_quit_requested(&self) -> bool {
        self.lock().background_quit_requested
    }

    fn mark_clipboard_ready(&self) {
        self.lock().clipboard_ready = true;
    }

    fn claim_clipboard_acknowledgement(&self) -> bool {
        let mut state = self.lock();
        if state.clipboard_ready && !state.clipboard_acknowledged {
            state.clipboard_acknowledged = true;
            true
        } else {
            false
        }
    }

    fn claim_clipboard_quit(&self) -> bool {
        let mut state = self.lock();
        if state.clipboard_acknowledged && !state.clipboard_quit_requested {
            state.clipboard_quit_requested = true;
            true
        } else {
            false
        }
    }

    fn clipboard_acknowledged(&self) -> bool {
        self.lock().clipboard_acknowledged
    }

    fn clipboard_quit_requested(&self) -> bool {
        self.lock().clipboard_quit_requested
    }

    fn record_menu_command_dispatch(&self) -> usize {
        let mut state = self.lock();
        state.menu_command_callback_count += 1;
        state.menu_command_callback_count
    }

    fn menu_command_callback_count(&self) -> usize {
        self.lock().menu_command_callback_count
    }

    fn lock(&self) -> MutexGuard<'_, ScenarioStateInner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

pub(crate) fn run(scenario: Scenario, protocol: Protocol) -> anyhow::Result<ScenarioOutcome> {
    let exit_policy = match scenario {
        Scenario::LifecycleBackgroundQuit => "when_idle",
        _ => "explicit",
    };
    protocol.emit(
        "scenario_started",
        json!({"runner": "native", "exit_policy": exit_policy}),
    )?;

    match scenario {
        Scenario::LifecycleClean => run_lifecycle_clean(protocol),
        Scenario::LifecycleStartupFailure => run_lifecycle_startup_failure(protocol),
        Scenario::LifecycleBackgroundQuit => run_lifecycle_background_quit(protocol),
        Scenario::WindowCycle => run_window_cycle(protocol),
        Scenario::MenuCommand => run_menu_command(protocol),
        Scenario::Clipboard => run_clipboard(protocol),
        Scenario::InteractionContracts => interaction_contracts::run(protocol),
    }
}

fn run_lifecycle_clean(protocol: Protocol) -> anyhow::Result<ScenarioOutcome> {
    let state = ScenarioState::default();
    let event_protocol = protocol.clone();
    let event_state = state.clone();
    let startup_protocol = protocol.clone();
    let startup_state = state.clone();

    let result = AppShell::builder(crate::APP_IDENTITY)
        .shell_preferences()
        .exit_policy(ExitPolicy::Explicit)
        .on_event(move |event, _cx| {
            observe_app_event(&event_protocol, &event_state, event);
            Ok(())
        })
        .start(move |_, cx| {
            startup_state.emit(&startup_protocol, "startup_transaction_started", json!({}));
            let presentation_protocol = startup_protocol.clone();
            let presentation_state = startup_state.clone();
            let _opened = open_native_window(
                cx,
                startup_protocol.clone(),
                startup_state,
                "main",
                "Lifecycle Clean",
                move |cx| {
                    presentation_state.mark_first_presentation();
                    presentation_state.emit(
                        &presentation_protocol,
                        "quit_requested",
                        json!({"source": "first_presentation"}),
                    );
                    cx.request_quit();
                },
            )?;
            Ok(())
        })
        .run();

    if !state.first_presentation_observed() {
        state.record_failure("lifecycle-clean ended before first presentation".to_owned());
    }
    finish_normal_run(protocol, state, result)
}

fn run_lifecycle_startup_failure(protocol: Protocol) -> anyhow::Result<ScenarioOutcome> {
    let state = ScenarioState::default();
    let event_protocol = protocol.clone();
    let event_state = state.clone();
    let startup_protocol = protocol.clone();
    let startup_state = state.clone();

    let result = AppShell::builder(crate::APP_IDENTITY)
        .shell_preferences()
        .exit_policy(ExitPolicy::Explicit)
        .on_event(move |event, _cx| {
            observe_app_event(&event_protocol, &event_state, event);
            Ok(())
        })
        .start(move |_, _cx| {
            startup_state.emit(
                &startup_protocol,
                "startup_failure_triggered",
                json!({"source": "transactional_start"}),
            );
            bail!("intentional lifecycle startup failure");
        })
        .run();

    emit_after_run(&protocol, &result)?;
    match result {
        Err(AppShellError::Startup(_)) => {
            if let Some(failure) = state.failure() {
                bail!("startup-failure protocol failed: {failure}");
            }
            Ok(ScenarioOutcome::ExpectedStartupFailure)
        }
        Ok(()) => bail!("startup-failure scenario unexpectedly returned success"),
        Err(error) => bail!("startup-failure scenario returned unexpected error: {error:#}"),
    }
}

fn run_lifecycle_background_quit(protocol: Protocol) -> anyhow::Result<ScenarioOutcome> {
    let state = ScenarioState::default();
    let worker_slot: Arc<Mutex<Option<JoinHandle<BackgroundWorkerReport>>>> =
        Arc::new(Mutex::new(None));
    let (dispatch_trigger, dispatch_waiter) = mpsc::channel();

    let event_protocol = protocol.clone();
    let event_state = state.clone();
    let mut dispatch_trigger = Some(dispatch_trigger);
    let startup_protocol = protocol.clone();
    let startup_state = state.clone();
    let startup_worker_slot = Arc::clone(&worker_slot);

    let result = AppShell::builder(crate::APP_IDENTITY)
        .shell_preferences()
        .initial_activation(InitialActivation::Passive)
        .exit_policy(ExitPolicy::WhenIdle)
        .on_event(move |event, _cx| {
            observe_app_event(&event_protocol, &event_state, event);
            if matches!(event, AppEvent::Started(_)) {
                let Some(trigger) = dispatch_trigger.take() else {
                    event_state.record_failure(
                        "background dispatch trigger was delivered more than once".to_owned(),
                    );
                    return Ok(());
                };
                event_state.emit(
                    &event_protocol,
                    "background_dispatch_triggered",
                    json!({"source": "app_started"}),
                );
                if trigger.send(()).is_err() {
                    event_state.record_failure(
                        "background worker stopped before dispatch trigger".to_owned(),
                    );
                    event_state.emit(
                        &event_protocol,
                        "background_dispatch_trigger_failed",
                        json!({"reason": "worker_disconnected"}),
                    );
                }
            }
            Ok(())
        })
        .start(move |_, cx| {
            startup_state.emit(&startup_protocol, "startup_transaction_started", json!({}));
            let hold = cx.shell().hold("lifecycle-background-quit");
            let proxy = cx.app_proxy();
            let worker_protocol = startup_protocol.clone();
            let worker_state = startup_state.clone();
            let worker = thread::spawn(move || {
                run_background_worker(dispatch_waiter, proxy, worker_protocol, worker_state, hold)
            });
            let mut slot = lock_or_recover(&startup_worker_slot);
            if slot.replace(worker).is_some() {
                startup_state.record_failure("background worker was already installed".to_owned());
            }
            Ok(())
        })
        .run();

    emit_after_run(&protocol, &result)?;
    let worker_report = join_background_worker(&worker_slot, &protocol, &state)?;

    if let Err(error) = result {
        bail!("background-quit scenario returned AppShell error: {error:#}");
    }
    if worker_report != BackgroundWorkerReport::Accepted {
        bail!(
            "background-quit worker did not admit its AppProxy dispatch: {}",
            worker_report.as_str()
        );
    }
    if !state.background_quit_requested() {
        state.record_failure("background-quit ended before releasing its shell hold".to_owned());
    }
    if let Some(failure) = state.failure() {
        bail!("background-quit scenario failed: {failure}");
    }
    Ok(ScenarioOutcome::Passed)
}

fn run_window_cycle(protocol: Protocol) -> anyhow::Result<ScenarioOutcome> {
    let state = ScenarioState::default();
    let cycle = WindowCycleState::default();
    let first_window: Arc<Mutex<Option<AnyWindowHandle>>> = Arc::new(Mutex::new(None));
    let event_protocol = protocol.clone();
    let event_state = state.clone();
    let event_cycle = cycle.clone();
    let completion_cycle = cycle.clone();
    let startup_protocol = protocol.clone();
    let startup_state = state.clone();
    let startup_cycle = cycle;
    let startup_first_window = Arc::clone(&first_window);

    let result = AppShell::builder(crate::APP_IDENTITY)
        .shell_preferences()
        .exit_policy(ExitPolicy::Explicit)
        .on_event(move |event, cx| {
            observe_app_event(&event_protocol, &event_state, event);
            if matches!(event, AppEvent::LastWindowClosed) {
                match event_cycle.last_window_closed() {
                    Ok(true) => {
                        if !cx.windows().is_empty() {
                            fail_window_cycle(
                                cx,
                                &event_protocol,
                                &event_state,
                                "last-window lifecycle event fired before native windows closed",
                            );
                            return Ok(());
                        }
                        event_state.emit(
                            &event_protocol,
                            "window_closed",
                            json!({"generation": 1, "source": "last_window_closed"}),
                        );
                        event_state.emit(
                            &event_protocol,
                            "explicit_hold_verified",
                            json!({"window_count": 0}),
                        );
                        let protocol_for_recreate = event_protocol.clone();
                        let state_for_recreate = event_state.clone();
                        let cycle_for_recreate = event_cycle.clone();
                        cx.defer(move |cx| {
                            open_window_cycle_replacement(
                                cx,
                                protocol_for_recreate,
                                state_for_recreate,
                                cycle_for_recreate,
                            );
                        });
                    }
                    Ok(false) => {}
                    Err(error) => fail_window_cycle(cx, &event_protocol, &event_state, &error),
                }
            }
            Ok(())
        })
        .start(move |_, cx| {
            startup_state.emit(&startup_protocol, "startup_transaction_started", json!({}));

            let first_presentation_protocol = startup_protocol.clone();
            let first_presentation_state = startup_state.clone();
            let first_presentation_cycle = startup_cycle.clone();
            let first_window_for_presentation = Arc::clone(&startup_first_window);
            let opened = open_native_window(
                cx,
                startup_protocol.clone(),
                startup_state.clone(),
                "window-cycle-initial",
                "Window Cycle Initial",
                move |cx| {
                    if let Err(error) = first_presentation_cycle.first_window_presented() {
                        fail_window_cycle(
                            cx,
                            &first_presentation_protocol,
                            &first_presentation_state,
                            &error,
                        );
                        return;
                    }
                    let Some(window) = lock_or_recover(&first_window_for_presentation).take()
                    else {
                        fail_window_cycle(
                            cx,
                            &first_presentation_protocol,
                            &first_presentation_state,
                            "initial native window handle was unavailable after presentation",
                        );
                        return;
                    };
                    first_presentation_state.emit(
                        &first_presentation_protocol,
                        "window_close_requested",
                        json!({"generation": 1}),
                    );
                    let protocol_for_close = first_presentation_protocol.clone();
                    let state_for_close = first_presentation_state.clone();
                    cx.defer(move |cx| {
                        if let Err(error) = window.update(cx, |_, window, _| window.remove_window())
                        {
                            fail_window_cycle(
                                cx,
                                &protocol_for_close,
                                &state_for_close,
                                &format!("could not close initial native window: {error:?}"),
                            );
                        }
                    });
                },
            )?;
            *lock_or_recover(&startup_first_window) = Some(opened.window.into());
            Ok(())
        })
        .run();

    if !completion_cycle.is_complete() {
        state.record_failure("window-cycle ended before verification".to_owned());
    }
    finish_normal_run(protocol, state, result)
}

fn open_window_cycle_replacement(
    cx: &mut App,
    protocol: Protocol,
    state: ScenarioState,
    cycle: WindowCycleState,
) {
    let presentation_protocol = protocol.clone();
    let presentation_state = state.clone();
    let presentation_cycle = cycle.clone();
    match open_native_window(
        cx,
        protocol.clone(),
        state.clone(),
        "window-cycle-recreated",
        "Window Cycle Recreated",
        move |cx| {
            if let Err(error) = presentation_cycle.recreated_window_presented() {
                fail_window_cycle(cx, &presentation_protocol, &presentation_state, &error);
                return;
            }
            presentation_state.emit(
                &presentation_protocol,
                "window_cycle_verified",
                json!({
                    "key": "window-cycle",
                    "opened": 2,
                    "presentations": 2,
                    "closed": 1,
                    "zero_windows": true,
                }),
            );
            presentation_state.emit(
                &presentation_protocol,
                "quit_requested",
                json!({"source": "window_cycle_recreated_presentation"}),
            );
            cx.request_quit();
        },
    ) {
        Ok(_) => {
            if let Err(error) = cycle.recreated_window_opened() {
                fail_window_cycle(cx, &protocol, &state, &error);
                return;
            }
            state.emit(
                &protocol,
                "window_recreated",
                json!({"generation": 2, "key": "window-cycle-recreated"}),
            );
        }
        Err(error) => fail_window_cycle(
            cx,
            &protocol,
            &state,
            &format!("could not recreate native window: {error:#}"),
        ),
    }
}

fn fail_window_cycle(cx: &mut App, protocol: &Protocol, state: &ScenarioState, failure: &str) {
    state.record_failure(failure.to_owned());
    state.emit(protocol, "window_cycle_failed", json!({"reason": failure}));
    cx.request_quit();
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WindowCyclePhase {
    AwaitingInitialPresentation,
    AwaitingLastWindowClosed,
    Recreating,
    AwaitingRecreatedPresentation,
    Quitting,
}

#[derive(Clone)]
struct WindowCycleState {
    phase: Arc<Mutex<WindowCyclePhase>>,
}

impl Default for WindowCycleState {
    fn default() -> Self {
        Self {
            phase: Arc::new(Mutex::new(WindowCyclePhase::AwaitingInitialPresentation)),
        }
    }
}

impl WindowCycleState {
    fn first_window_presented(&self) -> Result<(), String> {
        self.transition(
            WindowCyclePhase::AwaitingInitialPresentation,
            WindowCyclePhase::AwaitingLastWindowClosed,
            "initial window presentation",
        )
    }

    fn last_window_closed(&self) -> Result<bool, String> {
        let mut phase = lock_or_recover(&self.phase);
        match *phase {
            WindowCyclePhase::AwaitingLastWindowClosed => {
                *phase = WindowCyclePhase::Recreating;
                Ok(true)
            }
            WindowCyclePhase::Quitting => Ok(false),
            actual => Err(format!(
                "last-window lifecycle event occurred during unexpected phase {actual:?}"
            )),
        }
    }

    fn recreated_window_opened(&self) -> Result<(), String> {
        self.transition(
            WindowCyclePhase::Recreating,
            WindowCyclePhase::AwaitingRecreatedPresentation,
            "recreated window open",
        )
    }

    fn recreated_window_presented(&self) -> Result<(), String> {
        self.transition(
            WindowCyclePhase::AwaitingRecreatedPresentation,
            WindowCyclePhase::Quitting,
            "recreated window presentation",
        )
    }

    fn is_complete(&self) -> bool {
        *lock_or_recover(&self.phase) == WindowCyclePhase::Quitting
    }

    fn transition(
        &self,
        expected: WindowCyclePhase,
        next: WindowCyclePhase,
        operation: &str,
    ) -> Result<(), String> {
        let mut phase = lock_or_recover(&self.phase);
        if *phase != expected {
            return Err(format!(
                "{operation} occurred during unexpected phase {phase:?}"
            ));
        }
        *phase = next;
        Ok(())
    }
}

fn run_menu_command(protocol: Protocol) -> anyhow::Result<ScenarioOutcome> {
    let state = ScenarioState::default();
    let event_protocol = protocol.clone();
    let event_state = state.clone();
    let startup_protocol = protocol.clone();
    let startup_state = state.clone();

    let result = AppShell::builder(crate::APP_IDENTITY)
        .shell_preferences()
        .menus(MenuPlan::from_keys([CONFORMANCE_MENU]))
        .exit_policy(ExitPolicy::Explicit)
        .on_event(move |event, _cx| {
            observe_app_event(&event_protocol, &event_state, event);
            Ok(())
        })
        .start(move |_, cx| {
            startup_state.emit(&startup_protocol, "startup_transaction_started", json!({}));
            cx.register_command(
                Command::new(
                    CommandId(MENU_CHECKED_COMMAND_ID),
                    MENU_CHECKED_COMMAND_LABEL,
                    CommandScope::App,
                    DispatchCheckedMenuCommand,
                )
                .with_checked(menu_checked)
                .placed(CONFORMANCE_MENU, 0, 0),
            )?;
            cx.register_command(
                Command::new(
                    CommandId(MENU_UNCHECKED_COMMAND_ID),
                    MENU_UNCHECKED_COMMAND_LABEL,
                    CommandScope::App,
                    DispatchUncheckedMenuCommand,
                )
                .with_checked(menu_unchecked)
                .placed(CONFORMANCE_MENU, 0, 1),
            )?;
            cx.register_command(
                Command::new(
                    CommandId(MENU_DISABLED_COMMAND_ID),
                    MENU_DISABLED_COMMAND_LABEL,
                    CommandScope::App,
                    DispatchDisabledMenuCommand,
                )
                .with_checked(menu_unchecked)
                .with_enabled(menu_disabled)
                .placed(CONFORMANCE_MENU, 0, 2),
            )?;
            startup_state.emit(
                &startup_protocol,
                "menu_commands_registered",
                json!({
                    "menu": CONFORMANCE_MENU,
                    "command_ids": [
                        MENU_CHECKED_COMMAND_ID,
                        MENU_UNCHECKED_COMMAND_ID,
                        MENU_DISABLED_COMMAND_ID,
                    ],
                }),
            );

            let command_protocol = startup_protocol.clone();
            let command_state = startup_state.clone();
            cx.on_action(move |_: &DispatchCheckedMenuCommand, cx| {
                let callback_count = command_state.record_menu_command_dispatch();
                if callback_count != 1 {
                    fail_menu_command(
                        cx,
                        &command_protocol,
                        &command_state,
                        "enabled checked menu command was dispatched more than once",
                    );
                    return;
                }
                command_state.emit(
                    &command_protocol,
                    "menu_command_dispatched",
                    json!({
                        "command_id": MENU_CHECKED_COMMAND_ID,
                        "dispatch": "app_action",
                        "callback_count": callback_count,
                    }),
                );
                command_state.emit(
                    &command_protocol,
                    "menu_command_verified",
                    json!({"registered": true, "dispatched": true}),
                );
                command_state.emit(
                    &command_protocol,
                    "quit_requested",
                    json!({"source": "projected_menu_command"}),
                );
                cx.request_quit();
            });

            let unchecked_protocol = startup_protocol.clone();
            let unchecked_state = startup_state.clone();
            cx.on_action(move |_: &DispatchUncheckedMenuCommand, cx| {
                fail_menu_command(
                    cx,
                    &unchecked_protocol,
                    &unchecked_state,
                    "unchecked menu command was dispatched unexpectedly",
                );
            });

            let disabled_protocol = startup_protocol.clone();
            let disabled_state = startup_state.clone();
            cx.on_action(move |_: &DispatchDisabledMenuCommand, cx| {
                fail_menu_command(
                    cx,
                    &disabled_protocol,
                    &disabled_state,
                    "disabled menu command was dispatched unexpectedly",
                );
            });

            let projection_protocol = startup_protocol.clone();
            let projection_state = startup_state.clone();
            let _opened = open_native_window(
                cx,
                startup_protocol,
                startup_state,
                "menu-command",
                "Menu Command",
                move |cx| {
                    cx.defer(move |cx| match projected_menu_command(cx) {
                        Ok(projected) => {
                            projection_state.emit(
                                &projection_protocol,
                                "menu_projection_observed",
                                json!({
                                    "projection": "owned_menu_model",
                                    "items": projected.items,
                                }),
                            );
                            cx.dispatch_action(projected.checked_action.as_ref());
                        }
                        Err(error) => {
                            fail_menu_command(
                                cx,
                                &projection_protocol,
                                &projection_state,
                                &format!(
                                    "could not obtain projected native menu commands: {error:#}"
                                ),
                            );
                        }
                    });
                },
            )?;
            Ok(())
        })
        .run();

    if state.menu_command_callback_count() != 1 {
        state.record_failure(format!(
            "enabled checked menu command callback count was {} instead of one",
            state.menu_command_callback_count()
        ));
    }
    finish_normal_run(protocol, state, result)
}

fn menu_checked(_: &App) -> bool {
    true
}

fn menu_unchecked(_: &App) -> bool {
    false
}

fn menu_disabled(_: &App) -> bool {
    false
}

const CLIPBOARD_ACKNOWLEDGEMENT: &[u8] = b"verified\n";
const CLIPBOARD_CANCELLATION: &[u8] = b"cancel\n";

fn run_clipboard(protocol: Protocol) -> anyhow::Result<ScenarioOutcome> {
    let listener =
        TcpListener::bind("127.0.0.1:0").context("bind clipboard acknowledgement listener")?;
    let acknowledgement_address = listener
        .local_addr()
        .context("obtain clipboard acknowledgement listener address")?;
    let state = ScenarioState::default();
    let worker_slot: Arc<Mutex<Option<JoinHandle<ClipboardWorkerReport>>>> =
        Arc::new(Mutex::new(None));
    let cancellation = Arc::new(ClipboardCancellation::default());
    let (worker_signal, worker_waiter) = mpsc::channel();

    let event_protocol = protocol.clone();
    let event_state = state.clone();
    let startup_protocol = protocol.clone();
    let startup_state = state.clone();
    let startup_listener = listener;
    let startup_worker_slot = Arc::clone(&worker_slot);
    let startup_worker_signal = worker_signal.clone();
    let startup_cancellation = Arc::clone(&cancellation);

    let result = AppShell::builder(crate::APP_IDENTITY)
        .shell_preferences()
        .exit_policy(ExitPolicy::Explicit)
        .on_event(move |event, _cx| {
            observe_app_event(&event_protocol, &event_state, event);
            Ok(())
        })
        .start(move |_, cx| {
            startup_state.emit(&startup_protocol, "startup_transaction_started", json!({}));

            let readiness_protocol = startup_protocol.clone();
            let readiness_state = startup_state.clone();
            let readiness_signal = startup_worker_signal;
            #[cfg(not(feature = "wayland-conformance"))]
            let _opened = open_native_window(
                cx,
                startup_protocol.clone(),
                startup_state.clone(),
                "clipboard",
                "Clipboard",
                move |cx| {
                    publish_clipboard_ready(
                        cx,
                        &readiness_protocol,
                        &readiness_state,
                        &readiness_signal,
                        acknowledgement_address,
                    );
                },
            )?;
            #[cfg(feature = "wayland-conformance")]
            let _opened = {
                let key_protocol = readiness_protocol.clone();
                let key_state = readiness_state.clone();
                open_native_window_with_key_down(
                    cx,
                    startup_protocol.clone(),
                    startup_state.clone(),
                    "clipboard",
                    "Clipboard",
                    move |event, _, cx| {
                        if event.is_held || event.keystroke.key != "a" {
                            fail_clipboard(
                                cx,
                                &key_protocol,
                                &key_state,
                                "Wayland conformance delivered an unexpected key-down event",
                            );
                            return;
                        }
                        key_state.emit(
                            &key_protocol,
                            "wayland_key_down_observed",
                            json!({"key": "a", "source": "weston_test"}),
                        );
                        publish_clipboard_ready(
                            cx,
                            &key_protocol,
                            &key_state,
                            &readiness_signal,
                            acknowledgement_address,
                        );
                    },
                    move |window, cx| {
                        request_wayland_clipboard_input(
                            window,
                            cx,
                            readiness_protocol,
                            readiness_state,
                        );
                    },
                )?
            };

            let worker_protocol = startup_protocol.clone();
            let worker_state = startup_state.clone();
            let proxy = cx.app_proxy();
            let worker = thread::spawn(move || {
                run_clipboard_worker(
                    startup_listener,
                    worker_waiter,
                    proxy,
                    startup_cancellation,
                    worker_protocol,
                    worker_state,
                )
            });
            let mut slot = lock_or_recover(&startup_worker_slot);
            if slot.replace(worker).is_some() {
                startup_state.record_failure("clipboard worker was already installed".to_owned());
            }
            Ok(())
        })
        .run();

    let _ = worker_signal.send(ClipboardWorkerSignal::Cancel);
    cancel_clipboard_worker(acknowledgement_address, &cancellation);
    let after_run_result = emit_after_run(&protocol, &result);
    let worker_report = join_clipboard_worker(&worker_slot, &protocol, &state);

    if let Err(error) = result {
        after_run_result?;
        if let Err(worker_error) = worker_report {
            state.record_failure(format!("clipboard worker could not join: {worker_error:#}"));
        }
        bail!("clipboard scenario returned AppShell error: {error:#}");
    }
    after_run_result?;
    let worker_report = worker_report?;
    if worker_report != ClipboardWorkerReport::AcknowledgementDispatched {
        state.record_failure(format!(
            "clipboard worker did not dispatch the verified acknowledgement: {}",
            worker_report.as_str()
        ));
    }
    if !state.clipboard_acknowledged() {
        state
            .record_failure("clipboard scenario ended without external acknowledgement".to_owned());
    }
    if !state.clipboard_quit_requested() {
        state.record_failure(
            "clipboard scenario ended before its acknowledgement quit callback".to_owned(),
        );
    }
    if let Some(failure) = state.failure() {
        bail!("clipboard scenario failed: {failure}");
    }
    Ok(ScenarioOutcome::Passed)
}

#[cfg(feature = "wayland-conformance")]
fn request_wayland_clipboard_input(
    window: &mut gpui::Window,
    cx: &mut App,
    protocol: Protocol,
    state: ScenarioState,
) {
    let input_result = window.request_wayland_conformance_key_press();
    state.emit(
        &protocol,
        "wayland_input_requested",
        json!({"protocol": "weston_test", "key": "a"}),
    );
    let proxy = cx.app_proxy();
    window
        .spawn(cx, async move |async_cx| match input_result.await {
            Ok(Ok(())) => {
                let update_protocol = protocol.clone();
                let update_state = state.clone();
                if let Err(error) = async_cx.update(move |_, _| {
                    update_state.emit(
                        &update_protocol,
                        "wayland_input_completed",
                        json!({"result": "key_press_delivered"}),
                    );
                }) {
                    state.record_failure(format!(
                        "could not record completed Wayland conformance input: {error:#}"
                    ));
                    let _ = proxy.dispatch(|cx| cx.request_quit());
                }
            }
            Ok(Err(error)) => {
                let update_protocol = protocol.clone();
                let update_state = state.clone();
                let message = format!("Wayland conformance input failed: {error:#}");
                if async_cx
                    .update(move |_, cx| {
                        fail_clipboard(cx, &update_protocol, &update_state, &message);
                    })
                    .is_err()
                {
                    state.record_failure(
                        "Wayland conformance input failed after window close".into(),
                    );
                    let _ = proxy.dispatch(|cx| cx.request_quit());
                }
            }
            Err(_) => {
                let update_protocol = protocol.clone();
                let update_state = state.clone();
                if async_cx
                    .update(move |_, cx| {
                        fail_clipboard(
                            cx,
                            &update_protocol,
                            &update_state,
                            "Wayland conformance input completion was cancelled",
                        );
                    })
                    .is_err()
                {
                    state.record_failure(
                        "Wayland conformance input was cancelled after window close".into(),
                    );
                    let _ = proxy.dispatch(|cx| cx.request_quit());
                }
            }
        })
        .detach();
}

fn publish_clipboard_ready(
    cx: &mut App,
    protocol: &Protocol,
    state: &ScenarioState,
    worker_signal: &mpsc::Sender<ClipboardWorkerSignal>,
    acknowledgement_address: SocketAddr,
) {
    cx.write_to_clipboard(gpui::ClipboardItem::new_string(
        CLIPBOARD_EXPECTED_PAYLOAD.to_owned(),
    ));
    if let Err(error) = protocol.emit(
        "clipboard_ready",
        json!({
            "expected_payload": CLIPBOARD_EXPECTED_PAYLOAD,
            "ack_address": acknowledgement_address.to_string(),
        }),
    ) {
        fail_clipboard(
            cx,
            protocol,
            state,
            &format!("could not write clipboard_ready protocol record: {error:#}"),
        );
        return;
    }

    state.mark_clipboard_ready();
    if worker_signal.send(ClipboardWorkerSignal::Ready).is_err() {
        fail_clipboard(
            cx,
            protocol,
            state,
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

fn join_clipboard_worker(
    worker_slot: &Arc<Mutex<Option<JoinHandle<ClipboardWorkerReport>>>>,
    protocol: &Protocol,
    state: &ScenarioState,
) -> anyhow::Result<ClipboardWorkerReport> {
    let worker = lock_or_recover(worker_slot)
        .take()
        .ok_or_else(|| anyhow::anyhow!("clipboard worker was never started"))?;
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

fn fail_clipboard(cx: &mut App, protocol: &Protocol, state: &ScenarioState, failure: &str) {
    state.record_failure(failure.to_owned());
    state.emit(protocol, "clipboard_failed", json!({"reason": failure}));
    cx.request_quit();
}

fn fail_menu_command(cx: &mut App, protocol: &Protocol, state: &ScenarioState, failure: &str) {
    state.record_failure(failure.to_owned());
    state.emit(protocol, "menu_command_failed", json!({"reason": failure}));
    cx.request_quit();
}

struct ProjectedMenuCommand {
    checked_action: Box<dyn Action>,
    items: Vec<MenuProjectionItem>,
}

#[derive(Serialize)]
struct MenuProjectionItem {
    label: &'static str,
    checked: bool,
    disabled: bool,
}

/// `OwnedMenuItem` exposes checked and disabled state but no independent
/// checkability flag, so the unchecked entry proves only its observed state.
fn projected_menu_command(cx: &App) -> anyhow::Result<ProjectedMenuCommand> {
    let menus = cx
        .get_menus()
        .ok_or_else(|| anyhow::anyhow!("native menu projection was unavailable"))?;
    let menu = menus
        .iter()
        .find(|menu| menu.name == CONFORMANCE_MENU)
        .ok_or_else(|| anyhow::anyhow!("conformance menu was not projected"))?;
    let action_count = menu
        .items
        .iter()
        .filter(|item| matches!(item, OwnedMenuItem::Action { .. }))
        .count();
    if action_count != 3 {
        bail!("conformance menu projected {action_count} actions instead of three");
    }

    let expected = [
        (MENU_CHECKED_COMMAND_LABEL, true, false),
        (MENU_UNCHECKED_COMMAND_LABEL, false, false),
        (MENU_DISABLED_COMMAND_LABEL, false, true),
    ];
    let mut checked_action = None;
    let mut items = Vec::with_capacity(expected.len());
    for (label, checked, disabled) in expected {
        let mut matches = menu.items.iter().filter_map(|item| match item {
            OwnedMenuItem::Action {
                name,
                action,
                checked: actual_checked,
                disabled: actual_disabled,
                ..
            } if name == label => Some((action, *actual_checked, *actual_disabled)),
            _ => None,
        });
        let Some((action, actual_checked, actual_disabled)) = matches.next() else {
            bail!("conformance menu did not project {label:?}");
        };
        if matches.next().is_some() {
            bail!("conformance menu projected {label:?} more than once");
        }
        if actual_checked != checked || actual_disabled != disabled {
            bail!(
                "conformance menu projected {label:?} as checked={actual_checked}, disabled={actual_disabled}"
            );
        }
        if label == MENU_CHECKED_COMMAND_LABEL {
            checked_action = Some(action.boxed_clone());
        }
        items.push(MenuProjectionItem {
            label,
            checked: actual_checked,
            disabled: actual_disabled,
        });
    }

    Ok(ProjectedMenuCommand {
        checked_action: checked_action
            .ok_or_else(|| anyhow::anyhow!("checked menu command was not projected"))?,
        items,
    })
}

fn finish_normal_run(
    protocol: Protocol,
    state: ScenarioState,
    result: Result<(), AppShellError>,
) -> anyhow::Result<ScenarioOutcome> {
    emit_after_run(&protocol, &result)?;
    if let Err(error) = result {
        return Err(
            anyhow::Error::new(error).context("native lifecycle scenario returned AppShell error")
        );
    }
    if let Some(failure) = state.failure() {
        bail!("native lifecycle scenario failed: {failure}");
    }
    Ok(ScenarioOutcome::Passed)
}

/// These records are deliberately emitted after `AppShell::run` returns.
fn emit_after_run(protocol: &Protocol, result: &Result<(), AppShellError>) -> anyhow::Result<()> {
    protocol.emit("shutdown_complete", json!({}))?;
    protocol.emit(
        "run_returned",
        json!({
            "result": if result.is_ok() { "ok" } else { "error" },
        }),
    )?;
    Ok(())
}

fn observe_app_event(protocol: &Protocol, state: &ScenarioState, event: &AppEvent) {
    state.emit(protocol, "app_event", json!({"kind": event.name()}));
    match event {
        AppEvent::ShutdownRequested(reason) => {
            state.emit(
                protocol,
                "shutdown_started",
                json!({"reason": shutdown_reason_name(*reason)}),
            );
        }
        AppEvent::WillExit => {
            state.emit(protocol, "will_exit", json!({}));
        }
        _ => {}
    }
}

fn shutdown_reason_name(reason: ShutdownReason) -> &'static str {
    match reason {
        ShutdownReason::StartupFailure => "startup_failure",
        ShutdownReason::LastWindowClosed => "last_window_closed",
        ShutdownReason::Requested => "requested",
        ShutdownReason::PlatformQuit => "platform_quit",
        _ => "other",
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BackgroundWorkerReport {
    Accepted,
    Rejected,
    NotTriggered,
}

impl BackgroundWorkerReport {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
            Self::NotTriggered => "not_triggered",
        }
    }
}

fn run_background_worker(
    dispatch_waiter: mpsc::Receiver<()>,
    proxy: AppProxy,
    protocol: Protocol,
    state: ScenarioState,
    hold: ShellHold,
) -> BackgroundWorkerReport {
    state.emit(&protocol, "background_worker_started", json!({}));
    if dispatch_waiter.recv().is_err() {
        state.emit(
            &protocol,
            "background_dispatch_not_triggered",
            json!({"reason": "startup_did_not_complete"}),
        );
        return BackgroundWorkerReport::NotTriggered;
    }

    let callback_protocol = protocol.clone();
    let callback_state = state.clone();
    let admission = proxy.dispatch(move |cx| {
        callback_state.emit(
            &callback_protocol,
            "background_dispatch_executed",
            json!({"result": "executed"}),
        );
        callback_state.mark_background_dispatch_executed();
        if !cx.windows().is_empty() {
            callback_state
                .record_failure("background-quit dispatch found native windows".to_owned());
            callback_state.emit(
                &callback_protocol,
                "background_zero_windows_failed",
                json!({"window_count": cx.windows().len()}),
            );
            drop(hold);
            return;
        }
        callback_state.emit(
            &callback_protocol,
            "background_zero_windows_verified",
            json!({"window_count": 0}),
        );
        if callback_state.claim_background_quit() {
            callback_state.emit(
                &callback_protocol,
                "background_hold_released",
                json!({"reason": hold.reason()}),
            );
            drop(hold);
        } else {
            callback_state.record_failure(
                "background-quit dispatch could not release its shell hold".to_owned(),
            );
            drop(hold);
        }
    });

    match admission {
        Ok(()) => {
            state.emit(
                &protocol,
                "background_dispatch_admission",
                json!({"accepted": true, "result": "queued"}),
            );
            BackgroundWorkerReport::Accepted
        }
        Err(error) => {
            state.record_failure(format!(
                "background AppProxy dispatch was rejected: {error}"
            ));
            state.emit(
                &protocol,
                "background_dispatch_admission",
                json!({"accepted": false, "result": "app_closed"}),
            );
            BackgroundWorkerReport::Rejected
        }
    }
}

fn join_background_worker(
    worker_slot: &Arc<Mutex<Option<JoinHandle<BackgroundWorkerReport>>>>,
    protocol: &Protocol,
    state: &ScenarioState,
) -> anyhow::Result<BackgroundWorkerReport> {
    let worker = lock_or_recover(worker_slot)
        .take()
        .ok_or_else(|| anyhow::anyhow!("background worker was never started"))?;
    let report = worker
        .join()
        .map_err(|panic| anyhow::anyhow!("background worker panicked: {}", panic_message(panic)))?;
    state.emit(
        protocol,
        "background_worker_joined",
        json!({"dispatch_admission": report.as_str()}),
    );
    Ok(report)
}

fn lock_or_recover<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn panic_message(panic: Box<dyn std::any::Any + Send + 'static>) -> String {
    if let Some(message) = panic.downcast_ref::<&str>() {
        (*message).to_owned()
    } else if let Some(message) = panic.downcast_ref::<String>() {
        message.clone()
    } else {
        "non-string panic payload".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_cycle_requires_last_window_closed_before_recreation() {
        let cycle = WindowCycleState::default();

        assert!(!cycle.is_complete());
        assert!(cycle.last_window_closed().is_err());
        cycle
            .first_window_presented()
            .expect("initial presentation should advance the cycle");
        assert!(
            cycle
                .last_window_closed()
                .expect("last-window lifecycle event should schedule recreation")
        );
        cycle
            .recreated_window_opened()
            .expect("recreated window should open after the lifecycle event");
        cycle
            .recreated_window_presented()
            .expect("recreated presentation should complete the cycle");
        assert!(cycle.is_complete());
        assert!(
            !cycle
                .last_window_closed()
                .expect("shutdown-driven close should not recreate another window")
        );
    }

    #[test]
    fn background_hold_release_requires_dispatch() {
        let state = ScenarioState::default();

        assert!(!state.background_quit_requested());
        assert!(!state.claim_background_quit());
        state.mark_background_dispatch_executed();
        assert!(state.claim_background_quit());
        assert!(state.background_quit_requested());
    }

    #[test]
    fn menu_command_callback_count_tracks_duplicate_dispatches() {
        let state = ScenarioState::default();

        assert_eq!(state.record_menu_command_dispatch(), 1);
        assert_eq!(state.record_menu_command_dispatch(), 2);
        assert_eq!(state.menu_command_callback_count(), 2);
    }

    #[test]
    fn clipboard_quit_requires_external_acknowledgement() {
        let state = ScenarioState::default();

        assert!(!state.clipboard_acknowledged());
        assert!(!state.clipboard_quit_requested());
        assert!(!state.claim_clipboard_acknowledgement());
        assert!(!state.claim_clipboard_quit());
        state.mark_clipboard_ready();
        assert!(state.claim_clipboard_acknowledgement());
        assert!(state.clipboard_acknowledged());
        assert!(!state.claim_clipboard_acknowledgement());
        assert!(state.claim_clipboard_quit());
        assert!(state.clipboard_quit_requested());
        assert!(!state.claim_clipboard_quit());
    }

    #[test]
    fn clipboard_acknowledgement_does_not_wait_for_connection_close() {
        use std::io::Write as _;
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
}
