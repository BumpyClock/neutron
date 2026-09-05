use std::{mem, ops::Range, time::Instant};

use collections::FxHashSet;

use crate::{
    Animation, AnyElement, App, Bounds, ContentMask, EasingBounds, Element, ElementId, EntityId,
    GlobalElementId, InspectorElementId, IntoElement, LayoutId, PaintIndex, PrepaintStateIndex,
    RetainedLayer, RetainedLayerContentRevision, TextStyle, Transformation, TransformationMatrix,
    Window,
};

/// Compositor properties for a retained layer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RetainedLayerStyle {
    /// Bounds-relative transform applied by the renderer after child content is cached.
    pub transformation: Option<Transformation>,
    /// Opacity applied by the renderer after child content is cached.
    pub opacity: f32,
}

impl Default for RetainedLayerStyle {
    fn default() -> Self {
        Self {
            transformation: None,
            opacity: 1.0,
        }
    }
}

impl RetainedLayerStyle {
    /// Create retained layer style with identity transform and full opacity.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set compositor transform.
    pub fn transform(mut self, transformation: Transformation) -> Self {
        self.transformation = Some(transformation);
        self
    }

    /// Set compositor opacity.
    pub fn opacity(mut self, opacity: f32) -> Self {
        self.opacity = opacity;
        self
    }
}

/// Extension trait for wrapping element content in retained compositor layers.
pub trait RetainedLayerExt {
    /// Wrap this element in a retained compositor layer.
    fn with_retained_layer(
        self,
        id: impl Into<ElementId>,
        content_revision: impl Into<RetainedLayerContentRevision>,
    ) -> RetainedLayerElement<Self>
    where
        Self: Sized,
    {
        RetainedLayerElement {
            id: id.into(),
            element: Some(self),
            content_revision: content_revision.into(),
            style: RetainedLayerStyle::default(),
        }
    }
}

impl<E: IntoElement + 'static> RetainedLayerExt for E {}

/// Element wrapper that records a retained compositor layer descriptor.
pub struct RetainedLayerElement<E> {
    id: ElementId,
    element: Option<E>,
    content_revision: RetainedLayerContentRevision,
    style: RetainedLayerStyle,
}

impl<E> RetainedLayerElement<E> {
    /// Set compositor transform.
    pub fn transform(mut self, transformation: Transformation) -> Self {
        self.style.transformation = Some(transformation);
        self
    }

    /// Set compositor opacity.
    pub fn opacity(mut self, opacity: f32) -> Self {
        self.style.opacity = opacity;
        self
    }

    /// Set child content revision.
    pub fn content_revision(
        mut self,
        content_revision: impl Into<RetainedLayerContentRevision>,
    ) -> Self {
        self.content_revision = content_revision.into();
        self
    }
}

impl<E: IntoElement + 'static> IntoElement for RetainedLayerElement<E> {
    type Element = RetainedLayerElement<E>;

    fn into_element(self) -> Self::Element {
        self
    }
}

struct RetainedLayerState {
    content_revision: RetainedLayerContentRevision,
    bounds: Bounds<crate::Pixels>,
    content_mask: ContentMask<crate::Pixels>,
    text_style: TextStyle,
    accessed_entities: FxHashSet<EntityId>,
    has_deferred_draws: bool,
    prepaint_range: Range<PrepaintStateIndex>,
    paint_range: Range<PaintIndex>,
}

impl<E: IntoElement + 'static> Element for RetainedLayerElement<E> {
    type RequestLayoutState = AnyElement;
    type PrepaintState = bool;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
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
        let mut element = self
            .element
            .take()
            .expect("retained layer element should only be requested once")
            .into_any_element();
        let layout_id = element.request_layout(window, cx);
        (layout_id, element)
    }

    fn prepaint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<crate::Pixels>,
        element: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let global_id = global_id.expect("retained layer element must have a global id");
        window.with_element_state(
            global_id,
            |mut state: Option<RetainedLayerState>, window| {
                let content_mask = window.content_mask();
                let text_style = window.text_style();
                let content_dirty = state.as_ref().is_none_or(|state| {
                    state.content_revision != self.content_revision
                        || state.bounds != bounds
                        || state.content_mask != content_mask
                        || state.text_style != text_style
                        // Deferred replay does not update descendant element cache ranges.
                        || state.has_deferred_draws
                        || window.refreshing
                        || window.a11y.is_active()
                        || window.is_inspector_picking(cx)
                });
                let prepaint_start = window.prepaint_index();
                let deferred_draws_start = window.next_frame.deferred_draws.len();
                let accessed_entities = if content_dirty {
                    let refreshing = mem::replace(&mut window.refreshing, true);
                    let (_, accessed_entities) =
                        cx.detect_accessed_entities(|cx| element.prepaint(window, cx));
                    window.refreshing = refreshing;
                    accessed_entities
                } else {
                    // Paint callbacks retain hitbox IDs, so reuse both phases together.
                    let state = state.as_mut().unwrap();
                    window.reuse_prepaint(state.prepaint_range.clone());
                    cx.entities.extend_accessed(&state.accessed_entities);
                    mem::take(&mut state.accessed_entities)
                };
                let prepaint_end = window.prepaint_index();
                let paint_range = state.map(|state| state.paint_range).unwrap_or_default();
                (
                    content_dirty,
                    RetainedLayerState {
                        content_revision: self.content_revision,
                        bounds,
                        content_mask,
                        text_style,
                        accessed_entities,
                        has_deferred_draws: window.next_frame.deferred_draws.len()
                            > deferred_draws_start,
                        prepaint_range: prepaint_start..prepaint_end,
                        paint_range,
                    },
                )
            },
        )
    }

    fn paint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<crate::Pixels>,
        element: &mut Self::RequestLayoutState,
        content_dirty: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let global_id = global_id.expect("retained layer element must have a global id");
        let content_mask = window.content_mask();
        let content_revision = self.content_revision;
        let style = self.style;
        let transform = style
            .transformation
            .map(|transformation| {
                transformation.into_matrix(bounds.center(), window.scale_factor())
            })
            .unwrap_or_else(TransformationMatrix::unit);

        window.with_element_state(global_id, |state: Option<RetainedLayerState>, window| {
            let mut state = state.expect("retained layer must prepaint before paint");
            let paint_start = window.paint_index();
            if *content_dirty {
                let refreshing = mem::replace(&mut window.refreshing, true);
                let (_, accessed_entities) =
                    cx.detect_accessed_entities(|cx| element.paint(window, cx));
                state.accessed_entities.extend(accessed_entities);
                window.refreshing = refreshing;
            } else {
                let retained_layers_start = window.next_frame.scene.retained_layers.len();
                window.reuse_paint(state.paint_range.clone());
                let mut retained_layers = window
                    .next_frame
                    .scene
                    .retained_layers
                    .split_off(retained_layers_start);
                let retained_layer_id = global_id.clone();
                retained_layers.retain(|layer| layer.id != retained_layer_id);
                window
                    .next_frame
                    .scene
                    .retained_layers
                    .extend(retained_layers);
            }
            let paint_end = window.paint_index();

            window
                .next_frame
                .scene
                .insert_retained_layer(RetainedLayer {
                    id: global_id.clone(),
                    content_revision,
                    content_dirty: *content_dirty,
                    bounds: bounds.scale(window.scale_factor()),
                    content_mask: content_mask.scale(window.scale_factor()),
                    transform,
                    opacity: style.opacity,
                    paint_range: paint_start.scene_index()..paint_end.scene_index(),
                });

            state.paint_range = paint_start..paint_end;
            ((), state)
        });
    }
}

