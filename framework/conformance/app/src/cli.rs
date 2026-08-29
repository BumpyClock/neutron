use std::fmt;

use neutron_components_app::ProcessLaunch;

/// Native conformance scenarios supported by this executable.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Scenario {
    LifecycleClean,
    LifecycleStartupFailure,
    LifecycleBackgroundQuit,
    WindowCycle,
    MenuCommand,
    Clipboard,
    InteractionContracts,
    /// Validate-only: `neutron-story --smoke` writes this stream itself, so
    /// this runner never launches a `story-smoke` conformance application.
    StorySmoke,
}

impl Scenario {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::LifecycleClean => "lifecycle-clean",
            Self::LifecycleStartupFailure => "lifecycle-startup-failure",
            Self::LifecycleBackgroundQuit => "lifecycle-background-quit",
            Self::WindowCycle => "window-cycle",
            Self::MenuCommand => "menu-command",
            Self::Clipboard => "clipboard",
            Self::InteractionContracts => "interaction-contracts",
            Self::StorySmoke => "story-smoke",
        }
    }

    /// Whether this runner can execute the scenario. `story-smoke` is
    /// produced by the `neutron-story` binary, so only its validator lives
    /// here.
    const fn is_runnable(self) -> bool {
        !matches!(self, Self::StorySmoke)
    }

    fn parse(value: &str) -> Result<Self, CliError> {
        match value {
            "lifecycle-clean" => Ok(Self::LifecycleClean),
            "lifecycle-startup-failure" => Ok(Self::LifecycleStartupFailure),
            "lifecycle-background-quit" => Ok(Self::LifecycleBackgroundQuit),
            "window-cycle" => Ok(Self::WindowCycle),
            "menu-command" => Ok(Self::MenuCommand),
            "clipboard" => Ok(Self::Clipboard),
            "interaction-contracts" => Ok(Self::InteractionContracts),
            "story-smoke" => Ok(Self::StorySmoke),
            _ => Err(CliError::InvalidScenario(value.to_owned())),
        }
    }
}

impl fmt::Display for Scenario {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A target-specific native evidence contract for validation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ValidationProfile {
    MacosMetal,
    WindowsWarp,
    LinuxX11Lavapipe,
    LinuxWaylandLavapipe,
}

impl ValidationProfile {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::MacosMetal => "macos-metal",
            Self::WindowsWarp => "windows-warp",
            Self::LinuxX11Lavapipe => "linux-x11-lavapipe",
            Self::LinuxWaylandLavapipe => "linux-wayland-lavapipe",
        }
    }

    fn parse(value: &str) -> Result<Self, CliError> {
        match value {
            "macos-metal" => Ok(Self::MacosMetal),
            "windows-warp" => Ok(Self::WindowsWarp),
            "linux-x11-lavapipe" => Ok(Self::LinuxX11Lavapipe),
            "linux-wayland-lavapipe" => Ok(Self::LinuxWaylandLavapipe),
            _ => Err(CliError::InvalidProfile(value.to_owned())),
        }
    }
}

impl fmt::Display for ValidationProfile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug)]
pub(crate) enum Command {
    Run(Scenario),
    Validate {
        scenario: Scenario,
        profile: Option<ValidationProfile>,
    },
    Help,
    Version,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum CliError {
    DuplicateScenario,
    DuplicateValidation,
    DuplicateProfile,
    ConflictingModes,
    InvalidScenario(String),
    InvalidProfile(String),
    MissingScenarioValue,
    MissingValidationValue,
    MissingProfileValue,
    MissingScenario,
    ProfileRequiresValidation,
    ValidateOnlyScenario(Scenario),
    UnexpectedArgument(String),
}

impl fmt::Display for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateScenario => formatter.write_str("--scenario may only be provided once"),
            Self::DuplicateValidation => {
                formatter.write_str("--validate may only be provided once")
            }
            Self::DuplicateProfile => formatter.write_str("--profile may only be provided once"),
            Self::ConflictingModes => {
                formatter.write_str("--scenario and --validate may not be combined")
            }
            Self::InvalidScenario(value) => write!(
                formatter,
                "unknown scenario {value:?}; expected lifecycle-clean, lifecycle-startup-failure, lifecycle-background-quit, window-cycle, menu-command, clipboard, interaction-contracts, or story-smoke"
            ),
            Self::InvalidProfile(value) => write!(
                formatter,
                "unknown validation profile {value:?}; expected macos-metal, windows-warp, linux-x11-lavapipe, or linux-wayland-lavapipe"
            ),
            Self::MissingScenarioValue => formatter.write_str("--scenario requires a value"),
            Self::MissingValidationValue => formatter.write_str("--validate requires a value"),
            Self::MissingProfileValue => formatter.write_str("--profile requires a value"),
            Self::MissingScenario => formatter.write_str("--scenario or --validate is required"),
            Self::ProfileRequiresValidation => {
                formatter.write_str("--profile may only be used with --validate")
            }
            Self::ValidateOnlyScenario(scenario) => write!(
                formatter,
                "scenario {scenario} cannot be run by this executable; it is produced by neutron-story --smoke and is only accepted as --validate {scenario}"
            ),
            Self::UnexpectedArgument(argument) => {
                write!(formatter, "unexpected argument {argument:?}")
            }
        }
    }
}

