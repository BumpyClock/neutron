//! `neutron-story` — the Neutron Components gallery application.

mod app;
mod commands;
mod evidence;
mod gallery;
mod launch;
mod setup;

use std::process::ExitCode;

use neutron_components_app::AppShell;

neutron_components_app::include_identity!();

use app::StoryApp;

fn main() -> ExitCode {
    let outcome = AppShell::run::<StoryApp>();
    // The post-run tail owns the `story-smoke` terminal record: a `passed`
    // outcome is only ever written once `AppShell::run` has actually
    // returned. A no-op unless Stage 1 asked for evidence.
    let evidence = evidence::finish(&outcome);

    match outcome {
        Ok(()) => match evidence {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("neutron-story story-smoke evidence failed: {error:#}");
                ExitCode::from(2)
            }
        },
        Err(error) => {
            if let Err(evidence_error) = evidence {
                eprintln!("neutron-story story-smoke evidence failed: {evidence_error:#}");
            }
            eprintln!("neutron-story failed: {:#}", anyhow::Error::new(error));
            ExitCode::from(2)
        }
    }
}
