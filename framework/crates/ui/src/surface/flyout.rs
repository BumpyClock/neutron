//! Centralized geometry, typography and color tokens for flyout surfaces.
//!
//! Every transient surface in the library — popover, hover card, popup menu,
//! context menu, select popup, command palette, editor popovers and the collapsed
//! sidebar submenu — shares one geometry language so they read as a family:
//!
//! - one container radius (`radius`, from [`Theme::radius_lg`]),
//! - one edge inset between the container and its rows (`inset`),
//! - a row radius that is *concentric* with the container (`item_radius = radius - inset`),
//! - one horizontal rhythm for rows (`item_padding_x`, `icon_gap`, `accessory_gap`),
//! - two type steps: `label` (Fluent body) for row labels, `meta` (Fluent caption)
//!   for shortcuts, subtitles, categories and section headers.
//!
//! [`SurfacePreset::flyout`](super::SurfacePreset::flyout) owns the *material*
//! (blur, noise, elevation, stroke); this module owns the *layout* on top of it.
//! Both derive from theme tokens, so a theme override moves every flyout together.

use gpui::{App, FontWeight, Hsla, Pixels, px};

use super::SurfacePreset;
use crate::{
    ActiveTheme, Size,
    theme::contrast::{MIN_TEXT_CONTRAST, contrast_adjusted},
};

/// Padding between the flyout edge and its rows.
const INSET: Pixels = px(4.);
/// Horizontal padding inside a row.
const ITEM_PADDING_X: Pixels = px(8.);
/// Vertical gap between adjacent rows.
const ITEM_GAP: Pixels = px(2.);
/// Gap between a row's leading icon and its label.
const ICON_GAP: Pixels = px(8.);
/// Gap between a row's label and its trailing accessories (shortcut, chevron, check).
const ACCESSORY_GAP: Pixels = px(12.);
/// Height of a group label or section header row.
const SECTION_HEADER_HEIGHT: Pixels = px(24.);
/// Vertical margin around a separator rule.
const SEPARATOR_MARGIN: Pixels = px(4.);
/// Padding for prose flyouts (popover body, hover card, documentation).
const CONTENT_PADDING: Pixels = px(12.);
/// Minimum width of a menu-style flyout.
const MIN_WIDTH: Pixels = px(128.);
/// Maximum width of a menu-style flyout.
const MAX_WIDTH: Pixels = px(500.);
/// Smallest row radius that still reads as rounded.
const MIN_ITEM_RADIUS: Pixels = px(2.);

/// Layout and text tokens shared by every flyout surface.
///
/// Build with [`FlyoutTokens::new`] for the default (medium) density, or
/// [`FlyoutTokens::sized`] to match a control's [`Size`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FlyoutTokens {
    // -- Container geometry --
    /// Corner radius of the flyout container.
    pub radius: Pixels,
    /// Padding between the container edge and its rows.
    pub inset: Pixels,
    /// Padding for prose content (popover body, hover card, documentation).
    pub content_padding: Pixels,
    /// Minimum width of a menu-style flyout.
    pub min_width: Pixels,
    /// Maximum width of a menu-style flyout.
    pub max_width: Pixels,

    // -- Row geometry --
    /// Corner radius of a row, concentric with [`Self::radius`].
    pub item_radius: Pixels,
    /// Height of a single-line row.
    pub item_height: Pixels,
    /// Horizontal padding inside a row.
    pub item_padding_x: Pixels,
    /// Vertical gap between adjacent rows.
    pub item_gap: Pixels,
    /// Gap between a leading icon and its label.
    pub icon_gap: Pixels,
    /// Gap between a label and its trailing accessories.
    pub accessory_gap: Pixels,
    /// Height of a group label or section header row.
    pub section_header_height: Pixels,
    /// Vertical margin around a separator rule.
    pub separator_margin: Pixels,

    // -- Typography --
    /// Font size for row labels.
    pub label_size: Pixels,
    /// Font size for shortcuts, subtitles, categories and section headers.
    pub meta_size: Pixels,
    /// Font weight for group labels and section headers.
    pub section_weight: FontWeight,
}

impl FlyoutTokens {
    /// Tokens at the default (medium) row density.
    pub fn new(cx: &App) -> Self {
        Self::sized(Size::Medium, cx)
    }

    /// Tokens matching a control [`Size`].
    pub fn sized(size: Size, cx: &App) -> Self {
        let theme = cx.theme();
        let radius = theme.radius_lg;

        Self {
            radius,
            inset: INSET,
            content_padding: CONTENT_PADDING,
            min_width: MIN_WIDTH,
            max_width: MAX_WIDTH,

            item_radius: (radius - INSET).max(MIN_ITEM_RADIUS),
            item_height: Self::item_height_for(size),
            item_padding_x: ITEM_PADDING_X,
            item_gap: ITEM_GAP,
            icon_gap: ICON_GAP,
            accessory_gap: ACCESSORY_GAP,
            section_header_height: SECTION_HEADER_HEIGHT,
            separator_margin: SEPARATOR_MARGIN,

            label_size: theme.typography.body.size,
            meta_size: theme.typography.caption.size,
            section_weight: FontWeight::MEDIUM,
        }
    }

