use std::rc::Rc;

use anyhow::Context as _;
use neutron_components_app::Shell as _;
use neutron_components_app::gpui::prelude::FluentBuilder as _;
use neutron_components_app::gpui::{
    App, AppContext as _, Context, Entity, FocusHandle, InteractiveElement as _, IntoElement,
    KeyDownEvent, ParentElement, Render, RendererInfo, Styled, Window, div,
};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use serde::Serialize;
use serde_json::json;

use crate::protocol::Protocol;
use crate::scenarios::ScenarioState;

/// A pointer-free native window-handle classification for conformance output.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum NativeWindowHandleKind {
    AppKit,
    Win32,
    Xlib,
    Xcb,
    Wayland,
}

/// A pointer-free native display-handle classification for conformance output.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum NativeDisplayHandleKind {
    AppKit,
    Windows,
    Xcb,
    Wayland,
}

struct NativeWindowEvidence {
    handle_kind: NativeWindowHandleKind,
    display_kind: NativeDisplayHandleKind,
    renderer_info: RendererInfo,
}

type KeyDownHandler = Rc<dyn Fn(&KeyDownEvent, &mut Window, &mut App)>;

/// A minimal rendered root used by native lifecycle scenarios.
pub(crate) struct ConformanceView {
    focus_handle: FocusHandle,
    on_key_down: Option<KeyDownHandler>,
}

impl Render for ConformanceView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("native-conformance-root")
            .track_focus(&self.focus_handle)
            .size_full()
            .child("Neutron Components native conformance")
            .when_some(self.on_key_down.clone(), |root, on_key_down| {
                root.on_key_down(move |event, window, cx| on_key_down(event, window, cx))
            })
    }
}

/// Build the plain conformance root with no key-down handler. Used as a
/// `Surface`/`SurfaceKey` build hook: non-capturing, so every scenario that
/// needs only the root's presence and focus target shares this one function.
///
/// Generic over the launch type `T` because each scenario now declares its
/// own `LaunchSpec<T>` (so its primary surface's arguments are `&T`, not
/// `&()`); the root itself never reads the launch value, so one function
/// still serves every scenario, monomorphized per call site.
pub(crate) fn build_conformance_view<T>(
    _args: &T,
    _window: &mut Window,
    cx: &mut App,
) -> Entity<ConformanceView> {
    cx.new(|cx| ConformanceView {
        focus_handle: cx.focus_handle(),
        on_key_down: None,
    })
}

/// Build the conformance root with a key-down handler installed and focused.
/// Used by the Wayland conformance path of the clipboard scenario, where a
/// synthetic key press must reach the window's content.
#[cfg(feature = "wayland-conformance")]
pub(crate) fn build_conformance_view_with_key_down(
    window: &mut Window,
    cx: &mut App,
    on_key_down: impl Fn(&KeyDownEvent, &mut Window, &mut App) + 'static,
) -> Entity<ConformanceView> {
    let on_key_down = Rc::new(on_key_down) as KeyDownHandler;
    let view = cx.new(move |cx| ConformanceView {
        focus_handle: cx.focus_handle(),
        on_key_down: Some(on_key_down),
    });
    let focus_handle = view.read(cx).focus_handle.clone();
    focus_handle.focus(window, cx);
    view
}

/// Capture native/renderer evidence for a just-opened surface window, emit it
/// (plus `window_opened`), and arrange for `after_first_presentation` to run
/// once the window's first renderer presentation evidence (not display
/// scanout) resolves.
///
/// Intended to be called from a `Surface::after_open` hook, once the
/// declaration's conformance global is in hand. Non-fatal: an evidence
/// failure is recorded on `state` and requests quit, matching every other
/// conformance failure path, rather than propagating an error the infallible
/// `after_open` hook signature cannot carry.
pub(crate) fn observe_native_window(
    window: &mut Window,
    cx: &mut App,
    protocol: &Protocol,
    state: &ScenarioState,
    key: &'static str,
    title: &'static str,
    after_first_presentation: fn(&mut Window, &mut App),
) {
    let evidence = match capture_evidence(window) {
        Ok(evidence) => evidence,
        Err(error) => {
            state.record_failure(format!("native window evidence capture failed: {error:#}"));
            cx.request_quit();
            return;
        }
    };

    state.emit(
        protocol,
        "native_window_handle",
        json!({"kind": evidence.handle_kind}),
    );
    state.emit(
        protocol,
        "native_display_handle",
        json!({"kind": evidence.display_kind}),
    );
    state.emit(
        protocol,
        "renderer_info",
        json!({"renderer_info": evidence.renderer_info}),
    );
    if state.failure().is_some() {
        cx.request_quit();
        return;
    }

    state.emit(
        protocol,
        "window_opened",
        json!({"key": key, "title": title}),
    );
    if state.failure().is_some() {
        cx.request_quit();
        return;
    }

    observe_first_presentation(
        window,
        cx,
        protocol.clone(),
        state.clone(),
        after_first_presentation,
    );
}