// Every variant is already a stable, leaf `Display` message with no further
// cause to chain, so the default `source() -> None` is correct as-is. This
// impl exists so a scenario's launch parser can propagate a `CliError` with
// `?` into its `anyhow::Result`, re-parsing a `ProcessLaunch` through exactly
// the same grammar `main` used instead of a second, potentially-diverging
// implementation.
impl std::error::Error for CliError {}

/// Parse a process argument vector. Scenario runs write versioned JSONL to stdout.
pub(crate) fn parse(args: impl IntoIterator<Item = String>) -> Result<Command, CliError> {
    let mut arguments = args.into_iter();
    let _program = arguments.next();
    parse_arguments(arguments.collect())
}

/// Parse a [`ProcessLaunch`]'s complete argument list with exactly the same
/// grammar as [`parse`]. `ProcessLaunch::args` already excludes the executable
/// name (unlike the raw argv `parse` receives), so there is no program name to
/// skip here.
///
/// A typed scenario's launch parser calls this to confirm, from the real
/// process facts, that it was the scenario `main` selected — through the one
/// shared grammar, never a second, potentially-diverging implementation.
pub(crate) fn parse_process(process: &ProcessLaunch) -> Result<Command, CliError> {
    let mut arguments = Vec::with_capacity(process.args().len());
    for argument in process.args() {
        match argument.to_str() {
            Some(argument) => arguments.push(argument.to_owned()),
            None => {
                return Err(CliError::UnexpectedArgument(
                    argument.to_string_lossy().into_owned(),
                ));
            }
        }
    }
    parse_arguments(arguments)
}

/// The shared grammar behind [`parse`] and [`parse_process`], over an
/// argument list that already excludes the executable name.
fn parse_arguments(arguments: Vec<String>) -> Result<Command, CliError> {
    if arguments
        .iter()
        .any(|argument| matches!(argument.as_str(), "-h" | "--help"))
    {
        return Ok(Command::Help);
    }
    if arguments.iter().any(|argument| argument == "--version") {
        return Ok(Command::Version);
    }

    let mut scenario = None;
    let mut validation = None;
    let mut profile = None;
    let mut index = 0;
    while let Some(argument) = arguments.get(index) {
        if argument == "--scenario" {
            index += 1;
            let value = arguments.get(index).ok_or(CliError::MissingScenarioValue)?;
            if scenario.replace(Scenario::parse(value)?).is_some() {
                return Err(CliError::DuplicateScenario);
            }
        } else if let Some(value) = argument.strip_prefix("--scenario=") {
            if scenario.replace(Scenario::parse(value)?).is_some() {
                return Err(CliError::DuplicateScenario);
            }
        } else if argument == "--validate" {
            index += 1;
            let value = arguments
                .get(index)
                .ok_or(CliError::MissingValidationValue)?;
            if validation.replace(Scenario::parse(value)?).is_some() {
                return Err(CliError::DuplicateValidation);
            }
        } else if let Some(value) = argument.strip_prefix("--validate=") {
            if validation.replace(Scenario::parse(value)?).is_some() {
                return Err(CliError::DuplicateValidation);
            }
        } else if argument == "--profile" {
            index += 1;
            let value = arguments.get(index).ok_or(CliError::MissingProfileValue)?;
            if profile.replace(ValidationProfile::parse(value)?).is_some() {
                return Err(CliError::DuplicateProfile);
            }
        } else if let Some(value) = argument.strip_prefix("--profile=") {
            if profile.replace(ValidationProfile::parse(value)?).is_some() {
                return Err(CliError::DuplicateProfile);
            }
        } else {
            return Err(CliError::UnexpectedArgument(argument.clone()));
        }
        index += 1;
    }

    match (scenario, validation) {
        (Some(_), Some(_)) => Err(CliError::ConflictingModes),
        (Some(scenario), None) => {
            if profile.is_some() {
                Err(CliError::ProfileRequiresValidation)
            } else if !scenario.is_runnable() {
                Err(CliError::ValidateOnlyScenario(scenario))
            } else {
                Ok(Command::Run(scenario))
            }
        }
        (None, Some(scenario)) => Ok(Command::Validate { scenario, profile }),
        (None, None) => {
            if profile.is_some() {
                Err(CliError::ProfileRequiresValidation)
            } else {
                Err(CliError::MissingScenario)
            }
        }
    }
}

