use gpui::{BoxShadow, hsla, point, px};
use smallvec::SmallVec;

use crate::ThemeElevation;

impl ThemeElevation {
    /// Compute Fluent-style box shadows for a given elevation level.
    ///
    /// Returns up to 2 shadows (directional + ambient) based on the Fluent elevation equations:
    /// - Level 0-2: no shadow (stroke only)
    /// - Level 3-32: directional only (blur=0.5n, y=0.25n)
    /// - Level >=33: directional + ambient (ambient: blur=0.167n, y=2)
    /// - Level 128 (active window): special opacities
    pub fn computed_shadow(&self, level: usize, is_dark: bool) -> SmallVec<[BoxShadow; 2]> {
        let mut shadows = SmallVec::new();

        if level <= 2 {
            return shadows;
        }

        let n = level as f32;

        // Directional shadow
        let dir_blur = 0.5 * n;
        let dir_y = 0.25 * n;
        let dir_opacity = if level == 128 {
            if is_dark { 0.56 } else { 0.28 }
        } else if level >= 33 {
            if is_dark { 0.37 } else { 0.19 }
        } else if is_dark {
            0.26
        } else {
            ((n + 6.0) / 100.0).min(0.14)
        };

        shadows.push(BoxShadow {
            offset: point(px(0.), px(dir_y)),
            blur_radius: px(dir_blur),
            spread_radius: px(0.),
            color: hsla(0., 0., 0., dir_opacity),
            inset: false,
        });

        // Ambient shadow (high elevations only)
        if level >= 33 {
            let amb_blur = 0.167 * n;
            let amb_opacity = if level == 128 {
                if is_dark { 0.55 } else { 0.22 }
            } else if is_dark {
                0.37
            } else {
                0.15
            };

            shadows.push(BoxShadow {
                offset: point(px(0.), px(2.)),
                blur_radius: px(amb_blur),
                spread_radius: px(0.),
                color: hsla(0., 0., 0., amb_opacity),
                inset: false,
            });
        }

        shadows
    }
}
