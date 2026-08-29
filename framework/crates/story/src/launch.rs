//! The typed CLI contract for the `neutron-story` binary.
//!
//! ```text
//! neutron-story [--story <story_klass>] [--smoke | --asset-smoke | --fail-start]
//! neutron-story --help
//! neutron-story --version
//! ```
//!
//! `--help` and `--version` are valid only as the process's single, standalone
//! argument: the complete argument list is parsed first, so `--help --bogus`,
//! `--version --smoke`, or any other mixed invocation is rejected rather than
//! silently honoring the help/version request.
//!
//! `--story` requires a following non-flag value, may appear at most once, and
//! resolves by exact (case-sensitive) `story_klass` through the static
//! descriptor registry before any GPUI or platform construction. An unresolved
//! value is rejected with deterministic close-match suggestions. In-app search
//! remains fuzzy; only this pre-platform resolution is exact.
//!
//! `--smoke`, `--asset-smoke`, and `--fail-start` are mutually exclusive.
//! Unknown flags, duplicate singleton flags, a missing or flag-shaped
//! `--story` value, and unexpected positional arguments are all rejected.

use std::fmt;
use std::fs;
use std::thread;
use std::time::Duration;

use anyhow::{Context as _, bail};
use gpui::{Action, App, Entity, Focusable as _, Global, OwnedMenuItem, Window};
use neutron_components_app::commands::standard::{About, OpenSettings};
use neutron_components_app::prelude::*;
use neutron_components_app::{
    DesktopPlatform, LaunchDecision, ProcessLaunch, ThemeMenuGroup, theme_menu_items,
};
use neutron_story::{story_descriptor, story_descriptors};
use serde_json::{Value, json};

use crate::APP_IDENTITY;
use crate::app::{PRIMARY_TITLE, embedded_theme_assets};
use crate::commands::{OpenRepository, ToggleSearch};
use crate::evidence::{self, StoryEvidence};
use crate::gallery::Gallery;

/// How long an ordinary `--smoke` run stays up before requesting a clean quit
/// if first presentation never resolves. Never used in evidence mode: Stage 1
/// requires real presentation, and the external watchdog bounds the process.
const SMOKE_LIFETIME: Duration = Duration::from_secs(3);
/// A bundled component asset validated by `--asset-smoke`.
const COMPONENT_ASSET_PATH: &str = "surface/NoiseAsset_256.png";
/// The bundled-theme manifest the framework's bundled theme source writes
/// beside the synchronized theme directory.
const BUNDLED_THEME_MANIFEST: &str = "themes-bundled.lst";
/// The synchronized bundled-theme directory, relative to the config directory.
const BUNDLED_THEME_DIR: &str = "themes";
/// How many close-match suggestions an unknown `--story` value reports.
const MAX_SUGGESTIONS: usize = 3;
/// The furthest edit distance still considered a "close" match.
const MAX_SUGGESTION_DISTANCE: usize = 4;

/// The mutually exclusive smoke/launch modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SmokeMode {
    Normal,
    Smoke,
    AssetSmoke,
    FailStart,
}

/// The gallery's typed launch value.
///
/// `story_klass` carries the already-resolved, stable, exact registry klass
/// (never the raw, unvalidated user string): the Gallery selects that
/// descriptor directly rather than routing it through the fuzzy search field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StoryLaunch {
    pub(crate) story_klass: Option<&'static str>,
    pub(crate) mode: SmokeMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CliError {
    DuplicateStory,
    MissingStoryValue,
    UnknownStory {
        value: String,
        suggestions: Vec<&'static str>,
    },
    DuplicateSmokeFlag(&'static str),
    ConflictingSmokeModes,
    UnknownFlag(String),
    UnexpectedPositional(String),
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateStory => f.write_str("--story may only be provided once"),
            Self::MissingStoryValue => {
                f.write_str("--story requires a following value that is not itself a flag")
            }
            Self::UnknownStory { value, suggestions } => {
                write!(f, "unknown story_klass {value:?}")?;
                if !suggestions.is_empty() {
                    write!(f, "; did you mean: {}?", suggestions.join(", "))?;
                }
                Ok(())
            }
            Self::DuplicateSmokeFlag(flag) => write!(f, "{flag} may only be provided once"),
            Self::ConflictingSmokeModes => {
                f.write_str("--smoke, --asset-smoke, and --fail-start may not be combined")
            }
            Self::UnknownFlag(flag) => write!(f, "unknown flag {flag:?}"),
            Self::UnexpectedPositional(argument) => {
                write!(f, "unexpected positional argument {argument:?}")
            }
        }
    }
}

