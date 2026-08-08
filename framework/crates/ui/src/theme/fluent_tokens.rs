use gpui::{Hsla, px};

use crate::{ThemeElevation, ThemeMaterial, ThemeMotion, ThemeShadowToken, try_parse_color};

pub(crate) fn theme_motion_defaults() -> ThemeMotion {
    ThemeMotion {
        // Fluent cadence: 83 fade / 167 dismiss / 187 invoke / 667 emphasis.
        fade_duration_ms: 83,
        exit_duration_ms: 167,
        enter_duration_ms: 187,
        emphasis_duration_ms: 667,
        // One spring for transform reveals; settles within the enter window.
        spring_damping_ratio: 0.78,
        spring_frequency: 2.0,
        decelerate_easing: "cubic-bezier(0, 0, 0, 1)".into(),
        standard_easing: "cubic-bezier(0.55, 0.55, 0, 1)".into(),
        emphasis_easing: "cubic-bezier(0.13, 1.62, 0, 0.92)".into(),
        fade_easing: "linear".into(),
    }
}

pub(crate) fn theme_elevation_defaults() -> ThemeElevation {
    ThemeElevation {
        // Fluent elevation levels
        control_level: 2,
        card_rest_level: 8,
        tooltip_level: 16,
        flyout_level: 32,
        dialog_level: 128,
        shell_level: 36,
        inactive_window_level: 64,
        active_window_level: 128,
        surface_flyout_shadow: ThemeShadowToken::Md,
        surface_panel_shadow: ThemeShadowToken::Lg,
        surface_card_shadow: ThemeShadowToken::Sm,
    }
}

pub(crate) fn theme_material_defaults() -> ThemeMaterial {
    ThemeMaterial {
        flyout_blur_radius: px(24.0),
        panel_blur_radius: px(48.0),
        flyout_light_opacity: 0.86,
        flyout_dark_opacity: 0.88,
        panel_light_opacity: 0.88,
        panel_dark_opacity: 0.90,
        card_light_opacity: 0.70,
        card_dark_opacity: 0.05,
        subtle_stroke_light_opacity: 0.5,
        subtle_stroke_dark_opacity: 0.5,

        // Fluent layering + material palette tokens
        smoke_light: fluent_color("#0000004D"),
        smoke_dark: fluent_color("#0000004D"),
        layer_light: fluent_color("#FFFFFF80"),
        layer_dark: fluent_color("#3A3A3A4C"),
        layer_alt_light: fluent_color("#FFFFFFFF"),
        layer_alt_dark: fluent_color("#FFFFFF0D"),
    }
}

fn fluent_color(value: &str) -> Hsla {
    try_parse_color(value).unwrap_or_else(|_| gpui::transparent_black())
}
