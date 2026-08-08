//! Window/overlay open specifications (plan §3 "Windows" builder bullet).
//!
//! [`WindowSpec`] is the builder handed to [`super::WindowManager::open`]: a
//! stable [`WindowKey`], a title, an initial size policy, a [`RootPolicy`], blur
//! /transparency options that delegate to `WindowShell`/`TitleBar`, a pre-show
//! `WindowOptions` customization hook, and a post-open hook for native tweaks
//! (e.g. agent-term's objc2 titlebar). [`OverlaySpec`] is the analogous, much
//! smaller builder for capability-gated overlay surfaces.

use gpui::{
    App, OverlaySurfaceOptions, Pixels, Size, Window, WindowBackgroundAppearance, WindowOptions,
    size,
};
use gpui_component::WindowShell;

use super::key::WindowKey;

/// How the manager treats the caller's content view as a window root.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RootPolicy {
    /// Auto-wrap the content view in `gpui_component::Root` (default). The window
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
            pre_show: None,
            post_open: None,
        }
    }

    /// Set the base window title. Numbering (`" - N"`) is applied by the manager;
    /// pass the un-numbered base here. Defaults to the app display name.
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Override the manifest application id for this window. Most apps should
    /// use the manifest identity supplied by [`super::WindowManager`].
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
    pub fn display_fraction(mut self, fraction: f32) -> Self {
        self.size = WindowSize::DisplayFraction(fraction);
        self
    }

    /// Center the window at an explicit logical size.
    pub fn fixed_size(mut self, width: impl Into<Pixels>, height: impl Into<Pixels>) -> Self {
        self.size = WindowSize::Fixed(size(width.into(), height.into()));
        self
    }

    /// Select the root policy. Prefer [`WindowSpec::raw`] for the non-default.
    pub fn root_policy(mut self, policy: RootPolicy) -> Self {
        self.root_policy = policy;
        self
    }

    /// Use the content view as the window root directly (no `Root` wrapper).
    /// The window must then be opened with [`super::WindowManager::open_raw`].
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
    pub fn blurred(mut self, corner_radius: impl Into<Pixels>) -> Self {
        self.background = WindowBackgroundAppearance::Blurred {
            corner_radius: corner_radius.into(),
        };
        self
    }

    /// Request a plain-transparent background.
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

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::px;

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
}
