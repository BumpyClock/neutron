//! Strict validation for the `story-smoke` stream.
//!
//! `neutron-story --smoke` writes this stream itself when Stage 1 sets
//! `GPUI_STAGE1_STORY_EVIDENCE_PATH`; this runner never produces it. The
//! contract is deliberately narrow: it proves that the real typed
//! `DesktopApp` declaration reached its primary Gallery surface, installed a
//! native menu projection the running process could read back, installed its
//! embedded themes byte-for-byte, presented exactly once, requested quit only
//! after that observation, and shut down through AppShell with `Ok`.
//!
//! The expectations below are hardcoded on purpose: they are the stable
//! action type names, displayed labels, and platform menu names the story
//! declaration is contracted to produce. The emitter never reconstructs them
//! — it reads the installed model back through `gpui::App::get_menus` — so a
//! removed command, a dropped menu contribution, a disabled Settings or About
//! feature, or a menu projection that never reached the platform all fail
//! here.
//!
//! Native window handles, renderer selection, presentation backends,
//! clipboard, input, and accessibility stay owned by the conformance
//! scenarios in this executable. A `story-smoke` trace that carries any of
//! that evidence is rejected rather than accepted as a stronger claim.

use serde_json::Value;

use super::{ParsedRecord, required_string};

/// The exact record order a passing `story-smoke` stream has, start to end.
/// Any extra, missing, duplicated, or reordered record is rejected.
const EXPECTED_EVENTS: [&str; 10] = [
    "story_started",
    "primary_opened",
    "menu_projected",
    "themes_loaded",
    "first_presented",
    "quit_requested",
    "shutdown_requested",
    "will_exit",
    "run_returned",
    "terminal",
];

/// The first event of a `story-smoke` stream. `story_started`, not
/// `scenario_started`: this stream is written by the application under test.
pub(super) const FIRST_EVENT: &str = "story_started";

/// Every event name this scenario may carry.
pub(super) fn event_allowed(event: &str) -> bool {
    EXPECTED_EVENTS.contains(&event)
}

/// The allowed `data` fields for a `story-smoke` event.
pub(super) fn allowed_data_fields(event: &str) -> Option<&'static [&'static str]> {
    Some(match event {
        "story_started" => &["runner", "mode"],
        "primary_opened" => &["surface", "view", "title"],
        "menu_projected" => &[
            "observation",
            "platform",
            "menu_names",
            "items",
            "system_menus",
            "available_actions",
        ],
        "themes_loaded" => &[
            "source",
            "embedded_count",
            "verified_count",
            "catalog",
            "selected",
        ],
        "first_presented" => &["count"],
        "quit_requested" => &["source"],
        "shutdown_requested" => &["reason"],
        "will_exit" => &[],
        "run_returned" => &["result"],
        "terminal" => &["outcome", "exit_code", "error"],
        _ => return None,
    })
}

/// The application display name the macOS application menu renders, and that
/// the About, Quit, and Hide items interpolate.
const APP_DISPLAY_NAME: &str = "Neutron Story";

/// Exactly the actions the running process must report available at first
/// presentation, sorted. Three are app-scoped (the story's own Open
/// Repository, and the standard Settings and About features); Toggle Search is
/// window-scoped and answers from the rendered Gallery's dispatch tree.
const REQUIRED_AVAILABLE_ACTIONS: [&str; 4] = [
    "app::About",
    "app::OpenSettings",
    "story::OpenRepository",
    "story::ToggleSearch",
];

