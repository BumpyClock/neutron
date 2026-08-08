mod interaction_contracts;

use std::io::{self, BufRead, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex, MutexGuard};

use anyhow::Context as _;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::cli::{Scenario, ValidationProfile};

pub(crate) const SCHEMA_VERSION: u8 = 1;
pub(crate) const CLIPBOARD_EXPECTED_PAYLOAD: &str = "gpui-component-conformance-clipboard-v1";

/// A synchronized JSONL writer shared by the application and worker threads.
#[derive(Clone)]
pub(crate) struct Protocol {
    state: Arc<Mutex<ProtocolState>>,
}

struct ProtocolState {
    scenario: Scenario,
    next_sequence: u64,
    terminal_written: bool,
    writer: Box<dyn Write + Send>,
}

#[derive(Serialize)]
struct Record<'a> {
    schema: u8,
    sequence: u64,
    scenario: Scenario,
    event: &'a str,
    data: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ParsedRecord {
    schema: u8,
    sequence: u64,
    scenario: Scenario,
    event: String,
    data: Value,
}

pub(crate) enum TerminalOutcome {
    Passed,
    ExpectedStartupFailure,
    Failed(String),
    Panicked(String),
}

impl TerminalOutcome {
    pub(crate) const fn exit_code(&self) -> u8 {
        match self {
            Self::Passed => 0,
            Self::ExpectedStartupFailure => 2,
            Self::Failed(_) | Self::Panicked(_) => 1,
        }
    }

    fn data(&self) -> Value {
        let mut data = Map::new();
        data.insert("exit_code".into(), json!(self.exit_code()));
        match self {
            Self::Passed => {
                data.insert("outcome".into(), json!("passed"));
            }
            Self::ExpectedStartupFailure => {
                data.insert("outcome".into(), json!("expected_startup_failure"));
            }
            Self::Failed(error) => {
                data.insert("outcome".into(), json!("failed"));
                data.insert("error".into(), json!(error));
            }
            Self::Panicked(message) => {
                data.insert("outcome".into(), json!("panicked"));
                data.insert("panic".into(), json!(message));
            }
        }
        Value::Object(data)
    }
}

/// Validate a captured conformance JSONL trace without starting GPUI.
///
/// The validator accepts the JSONL protocol on stdin through `--validate`, so
/// CI can independently check a trace emitted by a prior native scenario run.
pub(crate) fn validate_jsonl(
    reader: impl BufRead,
    expected_scenario: Scenario,
) -> anyhow::Result<()> {
    validate_jsonl_with_profile(reader, expected_scenario, None)
}

pub(crate) fn validate_jsonl_with_profile(
    reader: impl BufRead,
    expected_scenario: Scenario,
    profile: Option<ValidationProfile>,
) -> anyhow::Result<()> {
    let mut records = Vec::new();
    let mut next_sequence = 1_u64;
    let mut terminal_seen = false;

    for (line_index, line) in reader.lines().enumerate() {
        let line_number = line_index + 1;
        let line = line.with_context(|| format!("read JSONL line {line_number}"))?;
        if line.trim().is_empty() {
            anyhow::bail!("JSONL line {line_number} was blank");
        }
        let record: ParsedRecord = serde_json::from_str(&line)
            .with_context(|| format!("parse JSONL line {line_number}"))?;
        if record.schema != SCHEMA_VERSION {
            anyhow::bail!(
                "JSONL line {line_number} has schema {}; expected {SCHEMA_VERSION}",
                record.schema
            );
        }
        if record.scenario != expected_scenario {
            anyhow::bail!(
                "JSONL line {line_number} has scenario {}; expected {expected_scenario}",
                record.scenario
            );
        }
        if record.sequence != next_sequence {
            anyhow::bail!(
                "JSONL line {line_number} has sequence {}; expected {next_sequence}",
                record.sequence
            );
        }
        validate_record_shape(&record, expected_scenario)
            .with_context(|| format!("validate JSONL line {line_number}"))?;
        if terminal_seen {
            anyhow::bail!("JSONL record appeared after terminal at line {line_number}");
        }
        if records.is_empty() && record.event != "scenario_started" {
            anyhow::bail!("first JSONL event must be scenario_started");
        }
        if record.event == "terminal" {
            validate_terminal(&record.data)?;
            terminal_seen = true;
        }
        records.push(record);
        next_sequence = next_sequence
            .checked_add(1)
            .context("JSONL sequence overflow")?;
    }

    if records.is_empty() {
        anyhow::bail!("JSONL trace was empty");
    }
    if !terminal_seen {
        anyhow::bail!("JSONL trace did not contain a terminal record");
    }
    validate_event_cardinality(&records, expected_scenario)?;
    validate_post_run_records(&records)?;
    match expected_scenario {
        Scenario::LifecycleClean => validate_lifecycle_clean(&records),
        Scenario::LifecycleStartupFailure => validate_lifecycle_startup_failure(&records),
        Scenario::LifecycleBackgroundQuit => validate_lifecycle_background_quit(&records),
        Scenario::WindowCycle => validate_window_cycle(&records),
        Scenario::MenuCommand => validate_menu_command(&records),
        Scenario::Clipboard => validate_clipboard(&records),
        Scenario::InteractionContracts => interaction_contracts::validate(&records),
    }?;
    validate_no_failure_evidence(&records)?;
    if let Some(profile) = profile {
        validate_profile(&records, expected_scenario, profile)?;
    }
    Ok(())
}

fn validate_record_shape(record: &ParsedRecord, scenario: Scenario) -> anyhow::Result<()> {
    if !event_allowed_for_scenario(scenario, &record.event) {
        anyhow::bail!(
            "event {:?} is not allowed for scenario {scenario}",
            record.event
        );
    }

    let fields = allowed_data_fields(&record.event)
        .ok_or_else(|| anyhow::anyhow!("unknown protocol event {:?}", record.event))?;
    validate_object_fields(&record.data, fields, &format!("{} data", record.event))?;

    if record.event == "app_event" {
        let kind = required_string(&record.data, "kind")?;
        let allowed = match scenario {
            Scenario::LifecycleStartupFailure => {
                matches!(kind, "shutdown_requested" | "will_exit")
            }
            Scenario::WindowCycle => matches!(
                kind,
                "started" | "last_window_closed" | "shutdown_requested" | "will_exit"
            ),
            Scenario::LifecycleClean
            | Scenario::LifecycleBackgroundQuit
            | Scenario::MenuCommand
            | Scenario::Clipboard
            | Scenario::InteractionContracts => {
                matches!(kind, "started" | "shutdown_requested" | "will_exit")
            }
        };
        if !allowed {
            anyhow::bail!("app_event kind {kind:?} is not allowed for scenario {scenario}");
        }
    }

    if record.event == "renderer_info"
        && let Some(info) = record.data.get("renderer_info")
    {
        validate_object_fields(
            info,
            &[
                "selection",
                "renderer",
                "backend",
                "adapter_name",
                "adapter_type",
                "vendor_id",
                "device_id",
            ],
            "renderer_info payload",
        )?;
    }
    if record.event == "menu_projection_observed"
        && let Some(items) = record.data.get("items").and_then(Value::as_array)
    {
        for (index, item) in items.iter().enumerate() {
            validate_object_fields(
                item,
                &["label", "checked", "disabled"],
                &format!("menu projection item {}", index + 1),
            )?;
        }
    }
    Ok(())
}

fn event_allowed_for_scenario(scenario: Scenario, event: &str) -> bool {
    if matches!(
        event,
        "scenario_started"
            | "app_event"
            | "shutdown_started"
            | "will_exit"
            | "shutdown_complete"
            | "run_returned"
            | "terminal"
    ) {
        return true;
    }

    match scenario {
        Scenario::LifecycleClean => matches!(
            event,
            "startup_transaction_started"
                | "native_window_handle"
                | "native_display_handle"
                | "renderer_info"
                | "window_opened"
                | "frame_presented"
                | "quit_requested"
                | "presentation_cancelled"
                | "presentation_count_invalid"
                | "presentation_delivery_failed"
        ),
        Scenario::LifecycleStartupFailure => event == "startup_failure_triggered",
        Scenario::LifecycleBackgroundQuit => matches!(
            event,
            "startup_transaction_started"
                | "background_worker_started"
                | "background_dispatch_triggered"
                | "background_dispatch_not_triggered"
                | "background_dispatch_trigger_failed"
                | "background_dispatch_admission"
                | "background_dispatch_executed"
                | "background_zero_windows_verified"
                | "background_zero_windows_failed"
                | "background_hold_released"
                | "background_worker_joined"
        ),
        Scenario::WindowCycle => matches!(
            event,
            "startup_transaction_started"
                | "native_window_handle"
                | "native_display_handle"
                | "renderer_info"
                | "window_opened"
                | "frame_presented"
                | "window_close_requested"
                | "window_closed"
                | "explicit_hold_verified"
                | "window_recreated"
                | "window_cycle_verified"
                | "window_cycle_failed"
                | "quit_requested"
                | "presentation_cancelled"
                | "presentation_count_invalid"
                | "presentation_delivery_failed"
        ),
        Scenario::MenuCommand => matches!(
            event,
            "startup_transaction_started"
                | "menu_commands_registered"
                | "native_window_handle"
                | "native_display_handle"
                | "renderer_info"
                | "window_opened"
                | "frame_presented"
                | "menu_projection_observed"
                | "menu_command_dispatched"
                | "menu_command_verified"
                | "menu_command_failed"
                | "quit_requested"
                | "presentation_cancelled"
                | "presentation_count_invalid"
                | "presentation_delivery_failed"
        ),
        Scenario::Clipboard => matches!(
            event,
            "startup_transaction_started"
                | "native_window_handle"
                | "native_display_handle"
                | "renderer_info"
                | "window_opened"
                | "frame_presented"
                | "wayland_input_requested"
                | "wayland_key_down_observed"
                | "wayland_input_completed"
                | "clipboard_worker_started"
                | "clipboard_ready"
                | "clipboard_acknowledged"
                | "clipboard_acknowledgement_rejected"
                | "clipboard_worker_joined"
                | "clipboard_failed"
                | "quit_requested"
                | "presentation_cancelled"
                | "presentation_count_invalid"
                | "presentation_delivery_failed"
        ),
        Scenario::InteractionContracts => matches!(
            event,
            "startup_transaction_started"
                | "native_window_handle"
                | "native_display_handle"
                | "renderer_info"
                | "window_opened"
                | "frame_presented"
                | "focus_text_verified"
                | "composition_verified"
                | "scale_verified"
                | "accessibility_verified"
                | "interaction_contracts_failed"
                | "quit_requested"
                | "presentation_cancelled"
                | "presentation_count_invalid"
                | "presentation_delivery_failed"
        ),
    }
}

fn allowed_data_fields(event: &str) -> Option<&'static [&'static str]> {
    Some(match event {
        "scenario_started" => &["runner", "exit_policy"],
        "startup_transaction_started"
        | "will_exit"
        | "shutdown_complete"
        | "background_worker_started"
        | "clipboard_worker_started" => &[],
        "app_event" => &["kind"],
        "native_window_handle" | "native_display_handle" => &["kind"],
        "renderer_info" => &["renderer_info"],
        "window_opened" => &["key", "title"],
        "frame_presented" => &["presentation_evidence", "count"],
        "quit_requested" => &["source"],
        "shutdown_started" => &["reason"],
        "run_returned" => &["result"],
        "terminal" => &["outcome", "exit_code", "error", "panic"],
        "startup_failure_triggered" => &["source"],
        "background_dispatch_triggered" => &["source"],
        "background_dispatch_not_triggered"
        | "background_dispatch_trigger_failed"
        | "window_cycle_failed"
        | "menu_command_failed"
        | "clipboard_acknowledgement_rejected"
        | "clipboard_failed"
        | "interaction_contracts_failed"
        | "presentation_cancelled"
        | "presentation_delivery_failed" => &["reason"],
        "background_dispatch_admission" => &["accepted", "result"],
        "background_dispatch_executed" => &["result"],
        "background_zero_windows_verified" | "background_zero_windows_failed" => &["window_count"],
        "background_hold_released" => &["reason"],
        "background_worker_joined" => &["dispatch_admission"],
        "window_close_requested" => &["generation"],
        "window_closed" => &["generation", "source"],
        "explicit_hold_verified" => &["window_count"],
        "window_recreated" => &["generation", "key"],
        "window_cycle_verified" => &["key", "opened", "presentations", "closed", "zero_windows"],
        "menu_commands_registered" => &["menu", "command_ids"],
        "menu_projection_observed" => &["projection", "items"],
        "menu_command_dispatched" => &["command_id", "dispatch", "callback_count"],
        "menu_command_verified" => &["registered", "dispatched"],
        "wayland_input_requested" => &["protocol", "key"],
        "wayland_key_down_observed" => &["key", "source"],
        "wayland_input_completed" => &["result"],
        "clipboard_ready" => &["expected_payload", "ack_address"],
        "clipboard_acknowledged" => &["acknowledgement"],
        "clipboard_worker_joined" => &["result"],
        "focus_text_verified" => &["activation_order", "inserted", "selection_utf16", "value"],
        "composition_verified" => &[
            "committed_value",
            "marked_range_utf16",
            "selection_utf16",
            "terminal",
        ],
        "scale_verified" => &["native_scale_factor", "tested_scale_factors"],
        "accessibility_verified" => &[
            "button_label",
            "button_supports_click",
            "focused_label",
            "focused_role",
            "focused_supports_focus",
            "focused_value",
            "node_count",
            "published",
            "toggle_label",
            "toggle_state",
        ],
        "presentation_count_invalid" => &["count"],
        _ => return None,
    })
}

