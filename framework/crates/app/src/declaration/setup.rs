//! Typed application setup modules: a validated stable key, explicit
//! dependencies, a one-time initializer, and an optional teardown that consumes
//! the module's private state.
//!
//! ## Typing
//!
//! [`SetupModule<State>`] keeps `State` local to the constructor. Erasure into
//! the non-generic [`super::AppDeclaration`] happens once, in
//! [`DeclaredSetupModule::erase`], after the initializer and teardown are bound
//! to the same `State`. `State` exists only so the shell owns the lifetime of
//! whatever the module registered; it is never handed back to the application.
//!
//! ## Purity
//!
//! Declaring and planning a setup graph is pure: [`plan`] validates keys,
//! dependencies, and cycles and resolves the deterministic initialization order
//! without GPUI, the filesystem, or host inspection. The runtime half lives in
//! [`crate::setup`].

use std::any::{Any, type_name};
use std::fmt;

use crate::setup::SetupContext;

use super::errors::DeclarationError;

/// A validated stable setup-module identifier.
///
/// Used for duplicate detection, dependency references, declaration
/// diagnostics, startup error attribution, and teardown reporting. The value is
/// checked by [`plan`], not by the constructor, so keys stay usable as
/// associated constants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetupKey(&'static str);

impl SetupKey {
    /// A key for an application-chosen stable identifier.
    pub const fn new(key: &'static str) -> Self {
        Self(key)
    }

    /// The stable identifier.
    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

impl fmt::Display for SetupKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

/// One-time registration work. Non-capturing by design: a declaration is a pure
/// value the shell may retain for the whole process lifetime.
pub(crate) type SetupInit<State> = fn(&mut SetupContext<'_>) -> anyhow::Result<State>;

/// Teardown that consumes the module's retained state.
pub(crate) type SetupTeardown<State> = fn(State, &mut SetupContext<'_>) -> anyhow::Result<()>;

/// One declared application setup module.
///
/// Registration-only modules use `State = ()`. Modules that create
/// subscriptions or registration handles return them as state so the shell owns
/// their lifetime.
pub struct SetupModule<State = ()> {
    key: SetupKey,
    dependencies: Vec<SetupKey>,
    init: SetupInit<State>,
    teardown: Option<SetupTeardown<State>>,
    /// How many teardowns were declared beyond the first.
    ///
    /// Counted rather than applied: a repeated `shutdown` is a declaration
    /// mistake, and the first hook is the one that runs.
    surplus_teardowns: usize,
}

impl<State: 'static> SetupModule<State> {
    /// Declare a setup module under `key`, initialized by `init`.
    #[must_use]
    pub fn new(key: SetupKey, init: SetupInit<State>) -> Self {
        Self {
            key,
            dependencies: Vec::new(),
            init,
            teardown: None,
            surplus_teardowns: 0,
        }
    }

    /// Require another application setup module to initialize first.
    ///
    /// Repeatable. Every framework module is already initialized before any
    /// application setup module runs, so only application keys are nameable.
    ///
    /// "Initializes before this one" is a set membership, not a count, so
    /// naming the same dependency twice is idempotent: the repeat is dropped
    /// here and the first mention keeps its position. That keeps one
    /// undeclared dependency from being reported once per mention, and keeps
    /// the resolved order identical whether or not it was repeated.
    #[must_use]
    pub fn after(mut self, dependency: SetupKey) -> Self {
        if !self.dependencies.contains(&dependency) {
            self.dependencies.push(dependency);
        }
        self
    }

    /// Tear the module down, consuming its retained state.
    ///
    /// At most one may be declared. A second is counted and reported by
    /// [`plan`] rather than replacing the first, so a module can never lose the
    /// teardown it declared to a later call.
    #[must_use]
    pub fn shutdown(mut self, teardown: SetupTeardown<State>) -> Self {
        match self.teardown {
            None => self.teardown = Some(teardown),
            Some(_) => self.surplus_teardowns += 1,
        }
        self
    }

    /// The declared key.
    #[allow(dead_code)] // Exercised only by unit tests; no production caller yet.
    pub(crate) fn key(&self) -> SetupKey {
        self.key
    }
}

/// The init/teardown pair behind one object-safe seam.
///
/// Deliberately not `Send`/`Sync`: setup runs on the main thread.
trait ErasedSetup: 'static {
    #[allow(dead_code)] // Exercised only by unit tests; no production caller yet.
    fn state_name(&self) -> &'static str;

    #[allow(dead_code)] // Exercised only by unit tests; no production caller yet.
    fn has_teardown(&self) -> bool;

    fn init(&self, cx: &mut SetupContext<'_>) -> anyhow::Result<Box<dyn Any>>;

    fn teardown(&self, state: Box<dyn Any>, cx: &mut SetupContext<'_>) -> anyhow::Result<()>;
}

struct TypedSetup<State: 'static> {
    init: SetupInit<State>,
    teardown: Option<SetupTeardown<State>>,
}

impl<State: 'static> ErasedSetup for TypedSetup<State> {
    fn state_name(&self) -> &'static str {
        type_name::<State>()
    }