impl std::error::Error for CliError {}

pub(crate) const USAGE: &str = "\
neutron-story - Neutron Components gallery.

Usage:
  neutron-story [--story <story_klass>] [--smoke | --asset-smoke | --fail-start]
  neutron-story --help
  neutron-story --version

Options:
  --story <story_klass>   Select a story by its exact, case-sensitive story_klass
                          (see the sidebar for names, e.g. WelcomeStory).
  --smoke                 Open the gallery window, then quit automatically.
  --asset-smoke            Validate the bundled asset chain, then quit without opening a window.
  --fail-start             Force a startup failure (used by launch conformance checks).
  --help                   Print this help and exit.
  --version                Print the version and exit.

--help and --version are valid only on their own; combining either with any
other argument is a usage error.
";

fn set_mode(
    current: &mut SmokeMode,
    requested: SmokeMode,
    flag: &'static str,
) -> Result<(), CliError> {
    match *current {
        SmokeMode::Normal => {
            *current = requested;
            Ok(())
        }
        existing if existing == requested => Err(CliError::DuplicateSmokeFlag(flag)),
        _ => Err(CliError::ConflictingSmokeModes),
    }
}

/// Whether `value` looks like a flag (`--story`, `--smoke`, ...) rather than a
/// value: every flag in this contract starts with `--`, so a `--story` value
/// that itself starts with `--` was never supplied.
fn looks_like_flag(value: &str) -> bool {
    value.starts_with("--")
}

/// Pure, deterministic Levenshtein edit distance; callers pre-lowercase both
/// strings for case-insensitive scoring. Used only to rank close-match
/// suggestions for an unresolved `--story` value; no dependency beyond the
/// standard library.
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    let mut current = vec![0usize; b.len() + 1];

    for (i, &char_a) in a.iter().enumerate() {
        current[0] = i + 1;
        for (j, &char_b) in b.iter().enumerate() {
            let substitution_cost = if char_a == char_b { 0 } else { 1 };
            current[j + 1] = (previous[j + 1] + 1)
                .min(current[j] + 1)
                .min(previous[j] + substitution_cost);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[b.len()]
}

/// Deterministic close matches for an unresolved `--story` value, nearest
/// first and ties broken by registry order. Pure and dependency-free, per the
/// exact-CLI contract; in-app search keeps its existing fuzzy matcher.
fn close_matches(query: &str) -> Vec<&'static str> {
    let query_lower = query.to_lowercase();
    let mut scored: Vec<(usize, &'static str)> = story_descriptors()
        .iter()
        .map(|descriptor| {
            let distance = edit_distance(&query_lower, &descriptor.story_klass.to_lowercase());
            (distance, descriptor.story_klass)
        })
        .filter(|(distance, _)| *distance <= MAX_SUGGESTION_DISTANCE)
        .collect();
    scored.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(right.1)));
    scored
        .into_iter()
        .take(MAX_SUGGESTIONS)
        .map(|(_, klass)| klass)
        .collect()
}

/// Parse the documented CLI contract. Deterministic: unrecognized input is
/// always rejected rather than ignored, and the full argument list is walked
/// before any help/version/story decision is made.
pub(crate) fn parse_launch(process: &ProcessLaunch) -> anyhow::Result<LaunchDecision<StoryLaunch>> {
    let arguments = process.args();

    // Help and version are valid only as the process's single, standalone
    // argument. Any other shape (extra flags, `--story`, a second copy, ...)
    // falls through to the normal parser below, which rejects an unrecognized
    // `--help`/`--version` token like any other unknown flag.
    if let [only] = arguments {
        match only.to_str() {
            Some("--help") => {
                return Ok(LaunchDecision::ExitSuccess {
                    stdout: Some(USAGE.to_owned()),
                });
            }
            Some("--version") => {
                return Ok(LaunchDecision::ExitSuccess {
                    stdout: Some(format!("{}\n", APP_IDENTITY.version)),
                });
            }
            _ => {}
        }
    }

    let mut story_klass: Option<&'static str> = None;
    let mut mode = SmokeMode::Normal;
    let mut index = 0;
    while let Some(argument) = arguments.get(index) {
        match argument.to_str() {
            Some("--story") => {
                index += 1;
                let value = arguments.get(index).and_then(|value| value.to_str());
                let value = match value {
                    Some(value) if !looks_like_flag(value) => value,
                    _ => return Err(CliError::MissingStoryValue.into()),
                };
                let descriptor = story_descriptor(value).ok_or_else(|| CliError::UnknownStory {
                    value: value.to_owned(),
                    suggestions: close_matches(value),
                })?;
                if story_klass.replace(descriptor.story_klass).is_some() {
                    return Err(CliError::DuplicateStory.into());
                }
            }
            Some("--smoke") => set_mode(&mut mode, SmokeMode::Smoke, "--smoke")?,
            Some("--asset-smoke") => set_mode(&mut mode, SmokeMode::AssetSmoke, "--asset-smoke")?,
            Some("--fail-start") => set_mode(&mut mode, SmokeMode::FailStart, "--fail-start")?,
            Some(flag) if looks_like_flag(flag) => {
                return Err(CliError::UnknownFlag(flag.to_owned()).into());
            }
            _ => {
                return Err(CliError::UnexpectedPositional(
                    argument.to_string_lossy().into_owned(),
                )
                .into());
            }
        }
        index += 1;
    }

    Ok(LaunchDecision::Run(StoryLaunch { story_klass, mode }))
}

