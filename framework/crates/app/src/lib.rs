//! Application shell for gpui-component apps.
//!
//! `AppShell` is a thin builder over a sealed plugin/phase mechanism. Builder
//! methods record intent; a central sequencer ([`phases`]) fixes the startup and
//! reverse-shutdown order. Lifecycle is an event stream ([`lifecycle::AppEvent`])
//! with early platform-listener registration and queue-until-ready delivery;
//! liveness is hold/release leases ([`liveness`]); thread affinity is explicit
//! ([`AppInfo`]/[`AppProxy`] are `Send + Sync`, main-thread state is a GPUI
//! global reached via [`AppShellExt`]).
//!
//! See `docs/learned/app-platform-plan.md` §3 for the reviewed contracts.

pub mod capabilities;
pub mod commands;
mod defaults;
pub mod error;
pub mod handles;
pub mod lifecycle;
pub mod liveness;
pub mod phases;
pub mod plugin;
pub mod settings;
pub mod shell;
pub mod theme;
pub mod windows;

// Framework re-exports so app crates depend on one thing.
pub use gpui;
pub use gpui_component as ui;
pub use gpui_platform;

// Identity: the `include_identity!()` macro and its schema types.
pub use gpui_component_manifest::include_identity;
pub use gpui_component_manifest::schema::{AppIdentity, IdentityRef};

// Storage types used directly in the shell API.
pub use gpui_component_storage::{AppPaths, PathLayout};

// Core surface.
pub use capabilities::{Capability, PlatformCapabilities};
pub use error::{
    AppClosed, AppShellError, BuilderConfigurationError, MenuConfiguration, RuntimeError,
    RuntimeOperation, StartupHook,
};
pub use handles::{AppInfo, AppProxy, AppShellExt, ShellHandle, ShellState};
pub use lifecycle::{AppEvent, LaunchRequest, OpenRequest, ShutdownReason};
pub use liveness::{ExitPolicy, InitialActivation, ShellHold};
pub use phases::Phase;
pub use plugin::{AppPlugin, BuildContext, ShellSeed};
pub use shell::{AppShell, AppShellBuilder, EnvironmentPolicy, LoggingPolicy, PlatformRunner};

// Service surface.
pub use commands::{
    ABOUT_COMMAND_ID, About, AppCommandsExt, AppMenusExt, Command, CommandError, CommandId,
    CommandRegistry, CommandScope, MenuPlacement, MenuPlan, MenuSection, MenusPlugin,
    OPEN_SETTINGS_COMMAND_ID, OpenSettings, QUIT_COMMAND_ID, StandardMenus, THEME_SECTION,
    menus_invalidate,
};
pub use settings::{
    AppSettings, FutureVersionPolicy, SettingsExt, SettingsPlugin, ShellPreferences,
    ShellPreferencesPlugin, StoreKey, ThemeMode, shell_preferences, update_shell_preferences,
};
pub use theme::{ThemePlugin, ThemeSelection, ThemeSource};
pub use windows::{
    AppWindowsExt, OpenedWindow, OverlaySpec, RootPolicy, Singleton, WindowError, WindowKey,
    WindowManager, WindowSpec, WindowsPlugin,
};

/// Common imports for application entry points: `use gpui_component_app::prelude::*;`.
pub mod prelude {
    pub use crate::commands::{AppCommandsExt, AppMenusExt, MenuPlan, StandardMenus};
    pub use crate::error::{AppClosed, AppShellError, RuntimeError, RuntimeOperation};
    pub use crate::handles::{AppInfo, AppProxy, AppShellExt, ShellHandle};
    pub use crate::lifecycle::{AppEvent, LaunchRequest, OpenRequest, ShutdownReason};
    pub use crate::liveness::{ExitPolicy, InitialActivation, ShellHold};
    pub use crate::settings::{AppSettings, SettingsExt, StoreKey};
    pub use crate::shell::{AppShell, EnvironmentPolicy, LoggingPolicy, PlatformRunner};
    pub use crate::theme::ThemeSource;
    pub use crate::windows::{
        AppWindowsExt, RootPolicy, Singleton, WindowError, WindowKey, WindowManager, WindowSpec,
    };
    pub use crate::{IdentityRef, PathLayout};
    pub use gpui::App;
}
