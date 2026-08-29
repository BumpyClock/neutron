//! Window/overlay open specifications (plan §3 "Windows" builder bullet).
//!
//! [`WindowSpec`] is the builder handed to [`super::WindowManager::open`]: a
//! stable [`WindowKey`], a title, an initial size policy, a [`RootPolicy`], blur
//! /transparency options that delegate to `WindowShell`/`TitleBar`, a pre-show
//! `WindowOptions` customization hook, and a post-open hook for native tweaks
//! (e.g. agent-term's objc2 titlebar). [`OverlaySpec`] is the analogous, much
//! smaller builder for capability-gated overlay surfaces.

use gpui::{
    App, Entity, OverlaySurfaceOptions, Pixels, SharedString, Size, Window,
    WindowBackgroundAppearance, WindowDecorations, WindowOptions, px, size,
};
use neutron_components::WindowShell;

use super::key::WindowKey;

/// How the manager treats the caller's content view as a window root.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RootPolicy {
    /// Auto-wrap the content view in `neutron_components::Root` (default). The window
    /// handle is therefore typed `WindowHandle<Root>`; the content entity is
    /// returned separately. Realized by [`super::WindowManager::open`].
    #[default]
    ComponentRoot,
    /// Use the content view directly as the window root — no `Root` wrapper.
    /// Realized by [`super::WindowManager::open_raw`].
    Raw,
}

/// Initial window size + placement.
#[derive(Debug, Clone, Copy)]
pub enum WindowSize {
    /// Centered on the active display at `fraction` of the display size
    /// (the story-gallery pattern; default `0.85`).
    DisplayFraction(f32),
    /// Centered at an explicit logical size.
    Fixed(Size<Pixels>),
    /// Centered at an explicit logical size, clamped component-wise to
    /// `max_display_fraction` of the active display size. Preserves a
    /// requested maximum (unlike [`WindowSize::DisplayFraction`]) while
    /// still shrinking to fit small displays (unlike [`WindowSize::Fixed`]).
    /// Falls back to `size` unclamped when there is no active display.
    FixedClamped {
        /// The requested logical size.
        size: Size<Pixels>,
        /// The fraction of the display size each dimension is clamped to.
        /// Guarded the same way as [`WindowSize::DisplayFraction`]: a
        /// non-finite or non-positive value falls back to `0.85`.
        max_display_fraction: f32,
    },
}

impl WindowSize {
    /// Build a [`WindowSize::FixedClamped`] requesting `size`, clamped
    /// component-wise to `max_display_fraction` of the active display.
    pub fn fixed_clamped(size: Size<Pixels>, max_display_fraction: f32) -> Self {
        WindowSize::FixedClamped {
            size,
            max_display_fraction,
        }
    }
}

impl Default for WindowSize {
    fn default() -> Self {
        WindowSize::DisplayFraction(0.85)
    }
}

/// A window-open specification. Single-use: [`super::WindowManager::open`]
/// consumes it (the hooks are `FnOnce`).
pub struct WindowSpec {
    pub(super) key: WindowKey,
    pub(super) title: Option<String>,
    pub(super) app_id: Option<String>,
    pub(super) size: WindowSize,
    pub(super) root_policy: RootPolicy,
    pub(super) background: WindowBackgroundAppearance,
    /// Whether in-window menu chrome may be attached (managed windows only).
    /// Declared surfaces set this from their role/`menu_bar` policy.
    pub(super) menu_bar: bool,
    /// Which counter this window draws its instance number from. `None` keeps
    /// the historical base-title scope; declared surfaces number per key.
    pub(super) numbering_scope: Option<WindowKey>,
    pub(super) pre_show: Option<Box<dyn FnOnce(&mut WindowOptions)>>,
    pub(super) post_open: Option<Box<dyn FnOnce(&mut Window, &mut App)>>,
}

impl WindowSpec {
    /// Start a spec for the given stable key.
    pub fn new(key: impl Into<WindowKey>) -> Self {
        Self {
            key: key.into(),
            title: None,
            app_id: None,
            size: WindowSize::default(),
            root_policy: RootPolicy::default(),
            background: WindowBackgroundAppearance::Opaque,
            menu_bar: true,
            numbering_scope: None,
            pre_show: None,
            post_open: None,
        }
    }

