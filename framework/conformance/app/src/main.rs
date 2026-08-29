//! Native downstream conformance runner for neutron-components AppShell.
//!
//! Scenario runs write synchronized, schema-versioned JSONL to stdout. The
//! executable owns process exit status; AppShell and GPUI never self-terminate.

mod cli;
mod native_window;
mod protocol;
mod scenarios;

use std::any::Any;
use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::process::ExitCode;

use cli::{Command, Scenario, ValidationProfile};
use protocol::{Protocol, TerminalOutcome};

neutron_components_app::include_identity!();

fn main() -> ExitCode {
    match cli::parse(std::env::args()) {
        Ok(Command::Run(scenario)) => run_scenario(scenario),
        Ok(Command::Validate { scenario, profile }) => validate_trace(scenario, profile),
        Ok(Command::Help) => {
            print!("{}", cli::USAGE);
            ExitCode::SUCCESS
        }
        Ok(Command::Version) => {
            println!("{}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}\n\n{}", cli::USAGE);
            ExitCode::from(1)
        }
    }
}

fn validate_trace(scenario: Scenario, profile: Option<ValidationProfile>) -> ExitCode {
    let result = match profile {
        Some(profile) => {
            protocol::validate_jsonl_with_profile(io::stdin().lock(), scenario, Some(profile))
        }
        None => protocol::validate_jsonl(io::stdin().lock(), scenario),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("conformance trace validation failed: {error:#}");
            ExitCode::from(1)
        }
    }
}

fn run_scenario(scenario: Scenario) -> ExitCode {
    // `main` constructs this run's one canonical `Protocol` and hands it off
    // (see `scenarios::install_launch_protocol`) for the scenario's own
    // launch parser to take (see `scenarios::parse_core`) and use for
    // `scenario_started` onward, through its own tail. `protocol` here keeps
    // a clone of that same instance/sequence -- `Protocol` is a cheap `Arc`
    // handle -- purely as an emergency fallback: if a fault escapes a
    // scenario's own tail entirely (for example, a panic in that tail's own
    // cleanup, outside anything `catch_run` wraps, instead of one `catch_run`
    // itself catches), this still gets a terminal record to stdout on the
    // same stream instead of a second, independent one.
    let protocol = Protocol::stdout(scenario);
    if let Err(error) = scenarios::install_launch_protocol(protocol.clone()) {
        eprintln!("conformance protocol handoff install failed: {error:#}");
        return ExitCode::from(1);
    }
    let result = catch_unwind(AssertUnwindSafe(|| scenarios::run(scenario)));

    match result {
        Ok(Ok(scenarios::ScenarioOutcome::Passed)) => {
            ExitCode::from(TerminalOutcome::Passed.exit_code())
        }
        Ok(Ok(scenarios::ScenarioOutcome::ExpectedStartupFailure)) => {
            ExitCode::from(TerminalOutcome::ExpectedStartupFailure.exit_code())
        }
        Ok(Err(error)) => {
            eprintln!("conformance scenario failed: {error:#}");
            ExitCode::from(1)
        }
        Err(panic) => {
            let message = format!("conformance scenario panicked: {}", panic_message(panic));
            let terminal = TerminalOutcome::Panicked(message.clone());
            let exit_code = terminal.exit_code();
            // A no-op if the scenario's own tail already wrote the terminal
            // record through this same `Protocol` instance -- `terminal()`
            // ignores a repeated attempt -- so this never produces a second
            // sequence-1 stream.
            if let Err(error) = protocol.terminal(terminal) {
                eprintln!("conformance protocol terminal write failed: {error:#}");
                return ExitCode::from(1);
            }
            eprintln!("{message}");
            ExitCode::from(exit_code)
        }
    }
}

fn panic_message(panic: Box<dyn Any + Send + 'static>) -> String {
    if let Some(message) = panic.downcast_ref::<&str>() {
        (*message).to_owned()
    } else if let Some(message) = panic.downcast_ref::<String>() {
        message.clone()
    } else {
        "non-string panic payload".to_owned()
    }
}
