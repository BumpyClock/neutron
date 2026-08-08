use std::rc::Rc;

use crate::{ActiveTheme, Icon, IconName, Sizable, StyledExt, h_flex};
use gpui::{
    AnyElement, App, ClickEvent, Context, Decorations, Edges, Hsla, InteractiveElement,
    IntoElement, MouseButton, ParentElement, Pixels, Render, RenderOnce,
    StatefulInteractiveElement as _, StyleRefinement, Styled, TitlebarOptions, Window,
    WindowControlArea, div, prelude::FluentBuilder as _, px,
};
#[cfg(target_os = "windows")]
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use smallvec::SmallVec;
#[cfg(target_os = "windows")]
use windows::Win32::{
    Foundation::{HWND, LPARAM, WPARAM},
    UI::{
        Input::KeyboardAndMouse::ReleaseCapture,
        WindowsAndMessaging::{HTCAPTION, PostMessageW, WM_NCLBUTTONDOWN},
    },
};

pub const TITLE_BAR_HEIGHT: Pixels = px(34.);
#[cfg(target_os = "macos")]
const DEFAULT_CONTENT_INSET_LEFT: Pixels = px(80.);
#[cfg(not(target_os = "macos"))]
const DEFAULT_CONTENT_INSET_LEFT: Pixels = px(8.);
const DEFAULT_CONTENT_INSET_RIGHT: Pixels = px(12.);

/// TitleBar used to customize the appearance of the title bar.
///
/// We can put some elements inside the title bar.
#[derive(IntoElement)]
pub struct TitleBar {
    style: StyleRefinement,
    children: SmallVec<[AnyElement; 1]>,
    on_close_window: Option<Rc<Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>>>,
    content_insets: Option<Edges<Pixels>>,
    content_inset_left: Option<Pixels>,
    content_inset_right: Option<Pixels>,
    safe_area_left: Pixels,
    safe_area_right: Pixels,
}

impl TitleBar {
    /// Create a new TitleBar.
    pub fn new() -> Self {
        Self {
            style: StyleRefinement::default(),
            children: SmallVec::new(),
            on_close_window: None,
            content_insets: None,
            content_inset_left: None,
            content_inset_right: None,
            safe_area_left: px(0.0),
            safe_area_right: px(0.0),
        }
    }

    /// Returns the default title bar options for compatible with the [`crate::TitleBar`].
    pub fn title_bar_options() -> TitlebarOptions {
        TitlebarOptions {
            title: None,
            appears_transparent: true,
            traffic_light_position: Some(gpui::point(px(9.0), px(9.0))),
        }
    }

    /// Add custom for close window event, default is None, then click X button will call `window.remove_window()`.
    /// Linux only, this will do nothing on other platforms.
    pub fn on_close_window(
        mut self,
        f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        if cfg!(target_os = "linux") {
            self.on_close_window = Some(Rc::new(Box::new(f)));
        }
        self
    }

    /// Set content insets for title bar content area.
    pub fn content_insets(mut self, insets: Edges<Pixels>) -> Self {
        self.content_insets = Some(insets);
        self.content_inset_left = None;
        self.content_inset_right = None;
        self
    }

    /// Set left content inset for title bar content area.
    pub fn content_inset_left(mut self, inset: impl Into<Pixels>) -> Self {
        self.content_inset_left = Some(inset.into());
        self
    }

    /// Set right content inset for title bar content area.
    pub fn content_inset_right(mut self, inset: impl Into<Pixels>) -> Self {
        self.content_inset_right = Some(inset.into());
        self
    }

    /// Set additional left safe-area offset for title bar content.
    pub fn safe_area_left(mut self, inset: impl Into<Pixels>) -> Self {
        self.safe_area_left = inset.into();
        self
    }

    /// Set additional right safe-area offset for title bar content.
    pub fn safe_area_right(mut self, inset: impl Into<Pixels>) -> Self {
        self.safe_area_right = inset.into();
        self
    }
}

// The Windows control buttons have a fixed width of 35px.
//
// We don't need implementation the click event for the control buttons.
// If user clicked in the bounds, the window event will be triggered.
#[derive(IntoElement, Clone)]
enum ControlIcon {
    Minimize,
    Restore,
    Maximize,
    Close {
        on_close_window: Option<Rc<Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>>>,
    },
}

impl ControlIcon {
    fn minimize() -> Self {
        Self::Minimize
    }

    fn restore() -> Self {
        Self::Restore
    }

    fn maximize() -> Self {
        Self::Maximize
    }

    fn close(on_close_window: Option<Rc<Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>>>) -> Self {
        Self::Close { on_close_window }
    }