/// Whether the primary surface's `after_open` hook should schedule a smoke
/// quit. Stashed during `before_primary`: `after_open` does not receive the
/// typed launch value.
struct SmokeQuitRequested;

impl Global for SmokeQuitRequested {}

/// Launch-specific work before the primary surface opens: `--fail-start` and
/// `--asset-smoke` both request a quit here, which suppresses the primary
/// open.
pub(crate) fn before_primary(launch: &StoryLaunch, cx: &mut App) -> anyhow::Result<()> {
    match launch.mode {
        SmokeMode::FailStart => {
            eprintln!("NEUTRON_STORY_FAIL_START_REACHED");
            cx.request_quit();
            bail!("requested startup failure");
        }
        SmokeMode::AssetSmoke => {
            validate_asset_chain(cx)?;
            cx.request_quit();
            return Ok(());
        }
        SmokeMode::Smoke => {
            cx.set_global(SmokeQuitRequested);
            // The evidence path is the only opt-in. A failure to open or
            // write it fails startup here rather than letting the run report
            // success with no evidence.
            if let Some(evidence) = StoryEvidence::from_env()? {
                evidence.emit(
                    evidence::STORY_STARTED,
                    json!({"runner": "neutron-story", "mode": "smoke"}),
                )?;
                evidence::install(evidence)?;
            }
        }
        SmokeMode::Normal => {}
    }
    Ok(())
}

fn validate_asset_chain(cx: &App) -> anyhow::Result<()> {
    cx.asset_source()
        .load(COMPONENT_ASSET_PATH)?
        .ok_or_else(|| anyhow::anyhow!("bundled component asset missing"))?;
    Ok(())
}

/// Restores initial keyboard focus to the Gallery so window-scoped commands
/// (e.g. Toggle Search) work immediately, then arranges the `--smoke` quit
/// once the primary surface has opened.
pub(crate) fn after_primary_open(view: &Entity<Gallery>, window: &mut Window, cx: &mut App) {
    let focus_handle = view.read(cx).focus_handle(cx);
    window.defer(cx, move |window, cx| {
        focus_handle.focus(window, cx);
    });

    if !cx.has_global::<SmokeQuitRequested>() {
        return;
    }

    if let Some(evidence) = evidence::active() {
        let recorded = evidence.emit(
            evidence::PRIMARY_OPENED,
            json!({"surface": "primary", "view": "gallery", "title": PRIMARY_TITLE}),
        );
        if let Err(error) = recorded {
            evidence.record_failure(format!("story-smoke evidence failed: {error:#}"));
            cx.request_quit();
            return;
        }
    }

    quit_on_first_presentation(window, cx);

    // Evidence mode must prove real presentation, so it has no timer: an
    // unpresented run is bounded by the external Stage 1 watchdog and fails
    // there instead of claiming a smoke pass. An ordinary `--smoke` keeps a
    // bounded fallback so a developer or ordinary-CI run still terminates.
    if evidence::active().is_none() {
        schedule_smoke_quit_fallback(cx);
    }
}

