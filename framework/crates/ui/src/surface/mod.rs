//! Surface module providing a unified API for glass/blur/noise surface effects.
//!
//! This module consolidates surface rendering logic into a preset-based system that
//! handles backdrop blur, noise overlays, elevation shadows, and border styling.
//!
//! # Example
//!
//! ```rust,ignore
//! use ui::surface::{SurfacePreset, SurfaceContext};
//!
//! let surface = SurfacePreset::flyout()
//!     .with_transparency_factor(0.9)
//!     .wrap_with_bounds(content, width, height, window, cx, ctx);
//! ```

mod flyout;

pub use flyout::*;

use gpui::{
    App, Div, Hsla, ImageSource, IntoElement, ObjectFit, ParentElement, Pixels, Resource, Rgba,
    Styled, StyledImage, Window, div, img, px,
};

use crate::{ActiveTheme, StyledExt, ThemeShadowToken, global_state::GlobalState};

const GLASS_NOISE_ASSET_PATH: &str = "surface/NoiseAsset_256.png";
const GLASS_NOISE_TILE_SIZE_BASE: f32 = 128.0;

/// Runtime context for surface rendering decisions.
#[derive(Debug, Clone, Copy, Default)]
pub struct SurfaceContext {
    /// Whether blur effects are enabled (controlled by app settings).
    pub blur_enabled: bool,
}

impl SurfaceContext {
    /// Builds a context from the app's current blur setting.
    pub(crate) fn new(cx: &App) -> Self {
        Self {
            blur_enabled: GlobalState::global(cx).blur_enabled(),
        }
    }
}

/// Semantic categorization of surface types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SurfaceKind {
    #[default]
    Base,
    Flyout,
    Panel,
    Card,
}

/// Noise overlay intensity presets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NoiseIntensity {
    None,
    #[default]
    Subtle,
    Heavy,
}

impl NoiseIntensity {
    /// Returns the opacity value for this noise intensity.
    pub fn opacity(&self) -> f32 {
        match self {
            NoiseIntensity::None => 0.0,
            NoiseIntensity::Subtle => 0.02,
            NoiseIntensity::Heavy => 0.04,
        }
    }
}

/// Maps to existing theme elevation levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ElevationToken {
    #[default]
    None,
    Xs,
    Sm,
    Md,
    Lg,
    Xl,
}

impl ElevationToken {
    /// Applies the elevation shadow to the given element.
    pub fn apply<E: Styled + StyledExt>(&self, element: E, _cx: &App) -> E {
        match self {
            ElevationToken::None => element,
            ElevationToken::Xs => element.shadow_sm(),
            ElevationToken::Sm => element.shadow_sm(),
            ElevationToken::Md => element.shadow_md(),
            ElevationToken::Lg => element.shadow_lg(),
            ElevationToken::Xl => element.shadow_xl(),
        }
    }
}

/// Source for surface background color from theme.
///
/// Every variant derives from a theme color, so custom themes tint every surface
/// automatically. The `Elevated*` variants add the material's luminance character
/// on top of the theme color — see [`elevated`].
#[derive(Debug, Clone, Copy, Default)]
pub enum SurfaceColorSource {
    #[default]
    Popover,
    Sidebar,
    Background,
    /// [`crate::Theme::popover`] lifted to flyout-material luminance.
    ElevatedPopover,
    /// [`crate::Theme::sidebar`] lifted to panel-material luminance.
    ElevatedSidebar,
    /// [`crate::Theme::background`] lifted toward white for the card wash.
    ElevatedBackground,
}

/// Blend fractions lifting a theme color toward the mode's elevation pole — white
/// in dark mode (surfaces rise toward light), black in light mode (materials sit a
/// step under a white window). Calibrated so the neutral default theme reproduces
/// the Fluent material values these replaced — acrylic `#2C2C2C`/`#FCFCFC` and
/// mica `#202020`/`#F3F3F3` over the default `#0A0A0A`/`#FFFFFF` window — while
/// tinted themes keep their hue instead of collapsing to those neutrals.
const FLYOUT_LIFT_DARK: f32 = 34. / 245.;
const FLYOUT_LIFT_LIGHT: f32 = 3. / 255.;
const PANEL_LIFT_DARK: f32 = 22. / 245.;
const PANEL_LIFT_LIGHT: f32 = 12. / 255.;
/// The card wash blends far toward white in both modes (it replaced a pure white
/// wash) but keeps enough of the background to carry the theme's hue.
const CARD_WASH_LIFT: f32 = 0.85;

