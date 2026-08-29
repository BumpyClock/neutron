//! Typed declared surfaces: stable keys, the focused surface builder, and the
//! erasure that carries a complete typed surface into the declaration module
//! list.
//!
//! A *surface* is a normal managed window: `AppShell` wraps its content in
//! `neutron_components::Root`, numbers and titles it, registers it, and takes a
//! liveness hold. Raw windows and platform overlays are runtime escapes, not
//! declarable surfaces (see [`crate::windows::RawWindow`]).
//!
//! ## Typing
//!
//! [`SurfaceKey<View, Args>`] binds a stable ID to the content view type and the
//! open-argument type, so a declared surface can never be opened with the wrong
//! content or arguments. The typed [`Surface<View, Args>`] erases into a
//! [`DeclaredSurface`] only once it is complete, and the erased value keeps
//! `TypeId`/`type_name` metadata for diagnostics.
//!
//! ## Purity
//!
//! Everything here is pure: no GPUI globals, no filesystem, no host inspection.
//! Validation reports every independent fault in declaration order.

use std::any::{Any, TypeId};
use std::fmt;
use std::marker::PhantomData;

use gpui::{
    App, Entity, Pixels, Render, SharedString, Size, Window, WindowBackgroundAppearance,
    WindowDecorations, WindowOptions,
};

use crate::module::RuntimeModules;
use crate::windows::WindowSize;

use super::errors::DeclarationError;
use super::module::DeclarationModule;

/// Reserved ID of the standard primary surface.
pub(crate) const PRIMARY_SURFACE_ID: &str = "primary";
/// Reserved ID of the standard Settings surface.
pub(crate) const SETTINGS_SURFACE_ID: &str = "settings";
/// Reserved ID of the standard About surface.
pub(crate) const ABOUT_SURFACE_ID: &str = "about";

/// Every framework-reserved surface ID with the role that owns it.
const RESERVED_IDS: [(&str, SurfaceRole); 3] = [
    (PRIMARY_SURFACE_ID, SurfaceRole::Primary),
    (SETTINGS_SURFACE_ID, SurfaceRole::Settings),
    (ABOUT_SURFACE_ID, SurfaceRole::About),
];

/// The role a surface was declared in.
///
/// Roles come from the declaration method that accepted the surface
/// ([`super::AppDeclaration::primary_surface`] and friends), never from an
/// application-supplied value, so this type stays internal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SurfaceRole {
    /// The launch surface, restored on reopen. At most one per declaration.
    Primary,
    /// The standard Settings surface.
    Settings,
    /// The standard About surface.
    About,
    /// An application-declared auxiliary surface.
    Auxiliary,
}

impl SurfaceRole {
    /// Stable name used in diagnostics.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Primary => "primary",
            Self::Settings => "settings",
            Self::About => "about",
            Self::Auxiliary => "auxiliary",
        }
    }

    /// The reserved ID this role requires, if it is a standard role.
    const fn reserved_id(self) -> Option<&'static str> {
        match self {
            Self::Primary => Some(PRIMARY_SURFACE_ID),
            Self::Settings => Some(SETTINGS_SURFACE_ID),
            Self::About => Some(ABOUT_SURFACE_ID),
            Self::Auxiliary => None,
        }
    }

    /// Whether the role admits more than one live instance.
    const fn allows_multiple(self) -> bool {
        matches!(self, Self::Primary | Self::Auxiliary)
    }

    /// Whether in-window menu chrome is attached when the surface does not
    /// state a policy. Settings and About stay chrome-free by default.
    pub(crate) const fn default_menu_bar(self) -> bool {
        matches!(self, Self::Primary | Self::Auxiliary)
    }
}

impl fmt::Display for SurfaceRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A stable, typed surface identity.
///
/// Binds a compiled-in ID to the surface's content view type and open-argument
/// type. Copyable and comparable regardless of `View`/`Args`, because the type
/// parameters are carried in a `PhantomData` function marker rather than by
/// value.
pub struct SurfaceKey<View, Args = ()> {
    id: &'static str,
    marker: PhantomData<fn(&Args) -> View>,
}

impl<View, Args> SurfaceKey<View, Args> {
    /// A key for an application-chosen stable ID, e.g. `SurfaceKey::new("logs")`.
    ///
    /// The ID is checked by [`super::AppDeclaration::validate`], not here: the
    /// constructor stays `const` so keys can be associated constants.
    pub const fn new(id: &'static str) -> Self {
        Self {
            id,
            marker: PhantomData,
        }
    }