fn validate_object_fields(value: &Value, allowed: &[&str], context: &str) -> anyhow::Result<()> {
    let object = value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("{context} must be a JSON object"))?;
    if let Some(field) = object
        .keys()
        .find(|field| !allowed.contains(&field.as_str()))
    {
        anyhow::bail!("{context} contains unknown field {field:?}");
    }
    Ok(())
}

fn validate_event_cardinality(records: &[ParsedRecord], scenario: Scenario) -> anyhow::Result<()> {
    for (index, record) in records.iter().enumerate() {
        let maximum = match record.event.as_str() {
            "app_event" => match scenario {
                Scenario::LifecycleStartupFailure => 2,
                Scenario::WindowCycle => 4,
                Scenario::LifecycleClean
                | Scenario::LifecycleBackgroundQuit
                | Scenario::MenuCommand
                | Scenario::Clipboard
                | Scenario::InteractionContracts => 3,
            },
            // Native evidence cardinality belongs to exact profile validation. Generic scenario
            // validation intentionally permits extra evidence so source-blind profile tests can
            // distinguish malformed groups from protocol-shape failures.
            "native_window_handle"
            | "native_display_handle"
            | "renderer_info"
            | "frame_presented" => usize::MAX,
            "window_opened" if scenario == Scenario::WindowCycle => 2,
            _ => 1,
        };
        let occurrence = records[..=index]
            .iter()
            .filter(|candidate| candidate.event == record.event)
            .count();
        if occurrence > maximum {
            anyhow::bail!(
                "event {:?} exceeded maximum cardinality {maximum} for scenario {scenario}",
                record.event
            );
        }
        if record.event == "app_event" {
            let kind = required_string(&record.data, "kind")?;
            if records[..index]
                .iter()
                .any(|candidate| candidate.event == "app_event" && candidate.data["kind"] == kind)
            {
                anyhow::bail!("duplicate app_event kind {kind:?}");
            }
        }
    }
    Ok(())
}

fn validate_profile(
    records: &[ParsedRecord],
    scenario: Scenario,
    profile: ValidationProfile,
) -> anyhow::Result<()> {
    let expected_groups = match scenario {
        Scenario::LifecycleClean
        | Scenario::MenuCommand
        | Scenario::Clipboard
        | Scenario::InteractionContracts => 1,
        Scenario::LifecycleStartupFailure | Scenario::LifecycleBackgroundQuit => 0,
        Scenario::WindowCycle => 2,
    };
    let mut groups = 0;
    let mut expected_event = "native_window_handle";

    for record in records {
        let validates_native_evidence = matches!(
            record.event.as_str(),
            "native_window_handle" | "native_display_handle" | "renderer_info" | "frame_presented"
        );
        if !validates_native_evidence {
            continue;
        }
        if record.event != expected_event {
            anyhow::bail!(
                "profile {profile} expected {expected_event} before {}",
                record.event
            );
        }
        match record.event.as_str() {
            "native_window_handle" => {
                validate_profile_native_handle(&record.data, profile)?;
                expected_event = "native_display_handle";
            }
            "native_display_handle" => {
                validate_profile_native_display_handle(&record.data, profile)?;
                expected_event = "renderer_info";
            }
            "renderer_info" => {
                validate_profile_renderer_info(&record.data, profile)?;
                expected_event = "frame_presented";
            }
            "frame_presented" => {
                validate_profile_presentation(&record.data, profile)?;
                groups += 1;
                expected_event = "native_window_handle";
            }
            _ => unreachable!("native evidence event was filtered above"),
        }
    }
    if expected_event != "native_window_handle" {
        anyhow::bail!("profile {profile} trace ended with an incomplete native evidence group");
    }
    if groups != expected_groups {
        anyhow::bail!(
            "profile {profile} expected {expected_groups} native evidence groups, found {groups}"
        );
    }
    if profile == ValidationProfile::LinuxWaylandLavapipe && scenario == Scenario::Clipboard {
        validate_wayland_clipboard_input(records)?;
    }
    Ok(())
}

fn validate_wayland_clipboard_input(records: &[ParsedRecord]) -> anyhow::Result<()> {
    let frame = exactly_one_event(records, "frame_presented")?;
    let requested = exactly_one_event(records, "wayland_input_requested")?;
    let key_down = exactly_one_event(records, "wayland_key_down_observed")?;
    let clipboard_ready = exactly_one_event(records, "clipboard_ready")?;
    let completed = exactly_one_event(records, "wayland_input_completed")?;
    if !(frame < requested
        && requested < key_down
        && key_down < clipboard_ready
        && clipboard_ready < completed)
    {
        anyhow::bail!(
            "Wayland clipboard input must order presentation, input request, key down, clipboard write, and input completion"
        );
    }
    if records[key_down].data["key"] != "a"
        || records[key_down].data["source"] != "weston_test"
        || records[completed].data["result"] != "key_press_delivered"
    {
        anyhow::bail!(
            "Wayland clipboard input evidence did not match the Weston key-press contract"
        );
    }
    Ok(())
}

fn validate_profile_native_handle(data: &Value, profile: ValidationProfile) -> anyhow::Result<()> {
    let kind = required_string(data, "kind")?;
    let valid = match profile {
        ValidationProfile::MacosMetal => kind == "app_kit",
        ValidationProfile::WindowsWarp => kind == "win32",
        ValidationProfile::LinuxX11Lavapipe => kind == "xcb",
        ValidationProfile::LinuxWaylandLavapipe => kind == "wayland",
    };
    if !valid {
        anyhow::bail!("profile {profile} does not accept native handle kind {kind:?}");
    }
    Ok(())
}

fn validate_profile_native_display_handle(
    data: &Value,
    profile: ValidationProfile,
) -> anyhow::Result<()> {
    let kind = required_string(data, "kind")?;
    let valid = match profile {
        ValidationProfile::MacosMetal => kind == "app_kit",
        ValidationProfile::WindowsWarp => kind == "windows",
        ValidationProfile::LinuxX11Lavapipe => kind == "xcb",
        ValidationProfile::LinuxWaylandLavapipe => kind == "wayland",
    };
    if !valid {
        anyhow::bail!("profile {profile} does not accept native display kind {kind:?}");
    }
    Ok(())
}

fn validate_profile_renderer_info(data: &Value, profile: ValidationProfile) -> anyhow::Result<()> {
    let info = data
        .get("renderer_info")
        .ok_or_else(|| anyhow::anyhow!("renderer_info record is missing renderer_info"))?;
    let selection = required_string(info, "selection")?;
    let renderer = required_string(info, "renderer")?;
    let backend = required_string(info, "backend")?;
    let adapter_name = required_string(info, "adapter_name")?;
    let adapter_type = required_string(info, "adapter_type")?;

    let (expected_selection, expected_renderer, expected_backend, expected_adapter_type) =
        match profile {
            ValidationProfile::MacosMetal => ("default", "metal", "Metal", "hardware"),
            ValidationProfile::WindowsWarp => ("software", "direct3d11", "Direct3D11", "software"),
            ValidationProfile::LinuxX11Lavapipe | ValidationProfile::LinuxWaylandLavapipe => {
                ("software", "wgpu", "Vulkan", "software")
            }
        };
    if selection != expected_selection
        || renderer != expected_renderer
        || backend != expected_backend
        || adapter_type != expected_adapter_type
    {
        anyhow::bail!(
            "renderer_info does not satisfy profile {profile}: expected selection {expected_selection:?}, renderer {expected_renderer:?}, backend {expected_backend:?}, and adapter type {expected_adapter_type:?}"
        );
    }
    let adapter_name = adapter_name.trim();
    if adapter_name.is_empty() {
        anyhow::bail!("renderer_info adapter_name was empty for profile {profile}");
    }
    let adapter_name = adapter_name.to_ascii_lowercase();
    match profile {
        ValidationProfile::WindowsWarp
            if !adapter_name.contains("warp")
                && adapter_name != "microsoft basic render driver" =>
        {
            anyhow::bail!(
                "renderer_info adapter_name {adapter_name:?} was not a WARP adapter for profile {profile}"
            );
        }
        ValidationProfile::LinuxX11Lavapipe | ValidationProfile::LinuxWaylandLavapipe
            if !adapter_name.contains("lavapipe") && !adapter_name.contains("llvmpipe") =>
        {
            anyhow::bail!(
                "renderer_info adapter_name {adapter_name:?} was not lavapipe or llvmpipe for profile {profile}"
            );
        }
        _ => {}
    }
    Ok(())
}

fn validate_profile_presentation(data: &Value, profile: ValidationProfile) -> anyhow::Result<()> {
    if data["count"] != 1 {
        anyhow::bail!("frame_presented count must be one for profile {profile}");
    }
    let evidence = required_string(data, "presentation_evidence")?;
    let valid = match profile {
        ValidationProfile::MacosMetal | ValidationProfile::WindowsWarp => {
            evidence == "backend_accepted"
        }
        ValidationProfile::LinuxX11Lavapipe | ValidationProfile::LinuxWaylandLavapipe => {
            evidence == "api_submitted"
        }
    };
    if !valid {
        anyhow::bail!(
            "presentation evidence {evidence:?} does not meet the minimum for profile {profile}"
        );
    }
    Ok(())
}

fn validate_no_failure_evidence(records: &[ParsedRecord]) -> anyhow::Result<()> {
    if let Some(record) = records.iter().find(|record| {
        matches!(
            record.event.as_str(),
            "background_dispatch_not_triggered"
                | "background_dispatch_trigger_failed"
                | "background_zero_windows_failed"
                | "clipboard_acknowledgement_rejected"
                | "clipboard_failed"
                | "menu_command_failed"
                | "interaction_contracts_failed"
                | "presentation_cancelled"
                | "presentation_count_invalid"
                | "presentation_delivery_failed"
                | "window_cycle_failed"
        )
    }) {
        anyhow::bail!(
            "passed trace contained failure evidence event {:?}",
            record.event
        );
    }
    Ok(())
}

fn validate_no_native_window_evidence(records: &[ParsedRecord]) -> anyhow::Result<()> {
    if records.iter().any(|record| {
        matches!(
            record.event.as_str(),
            "native_window_handle"
                | "native_display_handle"
                | "renderer_info"
                | "frame_presented"
                | "presentation_cancelled"
                | "presentation_count_invalid"
                | "presentation_delivery_failed"
        ) || record.event.starts_with("window_")
    }) {
        anyhow::bail!(
            "lifecycle-background-quit must not emit window, renderer, or presentation records"
        );
    }
    Ok(())
}

fn validate_terminal(data: &Value) -> anyhow::Result<()> {
    let outcome = required_string(data, "outcome")?;
    let expected_exit_code = match outcome {
        "passed" => 0,
        "expected_startup_failure" => 2,
        "failed" | "panicked" => 1,
        _ => anyhow::bail!("terminal has unknown outcome {outcome:?}"),
    };
    let exit_code = data
        .get("exit_code")
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow::anyhow!("terminal is missing integer exit_code"))?;
    if exit_code != expected_exit_code {
        anyhow::bail!(
            "terminal outcome {outcome:?} requires exit code {expected_exit_code}, got {exit_code}"
        );
    }
    Ok(())
}