/// Blends `base` toward `pole` (per sRGB channel) by `amount`, keeping alpha.
fn mix_toward(base: Hsla, pole: f32, amount: f32) -> Hsla {
    let base = Rgba::from(base);
    let channel = |value: f32| value + (pole - value) * amount;
    Hsla::from(Rgba {
        r: channel(base.r),
        g: channel(base.g),
        b: channel(base.b),
        a: base.a,
    })
}

/// A theme color carried to a material's luminance step: lifted toward white in
/// dark mode, settled toward black in light mode.
pub(crate) fn elevated(base: Hsla, is_dark: bool, dark_lift: f32, light_lift: f32) -> Hsla {
    if is_dark {
        mix_toward(base, 1.0, dark_lift)
    } else {
        mix_toward(base, 0.0, light_lift)
    }
}

/// The flyout material's base color for a given theme popover color, before the
/// flyout opacity is applied. Shared with the theme contrast tests so they measure
/// the same surface the runtime renders.
pub(crate) fn flyout_base_color(popover: Hsla, is_dark: bool) -> Hsla {
    elevated(popover, is_dark, FLYOUT_LIFT_DARK, FLYOUT_LIFT_LIGHT)
}

/// Background configuration with light/dark mode variants.
#[derive(Debug, Clone, Copy)]
pub struct SurfaceBackground {
    pub color_source: SurfaceColorSource,
    pub light_opacity: f32,
    pub dark_opacity: f32,
}

impl Default for SurfaceBackground {
    fn default() -> Self {
        Self {
            color_source: SurfaceColorSource::Popover,
            light_opacity: 0.85,
            dark_opacity: 0.90,
        }
    }
}

impl SurfaceBackground {
    /// Resolves the background color based on theme mode and opacity settings.
    pub fn resolve(&self, cx: &App) -> Hsla {
        let is_dark = cx.theme().mode.is_dark();
        let base = match self.color_source {
            SurfaceColorSource::Popover => cx.theme().popover,
            SurfaceColorSource::Sidebar => cx.theme().sidebar,
            SurfaceColorSource::Background => cx.theme().background,
            SurfaceColorSource::ElevatedPopover => flyout_base_color(cx.theme().popover, is_dark),
            SurfaceColorSource::ElevatedSidebar => elevated(
                cx.theme().sidebar,
                is_dark,
                PANEL_LIFT_DARK,
                PANEL_LIFT_LIGHT,
            ),
            SurfaceColorSource::ElevatedBackground => {
                mix_toward(cx.theme().background, 1.0, CARD_WASH_LIFT)
            }
        };
        let opacity = if cx.theme().mode.is_dark() {
            self.dark_opacity
        } else {
            self.light_opacity
        };
        base.opacity(opacity)
    }
}

/// Border/stroke color options.
#[derive(Debug, Clone, Copy, Default)]
pub enum StrokeColor {
    #[default]
    Subtle,
    Default,
    Strong,
    SubtleWithOpacity(f32),
}

/// Border/stroke specification.
#[derive(Debug, Clone, Copy)]
pub struct StrokeSpec {
    pub width: Pixels,
    pub color: StrokeColor,
}

impl StrokeSpec {
    /// Creates a subtle stroke specification with 1px width.
    pub fn subtle() -> Self {
        Self {
            width: px(1.0),
            color: StrokeColor::Subtle,
        }
    }

    /// Creates a default border specification with 1px width.
    pub fn default_border() -> Self {
        Self {
            width: px(1.0),
            color: StrokeColor::Default,
        }
    }

    /// Resolves the stroke color based on the current theme.
    pub fn resolve_color(&self, cx: &App) -> Hsla {
        let subtle_stroke_opacity = if cx.theme().mode.is_dark() {
            cx.theme().material.subtle_stroke_dark_opacity
        } else {
            cx.theme().material.subtle_stroke_light_opacity
        };
        match self.color {
            StrokeColor::Subtle => cx.theme().border.opacity(subtle_stroke_opacity),
            StrokeColor::Default => cx.theme().border,
            StrokeColor::Strong => cx.theme().border,
            StrokeColor::SubtleWithOpacity(opacity) => cx.theme().border.opacity(opacity),
        }
    }
}