    fn has_teardown(&self) -> bool {
        self.teardown.is_some()
    }

    fn init(&self, cx: &mut SetupContext<'_>) -> anyhow::Result<Box<dyn Any>> {
        Ok(Box::new((self.init)(cx)?))
    }

    fn teardown(&self, state: Box<dyn Any>, cx: &mut SetupContext<'_>) -> anyhow::Result<()> {
        let state = *state
            .downcast::<State>()
            .expect("a setup module only ever tears down the state its own init produced");
        match self.teardown {
            // Dropping here, in reverse resolved order, is the teardown of a
            // module that declared none: its state's `Drop` still runs late.
            None => {
                drop(state);
                Ok(())
            }
            Some(teardown) => teardown(state, cx),
        }
    }
}

/// One complete typed setup module, erased.
///
/// The key and dependencies stay plain data so pure validation and ordering
/// never need to know `State`.
pub(crate) struct DeclaredSetupModule {
    key: SetupKey,
    dependencies: Vec<SetupKey>,
    surplus_teardowns: usize,
    module: Box<dyn ErasedSetup>,
}

impl DeclaredSetupModule {
    /// Erase a complete typed setup module.
    pub(crate) fn erase<State: 'static>(module: SetupModule<State>) -> Self {
        let SetupModule {
            key,
            dependencies,
            init,
            teardown,
            surplus_teardowns,
        } = module;
        Self {
            key,
            dependencies,
            surplus_teardowns,
            module: Box::new(TypedSetup { init, teardown }),
        }
    }

    /// The declared key.
    pub(crate) fn key(&self) -> SetupKey {
        self.key
    }

    /// The declared dependencies: deduplicated, in first-mention order.
    pub(crate) fn dependencies(&self) -> &[SetupKey] {
        &self.dependencies
    }

    /// `type_name` of the retained state, for diagnostics.
    #[allow(dead_code)] // Exercised only by unit tests; no production caller yet.
    pub(crate) fn state_name(&self) -> &'static str {
        self.module.state_name()
    }

    /// Whether an explicit teardown was declared.
    #[allow(dead_code)] // Exercised only by unit tests; no production caller yet.
    pub(crate) fn has_teardown(&self) -> bool {
        self.module.has_teardown()
    }

    /// Run the initializer once, returning the erased retained state.
    pub(crate) fn init(&self, cx: &mut SetupContext<'_>) -> anyhow::Result<Box<dyn Any>> {
        self.module.init(cx)
    }

    /// Run teardown, consuming the erased retained state.
    pub(crate) fn teardown(
        &self,
        state: Box<dyn Any>,
        cx: &mut SetupContext<'_>,
    ) -> anyhow::Result<()> {
        self.module.teardown(state, cx)
    }
}

