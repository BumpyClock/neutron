use std::{
    cell::{Cell, Ref, RefCell, RefMut},
    ffi::c_void,
    ptr::NonNull,
    rc::Rc,
    sync::Arc,
};

use collections::{FxHashMap, HashMap};
use futures::channel::oneshot::Receiver;

use raw_window_handle as rwh;
use wayland_backend::client::ObjectId;
use wayland_client::WEnum;
use wayland_client::{
    Proxy,
    protocol::{wl_output, wl_seat, wl_surface},
};
use wayland_protocols::wp::viewporter::client::wp_viewport;
use wayland_protocols::xdg::decoration::zv1::client::zxdg_toplevel_decoration_v1;
use wayland_protocols::xdg::shell::client::xdg_surface;
use wayland_protocols::xdg::shell::client::xdg_toplevel::{self};
use wayland_protocols::xdg::shell::client::{xdg_popup, xdg_positioner};
use wayland_protocols::{
    wp::fractional_scale::v1::client::wp_fractional_scale_v1,
    xdg::dialog::v1::client::xdg_dialog_v1::XdgDialogV1,
};
use wayland_protocols_plasma::blur::client::org_kde_kwin_blur;
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1;

use crate::linux::accesskit_shims::{
    TrivialActionHandler, TrivialActivationHandler, TrivialDeactivationHandler,
};
use crate::linux::wayland::{display::WaylandDisplay, serial::SerialKind};
use crate::linux::{Globals, Output, WaylandClientStatePtr, get_window};
use gpui::{
    AnyWindowHandle, Bounds, Capslock, Decorations, DevicePixels, FirstPresentationObserver,
    GpuSpecs, Modifiers, OverlayInputMode, Pixels, PlatformAtlas, PlatformDisplay, PlatformInput,
    PlatformInputHandler, PlatformWindow, Point, PromptButton, PromptLevel, RendererInfo,
    RequestFrameOptions, ResizeEdge, Scene, Size, Tiling, WindowAppearance,
    WindowBackgroundAppearance, WindowBounds, WindowControlArea, WindowControls, WindowDecorations,
    WindowKind, WindowParams, layer_shell::LayerShellNotSupportedError, popup::PopupOptions, px,
    size,
};
use gpui_wgpu::{CompositorGpuHint, GpuContext, WgpuRenderer, WgpuSurfaceConfig};

#[derive(Default)]
pub(crate) struct Callbacks {
    request_frame: Option<Box<dyn FnMut(RequestFrameOptions)>>,
    input: Option<Box<dyn FnMut(gpui::PlatformInput) -> gpui::DispatchEventResult>>,
    active_status_change: Option<Box<dyn FnMut(bool)>>,
    hover_status_change: Option<Box<dyn FnMut(bool)>>,
    resize: Option<Box<dyn FnMut(Size<Pixels>, f32)>>,
    moved: Option<Box<dyn FnMut()>>,
    should_close: Option<Box<dyn FnMut() -> bool>>,
    close: Option<Box<dyn FnOnce()>>,
    appearance_changed: Option<Box<dyn FnMut()>>,
}

struct RawWindow {
    window: *mut c_void,
    display: *mut c_void,
}

// Safety: The raw pointers in RawWindow point to Wayland surface/display
// which are valid for the window's lifetime. These are used only for
// passing to wgpu which needs Send+Sync for surface creation.
unsafe impl Send for RawWindow {}
unsafe impl Sync for RawWindow {}

impl rwh::HasWindowHandle for RawWindow {
    fn window_handle(&self) -> Result<rwh::WindowHandle<'_>, rwh::HandleError> {
        let window = NonNull::new(self.window).unwrap();
        let handle = rwh::WaylandWindowHandle::new(window);
        Ok(unsafe { rwh::WindowHandle::borrow_raw(handle.into()) })
    }
}
impl rwh::HasDisplayHandle for RawWindow {
    fn display_handle(&self) -> Result<rwh::DisplayHandle<'_>, rwh::HandleError> {
        let display = NonNull::new(self.display).unwrap();
        let handle = rwh::WaylandDisplayHandle::new(display);
        Ok(unsafe { rwh::DisplayHandle::borrow_raw(handle.into()) })
    }
}

#[derive(Debug)]
struct InProgressConfigure {
    size: Option<Size<Pixels>>,
    fullscreen: bool,
    maximized: bool,
    resizing: bool,
    tiling: Tiling,
}

#[derive(Clone, Debug)]
struct InputRegionState {
    overlay_mode: OverlayInputMode,
    // Outer `None` means overlay mode was the last setter. `Some(None)` means the generic
    // input-region API explicitly restored the protocol default (the whole surface).
    explicit: Option<Option<Vec<Bounds<Pixels>>>>,
}

#[derive(Debug, PartialEq)]
enum ResolvedInputRegion<'a> {
    Default,
    Rects(&'a [Bounds<Pixels>]),
    OverlayInteractive(Bounds<Pixels>),
}

impl Default for InputRegionState {
    fn default() -> Self {
        Self {
            overlay_mode: OverlayInputMode::Interactive,
            explicit: None,
        }
    }
}

impl InputRegionState {
    fn set_overlay_mode(&mut self, mode: OverlayInputMode) {
        self.overlay_mode = mode;
        self.explicit = None;
    }

    fn set_explicit(&mut self, region: Option<&[Bounds<Pixels>]>) {
        self.explicit = Some(region.map(<[_]>::to_vec));
    }

    fn resolve(&self, interactive_bounds: Bounds<Pixels>) -> ResolvedInputRegion<'_> {
        match &self.explicit {
            Some(None) => ResolvedInputRegion::Default,
            Some(Some(rects)) => ResolvedInputRegion::Rects(rects),
            None if self.overlay_mode == OverlayInputMode::ClickThrough => {
                ResolvedInputRegion::Rects(&[])
            }
            None => ResolvedInputRegion::OverlayInteractive(interactive_bounds),
        }
    }
}

pub struct WaylandWindowState {
    surface_state: WaylandSurfaceState,
    acknowledged_first_configure: bool,
    parent: Option<WaylandWindowStatePtr>,
    children: FxHashMap<ObjectId, bool>,
    pub surface: wl_surface::WlSurface,
    app_id: Option<String>,
    appearance: WindowAppearance,
    blur: Option<org_kde_kwin_blur::OrgKdeKwinBlur>,
    viewport: Option<wp_viewport::WpViewport>,
    outputs: HashMap<ObjectId, Output>,
    display: Option<(ObjectId, Output)>,
    globals: Globals,
    renderer: WgpuRenderer,
    bounds: Bounds<Pixels>,
    scale: f32,
    input_handler: Option<PlatformInputHandler>,
    decorations: WindowDecorations,
    background_appearance: WindowBackgroundAppearance,
    input_region: InputRegionState,
    fullscreen: bool,
    maximized: bool,
    tiling: Tiling,
    window_bounds: Bounds<Pixels>,
    client: WaylandClientStatePtr,
    handle: AnyWindowHandle,
    active: bool,
    hovered: bool,
    pub(crate) force_render_after_recovery: bool,
    renderer_presented: bool,
    in_progress_configure: Option<InProgressConfigure>,
    resize_throttle: bool,
    in_progress_window_controls: Option<WindowControls>,
    window_controls: WindowControls,
    client_inset: Option<Pixels>,
    accesskit_adapter: Option<accesskit_unix::Adapter>,
}

pub enum WaylandSurfaceState {
    Xdg(WaylandXdgSurfaceState),
    LayerShell(WaylandLayerSurfaceState),
    Popup(WaylandPopupSurfaceState),
}

