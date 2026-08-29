//! Typed launch input: raw process facts, the application-owned parser, and the
//! one immutable launch value the shell retains for the whole process lifetime.
//!
//! ## Typing
//!
//! [`LaunchSpec<T>`] binds the parser, the optional `before_primary` hook, and
//! the optional typed primary [`Surface<View, T>`] to one `T` at compile time.
//! Erasure into the non-generic [`super::AppDeclaration`] happens only in
//! [`LaunchSpec::erase`], after all three parts are connected, so no
//! application code ever selects a type at runtime.
//!
//! ## Purity
//!
//! Declaring a launch spec is pure. [`super::AppDeclaration::prepare_launch`]
//! is the one impure step: it runs the application's parser against supplied
//! process facts. It reads no ambient state of its own — the caller supplies
//! [`ProcessLaunch`] — so tests parse deterministic values without touching
//! process argv or the working directory.
//!
//! ## Handoff to the orchestrator
//!
//! This module does not touch startup itself. It hands the orchestrator
//! exactly two values, [`PreparedLaunch`] and [`LaunchRuntime`]; see
//! [`super::run::execute`](crate::declaration::run) for the pre-platform
//! sequence (validate, then parse) and [`crate::shell::RuntimePlan::run`]
//! for the post-platform sequence (deferred-quit check, `before_primary`,
//! deferred-quit check, typed primary open, readiness, `Started`, drain).

use std::any::{Any, TypeId, type_name};
use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui::{App, Render};

use crate::error::AppShellError;
use crate::windows::{WindowError, open_surface};

use super::AppDeclaration;
use super::errors::DeclarationError;
use super::surface::{DeclaredSurface, Surface, SurfaceKey, SurfaceRole};

/// The raw process facts a launch parser sees.
///
/// `args` excludes the executable name and preserves non-UTF-8 arguments; the
/// application owns all CLI syntax above that. Both fields are public (issue
/// #29): an external caller may construct or destructure this directly rather
/// than going through the `new`/`empty` constructors and accessors alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessLaunch {
    /// Process arguments, excluding `argv[0]`.
    pub args: Vec<OsString>,
    /// Working directory at launch, if resolvable.
    pub cwd: Option<PathBuf>,
}

impl ProcessLaunch {
    /// Collect the facts once from the process environment. Production only.
    pub(crate) fn from_env() -> Self {
        Self {
            args: std::env::args_os().skip(1).collect(),
            cwd: std::env::current_dir().ok(),
        }
    }

    /// The deterministic empty value: no arguments and no working directory.
    ///
    /// This is what the zero-argument test entry point parses, so a test run is
    /// identical on every machine and never reads or mutates process state.
    pub fn empty() -> Self {
        Self {
            args: Vec::new(),
            cwd: None,
        }
    }

    /// Construct explicit process facts, deterministic and independent of the
    /// real process: a test or an embedder supplies
    /// exactly the arguments and working directory a parser should see,
    /// without ever reading or mutating real argv or the real working
    /// directory.
    pub fn new(args: Vec<OsString>, cwd: Option<PathBuf>) -> Self {
        Self { args, cwd }
    }

    /// Process arguments, excluding `argv[0]`, preserved verbatim including
    /// non-UTF-8 values.
    pub fn args(&self) -> &[OsString] {
        &self.args
    }

    /// The working directory at launch, if resolvable.
    pub fn cwd(&self) -> Option<&Path> {
        self.cwd.as_deref()
    }
}

/// What an application's launch parser decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchDecision<T> {
    /// Continue startup with this immutable launch value.
    Run(T),
    /// The request was answered without running the application (`--help`,
    /// `--version`). The shell writes `stdout`, if any, and returns success
    /// before any path, platform, or GPUI construction.
    ExitSuccess {
        /// Text to write to standard output verbatim.
        stdout: Option<String>,
    },
}

/// Parses raw process facts into an application-owned launch value.
///
/// A non-capturing `fn` pointer: parsing must be deterministic and
/// side-effect-free, so it has no reason to close over state.
pub(crate) type LaunchParser<T> = fn(&ProcessLaunch) -> anyhow::Result<LaunchDecision<T>>;

