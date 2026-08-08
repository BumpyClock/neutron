//! FloatingSidebar combines SidebarShell and Sidebar with internal resize handling.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    App, Element, ElementId, Hsla, IntoElement, MouseMoveEvent, MouseUpEvent, ParentElement,
    Pixels, RenderOnce, SharedString, Style, StyleRefinement, Styled, Window, px,
};

use crate::{
    ActiveTheme, ElevationToken, Side, StyledExt,
    animation::{PresenceOptions, keyed_presence, theme_animation},
    global_state::GlobalState,
    sidebar::{COLLAPSED_WIDTH, DEFAULT_WIDTH, Sidebar, SidebarItem},
    sidebar_shell::SidebarShell,
};

/// Default values for floating sidebar configuration.
const DEFAULT_MIN_WIDTH: Pixels = px(200.0);
const DEFAULT_MAX_WIDTH: Pixels = px(400.0);
const DEFAULT_RESIZER_WIDTH: Pixels = px(8.0);
const DEFAULT_INSET: Pixels = px(8.0);

#[derive(Clone)]
struct FloatingSidebarState {
    expanded_width: Pixels,
    visual_width: Rc<Cell<Pixels>>,
    transition_from: Pixels,
    transition_duration_ms: u16,
    transition_generation: u64,
    last_collapsed: bool,
    resizing: bool,
    drag_origin_x: Pixels,
    drag_origin_width: Pixels,
}

impl FloatingSidebarState {
    fn new(width: Pixels, collapsed: bool) -> Self {
        let visual_width = if collapsed { COLLAPSED_WIDTH } else { width };
        Self {
            expanded_width: width,
            visual_width: Rc::new(Cell::new(visual_width)),
            transition_from: visual_width,
            transition_duration_ms: 0,
            transition_generation: 0,
            last_collapsed: collapsed,
            resizing: false,
            drag_origin_x: px(0.0),
            drag_origin_width: width,
        }
    }

    fn begin_width_transition(
        &mut self,
        collapsed: bool,
        target_width: Pixels,
        expanded_width: Pixels,
        base_duration_ms: u16,
    ) {
        if self.last_collapsed == collapsed {
            return;
        }

        self.last_collapsed = collapsed;
        self.transition_from = self.visual_width.get();
        self.transition_duration_ms = scaled_width_transition_duration_ms(
            self.transition_from,
            target_width,
            expanded_width,
            base_duration_ms,
        );
        self.transition_generation += 1;
    }
}

fn scaled_width_transition_duration_ms(
    from: Pixels,
    target: Pixels,
    expanded_width: Pixels,
    base_duration_ms: u16,
) -> u16 {
    let full_distance = (expanded_width - COLLAPSED_WIDTH).as_f32().abs();
    if full_distance <= f32::EPSILON {
        return 1;
    }

    let remaining_distance = (target - from).as_f32().abs().min(full_distance);
    ((f32::from(base_duration_ms) * remaining_distance / full_distance).round() as u16).max(1)
}

/// A floating sidebar that composes SidebarShell and Sidebar with internal resize handling.
///
/// By default, the sidebar uses the theme's panel elevation. Explicit elevation overrides may
/// require a larger inset near window edges to avoid native-window clipping.
///
/// Collapse and expand transitions use the theme's point-to-point motion and become immediate
/// when reduced motion is enabled.
#[derive(IntoElement)]
pub struct FloatingSidebar<E: SidebarItem + 'static> {
    id: ElementId,
    sidebar: Sidebar<E>,
    style: StyleRefinement,
    side: Side,
    collapsed: bool,
    width: Pixels,
    min_width: Pixels,
    max_width: Pixels,
    resizer_width: Pixels,
    resizer_hover_bg: Option<Hsla>,
    inset: Option<Pixels>,
    top_inset: Pixels,
    blur_enabled: Option<bool>,
    elevation: Option<ElevationToken>,
    on_resize_end: Option<Rc<dyn Fn(Pixels, &mut Window, &mut App)>>,
}