impl WaylandSurfaceState {
    fn new(
        surface: &wl_surface::WlSurface,
        globals: &Globals,
        params: &WindowParams,
        parent: Option<WaylandWindowStatePtr>,
        popup_grab: Option<(u32, wl_seat::WlSeat)>,
        target_output: Option<wl_output::WlOutput>,
    ) -> anyhow::Result<Self> {
        // For layer_shell windows, create a layer surface instead of an xdg surface
        if let WindowKind::LayerShell(options) = &params.kind {
            let Some(layer_shell) = globals.layer_shell.as_ref() else {
                return Err(LayerShellNotSupportedError.into());
            };

            let layer_surface = layer_shell.get_layer_surface(
                &surface,
                target_output.as_ref(),
                super::layer_shell::wayland_layer(options.layer),
                options.namespace.clone(),
                &globals.qh,
                surface.id(),
            );

            let width = f32::from(params.bounds.size.width);
            let height = f32::from(params.bounds.size.height);
            layer_surface.set_size(width as u32, height as u32);

            layer_surface.set_anchor(super::layer_shell::wayland_anchor(options.anchor));
            layer_surface.set_keyboard_interactivity(
                super::layer_shell::wayland_keyboard_interactivity(options.keyboard_interactivity),
            );

            if let Some(margin) = options.margin {
                layer_surface.set_margin(
                    f32::from(margin.0) as i32,
                    f32::from(margin.1) as i32,
                    f32::from(margin.2) as i32,
                    f32::from(margin.3) as i32,
                )
            }

            if let Some(exclusive_zone) = options.exclusive_zone {
                layer_surface.set_exclusive_zone(f32::from(exclusive_zone) as i32);
            }

            if let Some(exclusive_edge) = options.exclusive_edge {
                layer_surface
                    .set_exclusive_edge(super::layer_shell::wayland_anchor(exclusive_edge));
            }

            return Ok(WaylandSurfaceState::LayerShell(WaylandLayerSurfaceState {
                layer_surface,
            }));
        }

        if let WindowKind::AnchoredPopup(options) = &params.kind {
            let Some(parent) = parent.as_ref() else {
                return Err(anyhow::anyhow!("popup parent window not found"));
            };
            let positioner = build_popup_positioner(
                globals,
                options,
                params.bounds.size,
                parent.window_geometry(),
            );
            let xdg_surface = globals
                .wm_base
                .get_xdg_surface(surface, &globals.qh, surface.id());
            let xdg_popup = if let Some(parent_layer_surface) = parent.layer_surface() {
                let popup = xdg_surface.get_popup(None, &positioner, &globals.qh, surface.id());
                parent_layer_surface.get_popup(&popup);
                popup
            } else {
                xdg_surface.get_popup(
                    parent.xdg_surface().as_ref(),
                    &positioner,
                    &globals.qh,
                    surface.id(),
                )
            };
            positioner.destroy();

            if let Some((serial, seat)) = popup_grab {
                xdg_popup.grab(&seat, serial);
            }
            parent.add_child(surface.id(), false);

            return Ok(WaylandSurfaceState::Popup(WaylandPopupSurfaceState {
                xdg_surface,
                xdg_popup,
                options: options.clone(),
                next_reposition_token: Cell::new(0),
            }));
        }

        // All other WindowKinds result in a regular xdg surface
        let xdg_surface = globals
            .wm_base
            .get_xdg_surface(&surface, &globals.qh, surface.id());

        let toplevel = xdg_surface.get_toplevel(&globals.qh, surface.id());
        let xdg_parent = parent.as_ref().and_then(|w| w.toplevel());

        if params.kind == WindowKind::Floating || params.kind == WindowKind::Dialog {
            toplevel.set_parent(xdg_parent.as_ref());
        }

        let dialog = if params.kind == WindowKind::Dialog {
            let dialog = globals.dialog.as_ref().map(|dialog| {
                let xdg_dialog = dialog.get_xdg_dialog(&toplevel, &globals.qh, ());
                xdg_dialog.set_modal();
                xdg_dialog
            });

            if let Some(parent) = parent.as_ref() {
                parent.add_child(surface.id(), true);
            }

            dialog
        } else {
            None
        };

        if let Some(size) = params.window_min_size {
            toplevel.set_min_size(f32::from(size.width) as i32, f32::from(size.height) as i32);
        }

        // Attempt to set up window decorations based on the requested configuration
        let decoration = globals
            .decoration_manager
            .as_ref()
            .map(|decoration_manager| {
                decoration_manager.get_toplevel_decoration(&toplevel, &globals.qh, surface.id())
            });

        Ok(WaylandSurfaceState::Xdg(WaylandXdgSurfaceState {
            xdg_surface,
            toplevel,
            decoration,
            dialog,
        }))
    }
}

pub struct WaylandXdgSurfaceState {
    xdg_surface: xdg_surface::XdgSurface,
    toplevel: xdg_toplevel::XdgToplevel,
    decoration: Option<zxdg_toplevel_decoration_v1::ZxdgToplevelDecorationV1>,
    dialog: Option<XdgDialogV1>,
}

pub struct WaylandLayerSurfaceState {
    layer_surface: zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
}

pub struct WaylandPopupSurfaceState {
    xdg_surface: xdg_surface::XdgSurface,
    xdg_popup: xdg_popup::XdgPopup,
    options: PopupOptions,
    next_reposition_token: Cell<u32>,
}

fn build_popup_positioner(
    globals: &Globals,
    options: &PopupOptions,
    size: Size<Pixels>,
    parent_geometry: Bounds<Pixels>,
) -> xdg_positioner::XdgPositioner {
    let positioner = globals.wm_base.create_positioner(&globals.qh, ());
    positioner.set_size(
        f32::from(size.width).max(1.0) as i32,
        f32::from(size.height).max(1.0) as i32,
    );

    let anchor_rect = Bounds {
        origin: options.anchor_rect.origin - parent_geometry.origin,
        size: options.anchor_rect.size,
    };
    let one = Point::new(px(1.0), px(1.0));
    let geometry_bottom_right = Point::new(parent_geometry.size.width, parent_geometry.size.height);
    let top_left = anchor_rect
        .origin
        .min(&(geometry_bottom_right - one))
        .max(&Point::default());
    let bottom_right = anchor_rect
        .bottom_right()
        .min(&geometry_bottom_right)
        .max(&(top_left + one));
    let anchor_rect = Bounds::from_corners(top_left, bottom_right);
    positioner.set_anchor_rect(
        f32::from(anchor_rect.origin.x) as i32,
        f32::from(anchor_rect.origin.y) as i32,
        f32::from(anchor_rect.size.width) as i32,
        f32::from(anchor_rect.size.height) as i32,
    );
    positioner.set_anchor(super::popup::wayland_anchor(options.anchor));
    positioner.set_gravity(super::popup::wayland_gravity(options.gravity));
    positioner.set_constraint_adjustment(super::popup::wayland_constraint_adjustment(
        options.constraint_adjustment,
    ));
    positioner.set_offset(
        f32::from(options.offset.x) as i32,
        f32::from(options.offset.y) as i32,
    );
    positioner
}

impl WaylandSurfaceState {
    fn ack_configure(&self, serial: u32) {
        match self {
            WaylandSurfaceState::Xdg(WaylandXdgSurfaceState { xdg_surface, .. }) => {
                xdg_surface.ack_configure(serial);
            }
            WaylandSurfaceState::LayerShell(WaylandLayerSurfaceState { layer_surface, .. }) => {
                layer_surface.ack_configure(serial);
            }
            WaylandSurfaceState::Popup(WaylandPopupSurfaceState { xdg_surface, .. }) => {
                xdg_surface.ack_configure(serial);
            }
        }
    }

    fn decoration(&self) -> Option<&zxdg_toplevel_decoration_v1::ZxdgToplevelDecorationV1> {
        if let WaylandSurfaceState::Xdg(WaylandXdgSurfaceState { decoration, .. }) = self {
            decoration.as_ref()
        } else {
            None
        }
    }

    fn toplevel(&self) -> Option<&xdg_toplevel::XdgToplevel> {
        if let WaylandSurfaceState::Xdg(WaylandXdgSurfaceState { toplevel, .. }) = self {
            Some(toplevel)
        } else {
            None
        }
    }

    fn xdg_surface(&self) -> Option<&xdg_surface::XdgSurface> {
        match self {
            WaylandSurfaceState::Xdg(WaylandXdgSurfaceState { xdg_surface, .. })
            | WaylandSurfaceState::Popup(WaylandPopupSurfaceState { xdg_surface, .. }) => {
                Some(xdg_surface)
            }
            WaylandSurfaceState::LayerShell(_) => None,
        }
    }

    fn layer_surface(&self) -> Option<&zwlr_layer_surface_v1::ZwlrLayerSurfaceV1> {
        if let WaylandSurfaceState::LayerShell(WaylandLayerSurfaceState { layer_surface }) = self {
            Some(layer_surface)
        } else {
            None
        }
    }

    fn set_geometry(&self, x: i32, y: i32, width: i32, height: i32) {
        match self {
            WaylandSurfaceState::Xdg(WaylandXdgSurfaceState { xdg_surface, .. }) => {
                xdg_surface.set_window_geometry(x, y, width, height);
            }
            WaylandSurfaceState::LayerShell(WaylandLayerSurfaceState { layer_surface, .. }) => {
                // cannot set window position of a layer surface
                layer_surface.set_size(width as u32, height as u32);
            }
            WaylandSurfaceState::Popup(WaylandPopupSurfaceState { xdg_surface, .. }) => {
                xdg_surface.set_window_geometry(x, y, width, height);
            }
        }
    }

    fn reposition_popup(
        &self,
        globals: &Globals,
        size: Size<Pixels>,
        parent_geometry: Bounds<Pixels>,
    ) {
        if let WaylandSurfaceState::Popup(WaylandPopupSurfaceState {
            xdg_popup,
            options,
            next_reposition_token,
            ..
        }) = self
            && xdg_popup.version() >= xdg_popup::REQ_REPOSITION_SINCE
        {
            let token = next_reposition_token.get();
            next_reposition_token.set(token.wrapping_add(1));
            let positioner = build_popup_positioner(globals, options, size, parent_geometry);
            xdg_popup.reposition(&positioner, token);
            positioner.destroy();
        }
    }

