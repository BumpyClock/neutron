use serde_json::json;

use super::{
    ParsedRecord, find_event_after, frame_count_is_one, validate_native_display_handle,
    validate_native_handle, validate_renderer_info, validate_requested_shutdown,
};

pub(super) fn validate(records: &[ParsedRecord]) -> anyhow::Result<()> {
    if records[0].data["runner"] != "native" || records[0].data["exit_policy"] != "explicit" {
        anyhow::bail!("interaction-contracts must declare native runner and explicit exit policy");
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
        data["key"] == "interaction-contracts" && data["title"] == "Interaction Contracts"
    })? + 1;
    cursor = find_event_after(records, cursor, "app_event", |data| {
        data["kind"] == "started"
    })? + 1;
    cursor = find_event_after(records, cursor, "frame_presented", frame_count_is_one)? + 1;
    cursor = find_event_after(records, cursor, "focus_text_verified", |data| {
        data["activation_order"] == json!(["first", "second"])
            && data["inserted"] == "!"
            && data["selection_utf16"] == json!([0, 7])
            && data["value"] == "!second"
    })? + 1;
    cursor = find_event_after(records, cursor, "composition_verified", |data| {
        data["committed_value"] == "漢"
            && data["marked_range_utf16"] == json!([0, 1])
            && data["selection_utf16"] == json!([0, 1])
            && data["terminal"] == "unmark"
    })? + 1;
    cursor = find_event_after(records, cursor, "scale_verified", |data| {
        data["native_scale_factor"]
            .as_f64()
            .is_some_and(|scale| scale.is_finite() && scale > 0.0)
            && data["tested_scale_factors"] == json!([1.25, 1.5, 2.0])
    })? + 1;
    cursor = find_event_after(records, cursor, "accessibility_verified", |data| {
        data["button_label"] == "Interaction action"
            && data["button_supports_click"] == true
            && data["focused_label"] == "Interaction second"
            && data["focused_role"] == "text_input"
            && data["focused_supports_focus"] == true
            && data["focused_value"] == "!second"
            && data["node_count"].as_u64().is_some_and(|count| count >= 4)
            && data["published"] == json!(["button", "switch", "text_input"])
            && data["toggle_label"] == "Interaction toggle"
            && data["toggle_state"] == "true"
    })? + 1;
    let quit_requested = find_event_after(records, cursor, "quit_requested", |data| {
        data["source"] == "interaction_contracts_verified"
    })?;
    validate_requested_shutdown(records, quit_requested)?;
    Ok(())
}
