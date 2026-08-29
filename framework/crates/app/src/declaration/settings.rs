//! The declared desktop conventions: application settings stores, the About
//! policy, and the theme policy — with the runtime modules each one installs.
//!
//! These three share a shape. Each is a *convention* the framework resolves for
//! every application — a store list that is empty by default, an About surface
//! the framework provides, a theme source the framework picks — and each is
//! declared by narrow methods on [`super::AppDeclaration`] rather than by
//! wiring runtime modules directly.
//!
//! The two policies are modelled as intents rather than as last-wins fields.
//! The first explicit intent is authoritative and every later one is a fault, so
//! a declaration that contradicts itself is reported instead of silently
//! resolving to whichever call happened to come last.
//!
//! The theme convention is also where the otherwise decoupled service modules
//! meet: theme selection persists into the platform-owned
//! [`ShellPreferences`](crate::settings::ShellPreferences) store, and the theme
//! menu items project into the command registry's reserved section. Both seams
//! are tied here, in the one owner of that convention, rather than by either
//! service importing the other.

use std::marker::PhantomData;

use gpui::{App, MenuItem};
use neutron_components::ThemeModePreference;

use crate::commands::{AppCommandsExt as _, THEME_SECTION, menus_invalidate};
use crate::error::AppShellError;
use crate::handles::{AppInfo, AppProxy};
use crate::module::{RuntimeModule, RuntimeModules};
use crate::settings::{
    AppSettings, SHELL_PREFERENCES_KEY, SettingsModule, ShellPreferencesModule, StoreKey,
    ThemeMode, shell_preferences, update_shell_preferences,
};
use crate::theme::{ThemeMenuGroup, ThemeModule, ThemeSelection, ThemeSource, theme_menu_items};

use super::errors::DeclarationError;
use super::surface::{ABOUT_SURFACE_ID, DeclaredSurface, SurfaceRole};

/// One declared application settings store, erased.
///
/// Erasure keeps [`super::AppDeclaration`] non-generic while the schema type
/// survives to lowering, where it selects the typed plugin. Nothing about
/// migration, validation, or future-version policy appears here: those live on
/// the [`AppSettings`] implementation, which is the single place an application
/// describes its schema.
pub(crate) trait ErasedSettingsStore: 'static {
    /// The declared store key, for cross-store validation.
    fn key(&self) -> &StoreKey;

    /// Append the typed settings runtime module for this store.
    fn install(self: Box<Self>, modules: &mut RuntimeModules);
}

/// A typed settings store declaration.
pub(crate) struct DeclaredSettingsStore<T: AppSettings> {
    key: StoreKey,
    /// Carries `T` without owning a value: the declaration is pure data and the
    /// schema type only ever appears in the lowered plugin.
    schema: PhantomData<fn() -> T>,
}

impl<T: AppSettings> DeclaredSettingsStore<T> {
    pub(crate) fn new(key: StoreKey) -> Self {
        Self {
            key,
            schema: PhantomData,
        }
    }
}

impl<T: AppSettings> ErasedSettingsStore for DeclaredSettingsStore<T> {
    fn key(&self) -> &StoreKey {
        &self.key
    }

    fn install(self: Box<Self>, modules: &mut RuntimeModules) {
        modules.push(Box::new(SettingsModule::<T>::new(self.key)));
    }
}

/// Cross-store faults: a repeated key and the framework's reserved key.
///
/// Both are collisions on the *file*, not on the Rust type, because a store key
/// names a file. Two schemas sharing one key would overwrite each other, and an
/// application key equal to the shell's preferences key would fight the
/// framework for the same file.
///
/// Reported in declaration order and only for the surplus declaration, so the
/// first store of a colliding pair is never blamed.
pub(crate) fn validate_settings_stores(
    stores: &[Box<dyn ErasedSettingsStore>],
    errors: &mut Vec<DeclarationError>,
) {
    let mut seen: Vec<&str> = Vec::new();
    for store in stores {
        let key = store.key().as_str();
        if key == SHELL_PREFERENCES_KEY {
            errors.push(DeclarationError::ReservedSettingsStoreKey {
                key: key.to_string(),
            });
            continue;
        }
        if seen.contains(&key) {
            errors.push(DeclarationError::DuplicateSettingsStoreKey {
                key: key.to_string(),
            });
        } else {
            seen.push(key);
        }
    }
}