/// Observe the installed native menu model and verify bundled theme
/// provenance, then record both — in that fixed order — immediately before
/// `first_presented`.
///
/// First presentation is the earliest point at which the rendered window's
/// action dispatch path and the platform's installed menu model are both
/// observable, so the observation happens here rather than at surface open.
///
/// # Errors
///
/// Returns the first observation, availability, or provenance failure. The
/// caller records it and requests quit, so no passing `menu_projected` or
/// `themes_loaded` record is ever written for a failed observation.
fn record_presentation_evidence(
    evidence: &StoryEvidence,
    window: &Window,
    cx: &mut App,
) -> anyhow::Result<()> {
    let menu = observe_installed_menu(window, cx)?;
    let themes = verify_bundled_themes(cx)?;
    evidence.emit(evidence::MENU_PROJECTED, menu)?;
    evidence.emit(evidence::THEMES_LOADED, themes)?;
    Ok(())
}

/// The app-scoped actions this application must be able to dispatch once the
/// primary window has presented: the story's own Open Repository command and
/// the standard Settings and About features.
///
/// Real action values, not names: an availability answer is only meaningful
/// for the concrete type GPUI dispatches on.
fn required_app_actions() -> Vec<Box<dyn Action>> {
    vec![
        Box::new(OpenRepository),
        Box::new(OpenSettings),
        Box::new(About),
    ]
}

/// The window-scoped actions the rendered primary window must be able to
/// dispatch. Checked against the rendered frame's dispatch tree, which is the
/// seam that answers for the Gallery's own key context and focus, rather than
/// against whichever window the OS currently considers active.
fn required_window_actions() -> Vec<Box<dyn Action>> {
    vec![Box::new(ToggleSearch)]
}

/// Read the installed native menu tree back from the platform and record its
/// observed shape.
///
/// [`App::get_menus`] returns the projection the command registry actually
/// installed through `set_menus`, on every desktop platform — including the
/// platforms where `Root` also renders an in-window menu bar from the same
/// registry. Nothing here re-resolves the declaration.
fn observe_installed_menu(window: &Window, cx: &mut App) -> anyhow::Result<Value> {
    let menus = cx
        .get_menus()
        .context("the platform reported no installed native menu model")?;
    if menus.is_empty() {
        anyhow::bail!("the installed native menu model has no top-level menus");
    }

    let mut menu_names = Vec::with_capacity(menus.len());
    let mut items = Vec::new();
    let mut system_menus = Vec::new();
    for menu in &menus {
        menu_names.push(menu.name.to_string());
        collect_menu_items(
            &menu.name,
            &menu.name,
            &menu.items,
            &mut items,
            &mut system_menus,
        );
    }
    if items.is_empty() {
        anyhow::bail!("the installed native menu model has no actionable items");
    }

    let mut available = Vec::new();
    for action in required_window_actions() {
        if !window.is_action_available(action.as_ref(), cx) {
            anyhow::bail!(
                "window-scoped action {} is not available at first presentation",
                action.name()
            );
        }
        available.push(action.name().to_owned());
    }
    for action in required_app_actions() {
        if !cx.is_action_available(action.as_ref()) {
            anyhow::bail!(
                "app-scoped action {} is not available at first presentation",
                action.name()
            );
        }
        available.push(action.name().to_owned());
    }
    available.sort();

    Ok(json!({
        "observation": "installed_menu_model",
        "platform": DesktopPlatform::current().as_str(),
        "menu_names": menu_names,
        "items": items,
        "system_menus": system_menus,
        "available_actions": available,
    }))
}

/// Walk one menu's items, recording every actionable leaf's action name and
/// displayed label. Submenu structure is recorded only as the `path` an item
/// was found under, and system menus only by name: no pointer, handle, or
/// address is ever recorded.
fn collect_menu_items(
    menu: &str,
    path: &str,
    source: &[OwnedMenuItem],
    items: &mut Vec<Value>,
    system_menus: &mut Vec<Value>,
) {
    for item in source {
        match item {
            OwnedMenuItem::Separator => {}
            OwnedMenuItem::Submenu(submenu) => {
                let child = format!("{path} > {}", submenu.name);
                collect_menu_items(menu, &child, &submenu.items, items, system_menus);
            }
            OwnedMenuItem::SystemMenu(system) => system_menus.push(json!({
                "menu": menu,
                "path": path,
                "name": system.name.to_string(),
            })),
            OwnedMenuItem::Action {
                name,
                action,
                disabled,
                ..
            } => items.push(json!({
                "menu": menu,
                "path": path,
                "action": action.name(),
                "label": name,
                "disabled": disabled,
            })),
        }
    }
}