    /// Allow or suppress in-window menu chrome for this managed window.
    ///
    /// Internal: the declared-surface layer resolves a surface's role default
    /// and explicit override into this flag. Suppression is not the same as an
    /// empty menu projection, which never produces an inert menu bar anyway.
    pub(crate) fn menu_bar(mut self, enabled: bool) -> Self {
        self.menu_bar = enabled;
        self
    }

    /// Number this window within `key`'s own counter instead of the counter
    /// shared by every window with the same base title.
    ///
    /// Internal: typed surfaces and raw windows are identified by key, so two
    /// untitled surfaces are each their own first instance rather than
    /// `"App"` and `"App - 2"`. Untyped [`super::WindowManager`] callers keep
    /// the title-based numbering they have today.
    pub(crate) fn numbering_scope(mut self, key: WindowKey) -> Self {
        self.numbering_scope = Some(key);
        self
    }

    /// Set the base window title. Numbering (`" - N"`) is applied by the manager;
    /// pass the un-numbered base here. Defaults to the app display name.
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Override the manifest application id for this window. Most apps should
    /// use the manifest identity supplied by [`super::WindowManager`].
    #[allow(dead_code)] // Exercised only by unit tests; no production caller yet.
    pub fn app_id(mut self, app_id: impl Into<String>) -> Self {
        self.app_id = Some(app_id.into());
        self
    }

    /// Set the initial size policy (default [`WindowSize::DisplayFraction`]`(0.85)`).
    pub fn size(mut self, size: WindowSize) -> Self {
        self.size = size;
        self
    }

    /// Center the window at `fraction` of the active display.
    #[allow(dead_code)] // Exercised only by unit tests; no production caller yet.
    pub fn display_fraction(mut self, fraction: f32) -> Self {
        self.size = WindowSize::DisplayFraction(fraction);
        self
    }

    /// Center the window at an explicit logical size.
    #[allow(dead_code)] // Exercised only by unit tests; no production caller yet.
    pub fn fixed_size(mut self, width: impl Into<Pixels>, height: impl Into<Pixels>) -> Self {
        self.size = WindowSize::Fixed(size(width.into(), height.into()));
        self
    }

    /// Center the window at an explicit logical size, clamped component-wise
    /// to `max_display_fraction` of the active display (see
    /// [`WindowSize::FixedClamped`]).
    #[allow(dead_code)] // Exercised only by unit tests; no production caller yet.
    pub fn fixed_size_clamped(
        mut self,
        width: impl Into<Pixels>,
        height: impl Into<Pixels>,
        max_display_fraction: f32,
    ) -> Self {
        self.size =
            WindowSize::fixed_clamped(size(width.into(), height.into()), max_display_fraction);
        self
    }

    /// Select the root policy. Prefer [`WindowSpec::raw`] for the non-default.
    pub fn root_policy(mut self, policy: RootPolicy) -> Self {
        self.root_policy = policy;
        self
    }

    /// Use the content view as the window root directly (no `Root` wrapper).
    /// The window must then be opened with [`super::WindowManager::open_raw`].
    #[allow(dead_code)] // Exercised only by unit tests; no production caller yet.
    pub fn raw(mut self) -> Self {
        self.root_policy = RootPolicy::Raw;
        self
    }

    /// Set the window background appearance directly.
    pub fn background(mut self, appearance: WindowBackgroundAppearance) -> Self {
        self.background = appearance;
        self
    }

    /// Request an OS-blurred background clipped to `corner_radius` logical px
    /// (`px(0.)` for a full rectangular blur).
    #[allow(dead_code)] // Exercised only by unit tests; no production caller yet.
    pub fn blurred(mut self, corner_radius: impl Into<Pixels>) -> Self {
        self.background = WindowBackgroundAppearance::Blurred {
            corner_radius: corner_radius.into(),
        };
        self
    }

    /// Request a plain-transparent background.
    #[allow(dead_code)] // Exercised only by unit tests; no production caller yet.
    pub fn transparent(mut self) -> Self {
        self.background = WindowBackgroundAppearance::Transparent;
        self
    }