/// The resolved About policy.
enum AboutPolicy {
    /// The framework's own About surface.
    Framework,
    /// An application surface replacing the framework's content, with the
    /// typed opener its standard command needs.
    Custom(DeclaredSurface, StandardOpener),
    /// No About surface and no About command.
    Disabled,
}

/// An opener for one standard surface, monomorphized at the declaration site.
///
/// A plain function pointer rather than a boxed closure: the standard command
/// handlers need `Copy` openers they can hand to GPUI, and the view type is
/// already erased by monomorphization.
pub(crate) type StandardOpener = fn(&mut gpui::App) -> anyhow::Result<()>;

/// The declared About intent.
///
/// Defaults to the framework surface. The first explicit intent replaces it and
/// every later one is counted as a fault, so the resolution never depends on
/// declaration order among contradicting calls.
pub(crate) struct AboutIntent {
    policy: AboutPolicy,
    /// Set by the first explicit intent, so a later one is surplus rather than
    /// an override of the default.
    explicit: bool,
    /// Explicit intents past the first. Their surfaces are dropped rather than
    /// indexed: the About ID is framework-reserved, so indexing them would
    /// bury the real fault under a duplicate-ID fault the application cannot
    /// act on.
    surplus: usize,
}

impl AboutIntent {
    /// The convention: the framework's own About surface.
    pub(crate) fn new() -> Self {
        Self {
            policy: AboutPolicy::Framework,
            explicit: false,
            surplus: 0,
        }
    }

    /// Replace the framework content with an application surface.
    pub(crate) fn custom(&mut self, surface: DeclaredSurface, opener: StandardOpener) {
        if self.explicit {
            self.surplus += 1;
            return;
        }
        self.policy = AboutPolicy::Custom(surface, opener);
        self.explicit = true;
    }

    /// Drop the About surface and its command entirely.
    pub(crate) fn disable(&mut self) {
        if self.explicit {
            self.surplus += 1;
            return;
        }
        self.policy = AboutPolicy::Disabled;
        self.explicit = true;
    }

    /// The opener the standard About command routes to, or `None` when About is
    /// disabled.
    pub(crate) fn opener(&self) -> Option<StandardOpener> {
        match &self.policy {
            AboutPolicy::Framework => Some(crate::windows::default_about_opener()),
            AboutPolicy::Custom(_, opener) => Some(*opener),
            AboutPolicy::Disabled => None,
        }
    }

    /// The resolved surface-index entry, so the About surface takes part in
    /// duplicate-ID and role validation exactly like a declared one.
    pub(crate) fn surface_entry(&self) -> Option<(&'static str, SurfaceRole)> {
        match &self.policy {
            AboutPolicy::Framework => Some((ABOUT_SURFACE_ID, SurfaceRole::About)),
            AboutPolicy::Custom(surface, _) => Some((surface.id(), SurfaceRole::About)),
            AboutPolicy::Disabled => None,
        }
    }

    /// Per-surface faults for a custom About, plus one fault per surplus intent.
    ///
    /// Borrows: the erased surface hooks are not clonable, and validation must
    /// leave the declaration intact for lowering to consume.
    pub(crate) fn validate(&self, errors: &mut Vec<DeclarationError>) {
        if let AboutPolicy::Custom(surface, _) = &self.policy {
            surface.validate(errors);
        }
        for _ in 0..self.surplus {
            errors.push(DeclarationError::MultipleAboutDeclarations);
        }
    }

    /// The surface to install, materializing the framework default. Consuming:
    /// the erased hooks move into the surface module.
    pub(crate) fn into_surface(self) -> Option<DeclaredSurface> {
        match self.policy {
            AboutPolicy::Framework => Some(crate::windows::default_about_surface()),
            AboutPolicy::Custom(surface, _) => Some(surface),
            AboutPolicy::Disabled => None,
        }
    }
}

/// The resolved theme policy.
enum ThemePolicy {
    /// The framework's registry-backed source.
    Framework,
    /// An application-supplied source.
    Custom(ThemeSource),
    /// No theme convention: no shell preferences, no theme plugin, no
    /// Appearance section.
    Disabled,
}

/// The declared theme intent, resolved first-wins exactly like [`AboutIntent`].
pub(crate) struct ThemeIntent {
    policy: ThemePolicy,
    explicit: bool,
    surplus: usize,
}

