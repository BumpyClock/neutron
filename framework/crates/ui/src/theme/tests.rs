use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use gpui::Hsla;

use super::{
    Colorize as _, ThemeColor, ThemeConfig, ThemeMaterial, ThemeSet,
    contrast::{MIN_TEXT_CONTRAST, contrast_adjusted, contrast_ratio},
    try_parse_color,
};

const MIN_CHART_CONTRAST: f32 = 3.;
const MIN_CHART_COLOR_DISTANCE: f32 = 0.05;
const MIN_INTERACTION_STATE_DISTANCE: f32 = 0.02;

fn bundled_theme_configs() -> Vec<(PathBuf, ThemeConfig)> {
    let themes_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../themes");
    let mut paths = fs::read_dir(&themes_dir)
        .expect("bundled themes directory should be readable")
        .map(|entry| {
            entry
                .expect("theme directory entry should be readable")
                .path()
        })
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect::<Vec<_>>();
    paths.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/theme/default-theme.json"));
    paths.sort();

    paths
        .into_iter()
        .flat_map(|path| {
            let source = fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
            let set: ThemeSet = serde_json::from_str(&source)
                .unwrap_or_else(|error| panic!("failed to parse {}: {error}", path.display()));
            set.themes
                .into_iter()
                .map(move |config| (path.clone(), config))
        })
        .collect()
}

fn resolve_colors(config: &ThemeConfig) -> ThemeColor {
    let defaults = if config.mode.is_dark() {
        ThemeColor::dark()
    } else {
        ThemeColor::light()
    };
    let mut colors = ThemeColor::default();
    colors.apply_config(config, defaults.as_ref());
    colors
}

fn hsla_to_rgba(color: Hsla) -> [f32; 4] {
    let chroma = (1. - (2. * color.l - 1.).abs()) * color.s;
    let hue = color.h * 6.;
    let secondary = chroma * (1. - (hue.rem_euclid(2.) - 1.).abs());
    let (red, green, blue) = match hue as usize {
        0 => (chroma, secondary, 0.),
        1 => (secondary, chroma, 0.),
        2 => (0., chroma, secondary),
        3 => (0., secondary, chroma),
        4 => (secondary, 0., chroma),
        _ => (chroma, 0., secondary),
    };
    let match_lightness = color.l - chroma / 2.;
    [
        red + match_lightness,
        green + match_lightness,
        blue + match_lightness,
        color.a,
    ]
}

fn composite(foreground: [f32; 4], background: [f32; 4]) -> [f32; 4] {
    let alpha = foreground[3] + background[3] * (1. - foreground[3]);
    if alpha == 0. {
        return [0.; 4];
    }
    let channel = |index| {
        (foreground[index] * foreground[3]
            + background[index] * background[3] * (1. - foreground[3]))
            / alpha
    };
    [channel(0), channel(1), channel(2), alpha]
}

fn linear_srgb(channel: f32) -> f32 {
    if channel <= 0.04045 {
        channel / 12.92
    } else {
        ((channel + 0.055) / 1.055).powf(2.4)
    }
}

fn oklab(color: Hsla, background: Hsla) -> [f32; 3] {
    let color = composite(hsla_to_rgba(color), hsla_to_rgba(background));
    let red = linear_srgb(color[0]);
    let green = linear_srgb(color[1]);
    let blue = linear_srgb(color[2]);
    let lightness = (0.412_221_46 * red + 0.536_332_55 * green + 0.051_445_995 * blue).cbrt();
    let medium = (0.211_903_5 * red + 0.680_699_5 * green + 0.107_396_96 * blue).cbrt();
    let short = (0.088_302_46 * red + 0.281_718_85 * green + 0.629_978_7 * blue).cbrt();
    [
        0.210_454_26 * lightness + 0.793_617_8 * medium - 0.004_072_047 * short,
        1.977_998_5 * lightness - 2.428_592_2 * medium + 0.450_593_7 * short,
        0.025_904_037 * lightness + 0.782_771_77 * medium - 0.808_675_77 * short,
    ]
}