/// A preset configuration for surface appearance.
///
/// Surfaces are the foundational visual containers in the UI. This struct provides
/// a declarative way to configure backdrop blur, noise overlays, elevation shadows,
/// borders, and background colors.
#[derive(Debug, Clone)]
pub struct SurfacePreset {
    pub kind: SurfaceKind,
    pub blur_radius: Option<Pixels>,
    pub noise_intensity: NoiseIntensity,
    pub background: SurfaceBackground,
    pub elevation: ElevationToken,
    pub stroke: Option<StrokeSpec>,
    pub transparency_factor: f32,
    pub radius: Option<Pixels>,
    pub use_theme_material_defaults: bool,
    pub use_theme_elevation_defaults: bool,
}

impl SurfacePreset {
    /// Creates a base surface preset for primary content areas.
    ///
    /// - No blur
    /// - Heavy noise (only rendered when blur_enabled)
    /// - No background color (transparent)
    /// - No elevation or stroke
    pub fn base() -> Self {
        Self {
            kind: SurfaceKind::Base,
            blur_radius: None,
            noise_intensity: NoiseIntensity::Heavy,
            background: SurfaceBackground {
                color_source: SurfaceColorSource::Background,
                light_opacity: 0.0,
                dark_opacity: 0.0,
            },
            elevation: ElevationToken::None,
            stroke: None,
            transparency_factor: 1.0,
            radius: None,
            use_theme_material_defaults: false,
            use_theme_elevation_defaults: false,
        }
    }

    /// Creates a flyout surface preset for menus and dropdowns.
    ///
    /// - 24px blur radius
    /// - Subtle noise
    /// - Elevated popover background at 0.86/0.88 opacity
    /// - Medium elevation with subtle stroke
    /// - 12px border radius
    pub fn flyout() -> Self {
        Self {
            kind: SurfaceKind::Flyout,
            blur_radius: Some(px(24.0)),
            noise_intensity: NoiseIntensity::Subtle,
            background: SurfaceBackground {
                color_source: SurfaceColorSource::ElevatedPopover,
                light_opacity: 0.86,
                dark_opacity: 0.88,
            },
            elevation: ElevationToken::Md,
            stroke: Some(StrokeSpec::subtle()),
            transparency_factor: 1.0,
            radius: Some(px(12.0)),
            use_theme_material_defaults: true,
            use_theme_elevation_defaults: true,
        }
    }

    /// Creates a panel surface preset for sidebars and navigation.
    ///
    /// - 48px blur radius
    /// - Subtle noise
    /// - Elevated sidebar background at 0.88/0.90 opacity
    /// - Large elevation with subtle stroke
    /// - 16px border radius
    pub fn panel() -> Self {
        Self {
            kind: SurfaceKind::Panel,
            blur_radius: Some(px(48.0)),
            noise_intensity: NoiseIntensity::Subtle,
            background: SurfaceBackground {
                color_source: SurfaceColorSource::ElevatedSidebar,
                light_opacity: 0.88,
                dark_opacity: 0.90,
            },
            elevation: ElevationToken::Lg,
            stroke: Some(StrokeSpec::subtle()),
            transparency_factor: 1.0,
            radius: Some(px(16.0)),
            use_theme_material_defaults: true,
            use_theme_elevation_defaults: true,
        }
    }

    /// Creates a card surface preset for content cards.
    ///
    /// - No blur
    /// - No noise
    /// - Theme-tinted near-white wash at 0.70/0.05 opacity
    /// - Small elevation with default border
    /// - Uses theme radius
    pub fn card() -> Self {
        Self {
            kind: SurfaceKind::Card,
            blur_radius: None,
            noise_intensity: NoiseIntensity::None,
            background: SurfaceBackground {
                color_source: SurfaceColorSource::ElevatedBackground,
                light_opacity: 0.70,
                dark_opacity: 0.05,
            },
            elevation: ElevationToken::Sm,
            stroke: Some(StrokeSpec::default_border()),
            transparency_factor: 1.0,
            radius: None,
            use_theme_material_defaults: true,
            use_theme_elevation_defaults: true,
        }
    }

