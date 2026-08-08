//! CPU benchmark for GPUI update, layout, paint, and scene construction.
//!
//! The default benchmark platform intentionally has no headless renderer, so
//! these measurements do not claim to include GPU encoding or submission.

use std::fmt;

use gpui::{
    BenchAppContext, Context, IntoElement, Render, RetainedLayerExt, Window, div, hsla, prelude::*,
    px, rgb,
};

#[derive(Clone, Copy)]
enum SceneCase {
    BackdropBlur,
    RetainedWarm,
    RetainedContentDirty,
}

impl fmt::Display for SceneCase {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::BackdropBlur => "backdrop-blur",
            Self::RetainedWarm => "retained-warm",
            Self::RetainedContentDirty => "retained-content-dirty",
        })
    }
}

struct SceneBench {
    case: SceneCase,
    revision: u64,
    opacity: f32,
    phase: u64,
}

impl Render for SceneBench {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        match self.case {
            SceneCase::BackdropBlur => {
                let background = if self.phase.is_multiple_of(2) {
                    rgb(0x1e293b)
                } else {
                    rgb(0x334155)
                };
                let mut root = div().relative().size_full().bg(background);
                for index in 0..16 {
                    let column = index % 4;
                    let row = index / 4;
                    root = root.child(
                        div()
                            .absolute()
                            .left(px(32.0 + column as f32 * 112.0))
                            .top(px(32.0 + row as f32 * 88.0))
                            .w(px(144.0))
                            .h(px(112.0))
                            .rounded_xl()
                            .backdrop_blur(px(24.0))
                            .bg(hsla(0.0, 0.0, 1.0, 0.24)),
                    );
                }
                root.into_any_element()
            }
            SceneCase::RetainedWarm | SceneCase::RetainedContentDirty => div()
                .size_full()
                .bg(rgb(0x0f172a))
                .child(
                    div()
                        .size_full()
                        .bg(rgb(0x2563eb))
                        .with_retained_layer("scene-benchmark-layer", self.revision)
                        .opacity(self.opacity),
                )
                .into_any_element(),
        }
    }
}

#[gpui::bench(
    inputs = scene_cases(),
    group = "scene-construction",
    input_name = "mode",
    sample_size = 20
)]
fn scene_construction(case: &SceneCase, cx: &mut BenchAppContext) {
    let case = *case;
    let mut window = cx.add_empty_window();
    let view = window.update(|window, cx| {
        window.replace_root(cx, |_, _| SceneBench {
            case,
            revision: 0,
            opacity: 1.0,
            phase: 0,
        })
    });

    cx.bench_renderer(view, move |view, _window, cx| {
        view.phase = view.phase.wrapping_add(1);
        match case {
            SceneCase::BackdropBlur => {}
            SceneCase::RetainedWarm => {
                view.opacity = if view.opacity == 1.0 { 0.75 } else { 1.0 };
            }
            SceneCase::RetainedContentDirty => {
                view.revision = view.revision.wrapping_add(1);
            }
        }
        cx.notify();
    });
}

fn scene_cases() -> [SceneCase; 3] {
    [
        SceneCase::BackdropBlur,
        SceneCase::RetainedWarm,
        SceneCase::RetainedContentDirty,
    ]
}

gpui::bench_group!(benches, scene_construction);
gpui::bench_main!(benches);