    fn destroy(&mut self) {
        match self {
            WaylandSurfaceState::Xdg(WaylandXdgSurfaceState {
                xdg_surface,
                toplevel,
                decoration: _decoration,
                dialog,
            }) => {
                // drop the dialog before toplevel so compositor can explicitly unapply it's effects
                if let Some(dialog) = dialog {
                    dialog.destroy();
                }

                // The role object (toplevel) must always be destroyed before the xdg_surface.
                // See https://wayland.app/protocols/xdg-shell#xdg_surface:request:destroy
                toplevel.destroy();
                xdg_surface.destroy();
            }
            WaylandSurfaceState::LayerShell(WaylandLayerSurfaceState { layer_surface }) => {
                layer_surface.destroy();
            }
            WaylandSurfaceState::Popup(WaylandPopupSurfaceState {
                xdg_surface,
                xdg_popup,
                ..
            }) => {
                xdg_popup.destroy();
                xdg_surface.destroy();
            }
        }
    }
}

#[derive(Clone)]
pub struct WaylandWindowStatePtr {
    state: Rc<RefCell<WaylandWindowState>>,
    callbacks: Rc<RefCell<Callbacks>>,
}

impl WaylandWindowState {
    pub(crate) fn new(
        handle: AnyWindowHandle,
        surface: wl_surface::WlSurface,
        surface_state: WaylandSurfaceState,
        appearance: WindowAppearance,
        viewport: Option<wp_viewport::WpViewport>,
        client: WaylandClientStatePtr,
        globals: Globals,
        gpu_context: GpuContext,
        compositor_gpu: Option<CompositorGpuHint>,
        options: WindowParams,
        parent: Option<WaylandWindowStatePtr>,
    ) -> anyhow::Result<Self> {
        let renderer = {
            let raw_window = RawWindow {
                window: surface.id().as_ptr().cast::<c_void>(),
                display: surface
                    .backend()
                    .upgrade()
                    .unwrap()
                    .display_ptr()
                    .cast::<c_void>(),
            };
            let config = WgpuSurfaceConfig {
                size: Size {
                    width: DevicePixels(f32::from(options.bounds.size.width) as i32),
                    height: DevicePixels(f32::from(options.bounds.size.height) as i32),
                },
                transparent: true,
                preferred_present_mode: Some(wgpu::PresentMode::Mailbox),
            };
            WgpuRenderer::new(gpu_context, &raw_window, config, compositor_gpu)?
        };

        if let WaylandSurfaceState::Xdg(ref xdg_state) = surface_state {
            if let Some(title) = options.titlebar.and_then(|titlebar| titlebar.title) {
                xdg_state.toplevel.set_title(title.to_string());
            }
            if let Some(app_id) = options.app_id.as_ref() {
                xdg_state.toplevel.set_app_id(app_id.clone());
            }
            // Set max window size based on the GPU's maximum texture dimension.
            // This prevents the window from being resized larger than what the GPU can render.
            let max_texture_size = renderer.max_texture_size() as i32;
            xdg_state
                .toplevel
                .set_max_size(max_texture_size, max_texture_size);
        }

        Ok(Self {
            surface_state,
            acknowledged_first_configure: false,
            parent,
            children: FxHashMap::default(),
            surface,
            app_id: options.app_id,
            blur: None,
            viewport,
            globals,
            outputs: HashMap::default(),
            display: None,
            renderer,
            bounds: options.bounds,
            scale: 1.0,
            input_handler: None,
            decorations: WindowDecorations::Client,
            background_appearance: WindowBackgroundAppearance::Opaque,
            input_region: InputRegionState::default(),
            fullscreen: false,
            maximized: false,
            tiling: Tiling::default(),
            window_bounds: options.bounds,
            in_progress_configure: None,
            resize_throttle: false,
            client,
            appearance,
            handle,
            active: false,
            hovered: false,
            force_render_after_recovery: false,
            renderer_presented: false,
            in_progress_window_controls: None,
            window_controls: WindowControls::default(),
            client_inset: None,
            accesskit_adapter: None,
        })
    }

    pub fn is_transparent(&self) -> bool {
        self.decorations == WindowDecorations::Client
            || self.background_appearance != WindowBackgroundAppearance::Opaque
    }

    fn update_subpixel_layout(&mut self) {
        use wayland_client::protocol::wl_output::Subpixel;
        let is_bgr = self
            .display
            .as_ref()
            .and_then(|(_, output)| output.subpixel)
            .is_some_and(|subpixel| subpixel == Subpixel::HorizontalBgr);
        self.renderer.set_subpixel_layout(is_bgr);
    }

    pub fn primary_output_scale(&mut self) -> i32 {
        let mut scale = 1;
        let mut current_output = self.display.take();
        for (id, output) in self.outputs.iter() {
            if let Some((_, output_data)) = &current_output {
                if output.scale > output_data.scale {
                    current_output = Some((id.clone(), output.clone()));
                }
            } else {
                current_output = Some((id.clone(), output.clone()));
            }
            scale = scale.max(output.scale);
        }
        self.display = current_output;
        self.update_subpixel_layout();
        scale
    }

    pub fn inset(&self) -> Pixels {
        match self.decorations {
            WindowDecorations::Server => px(0.0),
            WindowDecorations::Client => self.client_inset.unwrap_or(px(0.0)),
        }
    }

    fn update_accesskit_window_bounds(&mut self) {
        let scale = self.scale;
        let bounds = self.bounds.map_origin(|_| px(0.0));
        let inner_bounds = bounds.inset(self.inset());

        let outer = accesskit::Rect {
            x0: f64::from(f32::from(bounds.origin.x) * scale),
            y0: f64::from(f32::from(bounds.origin.y) * scale),
            x1: f64::from(f32::from(bounds.origin.x + bounds.size.width) * scale),
            y1: f64::from(f32::from(bounds.origin.y + bounds.size.height) * scale),
        };

        let inner = accesskit::Rect {
            x0: f64::from(f32::from(inner_bounds.origin.x) * scale),
            y0: f64::from(f32::from(inner_bounds.origin.y) * scale),
            x1: f64::from(f32::from(inner_bounds.origin.x + inner_bounds.size.width) * scale),
            y1: f64::from(f32::from(inner_bounds.origin.y + inner_bounds.size.height) * scale),
        };

        if let Some(adapter) = self.accesskit_adapter.as_mut() {
            adapter.set_root_window_bounds(outer, inner);
        }
    }
}

pub(crate) struct WaylandWindow(pub WaylandWindowStatePtr);
pub enum ImeInput {
    InsertText(String),
    SetMarkedText(String),
    UnmarkText,
    DeleteText,
}

impl Drop for WaylandWindow {
    fn drop(&mut self) {
        let mut state = self.0.state.borrow_mut();
        let surface_id = state.surface.id();
        if let Some(parent) = state.parent.as_ref() {
            parent.state.borrow_mut().children.remove(&surface_id);
        }

        let client = state.client.clone();

        state.renderer.destroy();

        // Destroy blur first, this has no dependencies.
        if let Some(blur) = &state.blur {
            blur.release();
        }

        // Decorations must be destroyed before the xdg state.
        // See https://wayland.app/protocols/xdg-decoration-unstable-v1#zxdg_toplevel_decoration_v1
        if let Some(decoration) = &state.surface_state.decoration() {
            decoration.destroy();
        }

        // Surface state might contain xdg_toplevel/xdg_surface which can be destroyed now that
        // decorations are gone. layer_surface has no dependencies.
        state.surface_state.destroy();

        // Viewport must be destroyed before the wl_surface.
        // See https://wayland.app/protocols/viewporter#wp_viewport
        if let Some(viewport) = &state.viewport {
            viewport.destroy();
        }

        // The wl_surface itself should always be destroyed last.
        state.surface.destroy();

        let state_ptr = self.0.clone();
        state
            .globals
            .executor
            .spawn(async move {
                state_ptr.close();
                client.drop_window(&surface_id)
            })
            .detach();
        drop(state);
    }
}

impl WaylandWindow {
    fn borrow(&self) -> Ref<'_, WaylandWindowState> {
        self.0.state.borrow()
    }

    fn borrow_mut(&self) -> RefMut<'_, WaylandWindowState> {
        self.0.state.borrow_mut()
    }

    pub fn new(
        handle: AnyWindowHandle,
        globals: Globals,
        gpu_context: GpuContext,
        compositor_gpu: Option<CompositorGpuHint>,
        client: WaylandClientStatePtr,
        params: WindowParams,
        appearance: WindowAppearance,
        parent: Option<WaylandWindowStatePtr>,
        popup_grab: Option<(u32, wl_seat::WlSeat)>,
        target_output: Option<wl_output::WlOutput>,
    ) -> anyhow::Result<(Self, ObjectId)> {
        let surface = globals.compositor.create_surface(&globals.qh, ());
        let surface_state = WaylandSurfaceState::new(
            &surface,
            &globals,
            &params,
            parent.clone(),
            popup_grab,
            target_output,
        )?;

        if let Some(fractional_scale_manager) = globals.fractional_scale_manager.as_ref() {
            fractional_scale_manager.get_fractional_scale(&surface, &globals.qh, surface.id());
        }

        let viewport = globals
            .viewporter
            .as_ref()
            .map(|viewporter| viewporter.get_viewport(&surface, &globals.qh, ()));

        let this = Self(WaylandWindowStatePtr {
            state: Rc::new(RefCell::new(WaylandWindowState::new(
                handle,
                surface.clone(),
                surface_state,
                appearance,
                viewport,
                client,
                globals,
                gpu_context,
                compositor_gpu,
                params,
                parent,
            )?)),
            callbacks: Rc::new(RefCell::new(Callbacks::default())),
        });

        // Kick things off
        surface.commit();

        Ok((this, surface.id()))
    }
}

