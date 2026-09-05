use std::{cell::Cell, rc::Rc, sync::Arc};

use crate::{
    self as gpui, App, AppContext, Bounds, Context, Element, ElementId, GlobalElementId,
    InspectorElementId, InteractiveElement, IntoElement, LayoutId, MouseButton, MouseDownEvent,
    MouseUpEvent, ParentElement, PlatformInput, Render, Style, Styled, TestAppContext, Window, div,
    fill, point, px, size, white,
};

use super::*;

struct CountingElement {
    paint_count: Rc<Cell<usize>>,
}

impl IntoElement for CountingElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for CountingElement {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        (
            window.request_layout(
                Style {
                    size: size(px(10.).into(), px(10.).into()),
                    ..Default::default()
                },
                [],
                cx,
            ),
            (),
        )
    }

    fn prepaint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<crate::Pixels>,
        _state: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) {
    }

    fn paint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<crate::Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        _cx: &mut App,
    ) {
        self.paint_count.set(self.paint_count.get() + 1);
        window.paint_quad(fill(bounds, white()));
    }
}

struct A11yCountingElement {
    paint_count: Rc<Cell<usize>>,
    action_count: Rc<Cell<usize>>,
}

impl IntoElement for A11yCountingElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for A11yCountingElement {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some("retained-child".into())
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn a11y_role(&self) -> Option<accesskit::Role> {
        Some(accesskit::Role::Button)
    }

    fn write_a11y_info(&self, node: &mut accesskit::Node) {
        node.add_action(accesskit::Action::Click);
    }

    fn request_layout(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        (
            window.request_layout(
                Style {
                    size: size(px(10.).into(), px(10.).into()),
                    ..Default::default()
                },
                [],
                cx,
            ),
            (),
        )
    }

    fn prepaint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<crate::Pixels>,
        _state: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) {
    }

    fn paint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<crate::Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        _cx: &mut App,
    ) {
        self.paint_count.set(self.paint_count.get() + 1);
        window.paint_quad(fill(bounds, white()));

        if window.a11y.is_active() {
            let node_id = window
                .a11y
                .node_id_for_existing(global_id.expect("a11y child must have a global id"))
                .expect("a11y child should have a node id");
            let action_count = self.action_count.clone();
            window.on_a11y_action(node_id, accesskit::Action::Click, move |_, _, _| {
                action_count.set(action_count.get() + 1);
            });
        }
    }
}

#[gpui::test]
fn retained_layer_replays_child_paint_on_compositor_only_update(cx: &mut TestAppContext) {
    let paint_count = Rc::new(Cell::new(0));
    let cx = cx.add_empty_window();

    draw_retained_layer(cx, paint_count.clone(), 0, 1.0);
    finish_frame(cx);

    assert_eq!(paint_count.get(), 1);
    let first_layer = cx.update(|window, _| window.rendered_frame.scene.retained_layers[0].clone());
    assert!(first_layer.content_dirty);
    assert_eq!(first_layer.paint_range, 0..1);

    draw_retained_layer(cx, paint_count.clone(), 0, 0.25);
    finish_frame(cx);

    assert_eq!(paint_count.get(), 1);
    let second_layer =
        cx.update(|window, _| window.rendered_frame.scene.retained_layers[0].clone());
    assert!(!second_layer.content_dirty);
    assert_eq!(second_layer.opacity, 0.25);
    assert_eq!(second_layer.paint_range, 0..1);

    draw_retained_layer(cx, paint_count.clone(), 1, 0.25);
    finish_frame(cx);

    assert_eq!(paint_count.get(), 2);
    let third_layer = cx.update(|window, _| window.rendered_frame.scene.retained_layers[0].clone());
    assert!(third_layer.content_dirty);
    assert_eq!(third_layer.content_revision, 1.into());
}

struct RetainedPointerTestView {
    paint_count: Rc<Cell<usize>>,
    click_count: Rc<Cell<usize>>,
    opacity: f32,
    left: crate::Pixels,
}

impl Render for RetainedPointerTestView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let click_count = self.click_count.clone();
        div().relative().size_full().child(
            div()
                .id("retained-button")
                .absolute()
                .left(self.left)
                .top(px(30.))
                .size(px(20.))
                .on_mouse_down(MouseButton::Left, move |_, _, _| {
                    click_count.set(click_count.get() + 1);
                })
                .child(CountingElement {
                    paint_count: self.paint_count.clone(),
                })
                .with_retained_layer("pointer-layer", 0)
                .opacity(self.opacity),
        )
    }
}

