use gpui::{
    App, Bounds, Context, Pixels, Window, WindowBounds, WindowOptions, div, hsla,
    linear_color_stop, linear_gradient, prelude::*, px, rgb, size,
};
use gpui_platform::application;

const WINDOW_WIDTH: Pixels = px(1600.0);
const WINDOW_HEIGHT: Pixels = px(1000.0);
const CARD_SIZE: Pixels = px(220.0);

struct BackdropBlurMemory {
    frame: u32,
}

impl BackdropBlurMemory {
    fn phase(frame: u32) -> &'static str {
        match frame {
            0..=79 => "spread blur cards",
            80..=139 => "clustered blur cards",
            140..=199 => "no blur cards",
            _ => "complete",
        }
    }

    fn blur_card(label: &'static str, top: Pixels, left: Pixels) -> impl IntoElement {
        div()
            .absolute()
            .top(top)
            .left(left)
            .size(CARD_SIZE)
            .rounded_xl()
            .backdrop_blur(px(24.0))
            .bg(hsla(0.0, 0.0, 1.0, 0.30))
            .border_1()
            .border_color(hsla(0.0, 0.0, 1.0, 0.55))
            .shadow_lg()
            .flex()
            .items_center()
            .justify_center()
            .text_color(rgb(0xffffff))
            .text_xl()
            .child(label)
    }

    /// Solid opaque card — same size/layout as blur_card but NO backdrop_blur.
    /// Used to isolate animation memory cost from blur memory cost.
    fn solid_card(label: &'static str, top: Pixels, left: Pixels) -> impl IntoElement {
        div()
            .absolute()
            .top(top)
            .left(left)
            .size(CARD_SIZE)
            .rounded_xl()
            .bg(rgb(0x3b82f6))
            .border_1()
            .border_color(hsla(0.0, 0.0, 1.0, 0.55))
            .shadow_lg()
            .flex()
            .items_center()
            .justify_center()
            .text_color(rgb(0xffffff))
            .text_xl()
            .child(label)
    }

    fn mix_pixels(start: Pixels, end: Pixels, amount: f32) -> Pixels {
        px(start.as_f32() + (end.as_f32() - start.as_f32()) * amount)
    }

    fn animated_position(
        frame: u32,
        phase: u32,
        start_top: Pixels,
        start_left: Pixels,
        end_top: Pixels,
        end_left: Pixels,
    ) -> (Pixels, Pixels) {
        let cycle = ((frame + phase) % 240) as f32 / 240.0;
        let amount = if cycle < 0.5 {
            cycle * 2.0
        } else {
            (1.0 - cycle) * 2.0
        };

        (
            Self::mix_pixels(start_top, end_top, amount),
            Self::mix_pixels(start_left, end_left, amount),
        )
    }

    fn background_tile(color: gpui::Rgba, top: Pixels, left: Pixels) -> impl IntoElement {
        div()
            .absolute()
            .top(top)
            .left(left)
            .size(px(280.0))
            .rounded_xl()
            .bg(color)
    }
}

impl Render for BackdropBlurMemory {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.frame += 1;
        let auto_quit = std::env::var_os("GPUI_BACKDROP_BLUR_AUTORUN").is_some();
        let animate = std::env::var_os("GPUI_BACKDROP_BLUR_ANIMATE").is_some();
        if auto_quit || animate {
            if auto_quit && self.frame > 420 {
                cx.quit();
            }
            window.request_animation_frame();
        }

        let frame = if auto_quit || animate { self.frame } else { 1 };
        let blur_disabled = std::env::var_os("GPUI_BACKDROP_BLUR_DISABLE").is_some();
        let phase = if blur_disabled && animate {
            "animated solid cards (no blur)"
        } else if blur_disabled {
            "blur disabled"
        } else if animate {
            "animated blur cards"
        } else {
            Self::phase(frame)
        };
        let status = if animate {
            phase.to_string()
        } else {
            format!("frame {}: {}", frame, phase)
        };

        let mut root = div()
            .relative()
            .size_full()
            .overflow_hidden()
            .p_6()
            .bg(linear_gradient(
                135.0,
                linear_color_stop(rgb(0x111827), 0.0),
                linear_color_stop(rgb(0xf97316), 1.0),
            ))
            .text_color(rgb(0xffffff))
            .child(Self::background_tile(rgb(0xef4444), px(48.0), px(48.0)))
            .child(Self::background_tile(rgb(0x22c55e), px(48.0), px(1272.0)))
            .child(Self::background_tile(rgb(0x8b5cf6), px(672.0), px(660.0)))
            .child(
                div()
                    .absolute()
                    .top(px(24.0))
                    .left(px(24.0))
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .bg(hsla(0.0, 0.0, 0.0, 0.45))
                    .child(status),
            );

        if blur_disabled && animate {
            // Animated solid cards — no blur, isolates animation memory cost
            let card_a =
                Self::animated_position(frame, 0, px(60.0), px(60.0), px(120.0), px(160.0));
            let card_b =
                Self::animated_position(frame, 20, px(60.0), px(1320.0), px(150.0), px(300.0));
            let card_c =
                Self::animated_position(frame, 40, px(700.0), px(60.0), px(180.0), px(440.0));
            let card_d =
                Self::animated_position(frame, 60, px(700.0), px(1320.0), px(210.0), px(580.0));
            let card_e =
                Self::animated_position(frame, 80, px(390.0), px(690.0), px(240.0), px(720.0));

            root = root
                .child(Self::solid_card("solid a", card_a.0, card_a.1))
                .child(Self::solid_card("solid b", card_b.0, card_b.1))
                .child(Self::solid_card("solid c", card_c.0, card_c.1))
                .child(Self::solid_card("solid d", card_d.0, card_d.1))
                .child(Self::solid_card("solid e", card_e.0, card_e.1));
        } else if !blur_disabled && animate {
            let card_a =
                Self::animated_position(frame, 0, px(60.0), px(60.0), px(120.0), px(160.0));
            let card_b =
                Self::animated_position(frame, 20, px(60.0), px(1320.0), px(150.0), px(300.0));
            let card_c =
                Self::animated_position(frame, 40, px(700.0), px(60.0), px(180.0), px(440.0));
            let card_d =
                Self::animated_position(frame, 60, px(700.0), px(1320.0), px(210.0), px(580.0));
            let card_e =
                Self::animated_position(frame, 80, px(390.0), px(690.0), px(240.0), px(720.0));

            root = root
                .child(Self::blur_card("anim a", card_a.0, card_a.1))
                .child(Self::blur_card("anim b", card_b.0, card_b.1))
                .child(Self::blur_card("anim c", card_c.0, card_c.1))
                .child(Self::blur_card("anim d", card_d.0, card_d.1))
                .child(Self::blur_card("anim e", card_e.0, card_e.1));
        } else if !blur_disabled && frame <= 79 {
            root = root
                .child(Self::blur_card("top left", px(60.0), px(60.0)))
                .child(Self::blur_card("top right", px(60.0), px(1320.0)))
                .child(Self::blur_card("bottom", px(700.0), px(690.0)));
        } else if !blur_disabled && frame <= 139 {
            root = root
                .child(Self::blur_card("cluster a", px(120.0), px(120.0)))
                .child(Self::blur_card("cluster b", px(150.0), px(180.0)))
                .child(Self::blur_card("cluster c", px(180.0), px(240.0)));
        }

        root
    }
}

fn main() {
    application().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(WINDOW_WIDTH, WINDOW_HEIGHT), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(|_| BackdropBlurMemory { frame: 0 }),
        )
        .unwrap();
        cx.activate(true);
    });
}
