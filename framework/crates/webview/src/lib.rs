use std::{ops::Deref, rc::Rc};

use wry::{
    Rect,
    dpi::{self, LogicalSize},
};

use gpui::{
    App, Bounds, ContentMask, DismissEvent, Element, ElementId, Entity, EventEmitter, FocusHandle,
    Focusable, GlobalElementId, Hitbox, InteractiveElement, IntoElement, LayoutId, MouseDownEvent,
    ParentElement as _, Pixels, Render, SharedString, Size, Style, Styled as _, Window, canvas,
    div,
};

/// A webview based on wry WebView.
///
/// [experimental]
pub struct WebView {
    focus_handle: FocusHandle,
    webview: Rc<wry::WebView>,
    visible: bool,
    bounds: Bounds<Pixels>,
    last_error: Option<SharedString>,
}

impl Drop for WebView {
    fn drop(&mut self) {
        let _ = self.hide();
    }
}

impl WebView {
    /// Create a new WebView from a wry WebView.
    pub fn new(webview: wry::WebView, _: &mut Window, cx: &mut App) -> Self {
        let last_error = webview
            .set_bounds(Rect::default())
            .err()
            .map(|err| format!("Failed to initialize webview bounds: {err}").into());

        Self {
            focus_handle: cx.focus_handle(),
            visible: true,
            bounds: Bounds::default(),
            webview: Rc::new(webview),
            last_error,
        }
    }

    fn clear_error(&mut self) {
        self.last_error = None;
    }

    fn set_error(&mut self, message: impl Into<SharedString>) {
        self.last_error = Some(message.into());
    }

    /// Show the webview.
    pub fn show(&mut self) -> anyhow::Result<()> {
        if let Err(err) = self.webview.set_visible(true) {
            let message = format!("Failed to show webview: {err}");
            self.set_error(message.clone());
            return Err(anyhow::anyhow!(message));
        }
        self.visible = true;
        self.clear_error();
        Ok(())
    }

    /// Hide the webview.
    pub fn hide(&mut self) -> anyhow::Result<()> {
        if let Err(err) = self.webview.focus_parent() {
            let message = format!("Failed to return focus to the parent view: {err}");
            self.set_error(message.clone());
            return Err(anyhow::anyhow!(message));
        }
        if let Err(err) = self.webview.set_visible(false) {
            let message = format!("Failed to hide webview: {err}");
            self.set_error(message.clone());
            return Err(anyhow::anyhow!(message));
        }
        self.visible = false;
        self.clear_error();
        Ok(())
    }

    /// Get whether the webview is visible.
    pub fn visible(&self) -> bool {
        self.visible
    }

    /// Get the current bounds of the webview.
    pub fn bounds(&self) -> Bounds<Pixels> {
        self.bounds
    }

    /// Return the latest non-fatal webview error.
    pub fn last_error(&self) -> Option<&SharedString> {
        self.last_error.as_ref()
    }

    /// Go back in the webview history.
    pub fn back(&mut self) -> anyhow::Result<()> {
        if let Err(err) = self.webview.evaluate_script("history.back();") {
            let message = format!("Failed to navigate back: {err}");
            self.set_error(message.clone());
            return Err(anyhow::anyhow!(message));
        }
        self.clear_error();
        Ok(())
    }

    /// Load a URL in the webview.
    pub fn load_url(&mut self, url: &str) -> anyhow::Result<()> {
        if let Err(err) = self.webview.load_url(url) {
            let message = format!("Failed to load '{url}': {err}");
            self.set_error(message.clone());
            return Err(anyhow::anyhow!(message));
        }
        self.clear_error();
        Ok(())
    }

    /// Get the raw wry webview.
    pub fn raw(&self) -> &wry::WebView {
        &self.webview
    }
}

impl Deref for WebView {
    type Target = wry::WebView;

    fn deref(&self) -> &Self::Target {
        &self.webview
    }
}

impl Focusable for WebView {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<DismissEvent> for WebView {}

impl Render for WebView {
    fn render(
        &mut self,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl IntoElement {
        let view = cx.entity().clone();

        div()
            .track_focus(&self.focus_handle)
            .size_full()
            .child({
                let view = cx.entity().clone();
                canvas(
                    move |bounds, _, cx| view.update(cx, |r, _| r.bounds = bounds),
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full()
            })
            .child(WebViewElement::new(self.webview.clone(), view, window, cx))
    }
}

/// A webview element can display a wry webview.
pub struct WebViewElement {
    parent: Entity<WebView>,
    view: Rc<wry::WebView>,
}

impl WebViewElement {
    /// Create a new webview element from a wry WebView.
    pub fn new(
        view: Rc<wry::WebView>,
        parent: Entity<WebView>,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Self {
        Self { view, parent }
    }
}

impl IntoElement for WebViewElement {
    type Element = WebViewElement;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for WebViewElement {
    type RequestLayoutState = ();
    type PrepaintState = Option<Hitbox>;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let style = Style {
            size: Size::full(),
            flex_shrink: 1.,
            ..Default::default()
        };

        // If the parent view is no longer visible, we don't need to layout the webview
        let id = window.request_layout(style, [], cx);
        (id, ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        if !self.parent.read(cx).visible() {
            return None;
        }

        if let Err(err) = self.view.set_bounds(Rect {
            size: dpi::Size::Logical(LogicalSize {
                width: bounds.size.width.into(),
                height: bounds.size.height.into(),
            }),
            position: dpi::Position::Logical(dpi::LogicalPosition::new(
                bounds.origin.x.into(),
                bounds.origin.y.into(),
            )),
        }) {
            let message: SharedString = format!("Failed to resize webview: {err}").into();
            self.parent.update(cx, |parent, cx| {
                parent.set_error(message.clone());
                cx.notify();
            });
            return None;
        }

        self.parent.update(cx, |parent, cx| {
            if parent.last_error.is_some() {
                parent.clear_error();
                cx.notify();
            }
        });

        // Create a hitbox to handle mouse event
        Some(window.insert_hitbox(bounds, gpui::HitboxBehavior::Normal))
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        hitbox: &mut Self::PrepaintState,
        window: &mut Window,
        _: &mut App,
    ) {
        let bounds = hitbox.clone().map(|h| h.bounds).unwrap_or(bounds);
        let content_mask = ContentMask {
            bounds,
            ..Default::default()
        };
        window.with_content_mask(Some(content_mask), |window| {
            let webview = self.view.clone();
            window.on_mouse_event(move |event: &MouseDownEvent, _, _, _| {
                if !bounds.contains(&event.position) {
                    // Click white space to blur the input focus
                    let _ = webview.focus_parent();
                }
            });
        });
    }
}