    /// Customize the resolved [`WindowOptions`] just before the window is shown
    /// (after the manager has applied title/size/background defaults).
    pub fn customize_options(mut self, f: impl FnOnce(&mut WindowOptions) + 'static) -> Self {
        self.pre_show = Some(Box::new(f));
        self
    }

    /// Run a hook against the raw `Window` as it opens, before the root view is
    /// built — the sanctioned seam for native tweaks (macOS titlebar, etc.).
    #[allow(dead_code)] // Exercised only by unit tests; no production caller yet.
    pub fn on_open(mut self, f: impl FnOnce(&mut Window, &mut App) + 'static) -> Self {
        self.post_open = Some(Box::new(f));
        self
    }

    /// The stable key.
    pub fn key(&self) -> WindowKey {
        self.key
    }

    /// The declared root policy.
    pub fn declared_root_policy(&self) -> RootPolicy {
        self.root_policy
    }

    pub(super) fn resolved_app_id<'a>(&'a self, manifest_app_id: &'a str) -> &'a str {
        self.app_id.as_deref().unwrap_or(manifest_app_id)
    }
}

/// Build base `WindowOptions` from `WindowShell`/`TitleBar` defaults, then apply
/// `title` and `background`. Bounds are applied separately by the manager (they
/// need `&App` for display centering).
pub(super) fn base_window_options(
    title: &str,
    background: WindowBackgroundAppearance,
) -> WindowOptions {
    let mut options = WindowShell::window_options();
    if let Some(titlebar) = options.titlebar.as_mut() {
        titlebar.title = Some(title.to_string().into());
    }
    options.window_background = background;
    options
}

/// Apply the resolved identity after arbitrary caller option customization, so
/// a normal window cannot accidentally lose its manifest identity.
pub(super) fn apply_app_id(options: &mut WindowOptions, app_id: &str) {
    options.app_id = Some(app_id.to_string());
}

/// An overlay-surface specification (capability-gated, not `Root`-wrapped, not
/// numbered).
pub struct OverlaySpec {
    pub(super) key: WindowKey,
    pub(super) size: Size<Pixels>,
    pub(super) background: WindowBackgroundAppearance,
    pub(super) focus: bool,
    pub(super) customize: Option<Box<dyn FnOnce(&mut OverlaySurfaceOptions)>>,
}

impl OverlaySpec {
    /// Start an overlay spec for the given stable key and logical size.
    pub fn new(
        key: impl Into<WindowKey>,
        width: impl Into<Pixels>,
        height: impl Into<Pixels>,
    ) -> Self {
        Self {
            key: key.into(),
            size: size(width.into(), height.into()),
            background: WindowBackgroundAppearance::Transparent,
            focus: false,
            customize: None,
        }
    }

    /// Set the overlay background appearance (default transparent).
    pub fn background(mut self, appearance: WindowBackgroundAppearance) -> Self {
        self.background = appearance;
        self
    }

    /// Request an OS-blurred background clipped to `corner_radius` logical px.
    pub fn blurred(mut self, corner_radius: impl Into<Pixels>) -> Self {
        self.background = WindowBackgroundAppearance::Blurred {
            corner_radius: corner_radius.into(),
        };
        self
    }

    /// Whether the overlay should take focus when shown (default `false`).
    pub fn focus(mut self, focus: bool) -> Self {
        self.focus = focus;
        self
    }

    /// Customize the resolved [`OverlaySurfaceOptions`] just before creation.
    pub fn customize_options(
        mut self,
        f: impl FnOnce(&mut OverlaySurfaceOptions) + 'static,
    ) -> Self {
        self.customize = Some(Box::new(f));
        self
    }

    /// The stable key.
    pub fn key(&self) -> WindowKey {
        self.key
    }
}

/// Scale a logical size by `fraction`, guarding against non-finite/degenerate
/// fractions so a bad caller value can never produce a zero/NaN window.
pub(super) fn scale_size(base: Size<Pixels>, fraction: f32) -> Size<Pixels> {
    let f = if fraction.is_finite() && fraction > 0.0 {
        fraction
    } else {
        0.85
    };
    size(base.width * f, base.height * f)
}