/// One required actionable item: the top-level menu it must appear under, its
/// stable GPUI action type name, and its displayed label.
type RequiredItem = (&'static str, &'static str, &'static str);

/// The standard Edit vocabulary, identical on every platform.
const REQUIRED_EDIT_ITEMS: [RequiredItem; 10] = [
    ("Edit", "input::Undo", "Undo"),
    ("Edit", "input::Redo", "Redo"),
    ("Edit", "input::Cut", "Cut"),
    ("Edit", "input::Copy", "Copy"),
    ("Edit", "input::Paste", "Paste"),
    ("Edit", "input::Delete", "Delete"),
    (
        "Edit",
        "input::DeleteToPreviousWordStart",
        "Delete Previous Word",
    ),
    ("Edit", "input::DeleteToNextWordEnd", "Delete Next Word"),
    ("Edit", "input::Search", "Find"),
    ("Edit", "input::SelectAll", "Select All"),
];

/// The macOS application block: About and Settings above the Appearance
/// section, the Hide/Show group, and Quit.
const MACOS_ITEMS: [RequiredItem; 9] = [
    ("Neutron Story", "app::About", "About Neutron Story"),
    ("Neutron Story", "app::OpenSettings", "Settings\u{2026}"),
    ("Neutron Story", "app::HideApp", "Hide Neutron Story"),
    ("Neutron Story", "app::HideOthers", "Hide Others"),
    ("Neutron Story", "app::ShowAll", "Show All"),
    ("Neutron Story", "app::Quit", "Quit Neutron Story"),
    ("Window", "app::Minimize", "Minimize"),
    ("Window", "app::Zoom", "Zoom"),
    ("Window", "app::CloseWindow", "Close Window"),
];

/// The Windows File/Window/Help blocks. About is a Help item off macOS.
const WINDOWS_ITEMS: [RequiredItem; 4] = [
    ("File", "app::OpenSettings", "Settings"),
    ("File", "app::Quit", "Quit"),
    ("Window", "app::CloseWindow", "Close Window"),
    ("Help", "app::About", "About Neutron Story"),
];

/// The Linux blocks. Identical to Windows except for the Preferences label.
const LINUX_ITEMS: [RequiredItem; 4] = [
    ("File", "app::OpenSettings", "Preferences"),
    ("File", "app::Quit", "Quit"),
    ("Window", "app::CloseWindow", "Close Window"),
    ("Help", "app::About", "About Neutron Story"),
];

/// The story's own Help contribution, on every platform.
const HELP_CONTRIBUTION: RequiredItem = ("Help", "story::OpenRepository", "Open Repository");

/// The three theme-mode radio items the Appearance section always projects.
const THEME_MODE_LABELS: [&str; 3] = ["System", "Light", "Dark"];

/// What the installed menu model must look like on one platform.
struct MenuProfile {
    /// Top-level menu names, in bar order. The macOS application menu renders
    /// the app display name; every other standard menu renders its key.
    menu_names: &'static [&'static str],
    /// System-managed submenus, by name. Only macOS has one.
    system_menus: &'static [&'static str],
    /// Platform-specific required items, beyond the shared Edit vocabulary
    /// and the story's Help contribution.
    items: &'static [RequiredItem],
    /// The top-level menu hosting the Appearance/theme section.
    theme_menu: &'static str,
}

fn menu_profile(platform: &str) -> Option<MenuProfile> {
    Some(match platform {
        // macOS has no standard Help menu, so the story's Help contribution
        // becomes its own top-level menu, inserted before Window. The
        // Appearance section lives in the application menu.
        "macOS" => MenuProfile {
            menu_names: &[APP_DISPLAY_NAME, "Edit", "Help", "Window"],
            system_menus: &["Services"],
            items: &MACOS_ITEMS,
            theme_menu: APP_DISPLAY_NAME,
        },
        // Windows and Linux host the Appearance section in a View menu, which
        // the standard layout inserts only when a theme source is declared.
        "Windows" => MenuProfile {
            menu_names: &["File", "Edit", "View", "Window", "Help"],
            system_menus: &[],
            items: &WINDOWS_ITEMS,
            theme_menu: "View",
        },
        "Linux" => MenuProfile {
            menu_names: &["File", "Edit", "View", "Window", "Help"],
            system_menus: &[],
            items: &LINUX_ITEMS,
            theme_menu: "View",
        },
        _ => return None,
    })
}

/// The platform a `story-smoke` trace recorded, checked against the exact
/// event order first so a malformed trace never reaches profile validation
/// with a stray field.
pub(super) fn recorded_platform(records: &[ParsedRecord]) -> anyhow::Result<&str> {
    let menu = records
        .iter()
        .find(|record| record.event == "menu_projected")
        .ok_or_else(|| anyhow::anyhow!("story-smoke trace has no menu_projected record"))?;
    required_string(&menu.data, "platform")
}