    fn item_height_for(size: Size) -> Pixels {
        match size {
            Size::Size(height) => height,
            Size::XSmall => px(22.),
            Size::Small => px(26.),
            Size::Medium => px(30.),
            Size::Large => px(34.),
        }
    }
}

/// The composited color of the flyout material, over [`Theme::background`].
///
/// Text inside a flyout reads against *this*, not against the window background,
/// which is what makes [`flyout_secondary_foreground`] a distinct role rather
/// than an alias for [`Theme::muted_foreground`].
pub fn flyout_material_color(cx: &App) -> Hsla {
    SurfacePreset::flyout().resolve_background(cx).resolve(cx)
}

/// Secondary text color inside a flyout: shortcuts, subtitles, categories,
/// section headers and enabled accessory icons.
///
/// Flyout materials sit *above* the window background — in dark mode they are
/// noticeably lighter than it — so [`Theme::muted_foreground`], which themes tune
/// against the window background, loses roughly a stop of contrast here. This
/// role keeps that token wherever it is already readable and lightens (or, on a
/// light material, darkens) it by the minimum needed to hold WCAG AA otherwise.
/// The correction is a pure function of the theme, so custom themes get it too,
/// and it is scoped to flyouts: cards and other raised surfaces that share
/// `muted_foreground` are untouched.
///
/// Always pair it with [`flyout_primary_foreground`] for the label so the
/// two-step hierarchy is consistent across every flyout.
pub fn flyout_secondary_foreground(cx: &App) -> Hsla {
    contrast_adjusted(
        cx.theme().muted_foreground,
        flyout_material_color(cx),
        cx.theme().background,
        MIN_TEXT_CONTRAST,
    )
}

/// Primary text color inside a flyout: row labels and prose body.
///
/// Usually [`Theme::popover_foreground`] as-is; corrected by the minimum lightness
/// change when a theme's own label color falls under AA on the flyout material
/// (deliberately dim themes such as Alduin, where even the window foreground is a
/// mid grey).
pub fn flyout_primary_foreground(cx: &App) -> Hsla {
    contrast_adjusted(
        cx.theme().popover_foreground,
        flyout_material_color(cx),
        cx.theme().background,
        MIN_TEXT_CONTRAST,
    )
}

/// Secondary text color on a *selected* row, where the row paints
/// [`Theme::primary`] behind its content.
#[inline]
pub fn flyout_selected_secondary_foreground(cx: &App) -> Hsla {
    cx.theme().primary_foreground.opacity(0.8)
}

/// Opacity applied to [`flyout_secondary_foreground`] for disabled content.
const DISABLED_OPACITY: f32 = 0.5;

/// Text and icon color for a disabled flyout row — its label, shortcut and icons.
///
/// One clear step below [`flyout_secondary_foreground`], so a disabled row reads
/// as unavailable rather than merely secondary, and so section headers (which
/// stay at the secondary step) separate from it. Deriving it from the corrected
/// secondary keeps the three-step ladder consistent in every theme. WCAG 1.4.3
/// exempts disabled controls from the AA floor, which is what allows this role
/// to sit below it.
pub fn flyout_disabled_foreground(cx: &App) -> Hsla {
    flyout_secondary_foreground(cx).opacity(DISABLED_OPACITY)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_height_follows_size() {
        assert_eq!(FlyoutTokens::item_height_for(Size::XSmall), px(22.));
        assert_eq!(FlyoutTokens::item_height_for(Size::Small), px(26.));
        assert_eq!(FlyoutTokens::item_height_for(Size::Medium), px(30.));
        assert_eq!(FlyoutTokens::item_height_for(Size::Large), px(34.));
        assert_eq!(FlyoutTokens::item_height_for(Size::Size(px(41.))), px(41.));
    }

    #[test]
    fn row_radius_stays_concentric_with_the_container() {
        // The rounded row must be inset from the rounded container by exactly the
        // edge padding, otherwise the two curves visibly disagree at the corners.
        for radius in [px(4.), px(6.), px(8.), px(12.)] {
            let item_radius = (radius - INSET).max(MIN_ITEM_RADIUS);
            assert!(item_radius >= MIN_ITEM_RADIUS);
            if radius - INSET >= MIN_ITEM_RADIUS {
                assert_eq!(item_radius + INSET, radius);
            }
        }
    }
}

/// Accent text color inside a flyout: search match highlights and any other
/// emphasis that should read as "this is why the row matched".
///
/// Uses [`Theme::primary`], the theme's emphasis color, rather than a border
/// token — borders carry no legibility guarantee against the flyout material.
/// Corrected the same way as the other flyout roles so the accent stays
/// readable on materials the theme never tuned it against.
pub fn flyout_accent_foreground(cx: &App) -> Hsla {
    contrast_adjusted(
        cx.theme().primary,
        flyout_material_color(cx),
        cx.theme().background,
        MIN_TEXT_CONTRAST,
    )
}