/// Launch-specific work run after the common start hook and before the primary
/// surface is created. Non-capturing for the same reason as [`LaunchParser`].
pub(crate) type BeforePrimaryHook<T> = fn(&T, &mut App) -> anyhow::Result<()>;

/// One complete typed launch module.
///
/// At most one may be declared. `T` ties the parser, the `before_primary` hook,
/// and the primary surface's argument type together at compile time.
pub struct LaunchSpec<T> {
    parser: LaunchParser<T>,
    before_primary: Option<BeforePrimaryHook<T>>,
    /// How many `before_primary` hooks were declared beyond the first.
    ///
    /// Counted rather than applied: only one hook runs, and losing a declared
    /// hook silently would skip launch work the application asked for.
    surplus_hooks: usize,
    /// Declared primary surfaces, erased with their typed opener.
    ///
    /// A `Vec` rather than an `Option` so a repeated `primary_surface` call is
    /// never silently last-wins: every declaration reaches the declaration's
    /// surface index and the existing "only one primary" check reports it.
    primaries: Vec<(DeclaredSurface, PrimaryOpener)>,
}

impl<T: 'static> LaunchSpec<T> {
    /// Declare the application's launch parser.
    #[must_use]
    pub fn new(parser: LaunchParser<T>) -> Self {
        Self {
            parser,
            before_primary: None,
            surplus_hooks: 0,
            primaries: Vec::new(),
        }
    }

    /// Run launch-specific work with the parsed value, after the common start
    /// hook and before the primary surface is created.
    ///
    /// At most one may be declared. A second is counted and reported by
    /// [`super::AppDeclaration::validate`] rather than replacing the first, so
    /// a declaration can never silently drop launch work it asked for.
    #[must_use]
    pub fn before_primary(mut self, hook: BeforePrimaryHook<T>) -> Self {
        match self.before_primary {
            None => self.before_primary = Some(hook),
            Some(_) => self.surplus_hooks += 1,
        }
        self
    }

    /// Declare the primary surface, whose open arguments are the launch value.
    #[must_use]
    pub fn primary_surface<View: 'static + Render>(mut self, surface: Surface<View, T>) -> Self {
        self.primaries.push((
            DeclaredSurface::erase(surface, SurfaceRole::Primary),
            PrimaryOpener::of::<View, T>(),
        ));
        self
    }

    /// Erase the completed spec.
    ///
    /// The only erasure point, reached once every typed part has been bound to
    /// the same `T`.
    fn erase(self) -> (DeclaredLaunch, Vec<(DeclaredSurface, PrimaryOpener)>) {
        let Self {
            parser,
            before_primary,
            surplus_hooks,
            primaries,
        } = self;
        let declared = DeclaredLaunch {
            launch: Rc::new(TypedLaunch {
                parser,
                before_primary,
            }),
            surplus_hooks,
        };
        (declared, primaries)
    }
}

/// Retained identity of the launch value type, for diagnostics and for the
/// downcast that recovers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LaunchTypes {
    /// `TypeId` of the launch value.
    pub(crate) value: TypeId,
    /// `type_name` of the launch value.
    pub(crate) value_name: &'static str,
}

impl LaunchTypes {
    fn of<T: 'static>() -> Self {
        Self {
            value: TypeId::of::<T>(),
            value_name: type_name::<T>(),
        }
    }
}

/// The parser/hook pair behind one object-safe seam.
///
/// Deliberately not `Send`/`Sync`: a declaration is built and consumed on the
/// main thread.
trait ErasedLaunch: 'static {
    fn types(&self) -> LaunchTypes;

    fn parse(&self, process: &ProcessLaunch) -> anyhow::Result<LaunchDecision<Box<dyn Any>>>;

    #[allow(dead_code)] // Exercised only by unit tests; no production caller yet.
    fn has_before_primary(&self) -> bool;

    fn run_before_primary(&self, value: &dyn Any, cx: &mut App) -> anyhow::Result<()>;
}

struct TypedLaunch<T: 'static> {
    parser: LaunchParser<T>,
    before_primary: Option<BeforePrimaryHook<T>>,
}