fn validate_post_run_records(records: &[ParsedRecord]) -> anyhow::Result<()> {
    let terminal_index = records
        .iter()
        .position(|record| record.event == "terminal")
        .ok_or_else(|| anyhow::anyhow!("JSONL trace did not contain a terminal record"))?;
    let shutdown_complete = records[..terminal_index]
        .iter()
        .rposition(|record| record.event == "shutdown_complete")
        .ok_or_else(|| anyhow::anyhow!("terminal was not preceded by shutdown_complete"))?;
    let run_returned = records[..terminal_index]
        .iter()
        .rposition(|record| record.event == "run_returned")
        .ok_or_else(|| anyhow::anyhow!("terminal was not preceded by run_returned"))?;
    if shutdown_complete > run_returned {
        anyhow::bail!("shutdown_complete must precede run_returned");
    }
    Ok(())
}

fn validate_lifecycle_clean(records: &[ParsedRecord]) -> anyhow::Result<()> {
    if records[0].data["exit_policy"] != "explicit" {
        anyhow::bail!("lifecycle-clean must declare explicit exit policy");
    }

    let mut cursor = 1;
    cursor = find_event_after(records, cursor, "startup_transaction_started", |_| true)? + 1;
    cursor = find_event_after(records, cursor, "native_window_handle", |data| {
        validate_native_handle(data).is_ok()
    })? + 1;
    cursor = find_event_after(records, cursor, "native_display_handle", |data| {
        validate_native_display_handle(data).is_ok()
    })? + 1;
    cursor = find_event_after(records, cursor, "renderer_info", |data| {
        validate_renderer_info(data).is_ok()
    })? + 1;
    cursor = find_event_after(records, cursor, "window_opened", |data| {
        data["key"] == "main"
    })? + 1;
    cursor = find_event_after(records, cursor, "app_event", |data| {
        data["kind"] == "started"
    })? + 1;
    cursor = find_event_after(records, cursor, "frame_presented", frame_count_is_one)? + 1;
    let quit_requested = find_event_after(records, cursor, "quit_requested", |data| {
        data["source"] == "first_presentation"
    })?;
    validate_requested_shutdown(records, quit_requested)?;
    Ok(())
}

fn validate_lifecycle_startup_failure(records: &[ParsedRecord]) -> anyhow::Result<()> {
    if records[0].data["exit_policy"] != "explicit" {
        anyhow::bail!("lifecycle-startup-failure must declare explicit exit policy");
    }

    let mut cursor = 1;
    cursor = find_event_after(records, cursor, "startup_failure_triggered", |data| {
        data["source"] == "transactional_start"
    })? + 1;
    cursor = find_event_after(records, cursor, "app_event", |data| {
        data["kind"] == "shutdown_requested"
    })? + 1;
    cursor = find_event_after(records, cursor, "shutdown_started", |data| {
        data["reason"] == "startup_failure"
    })? + 1;
    cursor = find_event_after(records, cursor, "app_event", |data| {
        data["kind"] == "will_exit"
    })? + 1;
    cursor = find_event_after(records, cursor, "will_exit", |_| true)? + 1;
    cursor = find_event_after(records, cursor, "shutdown_complete", |_| true)? + 1;
    cursor = find_event_after(records, cursor, "run_returned", |data| {
        data["result"] == "error"
    })? + 1;
    let terminal = find_event_after(records, cursor, "terminal", |data| {
        data["outcome"] == "expected_startup_failure" && data["exit_code"] == 2
    })?;
    if terminal + 1 != records.len() {
        anyhow::bail!("terminal must be the final record");
    }
    Ok(())
}

fn validate_lifecycle_background_quit(records: &[ParsedRecord]) -> anyhow::Result<()> {
    if records[0].data["exit_policy"] != "when_idle" {
        anyhow::bail!("lifecycle-background-quit must declare when_idle exit policy");
    }
    validate_no_native_window_evidence(records)?;

    let mut cursor = 1;
    let startup = find_event_after(records, cursor, "startup_transaction_started", |_| true)?;
    let worker_started = exactly_one_event(records, "background_worker_started")?;
    if worker_started <= startup {
        anyhow::bail!("background worker started before the startup transaction");
    }
    let dispatch_admission = exactly_one_event(records, "background_dispatch_admission")?;
    if records[dispatch_admission].data["accepted"] != true
        || records[dispatch_admission].data["result"] != "queued"
    {
        anyhow::bail!("background dispatch admission was not accepted and queued");
    }
    cursor = startup + 1;
    cursor = find_event_after(records, cursor, "app_event", |data| {
        data["kind"] == "started"
    })? + 1;
    cursor = find_event_after(records, cursor, "background_dispatch_triggered", |data| {
        data["source"] == "app_started"
    })? + 1;
    let dispatch_executed =
        find_event_after(records, cursor, "background_dispatch_executed", |data| {
            data["result"] == "executed"
        })?;
    if worker_started >= dispatch_executed {
        anyhow::bail!("background worker did not start before its dispatch executed");
    }
    cursor = dispatch_executed + 1;
    cursor = find_event_after(
        records,
        cursor,
        "background_zero_windows_verified",
        |data| data["window_count"] == 0,
    )? + 1;
    let hold_released = find_event_after(records, cursor, "background_hold_released", |_| true)?;
    let (run_returned, terminal) = validate_shutdown_after(records, hold_released)?;
    let worker_joined = find_event_after(
        records,
        run_returned + 1,
        "background_worker_joined",
        |data| data["dispatch_admission"] == "accepted",
    )?;
    if worker_joined >= terminal {
        anyhow::bail!("background worker was not joined before terminal");
    }
    Ok(())
}

fn validate_window_cycle(records: &[ParsedRecord]) -> anyhow::Result<()> {
    if records[0].data["exit_policy"] != "explicit" {
        anyhow::bail!("window-cycle must declare explicit exit policy");
    }

    let mut cursor = 1;
    cursor = find_event_after(records, cursor, "startup_transaction_started", |_| true)? + 1;
    cursor = find_event_after(records, cursor, "native_window_handle", |data| {
        validate_native_handle(data).is_ok()
    })? + 1;
    cursor = find_event_after(records, cursor, "native_display_handle", |data| {
        validate_native_display_handle(data).is_ok()
    })? + 1;
    cursor = find_event_after(records, cursor, "renderer_info", |data| {
        validate_renderer_info(data).is_ok()
    })? + 1;
    cursor = find_event_after(records, cursor, "window_opened", |data| {
        data["key"] == "window-cycle-initial"
    })? + 1;
    let first_frame = find_event_after(records, cursor, "frame_presented", frame_count_is_one)?;
    cursor = first_frame + 1;
    cursor = find_event_after(records, cursor, "window_close_requested", |data| {
        data["generation"] == 1
    })? + 1;
    cursor = find_event_after(records, cursor, "app_event", |data| {
        data["kind"] == "last_window_closed"
    })? + 1;
    cursor = find_event_after(records, cursor, "window_closed", |data| {
        data["generation"] == 1 && data["source"] == "last_window_closed"
    })? + 1;
    cursor = find_event_after(records, cursor, "explicit_hold_verified", |data| {
        data["window_count"] == 0
    })? + 1;
    cursor = find_event_after(records, cursor, "native_window_handle", |data| {
        validate_native_handle(data).is_ok()
    })? + 1;
    cursor = find_event_after(records, cursor, "native_display_handle", |data| {
        validate_native_display_handle(data).is_ok()
    })? + 1;
    cursor = find_event_after(records, cursor, "renderer_info", |data| {
        validate_renderer_info(data).is_ok()
    })? + 1;
    cursor = find_event_after(records, cursor, "window_opened", |data| {
        data["key"] == "window-cycle-recreated"
    })? + 1;
    cursor = find_event_after(records, cursor, "window_recreated", |data| {
        data["generation"] == 2 && data["key"] == "window-cycle-recreated"
    })? + 1;
    let second_frame = find_event_after(records, cursor, "frame_presented", frame_count_is_one)?;
    if records[..second_frame].iter().any(|record| {
        record.event == "quit_requested"
            || record.event == "shutdown_started"
            || (record.event == "app_event" && record.data["kind"] == "shutdown_requested")
    }) {
        anyhow::bail!("window-cycle requested shutdown before the recreated window presented");
    }
    cursor = second_frame + 1;
    cursor = find_event_after(records, cursor, "window_cycle_verified", |data| {
        data["key"] == "window-cycle"
            && data["opened"] == 2
            && data["presentations"] == 2
            && data["closed"] == 1
            && data["zero_windows"] == true
    })? + 1;
    let quit_requested = find_event_after(records, cursor, "quit_requested", |_| true)?;
    validate_requested_shutdown(records, quit_requested)?;
    Ok(())
}

fn validate_menu_command(records: &[ParsedRecord]) -> anyhow::Result<()> {
    if records[0].data["exit_policy"] != "explicit" {
        anyhow::bail!("menu-command must declare explicit exit policy");
    }

    let mut cursor = 1;
    cursor = find_event_after(records, cursor, "startup_transaction_started", |_| true)? + 1;
    cursor = find_event_after(records, cursor, "menu_commands_registered", |data| {
        registered_menu_commands(data)
    })? + 1;
    cursor = find_event_after(records, cursor, "native_window_handle", |data| {
        validate_native_handle(data).is_ok()
    })? + 1;
    cursor = find_event_after(records, cursor, "native_display_handle", |data| {
        validate_native_display_handle(data).is_ok()
    })? + 1;
    cursor = find_event_after(records, cursor, "renderer_info", |data| {
        validate_renderer_info(data).is_ok()
    })? + 1;
    cursor = find_event_after(records, cursor, "window_opened", |data| {
        data["key"] == "menu-command"
    })? + 1;
    let frame = find_event_after(records, cursor, "frame_presented", frame_count_is_one)?;
    cursor = frame + 1;
    cursor = find_event_after(records, cursor, "menu_projection_observed", |data| {
        valid_menu_projection(data)
    })? + 1;
    if records
        .iter()
        .filter(|record| record.event == "menu_command_dispatched")
        .count()
        != 1
    {
        anyhow::bail!("menu-command must dispatch exactly one command");
    }
    cursor = find_event_after(records, cursor, "menu_command_dispatched", |data| {
        data["command_id"] == "conformance.menu-checked"
            && data["dispatch"] == "app_action"
            && data["callback_count"] == 1
    })? + 1;
    cursor = find_event_after(records, cursor, "menu_command_verified", |data| {
        data["registered"] == true && data["dispatched"] == true
    })? + 1;
    let quit_requested = find_event_after(records, cursor, "quit_requested", |_| true)?;
    validate_requested_shutdown(records, quit_requested)?;
    Ok(())
}