    /// The framework-reserved key for the primary surface.
    pub const fn primary() -> Self {
        Self::new(PRIMARY_SURFACE_ID)
    }

    /// The stable ID.
    pub const fn id(&self) -> &'static str {
        self.id
    }
}

impl<View> SurfaceKey<View, ()> {
    /// The framework-reserved key for the standard Settings surface.
    ///
    /// Only available with unit arguments: Settings is opened by the standard
    /// command contract, which carries no application payload.
    pub const fn settings() -> Self {
        Self::new(SETTINGS_SURFACE_ID)
    }

    /// The framework-reserved key for the standard About surface, with the same
    /// unit-argument rule as [`SurfaceKey::settings`].
    pub const fn about() -> Self {
        Self::new(ABOUT_SURFACE_ID)
    }
}

// Manual impls: deriving would demand `View: Clone`/`Args: Clone` etc., which a
// content view type has no reason to satisfy.
impl<View, Args> Clone for SurfaceKey<View, Args> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<View, Args> Copy for SurfaceKey<View, Args> {}

impl<View, Args> PartialEq for SurfaceKey<View, Args> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl<View, Args> Eq for SurfaceKey<View, Args> {}

impl<View, Args> fmt::Debug for SurfaceKey<View, Args> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("SurfaceKey").field(&self.id).finish()
    }
}

impl<View, Args> fmt::Display for SurfaceKey<View, Args> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id)
    }
}

/// How many live instances a surface admits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum SurfaceCardinality {
    /// One live window; a second open focuses and reuses it. The default.
    #[default]
    Single,
    /// Numbered instances, each open creating a new window.
    Multiple,
}

/// The non-generic window options of a declared surface.
#[derive(Debug, Clone)]
pub(crate) struct SurfaceOptions {
    /// Un-numbered base title; defaults to the app display name.
    pub(crate) title: Option<SharedString>,
    /// Initial size policy.
    pub(crate) size: WindowSize,
    /// Minimum window size, if constrained.
    pub(crate) min_size: Option<Size<Pixels>>,
    /// Window background appearance.
    pub(crate) background: WindowBackgroundAppearance,
    /// Client/server decoration request (Wayland-only, may be ignored).
    pub(crate) decorations: Option<WindowDecorations>,
    /// Explicit in-window menu-chrome policy; `None` means the role default.
    pub(crate) menu_bar: Option<bool>,
    /// Singleton (default) or numbered multiple instances.
    pub(crate) cardinality: SurfaceCardinality,
}

impl Default for SurfaceOptions {
    fn default() -> Self {
        Self {
            title: None,
            size: WindowSize::default(),
            min_size: None,
            background: WindowBackgroundAppearance::Opaque,
            decorations: None,
            menu_bar: None,
            cardinality: SurfaceCardinality::default(),
        }
    }
}

impl SurfaceOptions {
    /// Whether in-window menu chrome applies, given the declared role.
    pub(crate) fn menu_bar_for(&self, role: SurfaceRole) -> bool {
        self.menu_bar.unwrap_or_else(|| role.default_menu_bar())
    }
}

/// The typed hooks of a declared surface.
///
/// All are non-capturing `fn` pointers: a declaration is a pure value that the
/// shell may retain for the whole process lifetime, so it must not close over
/// application state that would need a lifetime, a lock, or a thread rule.
pub(crate) struct SurfaceHooks<View, Args> {
    /// Builds the content view. Borrows `Args` so the primary surface can be
    /// rebuilt from the shell's retained immutable launch value.
    pub(crate) build: fn(&Args, &mut Window, &mut App) -> Entity<View>,
    /// Applies later open arguments to an already-live singleton.
    pub(crate) on_reuse: Option<fn(&Entity<View>, &Args, &mut Window, &mut App)>,
    /// Escape hook for uncommon platform-specific window options.
    pub(crate) configure_window: Option<fn(&mut WindowOptions)>,
    /// Escape hook run against the opened window and its content.
    pub(crate) after_open: Option<fn(&Entity<View>, &mut Window, &mut App)>,
}

// Manual impls: `fn` pointers are always `Copy`, but deriving would require
// `View: Copy`/`Args: Copy`.
impl<View, Args> Clone for SurfaceHooks<View, Args> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<View, Args> Copy for SurfaceHooks<View, Args> {}