impl WaylandWindowStatePtr {
    pub fn handle(&self) -> AnyWindowHandle {
        self.state.borrow().handle
    }

    pub fn surface(&self) -> wl_surface::WlSurface {
        self.state.borrow().surface.clone()
    }

    pub fn toplevel(&self) -> Option<xdg_toplevel::XdgToplevel> {
        self.state.borrow().surface_state.toplevel().cloned()
    }

    pub fn xdg_surface(&self) -> Option<xdg_surface::XdgSurface> {
        self.state.borrow().surface_state.xdg_surface().cloned()
    }

    pub fn layer_surface(&self) -> Option<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1> {
        self.state.borrow().surface_state.layer_surface().cloned()
    }

    pub fn window_geometry(&self) -> Bounds<Pixels> {
        let state = self.state.borrow();
        inset_by_tiling(
            state.bounds.map_origin(|_| px(0.0)),
            state.inset(),
            state.tiling,
        )
    }

    pub fn ptr_eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.state, &other.state)
    }

    pub fn add_child(&self, child: ObjectId, blocking: bool) {
        let mut state = self.state.borrow_mut();
        state.children.insert(child, blocking);
    }

    pub fn is_blocked(&self) -> bool {
        let state = self.state.borrow();
        state.children.values().any(|&blocking| blocking)
    }

    pub fn frame(&self) {
        let mut state = self.state.borrow_mut();
        state.surface.frame(&state.globals.qh, state.surface.id());
        state.resize_throttle = false;
        let force_render = state.force_render_after_recovery;
        state.force_render_after_recovery = false;
        drop(state);

        let mut cb = self.callbacks.borrow_mut();
        if let Some(fun) = cb.request_frame.as_mut() {
            fun(RequestFrameOptions {
                force_render,
                ..Default::default()
            });
            drop(cb);
            self.update_ime_enabled();
        }
    }

    fn update_ime_enabled(&self) {
        let mut state = self.state.borrow_mut();
        if !state.active {
            return;
        }
        let client = state.client.clone();
        let ime_enabled = state
            .input_handler
            .as_mut()
            .map(|input_handler| input_handler.query_accepts_text_input())
            .unwrap_or(true);
        drop(state);
        if Some(ime_enabled) == client.ime_enabled() {
            return;
        }
        if ime_enabled {
            client.enable_ime();
        } else {
            client.disable_ime();
        }
    }

    pub fn handle_xdg_surface_event(&self, event: xdg_surface::Event) {
        if let xdg_surface::Event::Configure { serial } = event {
            {
                let mut state = self.state.borrow_mut();
                if let Some(window_controls) = state.in_progress_window_controls.take() {
                    state.window_controls = window_controls;

                    drop(state);
                    let mut callbacks = self.callbacks.borrow_mut();
                    if let Some(appearance_changed) = callbacks.appearance_changed.as_mut() {
                        appearance_changed();
                    }
                }
            }
            {
                let mut state = self.state.borrow_mut();

                if let Some(mut configure) = state.in_progress_configure.take() {
                    let got_unmaximized = state.maximized && !configure.maximized;
                    state.fullscreen = configure.fullscreen;
                    state.maximized = configure.maximized;
                    state.tiling = configure.tiling;
                    // Limit interactive resizes to once per vblank
                    if configure.resizing && state.resize_throttle {
                        state.surface_state.ack_configure(serial);
                        return;
                    } else if configure.resizing {
                        state.resize_throttle = true;
                    }
                    if !configure.fullscreen && !configure.maximized {
                        configure.size = if got_unmaximized {
                            Some(state.window_bounds.size)
                        } else {
                            compute_outer_size(state.inset(), configure.size, state.tiling)
                        };
                        if let Some(size) = configure.size {
                            state.window_bounds = Bounds {
                                origin: Point::default(),
                                size,
                            };
                        }
                    }
                    drop(state);
                    if let Some(size) = configure.size {
                        self.resize(size);
                    }
                }
            }
            let mut state = self.state.borrow_mut();
            state.surface_state.ack_configure(serial);

            let window_geometry = inset_by_tiling(
                state.bounds.map_origin(|_| px(0.0)),
                state.inset(),
                state.tiling,
            )
            .map(|v| f32::from(v) as i32)
            .map_size(|v| if v <= 0 { 1 } else { v });

            state.surface_state.set_geometry(
                window_geometry.origin.x,
                window_geometry.origin.y,
                window_geometry.size.width,
                window_geometry.size.height,
            );

            let request_frame_callback = !state.acknowledged_first_configure;
            if request_frame_callback {
                state.acknowledged_first_configure = true;
                drop(state);
                self.frame();
            }
        }
    }

    pub fn handle_toplevel_decoration_event(&self, event: zxdg_toplevel_decoration_v1::Event) {
        if let zxdg_toplevel_decoration_v1::Event::Configure { mode } = event {
            match mode {
                WEnum::Value(zxdg_toplevel_decoration_v1::Mode::ServerSide) => {
                    self.state.borrow_mut().decorations = WindowDecorations::Server;
                    if let Some(appearance_changed) =
                        self.callbacks.borrow_mut().appearance_changed.as_mut()
                    {
                        appearance_changed();
                    }
                }
                WEnum::Value(zxdg_toplevel_decoration_v1::Mode::ClientSide) => {
                    self.state.borrow_mut().decorations = WindowDecorations::Client;
                    // Update background to be transparent
                    if let Some(appearance_changed) =
                        self.callbacks.borrow_mut().appearance_changed.as_mut()
                    {
                        appearance_changed();
                    }
                }
                WEnum::Value(_) => {
                    log::warn!("Unknown decoration mode");
                }
                WEnum::Unknown(v) => {
                    log::warn!("Unknown decoration mode: {}", v);
                }
            }
        }
    }

    pub fn handle_fractional_scale_event(&self, event: wp_fractional_scale_v1::Event) {
        if let wp_fractional_scale_v1::Event::PreferredScale { scale } = event {
            self.rescale(scale as f32 / 120.0);
        }
    }

    pub fn handle_toplevel_event(&self, event: xdg_toplevel::Event) -> bool {
        match event {
            xdg_toplevel::Event::Configure {
                width,
                height,
                states,
            } => {
                let size = if width == 0 || height == 0 {
                    None
                } else {
                    Some(size(px(width as f32), px(height as f32)))
                };

                let states = extract_states::<xdg_toplevel::State>(&states);

                let mut tiling = Tiling::default();
                let mut fullscreen = false;
                let mut maximized = false;
                let mut resizing = false;

                for state in states {
                    match state {
                        xdg_toplevel::State::Maximized => {
                            maximized = true;
                        }
                        xdg_toplevel::State::Fullscreen => {
                            fullscreen = true;
                        }
                        xdg_toplevel::State::Resizing => resizing = true,
                        xdg_toplevel::State::TiledTop => {
                            tiling.top = true;
                        }
                        xdg_toplevel::State::TiledLeft => {
                            tiling.left = true;
                        }
                        xdg_toplevel::State::TiledRight => {
                            tiling.right = true;
                        }
                        xdg_toplevel::State::TiledBottom => {
                            tiling.bottom = true;
                        }
                        _ => {
                            // noop
                        }
                    }
                }

                if fullscreen || maximized {
                    tiling = Tiling::tiled();
                }

                let mut state = self.state.borrow_mut();
                state.in_progress_configure = Some(InProgressConfigure {
                    size,
                    fullscreen,
                    maximized,
                    resizing,
                    tiling,
                });

                false
            }
            xdg_toplevel::Event::Close => {
                let mut cb = self.callbacks.borrow_mut();
                if let Some(mut should_close) = cb.should_close.take() {
                    let result = (should_close)();
                    cb.should_close = Some(should_close);
                    if result {
                        drop(cb);
                        self.close();
                    }
                    result
                } else {
                    true
                }
            }
            xdg_toplevel::Event::WmCapabilities { capabilities } => {
                let mut window_controls = WindowControls::default();

                let states = extract_states::<xdg_toplevel::WmCapabilities>(&capabilities);

                for state in states {
                    match state {
                        xdg_toplevel::WmCapabilities::Maximize => {
                            window_controls.maximize = true;
                        }
                        xdg_toplevel::WmCapabilities::Minimize => {
                            window_controls.minimize = true;
                        }
                        xdg_toplevel::WmCapabilities::Fullscreen => {
                            window_controls.fullscreen = true;
                        }
                        xdg_toplevel::WmCapabilities::WindowMenu => {
                            window_controls.window_menu = true;
                        }
                        _ => {}
                    }
                }

                let mut state = self.state.borrow_mut();
                state.in_progress_window_controls = Some(window_controls);
                false
            }
            _ => false,
        }
    }

    pub fn handle_layersurface_event(&self, event: zwlr_layer_surface_v1::Event) -> bool {
        match event {
            zwlr_layer_surface_v1::Event::Configure {
                width,
                height,
                serial,
            } => {
                let size = if width == 0 || height == 0 {
                    None
                } else {
                    Some(size(px(width as f32), px(height as f32)))
                };

                let mut state = self.state.borrow_mut();
                state.in_progress_configure = Some(InProgressConfigure {
                    size,
                    fullscreen: false,
                    maximized: false,
                    resizing: false,
                    tiling: Tiling::default(),
                });
                drop(state);

                // just do the same thing we'd do as an xdg_surface
                self.handle_xdg_surface_event(xdg_surface::Event::Configure { serial });

                false
            }
            zwlr_layer_surface_v1::Event::Closed => {
                // unlike xdg, we don't have a choice here: the surface is closing.
                true
            }
            _ => false,
        }
    }

    pub fn handle_popup_event(&self, event: xdg_popup::Event) -> bool {
        match event {
            xdg_popup::Event::Configure { width, height, .. } => {
                let size = if width <= 0 || height <= 0 {
                    None
                } else {
                    Some(size(px(width as f32), px(height as f32)))
                };
                self.state.borrow_mut().in_progress_configure = Some(InProgressConfigure {
                    size,
                    fullscreen: false,
                    maximized: false,
                    resizing: false,
                    tiling: Tiling::default(),
                });
                false
            }
            xdg_popup::Event::PopupDone => true,
            xdg_popup::Event::Repositioned { .. } => false,
            _ => false,
        }
    }

    #[allow(clippy::mutable_key_type)]
    pub fn handle_surface_event(
        &self,
        event: wl_surface::Event,
        outputs: HashMap<ObjectId, Output>,
    ) {
        let mut state = self.state.borrow_mut();

        match event {
            wl_surface::Event::Enter { output } => {
                let id = output.id();

                let Some(output) = outputs.get(&id) else {
                    return;
                };

                state.outputs.insert(id, output.clone());

                let scale = state.primary_output_scale();

                // We use `PreferredBufferScale` instead to set the scale if it's available
                if state.surface.version() < wl_surface::EVT_PREFERRED_BUFFER_SCALE_SINCE {
                    state.surface.set_buffer_scale(scale);
                    drop(state);
                    self.rescale(scale as f32);
                }
            }
            wl_surface::Event::Leave { output } => {
                state.outputs.remove(&output.id());

                let scale = state.primary_output_scale();

                // We use `PreferredBufferScale` instead to set the scale if it's available
                if state.surface.version() < wl_surface::EVT_PREFERRED_BUFFER_SCALE_SINCE {
                    state.surface.set_buffer_scale(scale);
                    drop(state);
                    self.rescale(scale as f32);
                }
            }
            wl_surface::Event::PreferredBufferScale { factor } => {
                // We use `WpFractionalScale` instead to set the scale if it's available
                if state.globals.fractional_scale_manager.is_none() {
                    state.surface.set_buffer_scale(factor);
                    drop(state);
                    self.rescale(factor as f32);
                }
            }
            _ => {}
        }
    }

    pub fn handle_ime(&self, ime: ImeInput) {
        if self.is_blocked() {
            return;
        }
        let mut state = self.state.borrow_mut();
        if let Some(mut input_handler) = state.input_handler.take() {
            drop(state);
            match ime {
                ImeInput::InsertText(text) => {
                    input_handler.replace_text_in_range(None, &text);
                }
                ImeInput::SetMarkedText(text) => {
                    input_handler.replace_and_mark_text_in_range(None, &text, None);
                }
                ImeInput::UnmarkText => {
                    input_handler.unmark_text();
                }
                ImeInput::DeleteText => {
                    if let Some(marked) = input_handler.marked_text_range() {
                        input_handler.replace_text_in_range(Some(marked), "");
                    }
                }
            }
            self.state.borrow_mut().input_handler = Some(input_handler);
        }
    }

    pub fn get_ime_area(&self) -> Option<Bounds<Pixels>> {
        let mut state = self.state.borrow_mut();
        let mut bounds: Option<Bounds<Pixels>> = None;
        if let Some(mut input_handler) = state.input_handler.take() {
            drop(state);
            bounds = input_handler.ime_candidate_bounds();
            self.state.borrow_mut().input_handler = Some(input_handler);
        }
        bounds
    }

    pub fn set_size_and_scale(&self, size: Option<Size<Pixels>>, scale: Option<f32>) {
        let (size, scale, needs_blur_update) = {
            let mut state = self.state.borrow_mut();
            if size.is_none_or(|size| size == state.bounds.size)
                && scale.is_none_or(|scale| scale == state.scale)
            {
                return;
            }
            if let Some(size) = size {
                state.bounds.size = size;
            }
            if let Some(scale) = scale {
                state.scale = scale;
            }
            state.update_accesskit_window_bounds();
            let device_bounds = state.bounds.to_device_pixels(state.scale);
            state.renderer.update_drawable_size(device_bounds.size);
            // A blurred region depends on the window size, so it must be
            // rebuilt on resize. Capture the flag before dropping the borrow.
            let needs_blur_update = matches!(
                state.background_appearance,
                WindowBackgroundAppearance::Blurred { .. }
            );
            (state.bounds.size, state.scale, needs_blur_update)
        };

        if let Some(ref mut fun) = self.callbacks.borrow_mut().resize {
            fun(size, scale);
        }

        {
            let state = self.state.borrow();
            if let Some(viewport) = &state.viewport {
                viewport
                    .set_destination(f32::from(size.width) as i32, f32::from(size.height) as i32);
            }
        }

        if needs_blur_update {
            update_window(self.state.borrow_mut());
        }
    }

    pub fn resize(&self, size: Size<Pixels>) {
        self.set_size_and_scale(Some(size), None);
    }

    pub fn rescale(&self, scale: f32) {
        self.set_size_and_scale(None, Some(scale));
    }

    pub fn close(&self) {
        let state = self.state.borrow();
        let client = state.client.get_client();
        let children = state.children.keys().cloned().collect::<Vec<_>>();
        drop(state);

        for child in children {
            let mut client_state = client.borrow_mut();
            let window = get_window(&mut client_state, &child);
            drop(client_state);

            if let Some(child) = window {
                child.close();
            }
        }
        let mut callbacks = self.callbacks.borrow_mut();
        if let Some(fun) = callbacks.close.take() {
            fun()
        }
    }

    pub fn handle_input(&self, input: PlatformInput) -> bool {
        if self.is_blocked() {
            return false;
        }
        let mut callback_dispatched = false;
        if let Some(ref mut fun) = self.callbacks.borrow_mut().input {
            callback_dispatched = true;
            if !fun(input.clone()).propagate {
                return true;
            }
        }
        if let PlatformInput::KeyDown(event) = input
            && event.keystroke.modifiers.is_subset_of(&Modifiers::shift())
            && let Some(key_char) = &event.keystroke.key_char
        {
            let mut state = self.state.borrow_mut();
            if let Some(mut input_handler) = state.input_handler.take() {
                drop(state);
                input_handler.replace_text_in_range(None, key_char);
                self.state.borrow_mut().input_handler = Some(input_handler);
            }
        }
        callback_dispatched
    }

    pub fn set_focused(&self, focus: bool) {
        self.state.borrow_mut().active = focus;
        if let Some(ref mut fun) = self.callbacks.borrow_mut().active_status_change {
            fun(focus);
        }
        if let Some(adapter) = self.state.borrow_mut().accesskit_adapter.as_mut() {
            adapter.update_window_focus_state(focus);
        }
    }

    pub fn set_hovered(&self, focus: bool) {
        if let Some(ref mut fun) = self.callbacks.borrow_mut().hover_status_change {
            fun(focus);
        }
    }

    pub fn set_appearance(&mut self, appearance: WindowAppearance) {
        self.state.borrow_mut().appearance = appearance;

        let mut callbacks = self.callbacks.borrow_mut();
        if let Some(ref mut fun) = callbacks.appearance_changed {
            (fun)()
        }
    }

    pub fn primary_output_scale(&self) -> i32 {
        self.state.borrow_mut().primary_output_scale()
    }
}

