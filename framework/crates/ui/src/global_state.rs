use gpui::{App, Entity, Global, Pixels, px};

use crate::text::TextViewState;

pub(crate) fn init(cx: &mut App) {
    cx.set_global(GlobalState::new());
}

impl Global for GlobalState {}

pub struct GlobalState {
    pub(crate) text_view_state_stack: Vec<Entity<TextViewState>>,
    /// Stack for blur_enabled context values.
    blur_enabled_stack: Vec<bool>,
    /// Stack for reduced_motion context values.
    reduced_motion_stack: Vec<bool>,
    /// Stack for layer_closing context values (see [`crate::ClosingScope`]).
    layer_closing_stack: Vec<bool>,
    /// Stack for floating inset values.
    floating_inset_stack: Vec<Pixels>,
}

impl GlobalState {
    pub(crate) fn new() -> Self {
        Self {
            text_view_state_stack: Vec::new(),
            blur_enabled_stack: vec![true],    // Default to enabled
            reduced_motion_stack: vec![false], // Default to not reduced
            layer_closing_stack: vec![false],  // Default to not closing
            floating_inset_stack: vec![px(4.0)],
        }
    }

    pub fn global(cx: &App) -> &Self {
        cx.global::<Self>()
    }

    pub fn global_mut(cx: &mut App) -> &mut Self {
        cx.global_mut::<Self>()
    }

    pub(crate) fn text_view_state(&self) -> Option<&Entity<TextViewState>> {
        self.text_view_state_stack.last()
    }

    /// Returns whether blur effects are enabled (from the context stack).
    pub fn blur_enabled(&self) -> bool {
        self.blur_enabled_stack.last().copied().unwrap_or(true)
    }

    /// Push a blur_enabled value onto the context stack.
    pub fn push_blur_enabled(&mut self, enabled: bool) {
        self.blur_enabled_stack.push(enabled);
    }

    /// Pop a blur_enabled value from the context stack.
    pub fn pop_blur_enabled(&mut self) {
        if self.blur_enabled_stack.len() > 1 {
            self.blur_enabled_stack.pop();
        }
    }

    /// Sets the base blur_enabled value (replaces the bottom of the stack).
    #[allow(dead_code)]
    pub fn set_blur_enabled(&mut self, enabled: bool) {
        if let Some(first) = self.blur_enabled_stack.first_mut() {
            *first = enabled;
        }
    }

    /// Returns whether reduced motion is enabled (from the context stack).
    #[allow(dead_code)]
    pub fn reduced_motion(&self) -> bool {
        self.reduced_motion_stack.last().copied().unwrap_or(false)
    }

    /// Push a reduced_motion value onto the context stack.
    pub fn push_reduced_motion(&mut self, reduced: bool) {
        self.reduced_motion_stack.push(reduced);
    }

    /// Pop a reduced_motion value from the context stack.
    pub fn pop_reduced_motion(&mut self) {
        if self.reduced_motion_stack.len() > 1 {
            self.reduced_motion_stack.pop();
        }
    }

    /// Sets the base reduced_motion value (replaces the bottom of the stack).
    #[allow(dead_code)]
    pub fn set_reduced_motion(&mut self, reduced: bool) {
        if let Some(first) = self.reduced_motion_stack.first_mut() {
            *first = reduced;
        }
    }

    /// Returns whether the enclosing root layer (dialog, sheet) is closing.
    pub fn layer_closing(&self) -> bool {
        self.layer_closing_stack.last().copied().unwrap_or(false)
    }

    /// Push a layer_closing value onto the context stack.
    pub(crate) fn push_layer_closing(&mut self, closing: bool) {
        self.layer_closing_stack.push(closing);
    }

    /// Pop a layer_closing value from the context stack.
    pub(crate) fn pop_layer_closing(&mut self) {
        if self.layer_closing_stack.len() > 1 {
            self.layer_closing_stack.pop();
        }
    }

    /// Returns the current floating inset from the context stack.
    pub fn floating_inset(&self) -> Pixels {
        self.floating_inset_stack.last().copied().unwrap_or(px(4.0))
    }

    /// Push a floating inset value onto the context stack.
    pub fn push_floating_inset(&mut self, inset: Pixels) {
        self.floating_inset_stack.push(inset);
    }

    /// Pop a floating inset value from the context stack.
    pub fn pop_floating_inset(&mut self) {
        if self.floating_inset_stack.len() > 1 {
            self.floating_inset_stack.pop();
        }
    }
}
