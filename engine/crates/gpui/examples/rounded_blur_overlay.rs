//! Rounded native blur overlay.
//!
//! Shows a pill-shaped overlay window whose background is blurred by the OS
//! compositor, with the blur clipped to a rounded rectangle. This is created
//! with `cx.open_overlay_surface` + `OverlaySurfaceOptions` using
//! `WindowBackgroundAppearance::Blurred { corner_radius }`.
//!
//! The window is 360x72 logical px and the corner radius is 36 (half the
//! height), which turns the rounded rect into a perfect pill.
//!
//! Platform degradation matrix for `Blurred { corner_radius }`:
//! - Windows: acrylic accent clipped to a rounded HWND region. GDI regions are
//!   aliased, so the region edge looks jagged; the root view is rendered with a
//!   matching radius to cover it (see comment below).
//! - macOS 12+: NSVisualEffectView masked to the rounded rect. macOS < 12 has no
//!   mask API, so it degrades to unmasked (rectangular) window-background blur.
//! - Linux Wayland: KDE blur protocol with a region approximating the rounded
//!   rect; compositors without blur support degrade to plain transparency.
//! - X11 / web: degrade to plain transparency (no blur, no artifact).
//!
//! Quit with Ctrl+C.

use gpui::{
    App, Context, OverlayInputMode, OverlaySurfaceOptions, Window, WindowBackgroundAppearance,
    WindowBounds, div, hsla, prelude::*, px, rgb, rgba, size,
};
use gpui_platform::application;

/// Window size: the corner radius is half the height so the rounded rect is a pill.
const PILL_WIDTH: gpui::Pixels = px(360.0);
const PILL_HEIGHT: gpui::Pixels = px(72.0);
const CORNER_RADIUS: gpui::Pixels = px(36.0);

struct RoundedBlurOverlay;

impl Render for RoundedBlurOverlay {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            // The element radius MUST exactly match the window `corner_radius`.
            // On Windows the compositor blur is clipped by an aliased GDI region;
            // drawing our own rounded content at the same radius hides that jagged
            // region edge behind the crisp, anti-aliased element border.
            .rounded(CORNER_RADIUS)
            // Semi-transparent fill lets the blurred desktop show through.
            .bg(rgba(0x00000040))
            .border_1()
            .border_color(hsla(0.0, 0.0, 1.0, 0.20))
            .flex()
            .items_center()
            .justify_center()
            .text_color(rgb(0xffffff))
            .child("Rounded native blur")
    }
}

fn main() {
    application().run(|cx: &mut App| {
        let bounds = WindowBounds::centered(size(PILL_WIDTH, PILL_HEIGHT), cx);

        cx.open_overlay_surface(
            OverlaySurfaceOptions {
                window_bounds: Some(bounds),
                window_background: WindowBackgroundAppearance::Blurred {
                    corner_radius: CORNER_RADIUS,
                },
                show: true,
                focus: true,
                has_shadow: Some(false),
                // Interactive so the overlay can be clicked/focused (not click-through).
                input_mode: OverlayInputMode::Interactive,
                ..Default::default()
            },
            |_, cx| cx.new(|_| RoundedBlurOverlay),
        )
        .unwrap();

        cx.activate(true);
    });
}