fn extract_states<'a, S: TryFrom<u32> + 'a>(states: &'a [u8]) -> impl Iterator<Item = S> + 'a
where
    <S as TryFrom<u32>>::Error: 'a,
{
    states
        .chunks_exact(4)
        .flat_map(TryInto::<[u8; 4]>::try_into)
        .map(u32::from_ne_bytes)
        .flat_map(S::try_from)
}

impl rwh::HasWindowHandle for WaylandWindow {
    fn window_handle(&self) -> Result<rwh::WindowHandle<'_>, rwh::HandleError> {
        let surface = self.0.surface().id().as_ptr() as *mut libc::c_void;
        let c_ptr = NonNull::new(surface).ok_or(rwh::HandleError::Unavailable)?;
        let handle = rwh::WaylandWindowHandle::new(c_ptr);
        let raw_handle = rwh::RawWindowHandle::Wayland(handle);
        Ok(unsafe { rwh::WindowHandle::borrow_raw(raw_handle) })
    }
}

impl rwh::HasDisplayHandle for WaylandWindow {
    fn display_handle(&self) -> Result<rwh::DisplayHandle<'_>, rwh::HandleError> {
        let display = self
            .0
            .surface()
            .backend()
            .upgrade()
            .ok_or(rwh::HandleError::Unavailable)?
            .display_ptr() as *mut libc::c_void;

