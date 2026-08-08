//! WCAG contrast math over theme colors.
//!
//! Theme colors are [`Hsla`] and frequently translucent, so a foreground's real
//! contrast depends on what it is composited over. Every helper here therefore
//! takes three colors: the text, the *surface* it sits on, and the opaque
//! `backdrop` behind that surface (normally [`crate::Theme::background`]).

use gpui::{Hsla, Rgba};

/// WCAG AA floor for body text.
pub(crate) const MIN_TEXT_CONTRAST: f32 = 4.5;

/// Steps used to search for the smallest lightness change that clears a floor.
/// 10 halvings resolve lightness to under 0.1%, below one 8-bit step.
const CONTRAST_SEARCH_STEPS: usize = 10;

fn composite(foreground: Rgba, background: Rgba) -> Rgba {
    let a = foreground.a + background.a * (1. - foreground.a);
    if a == 0. {
        return Rgba {
            r: 0.,
            g: 0.,
            b: 0.,
            a: 0.,
        };
    }
    let channel =
        |fg: f32, bg: f32| (fg * foreground.a + bg * background.a * (1. - foreground.a)) / a;
    Rgba {
        r: channel(foreground.r, background.r),
        g: channel(foreground.g, background.g),
        b: channel(foreground.b, background.b),
        a,
    }
}

fn linear_srgb(channel: f32) -> f32 {
    if channel <= 0.04045 {
        channel / 12.92
    } else {
        ((channel + 0.055) / 1.055).powf(2.4)
    }
}

fn relative_luminance(color: Rgba) -> f32 {
    0.2126 * linear_srgb(color.r) + 0.7152 * linear_srgb(color.g) + 0.0722 * linear_srgb(color.b)
}

/// Contrast ratio of `foreground` against `surface`, with both composited over
/// the opaque `backdrop`.
pub(crate) fn contrast_ratio(foreground: Hsla, surface: Hsla, backdrop: Hsla) -> f32 {
    let backdrop = Rgba::from(backdrop);
    let surface = composite(Rgba::from(surface), backdrop);
    let foreground = composite(Rgba::from(foreground), surface);
    let foreground = relative_luminance(foreground);
    let surface = relative_luminance(surface);
    (foreground.max(surface) + 0.05) / (foreground.min(surface) + 0.05)
}

/// Returns `foreground` if it already clears `min_ratio` against `surface`,
/// otherwise the nearest variant that does.
///
/// Only lightness moves — hue, saturation and alpha are preserved, so a theme's
/// character survives the correction. Contrast is monotonic in lightness once a
/// direction is chosen (away from the surface), so a bisection finds the
/// *smallest* correction rather than clamping to black or white. When even the
/// extreme cannot clear the floor (a mid-luminance surface), the extreme is
/// returned, which is still the most readable option available.
pub(crate) fn contrast_adjusted(
    foreground: Hsla,
    surface: Hsla,
    backdrop: Hsla,
    min_ratio: f32,
) -> Hsla {
    if contrast_ratio(foreground, surface, backdrop) >= min_ratio {
        return foreground;
    }

    let composited_surface = composite(Rgba::from(surface), Rgba::from(backdrop));
    let composited_foreground = composite(Rgba::from(foreground), composited_surface);
    let limit =
        if relative_luminance(composited_foreground) < relative_luminance(composited_surface) {
            0.
        } else {
            1.
        };

    let with_lightness = |l: f32| Hsla { l, ..foreground };
    if contrast_ratio(with_lightness(limit), surface, backdrop) < min_ratio {
        return with_lightness(limit);
    }

    let (mut near, mut far) = (foreground.l, limit);
    let mut best = with_lightness(limit);
    for _ in 0..CONTRAST_SEARCH_STEPS {
        let mid = (near + far) / 2.;
        if contrast_ratio(with_lightness(mid), surface, backdrop) >= min_ratio {
            best = with_lightness(mid);
            far = mid;
        } else {
            near = mid;
        }
    }

    best
}

#[cfg(test)]
mod tests {
    use gpui::hsla;

    use super::*;

    const BLACK: Hsla = Hsla {
        h: 0.,
        s: 0.,
        l: 0.,
        a: 1.,
    };
    const WHITE: Hsla = Hsla {
        h: 0.,
        s: 0.,
        l: 1.,
        a: 1.,
    };

    #[test]
    fn contrast_ratio_matches_wcag_extremes() {
        assert!((contrast_ratio(WHITE, BLACK, BLACK) - 21.).abs() < 0.01);
        assert!((contrast_ratio(BLACK, BLACK, BLACK) - 1.).abs() < 0.01);
    }

    #[test]
    fn contrast_ratio_accounts_for_a_translucent_surface() {
        // A dark surface at 50% over white composites to mid grey, so white text on
        // it loses most of the contrast it has against the surface color alone.
        let translucent = BLACK.opacity(0.5);
        assert!(contrast_ratio(WHITE, translucent, WHITE) < contrast_ratio(WHITE, BLACK, WHITE));
    }

    #[test]
    fn passing_colors_are_returned_unchanged() {
        let text = hsla(0.6, 0.2, 0.9, 1.);
        assert_eq!(
            contrast_adjusted(text, BLACK, BLACK, MIN_TEXT_CONTRAST),
            text
        );
    }

    #[test]
    fn failing_colors_are_lightened_just_enough() {
        // Mid grey on near-black fails; the fix must clear the floor, keep hue and
        // saturation, and stop well short of pure white.
        let surface = hsla(0.6, 0.1, 0.12, 1.);
        let text = hsla(0.6, 0.3, 0.35, 1.);
        assert!(contrast_ratio(text, surface, surface) < MIN_TEXT_CONTRAST);

        let fixed = contrast_adjusted(text, surface, surface, MIN_TEXT_CONTRAST);
        assert!(contrast_ratio(fixed, surface, surface) >= MIN_TEXT_CONTRAST);
        assert_eq!((fixed.h, fixed.s, fixed.a), (text.h, text.s, text.a));
        assert!(fixed.l > text.l, "should lighten against a dark surface");
        assert!(fixed.l < 1., "should not clamp to white");
    }

    #[test]
    fn failing_colors_are_darkened_against_a_light_surface() {
        let surface = hsla(0.1, 0.1, 0.95, 1.);
        let text = hsla(0.1, 0.3, 0.75, 1.);
        let fixed = contrast_adjusted(text, surface, surface, MIN_TEXT_CONTRAST);

        assert!(contrast_ratio(fixed, surface, surface) >= MIN_TEXT_CONTRAST);
        assert!(fixed.l < text.l, "should darken against a light surface");
    }

    #[test]
    fn unreachable_floors_fall_back_to_the_readable_extreme() {
        // Correction only moves away from the surface, so text already darker than a
        // surface this dark cannot reach 4.5:1 even at black (~3.0:1). Black is still
        // the most readable answer in that direction, so it is what we return.
        let surface = hsla(0., 0., 0.35, 1.);
        let text = hsla(0., 0., 0.30, 1.);
        assert!(contrast_ratio(BLACK, surface, surface) < MIN_TEXT_CONTRAST);

        let fixed = contrast_adjusted(text, surface, surface, MIN_TEXT_CONTRAST);
        assert_eq!(fixed.l, 0.);
    }
}