fn validate_clipboard(records: &[ParsedRecord]) -> anyhow::Result<()> {
    if records[0].data["exit_policy"] != "explicit" {
        anyhow::bail!("clipboard must declare explicit exit policy");
    }

    let mut cursor = 1;
    cursor = find_event_after(records, cursor, "startup_transaction_started", |_| true)? + 1;
    cursor = find_event_after(records, cursor, "native_window_handle", |data| {
        validate_native_handle(data).is_ok()
    })? + 1;
    cursor = find_event_after(records, cursor, "native_display_handle", |data| {
        validate_native_display_handle(data).is_ok()
    })? + 1;
    cursor = find_event_after(records, cursor, "renderer_info", |data| {
        validate_renderer_info(data).is_ok()
    })? + 1;
    let window_opened = find_event_after(records, cursor, "window_opened", |data| {
        data["key"] == "clipboard"
    })?;
    let frame_presented = find_event_after(
        records,
        window_opened + 1,
        "frame_presented",
        frame_count_is_one,
    )?;
    let clipboard_ready = exactly_one_event(records, "clipboard_ready")?;
    if clipboard_ready <= frame_presented {
        anyhow::bail!("clipboard_ready must follow first presentation evidence");
    }
    validate_clipboard_ready(&records[clipboard_ready].data)?;
    if records.iter().any(|record| {
        matches!(
            record.event.as_str(),
            "clipboard_acknowledgement_rejected"
                | "clipboard_failed"
                | "presentation_cancelled"
                | "presentation_count_invalid"
                | "presentation_delivery_failed"
        )
    }) {
        anyhow::bail!("clipboard passed trace contained failure evidence");
    }

    let clipboard_acknowledged = exactly_one_event(records, "clipboard_acknowledged")?;
    let worker_started = exactly_one_event(records, "clipboard_worker_started")?;
    if worker_started <= window_opened || worker_started >= clipboard_acknowledged {
        anyhow::bail!(
            "clipboard worker must start after its window opens and before acknowledgement"
        );
    }
    if clipboard_acknowledged <= clipboard_ready {
        anyhow::bail!("clipboard_acknowledged must follow clipboard_ready");
    }
    if records[clipboard_acknowledged].data["acknowledgement"] != "verified" {
        anyhow::bail!("clipboard_acknowledged must record the verified acknowledgement");
    }
    if records[..clipboard_acknowledged].iter().any(|record| {
        record.event == "quit_requested"
            || record.event == "shutdown_started"
            || record.event == "will_exit"
            || record.event == "shutdown_complete"
            || record.event == "run_returned"
            || (record.event == "app_event"
                && matches!(
                    record.data["kind"].as_str(),
                    Some("shutdown_requested" | "will_exit")
                ))
    }) {
        anyhow::bail!("clipboard began shutdown before external acknowledgement");
    }

    let quit_requested = clipboard_acknowledged + 1;
    require_event_at(records, quit_requested, "quit_requested", |data| {
        data["source"] == "external_clipboard_acknowledgement"
    })?;
    let shutdown_requested = quit_requested + 1;
    require_event_at(records, shutdown_requested, "app_event", |data| {
        data["kind"] == "shutdown_requested"
    })?;
    let shutdown_started = shutdown_requested + 1;
    require_event_at(records, shutdown_started, "shutdown_started", |data| {
        data["reason"] == "requested"
    })?;
    let will_exit_event = shutdown_started + 1;
    require_event_at(records, will_exit_event, "app_event", |data| {
        data["kind"] == "will_exit"
    })?;
    let will_exit = will_exit_event + 1;
    require_event_at(records, will_exit, "will_exit", |_| true)?;
    let shutdown_complete = will_exit + 1;
    require_event_at(records, shutdown_complete, "shutdown_complete", |_| true)?;
    let run_returned = shutdown_complete + 1;
    require_event_at(records, run_returned, "run_returned", |data| {
        data["result"] == "ok"
    })?;
    let worker_joined = run_returned + 1;
    require_event_at(records, worker_joined, "clipboard_worker_joined", |data| {
        data["result"] == "acknowledgement_dispatched"
    })?;
    if exactly_one_event(records, "clipboard_worker_joined")? != worker_joined {
        anyhow::bail!("clipboard worker joined outside the successful shutdown tail");
    }
    let terminal = worker_joined + 1;
    require_event_at(records, terminal, "terminal", |data| {
        data["outcome"] == "passed" && data["exit_code"] == 0
    })?;
    if terminal + 1 != records.len() {
        anyhow::bail!("terminal must be the final record");
    }
    Ok(())
}

fn validate_clipboard_ready(data: &Value) -> anyhow::Result<()> {
    let payload = required_string(data, "expected_payload")?;
    if payload != CLIPBOARD_EXPECTED_PAYLOAD
        || payload.is_empty()
        || payload.ends_with('\n')
        || payload.ends_with('\r')
    {
        anyhow::bail!("clipboard_ready expected_payload was not the fixed nonempty payload");
    }

    let address = required_string(data, "ack_address")?;
    let address: SocketAddr = address
        .parse()
        .with_context(|| "clipboard_ready ack_address was not a socket address")?;
    if address.ip() != IpAddr::V4(Ipv4Addr::LOCALHOST) || address.port() == 0 {
        anyhow::bail!("clipboard_ready ack_address must be a nonzero 127.0.0.1 port");
    }
    Ok(())
}

fn validate_shutdown_after(
    records: &[ParsedRecord],
    after: usize,
) -> anyhow::Result<(usize, usize)> {
    let mut cursor = after + 1;
    cursor = find_event_after(records, cursor, "app_event", |data| {
        data["kind"] == "shutdown_requested"
    })? + 1;
    cursor = find_event_after(records, cursor, "shutdown_started", |data| {
        data["reason"] == "requested"
    })? + 1;
    cursor = find_event_after(records, cursor, "app_event", |data| {
        data["kind"] == "will_exit"
    })? + 1;
    cursor = find_event_after(records, cursor, "will_exit", |_| true)? + 1;
    cursor = find_event_after(records, cursor, "shutdown_complete", |_| true)? + 1;
    let run_returned = find_event_after(records, cursor, "run_returned", |data| {
        data["result"] == "ok"
    })?;
    let terminal = find_event_after(records, run_returned + 1, "terminal", |data| {
        data["outcome"] == "passed" && data["exit_code"] == 0
    })?;
    if terminal + 1 != records.len() {
        anyhow::bail!("terminal must be the final record");
    }
    Ok((run_returned, terminal))
}

fn validate_requested_shutdown(
    records: &[ParsedRecord],
    after: usize,
) -> anyhow::Result<(usize, usize)> {
    let mut cursor = after + 1;
    cursor = find_event_after(records, cursor, "app_event", |data| {
        data["kind"] == "shutdown_requested"
    })? + 1;
    cursor = find_event_after(records, cursor, "shutdown_started", |data| {
        data["reason"] == "requested"
    })? + 1;
    cursor = find_event_after(records, cursor, "app_event", |data| {
        data["kind"] == "will_exit"
    })? + 1;
    cursor = find_event_after(records, cursor, "will_exit", |_| true)? + 1;
    cursor = find_event_after(records, cursor, "shutdown_complete", |_| true)? + 1;
    let run_returned = find_event_after(records, cursor, "run_returned", |data| {
        data["result"] == "ok"
    })?;
    cursor = run_returned + 1;
    let terminal = find_event_after(records, cursor, "terminal", |data| {
        data["outcome"] == "passed" && data["exit_code"] == 0
    })?;
    if terminal + 1 != records.len() {
        anyhow::bail!("terminal must be the final record");
    }
    Ok((run_returned, terminal))
}

fn require_event_at(
    records: &[ParsedRecord],
    index: usize,
    event: &str,
    predicate: impl Fn(&Value) -> bool,
) -> anyhow::Result<()> {
    let record = records
        .get(index)
        .ok_or_else(|| anyhow::anyhow!("missing expected {event} record"))?;
    if record.event != event || !predicate(&record.data) {
        anyhow::bail!("expected {event} record at trace position {}", index + 1);
    }
    Ok(())
}

fn exactly_one_event(records: &[ParsedRecord], event: &str) -> anyhow::Result<usize> {
    let mut matches = records
        .iter()
        .enumerate()
        .filter(|(_, record)| record.event == event);
    let index = matches
        .next()
        .map(|(index, _)| index)
        .ok_or_else(|| anyhow::anyhow!("missing expected {event} record"))?;
    if matches.next().is_some() {
        anyhow::bail!("expected exactly one {event} record");
    }
    Ok(index)
}

fn find_event_after(
    records: &[ParsedRecord],
    start: usize,
    event: &str,
    predicate: impl Fn(&Value) -> bool,
) -> anyhow::Result<usize> {
    records
        .iter()
        .enumerate()
        .skip(start)
        .find(|(_, record)| record.event == event && predicate(&record.data))
        .map(|(index, _)| index)
        .ok_or_else(|| anyhow::anyhow!("missing expected {event} record"))
}

fn frame_count_is_one(data: &Value) -> bool {
    matches!(
        data["presentation_evidence"].as_str(),
        Some("backend_accepted" | "api_submitted")
    ) && data["count"] == 1
}

fn validate_native_handle(data: &Value) -> anyhow::Result<()> {
    let kind = required_string(data, "kind")?;
    if !matches!(kind, "app_kit" | "win32" | "xlib" | "xcb" | "wayland") {
        anyhow::bail!("unsupported native handle kind {kind:?}");
    }
    Ok(())
}

fn validate_native_display_handle(data: &Value) -> anyhow::Result<()> {
    let kind = required_string(data, "kind")?;
    if !matches!(kind, "app_kit" | "windows" | "xcb" | "wayland") {
        anyhow::bail!("unsupported native display handle kind {kind:?}");
    }
    Ok(())
}

fn validate_renderer_info(data: &Value) -> anyhow::Result<()> {
    let info = data
        .get("renderer_info")
        .ok_or_else(|| anyhow::anyhow!("renderer_info record is missing renderer_info"))?;
    for field in ["selection", "renderer", "backend", "adapter_name"] {
        if required_string(info, field)?.is_empty() {
            anyhow::bail!("renderer_info field {field:?} was empty");
        }
    }
    Ok(())
}

fn registered_menu_commands(data: &Value) -> bool {
    data["menu"] == "Conformance"
        && data["command_ids"].as_array().is_some_and(|command_ids| {
            command_ids
                == &[
                    json!("conformance.menu-checked"),
                    json!("conformance.menu-unchecked"),
                    json!("conformance.menu-disabled"),
                ]
        })
}

fn valid_menu_projection(data: &Value) -> bool {
    if data["projection"] != "owned_menu_model" {
        return false;
    }
    let Some(items) = data["items"].as_array() else {
        return false;
    };
    let expected = [
        ("Checked Conformance Command", true, false),
        ("Unchecked Conformance Command", false, false),
        ("Disabled Conformance Command", false, true),
    ];
    items.len() == expected.len()
        && expected.iter().all(|(label, checked, disabled)| {
            items
                .iter()
                .filter(|item| {
                    item["label"] == *label
                        && item["checked"] == *checked
                        && item["disabled"] == *disabled
                })
                .count()
                == 1
        })
}

fn required_string<'a>(data: &'a Value, field: &str) -> anyhow::Result<&'a str> {
    data.get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("record is missing string field {field:?}"))
}

impl Protocol {
    pub(crate) fn stdout(scenario: Scenario) -> Self {
        Self::with_writer(scenario, Box::new(io::stdout()))
    }

    fn with_writer(scenario: Scenario, writer: Box<dyn Write + Send>) -> Self {
        Self {
            state: Arc::new(Mutex::new(ProtocolState {
                scenario,
                next_sequence: 1,
                terminal_written: false,
                writer,
            })),
        }
    }

    /// Write one non-terminal protocol record and flush it before returning.
    pub(crate) fn emit(&self, event: &'static str, data: Value) -> anyhow::Result<()> {
        self.write_record(event, data, false).map(|_| ())
    }

    /// Write the terminal record exactly once. A repeated attempt is ignored.
    pub(crate) fn terminal(&self, outcome: TerminalOutcome) -> anyhow::Result<bool> {
        self.write_record("terminal", outcome.data(), true)
    }

    fn write_record(
        &self,
        event: &'static str,
        data: Value,
        terminal: bool,
    ) -> anyhow::Result<bool> {
        let mut state = self.lock();
        if state.terminal_written {
            if terminal {
                return Ok(false);
            }
            anyhow::bail!("cannot write protocol record after terminal");
        }

        let sequence = state.next_sequence;
        state.next_sequence = sequence
            .checked_add(1)
            .context("protocol sequence overflow")?;
        if terminal {
            // Set this before writing so a partial I/O failure cannot produce a
            // second terminal line from another caller.
            state.terminal_written = true;
        }

        let record = Record {
            schema: SCHEMA_VERSION,
            sequence,
            scenario: state.scenario,
            event,
            data,
        };
        serde_json::to_writer(&mut state.writer, &record).context("serialize protocol record")?;
        state
            .writer
            .write_all(b"\n")
            .context("terminate protocol record")?;
        state.writer.flush().context("flush protocol record")?;
        Ok(true)
    }

    fn lock(&self) -> MutexGuard<'_, ProtocolState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::sync::Arc;

    use serde_json::Value;

    use super::*;

    #[derive(Clone, Default)]
    struct TestWriter {
        bytes: Arc<Mutex<Vec<u8>>>,
    }