        let c_ptr = NonNull::new(display).ok_or(rwh::HandleError::Unavailable)?;
        let handle = rwh::WaylandDisplayHandle::new(c_ptr);
        let raw_handle = rwh::RawDisplayHandle::Wayland(handle);
        Ok(unsafe { rwh::DisplayHandle::borrow_raw(raw_handle) })
    }
}

impl PlatformWindow for WaylandWindow {
    fn bounds(&self) -> Bounds<Pixels> {
        self.borrow().bounds
    }

    fn is_maximized(&self) -> bool {
        self.borrow().maximized
    }

    fn window_bounds(&self) -> WindowBounds {
        let state = self.borrow();
        if state.fullscreen {
            WindowBounds::Fullscreen(state.window_bounds)
        } else if state.maximized {
            WindowBounds::Maximized(state.window_bounds)
        } else {
            drop(state);
            WindowBounds::Windowed(self.bounds())
        }
    }

    fn inner_window_bounds(&self) -> WindowBounds {
        let state = self.borrow();
        if state.fullscreen {
            WindowBounds::Fullscreen(state.window_bounds)
        } else if state.maximized {
            WindowBounds::Maximized(state.window_bounds)
        } else {
            let inset = state.inset();
            drop(state);
            WindowBounds::Windowed(self.bounds().inset(inset))
        }
    }

    fn content_size(&self) -> Size<Pixels> {
        self.borrow().bounds.size
    }

    fn resize(&mut self, size: Size<Pixels>) {
        let state = self.borrow();
        let state_ptr = self.0.clone();

        if matches!(state.surface_state, WaylandSurfaceState::Popup(_)) {
            if state.acknowledged_first_configure {
                let parent_geometry = state
                    .parent
                    .as_ref()
                    .map(WaylandWindowStatePtr::window_geometry)
                    .unwrap_or_default();
                state
                    .surface_state
                    .reposition_popup(&state.globals, size, parent_geometry);
            }
            return;
        }

        // Keep window geometry consistent with configure handling. On Wayland, window geometry is
        // surface-local: resizing should not attempt to translate the window; the compositor
        // controls placement. We also account for client-side decoration insets and tiling.
        let window_geometry = inset_by_tiling(
            Bounds {
                origin: Point::default(),
                size,
            },
            state.inset(),
            state.tiling,
        )
        .map(|v| f32::from(v) as i32)
        .map_size(|v| if v <= 0 { 1 } else { v });

        state.surface_state.set_geometry(
            window_geometry.origin.x,
            window_geometry.origin.y,
            window_geometry.size.width,
            window_geometry.size.height,
        );

        state
            .globals
            .executor
            .spawn(async move { state_ptr.resize(size) })
            .detach();
    }

    fn set_position(&self, _origin: Point<Pixels>) {
        log::debug!("Wayland compositor controls window positioning");
    }

    fn set_visible(&self, visible: bool) {
        let state = self.borrow();
        if visible {
            state.surface.commit();
        } else {
            state.surface.attach(None, 0, 0);
            state.surface.commit();
        }
    }

    fn scale_factor(&self) -> f32 {
        self.borrow().scale
    }

    fn appearance(&self) -> WindowAppearance {
        self.borrow().appearance
    }

    fn display(&self) -> Option<Rc<dyn PlatformDisplay>> {
        let state = self.borrow();
        state.display.as_ref().map(|(id, display)| {
            Rc::new(WaylandDisplay {
                id: id.clone(),
                name: display.name.clone(),
                bounds: display.bounds.to_pixels(state.scale),
            }) as Rc<dyn PlatformDisplay>
        })
    }

    fn mouse_position(&self) -> Point<Pixels> {
        self.borrow()
            .client
            .get_client()
            .borrow()
            .mouse_location
            .unwrap_or_default()
    }

    fn modifiers(&self) -> Modifiers {
        self.borrow().client.get_client().borrow().modifiers
    }

    fn capslock(&self) -> Capslock {
        self.borrow().client.get_client().borrow().capslock
    }

    fn set_input_handler(&mut self, input_handler: PlatformInputHandler) {
        self.borrow_mut().input_handler = Some(input_handler);
    }

    fn take_input_handler(&mut self) -> Option<PlatformInputHandler> {
        self.borrow_mut().input_handler.take()
    }

    fn prompt(
        &self,
        _level: PromptLevel,
        _msg: &str,
        _detail: Option<&str>,
        _answers: &[PromptButton],
    ) -> Option<Receiver<usize>> {
        None
    }

    fn activate(&self) {
        // Try to request an activation token. Even though the activation is likely going to be rejected,
        // KWin and Mutter can use the app_id to visually indicate we're requesting attention.
        let state = self.borrow();
        if let (Some(activation), Some(app_id)) = (&state.globals.activation, state.app_id.clone())
        {
            state.client.set_pending_activation(state.surface.id());
            let token = activation.get_activation_token(&state.globals.qh, ());
            // The serial isn't exactly important here, since the activation is probably going to be rejected anyway.
            let serial = state.client.get_serial(SerialKind::MousePress);
            token.set_app_id(app_id);
            token.set_serial(serial, &state.globals.seat);
            token.set_surface(&state.surface);
            token.commit();
        }
    }

    fn is_active(&self) -> bool {
        self.borrow().active
    }

    fn is_hovered(&self) -> bool {
        self.borrow().hovered
    }

    fn set_title(&mut self, title: &str) {
        if let Some(toplevel) = self.borrow().surface_state.toplevel() {
            toplevel.set_title(title.to_string());
        }
    }

    fn set_app_id(&mut self, app_id: &str) {
        let mut state = self.borrow_mut();
        if let Some(toplevel) = state.surface_state.toplevel() {
            toplevel.set_app_id(app_id.to_owned());
        }
        state.app_id = Some(app_id.to_owned());
    }

    fn set_background_appearance(&self, background_appearance: WindowBackgroundAppearance) {
        let mut state = self.borrow_mut();
        state.background_appearance = background_appearance;
        update_window(state);
    }

    fn set_overlay_input_mode(&self, input_mode: OverlayInputMode) {
        let mut state = self.borrow_mut();
        state.input_region.set_overlay_mode(input_mode);
        update_window(state);
    }

    fn set_input_region(&self, region: Option<&[Bounds<Pixels>]>) {
        let mut state = self.borrow_mut();
        state.input_region.set_explicit(region);
        update_window(state);
        self.borrow().surface.commit();
    }

    fn background_appearance(&self) -> WindowBackgroundAppearance {
        self.borrow().background_appearance
    }

    fn is_subpixel_rendering_supported(&self) -> bool {
        self.borrow().renderer.supports_dual_source_blending()
    }

    fn minimize(&self) {
        if let Some(toplevel) = self.borrow().surface_state.toplevel() {
            toplevel.set_minimized();
        }
    }

    fn zoom(&self) {
        let state = self.borrow();
        if let Some(toplevel) = state.surface_state.toplevel() {
            if !state.maximized {
                toplevel.set_maximized();
            } else {
                toplevel.unset_maximized();
            }
        }
    }

    fn toggle_fullscreen(&self) {
        let state = self.borrow();
        if let Some(toplevel) = state.surface_state.toplevel() {
            if !state.fullscreen {
                toplevel.set_fullscreen(None);
            } else {
                toplevel.unset_fullscreen();
            }
        }
    }

    fn is_fullscreen(&self) -> bool {
        self.borrow().fullscreen
    }

    fn on_request_frame(&self, callback: Box<dyn FnMut(RequestFrameOptions)>) {
        self.0.callbacks.borrow_mut().request_frame = Some(callback);
    }

    fn on_input(&self, callback: Box<dyn FnMut(PlatformInput) -> gpui::DispatchEventResult>) {
        self.0.callbacks.borrow_mut().input = Some(callback);
    }

    fn on_active_status_change(&self, callback: Box<dyn FnMut(bool)>) {
        self.0.callbacks.borrow_mut().active_status_change = Some(callback);
    }

