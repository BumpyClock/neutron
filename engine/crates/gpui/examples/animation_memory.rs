use std::time::Duration;

use gpui::{
    Animation, AnimationExt as _, AnyElement, App, Bounds, Context, Pixels, Window, WindowBounds,
    WindowOptions, div, prelude::*, px, rgb, size,
};
use gpui_platform::application;

const WINDOW_WIDTH: Pixels = px(900.0);
const WINDOW_HEIGHT: Pixels = px(600.0);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Static,
    Manual,
    WithAnimation,
}

impl Mode {
    fn from_env() -> Self {
        match std::env::var("GPUI_ANIMATION_MEMORY_MODE").as_deref() {
            Ok("manual") => Self::Manual,
            Ok("with_animation") => Self::WithAnimation,
            _ => Self::Static,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Static => "static",
            Self::Manual => "manual request_animation_frame",
            Self::WithAnimation => "with_animation",
        }
    }
}

struct AnimationMemory {
    frame: u32,
    mode: Mode,
}

impl AnimationMemory {
    fn card(label: &'static str) -> gpui::Div {
        div()
            .absolute()
            .top(px(220.0))
            .left(px(180.0))
            .w(px(220.0))
            .h(px(140.0))
            .rounded_xl()
            .bg(rgb(0x2563eb))
            .text_color(rgb(0xffffff))
            .text_xl()
            .flex()
            .items_center()
            .justify_center()
            .child(label)
    }

    fn offset(frame: u32) -> Pixels {
        let cycle = (frame % 240) as f32 / 240.0;
        let amount = if cycle < 0.5 {
            cycle * 2.0
        } else {
            (1.0 - cycle) * 2.0
        };
        px(amount * 320.0)
    }

    fn animated_card(&self) -> AnyElement {
        match self.mode {
            Mode::Static => Self::card("static").into_any_element(),
            Mode::Manual => Self::card("manual")
                .translate_x(Self::offset(self.frame))
                .into_any_element(),
            Mode::WithAnimation => Self::card("with_animation")
                .with_animation(
                    "moving-card",
                    Animation::new(Duration::from_secs(2)).repeat(),
                    |card, delta| card.translate_x(px(delta * 320.0)),
                )
                .into_any_element(),
        }
    }
}

impl Render for AnimationMemory {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.frame += 1;

        let autorun = std::env::var_os("GPUI_ANIMATION_MEMORY_AUTORUN").is_some();
        if autorun && self.frame > 420 {
            cx.quit();
        }

        if self.mode == Mode::Manual || (autorun && self.mode == Mode::Static) {
            window.request_animation_frame();
        }

        div()
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(rgb(0x0f172a))
            .text_color(rgb(0xffffff))
            .child(div().absolute().top(px(24.0)).left(px(24.0)).child(format!(
                "mode: {} | frame: {}",
                self.mode.label(),
                self.frame
            )))
            .child(self.animated_card())
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
            |_, cx| {
                cx.new(|_| AnimationMemory {
                    frame: 0,
                    mode: Mode::from_env(),
                })
            },
        )
        .unwrap();
        cx.activate(true);
    });
}
