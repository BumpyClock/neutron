//! SidebarShell component providing a resizable sidebar panel with glass effects.
//!
//! This module provides a reusable sidebar shell that handles:
//! - Resizable panel with configurable min/max width constraints
//! - Theme-controlled panel shadow elevation
//! - Glass surface effects via SurfacePreset
//! - Draggable resize handle on the inner edge
//! - Support for left/right placement
//!
//! # Resize Model
//!
//! SidebarShell uses a consumer-managed resize model. The component provides:
//! - `on_resize_start`: Called when the user starts dragging the resizer (mouse down)
//! - `on_resize_end`: Called when the user stops dragging (mouse up)
//!
//! The consumer is responsible for:
//! - Tracking drag state (resizing: bool, start_x, start_width)
//! - Handling mouse move events at the window/root level
//! - Calculating and applying the new width
//!
//! This model is necessary because GPUI's `RenderOnce` components cannot use
//! `cx.listener()` or `window.listener_for()` patterns.
//!
//! # Example
//!
//! ```rust,ignore
//! use ui::sidebar_shell::SidebarShell;
//!
//! SidebarShell::left(px(self.sidebar_width))
//!     .min_width(px(200.0))
//!     .max_width(px(400.0))
//!     .on_resize_start(move |width, x, _window, cx| {
//!         // Store resize start state: width, x position
//!     })
//!     .on_resize_end(move |_window, cx| {
//!         // Clear resize state
//!     })
//!     .child(sidebar_content)
//! ```

use std::{cell::Cell, rc::Rc};

use gpui::{
    Animation, AnimationExt as _, AnyElement, App, BoxShadow, Hsla, InteractiveElement,
    IntoElement, ParentElement, Pixels, RenderOnce, SharedString, StyleRefinement, Styled, Window,
    div, hsla, point, prelude::FluentBuilder, px,
};
use smallvec::SmallVec;

use crate::{
    ActiveTheme, ElevationToken, Side, StyledExt, SurfaceContext, SurfacePreset,
    global_state::GlobalState,
};

/// Default values for sidebar shell configuration.
const DEFAULT_MIN_WIDTH: f32 = 200.0;
const DEFAULT_MAX_WIDTH: f32 = 400.0;
const DEFAULT_RESIZER_WIDTH: f32 = 6.0;

struct SidebarShellWidthTransition {
    from: Pixels,
    animation_id: SharedString,
    animation: Animation,
    current_width: Rc<Cell<Pixels>>,
}

/// Creates a 3-layer shadow effect for elevated sidebar panels.
///
/// This shadow configuration provides a natural depth effect with:
/// - Subtle near-edge shadow (4% opacity)
/// - Medium distance shadow (8% opacity)
/// - Far distance shadow (12% opacity)
pub fn sidebar_shadow() -> Vec<BoxShadow> {
    vec![
        BoxShadow {
            color: hsla(0., 0., 0., 0.04),
            offset: point(px(0.0), px(1.0)),
            blur_radius: px(6.0),
            spread_radius: px(0.0),
            inset: false,
        },
        BoxShadow {
            color: hsla(0., 0., 0., 0.08),
            offset: point(px(0.0), px(8.0)),
            blur_radius: px(22.0),
            spread_radius: px(0.0),
            inset: false,
        },
        BoxShadow {
            color: hsla(0., 0., 0., 0.12),
            offset: point(px(0.0), px(22.0)),
            blur_radius: px(54.0),
            spread_radius: px(0.0),
            inset: false,
        },
    ]
}

