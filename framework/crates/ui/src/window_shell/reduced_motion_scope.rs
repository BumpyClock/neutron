//! ReducedMotionScope - A wrapper element that provides reduced motion context to children.
//!
//! This element pushes a `reduced_motion` value onto the global context stack before
//! rendering children, then pops it after. Child components can read this value via
//! `crate::animation::reduced_motion(cx)`.
//!
//! # Example
//!
//! ```ignore
//! use neutron_components::{ReducedMotionContext, ReducedMotionScope};
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

    fn scoped<R>(&self, cx: &mut App, f: impl FnOnce(&mut App) -> R) -> R {
        GlobalState::global_mut(cx).push_reduced_motion(self.reduced_motion);
        let result = f(cx);
        GlobalState::global_mut(cx).pop_reduced_motion();
        result
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
        let layout_id = self.scoped(cx, |cx| child.request_layout(window, cx));
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

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc};

    use gpui::{App, AppContext as _, Context, Render, TestAppContext, canvas, point, px, size};

    use super::*;

    #[derive(Debug, PartialEq, Eq)]
    struct MotionSample {
        phase: &'static str,
        framework: bool,
        engine: bool,
        combined: bool,
    }

    fn sample(phase: &'static str, cx: &App) -> MotionSample {
        MotionSample {
            phase,
            framework: GlobalState::global(cx).reduced_motion(),
            engine: cx.reduce_motion(),
            combined: crate::animation::reduced_motion(cx),
        }
    }

    struct MotionProbe {
        samples: Rc<RefCell<Vec<MotionSample>>>,
    }

    impl Render for MotionProbe {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            self.samples.borrow_mut().push(sample("render", cx));

            let prepaint_samples = self.samples.clone();
            let paint_samples = self.samples.clone();
            canvas(
                move |_, _, cx| {
                    prepaint_samples.borrow_mut().push(sample("prepaint", cx));
                },
                move |_, _, _, cx| {
                    paint_samples.borrow_mut().push(sample("paint", cx));
                },
            )
        }
    }

    #[gpui::test]
    fn reduced_motion_scope_surrounds_every_child_phase(cx: &mut TestAppContext) {
        cx.update(crate::init);
        cx.update(|cx| cx.set_reduce_motion(false));
        let cx = cx.add_empty_window();

        let samples = Rc::new(RefCell::new(Vec::new()));
        let probe = cx.new({
            let samples = samples.clone();
            move |_| MotionProbe { samples }
        });

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(100.)), |_, _| {
            ReducedMotionScope::new(true, probe)
        });

        assert_eq!(
            *samples.borrow(),
            vec![
                MotionSample {
                    phase: "render",
                    framework: true,
                    engine: false,
                    combined: true,
                },
                MotionSample {
                    phase: "prepaint",
                    framework: true,
                    engine: false,
                    combined: true,
                },
                MotionSample {
                    phase: "paint",
                    framework: true,
                    engine: false,
                    combined: true,
                },
            ]
        );

        cx.update(|_, cx| cx.set_reduce_motion(true));
        let engine_samples = Rc::new(RefCell::new(Vec::new()));
        let engine_probe = cx.new({
            let samples = engine_samples.clone();
            move |_| MotionProbe { samples }
        });

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(100.)), |_, _| {
            ReducedMotionScope::new(false, engine_probe)
        });

        assert_eq!(
            *engine_samples.borrow(),
            vec![
                MotionSample {
                    phase: "render",
                    framework: false,
                    engine: true,
                    combined: true,
                },
                MotionSample {
                    phase: "prepaint",
                    framework: false,
                    engine: true,
                    combined: true,
                },
                MotionSample {
                    phase: "paint",
                    framework: false,
                    engine: true,
                    combined: true,
                },
            ]
        );
    }
}