    impl Write for TestWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.bytes
                .lock()
                .expect("test output mutex should not be poisoned")
                .extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn terminal_is_written_once_with_monotonic_sequences() {
        let writer = TestWriter::default();
        let bytes = Arc::clone(&writer.bytes);
        let protocol = Protocol::with_writer(Scenario::LifecycleClean, Box::new(writer));

        protocol
            .emit("scenario_started", json!({"runner": "native"}))
            .expect("record should write");
        assert!(
            protocol
                .terminal(TerminalOutcome::Passed)
                .expect("terminal should write")
        );
        assert!(
            !protocol
                .terminal(TerminalOutcome::Failed("duplicate".into()))
                .expect("second terminal should be ignored")
        );

        let output = String::from_utf8(
            bytes
                .lock()
                .expect("test output mutex should not be poisoned")
                .clone(),
        )
        .expect("protocol should be utf-8");
        let records: Vec<Value> = output
            .lines()
            .map(|line| serde_json::from_str(line).expect("line should be JSON"))
            .collect();

        assert_eq!(records.len(), 2);
        assert_eq!(records[0]["schema"], SCHEMA_VERSION);
        assert_eq!(records[0]["sequence"], 1);
        assert_eq!(records[1]["sequence"], 2);
        assert_eq!(records[1]["event"], "terminal");
        assert_eq!(records[1]["data"]["outcome"], "passed");
    }

    #[test]
    fn validates_lifecycle_contracts() {
        validate_jsonl(
            trace(valid_lifecycle_clean_trace()),
            Scenario::LifecycleClean,
        )
        .expect("clean lifecycle trace should validate");
        validate_jsonl(
            trace(valid_lifecycle_startup_failure_trace()),
            Scenario::LifecycleStartupFailure,
        )
        .expect("startup-failure lifecycle trace should validate");
        validate_jsonl(
            trace(valid_lifecycle_background_quit_trace()),
            Scenario::LifecycleBackgroundQuit,
        )
        .expect("background-quit lifecycle trace should validate");

        let mut invalid_startup_failure = valid_lifecycle_startup_failure_trace();
        let terminal = invalid_startup_failure
            .last_mut()
            .expect("fixture should contain a terminal");
        terminal["data"] = json!({"outcome": "passed", "exit_code": 0});
        assert!(
            validate_jsonl(
                trace(invalid_startup_failure),
                Scenario::LifecycleStartupFailure,
            )
            .is_err()
        );

        let mut missing_background_dispatch = valid_lifecycle_background_quit_trace();
        missing_background_dispatch
            .retain(|record| record["event"] != "background_dispatch_executed");
        renumber(&mut missing_background_dispatch);
        assert!(
            validate_jsonl(
                trace(missing_background_dispatch),
                Scenario::LifecycleBackgroundQuit,
            )
            .is_err()
        );

        let mut delayed_admission = valid_lifecycle_background_quit_trace();
        let admission = delayed_admission.remove(
            delayed_admission
                .iter()
                .position(|record| record["event"] == "background_dispatch_admission")
                .expect("fixture should contain dispatch admission"),
        );
        let after_run = delayed_admission
            .iter()
            .position(|record| record["event"] == "run_returned")
            .expect("fixture should contain run_returned")
            + 1;
        delayed_admission.insert(after_run, admission);
        renumber(&mut delayed_admission);
        validate_jsonl(trace(delayed_admission), Scenario::LifecycleBackgroundQuit)
            .expect("dispatch admission may be recorded after the app run returns");

        let mut delayed_worker_start = valid_lifecycle_background_quit_trace();
        let worker_started = delayed_worker_start.remove(
            delayed_worker_start
                .iter()
                .position(|record| record["event"] == "background_worker_started")
                .expect("fixture should contain worker startup"),
        );
        let after_trigger = delayed_worker_start
            .iter()
            .position(|record| record["event"] == "background_dispatch_triggered")
            .expect("fixture should contain dispatch trigger")
            + 1;
        delayed_worker_start.insert(after_trigger, worker_started);
        renumber(&mut delayed_worker_start);
        validate_jsonl(
            trace(delayed_worker_start),
            Scenario::LifecycleBackgroundQuit,
        )
        .expect("worker startup may race with app-started event delivery");
    }

    #[test]
    fn rejects_lifecycle_traces_without_scenario_evidence() {
        for (scenario, name) in [
            (Scenario::LifecycleClean, "lifecycle-clean"),
            (
                Scenario::LifecycleStartupFailure,
                "lifecycle-startup-failure",
            ),
            (
                Scenario::LifecycleBackgroundQuit,
                "lifecycle-background-quit",
            ),
        ] {
            let trace = trace([
                record(1, name, "scenario_started", json!({})),
                record(2, name, "shutdown_complete", json!({})),
                record(3, name, "run_returned", json!({"result": "ok"})),
                record(
                    4,
                    name,
                    "terminal",
                    json!({"outcome": "passed", "exit_code": 0}),
                ),
            ]);

            assert!(
                validate_jsonl(trace, scenario).is_err(),
                "{name} must require scenario evidence"
            );
        }
    }

    #[test]
    fn rejects_malformed_or_noncontiguous_trace() {
        assert!(validate_jsonl(Cursor::new(b"not json\n"), Scenario::LifecycleClean).is_err());

        let trace = trace([
            record(1, "lifecycle-clean", "scenario_started", json!({})),
            record(3, "lifecycle-clean", "shutdown_complete", json!({})),
            record(
                4,
                "lifecycle-clean",
                "run_returned",
                json!({"result": "ok"}),
            ),
            record(
                5,
                "lifecycle-clean",
                "terminal",
                json!({"outcome": "passed", "exit_code": 0}),
            ),
        ]);
        assert!(validate_jsonl(trace, Scenario::LifecycleClean).is_err());
    }

    #[test]
    fn rejects_unknown_top_level_and_payload_fields() {
        let mut top_level = valid_lifecycle_clean_trace();
        top_level[0]["unexpected"] = json!(true);
        assert!(validate_jsonl(trace(top_level), Scenario::LifecycleClean).is_err());

        let mut payload = valid_lifecycle_clean_trace();
        first_event_mut(&mut payload, "shutdown_complete")["data"]["unexpected"] = json!(true);
        assert!(validate_jsonl(trace(payload), Scenario::LifecycleClean).is_err());
    }

    #[test]
    fn rejects_unknown_events() {
        let mut records = valid_lifecycle_clean_trace();
        records.insert(
            records.len() - 1,
            record(0, "lifecycle-clean", "future_protocol_event", json!({})),
        );
        renumber(&mut records);

        assert!(validate_jsonl(trace(records), Scenario::LifecycleClean).is_err());
    }

    #[test]
    fn rejects_duplicate_lifecycle_milestones() {
        let mut records = valid_lifecycle_clean_trace();
        let duplicate = first_event_mut(&mut records, "shutdown_complete").clone();
        let run_returned = records
            .iter()
            .position(|record| record["event"] == "run_returned")
            .expect("fixture should contain run_returned");
        records.insert(run_returned, duplicate);
        renumber(&mut records);

        assert!(validate_jsonl(trace(records), Scenario::LifecycleClean).is_err());
    }

    #[test]
    fn rejects_failure_evidence_in_passed_traces() {
        for event in [
            "background_dispatch_trigger_failed",
            "background_zero_windows_failed",
            "clipboard_acknowledgement_rejected",
            "clipboard_failed",
            "menu_command_failed",
            "presentation_cancelled",
            "presentation_count_invalid",
            "presentation_delivery_failed",
            "window_cycle_failed",
        ] {
            let mut records = valid_lifecycle_clean_trace();
            records.insert(2, record(0, "lifecycle-clean", event, json!({})));
            renumber(&mut records);
            assert!(
                validate_jsonl(trace(records), Scenario::LifecycleClean).is_err(),
                "passed trace accepted failure evidence event {event}"
            );
        }
    }

    #[test]
    fn rejects_invalid_terminal_or_post_terminal_record() {
        let invalid_terminal = trace([
            record(1, "lifecycle-clean", "scenario_started", json!({})),
            record(2, "lifecycle-clean", "shutdown_complete", json!({})),
            record(
                3,
                "lifecycle-clean",
                "run_returned",
                json!({"result": "ok"}),
            ),
            record(
                4,
                "lifecycle-clean",
                "terminal",
                json!({"outcome": "passed", "exit_code": 1}),
            ),
        ]);
        assert!(validate_jsonl(invalid_terminal, Scenario::LifecycleClean).is_err());

        let post_terminal = trace([
            record(1, "lifecycle-clean", "scenario_started", json!({})),
            record(2, "lifecycle-clean", "shutdown_complete", json!({})),
            record(
                3,
                "lifecycle-clean",
                "run_returned",
                json!({"result": "ok"}),
            ),
            record(
                4,
                "lifecycle-clean",
                "terminal",
                json!({"outcome": "passed", "exit_code": 0}),
            ),
            record(5, "lifecycle-clean", "after_terminal", json!({})),
        ]);
        assert!(validate_jsonl(post_terminal, Scenario::LifecycleClean).is_err());
    }

    #[test]
    fn validates_window_cycle_contract() {
        validate_jsonl(trace(valid_window_cycle_trace()), Scenario::WindowCycle)
            .expect("window-cycle trace should validate");

        let mut missing_last_window_closed = valid_window_cycle_trace();
        missing_last_window_closed.retain(|record| {
            !(record["event"] == "app_event" && record["data"]["kind"] == "last_window_closed")
        });
        renumber(&mut missing_last_window_closed);
        assert!(validate_jsonl(trace(missing_last_window_closed), Scenario::WindowCycle).is_err());
    }

    #[test]
    fn validates_menu_command_contract() {
        validate_jsonl(trace(valid_menu_command_trace()), Scenario::MenuCommand)
            .expect("menu-command trace should validate");

        let mut invalid_projection = valid_menu_command_trace();
        let projection = invalid_projection
            .iter_mut()
            .find(|record| record["event"] == "menu_projection_observed")
            .expect("fixture should contain menu projection");
        projection["data"]["items"][2]["disabled"] = json!(false);
        assert!(validate_jsonl(trace(invalid_projection), Scenario::MenuCommand).is_err());
    }

    #[test]
    fn validates_clipboard_external_handshake_contract() {
        validate_jsonl(trace(valid_clipboard_trace()), Scenario::Clipboard)
            .expect("clipboard handshake trace should validate");

        let mut delayed_worker_start = valid_clipboard_trace();
        let worker_started = delayed_worker_start.remove(
            delayed_worker_start
                .iter()
                .position(|record| record["event"] == "clipboard_worker_started")
                .expect("fixture should contain clipboard worker startup"),
        );
        let acknowledgement = delayed_worker_start
            .iter()
            .position(|record| record["event"] == "clipboard_acknowledged")
            .expect("fixture should contain clipboard acknowledgement");
        delayed_worker_start.insert(acknowledgement, worker_started);
        renumber(&mut delayed_worker_start);
        validate_jsonl(trace(delayed_worker_start), Scenario::Clipboard)
            .expect("worker startup may race after clipboard readiness");

        let mut wrong_payload = valid_clipboard_trace();
        wrong_payload
            .iter_mut()
            .find(|record| record["event"] == "clipboard_ready")
            .expect("fixture should contain clipboard readiness")["data"]["expected_payload"] =
            json!("not-the-expected-payload");
        assert!(validate_jsonl(trace(wrong_payload), Scenario::Clipboard).is_err());

        let mut malformed_address = valid_clipboard_trace();
        malformed_address
            .iter_mut()
            .find(|record| record["event"] == "clipboard_ready")
            .expect("fixture should contain clipboard readiness")["data"]["ack_address"] =
            json!("not-an-address");
        assert!(validate_jsonl(trace(malformed_address), Scenario::Clipboard).is_err());

        let mut non_loopback_address = valid_clipboard_trace();
        non_loopback_address
            .iter_mut()
            .find(|record| record["event"] == "clipboard_ready")
            .expect("fixture should contain clipboard readiness")["data"]["ack_address"] =
            json!("127.0.0.2:49152");
        assert!(validate_jsonl(trace(non_loopback_address), Scenario::Clipboard).is_err());

        let mut missing_acknowledgement = valid_clipboard_trace();
        missing_acknowledgement.retain(|record| record["event"] != "clipboard_acknowledged");
        renumber(&mut missing_acknowledgement);
        assert!(validate_jsonl(trace(missing_acknowledgement), Scenario::Clipboard).is_err());

        let mut rejected_acknowledgement = valid_clipboard_trace();
        let acknowledgement = rejected_acknowledgement
            .iter()
            .position(|record| record["event"] == "clipboard_acknowledged")
            .expect("fixture should contain clipboard acknowledgement");
        rejected_acknowledgement.insert(
            acknowledgement,
            record(
                0,
                "clipboard",
                "clipboard_acknowledgement_rejected",
                json!({"reason": "unexpected_token"}),
            ),
        );
        renumber(&mut rejected_acknowledgement);
        assert!(validate_jsonl(trace(rejected_acknowledgement), Scenario::Clipboard).is_err());

        let mut acknowledgement_before_ready = valid_clipboard_trace();
        let acknowledgement = acknowledgement_before_ready.remove(
            acknowledgement_before_ready
                .iter()
                .position(|record| record["event"] == "clipboard_acknowledged")
                .expect("fixture should contain clipboard acknowledgement"),
        );
        let ready = acknowledgement_before_ready
            .iter()
            .position(|record| record["event"] == "clipboard_ready")
            .expect("fixture should contain clipboard readiness");
        acknowledgement_before_ready.insert(ready, acknowledgement);
        renumber(&mut acknowledgement_before_ready);
        assert!(validate_jsonl(trace(acknowledgement_before_ready), Scenario::Clipboard).is_err());

        let mut quit_before_acknowledgement = valid_clipboard_trace();
        let quit_requested = quit_before_acknowledgement.remove(
            quit_before_acknowledgement
                .iter()
                .position(|record| record["event"] == "quit_requested")
                .expect("fixture should contain quit request"),
        );
        let acknowledgement = quit_before_acknowledgement
            .iter()
            .position(|record| record["event"] == "clipboard_acknowledged")
            .expect("fixture should contain clipboard acknowledgement");
        quit_before_acknowledgement.insert(acknowledgement, quit_requested);
        renumber(&mut quit_before_acknowledgement);
        assert!(validate_jsonl(trace(quit_before_acknowledgement), Scenario::Clipboard).is_err());

        let mut competing_shutdown = valid_clipboard_trace();
        let acknowledgement = competing_shutdown
            .iter()
            .position(|record| record["event"] == "clipboard_acknowledged")
            .expect("fixture should contain clipboard acknowledgement")
            + 1;
        competing_shutdown.insert(
            acknowledgement,
            record(
                0,
                "clipboard",
                "app_event",
                json!({"kind": "shutdown_requested"}),
            ),
        );
        renumber(&mut competing_shutdown);
        assert!(validate_jsonl(trace(competing_shutdown), Scenario::Clipboard).is_err());

        let mut wrong_terminal = valid_clipboard_trace();
        wrong_terminal
            .last_mut()
            .expect("fixture should contain terminal")["data"] =
            json!({"outcome": "failed", "exit_code": 1});
        assert!(validate_jsonl(trace(wrong_terminal), Scenario::Clipboard).is_err());

        let mut post_terminal = valid_clipboard_trace();
        post_terminal.push(record(0, "clipboard", "after_terminal", json!({})));
        renumber(&mut post_terminal);
        assert!(validate_jsonl(trace(post_terminal), Scenario::Clipboard).is_err());
    }