impl<T: 'static> ErasedLaunch for TypedLaunch<T> {
    fn types(&self) -> LaunchTypes {
        LaunchTypes::of::<T>()
    }

    fn parse(&self, process: &ProcessLaunch) -> anyhow::Result<LaunchDecision<Box<dyn Any>>> {
        Ok(match (self.parser)(process)? {
            LaunchDecision::Run(value) => LaunchDecision::Run(Box::new(value)),
            LaunchDecision::ExitSuccess { stdout } => LaunchDecision::ExitSuccess { stdout },
        })
    }

    fn has_before_primary(&self) -> bool {
        self.before_primary.is_some()
    }

    fn run_before_primary(&self, value: &dyn Any, cx: &mut App) -> anyhow::Result<()> {
        let Some(hook) = self.before_primary else {
            return Ok(());
        };
        let value = value
            .downcast_ref::<T>()
            .expect("a launch hook only ever sees the value its own parser produced");
        hook(value, cx)
    }
}

/// One erased launch module held by [`AppDeclaration`].
pub(crate) struct DeclaredLaunch {
    launch: Rc<dyn ErasedLaunch>,
    surplus_hooks: usize,
}

impl DeclaredLaunch {
    /// The launch value's retained type identity.
    pub(crate) fn types(&self) -> LaunchTypes {
        self.launch.types()
    }
}

/// Opens the declared primary surface from the erased retained launch value.
///
/// Built at the declaration site, where the content view type and the argument
/// type are both statically known, so the runtime never selects a type.
#[derive(Clone, Copy)]
pub(crate) struct PrimaryOpener {
    args: TypeId,
    args_name: &'static str,
    open: fn(&dyn Any, &mut App) -> Result<(), WindowError>,
}

impl PrimaryOpener {
    /// Capture the opener for a primary `Surface<View, Args>`.
    pub(crate) fn of<View: 'static + Render, Args: 'static>() -> Self {
        Self {
            args: TypeId::of::<Args>(),
            args_name: type_name::<Args>(),
            open: open_primary::<View, Args>,
        }
    }

    /// `type_name` of the arguments this primary surface requires.
    #[allow(dead_code)] // Exercised only by unit tests; no production caller yet.
    pub(crate) fn args_name(&self) -> &'static str {
        self.args_name
    }
}

impl fmt::Debug for PrimaryOpener {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("PrimaryOpener")
            .field(&self.args_name)
            .finish()
    }
}

fn open_primary<View: 'static + Render, Args: 'static>(
    value: &dyn Any,
    cx: &mut App,
) -> Result<(), WindowError> {
    let args = value
        .downcast_ref::<Args>()
        .expect("the primary opener is type-checked against the launch value before it runs");
    open_surface(cx, SurfaceKey::<View, Args>::primary(), args).map(|_| ())
}

/// The outcome of parsing process facts, before any platform work starts.
pub(crate) enum PreparedLaunch {
    /// Startup continues with this runtime.
    Run(LaunchRuntime),
    /// The parser answered the request itself. The caller writes `stdout` and
    /// returns success without constructing paths, the platform, or GPUI.
    ExitSuccess {
        /// Text to write to standard output verbatim.
        stdout: Option<String>,
    },
}

/// The retained immutable launch value plus the launch-specific runtime steps.
///
/// See the module docs for the exact call sequence the orchestrator executes.
pub(crate) struct LaunchRuntime {
    value: Box<dyn Any>,
    types: LaunchTypes,
    /// `None` when the declaration has no [`LaunchSpec`]; the value is `()`.
    launch: Option<Rc<dyn ErasedLaunch>>,
    primary: Option<PrimaryOpener>,
}

impl LaunchRuntime {
    /// The runtime for a declaration without a [`LaunchSpec`]: a unit launch
    /// value and no launch-specific hook. A declared unit primary surface still
    /// opens through [`LaunchRuntime::open_primary`].
    pub(crate) fn unit(primary: Option<PrimaryOpener>) -> Self {
        Self {
            value: Box::new(()),
            types: LaunchTypes::of::<()>(),
            launch: None,
            primary,
        }
    }

