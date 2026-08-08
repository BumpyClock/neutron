//! ReducedMotionScope - A wrapper element that provides reduced motion context to children.
//!
//! This element pushes a `reduced_motion` value onto the global context stack before
//! rendering children, then pops it after. Child components can read this value via
//! `GlobalState::global(cx).reduced_motion()` or the `ReducedMotionContext` trait.
//!
//! # Example
//!
//! ```ignore
//! use gpui_component::{ReducedMotionContext, ReducedMotionScope};
//!
//! // Parent provides reduced motion context
//! ReducedMotionScope::new(true, div().child(my_sidebar))
//!
//! // Child reads from context
//! fn render(&self, window: &mut Window, cx: &mut App) -> impl IntoElement {
//!     let reduced_motion = cx.reduced_motion(); // Reads from parent's context
//!     // ...
//! }
//! ```

use gpui::{
    AnyElement, App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement,
    LayoutId, Pixels, Window,
};

use crate::global_state::GlobalState;

/// A wrapper element that provides `reduced_motion` context to its children.
///
/// When rendered, this element pushes its `reduced_motion` value onto the global
/// context stack, renders its child, then pops the value. This allows child
/// components to inherit motion preferences without explicit prop drilling.
///
/// # Usage
///
/// Typically used internally by `WindowShell` to provide motion preferences to sidebars
/// and other child components. Can also be used directly if you need to override
/// the motion preference for a subtree.
///
/// ```ignore
/// // Override reduced motion for a specific subtree
/// ReducedMotionScope::new(true,
///     div()
///         .child(sidebar_that_should_not_animate)
/// )
/// ```
pub struct ReducedMotionScope {
    reduced_motion: bool,
    child: Option<AnyElement>,
}

impl ReducedMotionScope {
    /// Create a new reduced motion context scope.
    ///
    /// # Arguments
    ///
    /// * `reduced_motion` - The reduced_motion value to provide to children
    /// * `child` - The child element that will inherit this motion context
    pub fn new(reduced_motion: bool, child: impl IntoElement) -> Self {
        Self {
            reduced_motion,
            child: Some(child.into_any_element()),
        }
    }
}

impl IntoElement for ReducedMotionScope {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

/// Layout state for ReducedMotionScope, holds the child element.
pub struct ReducedMotionScopeLayoutState {
    child: AnyElement,
}

impl Element for ReducedMotionScope {
    type RequestLayoutState = ReducedMotionScopeLayoutState;
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
            .expect("ReducedMotionScope child already taken");
        let layout_id = child.request_layout(window, cx);
        (layout_id, ReducedMotionScopeLayoutState { child })
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
        GlobalState::global_mut(cx).push_reduced_motion(self.reduced_motion);
        request_layout.child.paint(window, cx);
        GlobalState::global_mut(cx).pop_reduced_motion();
    }
}