impl<E: SidebarItem> FloatingSidebar<E> {
    /// Create a new FloatingSidebar with the given ID.
    pub fn new(id: impl Into<ElementId>) -> Self {
        let id = id.into();
        Self {
            id: id.clone(),
            sidebar: Sidebar::new(id),
            style: StyleRefinement::default(),
            side: Side::Left,
            collapsed: false,
            width: DEFAULT_WIDTH,
            min_width: DEFAULT_MIN_WIDTH,
            max_width: DEFAULT_MAX_WIDTH,
            resizer_width: DEFAULT_RESIZER_WIDTH,
            resizer_hover_bg: None,
            inset: Some(DEFAULT_INSET),
            top_inset: px(0.0),
            blur_enabled: None,
            elevation: None,
            on_resize_end: None,
        }
    }

    /// Set the side of the floating sidebar.
    ///
    /// Default is `Side::Left`.
    pub fn side(mut self, side: Side) -> Self {
        self.side = side;
        self
    }

    /// Set the sidebar to be collapsible, default is true.
    pub fn collapsible(mut self, collapsible: bool) -> Self {
        self.sidebar = self.sidebar.collapsible(collapsible);
        self
    }

    /// Set the sidebar to be collapsed.
    pub fn collapsed(mut self, collapsed: bool) -> Self {
        self.collapsed = collapsed;
        self
    }

    /// Set the expanded width of the floating sidebar.
    ///
    /// The value is clamped to the current `[min_width, max_width]` constraints.
    pub fn width(mut self, width: impl Into<Pixels>) -> Self {
        self.width = width.into().max(self.min_width).min(self.max_width);
        self
    }

    /// Set the minimum width constraint for resizing.
    pub fn min_width(mut self, width: impl Into<Pixels>) -> Self {
        self.min_width = width.into();
        if self.min_width > self.max_width {
            self.max_width = self.min_width;
        }
        self.width = self.width.max(self.min_width).min(self.max_width);
        self
    }

    /// Set the maximum width constraint for resizing.
    pub fn max_width(mut self, width: impl Into<Pixels>) -> Self {
        self.max_width = width.into();
        if self.max_width < self.min_width {
            self.min_width = self.max_width;
        }
        self.width = self.width.max(self.min_width).min(self.max_width);
        self
    }

    /// Set the width of the resize handle.
    pub fn resizer_width(mut self, width: impl Into<Pixels>) -> Self {
        self.resizer_width = width.into();
        self
    }

    /// Set the hover background color for the resize handle.
    pub fn resizer_hover_bg(mut self, color: impl Into<Hsla>) -> Self {
        self.resizer_hover_bg = Some(color.into());
        self
    }

    /// Set the inset from window edges.
    ///
    /// Default is 8px.
    pub fn inset(mut self, inset: impl Into<Pixels>) -> Self {
        self.inset = Some(inset.into());
        self
    }

    /// Set additional top inset on top of the base inset.
    pub fn top_inset(mut self, inset: impl Into<Pixels>) -> Self {
        self.top_inset = inset.into();
        self
    }

    /// Explicitly set whether blur effects are enabled for the glass surface.
    pub fn blur_enabled(mut self, enabled: bool) -> Self {
        self.blur_enabled = Some(enabled);
        self
    }

    /// Set the shadow elevation level for the complete sidebar panel.
    ///
    /// When not set, the theme's panel elevation is used. Large elevations may require a larger
    /// inset near window edges to avoid native-window clipping.
    pub fn elevation(mut self, elevation: ElevationToken) -> Self {
        self.elevation = Some(elevation);
        self
    }

