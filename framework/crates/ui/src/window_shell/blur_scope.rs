//! BlurEnabledScope - A wrapper element that provides blur context to children.
//!
//! This element pushes a `blur_enabled` value onto the global context stack before
//! rendering children, then pops it after. Child components can read this value via
//! `GlobalState::global(cx).blur_enabled()` or the `BlurContext` trait.
//!
//! # Example
//!
//! ```ignore
//! use gpui_component::{BlurEnabledScope, BlurContext};
//!
//! // Parent provides blur context
//! BlurEnabledScope::new(true, div().child(my_sidebar))
//!
//! // Child reads from context
//! fn render(&self, window: &mut Window, cx: &mut App) -> impl IntoElement {
//!     let blur = cx.blur_enabled(); // Reads from parent's context
//!     // ...
//! }
//! ```

use gpui::{
    AnyElement, App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement,
    LayoutId, Pixels, Window,
};

use crate::global_state::GlobalState;

/// A wrapper element that provides `blur_enabled` context to its children.
///
/// When rendered, this element pushes its `blur_enabled` value onto the global
/// context stack, renders its child, then pops the value. This allows child
/// components to inherit blur settings without explicit prop drilling.
///
/// # Usage
///
/// Typically used internally by `WindowShell` to provide blur context to sidebars
/// and other child components. Can also be used directly if you need to override
/// the blur context for a subtree.
///
/// ```ignore
/// // Override blur for a specific subtree
/// BlurEnabledScope::new(false,
///     div()
///         .child(sidebar_that_should_not_blur)
/// )
/// ```
pub struct BlurEnabledScope {
    enabled: bool,
    child: Option<AnyElement>,
}

impl BlurEnabledScope {
    /// Create a new blur context scope.
    ///
    /// # Arguments
    ///
    /// * `enabled` - The blur_enabled value to provide to children
    /// * `child` - The child element that will inherit this blur context
    pub fn new(enabled: bool, child: impl IntoElement) -> Self {
        Self {
            enabled,
            child: Some(child.into_any_element()),
        }
    }
}

impl IntoElement for BlurEnabledScope {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

/// Layout state for BlurEnabledScope, holds the child element.
pub struct BlurEnabledScopeLayoutState {
    child: AnyElement,
}

impl Element for BlurEnabledScope {
    type RequestLayoutState = BlurEnabledScopeLayoutState;
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
            .expect("BlurEnabledScope child already taken");
        let layout_id = child.request_layout(window, cx);
        (layout_id, BlurEnabledScopeLayoutState { child })
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
        // Push blur context before painting children
        GlobalState::global_mut(cx).push_blur_enabled(self.enabled);

        // Paint the child (which can now read blur_enabled from context)
        request_layout.child.paint(window, cx);

        // Pop blur context after painting children
        GlobalState::global_mut(cx).pop_blur_enabled();
    }
}