    /// The retained launch value, when it really has type `T`.
    ///
    /// The shell borrows the value; it is never handed out by value and never
    /// published as a global, so a mismatched `T` is a `None`, not a panic.
    #[allow(dead_code)] // Exercised only by unit tests; no production caller yet.
    pub(crate) fn value<T: 'static>(&self) -> Option<&T> {
        self.value.downcast_ref::<T>()
    }

    /// The launch value's retained type identity.
    #[allow(dead_code)] // Exercised only by unit tests; no production caller yet.
    pub(crate) fn types(&self) -> LaunchTypes {
        self.types
    }

    /// Whether a `before_primary` hook was declared.
    #[allow(dead_code)] // Exercised only by unit tests; no production caller yet.
    pub(crate) fn has_before_primary(&self) -> bool {
        self.launch
            .as_ref()
            .is_some_and(|launch| launch.has_before_primary())
    }

    /// Whether a primary surface was declared.
    #[allow(dead_code)] // Exercised only by unit tests; no production caller yet.
    pub(crate) fn has_primary(&self) -> bool {
        self.primary.is_some()
    }

    /// Run the declared `before_primary` hook once, with the retained value.
    ///
    /// A no-op when no hook was declared. A hook failure is part of the fatal
    /// start/primary composition transaction, so it maps to
    /// [`AppShellError::Startup`].
    pub(crate) fn before_primary(&self, cx: &mut App) -> Result<(), AppShellError> {
        let Some(launch) = &self.launch else {
            return Ok(());
        };
        launch
            .run_before_primary(self.value.as_ref(), cx)
            .map_err(AppShellError::Startup)
    }

    /// Open the declared primary surface with the retained launch value.
    ///
    /// A no-op when no primary surface was declared: such an application is a
    /// background process. Used both for the initial startup open, where the
    /// caller classifies a failure as `AppShellError::Startup` because it is
    /// fatal there, and later to restore the primary on `Reopened` (see
    /// `handles::restore_primary_on_reopen`), where the caller reports the
    /// same underlying error directly as `RuntimeError::lifecycle` instead.
    ///
    /// Returns the underlying failure undecorated — no startup-specific
    /// wrapping — so each caller applies its own classification instead of
    /// inheriting a `Startup` label that is only accurate for one of them.
    pub(crate) fn open_primary(&self, cx: &mut App) -> Result<(), anyhow::Error> {
        let Some(primary) = self.primary else {
            return Ok(());
        };
        // Only reachable when a declared `LaunchSpec<T>` (`T != ()`) coexists
        // with a primary declared through `AppDeclaration::primary_surface`,
        // which always takes unit arguments and so can never itself carry a
        // non-unit argument type. The matching declaration fault is reported
        // by pure validation; this is the runtime backstop that keeps the
        // erased opener's downcast sound.
        if primary.args != self.types.value {
            return Err(anyhow::anyhow!(
                "the primary surface takes `{}` but the launch value is `{}`",
                primary.args_name,
                self.types.value_name,
            ));
        }
        (primary.open)(self.value.as_ref(), cx).map_err(anyhow::Error::from)
    }
}

/// Launch faults: a surplus [`LaunchSpec`], and a primary surface whose
/// arguments nothing produces.
///
/// [`LaunchSpec::primary_surface`] ties the primary's arguments to the launch
/// value's type, so it can never produce the second fault.
/// [`AppDeclaration::primary_surface`] always takes unit arguments and cannot
/// itself carry a non-unit argument type, so the fault is only reachable when
/// a declared `LaunchSpec<T>` (`T != ()`) leaves that unit primary with no
/// matching launch value. Without this rule such a
/// primary would be declared, validated, and then impossible to open.
pub(crate) fn validate_launch_set(
    launch: &[DeclaredLaunch],
    primary: &[PrimaryOpener],
    errors: &mut Vec<DeclarationError>,
) {
    for declared in launch.iter().skip(1) {
        errors.push(DeclarationError::MultipleLaunchSpecs {
            launch: declared.types().value_name,
        });
    }
    // Every spec's surplus hooks are reported, including a surplus spec's, so
    // fixing the duplicate spec cannot uncover a second, previously silent
    // mistake.
    for declared in launch {
        for _ in 0..declared.surplus_hooks {
            errors.push(DeclarationError::MultipleBeforePrimaryHooks {
                launch: declared.types().value_name,
            });
        }
    }

    let value = launch
        .first()
        .map_or_else(LaunchTypes::of::<()>, DeclaredLaunch::types);
    // A duplicate primary is already reported as a surface fault; blaming the
    // arguments too would report one mistake twice, so any matching primary
    // clears the set.
    if !primary.is_empty() && !primary.iter().any(|opener| opener.args == value.value) {
        errors.push(DeclarationError::PrimarySurfaceArguments {
            arguments: primary[0].args_name,
        });
    }
}