    /// Set a callback invoked when resizing ends with the final width.
    pub fn on_resize_end(
        mut self,
        callback: impl Fn(Pixels, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_resize_end = Some(Rc::new(callback));
        self
    }

    /// Set the header of the sidebar.
    pub fn header(mut self, header: impl IntoElement) -> Self {
        self.sidebar = self.sidebar.header(header);
        self
    }

    /// Set a dynamic header that receives the visual collapsed state.
    pub fn header_with<F, H>(mut self, builder: F) -> Self
    where
        F: Fn(bool, &mut Window, &mut App) -> H + 'static,
        H: IntoElement,
    {
        self.sidebar = self.sidebar.header_with(builder);
        self
    }

    /// Set the footer of the sidebar.
    pub fn footer(mut self, footer: impl IntoElement) -> Self {
        self.sidebar = self.sidebar.footer(footer);
        self
    }

    /// Set a dynamic footer that receives the visual collapsed state.
    pub fn footer_with<F, H>(mut self, builder: F) -> Self
    where
        F: Fn(bool, &mut Window, &mut App) -> H + 'static,
        H: IntoElement,
    {
        self.sidebar = self.sidebar.footer_with(builder);
        self
    }

    /// Add a child element to the sidebar, the child must implement `SidebarItem`.
    pub fn child(mut self, child: E) -> Self {
        self.sidebar = self.sidebar.child(child);
        self
    }

    /// Add multiple children to the sidebar, the children must implement `SidebarItem`.
    pub fn children(mut self, children: impl IntoIterator<Item = E>) -> Self {
        self.sidebar = self.sidebar.children(children);
        self
    }
}

impl<E: SidebarItem> Styled for FloatingSidebar<E> {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl<E: SidebarItem> RenderOnce for FloatingSidebar<E> {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let FloatingSidebar {
            id,
            sidebar,
            style,
            side,
            collapsed,
            width,
            min_width,
            max_width,
            resizer_width,
            resizer_hover_bg,
            inset,
            top_inset,
            blur_enabled,
            elevation,
            on_resize_end,
        } = self;

        let initial_width = width.max(min_width).min(max_width);
        let state_key = SharedString::from(format!("{}-floating-sidebar-state", id));
        let state = window.use_keyed_state(state_key, cx, |_, _| {
            FloatingSidebarState::new(initial_width, collapsed)
        });

        let expanded_width = {
            let current_width = state.read(cx).expanded_width;
            let clamped_width = current_width.max(min_width).min(max_width);
            if clamped_width != current_width {
                state.update(cx, |state, cx| {
                    state.expanded_width = clamped_width;
                    if !state.last_collapsed {
                        state.visual_width.set(clamped_width);
                    }
                    cx.notify();
                });
            }
            clamped_width
        };

        let reduced_motion = GlobalState::global(cx).reduced_motion();
        let motion = cx.theme().motion.clone();
        let width_spring_duration_ms = motion.enter_duration_ms;
        let target_width = if collapsed {
            COLLAPSED_WIDTH
        } else {
            expanded_width
        };
        let base_duration_ms = if collapsed {
            width_spring_duration_ms.max(motion.exit_duration_ms)
        } else {
            width_spring_duration_ms.max(motion.enter_duration_ms)
        };
        state.update(cx, |state, _| {
            state.begin_width_transition(collapsed, target_width, expanded_width, base_duration_ms);
        });
        let (from_width, width_duration_ms, transition_generation, visual_width) = {
            let state = state.read(cx);
            (
                state.transition_from,
                state.transition_duration_ms,
                state.transition_generation,
                state.visual_width.clone(),
            )
        };
        let width_presence = keyed_presence(
            SharedString::from(format!("{}-floating-sidebar-width", id)),
            !collapsed,
            !reduced_motion,
            Duration::from_millis(u64::from(width_duration_ms.max(1))),
            Duration::from_millis(u64::from(width_duration_ms.max(1))),
            PresenceOptions::default(),
            window,
            cx,
        );
        let transition_active = !reduced_motion && width_presence.transition_active();
        if !transition_active {
            visual_width.set(target_width);
        }
        let visual_collapsed = collapsed && !transition_active;
        let resizer_width = if collapsed || transition_active {
            px(0.0)
        } else {
            resizer_width
        };
        let resizer_hover_bg =
            resizer_hover_bg.unwrap_or_else(|| cx.theme().sidebar_primary.opacity(0.18));

        let end_resize = {
            let state = state.clone();
            let on_resize_end = on_resize_end.clone();
            Rc::new(move |window: &mut Window, cx: &mut App| {
                let is_resizing = state.read(cx).resizing;
                if !is_resizing {
                    return;
                }
                let width = state.read(cx).expanded_width;
                state.update(cx, |state, cx| {
                    state.resizing = false;
                    cx.notify();
                });
                if let Some(callback) = on_resize_end.as_ref() {
                    callback(width, window, cx);
                }
            })
        };