fn capture_evidence(window: &Window) -> anyhow::Result<NativeWindowEvidence> {
    let raw_handle = HasWindowHandle::window_handle(window)
        .map_err(|error| anyhow::anyhow!("obtain raw native window handle: {error:?}"))?
        .as_raw();
    let handle_kind = classify_raw_window_handle(raw_handle).map_err(anyhow::Error::msg)?;
    let raw_display = HasDisplayHandle::display_handle(window)
        .map_err(|error| anyhow::anyhow!("obtain raw native display handle: {error:?}"))?
        .as_raw();
    let display_kind = classify_raw_display_handle(raw_display).map_err(anyhow::Error::msg)?;

    Ok(NativeWindowEvidence {
        handle_kind,
        display_kind,
        renderer_info: window
            .renderer_info()
            .context("native window did not report renderer info")?,
    })
}

/// Validate a supported raw native handle without serializing its pointer/value.
fn classify_raw_window_handle(
    handle: RawWindowHandle,
) -> Result<NativeWindowHandleKind, &'static str> {
    match handle {
        // raw-window-handle encodes these four native values as NonNull/NonZero,
        // so matching the variant validates the non-null requirement without
        // exposing an address in the protocol.
        RawWindowHandle::AppKit(_) => Ok(NativeWindowHandleKind::AppKit),
        RawWindowHandle::Win32(_) => Ok(NativeWindowHandleKind::Win32),
        RawWindowHandle::Wayland(_) => Ok(NativeWindowHandleKind::Wayland),
        RawWindowHandle::Xcb(_) => Ok(NativeWindowHandleKind::Xcb),
        RawWindowHandle::Xlib(handle) if handle.window != 0 => Ok(NativeWindowHandleKind::Xlib),
        RawWindowHandle::Xlib(_) => Err("Xlib window handle was zero"),
        _ => Err("unsupported raw native window handle"),
    }
}

/// Validate a supported raw native display handle without serializing its pointer/value.
fn classify_raw_display_handle(
    handle: RawDisplayHandle,
) -> Result<NativeDisplayHandleKind, &'static str> {
    match handle {
        RawDisplayHandle::AppKit(_) => Ok(NativeDisplayHandleKind::AppKit),
        RawDisplayHandle::Windows(_) => Ok(NativeDisplayHandleKind::Windows),
        RawDisplayHandle::Xcb(handle) if handle.connection.is_some() => {
            Ok(NativeDisplayHandleKind::Xcb)
        }
        RawDisplayHandle::Xcb(_) => Err("Xcb display handle did not contain a connection"),
        RawDisplayHandle::Wayland(_) => Ok(NativeDisplayHandleKind::Wayland),
        _ => Err("unsupported raw native display handle"),
    }
}

