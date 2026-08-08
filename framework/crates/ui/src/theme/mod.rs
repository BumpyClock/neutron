use crate::{
    highlighter::HighlightTheme, list::ListSettings, notification::NotificationSettings,
    scroll::ScrollbarShow, sheet::SheetSettings,
};
use gpui::{App, Global, Hsla, Pixels, SharedString, Window, WindowAppearance, px};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    ops::{Deref, DerefMut},
    rc::Rc,
    sync::Arc,
};

mod color;
pub(crate) mod contrast;
mod elevation;
mod fluent_tokens;
mod registry;
mod schema;
#[cfg(test)]
mod tests;
mod theme_color;
mod typography;

pub use color::*;
pub use registry::*;
pub use schema::*;
pub use theme_color::*;
pub use typography::*;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ThemeShadowToken {
    #[default]
    None,
    Xs,
    Sm,
    Md,
    Lg,
    Xl,
}

/// Motion tokens: four durations, one spring, four easing curves.
///
/// Every animated surface draws from this set so the system moves as one:
///
/// - `fade` (83ms) — micro state changes: tooltips, carets, tab strips.
/// - `exit` (167ms) — every dismiss. Presence close windows and close animations
///   share this token, so an exit can never outlive its element.
/// - `enter` (187ms) — every reveal. Also the settle window for the spring, so a
///   fade and its transform partner end together.
/// - `emphasis` (667ms, overshoot) — attention accents (badges), used sparingly.
///
/// One spring (`spring_damping_ratio`/`spring_frequency`) drives all transform
/// reveals; a mild/medium split existed before but measured within a few frames
/// of each other at 60Hz and collapsed into this single token.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ThemeMotion {
    pub fade_duration_ms: u16,
    pub exit_duration_ms: u16,
    pub enter_duration_ms: u16,
    pub emphasis_duration_ms: u16,
    pub spring_damping_ratio: f32,
    pub spring_frequency: f32,
    /// Fast-out curve for enters: most of the travel happens immediately.
    pub decelerate_easing: SharedString,
    /// Symmetric curve for point-to-point moves (switches, progress, widths).
    pub standard_easing: SharedString,
    /// Overshoot curve for emphasis accents.
    pub emphasis_easing: SharedString,
    pub fade_easing: SharedString,
}