        let on_resize_start = {
            let state = state.clone();
            move |width: Pixels, x: Pixels, _window: &mut Window, cx: &mut App| {
                if collapsed {
                    return;
                }
                let width = width.max(min_width).min(max_width);
                state.update(cx, |state, cx| {
                    state.expanded_width = width;
                    state.visual_width.set(width);
                    state.drag_origin_width = width;
                    state.drag_origin_x = x;
                    state.resizing = true;
                    cx.notify();
                });
            }
        };

        let resize_tracker = FloatingSidebarResizeTracker {
            state: state.clone(),
            side,
            min_width,
            max_width,
            collapsed,
            end_resize: end_resize.clone(),
        };

        let mut shell = if side.is_left() {
            SidebarShell::left(target_width)
        } else {
            SidebarShell::right(target_width)
        };

        shell = shell
            .min_width(min_width)
            .max_width(max_width)
            .resizer_width(resizer_width)
            .top_inset(top_inset)
            .on_resize_start(on_resize_start)
            .on_resize_end(move |window, cx| {
                end_resize(window, cx);
            });

        if transition_active {
            if let Some(animation) =
                theme_animation(width_duration_ms, &motion.standard_easing, reduced_motion)
            {
                shell = shell.animate_width_from(
                    from_width,
                    SharedString::from(format!(
                        "{}-floating-sidebar-width-{}",
                        id, transition_generation
                    )),
                    animation,
                    visual_width,
                );
            }
        }
        if let Some(elevation) = elevation {
            shell = shell.elevation(elevation);
        }
        shell = shell.resizer_hover_bg(resizer_hover_bg);
        if let Some(inset) = inset {
            shell = shell.inset(inset);
        }
        if let Some(blur_enabled) = blur_enabled {
            shell = shell.blur_enabled(blur_enabled);
        }