/// A resizable sidebar panel with a theme-controlled shadow and glass surface effects.
///
/// SidebarShell provides a container for sidebar content that handles:
/// - Absolute positioning with configurable inset from window edges
/// - A single complete panel shadow, controlled by its elevation
/// - Glass blur and noise effects via SurfacePreset::panel()
/// - Draggable resize handle with hover feedback
///
/// By default, the shell uses the theme's panel elevation. Explicit elevation overrides may
/// require a larger inset near window edges to avoid native-window clipping.
///
/// The component uses a builder pattern for configuration and implements
/// `ParentElement` for adding child content, `Styled` for style refinement,
/// and `RenderOnce` for rendering.
///
/// # Resize Model
///
/// Uses consumer-managed resize. The component fires `on_resize_start` and
/// `on_resize_end` callbacks, but the consumer must handle mouse move events
/// at the window level to track the actual resize operation.
///
/// # Layout Structure
///
/// ```text
/// +-- Outer Container (absolute, inset from edges) --+
/// |  +-- Surface (shadow + glass effects) --------+ |
/// |  |                                            | |
/// |  |  [Child Content]                           | |
/// |  |                                            | |
/// |  +--------------------------------------------+ |
/// |  [Resizer Handle] (on inner edge)              |
/// +------------------------------------------------+
/// ```
#[derive(IntoElement)]
pub struct SidebarShell {
    /// Current width of the sidebar in pixels.
    width: Pixels,
    /// Minimum width constraint for resizing.
    min_width: Pixels,
    /// Maximum width constraint for resizing.
    max_width: Pixels,
    /// Width of the resize handle in pixels.
    resizer_width: Pixels,
    /// Optional override for resizer hover background color.
    resizer_hover_bg: Option<Hsla>,
    /// Callback invoked when resize starts (mouse down on resizer).
    /// Receives: (current_width, mouse_x, window, cx)
    on_resize_start: Option<Rc<dyn Fn(Pixels, Pixels, &mut Window, &mut App)>>,
    /// Callback invoked when resize ends (mouse up).
    on_resize_end: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
    /// Optional shadow elevation override for the complete sidebar panel.
    /// If `None`, the theme's panel elevation is used.
    elevation: Option<ElevationToken>,
    /// Placement side (left or right).
    side: Side,
    /// Inset from window edges in pixels. If `None`, inherits from context.
    inset: Option<Pixels>,
    /// Additional top inset applied above the inherited/explicit inset.
    top_inset: Pixels,
    /// Whether blur effects are enabled for the glass surface.
    /// If `None`, the value is inherited from the parent context (e.g., WindowShell).
    /// If `Some(value)`, the explicit value is used.
    blur_enabled: Option<bool>,
    /// Optional width transition applied to the complete shell.
    width_transition: Option<SidebarShellWidthTransition>,
    /// Child elements rendered inside the surface.
    children: SmallVec<[AnyElement; 1]>,
    /// Style refinement for the outer container.
    style: StyleRefinement,
}

impl SidebarShell {
    /// Creates a new left-aligned sidebar shell with the specified width.
    ///
    /// # Arguments
    ///
    /// * `width` - The initial width of the sidebar in pixels.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let sidebar = SidebarShell::left(px(260.0));
    /// ```
    pub fn left(width: impl Into<Pixels>) -> Self {
        Self::new(width, Side::Left)
    }

    /// Creates a new right-aligned sidebar shell with the specified width.
    ///
    /// # Arguments
    ///
    /// * `width` - The initial width of the sidebar in pixels.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let sidebar = SidebarShell::right(px(300.0));
    /// ```
    pub fn right(width: impl Into<Pixels>) -> Self {
        Self::new(width, Side::Right)
    }

    fn new(width: impl Into<Pixels>, side: Side) -> Self {
        Self {
            width: width.into(),
            min_width: px(DEFAULT_MIN_WIDTH),
            max_width: px(DEFAULT_MAX_WIDTH),
            resizer_width: px(DEFAULT_RESIZER_WIDTH),
            resizer_hover_bg: None,
            on_resize_start: None,
            on_resize_end: None,
            elevation: None,
            side,
            inset: None,
            top_inset: px(0.0),
            blur_enabled: None, // Inherit from context by default
            width_transition: None,
            children: SmallVec::new(),
            style: StyleRefinement::default(),
        }
    }

    /// Sets the minimum width constraint for resizing.
    ///
    /// The sidebar cannot be resized smaller than this width.
    /// Default: 200px.
    pub fn min_width(mut self, width: impl Into<Pixels>) -> Self {
        self.min_width = width.into();
        self
    }

    /// Sets the maximum width constraint for resizing.
    ///
    /// The sidebar cannot be resized larger than this width.
    /// Default: 400px.
    pub fn max_width(mut self, width: impl Into<Pixels>) -> Self {
        self.max_width = width.into();
        self
    }

    /// Sets the width of the resize handle.
    ///
    /// Default: 6px.
    pub fn resizer_width(mut self, width: impl Into<Pixels>) -> Self {
        self.resizer_width = width.into();
        self
    }

    /// Sets the hover background color for the resize handle.
    ///
    /// If not set, defaults to theme foreground at 20% opacity.
    pub fn resizer_hover_bg(mut self, color: impl Into<Hsla>) -> Self {
        self.resizer_hover_bg = Some(color.into());
        self
    }