/// Extension trait for typed compositor-only animations.
pub trait CompositorAnimationExt {
    /// Animate compositor transform and opacity without mutating child content.
    fn with_compositor_animation(
        self,
        id: impl Into<ElementId>,
        content_revision: impl Into<RetainedLayerContentRevision>,
        animation: Animation,
        animator: impl Fn(f32) -> RetainedLayerStyle + 'static,
    ) -> CompositorAnimationElement<Self>
    where
        Self: Sized,
    {
        CompositorAnimationElement {
            id: id.into(),
            element: Some(self),
            content_revision: content_revision.into(),
            animation,
            animator: Box::new(animator),
        }
    }
}

impl<E: IntoElement + 'static> CompositorAnimationExt for E {}

/// Element wrapper for compositor-only transform and opacity animations.
pub struct CompositorAnimationElement<E> {
    id: ElementId,
    element: Option<E>,
    content_revision: RetainedLayerContentRevision,
    animation: Animation,
    animator: Box<dyn Fn(f32) -> RetainedLayerStyle + 'static>,
}

struct CompositorAnimationState {
    start: Instant,
}

impl<E: IntoElement + 'static> IntoElement for CompositorAnimationElement<E> {
    type Element = CompositorAnimationElement<E>;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl<E: IntoElement + 'static> Element for CompositorAnimationElement<E> {
    type RequestLayoutState = AnyElement;
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
        let style = window.with_global_id(self.id.clone(), |global_id, window| {
            window.with_element_state(
                global_id,
                |state: Option<CompositorAnimationState>, window| {
                    let state = state.unwrap_or_else(|| CompositorAnimationState {
                        start: Instant::now(),
                    });
                    let mut delta =
                        state.start.elapsed().as_secs_f32() / self.animation.duration.as_secs_f32();
                    let done = delta > 1.0 && self.animation.oneshot;

                    if delta > 1.0 {
                        if self.animation.oneshot {
                            delta = 1.0;
                        } else {
                            delta %= 1.0;
                        }
                    }

                    let delta = (self.animation.easing)(delta);
                    match self.animation.easing_bounds {
                        EasingBounds::Bounded => {
                            debug_assert!(
                                (0.0..=1.0).contains(&delta),
                                "delta should always be between 0 and 1"
                            );
                        }
                        EasingBounds::Unbounded => {
                            debug_assert!(delta.is_finite(), "delta must be finite");
                        }
                        EasingBounds::Range { min, max } => {
                            debug_assert!(
                                (min..=max).contains(&delta),
                                "delta should always be between {} and {}",
                                min,
                                max
                            );
                        }
                    }
                    if !done {
                        window.request_animation_frame();
                    }

                    ((self.animator)(delta), state)
                },
            )
        });

        let mut retained_layer = self
            .element
            .take()
            .expect("compositor animation element should only be requested once")
            .with_retained_layer(self.id.clone(), self.content_revision);
        if let Some(transformation) = style.transformation {
            retained_layer = retained_layer.transform(transformation);
        }
        let mut element = retained_layer.opacity(style.opacity).into_any_element();
        let layout_id = element.request_layout(window, cx);
        (layout_id, element)
    }

    fn prepaint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<crate::Pixels>,
        element: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        element.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<crate::Pixels>,
        element: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        element.paint(window, cx);
    }
}

#[cfg(test)]
#[path = "retained_layer_tests.rs"]
mod tests;