fn oklab_distance(left: Hsla, right: Hsla, background: Hsla) -> f32 {
    let left = oklab(left, background);
    let right = oklab(right, background);
    ((left[0] - right[0]).powi(2) + (left[1] - right[1]).powi(2) + (left[2] - right[2]).powi(2))
        .sqrt()
}

#[test]
fn bundled_themes_meet_text_contrast_floor() {
    let mut failures = Vec::new();

    for (path, config) in bundled_theme_configs() {
        let colors = resolve_colors(&config);
        let pairs = [
            ("foreground", colors.foreground, colors.background),
            ("muted", colors.muted_foreground, colors.background),
            ("primary", colors.primary_foreground, colors.primary),
            (
                "primary hover",
                colors.primary_foreground,
                colors.primary_hover,
            ),
            (
                "primary active",
                colors.primary_foreground,
                colors.primary_active,
            ),
            ("secondary", colors.secondary_foreground, colors.secondary),
            (
                "secondary hover",
                colors.secondary_foreground,
                colors.secondary_hover,
            ),
            (
                "secondary active",
                colors.secondary_foreground,
                colors.secondary_active,
            ),
            ("danger", colors.danger_foreground, colors.danger),
            (
                "danger hover",
                colors.danger_foreground,
                colors.danger_hover,
            ),
            (
                "danger active",
                colors.danger_foreground,
                colors.danger_active,
            ),
            ("success", colors.success_foreground, colors.success),
            (
                "success hover",
                colors.success_foreground,
                colors.success_hover,
            ),
            (
                "success active",
                colors.success_foreground,
                colors.success_active,
            ),
            ("warning", colors.warning_foreground, colors.warning),
            (
                "warning hover",
                colors.warning_foreground,
                colors.warning_hover,
            ),
            (
                "warning active",
                colors.warning_foreground,
                colors.warning_active,
            ),
            ("info", colors.info_foreground, colors.info),
            ("info hover", colors.info_foreground, colors.info_hover),
            ("info active", colors.info_foreground, colors.info_active),
        ];

        for (name, foreground, surface) in pairs {
            let ratio = contrast_ratio(foreground, surface, colors.background);
            if !ratio.is_finite() || ratio < MIN_TEXT_CONTRAST {
                failures.push(format!(
                    "{} / {} / {name}: {ratio:.2}:1",
                    path.display(),
                    config.name
                ));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "bundled theme contrast failures:\n{}",
        failures.join("\n")
    );
}

/// The flyout material composited over `background`, as
/// [`crate::flyout_material_color`] resolves it at runtime. Derives the base color
/// through [`crate::surface::flyout_base_color`] — the same function the renderer
/// uses — so these tests measure the surface that is actually painted.
fn flyout_material(config: &ThemeConfig, colors: &ThemeColor) -> Hsla {
    let mut material = ThemeMaterial::default();
    material.apply_config(config.material.as_ref(), &ThemeMaterial::default());

    let is_dark = config.mode.is_dark();
    let opacity = if is_dark {
        material.flyout_dark_opacity
    } else {
        material.flyout_light_opacity
    };
    crate::surface::flyout_base_color(colors.popover, is_dark).opacity(opacity)
}

/// Flyout materials (popover, menu, select popup, command palette, editor popovers)
/// sit *above* the window background and are lighter than it in dark mode, so text
/// on a flyout has less contrast than the same text on `background`. This checks the
/// two text roles used inside flyouts against the flyout material itself rather than
/// against `background`, which is what [`bundled_themes_meet_text_contrast_floor`]
/// covers.
///
/// Both text roles are the corrected ones the runtime paints:
/// [`crate::flyout_primary_foreground`] (usually `popover.foreground` untouched;
/// corrected only in deliberately dim themes such as Alduin) and
/// [`crate::flyout_secondary_foreground`], which corrects `muted.foreground` —
/// raw, that token is sub-AA on the flyout material in every bundled dark theme.
#[test]
fn bundled_themes_meet_flyout_text_contrast_floor() {
    let mut failures = Vec::new();

    for (path, config) in bundled_theme_configs() {
        let colors = resolve_colors(&config);
        let flyout = flyout_material(&config, &colors);

        for (name, foreground) in [
            (
                "flyout label",
                contrast_adjusted(
                    colors.popover_foreground,
                    flyout,
                    colors.background,
                    MIN_TEXT_CONTRAST,
                ),
            ),
            (
                "flyout secondary",
                contrast_adjusted(
                    colors.muted_foreground,
                    flyout,
                    colors.background,
                    MIN_TEXT_CONTRAST,
                ),
            ),
        ] {
            let ratio = contrast_ratio(foreground, flyout, colors.background);
            if !ratio.is_finite() || ratio < MIN_TEXT_CONTRAST {
                failures.push(format!(
                    "{} / {} / {name}: {ratio:.2}:1",
                    path.display(),
                    config.name
                ));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "bundled theme flyout contrast failures:\n{}",
        failures.join("\n")
    );
}

/// Pins *why* [`crate::flyout_secondary_foreground`] corrects rather than aliasing
/// `muted.foreground`: on dark materials the aliased token is routinely sub-AA, and
/// the correction both fixes those and leaves already-readable themes untouched.
///
/// Note it is not every dark theme — Catppuccin Macchiato and macOS Classic Dark
/// already clear the floor on their own flyout materials and are returned unchanged.
#[test]
fn bundled_theme_flyout_secondary_corrects_only_where_needed() {
    let mut dark_needing_correction = 0;

    for (path, config) in bundled_theme_configs() {
        let colors = resolve_colors(&config);
        let flyout = flyout_material(&config, &colors);
        let corrected = contrast_adjusted(
            colors.muted_foreground,
            flyout,
            colors.background,
            MIN_TEXT_CONTRAST,
        );

        if corrected == colors.muted_foreground {
            // Left alone only when it already passes.
            assert!(
                contrast_ratio(colors.muted_foreground, flyout, colors.background)
                    >= MIN_TEXT_CONTRAST,
                "{} / {}: sub-AA secondary was left uncorrected",
                path.display(),
                config.name
            );
            continue;
        }

        // Corrections move away from the material: lighter on dark, darker on light.
        if config.mode.is_dark() {
            assert!(
                corrected.l > colors.muted_foreground.l,
                "{} / {}: correction should lighten on a dark material",
                path.display(),
                config.name
            );
            dark_needing_correction += 1;
        } else {
            assert!(
                corrected.l < colors.muted_foreground.l,
                "{} / {}: correction should darken on a light material",
                path.display(),
                config.name
            );
        }
    }

    assert!(
        dark_needing_correction > 0,
        "no dark theme needed a correction, so the role is an alias and can be removed"
    );
}

#[test]
fn default_theme_semantic_active_states_remain_distinct() {
    let (_, config) = bundled_theme_configs()
        .into_iter()
        .find(|(_, config)| config.name == "Default Light")
        .expect("default light theme should exist");
    let colors = resolve_colors(&config);

    for (name, normal, active) in [
        ("danger", colors.danger, colors.danger_active),
        ("info", colors.info, colors.info_active),
        ("success", colors.success, colors.success_active),
        ("warning", colors.warning, colors.warning_active),
    ] {
        assert_ne!(normal, active, "default light {name} active state");
    }
}

#[test]
fn contrast_adjusted_interaction_states_remain_distinct() {
    type ColorGetter = fn(&ThemeColor) -> Hsla;

    let cases: [(&str, &str, [ColorGetter; 3]); 34] = [
        (
            "Adventure",
            "primary",
            [|c| c.primary, |c| c.primary_hover, |c| c.primary_active],
        ),
        (
            "Adventure",
            "danger",
            [|c| c.danger, |c| c.danger_hover, |c| c.danger_active],
        ),
        (
            "Adventure",
            "warning",
            [|c| c.warning, |c| c.warning_hover, |c| c.warning_active],
        ),
        (
            "Adventure Time",
            "primary",
            [|c| c.primary, |c| c.primary_hover, |c| c.primary_active],
        ),
        (
            "Adventure Time",
            "success",
            [|c| c.success, |c| c.success_hover, |c| c.success_active],
        ),
        (
            "Catppuccin Latte",
            "danger",
            [|c| c.danger, |c| c.danger_hover, |c| c.danger_active],
        ),
        (
            "Catppuccin Macchiato",
            "primary",
            [|c| c.primary, |c| c.primary_hover, |c| c.primary_active],
        ),
        (
            "Everforest Light",
            "secondary",
            [
                |c| c.secondary,
                |c| c.secondary_hover,
                |c| c.secondary_active,
            ],
        ),
        (
            "Everforest Dark",
            "primary",
            [|c| c.primary, |c| c.primary_hover, |c| c.primary_active],
        ),
        (
            "Alduin",
            "primary",
            [|c| c.primary, |c| c.primary_hover, |c| c.primary_active],
        ),
        (
            "Everforest Dark",
            "secondary",
            [
                |c| c.secondary,
                |c| c.secondary_hover,
                |c| c.secondary_active,
            ],
        ),
        (
            "Flexoki Light",
            "danger",
            [|c| c.danger, |c| c.danger_hover, |c| c.danger_active],
        ),
        (
            "Flexoki Light",
            "info",
            [|c| c.info, |c| c.info_hover, |c| c.info_active],
        ),
        (
            "Flexoki Dark",
            "success",
            [|c| c.success, |c| c.success_hover, |c| c.success_active],
        ),
        (
            "Gruvbox Light",
            "info",
            [|c| c.info, |c| c.info_hover, |c| c.info_active],
        ),
        (
            "Gruvbox Dark",
            "info",
            [|c| c.info, |c| c.info_hover, |c| c.info_active],
        ),
        (
            "Gruvbox Dark",
            "primary",
            [|c| c.primary, |c| c.primary_hover, |c| c.primary_active],
        ),
        (
            "Hybrid Light",
            "warning",
            [|c| c.warning, |c| c.warning_hover, |c| c.warning_active],
        ),
        (
            "Hybrid Dark",
            "warning",
            [|c| c.warning, |c| c.warning_hover, |c| c.warning_active],
        ),
        (
            "Hybrid Dark",
            "success",
            [|c| c.success, |c| c.success_hover, |c| c.success_active],
        ),
        (
            "Jellybeans",
            "info",
            [|c| c.info, |c| c.info_hover, |c| c.info_active],
        ),
        (
            "Kibble",
            "success",
            [|c| c.success, |c| c.success_hover, |c| c.success_active],
        ),
        (
            "Mellifluous Light",
            "primary",
            [|c| c.primary, |c| c.primary_hover, |c| c.primary_active],
        ),
        (
            "Mellifluous Light",
            "success",
            [|c| c.success, |c| c.success_hover, |c| c.success_active],
        ),
        (
            "Mellifluous Dark",
            "success",
            [|c| c.success, |c| c.success_hover, |c| c.success_active],
        ),
        (
            "macOS Classic Dark",
            "primary",
            [|c| c.primary, |c| c.primary_hover, |c| c.primary_active],
        ),
        (
            "Solarized Light",
            "success",
            [|c| c.success, |c| c.success_hover, |c| c.success_active],
        ),
        (
            "Solarized Light",
            "warning",
            [|c| c.warning, |c| c.warning_hover, |c| c.warning_active],
        ),
        (
            "Solarized Light",
            "info",
            [|c| c.info, |c| c.info_hover, |c| c.info_active],
        ),
        (
            "Spaceduck",
            "primary",
            [|c| c.primary, |c| c.primary_hover, |c| c.primary_active],
        ),
        (
            "Tokyo Storm",
            "primary",
            [|c| c.primary, |c| c.primary_hover, |c| c.primary_active],
        ),
        (
            "Tokyo Moon",
            "primary",
            [|c| c.primary, |c| c.primary_hover, |c| c.primary_active],
        ),
        (
            "Twilight",
            "danger",
            [|c| c.danger, |c| c.danger_hover, |c| c.danger_active],
        ),
        (
            "Twilight",
            "info",
            [|c| c.info, |c| c.info_hover, |c| c.info_active],
        ),
    ];
    let configs = bundled_theme_configs();

    for (theme_name, state_name, getters) in cases {
        let (path, config) = configs
            .iter()
            .find(|(_, config)| config.name == theme_name)
            .unwrap_or_else(|| panic!("bundled theme {theme_name} should exist"));
        let colors = resolve_colors(config);
        let states = getters.map(|get| get(&colors));

        for left in 0..states.len() {
            for right in left + 1..states.len() {
                let distance = oklab_distance(states[left], states[right], colors.background);
                assert!(
                    distance >= MIN_INTERACTION_STATE_DISTANCE,
                    "{} / {theme_name} / {state_name} states {left} and {right}: OKLab distance {distance:.3}",
                    path.display()
                );
            }
        }
    }
}

#[test]
fn bundled_themes_define_distinct_chart_palettes() {
    let mut failures = Vec::new();

    for (path, config) in bundled_theme_configs() {
        let configured = [
            config.colors.chart_1.as_ref(),
            config.colors.chart_2.as_ref(),
            config.colors.chart_3.as_ref(),
            config.colors.chart_4.as_ref(),
            config.colors.chart_5.as_ref(),
        ];
        if configured.iter().any(|color| color.is_none()) {
            failures.push(format!(
                "{} / {}: missing chart.1 through chart.5",
                path.display(),
                config.name
            ));
            continue;
        }
        if let Some(invalid) = configured
            .iter()
            .flatten()
            .find(|color| try_parse_color(color).is_err())
        {
            failures.push(format!(
                "{} / {}: invalid chart color {invalid}",
                path.display(),
                config.name
            ));
            continue;
        }

        let colors = resolve_colors(&config);
        let palette = [
            colors.chart_1,
            colors.chart_2,
            colors.chart_3,
            colors.chart_4,
            colors.chart_5,
        ];
        let hex = palette.map(|color| color.to_hex());
        if hex.iter().collect::<BTreeSet<_>>().len() != hex.len() {
            failures.push(format!(
                "{} / {}: duplicate chart colors {hex:?}",
                path.display(),
                config.name
            ));
        }

        for (index, color) in palette.iter().enumerate() {
            let ratio = contrast_ratio(*color, colors.background, colors.background);
            if !ratio.is_finite() || ratio < MIN_CHART_CONTRAST {
                failures.push(format!(
                    "{} / {} / chart.{}: {ratio:.2}:1 against background",
                    path.display(),
                    config.name,
                    index + 1
                ));
            }
        }

        for left in 0..palette.len() {
            for right in left + 1..palette.len() {
                let distance = oklab_distance(palette[left], palette[right], colors.background);
                if !distance.is_finite() || distance < MIN_CHART_COLOR_DISTANCE {
                    failures.push(format!(
                        "{} / {} / chart.{} and chart.{}: OKLab distance {distance:.3}",
                        path.display(),
                        config.name,
                        left + 1,
                        right + 1
                    ));
                }
            }
        }
    }

    assert!(
        failures.is_empty(),
        "bundled theme chart palette failures:\n{}",
        failures.join("\n")
    );
}
