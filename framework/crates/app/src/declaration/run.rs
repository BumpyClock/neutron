//! The private declaration entry point: `run::<A>()`.
//!
//! The whole startup order lives here, before any observable side effect:
//!
//! 1. build the declaration (pure, no I/O),
//! 2. validate it — a malformed declaration returns
//!    [`AppShellError::Declaration`] before paths, the platform, or GPUI exist,
//! 3. parse process facts — a parser failure returns
//!    [`AppShellError::Launch`], equally early,
//! 4. answer an [`PreparedLaunch::ExitSuccess`] request (`--help`,
//!    `--version`) by writing `stdout` and returning success, still with no
//!    platform,
//! 5. lower the declaration into its [`RuntimePlan`] with the prepared launch
//!    runtime, and run that plan.
//!
//! There is one startup implementation: step 5 builds the plan
//! [`RuntimePlan::run`](crate::shell::RuntimePlan::run) executes, and nothing
//! else starts a shell.
//!
//! Private: [`crate::AppShell::run`] and [`crate::testing`] are the exported
//! entry points onto this function. [`testing`] is gated behind the
//! `test-support` feature, matching its `crate::testing` re-export, so a
//! default consumer build never compiles this test-only entry point.

use crate::error::AppShellError;
use crate::shell::PlatformRunner;

use super::{AppDeclaration, DesktopApp, PreparedLaunch, ProcessLaunch};

/// Run an application declaration on the real platform.
///
/// # Errors
///
/// Returns [`AppShellError::Declaration`] for a malformed declaration and
/// [`AppShellError::Launch`] for a launch parse failure — both before any path
/// resolution, platform construction, or GPUI work — and then whatever the
/// shared startup path reports.
pub(crate) fn run<A: DesktopApp>() -> Result<(), AppShellError> {
    execute(
        A::declaration(),
        &ProcessLaunch::from_env(),
        PlatformRunner::native(),
    )
}

/// The deterministic entry points for tests.
///
/// [`run`] uses [`ProcessLaunch::empty`]. [`run_with`] accepts explicit process
/// facts. Both paths run headless and never read or mutate real process state.
/// `tests/headless.rs` exercises these entry points through startup, primary
/// surface creation, event delivery, and shutdown.
#[cfg(feature = "test-support")]
pub mod testing {
    use super::{AppShellError, DesktopApp, PlatformRunner, ProcessLaunch, execute};

    /// Run `A` headless against the empty process facts.
    ///
    /// # Errors
    ///
    /// As [`super::run`].
    pub fn run<A: DesktopApp>() -> Result<(), AppShellError> {
        run_with::<A>(ProcessLaunch::empty())
    }

    /// Run `A` headless against explicit process facts.
    ///
    /// # Errors
    ///
    /// As [`super::run`].
    pub fn run_with<A: DesktopApp>(process: ProcessLaunch) -> Result<(), AppShellError> {
        execute(A::declaration(), &process, PlatformRunner::headless())
    }
}

/// Validate, prepare, and run one declaration. The single implementation behind
/// every entry point.
///
/// The runtime plan is constructed here and nowhere else in production: a plan
/// only ever describes a declaration that already validated and whose launch
/// already parsed.
fn execute(
    declaration: AppDeclaration,
    process: &ProcessLaunch,
    runner: PlatformRunner,
) -> Result<(), AppShellError> {
    declaration.validate().map_err(AppShellError::Declaration)?;

    // Validation first, then parsing: a declaration fault is the application
    // author's mistake and must be reported whatever the user typed.
    let runtime = match declaration.prepare_launch(process)? {
        PreparedLaunch::ExitSuccess { stdout } => {
            if let Some(text) = stdout {
                print!("{text}");
            }
            return Ok(());
        }
        PreparedLaunch::Run(runtime) => runtime,
    };

    declaration.lower(runtime, runner).run()
}

#[cfg(test)]
mod tests {
    //! Every test here uses a tripwire runner and asserts only about the
    //! pre-platform prefix of [`execute`]. `tests/headless.rs` owns the
    //! end-to-end main-thread coverage for [`testing::run`].

    use super::super::tests::identity;
    use super::super::{LaunchDecision, LaunchSpec};
    use super::*;

    /// A runner whose construction always fails: reaching it proves the path
    /// under test did *not* stop before the platform.
    fn tripwire() -> PlatformRunner {
        PlatformRunner::failing()
    }

    fn broken_identity() -> neutron_components_manifest::schema::IdentityRef {
        let mut broken = identity();
        broken.app_id = "";
        broken
    }

    fn parse_fails(_process: &ProcessLaunch) -> anyhow::Result<LaunchDecision<u32>> {
        anyhow::bail!("unrecognized argument")
    }

    fn parse_help(_process: &ProcessLaunch) -> anyhow::Result<LaunchDecision<u32>> {
        Ok(LaunchDecision::ExitSuccess {
            stdout: Some("usage: probe\n".to_string()),
        })
    }

    fn parse_ok(_process: &ProcessLaunch) -> anyhow::Result<LaunchDecision<u32>> {
        Ok(LaunchDecision::Run(7))
    }

    #[test]
    fn a_declaration_fault_is_reported_before_the_platform() {
        let error = execute(
            AppDeclaration::new(broken_identity()),
            &ProcessLaunch::empty(),
            tripwire(),
        )
        .expect_err("an empty app_id is a declaration fault");

        let AppShellError::Declaration(errors) = &error else {
            panic!("expected a declaration error, got {error:?}");
        };
        assert_eq!(errors.len(), 1);
        assert_eq!(
            error.to_string(),
            "invalid application declaration: 1 declaration error: invalid app identity: `app_id` \
             is empty",
        );
    }

    #[test]
    fn a_declaration_fault_wins_over_a_launch_parse_failure() {
        let declaration =
            AppDeclaration::new(broken_identity()).launch(LaunchSpec::new(parse_fails));

        let error = execute(declaration, &ProcessLaunch::empty(), tripwire())
            .expect_err("the declaration is faulty");

        assert!(
            matches!(error, AppShellError::Declaration(_)),
            "validation runs before parsing: {error:?}",
        );
    }

    #[test]
    fn a_launch_parse_failure_is_reported_before_the_platform() {
        let declaration = AppDeclaration::new(identity()).launch(LaunchSpec::new(parse_fails));

        let error = execute(declaration, &ProcessLaunch::empty(), tripwire())
            .expect_err("the parser rejects the process facts");

        assert!(
            matches!(error, AppShellError::Launch(_)),
            "a parse failure is typed, not a generic callback error: {error:?}",
        );
        assert_eq!(
            std::error::Error::source(&error)
                .expect("the cause is preserved")
                .to_string(),
            "unrecognized argument",
        );
    }

    #[test]
    fn an_exit_success_request_never_constructs_the_platform() {
        let declaration = AppDeclaration::new(identity()).launch(LaunchSpec::new(parse_help));

        assert!(
            execute(declaration, &ProcessLaunch::empty(), tripwire()).is_ok(),
            "`--help` must return success without building a platform",
        );
    }

    #[test]
    fn a_valid_declaration_reaches_the_platform() {
        let declaration = AppDeclaration::new(identity()).launch(LaunchSpec::new(parse_ok));

        let error = execute(declaration, &ProcessLaunch::empty(), tripwire())
            .expect_err("the tripwire runner always fails");

        assert!(
            matches!(error, AppShellError::Platform(_)),
            "a well-formed declaration must reach platform construction: {error:?}",
        );
    }
}