pub(super) fn validate(records: &[ParsedRecord]) -> anyhow::Result<()> {
    if records.len() != EXPECTED_EVENTS.len() {
        anyhow::bail!(
            "story-smoke must contain exactly {} records, found {}",
            EXPECTED_EVENTS.len(),
            records.len()
        );
    }
    for (index, expected) in EXPECTED_EVENTS.iter().enumerate() {
        if records[index].event != *expected {
            anyhow::bail!(
                "story-smoke record {} was {:?}; expected {expected:?}",
                index + 1,
                records[index].event
            );
        }
    }

    validate_story_started(&records[0].data)?;
    validate_primary_opened(&records[1].data)?;
    validate_menu_projected(&records[2].data)?;
    validate_themes_loaded(&records[3].data)?;
    validate_first_presented(&records[4].data)?;
    validate_quit_requested(&records[5].data)?;
    validate_shutdown_requested(&records[6].data)?;
    validate_run_returned(&records[8].data)?;
    validate_terminal(&records[9].data)?;
    Ok(())
}

fn validate_story_started(data: &Value) -> anyhow::Result<()> {
    if required_string(data, "runner")? != "neutron-story" {
        anyhow::bail!("story_started must record the neutron-story runner");
    }
    if required_string(data, "mode")? != "smoke" {
        anyhow::bail!("story_started must record the smoke mode");
    }
    Ok(())
}

fn validate_primary_opened(data: &Value) -> anyhow::Result<()> {
    if required_string(data, "surface")? != "primary" {
        anyhow::bail!("primary_opened must record the primary surface");
    }
    if required_string(data, "view")? != "gallery" {
        anyhow::bail!("primary_opened must record the Gallery view");
    }
    if required_string(data, "title")?.trim().is_empty() {
        anyhow::bail!("primary_opened must record a nonempty window title");
    }
    Ok(())
}

/// One actionable item read back from the installed native menu model.
struct ObservedItem {
    menu: String,
    action: String,
    label: String,
    disabled: bool,
}

fn validate_menu_projected(data: &Value) -> anyhow::Result<()> {
    if required_string(data, "observation")? != "installed_menu_model" {
        anyhow::bail!(
            "menu_projected must record an observation of the installed menu model, not a reconstruction"
        );
    }
    let platform = required_string(data, "platform")?;
    let profile = menu_profile(platform)
        .ok_or_else(|| anyhow::anyhow!("menu_projected recorded unknown platform {platform:?}"))?;

    let menu_names = string_array(data, "menu_names")?;
    if menu_names != profile.menu_names {
        anyhow::bail!(
            "menu_projected observed top-level menus {menu_names:?}; {platform} requires {:?}",
            profile.menu_names
        );
    }

    let items = observed_items(data)?;
    for (menu, action, label) in profile
        .items
        .iter()
        .chain(REQUIRED_EDIT_ITEMS.iter())
        .chain(std::iter::once(&HELP_CONTRIBUTION))
    {
        if !items.iter().any(|item| {
            item.menu == *menu && item.action == *action && item.label == *label && !item.disabled
        }) {
            anyhow::bail!(
                "the installed {platform} menu has no enabled {menu} item dispatching {action} labelled {label:?}"
            );
        }
    }

    for label in THEME_MODE_LABELS {
        if !items.iter().any(|item| {
            item.menu == profile.theme_menu
                && item.action == "theme::SwitchThemeMode"
                && item.label == label
                && !item.disabled
        }) {
            anyhow::bail!(
                "the installed {platform} {} menu has no {label:?} theme-mode item",
                profile.theme_menu
            );
        }
    }
    if !items.iter().any(|item| {
        item.menu == profile.theme_menu && item.action == "theme::SwitchTheme" && !item.disabled
    }) {
        anyhow::bail!(
            "the installed {platform} {} menu projects no theme set",
            profile.theme_menu
        );
    }

    let system_menus = observed_system_menus(data)?;
    if system_menus != profile.system_menus {
        anyhow::bail!(
            "menu_projected observed system menus {system_menus:?}; {platform} requires {:?}",
            profile.system_menus
        );
    }

    let available = string_array(data, "available_actions")?;
    if available != REQUIRED_AVAILABLE_ACTIONS {
        anyhow::bail!(
            "menu_projected recorded available actions {available:?}; expected {REQUIRED_AVAILABLE_ACTIONS:?}"
        );
    }
    Ok(())
}