/// Verify that the installed bundled themes are byte-for-byte the assets this
/// binary embeds, then record the live registry's catalog and selection.
///
/// The claim is deliberately narrow: `bundled-verified` means the manifest
/// names equal the embedded names and every installed file matches its
/// embedded bytes. It does not claim the themes parsed, rendered, or
/// hot-reloaded.
fn verify_bundled_themes(cx: &App) -> anyhow::Result<Value> {
    let embedded = embedded_theme_assets()?;
    if embedded.is_empty() {
        anyhow::bail!("this binary embeds no bundled theme assets");
    }

    let config_dir = Shell::app_info(cx).paths().config_dir().to_path_buf();
    let manifest_path = config_dir.join(BUNDLED_THEME_MANIFEST);
    let installed_manifest = fs::read(&manifest_path)
        .with_context(|| format!("read bundled theme manifest {}", manifest_path.display()))?;
    let expected_manifest = embedded
        .keys()
        .cloned()
        .collect::<Vec<_>>()
        .join("\n")
        .into_bytes();
    if installed_manifest != expected_manifest {
        anyhow::bail!(
            "bundled theme manifest {} does not match the canonical embedded-theme manifest bytes",
            manifest_path.display(),
        );
    }

    let themes_dir = config_dir.join(BUNDLED_THEME_DIR);
    let mut verified = 0_usize;
    for (name, contents) in &embedded {
        let path = themes_dir.join(name);
        let installed =
            fs::read(&path).with_context(|| format!("read bundled theme {}", path.display()))?;
        if installed != *contents {
            anyhow::bail!(
                "installed bundled theme {} does not match its embedded asset ({} installed bytes, {} embedded)",
                path.display(),
                installed.len(),
                contents.len()
            );
        }
        verified += 1;
    }

    let catalog: Vec<_> = theme_menu_items(cx)
        .into_iter()
        .filter(|item| item.group == ThemeMenuGroup::Theme)
        .collect();
    if catalog.is_empty() {
        anyhow::bail!("the live theme registry catalog was empty");
    }
    let selected = catalog
        .iter()
        .find(|item| item.checked)
        .map(|item| item.label.clone())
        .context("the live theme registry has no selected theme set")?;

    Ok(json!({
        "source": "bundled-verified",
        "embedded_count": embedded.len(),
        "verified_count": verified,
        "catalog": catalog.len(),
        "selected": selected,
    }))
}

/// Quit once the window's own first-presentation evidence resolves. Stage 1
/// never infers presentation from a timer.
fn quit_on_first_presentation(window: &Window, cx: &mut App) {
    let first_presentation = window.observe_first_presentation();
    let proxy = cx.app_proxy();
    window
        .spawn(cx, async move |async_cx| {
            let presented = first_presentation.await.is_ok();
            let delivered = async_cx.update(move |window, cx| {
                if !presented {
                    // The window closed before presenting. Evidence mode
                    // records the failure and lets the terminal report it;
                    // an ordinary smoke run just quits.
                    if let Some(evidence) = evidence::active() {
                        evidence.record_failure(
                            "story-smoke window closed before first presentation".to_owned(),
                        );
                    }
                    cx.request_quit();
                    return;
                }
                if let Some(evidence) = evidence::active() {
                    let count = window.first_presentation_count();
                    let recorded = record_presentation_evidence(evidence, window, cx)
                        .and_then(|()| {
                            evidence.emit(evidence::FIRST_PRESENTED, json!({"count": count}))
                        })
                        .and_then(|()| {
                            if count == 1 {
                                Ok(())
                            } else {
                                Err(anyhow::anyhow!(
                                    "first-presentation observer resolved with count {count}"
                                ))
                            }
                        })
                        .and_then(|()| {
                            evidence.emit(
                                evidence::QUIT_REQUESTED,
                                json!({"source": "first_presentation"}),
                            )
                        });
                    if let Err(error) = recorded {
                        evidence.record_failure(format!("story-smoke evidence failed: {error:#}"));
                    }
                }
                cx.request_quit();
            });
            if delivered.is_err() {
                if let Some(evidence) = evidence::active() {
                    evidence.record_failure(
                        "story-smoke could not deliver the first-presentation result".to_owned(),
                    );
                }
                let _ = proxy.dispatch(|cx| cx.request_quit());
            }
        })
        .detach();
}

