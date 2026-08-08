//! Native downstream conformance runner for gpui-component AppShell.
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

gpui_component_app::include_identity!();

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
    let protocol = Protocol::stdout(scenario);
    let result = catch_unwind(AssertUnwindSafe(|| {
        scenarios::run(scenario, protocol.clone())
    }));

    let (terminal, diagnostic) = match result {
        Ok(Ok(scenarios::ScenarioOutcome::Passed)) => (TerminalOutcome::Passed, None),
        Ok(Ok(scenarios::ScenarioOutcome::ExpectedStartupFailure)) => {
            (TerminalOutcome::ExpectedStartupFailure, None)
        }
        Ok(Err(error)) => {
            let message = format!("conformance scenario failed: {error:#}");
            (TerminalOutcome::Failed(message.clone()), Some(message))
        }
        Err(panic) => {
            let message = format!("conformance scenario panicked: {}", panic_message(panic));
            (TerminalOutcome::Panicked(message.clone()), Some(message))
        }
    };

    let exit_code = terminal.exit_code();
    if let Err(error) = protocol.terminal(terminal) {
        eprintln!("conformance protocol terminal write failed: {error:#}");
        return ExitCode::from(1);
    }
    if let Some(diagnostic) = diagnostic {
        eprintln!("{diagnostic}");
    }
    ExitCode::from(exit_code)
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