#[gpui::test]
fn retained_layer_preserves_pointer_callbacks_on_compositor_only_update(cx: &mut TestAppContext) {
    let paint_count = Rc::new(Cell::new(0));
    let click_count = Rc::new(Cell::new(0));
    let window = cx.add_window(|_, _| RetainedPointerTestView {
        paint_count: paint_count.clone(),
        click_count: click_count.clone(),
        opacity: 1.0,
        left: px(40.),
    });
    let click_at = |cx: &mut TestAppContext, position| {
        for event in [
            PlatformInput::MouseDown(MouseDownEvent {
                button: MouseButton::Left,
                position,
                modifiers: Default::default(),
                click_count: 1,
                first_mouse: false,
            }),
            PlatformInput::MouseUp(MouseUpEvent {
                button: MouseButton::Left,
                position,
                modifiers: Default::default(),
                click_count: 1,
            }),
        ] {
            cx.update_window(window.into(), |_, window, cx| {
                window.dispatch_event(event, cx);
            })
            .unwrap();
        }
    };

    for (index, opacity) in [1.0, 0.25, 0.75].into_iter().enumerate() {
        window
            .update(cx, |view, _, cx| {
                view.opacity = opacity;
                cx.notify();
            })
            .unwrap();
        cx.update_window(window.into(), |_, window, cx| {
            window.draw(cx).clear(cx);
            assert_eq!(
                window.rendered_frame.scene.retained_layers[0].opacity,
                opacity
            );
        })
        .unwrap();
        assert_eq!(paint_count.get(), 1 + 2 * index);
        click_at(cx, point(px(45.), px(35.)));
        assert_eq!(click_count.get(), index + 1);
        // Press and release each change active state and require a content refresh.
        assert_eq!(paint_count.get(), 3 + 2 * index);
    }

    window
        .update(cx, |view, _, cx| {
            view.left = px(80.);
            cx.notify();
        })
        .unwrap();
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))
        .unwrap();
    assert_eq!(paint_count.get(), 8);
    click_at(cx, point(px(45.), px(35.)));
    assert_eq!(click_count.get(), 3);
    assert_eq!(paint_count.get(), 8);
    click_at(cx, point(px(85.), px(35.)));
    assert_eq!(click_count.get(), 4);
    assert_eq!(paint_count.get(), 10);
}

#[gpui::test]
fn retained_layer_a11y(cx: &mut TestAppContext) {
    let paint_count = Rc::new(Cell::new(0));
    let action_count = Rc::new(Cell::new(0));
    let cx = cx.add_empty_window();

    draw_a11y_retained_layer(cx, paint_count.clone(), action_count.clone(), 0, 1.0);
    finish_frame(cx);

    assert_eq!(paint_count.get(), 1);

    draw_a11y_retained_layer(cx, paint_count.clone(), action_count.clone(), 0, 0.25);
    finish_frame(cx);

    assert_eq!(paint_count.get(), 2);

    let child_id = cx
        .update(|window, _| {
            window
                .a11y
                .node_id_for_existing(&retained_child_global_id())
        })
        .expect("retained child should have an a11y node id");
    cx.update(|window, _| {
        assert!(
            window.a11y.node_bounds.contains_key(&child_id),
            "retained child should be emitted into the active a11y frame"
        );
        assert!(
            window.a11y.action_listeners.contains_key(&child_id),
            "retained child should rebuild its a11y action listener"
        );
    });

    cx.update(|window, cx| {
        window.handle_a11y_action(
            accesskit::ActionRequest {
                action: accesskit::Action::Click,
                target_tree: accesskit::TreeId::ROOT,
                target_node: child_id,
                data: None,
            },
            cx,
        );
    });

    assert_eq!(action_count.get(), 1);
}

#[gpui::test]
fn retained_layer_invalidates_nested_ranges_after_parent_reuse(cx: &mut TestAppContext) {
    let paint_count = Rc::new(Cell::new(0));
    let cx = cx.add_empty_window();

    for (revision, prefix) in [(0, false), (0, true), (1, false)] {
        cx.update(|window, cx| {
            window.invalidator.set_phase(crate::DrawPhase::Prepaint);
            let mut element = crate::Drawable::new(
                CountingElement {
                    paint_count: paint_count.clone(),
                }
                .with_retained_layer("inner", 0)
                .with_retained_layer("outer", revision),
            );
            element.layout_as_root(size(px(100.), px(100.)).into(), window, cx);
            window.with_absolute_element_offset(Default::default(), |window| {
                element.prepaint(window, cx)
            });

            window.invalidator.set_phase(crate::DrawPhase::Paint);
            if prefix {
                window.paint_quad(fill(
                    Bounds::new(point(px(40.), px(40.)), size(px(10.), px(10.))),
                    white(),
                ));
            }
            element.paint(window, cx);
            window.invalidator.set_phase(crate::DrawPhase::None);
        });
        finish_frame(cx);
        assert_eq!(paint_count.get(), revision as usize + 1);
    }

    cx.update(|window, _| {
        assert_eq!(window.rendered_frame.scene.quads.len(), 1);
        assert_eq!(window.rendered_frame.scene.retained_layers.len(), 2);
        for layer in &window.rendered_frame.scene.retained_layers {
            assert!(layer.content_dirty);
            assert_eq!(layer.paint_range, 0..1);
        }
    });
}

struct DeferredRetainedTestView {
    paint_count: Rc<Cell<usize>>,
    prefix: bool,
    revision: u64,
}