fn schedule_smoke_quit_fallback(cx: &App) {
    let proxy = cx.app_proxy();
    thread::spawn(move || {
        thread::sleep(SMOKE_LIFETIME);
        let _ = proxy.dispatch(|cx| cx.request_quit());
    });
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::*;

    fn process(args: &[&str]) -> ProcessLaunch {
        ProcessLaunch::new(args.iter().map(OsString::from).collect(), None)
    }

    fn run(args: &[&str]) -> anyhow::Result<LaunchDecision<StoryLaunch>> {
        parse_launch(&process(args))
    }

    fn parse_err(args: &[&str]) -> String {
        run(args).expect_err("expected a usage error").to_string()
    }

    #[test]
    fn help_alone_exits_successfully_with_usage() {
        match run(&["--help"]).expect("help is a valid standalone mode") {
            LaunchDecision::ExitSuccess { stdout } => {
                assert_eq!(stdout.as_deref(), Some(USAGE));
            }
            LaunchDecision::Run(_) => panic!("--help must not run the application"),
        }
    }

    #[test]
    fn version_alone_exits_successfully_with_version() {
        match run(&["--version"]).expect("version is a valid standalone mode") {
            LaunchDecision::ExitSuccess { stdout } => {
                assert_eq!(stdout, Some(format!("{}\n", APP_IDENTITY.version)));
            }
            LaunchDecision::Run(_) => panic!("--version must not run the application"),
        }
    }

    #[test]
    fn help_mixed_with_another_argument_fails() {
        run(&["--help", "--bogus"]).expect_err("--help --bogus is a mixed mode");
    }

    #[test]
    fn version_mixed_with_another_argument_fails() {
        run(&["--version", "--smoke"]).expect_err("--version --smoke is a mixed mode");
    }

    #[test]
    fn duplicate_help_is_not_a_standalone_mode() {
        run(&["--help", "--help"]).expect_err("--help must appear alone, not even twice");
    }

    #[test]
    fn exact_story_klass_resolves_the_descriptor() {
        match run(&["--story", "WelcomeStory"]).expect("WelcomeStory is a real story_klass") {
            LaunchDecision::Run(launch) => {
                assert_eq!(launch.story_klass, Some("WelcomeStory"));
                assert_eq!(launch.mode, SmokeMode::Normal);
            }
            LaunchDecision::ExitSuccess { .. } => panic!("--story must run the application"),
        }
    }

    #[test]
    fn unknown_story_reports_a_close_match() {
        let message = parse_err(&["--story", "WelcomStory"]);
        assert!(message.contains("WelcomStory"), "{message}");
        assert!(message.contains("WelcomeStory"), "{message}");
    }

    #[test]
    fn story_flag_missing_its_value_fails() {
        parse_err(&["--story"]);
    }

    #[test]
    fn story_flag_followed_by_another_flag_fails() {
        let message = parse_err(&["--story", "--smoke"]);
        assert!(message.contains("--story"), "{message}");
    }

    #[test]
    fn duplicate_story_flag_fails() {
        parse_err(&["--story", "WelcomeStory", "--story", "ButtonStory"]);
    }

    #[test]
    fn smoke_flag_parses_to_smoke_mode() {
        match run(&["--smoke"]).expect("valid") {
            LaunchDecision::Run(launch) => assert_eq!(launch.mode, SmokeMode::Smoke),
            LaunchDecision::ExitSuccess { .. } => panic!("--smoke must run the application"),
        }
    }

    #[test]
    fn asset_smoke_flag_parses_to_asset_smoke_mode() {
        match run(&["--asset-smoke"]).expect("valid") {
            LaunchDecision::Run(launch) => assert_eq!(launch.mode, SmokeMode::AssetSmoke),
            LaunchDecision::ExitSuccess { .. } => panic!("--asset-smoke must run the application"),
        }
    }

    #[test]
    fn fail_start_flag_parses_to_fail_start_mode() {
        match run(&["--fail-start"]).expect("valid") {
            LaunchDecision::Run(launch) => assert_eq!(launch.mode, SmokeMode::FailStart),
            LaunchDecision::ExitSuccess { .. } => panic!("--fail-start must run the application"),
        }
    }

    #[test]
    fn conflicting_smoke_modes_fail() {
        parse_err(&["--smoke", "--fail-start"]);
    }

    #[test]
    fn duplicate_smoke_flag_fails() {
        parse_err(&["--smoke", "--smoke"]);
    }

    #[test]
    fn unknown_flag_fails() {
        let message = parse_err(&["--bogus"]);
        assert!(message.contains("--bogus"), "{message}");
    }

    #[test]
    fn positional_argument_fails() {
        let message = parse_err(&["WelcomeStory"]);
        assert!(message.contains("WelcomeStory"), "{message}");
    }
}