    #[test]
    fn validates_interaction_contracts() {
        validate_jsonl(
            trace(valid_interaction_contracts_trace()),
            Scenario::InteractionContracts,
        )
        .expect("interaction-contracts trace should validate");

        for (event, field, wrong_value) in [
            ("scenario_started", "runner", json!("headless")),
            ("window_opened", "title", json!("Wrong")),
        ] {
            let mut missing = valid_interaction_contracts_trace();
            first_event_mut(&mut missing, event)["data"]
                .as_object_mut()
                .expect("fixture data must be an object")
                .remove(field);
            assert!(
                validate_jsonl(trace(missing), Scenario::InteractionContracts).is_err(),
                "missing {event}.{field} must fail"
            );

            let mut wrong = valid_interaction_contracts_trace();
            first_event_mut(&mut wrong, event)["data"][field] = wrong_value;
            assert!(
                validate_jsonl(trace(wrong), Scenario::InteractionContracts).is_err(),
                "wrong {event}.{field} must fail"
            );
        }

        let mut missing_accessibility = valid_interaction_contracts_trace();
        missing_accessibility.retain(|record| record["event"] != "accessibility_verified");
        renumber(&mut missing_accessibility);
        assert!(
            validate_jsonl(trace(missing_accessibility), Scenario::InteractionContracts,).is_err()
        );

        let mut wrong_selection = valid_interaction_contracts_trace();
        first_event_mut(&mut wrong_selection, "focus_text_verified")["data"]["selection_utf16"] =
            json!([0, 1]);
        assert!(validate_jsonl(trace(wrong_selection), Scenario::InteractionContracts).is_err());

        let mut wrong_scale = valid_interaction_contracts_trace();
        first_event_mut(&mut wrong_scale, "scale_verified")["data"]["tested_scale_factors"] =
            json!([1.0]);
        assert!(validate_jsonl(trace(wrong_scale), Scenario::InteractionContracts).is_err());

        for field in [
            "button_label",
            "button_supports_click",
            "focused_label",
            "focused_role",
            "focused_supports_focus",
            "focused_value",
            "node_count",
            "published",
            "toggle_label",
            "toggle_state",
        ] {
            let mut invalid = valid_interaction_contracts_trace();
            first_event_mut(&mut invalid, "accessibility_verified")["data"][field] = Value::Null;
            assert!(
                validate_jsonl(trace(invalid), Scenario::InteractionContracts).is_err(),
                "accessibility field {field} must be exact"
            );
        }
    }

    #[test]
    fn validates_target_profiles_source_blind() {
        for profile in validation_profiles() {
            let valid = profiled_lifecycle_clean_trace(profile);
            for expected_profile in validation_profiles() {
                let result = validate_jsonl_with_profile(
                    trace(valid.clone()),
                    Scenario::LifecycleClean,
                    Some(expected_profile),
                );
                assert_eq!(
                    result.is_ok(),
                    expected_profile == profile,
                    "profile {expected_profile} did not correctly validate {profile} evidence"
                );
            }

            let mut wrong_handle = profiled_lifecycle_clean_trace(profile);
            first_event_mut(&mut wrong_handle, "native_window_handle")["data"]["kind"] =
                json!("unsupported");
            assert_profile_rejects(profile, wrong_handle, "wrong native handle");

            let mut wrong_display = profiled_lifecycle_clean_trace(profile);
            first_event_mut(&mut wrong_display, "native_display_handle")["data"]["kind"] =
                json!("unsupported");
            assert_profile_rejects(profile, wrong_display, "wrong native display handle");

            let mut wrong_renderer = profiled_lifecycle_clean_trace(profile);
            first_event_mut(&mut wrong_renderer, "renderer_info")["data"]["renderer_info"]["renderer"] =
                json!("unsupported");
            assert_profile_rejects(profile, wrong_renderer, "wrong renderer");

            let mut wrong_backend = profiled_lifecycle_clean_trace(profile);
            first_event_mut(&mut wrong_backend, "renderer_info")["data"]["renderer_info"]["backend"] =
                json!("unsupported");
            assert_profile_rejects(profile, wrong_backend, "wrong backend");

            let mut wrong_selection = profiled_lifecycle_clean_trace(profile);
            first_event_mut(&mut wrong_selection, "renderer_info")["data"]["renderer_info"]["selection"] =
                json!("unsupported");
            assert_profile_rejects(profile, wrong_selection, "wrong selection");

            let mut wrong_adapter_type = profiled_lifecycle_clean_trace(profile);
            first_event_mut(&mut wrong_adapter_type, "renderer_info")["data"]["renderer_info"]["adapter_type"] =
                json!("unknown");
            assert_profile_rejects(profile, wrong_adapter_type, "wrong adapter type");

            let mut wrong_adapter_name = profiled_lifecycle_clean_trace(profile);
            first_event_mut(&mut wrong_adapter_name, "renderer_info")["data"]["renderer_info"]["adapter_name"] =
                json!(if matches!(
                    profile,
                    ValidationProfile::LinuxX11Lavapipe | ValidationProfile::LinuxWaylandLavapipe
                ) {
                    "unsupported adapter"
                } else if profile == ValidationProfile::WindowsWarp {
                    "unsupported adapter"
                } else {
                    ""
                });
            assert_profile_rejects(profile, wrong_adapter_name, "wrong adapter name");

            let mut insufficient_presentation = profiled_lifecycle_clean_trace(profile);
            first_event_mut(&mut insufficient_presentation, "frame_presented")["data"]["presentation_evidence"] =
                json!(if matches!(
                    profile,
                    ValidationProfile::LinuxX11Lavapipe | ValidationProfile::LinuxWaylandLavapipe
                ) {
                    "backend_accepted"
                } else {
                    "api_submitted"
                });
            assert_profile_rejects(
                profile,
                insufficient_presentation,
                "insufficient presentation evidence",
            );
        }
    }

    #[test]
    fn profiles_reject_extra_invalid_native_evidence() {
        let profile = ValidationProfile::MacosMetal;
        let mut records = valid_window_cycle_trace();
        apply_profile(&mut records, profile);
        records.insert(
            2,
            record(
                0,
                "window-cycle",
                "native_window_handle",
                json!({"kind": "wayland"}),
            ),
        );
        renumber(&mut records);

        validate_jsonl(trace(records.clone()), Scenario::WindowCycle)
            .expect("generic validation may ignore unrelated valid native evidence");
        assert!(
            validate_jsonl_with_profile(trace(records), Scenario::WindowCycle, Some(profile))
                .is_err(),
            "profile validation must inspect every native evidence record"
        );
    }

    #[test]
    fn profiles_preserve_zero_window_background_quit_contract() {
        for profile in validation_profiles() {
            validate_jsonl_with_profile(
                trace(valid_lifecycle_background_quit_trace()),
                Scenario::LifecycleBackgroundQuit,
                Some(profile),
            )
            .expect("profile must not require native evidence from background quit");
        }

        for event in [
            "native_window_handle",
            "native_display_handle",
            "renderer_info",
            "frame_presented",
            "window_opened",
        ] {
            let mut records = valid_lifecycle_background_quit_trace();
            records.insert(2, record(0, "lifecycle-background-quit", event, json!({})));
            renumber(&mut records);
            assert!(
                validate_jsonl_with_profile(
                    trace(records),
                    Scenario::LifecycleBackgroundQuit,
                    Some(ValidationProfile::MacosMetal),
                )
                .is_err(),
                "background quit must reject {event} under a profile"
            );
        }
    }

    #[test]
    fn profiles_require_exact_native_evidence_groups() {
        let profile = ValidationProfile::MacosMetal;
        for (scenario, mut records) in [
            (Scenario::LifecycleClean, valid_lifecycle_clean_trace()),
            (Scenario::WindowCycle, valid_window_cycle_trace()),
            (Scenario::MenuCommand, valid_menu_command_trace()),
            (Scenario::Clipboard, valid_clipboard_trace()),
            (
                Scenario::InteractionContracts,
                valid_interaction_contracts_trace(),
            ),
        ] {
            apply_profile(&mut records, profile);
            validate_jsonl_with_profile(trace(records), scenario, Some(profile))
                .expect("profiled native scenario should have its required evidence groups");
        }

        for profile in validation_profiles() {
            for (scenario, records) in [
                (
                    Scenario::LifecycleStartupFailure,
                    valid_lifecycle_startup_failure_trace(),
                ),
                (
                    Scenario::LifecycleBackgroundQuit,
                    valid_lifecycle_background_quit_trace(),
                ),
            ] {
                validate_jsonl_with_profile(trace(records), scenario, Some(profile))
                    .expect("profiled zero-window scenario must not require native evidence");
            }
        }

        for (event, reason) in [
            (
                "native_display_handle",
                "a native group without a display handle",
            ),
            ("renderer_info", "a native group without renderer info"),
        ] {
            let mut incomplete_group = profiled_lifecycle_clean_trace(profile);
            incomplete_group.retain(|record| record["event"] != event);
            renumber(&mut incomplete_group);
            assert_profile_rejects(profile, incomplete_group, reason);
        }

        let mut wrong_order = profiled_lifecycle_clean_trace(profile);
        let display = wrong_order
            .iter()
            .position(|record| record["event"] == "native_display_handle")
            .expect("fixture should contain native display evidence");
        let renderer = wrong_order
            .iter()
            .position(|record| record["event"] == "renderer_info")
            .expect("fixture should contain renderer evidence");
        wrong_order.swap(display, renderer);
        renumber(&mut wrong_order);
        assert_profile_rejects(profile, wrong_order, "native evidence in the wrong order");

        let mut extra_group = valid_window_cycle_trace();
        apply_profile(&mut extra_group, profile);
        let insertion = extra_group
            .iter()
            .position(|record| record["event"] == "window_cycle_verified")
            .expect("fixture should contain window-cycle verification");
        extra_group.splice(
            insertion..insertion,
            native_evidence_group("window-cycle", profile),
        );
        renumber(&mut extra_group);
        validate_jsonl(trace(extra_group.clone()), Scenario::WindowCycle)
            .expect("generic validation accepts additional otherwise-valid evidence");
        assert!(
            validate_jsonl_with_profile(trace(extra_group), Scenario::WindowCycle, Some(profile))
                .is_err(),
            "profile validation must reject an extra complete native evidence group"
        );

        for (scenario, records) in [
            (
                Scenario::LifecycleStartupFailure,
                valid_lifecycle_startup_failure_trace(),
            ),
            (
                Scenario::LifecycleBackgroundQuit,
                valid_lifecycle_background_quit_trace(),
            ),
        ] {
            for event in [
                "native_window_handle",
                "native_display_handle",
                "renderer_info",
                "frame_presented",
            ] {
                let mut records = records.clone();
                records.insert(2, record(0, scenario.as_str(), event, json!({})));
                renumber(&mut records);
                assert!(
                    validate_jsonl_with_profile(trace(records), scenario, Some(profile)).is_err(),
                    "{scenario} must reject injected {event} evidence"
                );
            }
        }
    }