    /// Sets the transparency factor for the background.
    pub fn with_transparency_factor(mut self, factor: f32) -> Self {
        self.transparency_factor = factor;
        self
    }

    /// Sets the blur radius for the backdrop blur effect.
    pub fn with_blur_radius(mut self, radius: Option<Pixels>) -> Self {
        self.blur_radius = radius;
        self.use_theme_material_defaults = false;
        self
    }

    /// Sets the border radius for the surface.
    pub fn with_radius(mut self, radius: Pixels) -> Self {
        self.radius = Some(radius);
        self
    }

    /// Sets the noise overlay intensity.
    pub fn with_noise(mut self, intensity: NoiseIntensity) -> Self {
        self.noise_intensity = intensity;
        self
    }

    /// Sets the elevation level for shadow effects.
    pub fn with_elevation(mut self, elevation: ElevationToken) -> Self {
        self.elevation = elevation;
        self.use_theme_elevation_defaults = false;
        self
    }

    /// Sets the stroke/border specification.
    pub fn with_stroke(mut self, stroke: Option<StrokeSpec>) -> Self {
        self.stroke = stroke;
        self.use_theme_material_defaults = false;
        self
    }

    /// Applies the surface material (radius, background, backdrop blur, stroke, elevation)
    /// directly to an element.
    ///
    /// Use this when the element already owns its layout and only needs the material, e.g.
    /// a popover body. Use [`Self::wrap_with_bounds`] instead when the surface should also
    /// render the noise overlay, which requires known bounds.
    pub(crate) fn apply_material<E>(&self, mut surface: E, cx: &App, ctx: SurfaceContext) -> E
    where
        E: Styled + StyledExt,
    {
        let radius = self.radius.unwrap_or(cx.theme().radius);
        let blur_radius = self.resolve_blur_radius(cx);
        let mut background = self.resolve_background(cx);
        let elevation = self.resolve_elevation(cx);
        let uses_opaque_fallback = !ctx.blur_enabled && blur_radius.is_some();

        if uses_opaque_fallback {
            background.light_opacity = 1.0;
            background.dark_opacity = 1.0;
        }

        let transparency_factor = if uses_opaque_fallback {
            1.0
        } else {
            self.transparency_factor
        };
        let bg_color = background.resolve(cx).opacity(transparency_factor);
        surface = surface.rounded(radius);

        if bg_color.a > 0.0 {
            surface = surface.bg(bg_color);
        }

        if ctx.blur_enabled {
            if let Some(blur_radius) = blur_radius {
                surface = surface.backdrop_blur(blur_radius);
            }
        }

        if let Some(ref stroke) = self.stroke {
            let stroke_color = if matches!(
                self.background.color_source,
                SurfaceColorSource::ElevatedPopover | SurfaceColorSource::ElevatedSidebar
            ) && matches!(stroke.color, StrokeColor::Subtle)
            {
                cx.theme().control_stroke
            } else {
                stroke.resolve_color(cx)
            };
            surface = surface.border(stroke.width).border_color(stroke_color);
        }

        elevation.apply(surface, cx)
    }

    /// Wraps content in a surface container with all configured effects.
    ///
    /// This method creates a complete surface element with:
    /// - One rounded clip shared by the background, backdrop blur, noise, and content
    /// - Border/stroke styling using the same rounded geometry
    /// - Elevation shadows painted outside the rounded clip
    /// - Noise overlay (if blur is enabled)
    pub fn wrap_with_bounds(
        &self,
        content: impl IntoElement,
        width: Pixels,
        height: Pixels,
        window: &Window,
        cx: &App,
        ctx: SurfaceContext,
    ) -> Div {
        let scale_factor = window.scale_factor();
        let noise_opacity = self.noise_intensity.opacity();
        let should_render_noise = ctx.blur_enabled && noise_opacity > 0.0;

        let mut surface = self.apply_material(div().relative().overflow_hidden(), cx, ctx);

        if should_render_noise {
            surface = surface.child(render_noise_overlay_unclipped(
                width,
                height,
                noise_opacity,
                scale_factor,
            ));
        }

        surface.child(content)
    }