impl AppDeclaration {
    /// Declare the application's typed launch module.
    ///
    /// At most one may be declared; a second is reported by
    /// [`AppDeclaration::validate`] rather than replacing the first. The
    /// spec's primary surface joins the declaration's surface index, so it
    /// takes part in the same ID, role, and cardinality validation as any
    /// other declared surface.
    #[must_use]
    pub fn launch<T: 'static>(mut self, spec: LaunchSpec<T>) -> Self {
        let (declared, primaries) = spec.erase();
        self.launch.push(declared);
        for (surface, opener) in primaries {
            self.primary.push(opener);
            self = self.declare_surface(surface);
        }
        self
    }

    /// Parse `process` with the declared launch parser, exactly once.
    ///
    /// Must run before paths, the platform, and GPUI exist, so an
    /// [`PreparedLaunch::ExitSuccess`] request costs nothing. A declaration
    /// without a [`LaunchSpec`] parses nothing and yields a unit runtime.
    pub(crate) fn prepare_launch(
        &self,
        process: &ProcessLaunch,
    ) -> Result<PreparedLaunch, AppShellError> {
        let primary = self.primary.first().copied();
        let Some(declared) = self.launch.first() else {
            return Ok(PreparedLaunch::Run(LaunchRuntime::unit(primary)));
        };
        match declared
            .launch
            .parse(process)
            .map_err(AppShellError::Launch)?
        {
            LaunchDecision::ExitSuccess { stdout } => Ok(PreparedLaunch::ExitSuccess { stdout }),
            LaunchDecision::Run(value) => Ok(PreparedLaunch::Run(LaunchRuntime {
                value,
                types: declared.types(),
                launch: Some(Rc::clone(&declared.launch)),
                primary,
            })),
        }
    }
}

#[cfg(test)]
mod tests {
    use gpui::Empty;

    use super::super::surface::tests::build_empty;
    use super::super::tests::identity;
    use super::*;

    /// A launch value that is deliberately not `()`, so a type round-trip test
    /// cannot pass by accident.
    #[derive(Debug, PartialEq, Eq)]
    struct Args {
        story: Option<String>,
        smoke: bool,
    }

    fn parse(process: &ProcessLaunch) -> anyhow::Result<LaunchDecision<Args>> {
        let mut story = None;
        let mut smoke = false;
        let mut args = process.args.iter();
        while let Some(arg) = args.next() {
            match arg.to_str() {
                Some("--help") => {
                    return Ok(LaunchDecision::ExitSuccess {
                        stdout: Some("usage\n".to_string()),
                    });
                }
                Some("--smoke") => smoke = true,
                Some("--story") => {
                    let value = args
                        .next()
                        .ok_or_else(|| anyhow::anyhow!("--story needs a value"))?;
                    story = Some(value.to_string_lossy().into_owned());
                }
                _ => anyhow::bail!("unexpected argument"),
            }
        }
        Ok(LaunchDecision::Run(Args { story, smoke }))
    }

    fn before_primary(_args: &Args, _cx: &mut App) -> anyhow::Result<()> {
        Ok(())
    }

    fn build_from_args(_: &Args, _: &mut gpui::Window, cx: &mut App) -> gpui::Entity<Empty> {
        use gpui::AppContext as _;
        cx.new(|_| Empty)
    }

    fn process(args: &[&str]) -> ProcessLaunch {
        ProcessLaunch {
            args: args.iter().map(OsString::from).collect(),
            cwd: None,
        }
    }