    #[test]
    fn wayland_clipboard_profile_requires_real_input_ordering() {
        let profile = ValidationProfile::LinuxWaylandLavapipe;
        let records = profiled_wayland_clipboard_trace();
        validate_jsonl_with_profile(trace(records.clone()), Scenario::Clipboard, Some(profile))
            .expect("Wayland clipboard evidence should satisfy the profile");

        for event in [
            "wayland_input_requested",
            "wayland_key_down_observed",
            "wayland_input_completed",
        ] {
            let mut missing = records.clone();
            missing.retain(|record| record["event"] != event);
            renumber(&mut missing);
            assert!(
                validate_jsonl_with_profile(trace(missing), Scenario::Clipboard, Some(profile))
                    .is_err(),
                "Wayland clipboard profile accepted missing {event} evidence"
            );
        }

        let mut wrong_order = records;
        let key_down = wrong_order
            .iter()
            .position(|record| record["event"] == "wayland_key_down_observed")
            .expect("fixture should contain key-down evidence");
        let ready = wrong_order
            .iter()
            .position(|record| record["event"] == "clipboard_ready")
            .expect("fixture should contain clipboard readiness");
        wrong_order.swap(key_down, ready);
        renumber(&mut wrong_order);
        assert!(
            validate_jsonl_with_profile(trace(wrong_order), Scenario::Clipboard, Some(profile))
                .is_err(),
            "Wayland clipboard profile accepted clipboard write before key down"
        );
    }

    #[test]
    fn profiles_reject_second_window_cycle_evidence_mismatches() {
        let profile = ValidationProfile::LinuxX11Lavapipe;
        for event in [
            "native_window_handle",
            "native_display_handle",
            "renderer_info",
            "frame_presented",
        ] {
            let mut records = valid_window_cycle_trace();
            apply_profile(&mut records, profile);
            match event {
                "native_window_handle" => {
                    event_mut(&mut records, event, 1)["data"]["kind"] = json!("xlib")
                }
                "native_display_handle" => {
                    event_mut(&mut records, event, 1)["data"]["kind"] = json!("wayland")
                }
                "renderer_info" => {
                    event_mut(&mut records, event, 1)["data"]["renderer_info"]["adapter_name"] =
                        json!("unsupported adapter")
                }
                "frame_presented" => {
                    event_mut(&mut records, event, 1)["data"]["presentation_evidence"] =
                        json!("backend_accepted")
                }
                _ => unreachable!("test only includes native evidence events"),
            }
            assert!(
                validate_jsonl_with_profile(trace(records), Scenario::WindowCycle, Some(profile))
                    .is_err(),
                "profile must reject a mismatched second {event} record"
            );
        }
    }

    #[test]
    fn profile_adapter_names_are_trimmed_and_linux_is_case_insensitive() {
        for profile in validation_profiles() {
            let mut blank_name = profiled_lifecycle_clean_trace(profile);
            first_event_mut(&mut blank_name, "renderer_info")["data"]["renderer_info"]["adapter_name"] =
                json!(" \t ");
            assert_profile_rejects(profile, blank_name, "a whitespace adapter name");
        }

        let profile = ValidationProfile::LinuxX11Lavapipe;
        let mut mixed_case_lavapipe = profiled_lifecycle_clean_trace(profile);
        first_event_mut(&mut mixed_case_lavapipe, "renderer_info")["data"]["renderer_info"]["adapter_name"] =
            json!("LaVaPiPe");
        validate_jsonl_with_profile(
            trace(mixed_case_lavapipe),
            Scenario::LifecycleClean,
            Some(profile),
        )
        .expect("Linux profile should accept a mixed-case lavapipe adapter name");

        let windows_profile = ValidationProfile::WindowsWarp;
        let mut basic_render_driver = profiled_lifecycle_clean_trace(windows_profile);
        first_event_mut(&mut basic_render_driver, "renderer_info")["data"]["renderer_info"]["adapter_name"] =
            json!("Microsoft Basic Render Driver");
        validate_jsonl_with_profile(
            trace(basic_render_driver),
            Scenario::LifecycleClean,
            Some(windows_profile),
        )
        .expect("Windows profile should accept the WARP-correlated Basic Render Driver name");

        let mut xlib = profiled_lifecycle_clean_trace(profile);
        first_event_mut(&mut xlib, "native_window_handle")["data"]["kind"] = json!("xlib");
        assert_profile_rejects(profile, xlib, "an Xlib native handle");

        let mut missing_adapter_type = profiled_lifecycle_clean_trace(profile);
        first_event_mut(&mut missing_adapter_type, "renderer_info")["data"]["renderer_info"]["adapter_type"] =
            Value::Null;
        assert_profile_rejects(profile, missing_adapter_type, "a missing adapter type");
    }

    fn validation_profiles() -> [ValidationProfile; 4] {
        [
            ValidationProfile::MacosMetal,
            ValidationProfile::WindowsWarp,
            ValidationProfile::LinuxX11Lavapipe,
            ValidationProfile::LinuxWaylandLavapipe,
        ]
    }

    fn profiled_lifecycle_clean_trace(profile: ValidationProfile) -> Vec<Value> {
        let mut records = valid_lifecycle_clean_trace();
        apply_profile(&mut records, profile);
        records
    }

    fn profiled_wayland_clipboard_trace() -> Vec<Value> {
        let profile = ValidationProfile::LinuxWaylandLavapipe;
        let mut records = valid_clipboard_trace();
        apply_profile(&mut records, profile);
        let frame = records
            .iter()
            .position(|record| record["event"] == "frame_presented")
            .expect("fixture should contain presentation evidence");
        records.insert(
            frame + 1,
            record(
                0,
                "clipboard",
                "wayland_input_requested",
                json!({"protocol": "weston_test", "key": "a"}),
            ),
        );
        let ready = records
            .iter()
            .position(|record| record["event"] == "clipboard_ready")
            .expect("fixture should contain clipboard readiness");
        records.insert(
            ready,
            record(
                0,
                "clipboard",
                "wayland_key_down_observed",
                json!({"key": "a", "source": "weston_test"}),
            ),
        );
        let ready = records
            .iter()
            .position(|record| record["event"] == "clipboard_ready")
            .expect("fixture should contain clipboard readiness");
        records.insert(
            ready + 1,
            record(
                0,
                "clipboard",
                "wayland_input_completed",
                json!({"result": "key_press_delivered"}),
            ),
        );
        renumber(&mut records);
        records
    }

    fn apply_profile(records: &mut [Value], profile: ValidationProfile) {
        let (handle, display, selection, renderer, backend, adapter_name, adapter_type, evidence) =
            match profile {
                ValidationProfile::MacosMetal => (
                    "app_kit",
                    "app_kit",
                    "default",
                    "metal",
                    "Metal",
                    "Apple M-series",
                    "hardware",
                    "backend_accepted",
                ),
                ValidationProfile::WindowsWarp => (
                    "win32",
                    "windows",
                    "software",
                    "direct3d11",
                    "Direct3D11",
                    "Microsoft WARP Adapter",
                    "software",
                    "backend_accepted",
                ),
                ValidationProfile::LinuxX11Lavapipe => (
                    "xcb",
                    "xcb",
                    "software",
                    "wgpu",
                    "Vulkan",
                    "llvmpipe (LLVM 20.0.0)",
                    "software",
                    "api_submitted",
                ),
                ValidationProfile::LinuxWaylandLavapipe => (
                    "wayland",
                    "wayland",
                    "software",
                    "wgpu",
                    "Vulkan",
                    "lavapipe (LLVM 20.0.0)",
                    "software",
                    "api_submitted",
                ),
            };

        for record in records {
            match record["event"].as_str() {
                Some("native_window_handle") => record["data"]["kind"] = json!(handle),
                Some("native_display_handle") => record["data"]["kind"] = json!(display),
                Some("renderer_info") => {
                    let info = &mut record["data"]["renderer_info"];
                    info["selection"] = json!(selection);
                    info["renderer"] = json!(renderer);
                    info["backend"] = json!(backend);
                    info["adapter_name"] = json!(adapter_name);
                    info["adapter_type"] = json!(adapter_type);
                }
                Some("frame_presented") => {
                    record["data"]["presentation_evidence"] = json!(evidence)
                }
                _ => {}
            }
        }
    }

    fn native_evidence_group(scenario: &str, profile: ValidationProfile) -> Vec<Value> {
        let mut records = vec![
            record(0, scenario, "native_window_handle", json!({})),
            record(0, scenario, "native_display_handle", json!({})),
            record(0, scenario, "renderer_info", json!({"renderer_info": {}})),
            record(
                0,
                scenario,
                "frame_presented",
                json!({"presentation_evidence": "api_submitted", "count": 1}),
            ),
        ];
        apply_profile(&mut records, profile);
        records
    }