/// Validate the declared setup graph and resolve its initialization order.
///
/// Pure. On success the returned indices are a deterministic topological order
/// of `modules`: the earliest-declared module whose dependencies are already
/// resolved always goes next, so independent modules keep declaration order.
///
/// Faults are reported in two stages, because a topological order over an
/// ill-formed graph would be arbitrary:
///
/// 1. invalid keys, duplicate keys, surplus teardowns, and missing
///    dependencies, in declaration order (and, within a module, in dependency
///    declaration order);
/// 2. only if stage 1 is clean, dependency cycles, naming every module that
///    could not be resolved, in declaration order.
pub(crate) fn plan(modules: &[DeclaredSetupModule]) -> Result<Vec<usize>, Vec<DeclarationError>> {
    let mut errors = Vec::new();

    let mut seen: Vec<&'static str> = Vec::new();
    for module in modules {
        let key = module.key().as_str();
        if let Some(reason) = invalid_key_reason(key) {
            errors.push(DeclarationError::InvalidSetupKey { key, reason });
        }
        if seen.contains(&key) {
            errors.push(DeclarationError::DuplicateSetupKey { key });
        } else {
            seen.push(key);
        }
        for _ in 0..module.surplus_teardowns {
            errors.push(DeclarationError::MultipleSetupTeardowns { key });
        }
    }
    for module in modules {
        for dependency in module.dependencies() {
            if !modules
                .iter()
                .any(|candidate| candidate.key() == *dependency)
            {
                errors.push(DeclarationError::MissingSetupDependency {
                    key: module.key().as_str(),
                    dependency: dependency.as_str(),
                });
            }
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }

    let mut resolved = vec![false; modules.len()];
    let mut order = Vec::with_capacity(modules.len());
    loop {
        let next = modules.iter().enumerate().find(|(index, module)| {
            !resolved[*index]
                && module.dependencies().iter().all(|dependency| {
                    modules
                        .iter()
                        .enumerate()
                        .any(|(other, candidate)| resolved[other] && candidate.key() == *dependency)
                })
        });
        match next {
            Some((index, _)) => {
                resolved[index] = true;
                order.push(index);
            }
            None => break,
        }
    }

    if order.len() != modules.len() {
        for (index, module) in modules.iter().enumerate() {
            if !resolved[index] {
                errors.push(DeclarationError::SetupDependencyCycle {
                    key: module.key().as_str(),
                });
            }
        }
        return Err(errors);
    }
    Ok(order)
}

/// Why `key` is unusable as a stable setup key, if it is.
fn invalid_key_reason(key: &str) -> Option<&'static str> {
    if key.is_empty() {
        return Some("is empty");
    }
    if !key
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
    {
        return Some("must contain only ASCII letters, digits, `.`, `-`, or `_`");
    }
    None
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn init_unit(_: &mut SetupContext<'_>) -> anyhow::Result<()> {
        Ok(())
    }

    pub(crate) fn teardown_unit(_: (), _: &mut SetupContext<'_>) -> anyhow::Result<()> {
        Ok(())
    }

    fn module(key: &'static str, after: &[&'static str]) -> DeclaredSetupModule {
        let mut declared = SetupModule::new(SetupKey::new(key), init_unit);
        for dependency in after {
            declared = declared.after(SetupKey::new(dependency));
        }
        DeclaredSetupModule::erase(declared)
    }

    fn keys(modules: &[DeclaredSetupModule], order: Vec<usize>) -> Vec<&'static str> {
        order
            .into_iter()
            .map(|i| modules[i].key().as_str())
            .collect()
    }

    fn errors(modules: &[DeclaredSetupModule]) -> Vec<String> {
        plan(modules)
            .expect_err("the graph is faulty")
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    #[test]
    fn independent_modules_keep_declaration_order() {
        let modules = vec![module("a", &[]), module("b", &[]), module("c", &[])];

        assert_eq!(
            keys(&modules, plan(&modules).expect("the graph is well formed")),
            ["a", "b", "c"],
        );
    }

    #[test]
    fn a_dependency_is_hoisted_and_nothing_else_moves() {
        // `c` is declared first but depends on `a`, so `a` must be hoisted in
        // front of it. That is the only reordering the graph forces: every
        // other pair keeps its declaration order, including `c` before `b`.
        let modules = vec![
            module("c", &["a"]),
            module("a", &[]),
            module("b", &[]),
            module("d", &[]),
        ];

        assert_eq!(
            keys(&modules, plan(&modules).expect("the graph is well formed")),
            ["a", "c", "b", "d"],
            "a dependency hoists its dependent's prerequisite, not the dependent",
        );
    }

    #[test]
    fn a_transitive_chain_resolves_in_dependency_order() {
        let modules = vec![
            module("third", &["second"]),
            module("second", &["first"]),
            module("first", &[]),
        ];

        assert_eq!(
            keys(&modules, plan(&modules).expect("the graph is well formed")),
            ["first", "second", "third"],
        );
    }

    #[test]
    fn planning_is_deterministic_across_repeated_calls() {
        let modules = vec![
            module("d", &["b"]),
            module("a", &[]),
            module("c", &["a"]),
            module("b", &["a"]),
        ];

        let first = plan(&modules).expect("the graph is well formed");
        let second = plan(&modules).expect("the graph is well formed");

        assert_eq!(first, second);
        assert_eq!(keys(&modules, first), ["a", "c", "b", "d"]);
    }

    #[test]
    fn invalid_and_duplicate_keys_are_reported_in_declaration_order() {
        let modules = vec![
            module("", &[]),
            module("http client", &[]),
            module("panels", &[]),
            module("panels", &[]),
        ];

        assert_eq!(
            errors(&modules),
            vec![
                "invalid setup key: `` is empty".to_string(),
                "invalid setup key: `http client` must contain only ASCII letters, digits, `.`, \
                 `-`, or `_`"
                    .to_string(),
                "setup key `panels` is declared more than once".to_string(),
            ],
            "only the surplus duplicate is blamed",
        );
    }

    #[test]
    fn missing_dependencies_are_reported_per_module_and_per_dependency() {
        let modules = vec![
            module("panels", &["http", "state"]),
            module("state", &[]),
            module("logs", &["http"]),
        ];

        assert_eq!(
            errors(&modules),
            vec![
                "setup module `panels` depends on `http`, which is not declared".to_string(),
                "setup module `logs` depends on `http`, which is not declared".to_string(),
            ],
        );
    }

    #[test]
    fn a_repeated_dependency_is_idempotent_and_keeps_its_first_position() {
        let declared = SetupModule::new(SetupKey::new("panels"), init_unit)
            .after(SetupKey::new("http"))
            .after(SetupKey::new("state"))
            .after(SetupKey::new("http"));

        assert_eq!(
            DeclaredSetupModule::erase(declared).dependencies(),
            [SetupKey::new("http"), SetupKey::new("state")],
            "the repeat is dropped and the first mention keeps its position",
        );
    }

    #[test]
    fn a_repeated_missing_dependency_is_reported_once() {
        let declared = SetupModule::new(SetupKey::new("panels"), init_unit)
            .after(SetupKey::new("http"))
            .after(SetupKey::new("http"));
        let modules = vec![DeclaredSetupModule::erase(declared)];

        assert_eq!(
            errors(&modules),
            vec!["setup module `panels` depends on `http`, which is not declared".to_string()],
            "one undeclared dependency is one mistake, however often it is named",
        );
    }

    #[test]
    fn a_repeated_dependency_does_not_disturb_the_resolved_order() {
        let repeated = vec![
            {
                let declared = SetupModule::new(SetupKey::new("panels"), init_unit)
                    .after(SetupKey::new("http"))
                    .after(SetupKey::new("http"));
                DeclaredSetupModule::erase(declared)
            },
            module("http", &[]),
            module("logs", &[]),
        ];
        let plain = vec![
            module("panels", &["http"]),
            module("http", &[]),
            module("logs", &[]),
        ];

        assert_eq!(
            keys(
                &repeated,
                plan(&repeated).expect("the graph is well formed")
            ),
            keys(&plain, plan(&plain).expect("the graph is well formed")),
            "repeating a dependency must not change the resolved order",
        );
    }

    #[test]
    fn a_second_shutdown_hook_is_reported_and_never_replaces_the_first() {
        fn later(_: (), _: &mut SetupContext<'_>) -> anyhow::Result<()> {
            unreachable!("the second teardown must never be retained")
        }

        let declared = DeclaredSetupModule::erase(
            SetupModule::new(SetupKey::new("theme"), init_unit)
                .shutdown(teardown_unit)
                .shutdown(later),
        );
        let modules = vec![declared];

        assert_eq!(
            errors(&modules),
            vec![
                "only one shutdown hook may be declared; setup module `theme` declares a second \
                 one"
                .to_string()
            ],
        );
        assert!(
            modules[0].has_teardown(),
            "the first teardown is still the one retained",
        );
    }

    #[test]
    fn every_surplus_shutdown_hook_is_reported_in_declaration_order() {
        fn later(_: (), _: &mut SetupContext<'_>) -> anyhow::Result<()> {
            unreachable!("the surplus teardowns must never be retained")
        }

        let modules = vec![
            DeclaredSetupModule::erase(
                SetupModule::new(SetupKey::new("first"), init_unit)
                    .shutdown(teardown_unit)
                    .shutdown(later)
                    .shutdown(later),
            ),
            DeclaredSetupModule::erase(
                SetupModule::new(SetupKey::new("second"), init_unit)
                    .shutdown(teardown_unit)
                    .shutdown(later),
            ),
        ];

        assert_eq!(
            errors(&modules),
            vec![
                "only one shutdown hook may be declared; setup module `first` declares a second \
                 one"
                .to_string(),
                "only one shutdown hook may be declared; setup module `first` declares a second \
                 one"
                .to_string(),
                "only one shutdown hook may be declared; setup module `second` declares a second \
                 one"
                .to_string(),
            ],
            "one fault per surplus hook, module by module, in declaration order",
        );
    }

    #[test]
    fn one_shutdown_hook_is_not_a_fault() {
        let modules = vec![DeclaredSetupModule::erase(
            SetupModule::new(SetupKey::new("theme"), init_unit).shutdown(teardown_unit),
        )];

        assert!(plan(&modules).is_ok());
    }

    #[test]
    fn a_cycle_names_every_unresolvable_module_in_declaration_order() {
        let modules = vec![
            module("a", &["b"]),
            module("standalone", &[]),
            module("b", &["a"]),
        ];

        assert_eq!(
            errors(&modules),
            vec![
                "setup module `a` is part of a dependency cycle".to_string(),
                "setup module `b` is part of a dependency cycle".to_string(),
            ],
            "modules outside the cycle still resolve and are not blamed",
        );
    }

    #[test]
    fn a_self_dependency_is_a_cycle() {
        let modules = vec![module("a", &["a"])];

        assert_eq!(
            errors(&modules),
            vec!["setup module `a` is part of a dependency cycle".to_string()],
        );
    }

    #[test]
    fn a_malformed_graph_is_not_topologically_ordered() {
        // A duplicate key makes every dependency reference ambiguous, so stage 1
        // must stop before cycle detection rather than guess.
        let modules = vec![module("a", &[]), module("a", &["missing"])];

        assert_eq!(
            errors(&modules),
            vec![
                "setup key `a` is declared more than once".to_string(),
                "setup module `a` depends on `missing`, which is not declared".to_string(),
            ],
        );
    }

    #[test]
    fn erasure_retains_the_state_type_and_teardown_presence() {
        let registered = DeclaredSetupModule::erase(
            SetupModule::new(SetupKey::new("panels"), init_unit).shutdown(teardown_unit),
        );
        let bare = DeclaredSetupModule::erase(SetupModule::new(SetupKey::new("http"), init_unit));

        assert!(registered.has_teardown());
        assert!(!bare.has_teardown());
        assert_eq!(registered.state_name(), type_name::<()>());
    }
}