        shell
            .child(
                sidebar
                    .side(side)
                    .collapsed(visual_collapsed)
                    .width(expanded_width)
                    .animate_width(false)
                    .bg(gpui::transparent_black())
                    .refine_style(&style),
            )
            .child(resize_tracker)
    }
}

struct FloatingSidebarResizeTracker {
    state: gpui::Entity<FloatingSidebarState>,
    side: Side,
    min_width: Pixels,
    max_width: Pixels,
    collapsed: bool,
    end_resize: Rc<dyn Fn(&mut Window, &mut App)>,
}

impl IntoElement for FloatingSidebarResizeTracker {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for FloatingSidebarResizeTracker {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui::LayoutId, Self::RequestLayoutState) {
        (window.request_layout(Style::default(), None, cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        _: gpui::Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
        ()
    }

    fn paint(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        _: gpui::Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        _cx: &mut App,
    ) {
        window.on_mouse_event({
            let state = self.state.clone();
            let side = self.side;
            let min_width = self.min_width;
            let max_width = self.max_width;
            let collapsed = self.collapsed;
            move |event: &MouseMoveEvent, phase, _window, cx| {
                if !phase.bubble() || collapsed {
                    return;
                }
                let (resizing, start_x, start_width, current_width) = {
                    let state = state.read(cx);
                    (
                        state.resizing,
                        state.drag_origin_x,
                        state.drag_origin_width,
                        state.expanded_width,
                    )
                };
                if !resizing {
                    return;
                }
                let delta = if side.is_left() {
                    event.position.x - start_x
                } else {
                    start_x - event.position.x
                };
                let next_width = (start_width + delta).max(min_width).min(max_width);
                if next_width == current_width {
                    return;
                }
                state.update(cx, |state, cx| {
                    state.expanded_width = next_width;
                    state.visual_width.set(next_width);
                    cx.notify();
                });
            }
        });

        window.on_mouse_event({
            let state = self.state.clone();
            let end_resize = self.end_resize.clone();
            move |_: &MouseUpEvent, phase, window, cx| {
                if !phase.bubble() {
                    return;
                }
                if state.read(cx).resizing {
                    end_resize(window, cx);
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sidebar::SidebarMenu;
    use gpui::hsla;

    #[gpui::test]
    fn test_floating_sidebar_builder(_cx: &mut gpui::TestAppContext) {
        let default_sidebar = FloatingSidebar::<SidebarMenu>::new("default-floating-sidebar");
        assert_eq!(default_sidebar.elevation, None);
        assert_eq!(default_sidebar.width, DEFAULT_WIDTH);
        assert_eq!(default_sidebar.inset, Some(DEFAULT_INSET));
        assert_eq!(default_sidebar.resizer_width, DEFAULT_RESIZER_WIDTH);
        assert_eq!(default_sidebar.blur_enabled, None);

        let sidebar = FloatingSidebar::<SidebarMenu>::new("floating-sidebar")
            .side(Side::Right)
            .collapsed(true)
            .width(px(280.0))
            .min_width(px(220.0))
            .max_width(px(420.0))
            .resizer_width(px(8.0))
            .resizer_hover_bg(hsla(0.0, 0.0, 0.0, 0.2))
            .inset(px(6.0))
            .top_inset(px(12.0))
            .blur_enabled(false)
            .elevation(ElevationToken::Md)
            .on_resize_end(|_, _, _| {});

        assert_eq!(sidebar.side, Side::Right);
        assert!(sidebar.collapsed);
        assert_eq!(sidebar.width, px(280.0));
        assert_eq!(sidebar.min_width, px(220.0));
        assert_eq!(sidebar.max_width, px(420.0));
        assert_eq!(sidebar.resizer_width, px(8.0));
        assert!(sidebar.resizer_hover_bg.is_some());
        assert_eq!(sidebar.inset, Some(px(6.0)));
        assert_eq!(sidebar.top_inset, px(12.0));
        assert_eq!(sidebar.blur_enabled, Some(false));
        assert_eq!(sidebar.elevation, Some(ElevationToken::Md));
        assert!(sidebar.on_resize_end.is_some());

        let no_shadow =
            FloatingSidebar::<SidebarMenu>::new("no-shadow").elevation(ElevationToken::None);
        assert_eq!(no_shadow.elevation, Some(ElevationToken::None));

        let large_shadow =
            FloatingSidebar::<SidebarMenu>::new("large-shadow").elevation(ElevationToken::Lg);
        assert_eq!(large_shadow.elevation, Some(ElevationToken::Lg));
    }

    #[gpui::test]
    fn test_floating_sidebar_width_constraints(_cx: &mut gpui::TestAppContext) {
        let raised_min = FloatingSidebar::<SidebarMenu>::new("raised-min").min_width(px(450.0));
        assert_eq!(raised_min.min_width, px(450.0));
        assert_eq!(raised_min.max_width, px(450.0));
        assert_eq!(raised_min.width, px(450.0));

        let lowered_max = FloatingSidebar::<SidebarMenu>::new("lowered-max").max_width(px(180.0));
        assert_eq!(lowered_max.min_width, px(180.0));
        assert_eq!(lowered_max.max_width, px(180.0));
        assert_eq!(lowered_max.width, px(180.0));

        let clamped_width = FloatingSidebar::<SidebarMenu>::new("clamped-width")
            .min_width(px(220.0))
            .max_width(px(420.0))
            .width(px(100.0));
        assert_eq!(clamped_width.width, px(220.0));
    }

    #[test]
    fn test_width_transition_reversal_starts_from_current_visual_width() {
        let mut state = FloatingSidebarState::new(px(260.0), false);

        state.visual_width.set(px(172.0));
        state.begin_width_transition(true, COLLAPSED_WIDTH, px(260.0), 300);
        assert_eq!(state.transition_from, px(172.0));
        assert_eq!(state.transition_generation, 1);
        assert!(state.transition_duration_ms < 300);

        state.visual_width.set(px(116.0));
        state.begin_width_transition(false, px(260.0), px(260.0), 300);
        assert_eq!(state.transition_from, px(116.0));
        assert_eq!(state.transition_generation, 2);
        assert!(state.transition_duration_ms < 300);
    }
}