    fn on_hover_status_change(&self, callback: Box<dyn FnMut(bool)>) {
        self.0.callbacks.borrow_mut().hover_status_change = Some(callback);
    }

    fn on_resize(&self, callback: Box<dyn FnMut(Size<Pixels>, f32)>) {
        self.0.callbacks.borrow_mut().resize = Some(callback);
    }

    fn on_moved(&self, callback: Box<dyn FnMut()>) {
        self.0.callbacks.borrow_mut().moved = Some(callback);
    }

    fn on_should_close(&self, callback: Box<dyn FnMut() -> bool>) {
        self.0.callbacks.borrow_mut().should_close = Some(callback);
    }

    fn on_close(&self, callback: Box<dyn FnOnce()>) {
        self.0.callbacks.borrow_mut().close = Some(callback);
    }

    fn on_hit_test_window_control(&self, _callback: Box<dyn FnMut() -> Option<WindowControlArea>>) {
    }

    fn on_appearance_changed(&self, callback: Box<dyn FnMut()>) {
        self.0.callbacks.borrow_mut().appearance_changed = Some(callback);
    }

    fn draw(&self, scene: &Scene) {
        let mut state = self.borrow_mut();
        if state.renderer.device_lost() {
            if let Err(error) = state.renderer.recover() {
                log::warn!("failed to recover lost wgpu device; will retry: {error:#}");
            }
            let _ = state.renderer.needs_redraw();
            state.force_render_after_recovery = true;
            return;
        }
        state.renderer_presented = state.renderer.draw(scene);
        if state.renderer.needs_redraw() {
            state.force_render_after_recovery = true;
        }
    }

    fn set_first_presentation_observer(&self, observer: FirstPresentationObserver) {
        self.borrow_mut()
            .renderer
            .set_first_presentation_observer(observer);
    }

    #[cfg(feature = "wayland-conformance")]
    fn request_wayland_conformance_key_press(&self) -> Receiver<anyhow::Result<()>> {
        let state = self.borrow();
        state
            .client
            .request_wayland_conformance_key_press(&state.surface)
    }

    fn completed_frame(&self) {
        let mut state = self.borrow_mut();
        if !state.renderer_presented {
            state.surface.commit();
        }
        state.renderer_presented = false;
    }

    fn sprite_atlas(&self) -> Arc<dyn PlatformAtlas> {
        let state = self.borrow();
        state.renderer.sprite_atlas().clone()
    }

    fn show_window_menu(&self, position: Point<Pixels>) {
        let state = self.borrow();
        let serial = state.client.get_serial(SerialKind::MousePress);
        if let Some(toplevel) = state.surface_state.toplevel() {
            toplevel.show_window_menu(
                &state.globals.seat,
                serial,
                f32::from(position.x) as i32,
                f32::from(position.y) as i32,
            );
        }
    }

    fn start_window_move(&self) {
        let state = self.borrow();
        let serial = state.client.get_serial(SerialKind::MousePress);
        if let Some(toplevel) = state.surface_state.toplevel() {
            toplevel._move(&state.globals.seat, serial);
        }
    }

    fn start_window_resize(&self, edge: gpui::ResizeEdge) {
        let state = self.borrow();
        if let Some(toplevel) = state.surface_state.toplevel() {
            toplevel.resize(
                &state.globals.seat,
                state.client.get_serial(SerialKind::MousePress),
                edge.to_xdg(),
            )
        }
    }

    fn window_decorations(&self) -> Decorations {
        let state = self.borrow();
        match state.decorations {
            WindowDecorations::Server => Decorations::Server,
            WindowDecorations::Client => Decorations::Client {
                tiling: state.tiling,
            },
        }
    }

    fn request_decorations(&self, decorations: WindowDecorations) {
        let mut state = self.borrow_mut();
        match state.surface_state.decoration().as_ref() {
            Some(decoration) => {
                decoration.set_mode(decorations.to_xdg());
                state.decorations = decorations;
                update_window(state);
            }
            None => {
                if matches!(decorations, WindowDecorations::Server) {
                    log::info!(
                        "Server-side decorations requested, but the Wayland server does not support them. Falling back to client-side decorations."
                    );
                }
                state.decorations = WindowDecorations::Client;
                update_window(state);
            }
        }
    }

    fn window_controls(&self) -> WindowControls {
        self.borrow().window_controls
    }

    fn set_client_inset(&self, inset: Pixels) {
        let mut state = self.borrow_mut();
        if Some(inset) != state.client_inset {
            state.client_inset = Some(inset);
            update_window(state);
        }
    }

    fn update_ime_position(&self, bounds: Bounds<Pixels>) {
        let state = self.borrow();
        state.client.update_ime_position(bounds);
    }

    fn gpu_specs(&self) -> Option<GpuSpecs> {
        self.borrow().renderer.gpu_specs().into()
    }

    fn renderer_info(&self) -> Option<RendererInfo> {
        Some(self.borrow().renderer.renderer_info())
    }

    fn play_system_bell(&self) {
        let state = self.borrow();
        let surface = if state.surface_state.toplevel().is_some() {
            Some(&state.surface)
        } else {
            None
        };
        if let Some(bell) = state.globals.system_bell.as_ref() {
            bell.ring(surface);
        }
    }

    fn a11y_init(&self, callbacks: gpui::A11yCallbacks) {
        let activation_handler = TrivialActivationHandler {
            callback: callbacks.activation,
        };
        let action_handler = TrivialActionHandler {
            callback: callbacks.action,
        };
        let deactivation_handler = TrivialDeactivationHandler {
            callback: callbacks.deactivation,
        };

        let mut adapter =
            accesskit_unix::Adapter::new(activation_handler, action_handler, deactivation_handler);
        adapter.update_window_focus_state(self.borrow().active);

        self.borrow_mut().accesskit_adapter = Some(adapter);
    }

    fn a11y_tree_update(&self, tree_update: accesskit::TreeUpdate) {
        let mut state = self.borrow_mut();
        if let Some(adapter) = state.accesskit_adapter.as_mut() {
            adapter.update_if_active(|| tree_update);
        }
    }

    fn a11y_update_window_bounds(&self) {
        let mut state = self.borrow_mut();
        state.update_accesskit_window_bounds();
    }
}

/// Approximates a rounded rectangle as horizontal bands for building a
/// `wl_region`. Returns `(x, y, width, height)` rects in surface-local
/// logical coordinates. `radius` is clamped to `min(width, height) / 2`;
/// radius 0 yields a single full rect.
pub(crate) fn rounded_rect_region_bands(
    width: i32,
    height: i32,
    radius: i32,
) -> Vec<(i32, i32, i32, i32)> {
    if width <= 0 || height <= 0 {
        return Vec::new();
    }
    let r = radius.clamp(0, width.min(height) / 2);
    if r == 0 {
        return vec![(0, 0, width, height)];
    }

    let rf = r as f64;
    let mut bands = Vec::with_capacity((2 * r + 1) as usize);
    for y in 0..r {
        // Sample the band's vertical center to pick a representative inset.
        let dy = rf - y as f64 - 0.5;
        let inset = (rf - (rf * rf - dy * dy).sqrt()).round() as i32;
        let band_width = width - 2 * inset;
        // Top band, then its mirror at the bottom.
        bands.push((inset, y, band_width, 1));
        bands.push((inset, height - 1 - y, band_width, 1));
    }
    // Middle slab between the two rounded ends.
    bands.push((0, r, width, height - 2 * r));
    bands
}