    fn resolve_blur_radius(&self, cx: &App) -> Option<Pixels> {
        if !self.use_theme_material_defaults {
            return self.blur_radius;
        }

        match self.kind {
            SurfaceKind::Flyout => Some(cx.theme().material.flyout_blur_radius),
            SurfaceKind::Panel => Some(cx.theme().material.panel_blur_radius),
            SurfaceKind::Card | SurfaceKind::Base => self.blur_radius,
        }
    }

    pub(crate) fn resolve_background(&self, cx: &App) -> SurfaceBackground {
        if !self.use_theme_material_defaults {
            return self.background;
        }

        match self.kind {
            SurfaceKind::Flyout => SurfaceBackground {
                color_source: self.background.color_source,
                light_opacity: cx.theme().material.flyout_light_opacity,
                dark_opacity: cx.theme().material.flyout_dark_opacity,
            },
            SurfaceKind::Panel => SurfaceBackground {
                color_source: self.background.color_source,
                light_opacity: cx.theme().material.panel_light_opacity,
                dark_opacity: cx.theme().material.panel_dark_opacity,
            },
            SurfaceKind::Card => SurfaceBackground {
                color_source: self.background.color_source,
                light_opacity: cx.theme().material.card_light_opacity,
                dark_opacity: cx.theme().material.card_dark_opacity,
            },
            SurfaceKind::Base => self.background,
        }
    }

    fn resolve_elevation(&self, cx: &App) -> ElevationToken {
        if !self.use_theme_elevation_defaults {
            return self.elevation;
        }

        match self.kind {
            SurfaceKind::Flyout => {
                theme_shadow_token_to_elevation_token(cx.theme().elevation.surface_flyout_shadow)
            }
            SurfaceKind::Panel => {
                theme_shadow_token_to_elevation_token(cx.theme().elevation.surface_panel_shadow)
            }
            SurfaceKind::Card => {
                theme_shadow_token_to_elevation_token(cx.theme().elevation.surface_card_shadow)
            }
            SurfaceKind::Base => self.elevation,
        }
    }
}

fn theme_shadow_token_to_elevation_token(token: ThemeShadowToken) -> ElevationToken {
    match token {
        ThemeShadowToken::None => ElevationToken::None,
        ThemeShadowToken::Xs => ElevationToken::Xs,
        ThemeShadowToken::Sm => ElevationToken::Sm,
        ThemeShadowToken::Md => ElevationToken::Md,
        ThemeShadowToken::Lg => ElevationToken::Lg,
        ThemeShadowToken::Xl => ElevationToken::Xl,
    }
}

/// Renders a tiled noise overlay for glass effects.
///
/// This is exposed publicly for cases where the full `wrap_with_bounds` API
/// is too restrictive due to borrow checker constraints.
pub fn render_noise_overlay(
    width: Pixels,
    height: Pixels,
    radius: Pixels,
    opacity: f32,
    scale_factor: f32,
) -> impl IntoElement {
    div()
        .absolute()
        .inset_0()
        .size_full()
        .rounded(radius)
        .overflow_hidden()
        .child(render_noise_tiles(width, height, opacity, scale_factor))
}