/// A declared normal surface: typed identity, focused window options, and
/// non-capturing hooks.
pub struct Surface<View, Args = ()> {
    key: SurfaceKey<View, Args>,
    options: SurfaceOptions,
    hooks: SurfaceHooks<View, Args>,
}

impl<View: 'static, Args: 'static> Surface<View, Args> {
    /// Declare a surface for `key`, built by `build`.
    #[must_use]
    pub fn new(
        key: SurfaceKey<View, Args>,
        build: fn(&Args, &mut Window, &mut App) -> Entity<View>,
    ) -> Self {
        Self {
            key,
            options: SurfaceOptions::default(),
            hooks: SurfaceHooks {
                build,
                on_reuse: None,
                configure_window: None,
                after_open: None,
            },
        }
    }

    /// Set the un-numbered base title (defaults to the app display name).
    #[must_use]
    pub fn title(mut self, title: impl Into<SharedString>) -> Self {
        self.options.title = Some(title.into());
        self
    }

    /// Set the initial size policy.
    #[must_use]
    pub fn size(mut self, size: WindowSize) -> Self {
        self.options.size = size;
        self
    }

    /// Constrain the window to a minimum logical size.
    #[must_use]
    pub fn min_size(mut self, size: Size<Pixels>) -> Self {
        self.options.min_size = Some(size);
        self
    }

    /// Set the window background appearance.
    #[must_use]
    pub fn background(mut self, background: WindowBackgroundAppearance) -> Self {
        self.options.background = background;
        self
    }

    /// Request client- or server-side decorations (Wayland; may be ignored).
    #[must_use]
    pub fn decorations(mut self, decorations: WindowDecorations) -> Self {
        self.options.decorations = Some(decorations);
        self
    }

    /// Override the role's in-window menu-chrome default. An empty menu
    /// projection still never produces an inert menu bar.
    #[must_use]
    pub fn menu_bar(mut self, enabled: bool) -> Self {
        self.options.menu_bar = Some(enabled);
        self
    }

    /// Allow numbered multiple instances instead of the singleton default.
    #[must_use]
    pub fn multiple(mut self) -> Self {
        self.options.cardinality = SurfaceCardinality::Multiple;
        self
    }

    /// Apply later open arguments to the already-live singleton instead of
    /// only focusing it.
    #[must_use]
    pub fn on_reuse(mut self, handler: fn(&Entity<View>, &Args, &mut Window, &mut App)) -> Self {
        self.hooks.on_reuse = Some(handler);
        self
    }

    /// Customize the resolved [`WindowOptions`] just before the window shows.
    #[must_use]
    pub fn configure_window(mut self, hook: fn(&mut WindowOptions)) -> Self {
        self.hooks.configure_window = Some(hook);
        self
    }

    /// Run a hook against the opened window and its content view.
    #[must_use]
    pub fn after_open(mut self, hook: fn(&Entity<View>, &mut Window, &mut App)) -> Self {
        self.hooks.after_open = Some(hook);
        self
    }

    /// The typed key.
    pub fn key(&self) -> SurfaceKey<View, Args> {
        self.key
    }
}

/// Content and argument type identities, retained after erasure for
/// diagnostics and for the typed downcast that recovers [`SurfaceHooks`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SurfaceTypes {
    /// `TypeId` of the content view.
    pub(crate) view: TypeId,
    /// `type_name` of the content view.
    pub(crate) view_name: &'static str,
    /// `TypeId` of the open arguments.
    pub(crate) args: TypeId,
    /// `type_name` of the open arguments.
    pub(crate) args_name: &'static str,
}

impl SurfaceTypes {
    fn of<View: 'static, Args: 'static>() -> Self {
        Self {
            view: TypeId::of::<View>(),
            view_name: std::any::type_name::<View>(),
            args: TypeId::of::<Args>(),
            args_name: std::any::type_name::<Args>(),
        }
    }
}

/// One complete typed surface, erased.
///
/// The window options and role are plain data; only the hooks stay typed, boxed
/// behind [`Any`] so the runtime can recover them with the same `View`/`Args`
/// the declaration used.
pub(crate) struct DeclaredSurface {
    id: &'static str,
    role: SurfaceRole,
    options: SurfaceOptions,
    types: SurfaceTypes,
    /// Whether the erased hooks include `on_reuse`, so validation can see it
    /// without knowing `View`/`Args`.
    has_on_reuse: bool,
    hooks: Box<dyn Any>,
}