fn update_window(mut state: RefMut<WaylandWindowState>) {
    let opaque = !state.is_transparent();

    state.renderer.update_transparency(!opaque);
    let opaque_area = state.window_bounds.map(|v| f32::from(v) as i32);
    opaque_area.inset(f32::from(state.inset()) as i32);

    let opaque_region = state
        .globals
        .compositor
        .create_region(&state.globals.qh, ());
    opaque_region.add(
        opaque_area.origin.x,
        opaque_area.origin.y,
        opaque_area.size.width,
        opaque_area.size.height,
    );

    // Note that rounded corners make this rectangle API hard to work with.
    // As this is common when using CSD, let's just disable this API.
    if state.background_appearance == WindowBackgroundAppearance::Opaque
        && state.decorations == WindowDecorations::Server
    {
        // Promise the compositor that this region of the window surface
        // contains no transparent pixels. This allows the compositor to skip
        // updating whatever is behind the surface for better performance.
        state.surface.set_opaque_region(Some(&opaque_region));
    } else {
        state.surface.set_opaque_region(None);
    }

    let input_region = match state
        .input_region
        .resolve(opaque_area.map(|value| px(value as f32)))
    {
        ResolvedInputRegion::Default => {
            state.surface.set_input_region(None);
            None
        }
        ResolvedInputRegion::Rects(rects) => {
            let input_region = state
                .globals
                .compositor
                .create_region(&state.globals.qh, ());
            for rect in rects {
                let rect = rect.map(|pixels| f32::from(pixels) as i32);
                input_region.add(
                    rect.origin.x,
                    rect.origin.y,
                    rect.size.width,
                    rect.size.height,
                );
            }
            state.surface.set_input_region(Some(&input_region));
            Some(input_region)
        }
        ResolvedInputRegion::OverlayInteractive(bounds) => {
            let input_region = state
                .globals
                .compositor
                .create_region(&state.globals.qh, ());
            let bounds = bounds.map(|pixels| f32::from(pixels) as i32);
            input_region.add(
                bounds.origin.x,
                bounds.origin.y,
                bounds.size.width,
                bounds.size.height,
            );
            state.surface.set_input_region(Some(&input_region));
            Some(input_region)
        }
    };

    if let Some(ref blur_manager) = state.globals.blur_manager {
        match state.background_appearance {
            WindowBackgroundAppearance::Blurred { corner_radius } => {
                if state.blur.is_none() {
                    let blur = blur_manager.create(&state.surface, &state.globals.qh, ());
                    state.blur = Some(blur);
                }
                // Clip the blur to a rounded rect approximated by horizontal
                // bands, using the same logical window size as the opaque region.
                let bounds = state.window_bounds.map(|v| f32::from(v) as i32);
                let radius = f32::from(corner_radius) as i32;
                let region = state
                    .globals
                    .compositor
                    .create_region(&state.globals.qh, ());
                for (x, y, w, h) in
                    rounded_rect_region_bands(bounds.size.width, bounds.size.height, radius)
                {
                    region.add(x, y, w, h);
                }
                let blur = state.blur.as_ref().unwrap();
                // Always set the region explicitly so a radius change clears any
                // stale region; a null region would mean "whole surface" to KWin.
                blur.set_region(Some(&region));
                blur.commit();
                region.destroy();
            }
            _ => {
                // It probably doesn't hurt to clear the blur for opaque windows
                blur_manager.unset(&state.surface);
                if let Some(b) = state.blur.take() {
                    b.release()
                }
            }
        }
    }

    if let Some(input_region) = input_region {
        input_region.destroy();
    }
    opaque_region.destroy();
}

pub(crate) trait WindowDecorationsExt {
    fn to_xdg(self) -> zxdg_toplevel_decoration_v1::Mode;
}

impl WindowDecorationsExt for WindowDecorations {
    fn to_xdg(self) -> zxdg_toplevel_decoration_v1::Mode {
        match self {
            WindowDecorations::Client => zxdg_toplevel_decoration_v1::Mode::ClientSide,
            WindowDecorations::Server => zxdg_toplevel_decoration_v1::Mode::ServerSide,
        }
    }
}

pub(crate) trait ResizeEdgeWaylandExt {
    fn to_xdg(self) -> xdg_toplevel::ResizeEdge;
}

impl ResizeEdgeWaylandExt for ResizeEdge {
    fn to_xdg(self) -> xdg_toplevel::ResizeEdge {
        match self {
            ResizeEdge::Top => xdg_toplevel::ResizeEdge::Top,
            ResizeEdge::TopRight => xdg_toplevel::ResizeEdge::TopRight,
            ResizeEdge::Right => xdg_toplevel::ResizeEdge::Right,
            ResizeEdge::BottomRight => xdg_toplevel::ResizeEdge::BottomRight,
            ResizeEdge::Bottom => xdg_toplevel::ResizeEdge::Bottom,
            ResizeEdge::BottomLeft => xdg_toplevel::ResizeEdge::BottomLeft,
            ResizeEdge::Left => xdg_toplevel::ResizeEdge::Left,
            ResizeEdge::TopLeft => xdg_toplevel::ResizeEdge::TopLeft,
        }
    }
}

/// The configuration event is in terms of the window geometry, which we are constantly
/// updating to account for the client decorations. But that's not the area we want to render
/// to, due to our intrusize CSD. So, here we calculate the 'actual' size, by adding back in the insets
fn compute_outer_size(
    inset: Pixels,
    new_size: Option<Size<Pixels>>,
    tiling: Tiling,
) -> Option<Size<Pixels>> {
    new_size.map(|mut new_size| {
        if !tiling.top {
            new_size.height += inset;
        }
        if !tiling.bottom {
            new_size.height += inset;
        }
        if !tiling.left {
            new_size.width += inset;
        }
        if !tiling.right {
            new_size.width += inset;
        }

        new_size
    })
}

fn inset_by_tiling(mut bounds: Bounds<Pixels>, inset: Pixels, tiling: Tiling) -> Bounds<Pixels> {
    if !tiling.top {
        bounds.origin.y += inset;
        bounds.size.height -= inset;
    }
    if !tiling.bottom {
        bounds.size.height -= inset;
    }
    if !tiling.left {
        bounds.origin.x += inset;
        bounds.size.width -= inset;
    }
    if !tiling.right {
        bounds.size.width -= inset;
    }

    bounds
}

#[cfg(test)]
mod tests {
    use super::{InputRegionState, ResolvedInputRegion, rounded_rect_region_bands};
    use gpui::{Bounds, OverlayInputMode, point, px, size};
    use std::collections::HashMap;

    #[test]
    fn radius_zero_is_single_full_rect() {
        assert_eq!(rounded_rect_region_bands(120, 80, 0), vec![(0, 0, 120, 80)]);
    }

    #[test]
    fn degenerate_sizes_yield_empty() {
        assert!(rounded_rect_region_bands(0, 80, 10).is_empty());
        assert!(rounded_rect_region_bands(120, 0, 10).is_empty());
        assert!(rounded_rect_region_bands(-5, 80, 10).is_empty());
    }

    #[test]
    fn band_count_is_two_r_plus_one() {
        let (w, h, r) = (200, 200, 30);
        assert!(r < w.min(h) / 2);
        let bands = rounded_rect_region_bands(w, h, r);
        assert_eq!(bands.len() as i32, 2 * r + 1);
    }

    #[test]
    fn top_bottom_symmetry() {
        let (w, h, r) = (200, 120, 25);
        let bands = rounded_rect_region_bands(w, h, r);
        // Map each band's y to its (x, width).
        let by_y: HashMap<i32, (i32, i32)> =
            bands.iter().map(|&(x, y, bw, _)| (y, (x, bw))).collect();
        for y in 0..r {
            let top = by_y[&y];
            let bottom = by_y[&(h - 1 - y)];
            assert_eq!(top, bottom, "rows {y} and {} differ", h - 1 - y);
        }
    }

    #[test]
    fn no_negative_widths_and_insets_in_range() {
        let (w, h, r) = (200, 120, 25);
        let bands = rounded_rect_region_bands(w, h, r);
        for &(x, _y, bw, bh) in &bands {
            assert!(bw >= 0, "negative width {bw}");
            assert!(bh >= 0, "negative height {bh}");
            // x is the inset for the rounded rows; must stay within [0, r].
            assert!(x >= 0 && x <= r, "inset {x} out of range");
        }
    }

    #[test]
    fn oversized_radius_clamps() {
        // min(width, height) / 2 == 25, so radius 100 behaves as radius 25.
        assert_eq!(
            rounded_rect_region_bands(200, 50, 100),
            rounded_rect_region_bands(200, 50, 25)
        );
    }

    #[test]
    fn insets_are_monotonic() {
        let (w, h, r) = (200, 120, 25);
        let bands = rounded_rect_region_bands(w, h, r);
        let by_y: HashMap<i32, i32> = bands.iter().map(|&(x, y, _, _)| (y, x)).collect();
        // Inset is widest at the very edge (y = 0) and shrinks toward the slab.
        let first = by_y[&0];
        let last = by_y[&(r - 1)];
        assert!(first >= last, "inset(0)={first} < inset(r-1)={last}");
        assert!(last >= 0);
    }

    #[test]
    fn input_region_last_setter_wins_and_survives_resolution() {
        let bounds = Bounds::new(point(px(0.), px(0.)), size(px(200.), px(120.)));
        let explicit = [Bounds::new(point(px(10.), px(12.)), size(px(40.), px(30.)))];
        let mut state = InputRegionState::default();

        assert_eq!(
            state.resolve(bounds),
            ResolvedInputRegion::OverlayInteractive(bounds)
        );
        state.set_explicit(Some(&explicit));
        assert_eq!(state.resolve(bounds), ResolvedInputRegion::Rects(&explicit));
        assert_eq!(
            state.resolve(Bounds::new(point(px(0.), px(0.)), size(px(640.), px(480.)))),
            ResolvedInputRegion::Rects(&explicit)
        );

        state.set_overlay_mode(OverlayInputMode::ClickThrough);
        assert_eq!(state.resolve(bounds), ResolvedInputRegion::Rects(&[]));

        state.set_explicit(None);
        assert_eq!(state.resolve(bounds), ResolvedInputRegion::Default);

        state.set_overlay_mode(OverlayInputMode::Interactive);
        assert_eq!(
            state.resolve(bounds),
            ResolvedInputRegion::OverlayInteractive(bounds)
        );
    }
}
