//! FloatingInsetScope - A wrapper element that provides floating inset context to children.

use gpui::{
    AnyElement, App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement,
    LayoutId, Pixels, Window,
};

use crate::global_state::GlobalState;

/// A wrapper element that provides `floating_inset` context to its children.
pub struct FloatingInsetScope {
    inset: Pixels,
    child: Option<AnyElement>,
}

impl FloatingInsetScope {
    /// Create a new floating inset context scope.
    pub fn new(inset: Pixels, child: impl IntoElement) -> Self {
        Self {
            inset,
            child: Some(child.into_any_element()),
        }
    }
}

impl IntoElement for FloatingInsetScope {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

pub struct FloatingInsetScopeLayoutState {
    child: AnyElement,
}

impl Element for FloatingInsetScope {
    type RequestLayoutState = FloatingInsetScopeLayoutState;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut child = self
            .child
            .take()
            .expect("FloatingInsetScope child already taken");
        let layout_id = child.request_layout(window, cx);
        (layout_id, FloatingInsetScopeLayoutState { child })
    }

    fn prepaint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        request_layout.child.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        GlobalState::global_mut(cx).push_floating_inset(self.inset);
        request_layout.child.paint(window, cx);
        GlobalState::global_mut(cx).pop_floating_inset();
    }
}