pub(crate) const USAGE: &str = "\
Neutron Components native conformance runner.

Usage:
  neutron-components-conformance --scenario <scenario>
  neutron-components-conformance --validate <scenario> [--profile <profile>] < trace.jsonl

Scenarios:
  lifecycle-clean             Open, present, and cleanly quit a native window.
  lifecycle-startup-failure   Exercise the transactional AppShell startup failure path.
  lifecycle-background-quit   Dispatch from std::thread and release an idle shell hold.
  window-cycle                Close and recreate a native window under Explicit exit policy.
  menu-command                Project and dispatch a registered native menu command.
  clipboard                   Write and externally verify a native clipboard payload.
  interaction-contracts       Verify focused UI contracts in a presented native window.

Validate-only scenarios:
  story-smoke                 Integration evidence written by `neutron-story --smoke`
                              (GPUI_STAGE1_STORY_EVIDENCE_PATH). This executable validates
                              the stream; it never runs the scenario.

Validation profiles:
  macos-metal
  windows-warp
  linux-x11-lavapipe
  linux-wayland-lavapipe

Output:
  Scenario runs write schema-versioned JSONL records to stdout. Diagnostics go to stderr.
  --validate reads JSONL from stdin, writes no stdout, and reports invalid traces to stderr.

Exit status:
  0  Scenario completed successfully.
  1  Invalid invocation, unexpected failure, or panic.
  2  lifecycle-startup-failure reached the expected AppShell startup error.