impl DeclaredSurface {
    /// Erase a complete typed surface declared in `role`.
    pub(crate) fn erase<View: 'static + Render, Args: 'static>(
        surface: Surface<View, Args>,
        role: SurfaceRole,
    ) -> Self {
        let Surface {
            key,
            options,
            hooks,
        } = surface;
        Self {
            id: key.id(),
            role,
            options,
            types: SurfaceTypes::of::<View, Args>(),
            has_on_reuse: hooks.on_reuse.is_some(),
            hooks: Box::new(hooks),
        }
    }

    /// The stable surface ID.
    pub(crate) fn id(&self) -> &'static str {
        self.id
    }

    /// The declared role.
    pub(crate) fn role(&self) -> SurfaceRole {
        self.role
    }

    /// The declared window options.
    pub(crate) fn options(&self) -> &SurfaceOptions {
        &self.options
    }

    /// Retained content/argument type identities.
    pub(crate) fn types(&self) -> SurfaceTypes {
        self.types
    }

    /// Recover the typed hooks, or `None` when `View`/`Args` do not match the
    /// declaration.
    pub(crate) fn hooks<View: 'static, Args: 'static>(&self) -> Option<SurfaceHooks<View, Args>> {
        self.hooks
            .downcast_ref::<SurfaceHooks<View, Args>>()
            .copied()
    }

    /// This surface's own pure faults, in a fixed order.
    ///
    /// Cross-surface faults (duplicate IDs, more than one primary) belong to
    /// [`super::AppDeclaration::validate`], which is the only place that sees
    /// every declared surface.
    pub(crate) fn validate(&self, errors: &mut Vec<DeclarationError>) {
        if let Some(reason) = invalid_id_reason(self.id) {
            errors.push(DeclarationError::InvalidSurfaceId {
                id: self.id,
                reason,
            });
        }
        match self.role.reserved_id() {
            Some(expected) if expected != self.id => {
                errors.push(DeclarationError::SurfaceRoleId {
                    role: self.role.as_str(),
                    expected,
                    actual: self.id,
                });
            }
            None => {
                if let Some((_, owner)) = RESERVED_IDS.iter().find(|(id, _)| *id == self.id) {
                    errors.push(DeclarationError::ReservedSurfaceId {
                        id: self.id,
                        role: owner.as_str(),
                    });
                }
            }
            Some(_) => {}
        }
        if self.options.cardinality == SurfaceCardinality::Multiple {
            if !self.role.allows_multiple() {
                errors.push(DeclarationError::InvalidSurfaceCardinality {
                    id: self.id,
                    role: self.role.as_str(),
                });
            }
            // Every open of a multiple surface creates, so a reuse hook is dead
            // code the application would reasonably expect to run.
            if self.has_on_reuse {
                errors.push(DeclarationError::UnreachableSurfaceReuse { id: self.id });
            }
        }
    }
}

/// Why `id` is unusable as a stable surface ID, if it is.
fn invalid_id_reason(id: &str) -> Option<&'static str> {
    if id.is_empty() {
        return Some("is empty");
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
    {
        return Some("must contain only ASCII letters, digits, `.`, `-`, or `_`");
    }
    None
}

/// The erased declaration module holding one declared surface.
pub(crate) struct SurfaceModule {
    surface: DeclaredSurface,
}

impl SurfaceModule {
    /// Wrap an erased surface as a declaration module.
    pub(crate) fn new(surface: DeclaredSurface) -> Self {
        Self { surface }
    }
}