    fn id(&self) -> &'static str {
        match self {
            Self::Minimize => "minimize",
            Self::Restore => "restore",
            Self::Maximize => "maximize",
            Self::Close { .. } => "close",
        }
    }

    fn icon(&self) -> IconName {
        match self {
            Self::Minimize => IconName::WindowMinimize,
            Self::Restore => IconName::WindowRestore,
            Self::Maximize => IconName::WindowMaximize,
            Self::Close { .. } => IconName::WindowClose,
        }
    }

    fn window_control_area(&self) -> WindowControlArea {
        match self {
            Self::Minimize => WindowControlArea::Min,
            Self::Restore | Self::Maximize => WindowControlArea::Max,
            Self::Close { .. } => WindowControlArea::Close,
        }
    }

    fn is_close(&self) -> bool {
        matches!(self, Self::Close { .. })
    }

    #[inline]
    fn hover_fg(&self, cx: &App) -> Hsla {
        if self.is_close() {
            cx.theme().danger_foreground
        } else {
            cx.theme().secondary_foreground
        }
    }

    #[inline]
    fn hover_bg(&self, cx: &App) -> Hsla {
        if self.is_close() {
            cx.theme().danger
        } else {
            cx.theme().secondary_hover
        }
    }

    #[inline]
    fn active_bg(&self, cx: &mut App) -> Hsla {
        if self.is_close() {
            cx.theme().danger_active
        } else {
            cx.theme().secondary_active
        }
    }
}

fn start_windows_titlebar_drag(_window: &Window) {
    #[cfg(target_os = "windows")]
    {
        let raw_handle = match <Window as HasWindowHandle>::window_handle(_window) {
            Ok(handle) => handle,
            Err(error) => {
                tracing::warn!(?error, "window_handle unavailable");
                return;
            }
        };

        let RawWindowHandle::Win32(handle) = raw_handle.as_raw() else {
            return;
        };

        let hwnd = HWND(handle.hwnd.get() as _);
        unsafe {
            if let Err(error) = ReleaseCapture() {
                tracing::warn!(?error, "ReleaseCapture failed");
            }
            // Ask Windows to begin a standard non-client titlebar drag.
            // PostMessage avoids re-entering GPUI event dispatch while the current callback still holds App borrows.
            if let Err(error) = PostMessageW(
                Some(hwnd),
                WM_NCLBUTTONDOWN,
                WPARAM(HTCAPTION as _),
                LPARAM(0),
            ) {
                tracing::warn!(?error, "PostMessageW(WM_NCLBUTTONDOWN) failed");
            }
        }
    }
}

impl RenderOnce for ControlIcon {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let is_linux = cfg!(target_os = "linux");
        let is_windows = cfg!(target_os = "windows");
        let hover_fg = self.hover_fg(cx);
        let hover_bg = self.hover_bg(cx);
        let active_bg = self.active_bg(cx);
        let icon = self.clone();
        let on_close_window = match &self {
            ControlIcon::Close { on_close_window } => on_close_window.clone(),
            _ => None,
        };

        // Match Zed's WindowsCaptionButton structure exactly
        h_flex()
            .id(self.id())
            .justify_center()
            .content_center()
            .occlude()
            .w(px(36.)) // Match Zed's 36px width
            .h_full()
            .hover(|style| style.bg(hover_bg).text_color(hover_fg))
            .active(|style| style.bg(active_bg).text_color(hover_fg))
            .window_control_area(self.window_control_area())
            // Linux + Windows use explicit click handlers for reliability.
            // - Linux doesn't have native NC handling for these custom buttons.
            // - Windows should always work, even if WM_NCHITTEST mapping is flaky in some layouts.
            .when(is_linux || is_windows, |this| {
                this.on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    window.prevent_default();
                    cx.stop_propagation();
                })
                .on_click(move |_, window, cx| {
                    cx.stop_propagation();
                    match icon {
                        Self::Minimize => window.minimize_window(),
                        Self::Restore | Self::Maximize => window.zoom_window(),
                        Self::Close { .. } => {
                            if let Some(f) = on_close_window.clone() {
                                f(&ClickEvent::default(), window, cx);
                            } else {
                                window.remove_window();
                            }
                        }
                    }
                })
            })
            .child(Icon::new(self.icon()).small())
    }
}

#[derive(IntoElement)]
struct WindowControls {
    on_close_window: Option<Rc<Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>>>,
}

impl RenderOnce for WindowControls {
    fn render(self, window: &mut Window, _: &mut App) -> impl IntoElement {
        if cfg!(target_os = "macos") {
            return div().id("window-controls");
        }

        h_flex()
            .id("window-controls")
            .items_center()
            .flex_shrink_0()
            .h_full()
            .child(ControlIcon::minimize())
            .child(if window.is_maximized() {
                ControlIcon::restore()
            } else {
                ControlIcon::maximize()
            })
            .child(ControlIcon::close(self.on_close_window))
    }
}