";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_named_scenario() {
        let command = parse([
            "conformance".to_owned(),
            "--scenario".to_owned(),
            "lifecycle-clean".to_owned(),
        ])
        .expect("scenario should parse");

        assert!(matches!(command, Command::Run(Scenario::LifecycleClean)));
    }

    #[test]
    fn parses_equals_scenario_form() {
        let command = parse([
            "conformance".to_owned(),
            "--scenario=lifecycle-background-quit".to_owned(),
        ])
        .expect("scenario should parse");

        assert!(matches!(
            command,
            Command::Run(Scenario::LifecycleBackgroundQuit)
        ));
    }

    #[test]
    fn parses_independent_native_scenarios() {
        let window_cycle = parse([
            "conformance".to_owned(),
            "--scenario".to_owned(),
            "window-cycle".to_owned(),
        ])
        .expect("window-cycle should parse");
        let menu_command = parse([
            "conformance".to_owned(),
            "--scenario".to_owned(),
            "menu-command".to_owned(),
        ])
        .expect("menu-command should parse");
        let clipboard = parse([
            "conformance".to_owned(),
            "--scenario".to_owned(),
            "clipboard".to_owned(),
        ])
        .expect("clipboard should parse");
        let interaction_contracts = parse([
            "conformance".to_owned(),
            "--scenario".to_owned(),
            "interaction-contracts".to_owned(),
        ])
        .expect("interaction-contracts should parse");

        assert!(matches!(window_cycle, Command::Run(Scenario::WindowCycle)));
        assert!(matches!(menu_command, Command::Run(Scenario::MenuCommand)));
        assert!(matches!(clipboard, Command::Run(Scenario::Clipboard)));
        assert!(matches!(
            interaction_contracts,
            Command::Run(Scenario::InteractionContracts)
        ));
    }

    #[test]
    fn parses_validation_mode_without_profile() {
        let command = parse([
            "conformance".to_owned(),
            "--validate=window-cycle".to_owned(),
        ])
        .expect("validation mode should parse");

        assert!(matches!(
            command,
            Command::Validate {
                scenario: Scenario::WindowCycle,
                profile: None,
            }
        ));
    }

    #[test]
    fn parses_each_validation_profile() {
        for (name, profile) in [
            ("macos-metal", ValidationProfile::MacosMetal),
            ("windows-warp", ValidationProfile::WindowsWarp),
            ("linux-x11-lavapipe", ValidationProfile::LinuxX11Lavapipe),
            (
                "linux-wayland-lavapipe",
                ValidationProfile::LinuxWaylandLavapipe,
            ),
        ] {
            let command = parse([
                "conformance".to_owned(),
                "--validate".to_owned(),
                "window-cycle".to_owned(),
                "--profile".to_owned(),
                name.to_owned(),
            ])
            .expect("profiled validation mode should parse");

            assert!(matches!(
                command,
                Command::Validate {
                    scenario: Scenario::WindowCycle,
                    profile: Some(actual),
                } if actual == profile
            ));
        }
    }

    #[test]
    fn accepts_equals_validation_profile_form() {
        let command = parse([
            "conformance".to_owned(),
            "--profile=windows-warp".to_owned(),
            "--validate=clipboard".to_owned(),
        ])
        .expect("equals profile form should parse");

        assert!(matches!(
            command,
            Command::Validate {
                scenario: Scenario::Clipboard,
                profile: Some(ValidationProfile::WindowsWarp),
            }
        ));
    }

    #[test]
    fn rejects_profile_conflicts_and_invalid_forms() {
        assert_eq!(
            parse([
                "conformance".to_owned(),
                "--scenario".to_owned(),
                "window-cycle".to_owned(),
                "--profile".to_owned(),
                "macos-metal".to_owned(),
            ])
            .unwrap_err(),
            CliError::ProfileRequiresValidation
        );
        assert_eq!(
            parse(["conformance".to_owned(), "--profile".to_owned()]).unwrap_err(),
            CliError::MissingProfileValue
        );
        assert_eq!(
            parse([
                "conformance".to_owned(),
                "--validate".to_owned(),
                "window-cycle".to_owned(),
                "--profile".to_owned(),
                "macos-metal".to_owned(),
                "--profile".to_owned(),
                "windows-warp".to_owned(),
            ])
            .unwrap_err(),
            CliError::DuplicateProfile
        );
        assert_eq!(
            parse([
                "conformance".to_owned(),
                "--validate".to_owned(),
                "window-cycle".to_owned(),
                "--profile".to_owned(),
                "unknown".to_owned(),
            ])
            .unwrap_err(),
            CliError::InvalidProfile("unknown".to_owned())
        );
    }

    #[test]
    fn rejects_mixed_run_and_validation_modes() {
        assert_eq!(
            parse([
                "conformance".to_owned(),
                "--scenario".to_owned(),
                "window-cycle".to_owned(),
                "--validate".to_owned(),
                "window-cycle".to_owned(),
            ])
            .unwrap_err(),
            CliError::ConflictingModes
        );
    }

    #[test]
    fn rejects_missing_scenario() {
        assert_eq!(
            parse(["conformance".to_owned()]).unwrap_err(),
            CliError::MissingScenario
        );
    }

    #[test]
    fn story_smoke_validates_but_never_runs() {
        let command = parse([
            "conformance".to_owned(),
            "--validate".to_owned(),
            "story-smoke".to_owned(),
            "--profile".to_owned(),
            "macos-metal".to_owned(),
        ])
        .expect("story-smoke validation should parse");
        assert!(matches!(
            command,
            Command::Validate {
                scenario: Scenario::StorySmoke,
                profile: Some(ValidationProfile::MacosMetal),
            }
        ));

        for arguments in [
            vec![
                "conformance".to_owned(),
                "--scenario=story-smoke".to_owned(),
            ],
            vec![
                "conformance".to_owned(),
                "--scenario".to_owned(),
                "story-smoke".to_owned(),
            ],
        ] {
            let error = parse(arguments).unwrap_err();
            assert_eq!(error, CliError::ValidateOnlyScenario(Scenario::StorySmoke));
            let message = error.to_string();
            assert!(message.contains("neutron-story --smoke"), "{message}");
            assert!(message.contains("--validate story-smoke"), "{message}");
        }
    }

    #[test]
    fn parse_process_also_refuses_to_run_story_smoke() {
        let process = ProcessLaunch::new(vec!["--scenario=story-smoke".into()], None);

        assert_eq!(
            parse_process(&process).unwrap_err(),
            CliError::ValidateOnlyScenario(Scenario::StorySmoke)
        );
    }

    #[test]
    fn help_ignores_other_arguments() {
        let command = parse([
            "conformance".to_owned(),
            "--scenario".to_owned(),
            "lifecycle-clean".to_owned(),
            "--help".to_owned(),
        ])
        .expect("help should parse");

        assert!(matches!(command, Command::Help));
    }

    #[test]
    fn parse_process_agrees_with_parse_on_the_same_selection() {
        let process = ProcessLaunch::new(vec!["--scenario=window-cycle".into()], None);

        let command = parse_process(&process).expect("process facts should parse");

        assert!(matches!(command, Command::Run(Scenario::WindowCycle)));
    }

    #[test]
    fn parse_process_does_not_skip_a_program_name() {
        // `ProcessLaunch::args` already excludes the executable name, unlike
        // the raw argv `parse` receives: the first element here is a real
        // flag, not a program name to discard.
        let process = ProcessLaunch::new(vec!["--scenario".into(), "lifecycle-clean".into()], None);

        let command = parse_process(&process).expect("process facts should parse");

        assert!(matches!(command, Command::Run(Scenario::LifecycleClean)));
    }

    #[test]
    fn parse_process_rejects_the_same_malformed_input_as_parse() {
        let process = ProcessLaunch::new(vec!["--scenario".into()], None);

        assert_eq!(
            parse_process(&process).unwrap_err(),
            CliError::MissingScenarioValue
        );
    }
}
