use std::{cell::Cell, rc::Rc, sync::Arc};

use crate::{
    self as gpui, App, Bounds, Context, Element, ElementId, GlobalElementId, InspectorElementId,
    IntoElement, LayoutId, Render, Style, TestAppContext, Window, fill, point, px, size, white,
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
