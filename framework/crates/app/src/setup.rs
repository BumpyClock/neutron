//! The private setup runtime: the opaque context handed to application setup
//! modules, and the single runtime module that runs them.
//!
//! One runtime module, not one per declared setup module: the setup pipeline is
//! a single participant in the fatal-init rollback and exactly-once reverse
//! teardown, so application setup can never observe runtime-module ordering or
//! lifecycle events. [`crate::declaration::AppDeclaration::lower`] appends it
//! after every other runtime module, which is what makes application setup
//! initialize last and tear down first.

use std::any::Any;

use gpui::App;

use crate::declaration::DeclaredSetupModule;
#[cfg(test)]
use crate::declaration::SetupKey;
use crate::error::{AppShellError, RuntimeError};
use crate::handles::{self, AppInfo, AppProxy};
use crate::module::RuntimeModule;

/// What an application setup module may reach.
///
/// Deliberately narrow: no startup state, no runtime-module storage, no manager
/// globals, no launch input, and no teardown scheduler. [`SetupContext::app`]
/// is the explicit escape for existing GPUI registration functions, and it
/// stays visible at every use site.
pub struct SetupContext<'a> {
    info: &'a AppInfo,
    proxy: &'a AppProxy,
    app: &'a mut App,
}

impl<'a> SetupContext<'a> {
    /// Build a context for one init or teardown call.
    pub(crate) fn new(app: &'a mut App, info: &'a AppInfo, proxy: &'a AppProxy) -> Self {
        Self { info, proxy, app }
    }

    /// The resolved application identity, paths, and capabilities.
    pub fn app_info(&self) -> &AppInfo {
        self.info
    }

    /// A cross-thread dispatch proxy.
    pub fn app_proxy(&self) -> AppProxy {
        self.proxy.clone()
    }

    /// The GPUI application. The explicit escape for global installation,
    /// observers, and factory registration.
    pub fn app(&mut self) -> &mut App {
        self.app
    }
}

/// Runs every declared application setup module, in resolved order.
pub(crate) struct SetupPipelineModule {
    /// Declared modules, already in resolved topological order.
    modules: Vec<DeclaredSetupModule>,
    /// `(index into `modules`, retained state)` for every module that
    /// initialized successfully. A failing module is never recorded here, so it
    /// can never be torn down.
    completed: Vec<(usize, Box<dyn Any>)>,
    /// Captured at `init` so teardown can rebuild a [`SetupContext`] without the
    /// app info and proxy, which `shutdown` is not given.
    shell: Option<(AppInfo, AppProxy)>,
}

impl SetupPipelineModule {
    /// Wrap the resolved module order.
    pub(crate) fn new(modules: Vec<DeclaredSetupModule>) -> Self {
        Self {
            modules,
            completed: Vec::new(),
            shell: None,
        }
    }

    /// Tear the completed prefix down in exact reverse resolved order.
    ///
    /// A teardown failure is nonfatal by definition: it is reported as
    /// [`RuntimeError::module`] and the remaining modules are still torn down.
    /// Idempotent — `completed` is drained, so a rollback during `init` cannot
    /// be repeated by a later `shutdown`.
    fn teardown_completed(&mut self, cx: &mut App) {
        let Some((info, proxy)) = self.shell.clone() else {
            return;
        };
        for (index, state) in std::mem::take(&mut self.completed).into_iter().rev() {
            let outcome = {
                let module = &self.modules[index];
                let mut context = SetupContext::new(cx, &info, &proxy);
                module
                    .teardown(state, &mut context)
                    .map_err(|source| (module.key(), source))
            };
            if let Err((key, source)) = outcome {
                handles::report_error(cx, RuntimeError::module(key.as_str(), source));
            }
        }
    }
}

impl RuntimeModule for SetupPipelineModule {
    fn id(&self) -> &'static str {
        "setup"
    }

    fn init(
        &mut self,
        cx: &mut App,
        info: &AppInfo,
        proxy: &AppProxy,
    ) -> Result<(), AppShellError> {
        let info = info.clone();
        let proxy = proxy.clone();
        self.shell = Some((info.clone(), proxy.clone()));

        for index in 0..self.modules.len() {
            let outcome = {
                let module = &self.modules[index];
                let mut context = SetupContext::new(cx, &info, &proxy);
                module
                    .init(&mut context)
                    .map_err(|source| (module.key(), source))
            };
            match outcome {
                Ok(state) => self.completed.push((index, state)),
                Err((key, source)) => {
                    // The shell does not call `shutdown` on a module whose
                    // `init` failed, so the pipeline unwinds its own completed
                    // prefix here before reporting the fatal fault.
                    self.teardown_completed(cx);
                    return Err(AppShellError::Module {
                        module: key.as_str(),
                        source,
                    });
                }
            }
        }
        Ok(())
    }

    fn shutdown(&mut self, cx: &mut App) {
        self.teardown_completed(cx);
    }
}