/// Resolve a [`WindowSize`] to a concrete logical size given the active
/// display's size, if any. Pure and display-independent so it is directly
/// unit-testable without a GPUI test context.
///
/// `display` is `None` when there is no active display (headless/off-screen
/// hosts): [`WindowSize::DisplayFraction`] falls back to a fixed default
/// display size for backward compatibility, while [`WindowSize::FixedClamped`]
/// uses the requested size unclamped, per its documented contract.
pub(super) fn resolve_window_size(size: WindowSize, display: Option<Size<Pixels>>) -> Size<Pixels> {
    match size {
        WindowSize::Fixed(size) => size,
        WindowSize::DisplayFraction(fraction) => {
            let display = display.unwrap_or_else(|| self::size(px(1024.0), px(768.0)));
            scale_size(display, fraction)
        }
        WindowSize::FixedClamped {
            size: requested,
            max_display_fraction,
        } => match display {
            Some(display) => requested.min(&scale_size(display, max_display_fraction)),
            None => requested,
        },
    }
}

/// Apply the geometry and platform options a declared surface or typed raw
/// window states, then let its own `configure_window` hook have the last word.
///
/// Installed as the spec's pre-show hook, so it runs after the manager applied
/// title/size/background defaults and before the manifest identity is reapplied.
pub(crate) fn apply_window_options(
    options: &mut WindowOptions,
    min_size: Option<Size<Pixels>>,
    decorations: Option<WindowDecorations>,
    configure: Option<fn(&mut WindowOptions)>,
) {
    if let Some(min_size) = min_size {
        options.window_min_size = Some(min_size);
    }
    if let Some(decorations) = decorations {
        options.window_decorations = Some(decorations);
    }
    if let Some(configure) = configure {
        configure(options);
    }
}

/// A typed raw-window value: the runtime escape from `Root` composition.
///
/// The content view *is* the window root, so the application owns all chrome
/// and overlay composition. Not declarable: raw windows are opened explicitly at
/// runtime via [`crate::Shell::open_raw`], never installed as a declared
/// surface, and they never receive framework menu chrome.
pub struct RawWindow<View, Args = ()> {
    key: WindowKey,
    build: fn(&Args, &mut Window, &mut App) -> Entity<View>,
    title: Option<SharedString>,
    size: WindowSize,
    min_size: Option<Size<Pixels>>,
    background: WindowBackgroundAppearance,
    decorations: Option<WindowDecorations>,
    configure_window: Option<fn(&mut WindowOptions)>,
    after_open: Option<fn(&Entity<View>, &mut Window, &mut App)>,
}

impl<View: 'static, Args: 'static> RawWindow<View, Args> {
    /// A raw window under `key`, whose root view is built by `build`.
    #[must_use]
    pub fn new(
        key: impl Into<WindowKey>,
        build: fn(&Args, &mut Window, &mut App) -> Entity<View>,
    ) -> Self {
        Self {
            key: key.into(),
            build,
            title: None,
            size: WindowSize::default(),
            min_size: None,
            background: WindowBackgroundAppearance::Opaque,
            decorations: None,
            configure_window: None,
            after_open: None,
        }
    }

