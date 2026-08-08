//! ClosingScope — exposes a root layer's closing state to its content.
//!
//! The Root keeps a closing dialog or sheet mounted for the exit window
//! ([`crate::animation::exit_duration`]) before unmounting it. This element is
//! how content inside that layer *learns* it is closing, so it can render its
//! own exit animation during that window: the layer wraps its content in a
//! `ClosingScope`, and content reads [`is_layer_closing`] during render.
//!
//! The value is pushed during layout as well as paint: entity children build
//! their element trees during the layout pass, so a paint-only scope (like the
//! reduced-motion scope) would be invisible to a child view's `render`.

use gpui::{
    AnyElement, App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement,
    LayoutId, Pixels, Window,
};

use crate::global_state::GlobalState;

/// Returns whether the enclosing root layer (dialog, sheet) is closing.
///
/// Content hosted in a layer that defers its unmount (see
/// [`crate::dialog::Dialog::defer_close`]) can use this to switch to an exit
/// animation; the layer stays mounted for
/// [`crate::animation::exit_duration`], then is torn down regardless — the
/// window is the ceiling, there is no completion signal to forget.
pub fn is_layer_closing(cx: &App) -> bool {
    GlobalState::global(cx).layer_closing()
}

/// A wrapper element that provides the layer-closing context to its children.
pub struct ClosingScope {
    closing: bool,
    child: Option<AnyElement>,
}

impl ClosingScope {
    /// Wrap `child`, exposing `closing` to everything inside it via
    /// [`is_layer_closing`].
    pub fn new(closing: bool, child: impl IntoElement) -> Self {
        Self {
            closing,
            child: Some(child.into_any_element()),
        }
    }

    fn scoped<R>(&self, cx: &mut App, f: impl FnOnce(&mut App) -> R) -> R {
        GlobalState::global_mut(cx).push_layer_closing(self.closing);
        let result = f(cx);
        GlobalState::global_mut(cx).pop_layer_closing();
        result
    }
}

impl IntoElement for ClosingScope {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

/// Layout state for ClosingScope, holds the child element.
pub struct ClosingScopeLayoutState {
    child: AnyElement,
}

impl Element for ClosingScope {
    type RequestLayoutState = ClosingScopeLayoutState;
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
        let mut child = self.child.take().expect("ClosingScope child already taken");
        let layout_id = self.scoped(cx, |cx| child.request_layout(window, cx));
        (layout_id, ClosingScopeLayoutState { child })
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
        self.scoped(cx, |cx| request_layout.child.prepaint(window, cx));
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
        self.scoped(cx, |cx| request_layout.child.paint(window, cx));
    }
}