impl Default for ThemeMotion {
    fn default() -> Self {
        fluent_tokens::theme_motion_defaults()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ThemeElevation {
    pub control_level: usize,
    pub card_rest_level: usize,
    pub tooltip_level: usize,
    pub flyout_level: usize,
    pub dialog_level: usize,
    pub shell_level: usize,
    pub inactive_window_level: usize,
    pub active_window_level: usize,
    pub surface_flyout_shadow: ThemeShadowToken,
    pub surface_panel_shadow: ThemeShadowToken,
    pub surface_card_shadow: ThemeShadowToken,
}

impl Default for ThemeElevation {
    fn default() -> Self {
        fluent_tokens::theme_elevation_defaults()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ThemeMaterial {
    pub flyout_blur_radius: Pixels,
    pub panel_blur_radius: Pixels,
    pub flyout_light_opacity: f32,
    pub flyout_dark_opacity: f32,
    pub panel_light_opacity: f32,
    pub panel_dark_opacity: f32,
    pub card_light_opacity: f32,
    pub card_dark_opacity: f32,
    pub subtle_stroke_light_opacity: f32,
    pub subtle_stroke_dark_opacity: f32,
    pub smoke_light: Hsla,
    pub smoke_dark: Hsla,
    pub layer_light: Hsla,
    pub layer_dark: Hsla,
    pub layer_alt_light: Hsla,
    pub layer_alt_dark: Hsla,
}

impl Default for ThemeMaterial {
    fn default() -> Self {
        fluent_tokens::theme_material_defaults()
    }
}

pub fn init(cx: &mut App) {
    registry::init(cx);

    Theme::sync_system_appearance(None, cx);
    Theme::sync_scrollbar_appearance(cx);
}

pub trait ActiveTheme {
    fn theme(&self) -> &Theme;
}

impl ActiveTheme for App {
    #[inline(always)]
    fn theme(&self) -> &Theme {
        Theme::global(self)
    }
}

/// The global theme configuration.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Theme {
    pub colors: ThemeColor,
    pub motion: ThemeMotion,
    pub elevation: ThemeElevation,
    pub material: ThemeMaterial,
    pub typography: ThemeTypography,
    pub highlight_theme: Arc<HighlightTheme>,
    pub light_theme: Rc<ThemeConfig>,
    pub dark_theme: Rc<ThemeConfig>,

    pub mode: ThemeMode,
    pub mode_preference: ThemeModePreference,
    pub theme_set_name: SharedString,
    /// The font family for the application, default is `.SystemUIFont`.
    pub font_family: SharedString,
    /// The base font size for the application, default is 16px.
    pub font_size: Pixels,
    /// The monospace font family for the application.
    ///
    /// Defaults to:
    ///
    /// - macOS: `Menlo`
    /// - Windows: `Consolas`
    /// - Linux: `DejaVu Sans Mono`
    pub mono_font_family: SharedString,
    /// The monospace font size for the application, default is 13px.
    pub mono_font_size: Pixels,
    /// Radius for the general elements.
    pub radius: Pixels,
    /// Radius for the large elements, e.g.: Dialog, Notification border radius.
    pub radius_lg: Pixels,
    pub shadow: bool,
    pub transparent: Hsla,
    /// Show the scrollbar mode, default: Scrolling
    pub scrollbar_show: ScrollbarShow,
    /// The notification setting.
    pub notification: NotificationSettings,
    /// Tile grid size, default is 4px.
    pub tile_grid_size: Pixels,
    /// The shadow of the tile panel.
    pub tile_shadow: bool,
    /// The border radius of the tile panel, default is 0px.
    pub tile_radius: Pixels,
    /// The list settings.
    pub list: ListSettings,
    /// The sheet settings.
    pub sheet: SheetSettings,
}

impl Default for Theme {
    fn default() -> Self {
        Self::from(&ThemeColor::default())
    }
}

impl Deref for Theme {
    type Target = ThemeColor;

    fn deref(&self) -> &Self::Target {
        &self.colors
    }
}

impl DerefMut for Theme {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.colors
    }
}

impl Global for Theme {}

impl Theme {
    /// Returns the global theme reference
    #[inline(always)]
    pub fn global(cx: &App) -> &Theme {
        cx.global::<Theme>()
    }

    /// Returns the global theme mutable reference
    #[inline(always)]
    pub fn global_mut(cx: &mut App) -> &mut Theme {
        cx.global_mut::<Theme>()
    }

    /// Returns true if the theme is dark.
    #[inline(always)]
    pub fn is_dark(&self) -> bool {
        self.mode.is_dark()
    }

    /// Returns the current theme name.
    pub fn theme_name(&self) -> &SharedString {
        if self.is_dark() {
            &self.dark_theme.name
        } else {
            &self.light_theme.name
        }
    }

    /// Sync the theme with the system appearance
    pub fn sync_system_appearance(window: Option<&mut Window>, cx: &mut App) {
        let appearance = window
            .as_ref()
            .map(|window| window.appearance())
            .unwrap_or_else(|| cx.window_appearance());

        let mode = if cx.has_global::<Theme>() {
            match cx.global::<Theme>().mode_preference {
                ThemeModePreference::Light => ThemeMode::Light,
                ThemeModePreference::Dark => ThemeMode::Dark,
                ThemeModePreference::System => appearance.into(),
            }
        } else {
            appearance.into()
        };

        Self::change(mode, window, cx);
    }

    /// Apply a theme set with the given mode preference.
    pub fn apply_theme_set(
        set: &ThemeSetEntry,
        preference: ThemeModePreference,
        window: Option<&mut Window>,
        cx: &mut App,
    ) {
        let appearance = window
            .as_ref()
            .map(|w| w.appearance())
            .unwrap_or_else(|| cx.window_appearance());
        let system_mode: ThemeMode = appearance.into();

        let Some(resolved) = ThemeRegistry::resolve_theme(set, preference, system_mode) else {
            tracing::warn!(
                "ThemeSetEntry '{}' has neither light nor dark variant, falling back to defaults",
                set.name
            );
            return;
        };

        if !cx.has_global::<Theme>() {
            let mut theme = Theme::default();
            theme.light_theme = ThemeRegistry::global(cx).default_light_theme().clone();
            theme.dark_theme = ThemeRegistry::global(cx).default_dark_theme().clone();
            cx.set_global(theme);
        }

        let theme = cx.global_mut::<Theme>();
        theme.theme_set_name = set.name.clone();
        theme.mode_preference = preference;

        if let Some(light) = &set.light {
            theme.light_theme = light.clone();
        }
        if let Some(dark) = &set.dark {
            theme.dark_theme = dark.clone();
        }

        theme.apply_config(resolved);

        if let Some(window) = window {
            window.refresh();
        }
    }

    /// Sync the Scrollbar showing behavior with the system
    pub fn sync_scrollbar_appearance(cx: &mut App) {
        Theme::global_mut(cx).scrollbar_show = if cx.should_auto_hide_scrollbars() {
            ScrollbarShow::Scrolling
        } else {
            ScrollbarShow::Hover
        };
    }

    /// Change the theme mode.
    pub fn change(mode: impl Into<ThemeMode>, window: Option<&mut Window>, cx: &mut App) {
        let mode = mode.into();
        if !cx.has_global::<Theme>() {
            let mut theme = Theme::default();
            theme.light_theme = ThemeRegistry::global(cx).default_light_theme().clone();
            theme.dark_theme = ThemeRegistry::global(cx).default_dark_theme().clone();
            cx.set_global(theme);
        }

        let theme = cx.global_mut::<Theme>();
        theme.mode = mode;
        if mode.is_dark() {
            theme.apply_config(&theme.dark_theme.clone());
        } else {
            theme.apply_config(&theme.light_theme.clone());
        }

        if let Some(window) = window {
            window.refresh();
        }
    }

    /// Get the editor background color, if not set, use the theme background color.
    #[inline]
    pub(crate) fn editor_background(&self) -> Hsla {
        self.highlight_theme
            .style
            .editor_background
            .unwrap_or(self.background)
    }
}

impl From<&ThemeColor> for Theme {
    fn from(colors: &ThemeColor) -> Self {
        Theme {
            mode: ThemeMode::default(),
            mode_preference: ThemeModePreference::default(),
            theme_set_name: "Default".into(),
            transparent: Hsla::transparent_black(),
            font_family: ".SystemUIFont".into(),
            font_size: px(16.),
            mono_font_family: if cfg!(target_os = "macos") {
                // https://en.wikipedia.org/wiki/Menlo_(typeface)
                "Menlo".into()
            } else if cfg!(target_os = "windows") {
                "Consolas".into()
            } else {
                "DejaVu Sans Mono".into()
            },
            mono_font_size: px(13.),
            radius: px(6.),
            radius_lg: px(8.),
            shadow: true,
            scrollbar_show: ScrollbarShow::default(),
            notification: NotificationSettings::default(),
            tile_grid_size: px(8.),
            tile_shadow: true,
            tile_radius: px(0.),
            list: ListSettings::default(),
            colors: *colors,
            motion: ThemeMotion::default(),
            elevation: ThemeElevation::default(),
            material: ThemeMaterial::default(),
            typography: ThemeTypography::default(),
            light_theme: Rc::new(ThemeConfig::default()),
            dark_theme: Rc::new(ThemeConfig::default()),
            highlight_theme: HighlightTheme::default_light(),
            sheet: SheetSettings::default(),
        }
    }
}

#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    PartialOrd,
    Eq,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ThemeMode {
    #[default]
    Light,
    Dark,
}

impl ThemeMode {
    #[inline(always)]
    pub fn is_dark(&self) -> bool {
        matches!(self, Self::Dark)
    }

    /// Return lower_case theme name: `light`, `dark`.
    pub fn name(&self) -> &'static str {
        match self {
            ThemeMode::Light => "light",
            ThemeMode::Dark => "dark",
        }
    }
}

impl From<WindowAppearance> for ThemeMode {
    fn from(appearance: WindowAppearance) -> Self {
        match appearance {
            WindowAppearance::Dark | WindowAppearance::VibrantDark => Self::Dark,
            WindowAppearance::Light | WindowAppearance::VibrantLight => Self::Light,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ThemeModePreference {
    #[default]
    System,
    Light,
    Dark,
}