impl ThemeIntent {
    /// The convention: the registry-backed theme source.
    pub(crate) fn new() -> Self {
        Self {
            policy: ThemePolicy::Framework,
            explicit: false,
            surplus: 0,
        }
    }

    /// Replace the framework source.
    pub(crate) fn custom(&mut self, source: ThemeSource) {
        if self.explicit {
            self.surplus += 1;
            return;
        }
        self.policy = ThemePolicy::Custom(source);
        self.explicit = true;
    }

    /// Drop the theme convention entirely.
    pub(crate) fn disable(&mut self) {
        if self.explicit {
            self.surplus += 1;
            return;
        }
        self.policy = ThemePolicy::Disabled;
        self.explicit = true;
    }

    /// Whether the Appearance section and its persistence are installed.
    pub(crate) fn enabled(&self) -> bool {
        !matches!(self.policy, ThemePolicy::Disabled)
    }

    /// One fault per surplus intent.
    pub(crate) fn validate(&self, errors: &mut Vec<DeclarationError>) {
        for _ in 0..self.surplus {
            errors.push(DeclarationError::MultipleThemeDeclarations);
        }
    }

    /// Append the whole theme convention, or nothing when it is disabled.
    ///
    /// All three runtime modules or none of them: the platform-owned shell
    /// preferences store the selection persists into, the theme module with
    /// that persistence mapping, and the Appearance menu bridge.
    pub(crate) fn install(self, modules: &mut RuntimeModules) {
        let source = match self.policy {
            ThemePolicy::Framework => ThemeSource::registry(),
            ThemePolicy::Custom(source) => source,
            ThemePolicy::Disabled => return,
        };
        modules.push(Box::new(ShellPreferencesModule::new()));
        modules.push(Box::new(
            ThemeModule::new(source)
                .with_preferences(read_theme_preference, write_theme_preference),
        ));
        modules.push(Box::new(ThemeMenuModule));
    }
}

/// Read the persisted theme selection out of the platform-owned store.
fn read_theme_preference(cx: &App) -> Option<ThemeSelection> {
    let prefs = shell_preferences(cx);
    Some(ThemeSelection {
        mode: match prefs.theme_mode {
            ThemeMode::System => ThemeModePreference::System,
            ThemeMode::Light => ThemeModePreference::Light,
            ThemeMode::Dark => ThemeModePreference::Dark,
        },
        name: prefs.theme_name,
    })
}

/// Write a theme selection back into the platform-owned store.
fn write_theme_preference(cx: &mut App, selection: &ThemeSelection) {
    let mode = match selection.mode {
        ThemeModePreference::System => ThemeMode::System,
        ThemeModePreference::Light => ThemeMode::Light,
        ThemeModePreference::Dark => ThemeMode::Dark,
    };
    let name = selection.name.clone();
    if let Err(e) = update_shell_preferences(cx, |prefs| {
        prefs.theme_mode = mode;
        prefs.theme_name = name;
    }) {
        crate::handles::report_error(
            cx,
            crate::error::RuntimeError::module("theme", anyhow::Error::new(e)),
        );
    }
}

/// Glue module: projects [`theme_menu_items`] into the command registry's
/// reserved theme section and invalidates menus on registry hot-reloads.
pub(crate) struct ThemeMenuModule;

impl RuntimeModule for ThemeMenuModule {
    fn id(&self) -> &'static str {
        "theme-menu"
    }

    fn init(
        &mut self,
        cx: &mut App,
        _info: &AppInfo,
        _proxy: &AppProxy,
    ) -> Result<(), AppShellError> {
        cx.register_menu_section(THEME_SECTION, theme_section_items);
        crate::theme::on_theme_registry_changed(cx, |cx| menus_invalidate(cx)).detach();
        Ok(())
    }
}

fn theme_section_items(cx: &App) -> Vec<MenuItem> {
    let mut items = Vec::new();
    let mut last_group: Option<ThemeMenuGroup> = None;
    for item in theme_menu_items(cx) {
        if last_group.is_some_and(|g| g != item.group) {
            items.push(MenuItem::Separator);
        }
        last_group = Some(item.group);
        items.push(MenuItem::Action {
            name: item.label.into(),
            action: item.action,
            os_action: None,
            checked: item.checked,
            disabled: false,
        });
    }
    items
}