    fn faults(declaration: &AppDeclaration) -> Vec<String> {
        declaration
            .validate()
            .expect_err("the declaration is faulty")
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    #[test]
    fn the_empty_process_value_is_deterministic() {
        assert_eq!(
            ProcessLaunch::empty(),
            ProcessLaunch {
                args: Vec::new(),
                cwd: None
            },
        );
    }

    #[test]
    fn a_parsed_value_round_trips_through_erasure_with_its_own_type() {
        let declaration = AppDeclaration::new(identity())
            .launch(LaunchSpec::new(parse).before_primary(before_primary));

        let PreparedLaunch::Run(runtime) = declaration
            .prepare_launch(&process(&["--story", "button", "--smoke"]))
            .expect("the parser succeeds")
        else {
            panic!("the parser returned Run");
        };

        assert_eq!(
            runtime.value::<Args>(),
            Some(&Args {
                story: Some("button".to_string()),
                smoke: true,
            }),
        );
        assert_eq!(runtime.types().value, TypeId::of::<Args>());
        assert!(
            runtime.value::<()>().is_none(),
            "erasure must not hand back an unrelated type",
        );
        assert!(runtime.has_before_primary());
    }

    #[test]
    fn a_declaration_without_a_launch_spec_retains_a_unit_value() {
        let declaration = AppDeclaration::new(identity());

        let PreparedLaunch::Run(runtime) = declaration
            .prepare_launch(&ProcessLaunch::empty())
            .expect("no parser can fail")
        else {
            panic!("an undeclared launch always runs");
        };

        assert_eq!(runtime.value::<()>(), Some(&()));
        assert!(!runtime.has_before_primary());
        assert!(!runtime.has_primary());
    }

    #[test]
    fn an_exit_success_decision_survives_erasure_with_its_stdout() {
        let declaration = AppDeclaration::new(identity()).launch(LaunchSpec::new(parse));

        let prepared = declaration
            .prepare_launch(&process(&["--help"]))
            .expect("help is a success");

        match prepared {
            PreparedLaunch::ExitSuccess { stdout } => {
                assert_eq!(stdout.as_deref(), Some("usage\n"));
            }
            PreparedLaunch::Run(_) => panic!("help must not start the application"),
        }
    }

    #[test]
    fn a_parser_failure_is_a_launch_error_with_its_source_chain() {
        use std::error::Error as _;

        let declaration = AppDeclaration::new(identity()).launch(LaunchSpec::new(parse));

        let error = declaration
            .prepare_launch(&process(&["--nonsense"]))
            .err()
            .expect("an unknown flag is a usage error");

        assert!(
            matches!(error, AppShellError::Launch(_)),
            "a parser failure must not be reported as preparation: {error}",
        );
        assert_eq!(
            error.source().expect("the cause is preserved").to_string(),
            "unexpected argument",
        );
    }

    #[test]
    fn a_launch_primary_surface_is_registered_and_typed() {
        let declaration =
            AppDeclaration::new(identity()).launch(LaunchSpec::new(parse).primary_surface(
                Surface::new(SurfaceKey::<Empty, Args>::primary(), build_from_args),
            ));

        assert!(declaration.validate().is_ok());
        assert_eq!(
            declaration.surfaces,
            vec![("primary", SurfaceRole::Primary)],
            "the launch primary joins the shared surface index",
        );
        assert_eq!(
            declaration
                .primary
                .first()
                .expect("the launch primary records an opener")
                .args_name(),
            type_name::<Args>(),
        );

        let PreparedLaunch::Run(runtime) = declaration
            .prepare_launch(&ProcessLaunch::empty())
            .expect("no arguments parse")
        else {
            panic!("the parser returned Run");
        };
        assert!(runtime.has_primary());
    }

    #[test]
    fn a_launch_primary_collides_with_a_declared_primary_surface() {
        let declaration = AppDeclaration::new(identity())
            .primary_surface(Surface::new(SurfaceKey::<Empty>::primary(), build_empty))
            .launch(LaunchSpec::new(parse).primary_surface(Surface::new(
                SurfaceKey::<Empty, Args>::primary(),
                build_from_args,
            )));

        assert_eq!(
            faults(&declaration),
            vec![
                "surface id `primary` is declared more than once".to_string(),
                "only one primary surface may be declared; `primary` is a second one".to_string(),
            ],
            "a launch primary is validated exactly like any other primary",
        );
    }

    #[test]
    fn a_repeated_primary_inside_one_spec_is_not_last_wins() {
        let declaration = AppDeclaration::new(identity()).launch(
            LaunchSpec::new(parse)
                .primary_surface(Surface::new(
                    SurfaceKey::<Empty, Args>::primary(),
                    build_from_args,
                ))
                .primary_surface(Surface::new(
                    SurfaceKey::<Empty, Args>::primary(),
                    build_from_args,
                )),
        );

        assert_eq!(
            faults(&declaration),
            vec![
                "surface id `primary` is declared more than once".to_string(),
                "only one primary surface may be declared; `primary` is a second one".to_string(),
            ],
        );
    }

    #[test]
    fn a_second_before_primary_hook_is_reported_and_never_replaces_the_first() {
        fn later(_: &Args, _: &mut App) -> anyhow::Result<()> {
            unreachable!("the second hook must never be retained")
        }

        let declaration = AppDeclaration::new(identity()).launch(
            LaunchSpec::new(parse)
                .before_primary(before_primary)
                .before_primary(later),
        );

        assert_eq!(
            faults(&declaration),
            vec![format!(
                "only one before_primary hook may be declared; the launch specification for \
                 `{}` declares a second one",
                type_name::<Args>(),
            )],
        );

        let PreparedLaunch::Run(runtime) = declaration
            .prepare_launch(&ProcessLaunch::empty())
            .expect("the parser succeeds")
        else {
            panic!("the parser returned Run");
        };
        assert!(
            runtime.has_before_primary(),
            "the first hook is still the one retained",
        );
    }

    #[test]
    fn every_surplus_before_primary_hook_is_reported() {
        fn later(_: &Args, _: &mut App) -> anyhow::Result<()> {
            unreachable!("the surplus hooks must never be retained")
        }

        let declaration = AppDeclaration::new(identity()).launch(
            LaunchSpec::new(parse)
                .before_primary(before_primary)
                .before_primary(later)
                .before_primary(later),
        );

        assert_eq!(
            faults(&declaration).len(),
            2,
            "one fault per surplus hook, so the count is not lost",
        );
    }

    #[test]
    fn one_before_primary_hook_is_not_a_fault() {
        let declaration = AppDeclaration::new(identity())
            .launch(LaunchSpec::new(parse).before_primary(before_primary));

        assert!(declaration.validate().is_ok());
    }

    #[test]
    fn a_second_launch_spec_is_reported_and_never_replaces_the_first() {
        fn other(_: &ProcessLaunch) -> anyhow::Result<LaunchDecision<()>> {
            anyhow::bail!("the second parser must never run")
        }

        let declaration = AppDeclaration::new(identity())
            .launch(LaunchSpec::new(parse))
            .launch(LaunchSpec::new(other));

        assert_eq!(
            faults(&declaration),
            vec![format!(
                "only one launch specification may be declared; `{}` is a second one",
                type_name::<()>(),
            )],
        );

        let PreparedLaunch::Run(runtime) = declaration
            .prepare_launch(&ProcessLaunch::empty())
            .expect("the first parser is the one that runs")
        else {
            panic!("the parser returned Run");
        };
        assert_eq!(runtime.types().value, TypeId::of::<Args>());
    }

    #[test]
    fn a_primary_whose_arguments_no_launch_produces_is_a_declaration_fault() {
        // `AppDeclaration::primary_surface` is unit-only, so this primary
        // takes `()`; the declared launch specification produces `Args`
        // instead, so nothing produces the primary's arguments.
        let declaration = AppDeclaration::new(identity())
            .launch(LaunchSpec::new(parse))
            .primary_surface(Surface::new(SurfaceKey::<Empty>::primary(), build_empty));

        assert_eq!(
            faults(&declaration),
            vec![format!(
                "the primary surface takes `{}` but no launch specification produces it",
                type_name::<()>(),
            )],
        );
    }
}