fn render_noise_overlay_unclipped(
    width: Pixels,
    height: Pixels,
    opacity: f32,
    scale_factor: f32,
) -> Div {
    div()
        .absolute()
        .inset_0()
        .size_full()
        .child(render_noise_tiles(width, height, opacity, scale_factor))
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct NoiseTileLayout {
    tile_size: Pixels,
    cols: usize,
    rows: usize,
    last_tile_width: Pixels,
    last_tile_height: Pixels,
}

fn noise_tile_layout(width: Pixels, height: Pixels, scale_factor: f32) -> NoiseTileLayout {
    let tile_size_value = (GLASS_NOISE_TILE_SIZE_BASE / scale_factor.max(1.0))
        .round()
        .max(1.0);
    let tile_size = px(tile_size_value);

    if width <= px(0.0) || height <= px(0.0) {
        return NoiseTileLayout {
            tile_size,
            cols: 0,
            rows: 0,
            last_tile_width: px(0.0),
            last_tile_height: px(0.0),
        };
    }

    let cols = (width / tile_size).ceil() as usize;
    let rows = (height / tile_size).ceil() as usize;
    let last_tile_width = width - px(tile_size_value * cols.saturating_sub(1) as f32);
    let last_tile_height = height - px(tile_size_value * rows.saturating_sub(1) as f32);

    NoiseTileLayout {
        tile_size,
        cols,
        rows,
        last_tile_width,
        last_tile_height,
    }
}

fn render_noise_tiles(width: Pixels, height: Pixels, opacity: f32, scale_factor: f32) -> Div {
    let layout = noise_tile_layout(width, height, scale_factor);
    let tiles = layout.cols.saturating_mul(layout.rows);

    div()
        .absolute()
        .top_0()
        .left_0()
        .w(width)
        .h(height)
        .flex()
        .flex_wrap()
        .items_start()
        .justify_start()
        .children((0..tiles).map(move |ix| {
            let col = ix % layout.cols;
            let row = ix / layout.cols;
            let tile_width = if col + 1 == layout.cols {
                layout.last_tile_width
            } else {
                layout.tile_size
            };
            let tile_height = if row + 1 == layout.rows {
                layout.last_tile_height
            } else {
                layout.tile_size
            };

            div()
                .w(tile_width)
                .h(tile_height)
                .flex_none()
                .overflow_hidden()
                .child(
                    img(ImageSource::Resource(Resource::Embedded(
                        GLASS_NOISE_ASSET_PATH.into(),
                    )))
                    .w(layout.tile_size)
                    .h(layout.tile_size)
                    .flex_none()
                    .object_fit(ObjectFit::Fill)
                    .opacity(opacity),
                )
        }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_surface_preset_builder() {
        let panel = SurfacePreset::panel();
        assert_eq!(panel.elevation, ElevationToken::Lg);
        assert!(panel.use_theme_elevation_defaults);

        let overridden = panel.with_elevation(ElevationToken::Sm);
        assert_eq!(overridden.elevation, ElevationToken::Sm);
        assert!(!overridden.use_theme_elevation_defaults);
    }

    #[test]
    fn noise_tile_layout_handles_exact_multiples() {
        let layout = noise_tile_layout(px(256.0), px(128.0), 1.0);

        assert_eq!(layout.cols, 2);
        assert_eq!(layout.rows, 1);
        assert_eq!(layout.last_tile_width, px(128.0));
        assert_eq!(layout.last_tile_height, px(128.0));
    }

    #[test]
    fn noise_tile_layout_handles_partial_edges() {
        let layout = noise_tile_layout(px(200.0), px(150.0), 1.0);

        assert_eq!(layout.cols, 2);
        assert_eq!(layout.rows, 2);
        assert_eq!(layout.last_tile_width, px(72.0));
        assert_eq!(layout.last_tile_height, px(22.0));
    }

    #[test]
    fn noise_tile_layout_handles_dimensions_smaller_than_a_tile() {
        let layout = noise_tile_layout(px(40.0), px(60.0), 1.0);

        assert_eq!(layout.cols, 1);
        assert_eq!(layout.rows, 1);
        assert_eq!(layout.last_tile_width, px(40.0));
        assert_eq!(layout.last_tile_height, px(60.0));
    }

    #[test]
    fn noise_tile_layout_clamps_tile_size_at_extreme_scale_factors() {
        let layout = noise_tile_layout(px(3.0), px(2.0), 512.0);

        assert_eq!(layout.tile_size, px(1.0));
        assert_eq!(layout.cols, 3);
        assert_eq!(layout.rows, 2);
        assert_eq!(layout.last_tile_width, px(1.0));
        assert_eq!(layout.last_tile_height, px(1.0));
    }

    #[test]
    fn noise_tile_layout_handles_zero_dimensions() {
        let layout = noise_tile_layout(px(0.0), px(0.0), 1.0);

        assert_eq!(layout.cols, 0);
        assert_eq!(layout.rows, 0);
        assert_eq!(layout.last_tile_width, px(0.0));
        assert_eq!(layout.last_tile_height, px(0.0));
    }
}
