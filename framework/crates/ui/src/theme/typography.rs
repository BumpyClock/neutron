use gpui::{FontWeight, Pixels, px};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A single step in the type ramp.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
pub struct TypeRampToken {
    pub size: Pixels,
    pub line_height: Pixels,
    pub weight: FontWeight,
}

/// Fluent 9-step type ramp.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ThemeTypography {
    pub caption: TypeRampToken,
    pub body: TypeRampToken,
    pub body_strong: TypeRampToken,
    pub body_large: TypeRampToken,
    pub body_large_strong: TypeRampToken,
    pub subtitle: TypeRampToken,
    pub title: TypeRampToken,
    pub title_large: TypeRampToken,
    pub display: TypeRampToken,
}

impl Default for ThemeTypography {
    fn default() -> Self {
        Self {
            caption: TypeRampToken {
                size: px(12.),
                line_height: px(16.),
                weight: FontWeight::NORMAL,
            },
            body: TypeRampToken {
                size: px(14.),
                line_height: px(20.),
                weight: FontWeight::NORMAL,
            },
            body_strong: TypeRampToken {
                size: px(14.),
                line_height: px(20.),
                weight: FontWeight::SEMIBOLD,
            },
            body_large: TypeRampToken {
                size: px(18.),
                line_height: px(24.),
                weight: FontWeight::NORMAL,
            },
            body_large_strong: TypeRampToken {
                size: px(18.),
                line_height: px(24.),
                weight: FontWeight::SEMIBOLD,
            },
            subtitle: TypeRampToken {
                size: px(20.),
                line_height: px(28.),
                weight: FontWeight::SEMIBOLD,
            },
            title: TypeRampToken {
                size: px(28.),
                line_height: px(36.),
                weight: FontWeight::SEMIBOLD,
            },
            title_large: TypeRampToken {
                size: px(40.),
                line_height: px(52.),
                weight: FontWeight::SEMIBOLD,
            },
            display: TypeRampToken {
                size: px(68.),
                line_height: px(92.),
                weight: FontWeight::SEMIBOLD,
            },
        }
    }
}

/// Optional overrides for ThemeTypography in JSON config.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ThemeTypographyConfig {
    pub caption: Option<TypeRampTokenConfig>,
    pub body: Option<TypeRampTokenConfig>,
    pub body_strong: Option<TypeRampTokenConfig>,
    pub body_large: Option<TypeRampTokenConfig>,
    pub body_large_strong: Option<TypeRampTokenConfig>,
    pub subtitle: Option<TypeRampTokenConfig>,
    pub title: Option<TypeRampTokenConfig>,
    pub title_large: Option<TypeRampTokenConfig>,
    pub display: Option<TypeRampTokenConfig>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct TypeRampTokenConfig {
    pub size: Option<f32>,
    pub line_height: Option<f32>,
    pub weight: Option<f32>,
}

impl ThemeTypography {
    pub fn apply_config(&mut self, config: Option<&ThemeTypographyConfig>) {
        let defaults = ThemeTypography::default();
        if let Some(config) = config {
            macro_rules! apply_ramp {
                ($field:ident) => {
                    if let Some(ref cfg) = config.$field {
                        self.$field.size = px(cfg.size.unwrap_or(f32::from(defaults.$field.size)));
                        self.$field.line_height = px(cfg
                            .line_height
                            .unwrap_or(f32::from(defaults.$field.line_height)));
                        self.$field.weight =
                            FontWeight(cfg.weight.unwrap_or(defaults.$field.weight.0));
                    } else {
                        self.$field = defaults.$field;
                    }
                };
            }
            apply_ramp!(caption);
            apply_ramp!(body);
            apply_ramp!(body_strong);
            apply_ramp!(body_large);
            apply_ramp!(body_large_strong);
            apply_ramp!(subtitle);
            apply_ramp!(title);
            apply_ramp!(title_large);
            apply_ramp!(display);
        } else {
            *self = defaults;
        }
    }
}