    /// Sets the callback invoked when resize starts (mouse down on resizer).
    ///
    /// The callback receives the current width and mouse X position.
    /// The consumer should store these to calculate width delta during mouse move.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// SidebarShell::left(px(260.0))
    ///     .on_resize_start(|width, x, window, cx| {
    ///         // Store: resizing = true, start_width = width, start_x = x
    ///     })
    /// ```
    pub fn on_resize_start(
        mut self,
        callback: impl Fn(Pixels, Pixels, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_resize_start = Some(Rc::new(callback));
        self
    }

    /// Sets the callback invoked when resize ends (mouse up).
    ///
    /// The consumer should clear their resize state.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// SidebarShell::left(px(260.0))
    ///     .on_resize_end(|window, cx| {
    ///         // Store: resizing = false
    ///     })
    /// ```
    pub fn on_resize_end(mut self, callback: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_resize_end = Some(Rc::new(callback));
        self
    }

    /// Sets the inset from window edges.
    ///
    /// This creates space between the sidebar and the window bounds.
    /// Default: inherited from context (4px when no parent scope provides a value).
    pub fn inset(mut self, inset: impl Into<Pixels>) -> Self {
        self.inset = Some(inset.into());
        self
    }

    /// Sets additional top inset on top of the base inset.
    ///
    /// Default: 0px.
    pub fn top_inset(mut self, inset: impl Into<Pixels>) -> Self {
        self.top_inset = inset.into();
        self
    }

    /// Explicitly sets whether blur effects are enabled for the glass surface.
    ///
    /// When set, this value overrides any inherited context from the parent
    /// `WindowShell`. When not called, the sidebar inherits `blur_enabled`
    /// from the parent context.
    ///
    /// When disabled, the surface will not render backdrop blur or noise
    /// overlays, which can improve performance on systems that don't
    /// support blur effects well.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // Inherit from WindowShell context (default behavior)
    /// SidebarShell::left(px(260.0))
    ///     .child(content)
    ///
    /// // Explicitly override to disable blur
    /// SidebarShell::left(px(260.0))
    ///     .blur_enabled(false)
    ///     .child(content)
    /// ```
    pub fn blur_enabled(mut self, enabled: bool) -> Self {
        self.blur_enabled = Some(enabled);
        self
    }

    /// Sets the shadow elevation level for the complete sidebar panel.
    ///
    /// Controls the single panel shadow using the theme's elevation system.
    /// When not set, the theme's panel elevation is used. Large elevations may require a larger
    /// inset near window edges to avoid native-window clipping.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// SidebarShell::left(px(260.0))
    ///     .elevation(ElevationToken::Md)  // Medium shadow
    ///     .child(content)
    /// ```
    pub fn elevation(mut self, elevation: ElevationToken) -> Self {
        self.elevation = Some(elevation);
        self
    }

    pub(crate) fn animate_width_from(
        mut self,
        from: Pixels,
        animation_id: impl Into<SharedString>,
        animation: Animation,
        current_width: Rc<Cell<Pixels>>,
    ) -> Self {
        self.width_transition = Some(SidebarShellWidthTransition {
            from,
            animation_id: animation_id.into(),
            animation,
            current_width,
        });
        self
    }

    fn surface_preset(&self) -> SurfacePreset {
        let mut surface_preset = SurfacePreset::panel();
        if let Some(elevation) = self.elevation {
            surface_preset = surface_preset.with_elevation(elevation);
        }
        surface_preset
    }
}

impl ParentElement for SidebarShell {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl Styled for SidebarShell {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for SidebarShell {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let resizer_hover_bg = self
            .resizer_hover_bg
            .unwrap_or_else(|| cx.theme().foreground.alpha(0.20));

        let window_bounds = window.window_bounds().get_bounds();
        let window_height = window_bounds.size.height;
        let inset = self
            .inset
            .unwrap_or_else(|| GlobalState::global(cx).floating_inset());
        let top = inset + self.top_inset;
        let bottom = inset;
        let sidebar_height = (window_height - (top + bottom)).max(px(0.0));
        let sidebar_width = self.width;
        let surface_render_width = self
            .width_transition
            .as_ref()
            .map_or(sidebar_width, |transition| {
                sidebar_width.max(transition.from)
            });

        // Use explicit value if set, otherwise inherit from context
        let blur_enabled = self
            .blur_enabled
            .unwrap_or_else(|| GlobalState::global(cx).blur_enabled());

        let sidebar_surface = self
            .surface_preset()
            .wrap_with_bounds(
                div(),
                surface_render_width,
                sidebar_height,
                window,
                cx,
                SurfaceContext { blur_enabled },
            )
            .children(self.children)
            .id("sidebar-shell-surface")
            .size_full();

        let resizer_half = self.resizer_width / 2.0;