/// The keys of the modules that initialized successfully, in resolved order.
///
/// Test-only view of otherwise private retained state.
#[cfg(test)]
impl SetupPipelineModule {
    fn completed_keys(&self) -> Vec<SetupKey> {
        self.completed
            .iter()
            .map(|(index, _)| self.modules[*index].key())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::sync::{Arc, Mutex};

    use neutron_components_storage::{AppPaths, PathLayout};

    use crate::capabilities::PlatformCapabilities;
    use crate::declaration::{SetupKey, SetupModule, tests::identity};
    use crate::error::{RuntimeError, RuntimeOperation};
    use crate::handles::PendingEvents;
    use crate::liveness::{ExitPolicy, InitialActivation, Liveness};

    use super::*;

    // Setup hooks are non-capturing `fn` pointers, so the recording log has to
    // be reachable without capture. Thread-local, so parallel tests never share
    // it.
    thread_local! {
        static LOG: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    }

    fn record(entry: &str) {
        LOG.with_borrow_mut(|log| log.push(entry.to_string()));
    }

    fn take_log() -> Vec<String> {
        LOG.with_borrow_mut(std::mem::take)
    }

    /// State that records its own drop, proving the pipeline really retains it
    /// until reverse teardown rather than dropping it at init.
    struct Handle(&'static str);

    impl Drop for Handle {
        fn drop(&mut self) {
            record(&format!("{}:drop", self.0));
        }
    }

    fn init_first(cx: &mut SetupContext<'_>) -> anyhow::Result<Handle> {
        record(&format!("first:init:{}", cx.app_info().app_id()));
        Ok(Handle("first"))
    }

    fn teardown_first(state: Handle, _: &mut SetupContext<'_>) -> anyhow::Result<()> {
        record(&format!("first:teardown:{}", state.0));
        Ok(())
    }

    fn init_second(cx: &mut SetupContext<'_>) -> anyhow::Result<()> {
        // The explicit escape: a real module would register a global here.
        let _ = cx.app();
        record("second:init");
        Ok(())
    }

    fn teardown_second(_: (), _: &mut SetupContext<'_>) -> anyhow::Result<()> {
        record("second:teardown");
        Ok(())
    }

    fn teardown_failing(_: (), _: &mut SetupContext<'_>) -> anyhow::Result<()> {
        record("failing-teardown");
        anyhow::bail!("teardown failed")
    }

    fn init_broken(_: &mut SetupContext<'_>) -> anyhow::Result<()> {
        record("broken:init");
        anyhow::bail!("broken module")
    }

    /// Declared so that tearing the failing module down would be *visible*.
    /// It must never run: a module whose init failed owns no state.
    fn teardown_broken(_: (), _: &mut SetupContext<'_>) -> anyhow::Result<()> {
        record("broken:teardown");
        Ok(())
    }

    fn init_never(_: &mut SetupContext<'_>) -> anyhow::Result<()> {
        record("never:init");
        Ok(())
    }

    fn erase<State: 'static>(module: SetupModule<State>) -> DeclaredSetupModule {
        DeclaredSetupModule::erase(module)
    }

    /// Install the shell global and return what a module's `init` receives.
    fn services(cx: &mut App, errors: Arc<Mutex<Vec<String>>>) -> (AppInfo, AppProxy) {
        let info = AppInfo::new(
            identity(),
            AppPaths::new("appshell-setup-tests", PathLayout::PlatformDefault)
                .expect("test paths resolve"),
            PlatformCapabilities::detect(),
        );
        let proxy = handles::install(
            cx,
            info.clone(),
            Liveness::new(ExitPolicy::Explicit, InitialActivation::Passive),
            Vec::new(),
            Vec::new(),
            Arc::new(PendingEvents::default()),
            Box::new(move |error: &RuntimeError, _: &mut App| {
                assert_eq!(error.operation(), RuntimeOperation::Module);
                errors.lock().expect("errors poisoned").push(format!(
                    "{}:{}",
                    error.module_id().expect("a module error names its module"),
                    error.source_error(),
                ));
            }),
            None,
        );
        (info, proxy)
    }

    #[gpui::test]
    fn modules_initialize_in_resolved_order_and_tear_down_in_reverse(
        cx: &mut gpui::TestAppContext,
    ) {
        take_log();
        let mut pipeline = SetupPipelineModule::new(vec![
            erase(SetupModule::new(SetupKey::new("first"), init_first).shutdown(teardown_first)),
            erase(SetupModule::new(SetupKey::new("second"), init_second).shutdown(teardown_second)),
        ]);

        cx.update(|app| {
            let (info, proxy) = services(app, Arc::new(Mutex::new(Vec::new())));
            pipeline
                .init(app, &info, &proxy)
                .expect("both modules initialize");
            assert_eq!(
                pipeline.completed_keys(),
                vec![SetupKey::new("first"), SetupKey::new("second")],
                "state is retained privately, in resolved order",
            );
            assert_eq!(
                take_log(),
                vec![
                    format!("first:init:{}", identity().app_id),
                    "second:init".to_string(),
                ],
                "no state is dropped while the pipeline is live",
            );

            pipeline.shutdown(app);
        });

        assert_eq!(
            take_log(),
            vec![
                "second:teardown".to_string(),
                "first:teardown:first".to_string(),
                "first:drop".to_string(),
            ],
            "teardown runs in exact reverse resolved order and consumes the state",
        );
    }

    #[gpui::test]
    fn teardown_is_exactly_once_across_rollback_and_shutdown(cx: &mut gpui::TestAppContext) {
        take_log();
        let mut pipeline = SetupPipelineModule::new(vec![erase(
            SetupModule::new(SetupKey::new("second"), init_second).shutdown(teardown_second),
        )]);

        cx.update(|app| {
            let (info, proxy) = services(app, Arc::new(Mutex::new(Vec::new())));
            pipeline
                .init(app, &info, &proxy)
                .expect("the module initializes");
            pipeline.shutdown(app);
            pipeline.shutdown(app);
        });

        assert_eq!(
            take_log(),
            vec!["second:init".to_string(), "second:teardown".to_string()],
            "a repeated shutdown must not tear a module down twice",
        );
    }

    #[gpui::test]
    fn a_failing_module_is_fatal_and_rolls_back_only_the_completed_prefix(
        cx: &mut gpui::TestAppContext,
    ) {
        take_log();
        let mut pipeline = SetupPipelineModule::new(vec![
            erase(SetupModule::new(SetupKey::new("first"), init_first).shutdown(teardown_first)),
            erase(SetupModule::new(SetupKey::new("broken"), init_broken).shutdown(teardown_broken)),
            erase(SetupModule::new(SetupKey::new("never"), init_never)),
        ]);

        let error = cx.update(|app| {
            let (info, proxy) = services(app, Arc::new(Mutex::new(Vec::new())));
            pipeline
                .init(app, &info, &proxy)
                .expect_err("the broken module aborts startup")
        });

        assert!(
            matches!(&error, AppShellError::Module { module, .. } if *module == "broken"),
            "a setup failure is attributed to its own key: {error}",
        );
        assert_eq!(
            error.to_string(),
            "module `broken` failed",
            "the key reaches the message, not just the payload",
        );
        assert_eq!(
            take_log(),
            vec![
                format!("first:init:{}", identity().app_id),
                "broken:init".to_string(),
                "first:teardown:first".to_string(),
                "first:drop".to_string(),
            ],
            "later modules never initialize and the failing module never tears down",
        );
        assert!(
            pipeline.completed_keys().is_empty(),
            "a rolled-back pipeline retains no teardown state",
        );
    }

    #[gpui::test]
    fn a_teardown_failure_is_nonfatal_and_the_remaining_modules_still_run(
        cx: &mut gpui::TestAppContext,
    ) {
        take_log();
        let reported = Arc::new(Mutex::new(Vec::new()));
        let mut pipeline = SetupPipelineModule::new(vec![
            erase(SetupModule::new(SetupKey::new("first"), init_first).shutdown(teardown_first)),
            erase(
                SetupModule::new(SetupKey::new("failing"), init_second).shutdown(teardown_failing),
            ),
        ]);

        cx.update(|app| {
            let (info, proxy) = services(app, Arc::clone(&reported));
            pipeline
                .init(app, &info, &proxy)
                .expect("both modules initialize");
            take_log();
            pipeline.shutdown(app);
        });

        assert_eq!(
            take_log(),
            vec![
                "failing-teardown".to_string(),
                "first:teardown:first".to_string(),
                "first:drop".to_string(),
            ],
            "a teardown failure must not stop the remaining reverse teardown",
        );
        assert_eq!(
            *reported.lock().expect("errors poisoned"),
            vec!["failing:teardown failed".to_string()],
            "the failure is reported as a nonfatal module runtime error",
        );
    }
}