fn observed_items(data: &Value) -> anyhow::Result<Vec<ObservedItem>> {
    let items = data
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("menu_projected is missing the items array"))?;
    if items.is_empty() {
        anyhow::bail!("menu_projected observed no actionable menu items");
    }
    items
        .iter()
        .map(|item| {
            require_exact_fields(
                item,
                &["menu", "path", "action", "label", "disabled"],
                "menu_projected item",
            )?;
            Ok(ObservedItem {
                menu: required_string(item, "menu")?.to_owned(),
                action: required_string(item, "action")?.to_owned(),
                label: required_string(item, "label")?.to_owned(),
                disabled: item
                    .get("disabled")
                    .and_then(Value::as_bool)
                    .ok_or_else(|| {
                        anyhow::anyhow!("menu_projected item is missing boolean field \"disabled\"")
                    })?,
            })
        })
        .collect()
}

fn observed_system_menus(data: &Value) -> anyhow::Result<Vec<String>> {
    let menus = data
        .get("system_menus")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("menu_projected is missing the system_menus array"))?;
    menus
        .iter()
        .map(|menu| {
            require_exact_fields(
                menu,
                &["menu", "path", "name"],
                "menu_projected system menu",
            )?;
            required_string(menu, "name").map(str::to_owned)
        })
        .collect()
}

fn require_exact_fields(value: &Value, expected: &[&str], context: &str) -> anyhow::Result<()> {
    let object = value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("{context} must be an object"))?;
    let mut actual = object.keys().map(String::as_str).collect::<Vec<_>>();
    actual.sort_unstable();
    let mut expected = expected.to_vec();
    expected.sort_unstable();
    if actual != expected {
        anyhow::bail!("{context} fields were {actual:?}; expected {expected:?}");
    }
    Ok(())
}

fn validate_themes_loaded(data: &Value) -> anyhow::Result<()> {
    if required_string(data, "source")? != "bundled-verified" {
        anyhow::bail!(
            "themes_loaded must record bundled themes verified against the embedded assets"
        );
    }
    let embedded = required_count(data, "embedded_count")?;
    let verified = required_count(data, "verified_count")?;
    if embedded == 0 {
        anyhow::bail!("themes_loaded recorded no embedded bundled theme assets");
    }
    if verified != embedded {
        anyhow::bail!(
            "themes_loaded verified {verified} of {embedded} embedded bundled theme assets"
        );
    }
    if required_count(data, "catalog")? == 0 {
        anyhow::bail!("themes_loaded recorded an empty theme catalog");
    }
    if required_string(data, "selected")?.trim().is_empty() {
        anyhow::bail!("themes_loaded must record a selected theme");
    }
    Ok(())
}

fn required_count(data: &Value, field: &str) -> anyhow::Result<u64> {
    data.get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow::anyhow!("themes_loaded is missing integer {field}"))
}

fn validate_first_presented(data: &Value) -> anyhow::Result<()> {
    if data.get("count").and_then(Value::as_u64) != Some(1) {
        anyhow::bail!("first_presented count must be exactly one");
    }
    Ok(())
}

fn validate_quit_requested(data: &Value) -> anyhow::Result<()> {
    if required_string(data, "source")? != "first_presentation" {
        anyhow::bail!("story-smoke must request quit from first presentation only");
    }
    Ok(())
}

fn validate_shutdown_requested(data: &Value) -> anyhow::Result<()> {
    if required_string(data, "reason")? != "requested" {
        anyhow::bail!("story-smoke shutdown must be the requested shutdown");
    }
    Ok(())
}

fn validate_run_returned(data: &Value) -> anyhow::Result<()> {
    if required_string(data, "result")? != "ok" {
        anyhow::bail!("story-smoke requires AppShell::run to return Ok");
    }
    Ok(())
}

fn validate_terminal(data: &Value) -> anyhow::Result<()> {
    if required_string(data, "outcome")? != "passed"
        || data.get("exit_code").and_then(Value::as_u64) != Some(0)
    {
        anyhow::bail!("story-smoke terminal must be passed with exit code 0");
    }
    Ok(())
}

fn string_array(data: &Value, field: &str) -> anyhow::Result<Vec<String>> {
    let values = data
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("record is missing array field {field:?}"))?;
    values
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| anyhow::anyhow!("field {field:?} contains a non-string entry"))
        })
        .collect()
}