impl DeclarationModule for SurfaceModule {
    #[cfg(test)]
    fn key(&self) -> &'static str {
        "surface"
    }

    fn validate(&self, errors: &mut Vec<DeclarationError>) {
        self.surface.validate(errors);
    }

    fn install(self: Box<Self>, modules: &mut RuntimeModules) {
        modules.push(Box::new(crate::windows::declared_surface_module(
            self.surface,
        )));
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use gpui::{AppContext as _, Empty, px, size};

    use super::*;

    /// A second content type, so type-metadata tests are not self-satisfying.
    pub(crate) struct Other;

    impl Render for Other {
        fn render(
            &mut self,
            _: &mut Window,
            _: &mut gpui::Context<Self>,
        ) -> impl gpui::IntoElement {
            gpui::Empty
        }
    }

    pub(crate) fn build_empty(_: &(), _: &mut Window, cx: &mut App) -> Entity<Empty> {
        cx.new(|_| Empty)
    }

    fn faults(surface: &DeclaredSurface) -> Vec<String> {
        let mut errors = Vec::new();
        surface.validate(&mut errors);
        errors.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn standard_constructors_bind_the_reserved_ids() {
        assert_eq!(SurfaceKey::<Empty>::primary().id(), "primary");
        assert_eq!(SurfaceKey::<Empty>::settings().id(), "settings");
        assert_eq!(SurfaceKey::<Empty>::about().id(), "about");
        assert_eq!(SurfaceKey::<Empty>::new("logs").id(), "logs");
    }

    #[test]
    fn keys_compare_by_id_and_stay_copyable() {
        let key = SurfaceKey::<Empty>::new("logs");
        let copy = key;
        assert_eq!(key, copy);
        assert_ne!(key, SurfaceKey::<Empty>::new("other"));
        assert_eq!(key.to_string(), "logs");
        assert_eq!(format!("{key:?}"), "SurfaceKey(\"logs\")");
    }

    #[test]
    fn surfaces_are_singleton_with_role_menu_defaults_until_overridden() {
        let surface = Surface::new(SurfaceKey::<Empty>::new("logs"), build_empty);

        assert_eq!(surface.options.cardinality, SurfaceCardinality::Single);
        assert_eq!(surface.options.menu_bar, None);
        assert!(surface.options.menu_bar_for(SurfaceRole::Primary));
        assert!(surface.options.menu_bar_for(SurfaceRole::Auxiliary));
        assert!(!surface.options.menu_bar_for(SurfaceRole::Settings));
        assert!(!surface.options.menu_bar_for(SurfaceRole::About));

        let surface = surface.menu_bar(true);
        assert!(surface.options.menu_bar_for(SurfaceRole::Settings));
    }

    #[test]
    fn builder_records_focused_window_options_and_hooks() {
        let surface = Surface::new(SurfaceKey::<Empty>::new("logs"), build_empty)
            .title("Logs")
            .size(WindowSize::DisplayFraction(0.5))
            .min_size(size(px(320.), px(240.)))
            .background(WindowBackgroundAppearance::Transparent)
            .decorations(WindowDecorations::Client)
            .menu_bar(false)
            .multiple()
            .on_reuse(|_, _, _, _| {})
            .configure_window(|_| {})
            .after_open(|_, _, _| {});

        assert_eq!(surface.options.title.as_deref(), Some("Logs"));
        assert!(matches!(
            surface.options.size,
            WindowSize::DisplayFraction(f) if (f - 0.5).abs() < f32::EPSILON
        ));
        assert_eq!(surface.options.min_size, Some(size(px(320.), px(240.))));
        assert!(matches!(
            surface.options.background,
            WindowBackgroundAppearance::Transparent
        ));
        assert_eq!(surface.options.decorations, Some(WindowDecorations::Client));
        assert_eq!(surface.options.menu_bar, Some(false));
        assert_eq!(surface.options.cardinality, SurfaceCardinality::Multiple);
        assert!(surface.hooks.on_reuse.is_some());
        assert!(surface.hooks.configure_window.is_some());
        assert!(surface.hooks.after_open.is_some());
    }

    #[test]
    fn erasure_keeps_type_metadata_and_recovers_typed_hooks() {
        let declared = DeclaredSurface::erase(
            Surface::new(SurfaceKey::<Empty>::new("logs"), build_empty),
            SurfaceRole::Auxiliary,
        );

        assert_eq!(declared.id(), "logs");
        assert_eq!(declared.role(), SurfaceRole::Auxiliary);
        assert_eq!(declared.types().view, TypeId::of::<Empty>());
        assert_eq!(declared.types().view_name, std::any::type_name::<Empty>());
        assert_eq!(declared.types().args, TypeId::of::<()>());
        assert_eq!(declared.types().args_name, std::any::type_name::<()>());

        assert!(declared.hooks::<Empty, ()>().is_some());
        assert!(
            declared.hooks::<Other, ()>().is_none(),
            "a mismatched view type must not recover the hooks",
        );
        assert!(
            declared.hooks::<Empty, u32>().is_none(),
            "a mismatched argument type must not recover the hooks",
        );
    }

    #[test]
    fn a_well_formed_auxiliary_surface_has_no_faults() {
        let declared = DeclaredSurface::erase(
            Surface::new(SurfaceKey::<Empty>::new("logs"), build_empty).multiple(),
            SurfaceRole::Auxiliary,
        );

        assert!(faults(&declared).is_empty());
    }

    #[test]
    fn invalid_ids_are_reported_with_their_reason() {
        let empty = DeclaredSurface::erase(
            Surface::new(SurfaceKey::<Empty>::new(""), build_empty),
            SurfaceRole::Auxiliary,
        );
        assert_eq!(
            faults(&empty),
            vec!["invalid surface id: `` is empty".to_string()],
        );

        let spaced = DeclaredSurface::erase(
            Surface::new(SurfaceKey::<Empty>::new("my surface"), build_empty),
            SurfaceRole::Auxiliary,
        );
        assert_eq!(
            faults(&spaced),
            vec![
                "invalid surface id: `my surface` must contain only ASCII letters, digits, \
                 `.`, `-`, or `_`"
                    .to_string()
            ],
        );
    }

    #[test]
    fn an_auxiliary_surface_may_not_take_a_reserved_id() {
        let declared = DeclaredSurface::erase(
            Surface::new(SurfaceKey::<Empty>::new("settings"), build_empty),
            SurfaceRole::Auxiliary,
        );

        assert_eq!(
            faults(&declared),
            vec!["surface id `settings` is reserved for the standard settings surface".to_string()],
        );
    }

    #[test]
    fn a_standard_role_requires_its_reserved_id() {
        let declared = DeclaredSurface::erase(
            Surface::new(SurfaceKey::<Empty>::new("prefs"), build_empty),
            SurfaceRole::Settings,
        );

        assert_eq!(
            faults(&declared),
            vec![
                "the settings surface must use the reserved id `settings`, not `prefs`".to_string()
            ],
        );
    }

    #[test]
    fn settings_and_about_may_not_declare_multiple_instances() {
        for role in [SurfaceRole::Settings, SurfaceRole::About] {
            let id = role.reserved_id().expect("standard role");
            let declared = DeclaredSurface::erase(
                Surface::new(SurfaceKey::<Empty>::new(id), build_empty).multiple(),
                role,
            );

            assert_eq!(
                faults(&declared),
                vec![format!(
                    "the {role} surface `{id}` must be a singleton and cannot declare multiple \
                     instances"
                )],
            );
        }
    }

    #[test]
    fn primary_and_auxiliary_surfaces_may_declare_multiple_instances() {
        for (role, id) in [
            (SurfaceRole::Primary, "primary"),
            (SurfaceRole::Auxiliary, "logs"),
        ] {
            let declared = DeclaredSurface::erase(
                Surface::new(SurfaceKey::<Empty>::new(id), build_empty).multiple(),
                role,
            );

            assert!(faults(&declared).is_empty(), "{role} may be multiple");
        }
    }

    #[test]
    fn a_multiple_surface_may_not_declare_a_reuse_hook() {
        fn reopen(_: &Entity<Empty>, _: &(), _: &mut Window, _: &mut App) {}

        let declared = DeclaredSurface::erase(
            Surface::new(SurfaceKey::<Empty>::new("logs"), build_empty)
                .multiple()
                .on_reuse(reopen),
            SurfaceRole::Auxiliary,
        );
        assert_eq!(
            faults(&declared),
            vec![
                "surface `logs` declares an on_reuse hook but admits multiple instances, so no \
                 open would ever reuse a window"
                    .to_string()
            ],
        );

        // A singleton is exactly where a reuse hook belongs, and a multiple
        // surface without one is fine.
        let singleton = DeclaredSurface::erase(
            Surface::new(SurfaceKey::<Empty>::new("logs"), build_empty).on_reuse(reopen),
            SurfaceRole::Auxiliary,
        );
        assert!(faults(&singleton).is_empty());
        let plain = DeclaredSurface::erase(
            Surface::new(SurfaceKey::<Empty>::new("logs"), build_empty).multiple(),
            SurfaceRole::Auxiliary,
        );
        assert!(faults(&plain).is_empty());
    }

    #[test]
    fn independent_surface_faults_are_all_reported_in_a_fixed_order() {
        let declared = DeclaredSurface::erase(
            Surface::new(SurfaceKey::<Empty>::new("Prefs Window"), build_empty).multiple(),
            SurfaceRole::Settings,
        );

        assert_eq!(
            faults(&declared).len(),
            3,
            "invalid id, wrong reserved id, and invalid cardinality are independent",
        );
    }
}