        let is_left = self.side.is_left();
        let on_resize_start = self.on_resize_start.clone();
        let on_resize_end = self.on_resize_end.clone();

        let outer = div()
            .id("sidebar-shell")
            .absolute()
            .top(top)
            .bottom(bottom)
            .w(self.width)
            .map(|el| {
                if is_left {
                    el.left(inset)
                } else {
                    el.right(inset)
                }
            })
            .child(sidebar_surface)
            .child(
                div()
                    .id("sidebar-shell-resizer")
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .when(is_left, |this| this.right(-resizer_half))
                    .when(!is_left, |this| this.left(-resizer_half))
                    .w(self.resizer_width)
                    .rounded(px(999.0))
                    .bg(gpui::transparent_black())
                    .cursor_col_resize()
                    .hover(move |s| s.bg(resizer_hover_bg))
                    .when_some(on_resize_start, move |el, callback| {
                        el.on_mouse_down(gpui::MouseButton::Left, move |event, window, cx| {
                            cx.stop_propagation();
                            callback(sidebar_width, event.position.x, window, cx);
                        })
                    })
                    .when_some(on_resize_end, move |el, callback| {
                        let callback_mouse_up = callback.clone();
                        el.on_mouse_up(gpui::MouseButton::Left, move |_event, window, cx| {
                            callback_mouse_up(window, cx);
                        })
                        .on_mouse_up_out(
                            gpui::MouseButton::Left,
                            move |_event, window, cx| {
                                callback(window, cx);
                            },
                        )
                    }),
            )
            .refine_style(&self.style);

        if let Some(transition) = self.width_transition {
            let SidebarShellWidthTransition {
                from,
                animation_id,
                animation,
                current_width,
            } = transition;
            outer
                .with_animation(animation_id, animation, move |this, delta| {
                    let width = from + (sidebar_width - from) * delta;
                    current_width.set(width);
                    this.w(width)
                })
                .into_any_element()
        } else {
            outer.into_any_element()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sidebar_shell_defaults_and_elevation_builder() {
        let default_shell = SidebarShell::left(px(260.0));
        assert_eq!(default_shell.elevation, None);
        assert_eq!(default_shell.min_width, px(DEFAULT_MIN_WIDTH));
        assert_eq!(default_shell.max_width, px(DEFAULT_MAX_WIDTH));
        assert_eq!(default_shell.resizer_width, px(DEFAULT_RESIZER_WIDTH));
        assert_eq!(default_shell.inset, None);
        assert_eq!(default_shell.side, Side::Left);
        assert!(default_shell.width_transition.is_none());

        let no_shadow = SidebarShell::left(px(260.0)).elevation(ElevationToken::None);
        assert_eq!(no_shadow.elevation, Some(ElevationToken::None));

        let large_shadow = SidebarShell::right(px(300.0))
            .inset(px(12.0))
            .resizer_width(px(8.0))
            .elevation(ElevationToken::Lg);
        assert_eq!(large_shadow.elevation, Some(ElevationToken::Lg));
        assert_eq!(large_shadow.inset, Some(px(12.0)));
        assert_eq!(large_shadow.resizer_width, px(8.0));
        assert_eq!(large_shadow.side, Side::Right);

        let animated = SidebarShell::left(px(48.0)).animate_width_from(
            px(260.0),
            "sidebar-shell-width",
            Animation::new(std::time::Duration::from_millis(187)),
            Rc::new(Cell::new(px(260.0))),
        );
        let transition = animated
            .width_transition
            .as_ref()
            .expect("width transition should be configured");
        assert_eq!(transition.from, px(260.0));
        assert_eq!(
            transition.animation.duration,
            std::time::Duration::from_millis(187)
        );
    }

    #[test]
    fn test_sidebar_shell_surface_preset_elevation() {
        let theme_default = SidebarShell::left(px(260.0)).surface_preset();
        assert_eq!(theme_default.elevation, ElevationToken::Lg);
        assert!(theme_default.use_theme_elevation_defaults);

        let no_shadow = SidebarShell::left(px(260.0))
            .elevation(ElevationToken::None)
            .surface_preset();
        assert_eq!(no_shadow.elevation, ElevationToken::None);
        assert!(!no_shadow.use_theme_elevation_defaults);

        let large_shadow = SidebarShell::left(px(260.0))
            .elevation(ElevationToken::Lg)
            .surface_preset();
        assert_eq!(large_shadow.elevation, ElevationToken::Lg);
        assert!(!large_shadow.use_theme_elevation_defaults);
    }
}