impl Styled for TitleBar {
    fn style(&mut self) -> &mut gpui::StyleRefinement {
        &mut self.style
    }
}

impl ParentElement for TitleBar {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

struct TitleBarState {
    should_move: bool,
}

// TODO: Remove this when GPUI has released v0.2.3
impl Render for TitleBarState {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

impl RenderOnce for TitleBar {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let is_client_decorated = matches!(window.window_decorations(), Decorations::Client { .. });
        let is_linux = cfg!(target_os = "linux");
        let is_macos = cfg!(target_os = "macos");
        let is_windows = cfg!(target_os = "windows");

        let state = window.use_state(cx, |_, _| TitleBarState { should_move: false });

        let default_insets = if window.is_fullscreen() {
            Edges {
                left: px(8.0),
                right: DEFAULT_CONTENT_INSET_RIGHT,
                top: px(0.0),
                bottom: px(0.0),
            }
        } else {
            Edges {
                left: DEFAULT_CONTENT_INSET_LEFT,
                right: DEFAULT_CONTENT_INSET_RIGHT,
                top: px(0.0),
                bottom: px(0.0),
            }
        };
        let mut content_insets = self.content_insets.unwrap_or(default_insets);
        if let Some(left) = self.content_inset_left {
            content_insets.left = left;
        }
        if let Some(right) = self.content_inset_right {
            content_insets.right = right;
        }
        content_insets.left += self.safe_area_left;
        content_insets.right += self.safe_area_right;

        // Match Zed's title bar structure exactly:
        // h_flex() with window_control_area(Drag) applied first
        h_flex()
            .window_control_area(WindowControlArea::Drag)
            .w_full()
            .h(TITLE_BAR_HEIGHT)
            // Mouse handlers for window dragging (same as Zed)
            .map(|this| {
                // Windows: explicitly start a native titlebar drag via Win32.
                // This avoids reliance on WM_NCHITTEST + cached hit-testing state.
                this.when(is_windows, |this| {
                    this.on_mouse_down(MouseButton::Left, move |event, window, cx| {
                        // Let the double-click handler run instead of starting a drag.
                        if event.click_count >= 2 {
                            return;
                        }
                        window.prevent_default();
                        cx.stop_propagation();
                        start_windows_titlebar_drag(window);
                    })
                })
                // Non-Windows: use GPUI's platform implementation.
                .when(!is_windows, |this| {
                    this.on_mouse_down_out(window.listener_for(&state, |state, _, _, _| {
                        state.should_move = false;
                    }))
                    .on_mouse_up(
                        MouseButton::Left,
                        window.listener_for(&state, |state, _, _, _| {
                            state.should_move = false;
                        }),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        window.listener_for(&state, |state, _, _, _| {
                            state.should_move = true;
                        }),
                    )
                    .on_mouse_move(window.listener_for(
                        &state,
                        |state, _, window, _| {
                            if state.should_move {
                                state.should_move = false;
                                window.start_window_move();
                            }
                        },
                    ))
                })
            })
            // Platform-specific double-click behavior
            .map(|this| {
                this.id("title-bar")
                    .when(is_macos, |this| {
                        this.on_click(|event, window, _| {
                            if event.click_count() == 2 {
                                window.titlebar_double_click();
                            }
                        })
                    })
                    .when(is_windows, |this| {
                        this.on_click(|event, window, _| {
                            if event.click_count() == 2 {
                                window.zoom_window();
                            }
                        })
                    })
                    .when(is_linux, |this| {
                        this.on_click(|event, window, _| {
                            if event.click_count() == 2 {
                                window.zoom_window();
                            }
                        })
                    })
            })
            // Styling
            .border_b_1()
            .border_color(cx.theme().title_bar_border)
            .bg(cx.theme().title_bar)
            .refine_style(&self.style)
            .content_stretch()
            // Content area
            .child(
                div()
                    .id("title-bar-content")
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .overflow_x_hidden()
                    .w_full()
                    .pl(content_insets.left)
                    .pr(content_insets.right)
                    .when(is_linux && is_client_decorated, |this| {
                        this.on_mouse_down(MouseButton::Right, move |ev, window, _| {
                            window.show_window_menu(ev.position)
                        })
                    })
                    .children(self.children),
            )
            // Window controls (conditionally shown)
            .when(!window.is_fullscreen(), |this| {
                this.child(WindowControls {
                    on_close_window: self.on_close_window,
                })
            })
    }
}