fn observe_first_presentation(
    window: &Window,
    cx: &mut App,
    protocol: Protocol,
    state: ScenarioState,
    after_first_presentation: fn(&mut Window, &mut App),
) {
    // This receiver is subscribed before the root view is built and before the
    // window has an opportunity to draw. Stage 1 bounds the process externally;
    // this runner must not infer presentation from a timer.
    let first_presentation = window.observe_first_presentation();
    let proxy = cx.app_proxy();
    window
        .spawn(cx, async move |async_cx| match first_presentation.await {
            Ok(evidence) => {
                let protocol_for_update = protocol.clone();
                let state_for_update = state.clone();
                if let Err(error) = async_cx.update(move |window, cx| {
                    let presentation_count = window.first_presentation_count();
                    if presentation_count != 1 {
                        state_for_update.record_failure(format!(
                            "first-presentation observer resolved with count {presentation_count}"
                        ));
                        state_for_update.emit(
                            &protocol_for_update,
                            "presentation_count_invalid",
                            json!({"count": presentation_count}),
                        );
                        cx.request_quit();
                        return;
                    }
                    state_for_update.emit(
                        &protocol_for_update,
                        "frame_presented",
                        json!({
                            "presentation_evidence": evidence,
                            "count": presentation_count,
                        }),
                    );
                    after_first_presentation(window, cx);
                }) {
                    state.record_failure(format!(
                        "could not deliver first-presentation result to native window: {error:#}"
                    ));
                    state.emit(
                        &protocol,
                        "presentation_delivery_failed",
                        json!({"reason": "window_unavailable"}),
                    );
                    let _ = proxy.dispatch(|cx| cx.request_quit());
                }
            }
            Err(_) => {
                state.record_failure("first-presentation observer was cancelled".to_owned());
                let protocol_for_update = protocol.clone();
                let state_for_update = state.clone();
                if let Err(error) = async_cx.update(move |_, cx| {
                    state_for_update.emit(
                        &protocol_for_update,
                        "presentation_cancelled",
                        json!({"reason": "window_closed_before_presentation"}),
                    );
                    cx.request_quit();
                }) {
                    state.record_failure(format!(
                        "could not deliver presentation cancellation to native window: {error:#}"
                    ));
                    state.emit(
                        &protocol,
                        "presentation_delivery_failed",
                        json!({"reason": "window_unavailable"}),
                    );
                    let _ = proxy.dispatch(|cx| cx.request_quit());
                }
            }
        })
        .detach();
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroIsize;
    use std::ptr::NonNull;

    use raw_window_handle::{
        AppKitDisplayHandle, AppKitWindowHandle, WaylandDisplayHandle, WaylandWindowHandle,
        Win32WindowHandle, WindowsDisplayHandle, XcbDisplayHandle, XcbWindowHandle,
        XlibWindowHandle,
    };

    use super::*;

    #[test]
    fn classifies_supported_raw_native_handles_without_addresses() {
        let pointer = NonNull::<u8>::dangling().cast();

        assert_eq!(
            classify_raw_window_handle(RawWindowHandle::AppKit(AppKitWindowHandle::new(pointer))),
            Ok(NativeWindowHandleKind::AppKit)
        );
        assert_eq!(
            classify_raw_window_handle(RawWindowHandle::Win32(Win32WindowHandle::new(
                NonZeroIsize::new(1).expect("one is non-zero"),
            ))),
            Ok(NativeWindowHandleKind::Win32)
        );
        assert_eq!(
            classify_raw_window_handle(RawWindowHandle::Xlib(XlibWindowHandle::new(1))),
            Ok(NativeWindowHandleKind::Xlib)
        );
        assert_eq!(
            classify_raw_window_handle(RawWindowHandle::Xcb(XcbWindowHandle::new(
                std::num::NonZeroU32::new(1).expect("one is non-zero"),
            ))),
            Ok(NativeWindowHandleKind::Xcb)
        );
        assert_eq!(
            classify_raw_window_handle(RawWindowHandle::Wayland(WaylandWindowHandle::new(pointer))),
            Ok(NativeWindowHandleKind::Wayland)
        );
    }

    #[test]
    fn classifies_supported_raw_native_display_handles_without_addresses() {
        let pointer = NonNull::<u8>::dangling().cast();

        assert_eq!(
            classify_raw_display_handle(RawDisplayHandle::AppKit(AppKitDisplayHandle::new())),
            Ok(NativeDisplayHandleKind::AppKit)
        );
        assert_eq!(
            classify_raw_display_handle(RawDisplayHandle::Windows(WindowsDisplayHandle::new())),
            Ok(NativeDisplayHandleKind::Windows)
        );
        assert_eq!(
            classify_raw_display_handle(RawDisplayHandle::Xcb(XcbDisplayHandle::new(
                Some(pointer),
                0,
            ))),
            Ok(NativeDisplayHandleKind::Xcb)
        );
        assert_eq!(
            classify_raw_display_handle(RawDisplayHandle::Wayland(WaylandDisplayHandle::new(
                pointer,
            ))),
            Ok(NativeDisplayHandleKind::Wayland)
        );
    }

    #[test]
    fn rejects_xcb_display_handle_without_connection() {
        assert_eq!(
            classify_raw_display_handle(RawDisplayHandle::Xcb(XcbDisplayHandle::new(None, 0))),
            Err("Xcb display handle did not contain a connection")
        );
    }

    #[test]
    fn rejects_zero_xlib_window_handle() {
        assert_eq!(
            classify_raw_window_handle(RawWindowHandle::Xlib(XlibWindowHandle::new(0))),
            Err("Xlib window handle was zero")
        );
    }
}