impl Render for DeferredRetainedTestView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .relative()
            .size_full()
            .children(
                self.prefix
                    .then(|| div().absolute().size(px(10.)).bg(white())),
            )
            .child(
                crate::deferred(
                    CountingElement {
                        paint_count: self.paint_count.clone(),
                    }
                    .with_retained_layer("inner", 0),
                )
                .with_retained_layer("outer", self.revision),
            )
    }
}

#[gpui::test]
fn retained_layer_updates_deferred_descendant_ranges(cx: &mut TestAppContext) {
    let paint_count = Rc::new(Cell::new(0));
    let window = cx.add_window(|_, _| DeferredRetainedTestView {
        paint_count: paint_count.clone(),
        prefix: true,
        revision: 0,
    });
    let assert_inner_range = |cx: &mut TestAppContext, expected: Range<usize>| {
        cx.update_window(window.into(), |_, window, _| {
            let scene = &window.rendered_frame.scene;
            let inner = scene
                .retained_layers
                .iter()
                .find(|layer| layer.id.0.last() == Some(&ElementId::from("inner")))
                .expect("deferred inner layer must remain in the scene");
            assert_eq!(inner.paint_range, expected);
            assert_eq!(scene.quads.len(), expected.end);
        })
        .unwrap();
    };
    assert_inner_range(cx, 1..2);

    window
        .update(cx, |view, _, cx| {
            view.prefix = false;
            cx.notify();
        })
        .unwrap();
    assert_inner_range(cx, 0..1);

    window
        .update(cx, |view, _, cx| {
            view.revision = 1;
            cx.notify();
        })
        .unwrap();
    assert_inner_range(cx, 0..1);
    assert_eq!(paint_count.get(), 1);
}

fn draw_retained_layer(
    cx: &mut crate::VisualTestContext,
    paint_count: Rc<Cell<usize>>,
    revision: u64,
    opacity: f32,
) {
    cx.update(|window, cx| {
        window.invalidator.set_phase(crate::DrawPhase::Prepaint);
        let mut element = crate::Drawable::new(
            CountingElement { paint_count }
                .with_retained_layer("layer", revision)
                .opacity(opacity),
        );
        element.layout_as_root(size(px(100.), px(100.)).into(), window, cx);
        window.with_absolute_element_offset(Default::default(), |window| {
            element.prepaint(window, cx)
        });

        window.invalidator.set_phase(crate::DrawPhase::Paint);
        element.paint(window, cx);
        window.invalidator.set_phase(crate::DrawPhase::None);
    });
}

fn draw_a11y_retained_layer(
    cx: &mut crate::VisualTestContext,
    paint_count: Rc<Cell<usize>>,
    action_count: Rc<Cell<usize>>,
    revision: u64,
    opacity: f32,
) {
    cx.update(|window, cx| {
        window.a11y.set_active_for_test(true);
        window.a11y.begin_frame();

        window.invalidator.set_phase(crate::DrawPhase::Prepaint);
        let mut element = crate::Drawable::new(
            A11yCountingElement {
                paint_count,
                action_count,
            }
            .with_retained_layer("layer", revision)
            .opacity(opacity),
        );
        element.layout_as_root(size(px(100.), px(100.)).into(), window, cx);
        window.with_absolute_element_offset(point(px(0.), px(0.)), |window| {
            element.prepaint(window, cx)
        });

        window.invalidator.set_phase(crate::DrawPhase::Paint);
        element.paint(window, cx);
        window.invalidator.set_phase(crate::DrawPhase::None);

        window.a11y.end_frame();
    });
}

fn retained_child_global_id() -> GlobalElementId {
    GlobalElementId(Arc::from([
        ElementId::from("layer"),
        ElementId::from("retained-child"),
    ]))
}

fn finish_frame(cx: &mut crate::VisualTestContext) {
    cx.update(|window, _| {
        window.next_frame.finish(&mut window.rendered_frame);
        std::mem::swap(&mut window.rendered_frame, &mut window.next_frame);
        window.next_frame.clear();
        window.refreshing = false;
    });
}

struct CompositorAnimationTestView {
    paint_count: Rc<Cell<usize>>,
}

impl Render for CompositorAnimationTestView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        CountingElement {
            paint_count: self.paint_count.clone(),
        }
        .with_compositor_animation(
            "animated-layer",
            0,
            Animation::new(std::time::Duration::from_millis(100)),
            |_| RetainedLayerStyle::new().opacity(0.4),
        )
    }
}

#[gpui::test]
fn compositor_animation_records_typed_layer_style(cx: &mut TestAppContext) {
    let paint_count = Rc::new(Cell::new(0));
    let (_, cx) = cx.add_window_view(|_, _| CompositorAnimationTestView {
        paint_count: paint_count.clone(),
    });

    assert_eq!(paint_count.get(), 1);
    let layer = cx.update(|window, _| window.rendered_frame.scene.retained_layers[0].clone());
    assert_eq!(layer.opacity, 0.4);
}