    fn first_event_mut<'a>(records: &'a mut [Value], event: &str) -> &'a mut Value {
        event_mut(records, event, 0)
    }

    fn event_mut<'a>(records: &'a mut [Value], event: &str, occurrence: usize) -> &'a mut Value {
        records
            .iter_mut()
            .filter(|record| record["event"] == event)
            .nth(occurrence)
            .expect("fixture should contain event occurrence")
    }

    fn assert_profile_rejects(profile: ValidationProfile, records: Vec<Value>, reason: &str) {
        assert!(
            validate_jsonl_with_profile(trace(records), Scenario::LifecycleClean, Some(profile))
                .is_err(),
            "profile {profile} accepted {reason}"
        );
    }

    fn record(sequence: u64, scenario: &str, event: &str, data: Value) -> Value {
        json!({
            "schema": SCHEMA_VERSION,
            "sequence": sequence,
            "scenario": scenario,
            "event": event,
            "data": data,
        })
    }

    fn trace(records: impl IntoIterator<Item = Value>) -> Cursor<Vec<u8>> {
        let output = records
            .into_iter()
            .map(|record| serde_json::to_string(&record).expect("fixture should serialize"))
            .collect::<Vec<_>>()
            .join("\n");
        Cursor::new(output.into_bytes())
    }

    fn renumber(records: &mut [Value]) {
        for (index, record) in records.iter_mut().enumerate() {
            record["sequence"] = json!(index + 1);
        }
    }

    fn native_handle(sequence: u64, scenario: &str) -> Value {
        record(
            sequence,
            scenario,
            "native_window_handle",
            json!({"kind": "app_kit"}),
        )
    }

    fn native_display_handle(sequence: u64, scenario: &str) -> Value {
        record(
            sequence,
            scenario,
            "native_display_handle",
            json!({"kind": "app_kit"}),
        )
    }

    fn renderer_info(sequence: u64, scenario: &str) -> Value {
        record(
            sequence,
            scenario,
            "renderer_info",
            json!({
                "renderer_info": {
                    "selection": "default",
                    "renderer": "metal",
                    "backend": "Metal",
                    "adapter_name": "Test Adapter",
                }
            }),
        )
    }

    fn normal_lifecycle_prefix(scenario: &str) -> Vec<Value> {
        vec![
            record(
                0,
                scenario,
                "scenario_started",
                json!({"exit_policy": "explicit"}),
            ),
            record(0, scenario, "startup_transaction_started", json!({})),
            native_handle(0, scenario),
            native_display_handle(0, scenario),
            renderer_info(0, scenario),
            record(0, scenario, "window_opened", json!({"key": "main"})),
        ]
    }

    fn requested_shutdown_tail(scenario: &str) -> Vec<Value> {
        vec![
            record(
                0,
                scenario,
                "app_event",
                json!({"kind": "shutdown_requested"}),
            ),
            record(
                0,
                scenario,
                "shutdown_started",
                json!({"reason": "requested"}),
            ),
            record(0, scenario, "app_event", json!({"kind": "will_exit"})),
            record(0, scenario, "will_exit", json!({})),
            record(0, scenario, "shutdown_complete", json!({})),
            record(0, scenario, "run_returned", json!({"result": "ok"})),
        ]
    }

    fn valid_lifecycle_clean_trace() -> Vec<Value> {
        let scenario = "lifecycle-clean";
        let mut records = normal_lifecycle_prefix(scenario);
        records.extend([
            record(0, scenario, "app_event", json!({"kind": "started"})),
            record(
                0,
                scenario,
                "frame_presented",
                json!({"presentation_evidence": "backend_accepted", "count": 1}),
            ),
            record(
                0,
                scenario,
                "quit_requested",
                json!({"source": "first_presentation"}),
            ),
        ]);
        records.extend(requested_shutdown_tail(scenario));
        records.push(record(
            0,
            scenario,
            "terminal",
            json!({"outcome": "passed", "exit_code": 0}),
        ));
        renumber(&mut records);
        records
    }

    fn valid_lifecycle_startup_failure_trace() -> Vec<Value> {
        let scenario = "lifecycle-startup-failure";
        let mut records = vec![
            record(
                0,
                scenario,
                "scenario_started",
                json!({"exit_policy": "explicit"}),
            ),
            record(
                0,
                scenario,
                "startup_failure_triggered",
                json!({"source": "transactional_start"}),
            ),
            record(
                0,
                scenario,
                "app_event",
                json!({"kind": "shutdown_requested"}),
            ),
            record(
                0,
                scenario,
                "shutdown_started",
                json!({"reason": "startup_failure"}),
            ),
            record(0, scenario, "app_event", json!({"kind": "will_exit"})),
            record(0, scenario, "will_exit", json!({})),
            record(0, scenario, "shutdown_complete", json!({})),
            record(0, scenario, "run_returned", json!({"result": "error"})),
            record(
                0,
                scenario,
                "terminal",
                json!({"outcome": "expected_startup_failure", "exit_code": 2}),
            ),
        ];
        renumber(&mut records);
        records
    }

    fn valid_lifecycle_background_quit_trace() -> Vec<Value> {
        let scenario = "lifecycle-background-quit";
        let mut records = vec![
            record(
                0,
                scenario,
                "scenario_started",
                json!({"exit_policy": "when_idle"}),
            ),
            record(0, scenario, "startup_transaction_started", json!({})),
            record(0, scenario, "background_worker_started", json!({})),
            record(0, scenario, "app_event", json!({"kind": "started"})),
            record(
                0,
                scenario,
                "background_dispatch_triggered",
                json!({"source": "app_started"}),
            ),
            record(
                0,
                scenario,
                "background_dispatch_admission",
                json!({"accepted": true, "result": "queued"}),
            ),
            record(
                0,
                scenario,
                "background_dispatch_executed",
                json!({"result": "executed"}),
            ),
            record(
                0,
                scenario,
                "background_zero_windows_verified",
                json!({"window_count": 0}),
            ),
            record(
                0,
                scenario,
                "background_hold_released",
                json!({"reason": "lifecycle-background-quit"}),
            ),
        ];
        records.extend(requested_shutdown_tail(scenario));
        records.extend([
            record(
                0,
                scenario,
                "background_worker_joined",
                json!({"dispatch_admission": "accepted"}),
            ),
            record(
                0,
                scenario,
                "terminal",
                json!({"outcome": "passed", "exit_code": 0}),
            ),
        ]);
        renumber(&mut records);
        records
    }

    fn valid_window_cycle_trace() -> Vec<Value> {
        let scenario = "window-cycle";
        let mut records = vec![
            record(
                1,
                scenario,
                "scenario_started",
                json!({"exit_policy": "explicit"}),
            ),
            record(2, scenario, "startup_transaction_started", json!({})),
            native_handle(3, scenario),
            native_display_handle(4, scenario),
            renderer_info(5, scenario),
            record(
                5,
                scenario,
                "window_opened",
                json!({"key": "window-cycle-initial"}),
            ),
            record(
                6,
                scenario,
                "frame_presented",
                json!({"presentation_evidence": "backend_accepted", "count": 1}),
            ),
            record(
                7,
                scenario,
                "window_close_requested",
                json!({"generation": 1}),
            ),
            record(
                8,
                scenario,
                "app_event",
                json!({"kind": "last_window_closed"}),
            ),
            record(
                9,
                scenario,
                "window_closed",
                json!({"generation": 1, "source": "last_window_closed"}),
            ),
            record(
                10,
                scenario,
                "explicit_hold_verified",
                json!({"window_count": 0}),
            ),
            native_handle(11, scenario),
            native_display_handle(12, scenario),
            renderer_info(13, scenario),
            record(
                13,
                scenario,
                "window_opened",
                json!({"key": "window-cycle-recreated"}),
            ),
            record(
                14,
                scenario,
                "window_recreated",
                json!({"generation": 2, "key": "window-cycle-recreated"}),
            ),
            record(
                15,
                scenario,
                "frame_presented",
                json!({"presentation_evidence": "backend_accepted", "count": 1}),
            ),
            record(
                16,
                scenario,
                "window_cycle_verified",
                json!({
                    "key": "window-cycle",
                    "opened": 2,
                    "presentations": 2,
                    "closed": 1,
                    "zero_windows": true,
                }),
            ),
            record(17, scenario, "quit_requested", json!({})),
            record(
                18,
                scenario,
                "app_event",
                json!({"kind": "shutdown_requested"}),
            ),
            record(
                19,
                scenario,
                "shutdown_started",
                json!({"reason": "requested"}),
            ),
            record(20, scenario, "app_event", json!({"kind": "will_exit"})),
            record(21, scenario, "will_exit", json!({})),
            record(22, scenario, "shutdown_complete", json!({})),
            record(23, scenario, "run_returned", json!({"result": "ok"})),
            record(
                24,
                scenario,
                "terminal",
                json!({"outcome": "passed", "exit_code": 0}),
            ),
        ];
        renumber(&mut records);
        records
    }

    fn valid_menu_command_trace() -> Vec<Value> {
        let scenario = "menu-command";
        let mut records = vec![
            record(
                1,
                scenario,
                "scenario_started",
                json!({"exit_policy": "explicit"}),
            ),
            record(2, scenario, "startup_transaction_started", json!({})),
            record(
                3,
                scenario,
                "menu_commands_registered",
                json!({
                    "menu": "Conformance",
                    "command_ids": [
                        "conformance.menu-checked",
                        "conformance.menu-unchecked",
                        "conformance.menu-disabled",
                    ],
                }),
            ),
            native_handle(4, scenario),
            native_display_handle(5, scenario),
            renderer_info(6, scenario),
            record(7, scenario, "window_opened", json!({"key": "menu-command"})),
            record(
                7,
                scenario,
                "frame_presented",
                json!({"presentation_evidence": "backend_accepted", "count": 1}),
            ),
            record(
                8,
                scenario,
                "menu_projection_observed",
                json!({
                    "projection": "owned_menu_model",
                    "items": [
                        {
                            "label": "Checked Conformance Command",
                            "checked": true,
                            "disabled": false,
                        },
                        {
                            "label": "Unchecked Conformance Command",
                            "checked": false,
                            "disabled": false,
                        },
                        {
                            "label": "Disabled Conformance Command",
                            "checked": false,
                            "disabled": true,
                        },
                    ],
                }),
            ),
            record(
                9,
                scenario,
                "menu_command_dispatched",
                json!({
                    "command_id": "conformance.menu-checked",
                    "dispatch": "app_action",
                    "callback_count": 1,
                }),
            ),
            record(
                10,
                scenario,
                "menu_command_verified",
                json!({"registered": true, "dispatched": true}),
            ),
            record(11, scenario, "quit_requested", json!({})),
            record(
                12,
                scenario,
                "app_event",
                json!({"kind": "shutdown_requested"}),
            ),
            record(
                13,
                scenario,
                "shutdown_started",
                json!({"reason": "requested"}),
            ),
            record(14, scenario, "app_event", json!({"kind": "will_exit"})),
            record(15, scenario, "will_exit", json!({})),
            record(16, scenario, "shutdown_complete", json!({})),
            record(17, scenario, "run_returned", json!({"result": "ok"})),
            record(
                18,
                scenario,
                "terminal",
                json!({"outcome": "passed", "exit_code": 0}),
            ),
        ];
        renumber(&mut records);
        records
    }

    fn valid_interaction_contracts_trace() -> Vec<Value> {
        let scenario = "interaction-contracts";
        let mut records = normal_lifecycle_prefix(scenario);
        records[0]["data"] = json!({"runner": "native", "exit_policy": "explicit"});
        records[5]["data"] = json!({
            "key": "interaction-contracts",
            "title": "Interaction Contracts",
        });
        records.extend([
            record(0, scenario, "app_event", json!({"kind": "started"})),
            record(
                0,
                scenario,
                "frame_presented",
                json!({"presentation_evidence": "backend_accepted", "count": 1}),
            ),
            record(
                0,
                scenario,
                "focus_text_verified",
                json!({
                    "activation_order": ["first", "second"],
                    "inserted": "!",
                    "selection_utf16": [0, 7],
                    "value": "!second",
                }),
            ),
            record(
                0,
                scenario,
                "composition_verified",
                json!({
                    "committed_value": "漢",
                    "marked_range_utf16": [0, 1],
                    "selection_utf16": [0, 1],
                    "terminal": "unmark",
                }),
            ),
            record(
                0,
                scenario,
                "scale_verified",
                json!({
                    "native_scale_factor": 2.0,
                    "tested_scale_factors": [1.25, 1.5, 2.0],
                }),
            ),
            record(
                0,
                scenario,
                "accessibility_verified",
                json!({
                    "button_label": "Interaction action",
                    "button_supports_click": true,
                    "focused_label": "Interaction second",
                    "focused_role": "text_input",
                    "focused_supports_focus": true,
                    "focused_value": "!second",
                    "node_count": 5,
                    "published": ["button", "switch", "text_input"],
                    "toggle_label": "Interaction toggle",
                    "toggle_state": "true",
                }),
            ),
            record(
                0,
                scenario,
                "quit_requested",
                json!({"source": "interaction_contracts_verified"}),
            ),
        ]);
        records.extend(requested_shutdown_tail(scenario));
        records.push(record(
            0,
            scenario,
            "terminal",
            json!({"outcome": "passed", "exit_code": 0}),
        ));
        renumber(&mut records);
        records
    }

    fn valid_clipboard_trace() -> Vec<Value> {
        let scenario = "clipboard";
        let mut records = vec![
            record(
                0,
                scenario,
                "scenario_started",
                json!({"exit_policy": "explicit"}),
            ),
            record(0, scenario, "startup_transaction_started", json!({})),
            native_handle(0, scenario),
            native_display_handle(0, scenario),
            renderer_info(0, scenario),
            record(0, scenario, "window_opened", json!({"key": "clipboard"})),
            record(0, scenario, "clipboard_worker_started", json!({})),
            record(
                0,
                scenario,
                "frame_presented",
                json!({"presentation_evidence": "backend_accepted", "count": 1}),
            ),
            record(
                0,
                scenario,
                "clipboard_ready",
                json!({
                    "expected_payload": CLIPBOARD_EXPECTED_PAYLOAD,
                    "ack_address": "127.0.0.1:49152",
                }),
            ),
            record(
                0,
                scenario,
                "clipboard_acknowledged",
                json!({"acknowledgement": "verified"}),
            ),
            record(
                0,
                scenario,
                "quit_requested",
                json!({"source": "external_clipboard_acknowledgement"}),
            ),
        ];
        records.extend(requested_shutdown_tail(scenario));
        records.extend([
            record(
                0,
                scenario,
                "clipboard_worker_joined",
                json!({"result": "acknowledgement_dispatched"}),
            ),
            record(
                0,
                scenario,
                "terminal",
                json!({"outcome": "passed", "exit_code": 0}),
            ),
        ]);
        renumber(&mut records);
        records
    }
}
