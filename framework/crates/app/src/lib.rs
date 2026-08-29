//! Application shell for neutron-components apps.
//!
//! Applications declare themselves as a type implementing [`DesktopApp`],
//! building an opaque [`AppDeclaration`] from [`AppDeclaration::new`], and run
//! it with [`AppShell::run`]. Lifecycle is an event stream ([`AppEvent`]) with
//! early platform-listener registration and queue-until-ready delivery;
//! liveness is hold/release leases ([`ShellHold`]); thread affinity is
//! explicit ([`AppInfo`]/[`AppProxy`] are `Send + Sync`, main-thread state is
//! reached through the [`Shell`] runtime trait).
//!
//! See `docs/learned/app-platform-plan.md` §3 for the reviewed contracts.

mod capabilities;
pub mod commands;
mod declaration;
mod error;
mod handles;
mod lifecycle;
mod liveness;
mod module;
mod runtime;
mod settings;
mod setup;
mod shell;
mod theme;
mod windows;

// Framework re-exports so app crates depend on one thing.
pub use gpui;
pub use gpui_platform;
pub use neutron_components as ui;

// Identity: the `include_identity!()` macro and its schema types.
pub use neutron_components_manifest::include_identity;
pub use neutron_components_manifest::schema::{AppIdentity, IdentityRef};

// Storage types used directly in the shell API.
pub use neutron_components_storage::{AppPaths, PathLayout};

// Declaration and runtime value types.
pub use capabilities::{
    Capability, PlatformCapabilities, PlatformCapability, UnsupportedCapability,
};
pub use commands::standard::DesktopPlatform;
pub use commands::{
    Command, CommandBinding, CommandError, CommandFault, CommandId, CommandLabel, Commands, Menu,
    MenuBar, MenuKey, MenuLabel, MenuNode, MenuOutline, MenuOutlineEntry, MenuSectionKey,
};
pub use declaration::{
    AdvancedHooks, AppDeclaration, DeclarationError, DeclarationErrors, DesktopApp, LaunchDecision,
    LaunchSpec, ProcessLaunch, SetupKey, SetupModule, Surface, SurfaceKey,
};
pub use error::{AppClosed, AppShellError, RuntimeError, RuntimeOperation};
pub use handles::{AppInfo, AppProxy};
pub use lifecycle::{AppEvent, OpenRequest, ShutdownReason};
pub use liveness::{ExitPolicy, InitialActivation, ShellHold};
pub use runtime::Shell;
pub use settings::{
    AppSettings, FutureVersionPolicy, Settings, SettingsError, ShellPreferences, StoreKey,
    ThemeMode, shell_preferences, update_shell_preferences,
};
pub use setup::SetupContext;
pub use shell::{AppShell, EnvironmentPolicy, LoggingPolicy};
pub use theme::{
    SwitchTheme, SwitchThemeMode, ThemeAsset, ThemeAssetSource, ThemeMenuGroup, ThemeMenuItem,
    ThemeSelection, ThemeSource, on_theme_registry_changed, theme_menu_items,
};
pub use windows::{
    OverlaySpec, RawWindow, SurfaceHandle, SurfaceOpen, WindowError, WindowKey, WindowSize,
};

/// Headless test entry points, exercising the same declared execution path as
/// [`AppShell::run`].
#[cfg(feature = "test-support")]
pub mod testing {
    pub use crate::declaration::run::testing::{run, run_with};
}

/// Common imports for application entry points: `use neutron_components_app::prelude::*;`.
pub mod prelude {
    pub use crate::commands::{Command, Commands, MenuBar};
    pub use crate::error::{AppClosed, AppShellError, RuntimeError, RuntimeOperation};
    pub use crate::handles::{AppInfo, AppProxy};
    pub use crate::lifecycle::{AppEvent, OpenRequest, ShutdownReason};
    pub use crate::liveness::{ExitPolicy, InitialActivation, ShellHold};
    pub use crate::runtime::Shell;
    pub use crate::settings::{AppSettings, Settings, StoreKey};
    pub use crate::shell::{AppShell, EnvironmentPolicy, LoggingPolicy};
    pub use crate::theme::ThemeSource;
    pub use crate::{DesktopApp, IdentityRef, PathLayout};
    pub use gpui::App;
}