    /// Set the un-numbered base title (defaults to the app display name).
    #[must_use]
    pub fn title(mut self, title: impl Into<SharedString>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Set the initial size policy.
    #[must_use]
    pub fn size(mut self, size: WindowSize) -> Self {
        self.size = size;
        self
    }

    /// Constrain the window to a minimum logical size.
    #[must_use]
    pub fn min_size(mut self, size: Size<Pixels>) -> Self {
        self.min_size = Some(size);
        self
    }

    /// Set the window background appearance.
    #[must_use]
    pub fn background(mut self, background: WindowBackgroundAppearance) -> Self {
        self.background = background;
        self
    }

    /// Request client- or server-side decorations (Wayland; may be ignored).
    #[must_use]
    pub fn decorations(mut self, decorations: WindowDecorations) -> Self {
        self.decorations = Some(decorations);
        self
    }

    /// Customize the resolved [`WindowOptions`] just before the window shows.
    #[must_use]
    pub fn configure_window(mut self, hook: fn(&mut WindowOptions)) -> Self {
        self.configure_window = Some(hook);
        self
    }

    /// Run a hook against the opened window and its root view.
    #[must_use]
    pub fn after_open(mut self, hook: fn(&Entity<View>, &mut Window, &mut App)) -> Self {
        self.after_open = Some(hook);
        self
    }

    /// The stable key.
    pub fn key(&self) -> WindowKey {
        self.key
    }

    /// The content factory.
    pub fn build(&self) -> fn(&Args, &mut Window, &mut App) -> Entity<View> {
        self.build
    }

    /// The post-open hook, if any.
    pub fn after_open_hook(&self) -> Option<fn(&Entity<View>, &mut Window, &mut App)> {
        self.after_open
    }

    /// The equivalent [`WindowSpec`], always in [`RootPolicy::Raw`].
    pub(crate) fn spec(&self) -> WindowSpec {
        let (min_size, decorations, configure) =
            (self.min_size, self.decorations, self.configure_window);
        let mut spec = WindowSpec::new(self.key)
            .root_policy(RootPolicy::Raw)
            .numbering_scope(self.key)
            .size(self.size)
            .background(self.background)
            .menu_bar(false)
            .customize_options(move |options| {
                apply_window_options(options, min_size, decorations, configure);
            });
        if let Some(title) = &self.title {
            spec = spec.title(title.to_string());
        }
        spec
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{AppContext as _, px};

    #[test]
    fn test_window_spec_builder() {
        // Defaults.
        let spec = WindowSpec::new(WindowKey::new("main"));
        assert_eq!(spec.key(), WindowKey::new("main"));
        assert_eq!(spec.declared_root_policy(), RootPolicy::ComponentRoot);
        assert!(spec.title.is_none());
        assert!(spec.app_id.is_none());
        assert!(matches!(
            spec.size,
            WindowSize::DisplayFraction(f) if (f - 0.85).abs() < f32::EPSILON
        ));
        assert!(matches!(
            spec.background,
            WindowBackgroundAppearance::Opaque
        ));
        assert!(spec.pre_show.is_none());
        assert!(spec.post_open.is_none());

        // Mutators + hooks.
        let spec = WindowSpec::new(WindowKey::new("main"))
            .title("MyApp")
            .app_id("com.example.override")
            .fixed_size(px(640.), px(480.))
            .raw()
            .blurred(px(8.))
            .customize_options(|_| {})
            .on_open(|_, _| {});
        assert_eq!(spec.title.as_deref(), Some("MyApp"));
        assert_eq!(
            spec.resolved_app_id("com.example.default"),
            "com.example.override"
        );
        assert_eq!(spec.declared_root_policy(), RootPolicy::Raw);
        assert!(matches!(
            spec.size,
            WindowSize::Fixed(s) if s.width == px(640.) && s.height == px(480.)
        ));
        assert!(matches!(
            spec.background,
            WindowBackgroundAppearance::Blurred { .. }
        ));
        assert!(spec.pre_show.is_some());
        assert!(spec.post_open.is_some());

        // Remaining size/background variants.
        let spec = WindowSpec::new(WindowKey::new("w"))
            .display_fraction(0.5)
            .transparent();
        assert!(matches!(
            spec.size,
            WindowSize::DisplayFraction(f) if (f - 0.5).abs() < f32::EPSILON
        ));
        assert!(matches!(
            spec.background,
            WindowBackgroundAppearance::Transparent
        ));

        // FixedClamped variant.
        let spec =
            WindowSpec::new(WindowKey::new("w")).fixed_size_clamped(px(1600.), px(1200.), 0.85);
        assert!(matches!(
            spec.size,
            WindowSize::FixedClamped { size, max_display_fraction }
                if size.width == px(1600.) && size.height == px(1200.)
                    && (max_display_fraction - 0.85).abs() < f32::EPSILON
        ));
    }

    #[test]
    fn test_overlay_spec_builder() {
        // Defaults.
        let spec = OverlaySpec::new(WindowKey::new("hud"), px(320.), px(200.));
        assert_eq!(spec.key(), WindowKey::new("hud"));
        assert_eq!(spec.size.width, px(320.));
        assert_eq!(spec.size.height, px(200.));
        assert!(matches!(
            spec.background,
            WindowBackgroundAppearance::Transparent
        ));
        assert!(!spec.focus);
        assert!(spec.customize.is_none());

        // Mutators + hook.
        let spec = OverlaySpec::new(WindowKey::new("hud"), px(10.), px(10.))
            .background(WindowBackgroundAppearance::Opaque)
            .focus(true)
            .customize_options(|_| {});
        assert!(matches!(
            spec.background,
            WindowBackgroundAppearance::Opaque
        ));
        assert!(spec.focus);
        assert!(spec.customize.is_some());

        let spec = OverlaySpec::new(WindowKey::new("hud"), px(10.), px(10.)).blurred(px(4.));
        assert!(matches!(
            spec.background,
            WindowBackgroundAppearance::Blurred { .. }
        ));
    }

    #[test]
    fn scale_size_guards_degenerate_fractions() {
        let base = size(px(100.), px(200.));
        let scaled = scale_size(base, 0.5);
        assert_eq!(scaled.width, px(50.));
        assert_eq!(scaled.height, px(100.));
        // Non-finite / non-positive fractions fall back to 0.85.
        for bad in [f32::NAN, f32::INFINITY, 0.0, -1.0] {
            let scaled = scale_size(base, bad);
            assert_eq!(scaled.width, px(85.));
            assert_eq!(scaled.height, px(170.));
        }
    }

    #[test]
    fn fixed_clamped_shrinks_to_fit_a_small_display() {
        let requested = size(px(1600.), px(1200.));
        let small_display = size(px(1000.), px(800.));
        let resolved = resolve_window_size(
            WindowSize::fixed_clamped(requested, 0.85),
            Some(small_display),
        );
        // 0.85 * 1000 = 850, 0.85 * 800 = 680: both below the request, so both
        // dimensions clamp down.
        assert_eq!(resolved.width, px(850.));
        assert_eq!(resolved.height, px(680.));
    }

    #[test]
    fn fixed_clamped_preserves_the_requested_maximum_on_a_large_display() {
        let requested = size(px(1600.), px(1200.));
        let large_display = size(px(3000.), px(2000.));
        let resolved = resolve_window_size(
            WindowSize::fixed_clamped(requested, 0.85),
            Some(large_display),
        );
        // 0.85 * 3000 = 2550, 0.85 * 2000 = 1700: both above the request, so
        // the requested size is preserved untouched (unlike DisplayFraction,
        // which would ignore the request and always scale the display).
        assert_eq!(resolved.width, px(1600.));
        assert_eq!(resolved.height, px(1200.));
    }

    #[test]
    fn fixed_clamped_clamps_only_the_dimension_that_exceeds_the_display() {
        let requested = size(px(1600.), px(1200.));
        let mixed_display = size(px(1920.), px(1080.));
        let resolved = resolve_window_size(
            WindowSize::fixed_clamped(requested, 0.85),
            Some(mixed_display),
        );
        // 0.85 * 1920 = 1632 (>= 1600, width untouched); 0.85 * 1080 = 918
        // (< 1200, height clamps down): a genuinely component-wise min.
        assert_eq!(resolved.width, px(1600.));
        assert_eq!(resolved.height, px(918.));
    }

    #[test]
    fn fixed_clamped_uses_the_requested_size_without_a_display() {
        let requested = size(px(1600.), px(1200.));
        let resolved = resolve_window_size(WindowSize::fixed_clamped(requested, 0.85), None);
        assert_eq!(resolved, requested);
    }

    #[test]
    fn fixed_clamped_guards_degenerate_fractions_like_display_fraction() {
        let requested = size(px(1600.), px(1200.));
        let display = size(px(1000.), px(800.));
        for bad in [f32::NAN, f32::INFINITY, 0.0, -1.0] {
            let resolved =
                resolve_window_size(WindowSize::fixed_clamped(requested, bad), Some(display));
            // Falls back to the same 0.85 default as `scale_size`, then clamps.
            assert_eq!(resolved.width, px(850.));
            assert_eq!(resolved.height, px(680.));
        }
    }

    #[test]
    fn fixed_stays_unclamped_regardless_of_display() {
        let requested = size(px(1600.), px(1200.));
        let small_display = size(px(100.), px(100.));
        assert_eq!(
            resolve_window_size(WindowSize::Fixed(requested), Some(small_display)),
            requested
        );
        assert_eq!(
            resolve_window_size(WindowSize::Fixed(requested), None),
            requested
        );
    }

    #[test]
    fn manifest_identity_wins_after_option_customization_without_an_override() {
        let mut spec = WindowSpec::new(WindowKey::new("main")).customize_options(|options| {
            options.app_id = Some("com.example.accidental".to_string());
        });
        let mut options = base_window_options("App", WindowBackgroundAppearance::Opaque);
        spec.pre_show.take().unwrap()(&mut options);
        apply_app_id(&mut options, spec.resolved_app_id("com.example.app"));
        assert_eq!(options.app_id.as_deref(), Some("com.example.app"));
    }

    #[test]
    fn explicit_window_identity_overrides_the_manifest() {
        let spec = WindowSpec::new(WindowKey::new("main")).app_id("com.example.window");
        let mut options = base_window_options("App", WindowBackgroundAppearance::Opaque);
        apply_app_id(&mut options, spec.resolved_app_id("com.example.app"));
        assert_eq!(options.app_id.as_deref(), Some("com.example.window"));
    }

    #[test]
    fn managed_windows_admit_menu_chrome_until_a_caller_suppresses_it() {
        assert!(WindowSpec::new(WindowKey::new("main")).menu_bar);
        assert!(
            !WindowSpec::new(WindowKey::new("settings"))
                .menu_bar(false)
                .menu_bar
        );
    }

    #[test]
    fn declared_window_options_apply_before_the_callers_own_hook() {
        let mut options = base_window_options("App", WindowBackgroundAppearance::Opaque);

        apply_window_options(
            &mut options,
            Some(size(px(320.), px(240.))),
            Some(WindowDecorations::Client),
            Some(|options: &mut WindowOptions| {
                options.window_min_size = Some(size(px(640.), px(480.)));
            }),
        );

        assert_eq!(
            options.window_min_size,
            Some(size(px(640.), px(480.))),
            "the explicit escape hook has the last word",
        );
        assert_eq!(options.window_decorations, Some(WindowDecorations::Client));
    }

    #[test]
    fn unstated_window_options_leave_the_platform_defaults_alone() {
        let mut options = base_window_options("App", WindowBackgroundAppearance::Opaque);

        apply_window_options(&mut options, None, None, None);

        assert_eq!(options.window_min_size, None);
        assert_eq!(options.window_decorations, None);
    }

    #[test]
    fn a_raw_window_lowers_to_a_raw_unchromed_spec() {
        fn build(_: &(), _: &mut Window, cx: &mut App) -> Entity<gpui::Empty> {
            cx.new(|_| gpui::Empty)
        }

        let raw = RawWindow::new("inspector", build)
            .title("Inspector")
            .size(WindowSize::DisplayFraction(0.5))
            .min_size(size(px(320.), px(240.)))
            .background(WindowBackgroundAppearance::Transparent)
            .decorations(WindowDecorations::Server)
            .after_open(|_, _, _| {});

        assert_eq!(raw.key(), WindowKey::new("inspector"));
        assert!(raw.after_open_hook().is_some());

        let mut spec = raw.spec();
        assert_eq!(spec.declared_root_policy(), RootPolicy::Raw);
        assert_eq!(spec.title.as_deref(), Some("Inspector"));
        assert!(!spec.menu_bar, "raw windows never take framework chrome");
        assert!(matches!(
            spec.background,
            WindowBackgroundAppearance::Transparent
        ));

        let mut options = base_window_options("Inspector", spec.background);
        spec.pre_show.take().expect("geometry hook installed")(&mut options);
        assert_eq!(options.window_min_size, Some(size(px(320.), px(240.))));
        assert_eq!(options.window_decorations, Some(WindowDecorations::Server));
    }

    #[test]
    fn a_raw_window_without_a_title_defers_to_the_app_display_name() {
        fn build(_: &(), _: &mut Window, cx: &mut App) -> Entity<gpui::Empty> {
            cx.new(|_| gpui::Empty)
        }

        assert!(RawWindow::new("inspector", build).spec().title.is_none());
    }
}
