use crate::{CompositorGpuHint, WgpuAtlas, WgpuContext};
use bytemuck::{Pod, Zeroable};
use gpui::{
    AtlasTextureId, BackdropBlur, BackdropBlurPlan, Background, Bounds, ContentMask, DevicePixels,
    FirstPresentationObserver, GlobalElementId, GpuSpecs, MonochromeSprite, Path, Point,
    PolychromeSprite, PresentationEvidence, PrimitiveBatch, Quad, RendererAdapterType,
    RendererInfo, RendererKind, RendererSelection, RetainedLayer, RetainedLayerContentRevision,
    ScaledPixels, Scene, Shadow, Size, SubpixelSprite, TransformationMatrix, Underline,
    backdrop_blur_clusters, backdrop_blur_level_sizes_for, backdrop_blur_plan_groups,
    backdrop_scratch_bounds, backdrop_source_bounds, can_reuse_backdrop_texture,
    fit_backdrop_scratch_bounds, get_gamma_correction_ratios, max_backdrop_texture_size,
    prepare_backdrop_blurs,
};
use log::warn;
#[cfg(not(target_family = "wasm"))]
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use std::cell::RefCell;
use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
use std::num::NonZeroU64;
use std::ops::Range;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

const MAX_BACKDROP_BLUR_LEVELS: usize = BackdropBlurPlan::MAX_PASSES;
const BACKDROP_TEXTURE_SIZE_QUANTUM: i32 = 64;
const INITIAL_INSTANCE_BUFFER_SIZE: u64 = 2 * 1024 * 1024;
const INSTANCE_BUFFER_SIZE_BUCKET: u64 = 1024 * 1024;
const MAX_INSTANCE_BUFFER_SIZE: u64 = 256 * 1024 * 1024;

fn select_present_mode(
    preferred: Option<wgpu::PresentMode>,
    supported: &[wgpu::PresentMode],
) -> wgpu::PresentMode {
    preferred
        .filter(|mode| {
            matches!(
                mode,
                wgpu::PresentMode::AutoVsync | wgpu::PresentMode::AutoNoVsync
            ) || supported.contains(mode)
        })
        .unwrap_or(wgpu::PresentMode::Fifo)
}

#[cfg(not(target_family = "wasm"))]
fn recovery_requires_new_context(context_device_lost: Option<bool>) -> bool {
    context_device_lost.is_none_or(|device_lost| device_lost)
}

fn surface_usage(surface_usages: wgpu::TextureUsages) -> (wgpu::TextureUsages, bool) {
    let supports_copy_src = surface_usages.contains(wgpu::TextureUsages::COPY_SRC);
    let usage = if supports_copy_src {
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC
    } else {
        wgpu::TextureUsages::RENDER_ATTACHMENT
    };
    (usage, supports_copy_src)
}

fn backdrop_blur_format(surface_format: wgpu::TextureFormat) -> wgpu::TextureFormat {
    surface_format.remove_srgb_suffix()
}

#[derive(Default)]
struct FrameFailureStreak {
    count: u32,
    recovery_frame_pending: bool,
}

impl FrameFailureStreak {
    fn record_error(&mut self) -> u32 {
        self.count += 1;
        self.count
    }

    fn begin_recovery(&mut self) {
        self.recovery_frame_pending = true;
    }

    fn record_no_error(&mut self) {
        // Recovery returns before submitting a replacement frame. The next draw
        // therefore has no prior error to report, but it is not evidence that
        // the GPU recovered successfully.
        if !std::mem::take(&mut self.recovery_frame_pending) {
            self.count = 0;
        }
    }

    fn transfer_to(&mut self, replacement: &mut Self) {
        *replacement = std::mem::take(self);
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GlobalParams {
    viewport_size: [f32; 2],
    premultiplied_alpha: u32,
    pad: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PodBounds {
    origin: [f32; 2],
    size: [f32; 2],
}

impl From<Bounds<ScaledPixels>> for PodBounds {
    fn from(bounds: Bounds<ScaledPixels>) -> Self {
        Self {
            origin: [bounds.origin.x.0, bounds.origin.y.0],
            size: [bounds.size.width.0, bounds.size.height.0],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PodContentMask {
    bounds: PodBounds,
    rounded_bounds: PodBounds,
    corner_radii: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SurfaceParams {
    bounds: PodBounds,
    content_mask: PodContentMask,
}

#[derive(Clone, Debug)]
#[repr(C)]
struct RetainedLayerSprite {
    bounds: Bounds<ScaledPixels>,
    content_mask: ContentMask<ScaledPixels>,
    transformation: TransformationMatrix,
    opacity: f32,
    pad: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GammaParams {
    gamma_ratios: [f32; 4],
    grayscale_enhanced_contrast: f32,
    subpixel_enhanced_contrast: f32,
    is_bgr: u32,
    _pad: u32,
}

#[derive(Clone, Debug)]
#[repr(C)]
struct PathSprite {
    bounds: Bounds<ScaledPixels>,
    scratch_bounds: Bounds<ScaledPixels>,
    texture_size: [f32; 2],
}

#[derive(Clone, Debug)]
#[repr(C)]
struct PathRasterizationVertex {
    xy_position: Point<ScaledPixels>,
    st_position: Point<f32>,
    color: Background,
    bounds: Bounds<ScaledPixels>,
    content_mask: ContentMask<ScaledPixels>,
    scratch_bounds: Bounds<ScaledPixels>,
    texture_size: [f32; 2],
}

#[derive(Clone, Copy)]
struct PathScratchBounds {
    bounds: Bounds<ScaledPixels>,
    texture_size: Size<DevicePixels>,
}

#[derive(Clone, Copy)]
#[repr(C)]
struct BackdropBlurPassParams {
    input_size: [f32; 2],
    sample_distance: f32,
    pad: f32,
}

pub struct WgpuSurfaceConfig {
    pub size: Size<DevicePixels>,
    pub transparent: bool,
    /// Preferred presentation mode. When `Some`, the renderer will use this
    /// mode if supported by the surface, falling back to `Fifo`.
    /// When `None`, defaults to `Fifo` (VSync).
    ///
    /// Mobile platforms may prefer `Mailbox` (triple-buffering) to avoid
    /// blocking in `get_current_texture()` during lifecycle transitions.
    pub preferred_present_mode: Option<wgpu::PresentMode>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct DesiredSurfaceState {
    size: Size<DevicePixels>,
    transparent: bool,
    preferred_present_mode: Option<wgpu::PresentMode>,
    is_bgr: bool,
}

impl DesiredSurfaceState {
    #[cfg(not(target_family = "wasm"))]
    fn renderer_config(self) -> WgpuSurfaceConfig {
        WgpuSurfaceConfig {
            size: self.size,
            transparent: self.transparent,
            preferred_present_mode: self.preferred_present_mode,
        }
    }
}

struct WgpuPipelines {
    quads: wgpu::RenderPipeline,
    shadows: wgpu::RenderPipeline,
    backdrop_blurs: wgpu::RenderPipeline,
    backdrop_blur_downsample: wgpu::RenderPipeline,
    backdrop_blur_upsample: wgpu::RenderPipeline,
    path_rasterization: wgpu::RenderPipeline,
    paths: wgpu::RenderPipeline,
    underlines: wgpu::RenderPipeline,
    mono_sprites: wgpu::RenderPipeline,
    subpixel_sprites: Option<wgpu::RenderPipeline>,
    poly_sprites: wgpu::RenderPipeline,
    retained_layers: wgpu::RenderPipeline,
    #[allow(dead_code)]
    surfaces: wgpu::RenderPipeline,
}

struct WgpuBindGroupLayouts {
    globals: wgpu::BindGroupLayout,
    instances: wgpu::BindGroupLayout,
    instances_with_texture: wgpu::BindGroupLayout,
    surfaces: wgpu::BindGroupLayout,
}

struct WgpuRetainedLayer {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    content_revision: RetainedLayerContentRevision,
    texture_size: Size<DevicePixels>,
    valid: bool,
}

#[derive(Clone, Hash, PartialEq, Eq)]
struct RetainedLayerCacheKey {
    id: GlobalElementId,
    occurrence: usize,
}

struct PreparedRetainedLayer {
    cache_key: RetainedLayerCacheKey,
    layer: RetainedLayer,
    scene: Scene,
    draw_order: u32,
    cache_valid: bool,
    needs_render: bool,
}

enum DrawCommand {
    Batch {
        batch: PrimitiveBatch,
        order: u32,
        kind: u8,
    },
    RetainedLayer {
        layer_index: usize,
        order: u32,
        kind: u8,
    },
}

/// Shared GPU context reference, used to coordinate recovery across windows.
pub type GpuContext = Rc<RefCell<Option<WgpuContext>>>;

#[cfg(not(target_family = "wasm"))]
#[derive(Clone, Copy)]
struct SurfaceHandles {
    display: raw_window_handle::RawDisplayHandle,
    window: raw_window_handle::RawWindowHandle,
}

/// Every handle tied to a particular device is dropped as one unit on loss.
#[doc(hidden)]
pub struct WgpuResources {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    surface: wgpu::Surface<'static>,
    surface_supports_copy_src: bool,
    pipelines: WgpuPipelines,
    bind_group_layouts: WgpuBindGroupLayouts,
    atlas_sampler: wgpu::Sampler,
    globals_buffer: wgpu::Buffer,
    globals_bind_group: wgpu::BindGroup,
    path_globals_bind_group: wgpu::BindGroup,
    instance_buffer: wgpu::Buffer,
    path_intermediate_texture: Option<wgpu::Texture>,
    path_intermediate_view: Option<wgpu::TextureView>,
    path_msaa_texture: Option<wgpu::Texture>,
    path_msaa_view: Option<wgpu::TextureView>,
    path_intermediate_size: Option<Size<DevicePixels>>,
    backdrop_texture: Option<wgpu::Texture>,
    backdrop_view: Option<wgpu::TextureView>,
    backdrop_blur_input_view: Option<wgpu::TextureView>,
    backdrop_size: Option<Size<DevicePixels>>,
    backdrop_blur_level_sizes: Vec<Size<DevicePixels>>,
    backdrop_blur_downsample_textures: Vec<wgpu::Texture>,
    backdrop_blur_downsample_views: Vec<wgpu::TextureView>,
    backdrop_blur_upsample_textures: Vec<wgpu::Texture>,
    backdrop_blur_upsample_views: Vec<wgpu::TextureView>,
    retained_layers: HashMap<RetainedLayerCacheKey, WgpuRetainedLayer>,
}

impl WgpuResources {
    fn discard_backdrop_resources(&mut self) {
        self.backdrop_texture = None;
        self.backdrop_view = None;
        self.backdrop_blur_input_view = None;
        self.backdrop_size = None;
        self.backdrop_blur_level_sizes.clear();
        self.backdrop_blur_downsample_textures.clear();
        self.backdrop_blur_downsample_views.clear();
        self.backdrop_blur_upsample_textures.clear();
        self.backdrop_blur_upsample_views.clear();
    }

    fn destroy_backdrop_resources(&mut self) {
        if let Some(texture) = self.backdrop_texture.take() {
            texture.destroy();
        }
        for texture in self.backdrop_blur_downsample_textures.drain(..) {
            texture.destroy();
        }
        for texture in self.backdrop_blur_upsample_textures.drain(..) {
            texture.destroy();
        }
        self.discard_backdrop_resources();
    }

    fn invalidate_cached_gpu_state(&mut self) {
        self.path_intermediate_texture = None;
        self.path_intermediate_view = None;
        self.path_msaa_texture = None;
        self.path_msaa_view = None;
        self.path_intermediate_size = None;
        self.discard_backdrop_resources();
        self.retained_layers.clear();
    }
}

pub struct WgpuRenderer {
    #[allow(dead_code)]
    context: Option<GpuContext>,
    #[allow(dead_code)]
    compositor_gpu: Option<CompositorGpuHint>,
    resources: Option<WgpuResources>,
    #[cfg(not(target_family = "wasm"))]
    surface_handles: Option<SurfaceHandles>,
    surface_config: wgpu::SurfaceConfiguration,
    atlas: Arc<WgpuAtlas>,
    path_globals_offset: u64,
    gamma_offset: u64,
    instance_buffer_capacity: u64,
    storage_buffer_alignment: u64,
    desired: DesiredSurfaceState,
    rendering_params: RenderingParameters,
    dual_source_blending: bool,
    adapter_info: wgpu::AdapterInfo,
    renderer_selection: RendererSelection,
    transparent_alpha_mode: wgpu::CompositeAlphaMode,
    opaque_alpha_mode: wgpu::CompositeAlphaMode,
    max_texture_size: u32,
    last_error: Arc<Mutex<Option<String>>>,
    frame_failure_streak: FrameFailureStreak,
    device_lost: Arc<std::sync::atomic::AtomicBool>,
    needs_redraw: bool,
    first_presentation_observer: Option<FirstPresentationObserver>,
}

impl std::ops::Deref for WgpuRenderer {
    type Target = WgpuResources;

    fn deref(&self) -> &Self::Target {
        self.resources
            .as_ref()
            .expect("GPU resources not available")
    }
}

impl std::ops::DerefMut for WgpuRenderer {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.resources
            .as_mut()
            .expect("GPU resources not available")
    }
}

impl WgpuRenderer {
    pub fn supports_dual_source_blending(&self) -> bool {
        self.dual_source_blending
    }

    /// Installs the observer notified after `SurfaceTexture::present` returns.
    pub fn set_first_presentation_observer(&mut self, observer: FirstPresentationObserver) {
        self.first_presentation_observer = Some(observer);
    }

    fn record_first_presentation(&mut self) {
        if let Some(observer) = self.first_presentation_observer.take() {
            observer.record_presentation(PresentationEvidence::ApiSubmitted);
        }
    }

    /// Creates a new WgpuRenderer from raw window handles.
    ///
    /// # Safety
    /// The caller must ensure that the window handle remains valid for the lifetime
    /// of the returned renderer.
    #[cfg(not(target_family = "wasm"))]
    pub fn new<W: HasWindowHandle + HasDisplayHandle>(
        gpu_context: GpuContext,
        window: &W,
        config: WgpuSurfaceConfig,
        compositor_gpu: Option<CompositorGpuHint>,
    ) -> anyhow::Result<Self> {
        let window_handle = window
            .window_handle()
            .map_err(|e| anyhow::anyhow!("Failed to get window handle: {e}"))?;
        let display_handle = window
            .display_handle()
            .map_err(|e| anyhow::anyhow!("Failed to get display handle: {e}"))?;

        let surface_handles = SurfaceHandles {
            display: display_handle.as_raw(),
            window: window_handle.as_raw(),
        };

        let instance = gpu_context
            .borrow()
            .as_ref()
            .map(|context| context.instance.clone())
            .unwrap_or_else(WgpuContext::instance);

        // Safety: The caller guarantees that the window handle is valid for the
        // lifetime of this renderer. In practice, the RawWindow struct is created
        // from the native window handles and the surface is dropped before the window.
        let surface = unsafe {
            instance
                .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                    raw_display_handle: Some(surface_handles.display),
                    raw_window_handle: surface_handles.window,
                })
                .map_err(|e| anyhow::anyhow!("Failed to create surface: {e}"))?
        };

        if gpu_context.borrow().is_none() {
            *gpu_context.borrow_mut() = Some(WgpuContext::new_for_surface(
                instance,
                &surface,
                compositor_gpu,
            )?);
        }
        let context_ref = gpu_context.borrow();
        let context = context_ref.as_ref().expect("context was initialized");
        context.check_compatible_with_surface(&surface)?;

        Self::from_surface_internal(
            context,
            surface,
            config,
            Some(Rc::clone(&gpu_context)),
            compositor_gpu,
            None,
            Some(surface_handles),
        )
    }

    #[cfg(target_family = "wasm")]
    pub fn new_from_canvas(
        context: &WgpuContext,
        canvas: &web_sys::HtmlCanvasElement,
        config: WgpuSurfaceConfig,
    ) -> anyhow::Result<Self> {
        let surface = context
            .instance
            .create_surface(wgpu::SurfaceTarget::Canvas(canvas.clone()))
            .map_err(|e| anyhow::anyhow!("Failed to create surface: {e}"))?;

        Self::from_surface(context, surface, config)
    }

    #[cfg(target_family = "wasm")]
    fn from_surface(
        context: &WgpuContext,
        surface: wgpu::Surface<'static>,
        config: WgpuSurfaceConfig,
    ) -> anyhow::Result<Self> {
        Self::from_surface_internal(
            context,
            surface,
            config,
            None,
            None,
            None,
            #[cfg(not(target_family = "wasm"))]
            None,
        )
    }

    fn from_surface_internal(
        context: &WgpuContext,
        surface: wgpu::Surface<'static>,
        config: WgpuSurfaceConfig,
        gpu_context: Option<GpuContext>,
        compositor_gpu: Option<CompositorGpuHint>,
        atlas: Option<Arc<WgpuAtlas>>,
        #[cfg(not(target_family = "wasm"))] surface_handles: Option<SurfaceHandles>,
    ) -> anyhow::Result<Self> {
        let surface_caps = surface.get_capabilities(&context.adapter);
        let preferred_formats = [
            wgpu::TextureFormat::Bgra8Unorm,
            wgpu::TextureFormat::Rgba8Unorm,
        ];
        let surface_format = preferred_formats
            .iter()
            .find(|f| surface_caps.formats.contains(f))
            .copied()
            .or_else(|| surface_caps.formats.iter().find(|f| !f.is_srgb()).copied())
            .or_else(|| surface_caps.formats.first().copied())
            .ok_or_else(|| anyhow::anyhow!("Surface reports no supported texture formats"))?;

        let pick_alpha_mode =
            |preferences: &[wgpu::CompositeAlphaMode]| -> anyhow::Result<wgpu::CompositeAlphaMode> {
                preferences
                    .iter()
                    .find(|p| surface_caps.alpha_modes.contains(p))
                    .copied()
                    .or_else(|| surface_caps.alpha_modes.first().copied())
                    .ok_or_else(|| anyhow::anyhow!("Surface reports no supported alpha modes"))
            };

        let transparent_alpha_mode = pick_alpha_mode(&[
            wgpu::CompositeAlphaMode::PreMultiplied,
            wgpu::CompositeAlphaMode::Inherit,
        ])?;

        let opaque_alpha_mode = pick_alpha_mode(&[
            wgpu::CompositeAlphaMode::Opaque,
            wgpu::CompositeAlphaMode::Inherit,
        ])?;

        let alpha_mode = if config.transparent {
            transparent_alpha_mode
        } else {
            opaque_alpha_mode
        };

        let present_mode =
            select_present_mode(config.preferred_present_mode, &surface_caps.present_modes);

        Self::new_with_surface(
            context,
            surface,
            surface_format,
            surface_caps.usages,
            alpha_mode,
            present_mode,
            transparent_alpha_mode,
            opaque_alpha_mode,
            config,
            gpu_context,
            compositor_gpu,
            atlas,
            #[cfg(not(target_family = "wasm"))]
            surface_handles,
        )
    }

    fn new_with_surface(
        context: &WgpuContext,
        surface: wgpu::Surface<'static>,
        surface_format: wgpu::TextureFormat,
        surface_usages: wgpu::TextureUsages,
        alpha_mode: wgpu::CompositeAlphaMode,
        present_mode: wgpu::PresentMode,
        transparent_alpha_mode: wgpu::CompositeAlphaMode,
        opaque_alpha_mode: wgpu::CompositeAlphaMode,
        config: WgpuSurfaceConfig,
        gpu_context: Option<GpuContext>,
        compositor_gpu: Option<CompositorGpuHint>,
        atlas: Option<Arc<WgpuAtlas>>,
        #[cfg(not(target_family = "wasm"))] surface_handles: Option<SurfaceHandles>,
    ) -> anyhow::Result<Self> {
        let device = Arc::clone(&context.device);
        let max_texture_size = device.limits().max_texture_dimension_2d;

        let requested_width = config.size.width.0 as u32;
        let requested_height = config.size.height.0 as u32;
        let clamped_width = requested_width.min(max_texture_size);
        let clamped_height = requested_height.min(max_texture_size);

        if clamped_width != requested_width || clamped_height != requested_height {
            warn!(
                "Requested surface size ({}, {}) exceeds maximum texture dimension {}. \
                 Clamping to ({}, {}). Window content may not fill the entire window.",
                requested_width, requested_height, max_texture_size, clamped_width, clamped_height
            );
        }

        let (surface_usage, surface_supports_copy_src) = surface_usage(surface_usages);
        if !surface_supports_copy_src {
            warn!("WGPU surface lacks COPY_SRC usage; backdrop blur is disabled");
        }
        let surface_config = wgpu::SurfaceConfiguration {
            usage: surface_usage,
            format: surface_format,
            width: clamped_width.max(1),
            height: clamped_height.max(1),
            present_mode,
            desired_maximum_frame_latency: 2,
            alpha_mode,
            view_formats: vec![],
        };
        surface.configure(&context.device, &surface_config);
        let queue = Arc::clone(&context.queue);
        let dual_source_blending = context.supports_dual_source_blending();

        let rendering_params = RenderingParameters::new(&context.adapter, surface_format);
        let bind_group_layouts = Self::create_bind_group_layouts(&device);
        let pipelines = Self::create_pipelines(
            &device,
            &bind_group_layouts,
            surface_format,
            alpha_mode,
            rendering_params.path_sample_count,
            dual_source_blending,
        );

        let atlas = atlas
            .unwrap_or_else(|| Arc::new(WgpuAtlas::new(Arc::clone(&device), Arc::clone(&queue))));
        let atlas_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("atlas_sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let uniform_alignment = device.limits().min_uniform_buffer_offset_alignment as u64;
        let globals_size = std::mem::size_of::<GlobalParams>() as u64;
        let gamma_size = std::mem::size_of::<GammaParams>() as u64;
        let path_globals_offset = globals_size.next_multiple_of(uniform_alignment);
        let gamma_offset = (path_globals_offset + globals_size).next_multiple_of(uniform_alignment);

        let globals_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals_buffer"),
            size: gamma_offset + gamma_size,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let storage_buffer_alignment = device.limits().min_storage_buffer_offset_alignment as u64;
        let instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("instance_buffer"),
            size: INITIAL_INSTANCE_BUFFER_SIZE,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let globals_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals_bind_group"),
            layout: &bind_group_layouts.globals,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &globals_buffer,
                        offset: 0,
                        size: Some(NonZeroU64::new(globals_size).unwrap()),
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &globals_buffer,
                        offset: gamma_offset,
                        size: Some(NonZeroU64::new(gamma_size).unwrap()),
                    }),
                },
            ],
        });

        let path_globals_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("path_globals_bind_group"),
            layout: &bind_group_layouts.globals,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &globals_buffer,
                        offset: path_globals_offset,
                        size: Some(NonZeroU64::new(globals_size).unwrap()),
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &globals_buffer,
                        offset: gamma_offset,
                        size: Some(NonZeroU64::new(gamma_size).unwrap()),
                    }),
                },
            ],
        });

        let adapter_info = context.adapter.get_info();

        let last_error: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let last_error_clone = Arc::clone(&last_error);
        device.on_uncaptured_error(Arc::new(move |error| {
            let mut guard = last_error_clone.lock().unwrap();
            *guard = Some(error.to_string());
        }));

        let resources = WgpuResources {
            device,
            queue,
            surface,
            surface_supports_copy_src,
            pipelines,
            bind_group_layouts,
            atlas_sampler,
            globals_buffer,
            globals_bind_group,
            path_globals_bind_group,
            instance_buffer,
            path_intermediate_texture: None,
            path_intermediate_view: None,
            path_msaa_texture: None,
            path_msaa_view: None,
            path_intermediate_size: None,
            backdrop_texture: None,
            backdrop_view: None,
            backdrop_blur_input_view: None,
            backdrop_size: None,
            backdrop_blur_level_sizes: Vec::new(),
            backdrop_blur_downsample_textures: Vec::new(),
            backdrop_blur_downsample_views: Vec::new(),
            backdrop_blur_upsample_textures: Vec::new(),
            backdrop_blur_upsample_views: Vec::new(),
            retained_layers: HashMap::default(),
        };

        Ok(Self {
            context: gpu_context,
            compositor_gpu,
            resources: Some(resources),
            #[cfg(not(target_family = "wasm"))]
            surface_handles,
            surface_config,
            atlas,
            path_globals_offset,
            gamma_offset,
            instance_buffer_capacity: INITIAL_INSTANCE_BUFFER_SIZE,
            storage_buffer_alignment,
            desired: DesiredSurfaceState {
                size: config.size,
                transparent: config.transparent,
                preferred_present_mode: config.preferred_present_mode,
                is_bgr: false,
            },
            rendering_params,
            dual_source_blending,
            adapter_info,
            renderer_selection: context.renderer_selection(),
            transparent_alpha_mode,
            opaque_alpha_mode,
            max_texture_size,
            last_error,
            frame_failure_streak: FrameFailureStreak::default(),
            device_lost: context.device_lost_flag(),
            needs_redraw: false,
            first_presentation_observer: None,
        })
    }

    fn create_bind_group_layouts(device: &wgpu::Device) -> WgpuBindGroupLayouts {
        let globals =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("globals_layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: NonZeroU64::new(
                                std::mem::size_of::<GlobalParams>() as u64
                            ),
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: NonZeroU64::new(
                                std::mem::size_of::<GammaParams>() as u64
                            ),
                        },
                        count: None,
                    },
                ],
            });

        let storage_buffer_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };

        let instances = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("instances_layout"),
            entries: &[storage_buffer_entry(0)],
        });

        let instances_with_texture =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("instances_with_texture_layout"),
                entries: &[
                    storage_buffer_entry(0),
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });

        let surfaces = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("surfaces_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(
                            std::mem::size_of::<SurfaceParams>() as u64
                        ),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        WgpuBindGroupLayouts {
            globals,
            instances,
            instances_with_texture,
            surfaces,
        }
    }

    fn create_pipelines(
        device: &wgpu::Device,
        layouts: &WgpuBindGroupLayouts,
        surface_format: wgpu::TextureFormat,
        alpha_mode: wgpu::CompositeAlphaMode,
        path_sample_count: u32,
        dual_source_blending: bool,
    ) -> WgpuPipelines {
        let shader_source = include_str!("shaders.wgsl");
        let shader_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gpui_shaders"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });

        let blend_mode = match alpha_mode {
            wgpu::CompositeAlphaMode::PreMultiplied => {
                wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING
            }
            _ => wgpu::BlendState::ALPHA_BLENDING,
        };

        let color_target = wgpu::ColorTargetState {
            format: surface_format,
            blend: Some(blend_mode),
            write_mask: wgpu::ColorWrites::ALL,
        };

        let create_pipeline = |name: &str,
                               vs_entry: &str,
                               fs_entry: &str,
                               globals_layout: &wgpu::BindGroupLayout,
                               data_layout: &wgpu::BindGroupLayout,
                               topology: wgpu::PrimitiveTopology,
                               color_targets: &[Option<wgpu::ColorTargetState>],
                               sample_count: u32| {
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(&format!("{name}_layout")),
                bind_group_layouts: &[Some(globals_layout), Some(data_layout)],
                immediate_size: 0,
            });

            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(name),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader_module,
                    entry_point: Some(vs_entry),
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader_module,
                    entry_point: Some(fs_entry),
                    targets: color_targets,
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    topology,
                    strip_index_format: None,
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: None,
                    polygon_mode: wgpu::PolygonMode::Fill,
                    unclipped_depth: false,
                    conservative: false,
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState {
                    count: sample_count,
                    mask: !0,
                    alpha_to_coverage_enabled: false,
                },
                multiview_mask: None,
                cache: None,
            })
        };

        let quads = create_pipeline(
            "quads",
            "vs_quad",
            "fs_quad",
            &layouts.globals,
            &layouts.instances,
            wgpu::PrimitiveTopology::TriangleStrip,
            &[Some(color_target.clone())],
            1,
        );

        let shadows = create_pipeline(
            "shadows",
            "vs_shadow",
            "fs_shadow",
            &layouts.globals,
            &layouts.instances,
            wgpu::PrimitiveTopology::TriangleStrip,
            &[Some(color_target.clone())],
            1,
        );
        let backdrop_blurs = create_pipeline(
            "backdrop_blurs",
            "vs_backdrop_blur",
            "fs_backdrop_blur",
            &layouts.globals,
            &layouts.instances_with_texture,
            wgpu::PrimitiveTopology::TriangleStrip,
            &[Some(color_target.clone())],
            1,
        );
        let backdrop_blur_target = wgpu::ColorTargetState {
            format: backdrop_blur_format(surface_format),
            blend: None,
            write_mask: wgpu::ColorWrites::ALL,
        };
        let backdrop_blur_downsample = create_pipeline(
            "backdrop_blur_downsample",
            "vs_backdrop_blur_pass",
            "fs_backdrop_blur_downsample",
            &layouts.globals,
            &layouts.instances_with_texture,
            wgpu::PrimitiveTopology::TriangleStrip,
            &[Some(backdrop_blur_target.clone())],
            1,
        );
        let backdrop_blur_upsample = create_pipeline(
            "backdrop_blur_upsample",
            "vs_backdrop_blur_pass",
            "fs_backdrop_blur_upsample",
            &layouts.globals,
            &layouts.instances_with_texture,
            wgpu::PrimitiveTopology::TriangleStrip,
            &[Some(backdrop_blur_target)],
            1,
        );

        let path_rasterization = create_pipeline(
            "path_rasterization",
            "vs_path_rasterization",
            "fs_path_rasterization",
            &layouts.globals,
            &layouts.instances,
            wgpu::PrimitiveTopology::TriangleList,
            &[Some(wgpu::ColorTargetState {
                format: surface_format,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            path_sample_count,
        );

        let paths_blend = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        };

        let paths = create_pipeline(
            "paths",
            "vs_path",
            "fs_path",
            &layouts.globals,
            &layouts.instances_with_texture,
            wgpu::PrimitiveTopology::TriangleStrip,
            &[Some(wgpu::ColorTargetState {
                format: surface_format,
                blend: Some(paths_blend),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            1,
        );

        let underlines = create_pipeline(
            "underlines",
            "vs_underline",
            "fs_underline",
            &layouts.globals,
            &layouts.instances,
            wgpu::PrimitiveTopology::TriangleStrip,
            &[Some(color_target.clone())],
            1,
        );

        let mono_sprites = create_pipeline(
            "mono_sprites",
            "vs_mono_sprite",
            "fs_mono_sprite",
            &layouts.globals,
            &layouts.instances_with_texture,
            wgpu::PrimitiveTopology::TriangleStrip,
            &[Some(color_target.clone())],
            1,
        );

        let subpixel_sprites = if dual_source_blending {
            let subpixel_blend = wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::Src1,
                    dst_factor: wgpu::BlendFactor::OneMinusSrc1,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::One,
                    dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                    operation: wgpu::BlendOperation::Add,
                },
            };

            Some(create_pipeline(
                "subpixel_sprites",
                "vs_subpixel_sprite",
                "fs_subpixel_sprite",
                &layouts.globals,
                &layouts.instances_with_texture,
                wgpu::PrimitiveTopology::TriangleStrip,
                &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(subpixel_blend),
                    write_mask: wgpu::ColorWrites::COLOR,
                })],
                1,
            ))
        } else {
            None
        };

        let poly_sprites = create_pipeline(
            "poly_sprites",
            "vs_poly_sprite",
            "fs_poly_sprite",
            &layouts.globals,
            &layouts.instances_with_texture,
            wgpu::PrimitiveTopology::TriangleStrip,
            &[Some(color_target.clone())],
            1,
        );

        let retained_layers = create_pipeline(
            "retained_layers",
            "vs_retained_layer",
            "fs_retained_layer",
            &layouts.globals,
            &layouts.instances_with_texture,
            wgpu::PrimitiveTopology::TriangleStrip,
            &[Some(color_target.clone())],
            1,
        );

        let surfaces = create_pipeline(
            "surfaces",
            "vs_surface",
            "fs_surface",
            &layouts.globals,
            &layouts.surfaces,
            wgpu::PrimitiveTopology::TriangleStrip,
            &[Some(color_target)],
            1,
        );

        WgpuPipelines {
            quads,
            shadows,
            backdrop_blurs,
            backdrop_blur_downsample,
            backdrop_blur_upsample,
            path_rasterization,
            paths,
            underlines,
            mono_sprites,
            subpixel_sprites,
            poly_sprites,
            retained_layers,
            surfaces,
        }
    }

    fn create_path_intermediate(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> (wgpu::Texture, wgpu::TextureView) {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("path_intermediate"),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        (texture, view)
    }

    fn create_backdrop_texture(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> (wgpu::Texture, wgpu::TextureView, wgpu::TextureView) {
        let linear_format = backdrop_blur_format(format);
        let view_formats = (linear_format != format).then_some([linear_format]);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("backdrop_texture"),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: view_formats.as_ref().map_or(&[], |formats| formats),
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let blur_input_view = texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(linear_format),
            ..Default::default()
        });
        (texture, view, blur_input_view)
    }

    fn create_backdrop_blur_texture(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        size: Size<DevicePixels>,
        label: &'static str,
    ) -> (wgpu::Texture, wgpu::TextureView) {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: size.width.0.max(1) as u32,
                height: size.height.0.max(1) as u32,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        (texture, view)
    }

    fn create_msaa_if_needed(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
        sample_count: u32,
    ) -> Option<(wgpu::Texture, wgpu::TextureView)> {
        if sample_count <= 1 {
            return None;
        }
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("path_msaa"),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Some((texture, view))
    }

    pub fn update_drawable_size(&mut self, size: Size<DevicePixels>) {
        self.desired.size = size;
        if self.resources.is_none() {
            return;
        }
        let width = size.width.0 as u32;
        let height = size.height.0 as u32;

        if width != self.surface_config.width || height != self.surface_config.height {
            let clamped_width = width.min(self.max_texture_size);
            let clamped_height = height.min(self.max_texture_size);

            if clamped_width != width || clamped_height != height {
                warn!(
                    "Requested surface size ({}, {}) exceeds maximum texture dimension {}. \
                     Clamping to ({}, {}). Window content may not fill the entire window.",
                    width, height, self.max_texture_size, clamped_width, clamped_height
                );
            }

            // Wait for any in-flight GPU work to complete before destroying textures
            if let Err(e) = self.device.poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            }) {
                warn!("Failed to poll device during resize: {e:?}");
            }

            // Destroy old textures before allocating new ones to avoid GPU memory spikes
            if let Some(ref texture) = self.path_intermediate_texture {
                texture.destroy();
            }
            if let Some(ref texture) = self.path_msaa_texture {
                texture.destroy();
            }

            self.surface_config.width = clamped_width.max(1);
            self.surface_config.height = clamped_height.max(1);
            self.surface.configure(&self.device, &self.surface_config);

            // Invalidate intermediate textures - they will be lazily recreated
            // in draw() after we confirm the surface is healthy. This avoids
            // panics when the device/surface is in an invalid state during resize.
            self.path_intermediate_texture = None;
            self.path_intermediate_view = None;
            self.path_msaa_texture = None;
            self.path_msaa_view = None;
            self.path_intermediate_size = None;

            self.destroy_backdrop_resources();
        }
    }

    fn ensure_intermediate_textures(
        &mut self,
        size: Size<DevicePixels>,
    ) -> Option<Size<DevicePixels>> {
        if let Some(current_size) = self.path_intermediate_size {
            if current_size.width >= size.width && current_size.height >= size.height {
                return Some(current_size);
            }
            return self.create_intermediate_textures(Size {
                width: current_size.width.max(size.width),
                height: current_size.height.max(size.height),
            });
        }

        self.create_intermediate_textures(size)
    }

    fn create_intermediate_textures(
        &mut self,
        size: Size<DevicePixels>,
    ) -> Option<Size<DevicePixels>> {
        if size.width.0 <= 0 || size.height.0 <= 0 {
            self.path_intermediate_texture = None;
            self.path_intermediate_view = None;
            self.path_msaa_texture = None;
            self.path_msaa_view = None;
            self.path_intermediate_size = None;
            return None;
        }

        if let Some(ref texture) = self.path_intermediate_texture {
            texture.destroy();
        }
        if let Some(ref texture) = self.path_msaa_texture {
            texture.destroy();
        }

        let (path_intermediate_texture, path_intermediate_view) = {
            let (t, v) = Self::create_path_intermediate(
                &self.device,
                self.surface_config.format,
                size.width.0 as u32,
                size.height.0 as u32,
            );
            (Some(t), Some(v))
        };
        self.path_intermediate_texture = path_intermediate_texture;
        self.path_intermediate_view = path_intermediate_view;

        let (path_msaa_texture, path_msaa_view) = Self::create_msaa_if_needed(
            &self.device,
            self.surface_config.format,
            size.width.0 as u32,
            size.height.0 as u32,
            self.rendering_params.path_sample_count,
        )
        .map(|(t, v)| (Some(t), Some(v)))
        .unwrap_or((None, None));
        self.path_msaa_texture = path_msaa_texture;
        self.path_msaa_view = path_msaa_view;
        self.path_intermediate_size = Some(size);
        Some(size)
    }

    fn ensure_backdrop_texture(
        &mut self,
        size: Size<DevicePixels>,
        max_size: Size<DevicePixels>,
    ) -> Option<Size<DevicePixels>> {
        let size = Self::quantize_backdrop_texture_size(size, max_size);
        if let Some(current_size) = self.backdrop_size
            && self.backdrop_texture.is_some()
        {
            if can_reuse_backdrop_texture(current_size, size) {
                return Some(current_size);
            }
            return self.create_backdrop_resources(Size {
                width: current_size.width.max(size.width).min(max_size.width),
                height: current_size.height.max(size.height).min(max_size.height),
            });
        }

        self.create_backdrop_resources(size)
    }

    fn quantize_backdrop_texture_size(
        size: Size<DevicePixels>,
        max_size: Size<DevicePixels>,
    ) -> Size<DevicePixels> {
        fn quantize(value: DevicePixels, max_value: DevicePixels) -> DevicePixels {
            if value.0 <= 0 {
                return value;
            }

            let rounded = ((value.0 + BACKDROP_TEXTURE_SIZE_QUANTUM - 1)
                / BACKDROP_TEXTURE_SIZE_QUANTUM)
                * BACKDROP_TEXTURE_SIZE_QUANTUM;
            DevicePixels(rounded.min(max_value.0.max(0)))
        }

        Size {
            width: quantize(size.width, max_size.width),
            height: quantize(size.height, max_size.height),
        }
    }

    fn create_backdrop_resources(
        &mut self,
        size: Size<DevicePixels>,
    ) -> Option<Size<DevicePixels>> {
        if size.width.0 <= 0 || size.height.0 <= 0 {
            self.discard_backdrop_resources();
            return None;
        }

        let (backdrop_texture, backdrop_view, backdrop_blur_input_view) =
            Self::create_backdrop_texture(
                &self.device,
                self.surface_config.format,
                size.width.0 as u32,
                size.height.0 as u32,
            );
        let level_sizes = backdrop_blur_level_sizes_for(size);
        let blur_format = backdrop_blur_format(self.surface_config.format);
        let mut downsample_textures = Vec::with_capacity(level_sizes.len().saturating_sub(1));
        let mut downsample_views = Vec::with_capacity(level_sizes.len().saturating_sub(1));
        for &level_size in level_sizes.iter().skip(1) {
            let (texture, view) = Self::create_backdrop_blur_texture(
                &self.device,
                blur_format,
                level_size,
                "backdrop_blur_downsample",
            );
            downsample_textures.push(texture);
            downsample_views.push(view);
        }
        let mut upsample_textures = Vec::with_capacity(level_sizes.len());
        let mut upsample_views = Vec::with_capacity(level_sizes.len());
        for &level_size in &level_sizes {
            let (texture, view) = Self::create_backdrop_blur_texture(
                &self.device,
                blur_format,
                level_size,
                "backdrop_blur_upsample",
            );
            upsample_textures.push(texture);
            upsample_views.push(view);
        }
        self.backdrop_texture = Some(backdrop_texture);
        self.backdrop_view = Some(backdrop_view);
        self.backdrop_blur_input_view = Some(backdrop_blur_input_view);
        self.backdrop_size = Some(size);
        self.backdrop_blur_level_sizes = level_sizes;
        self.backdrop_blur_downsample_textures = downsample_textures;
        self.backdrop_blur_downsample_views = downsample_views;
        self.backdrop_blur_upsample_textures = upsample_textures;
        self.backdrop_blur_upsample_views = upsample_views;
        Some(size)
    }

    pub fn update_transparency(&mut self, transparent: bool) {
        self.desired.transparent = transparent;
        if self.resources.is_none() {
            return;
        }
        let new_alpha_mode = if transparent {
            self.transparent_alpha_mode
        } else {
            self.opaque_alpha_mode
        };

        if new_alpha_mode != self.surface_config.alpha_mode {
            self.surface_config.alpha_mode = new_alpha_mode;
            self.surface.configure(&self.device, &self.surface_config);
            self.pipelines = Self::create_pipelines(
                &self.device,
                &self.bind_group_layouts,
                self.surface_config.format,
                self.surface_config.alpha_mode,
                self.rendering_params.path_sample_count,
                self.dual_source_blending,
            );
        }
    }

    pub fn max_texture_size(&self) -> u32 {
        self.max_texture_size
    }

    pub fn set_subpixel_layout(&mut self, is_bgr: bool) {
        self.desired.is_bgr = is_bgr;
    }

    #[allow(dead_code)]
    pub fn viewport_size(&self) -> Size<DevicePixels> {
        Size {
            width: DevicePixels(self.surface_config.width as i32),
            height: DevicePixels(self.surface_config.height as i32),
        }
    }

    pub fn sprite_atlas(&self) -> &Arc<WgpuAtlas> {
        &self.atlas
    }

    pub fn renderer_info(&self) -> RendererInfo {
        let adapter_type = match self.adapter_info.device_type {
            wgpu::DeviceType::Cpu => RendererAdapterType::Software,
            wgpu::DeviceType::Other => RendererAdapterType::Unknown,
            _ => RendererAdapterType::Hardware,
        };
        RendererInfo {
            selection: self.renderer_selection,
            renderer: RendererKind::Wgpu,
            backend: format!("{:?}", self.adapter_info.backend),
            adapter_name: self.adapter_info.name.clone(),
            adapter_type,
            vendor_id: Some(self.adapter_info.vendor),
            device_id: Some(self.adapter_info.device),
        }
    }

    pub fn gpu_specs(&self) -> GpuSpecs {
        GpuSpecs {
            is_software_emulated: self.adapter_info.device_type == wgpu::DeviceType::Cpu,
            device_name: self.adapter_info.name.clone(),
            driver_name: self.adapter_info.driver.clone(),
            driver_info: self.adapter_info.driver_info.clone(),
        }
    }

    /// Draws a frame and returns whether a surface buffer was presented.
    pub fn draw(&mut self, scene: &Scene) -> bool {
        if self.resources.is_none() {
            self.needs_redraw = true;
            return false;
        }
        let last_error = self.last_error.lock().unwrap().take();
        if let Some(error) = last_error {
            let failed_frame_count = self.frame_failure_streak.record_error();
            log::error!(
                "GPU error during frame (failure {} of 20): {error}",
                failed_frame_count
            );
            if failed_frame_count > 20 {
                panic!("Too many consecutive GPU errors. Last error: {error}");
            } else if failed_frame_count > 5 {
                self.resources
                    .as_mut()
                    .expect("GPU resources checked above")
                    .invalidate_cached_gpu_state();
                self.atlas.clear();
                self.needs_redraw = true;
                self.frame_failure_streak.begin_recovery();
                return false;
            }
        } else {
            self.frame_failure_streak.record_no_error();
        }

        self.atlas.before_frame();
        let viewport_size = self.viewport_size();
        if !scene
            .backdrop_blurs
            .iter()
            .any(|blur| backdrop_source_bounds(blur, viewport_size).is_some())
        {
            self.discard_backdrop_resources();
        }

        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => frame,
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                // Textures must be destroyed before the surface can be reconfigured.
                drop(frame);
                self.surface.configure(&self.device, &self.surface_config);
                return false;
            }
            wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated => {
                self.surface.configure(&self.device, &self.surface_config);
                return false;
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return false;
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                *self.last_error.lock().unwrap() =
                    Some("Surface texture validation error".to_string());
                return false;
            }
        };

        let frame_view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut prepared_retained_layers = self.prepare_retained_layers(scene);
        let retained_instance_bytes =
            prepared_retained_layers.len() * std::mem::size_of::<RetainedLayerSprite>();
        let main_capacity =
            self.required_instance_buffer_size(scene) + retained_instance_bytes as u64;
        let retained_capacity = prepared_retained_layers
            .iter()
            .filter(|layer| layer.cache_valid && layer.needs_render)
            .map(|layer| self.required_instance_buffer_size(&layer.scene))
            .max()
            .unwrap_or(0);
        let required_capacity = main_capacity.max(retained_capacity);
        self.ensure_instance_buffer_capacity(required_capacity);

        {
            let mut overflow = false;

            for prepared_layer in &mut prepared_retained_layers {
                if !prepared_layer.cache_valid || !prepared_layer.needs_render {
                    continue;
                }
                let Some(mut cache) = self.retained_layers.remove(&prepared_layer.cache_key) else {
                    prepared_layer.cache_valid = false;
                    continue;
                };

                let mut encoder =
                    self.device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("retained_layer_encoder"),
                        });
                let mut instance_offset: u64 = 0;

                self.write_globals(cache.texture_size);
                let rendered = self.draw_scene_batches(
                    &prepared_layer.scene,
                    &mut encoder,
                    &cache.texture,
                    &cache.view,
                    cache.texture_size,
                    true,
                    wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    &mut instance_offset,
                );
                if rendered {
                    cache.valid = true;
                    cache.content_revision = prepared_layer.layer.content_revision;
                    self.queue.submit(std::iter::once(encoder.finish()));
                }
                self.retained_layers
                    .insert(prepared_layer.cache_key.clone(), cache);
                if !rendered {
                    prepared_layer.cache_valid = false;
                    overflow = true;
                    break;
                }
            }

            if !overflow {
                self.write_globals(self.viewport_size());
                let excluded_ranges: Vec<_> = prepared_retained_layers
                    .iter()
                    .filter(|layer| layer.cache_valid)
                    .map(|layer| layer.layer.paint_range.clone())
                    .collect();
                let main_scene_storage;
                let main_scene = if excluded_ranges.is_empty() {
                    scene
                } else {
                    main_scene_storage = scene.clone_excluding_paint_ranges(&excluded_ranges);
                    &main_scene_storage
                };
                let mut encoder =
                    self.device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("main_encoder"),
                        });
                let mut instance_offset: u64 = 0;

                overflow = !self.draw_main_scene(
                    main_scene,
                    &prepared_retained_layers,
                    &mut encoder,
                    &frame.texture,
                    &frame_view,
                    self.viewport_size(),
                    self.surface_supports_copy_src,
                    &mut instance_offset,
                );

                if !overflow {
                    self.queue.submit(std::iter::once(encoder.finish()));
                }
            }

            if overflow {
                log::error!(
                    "precomputed instance buffer size was too small: {}",
                    self.instance_buffer_capacity
                );
                frame.present();
                return true;
            }

            frame.present();
            self.record_first_presentation();
        }
        true
    }

    fn prepare_retained_layers(&mut self, scene: &Scene) -> Vec<PreparedRetainedLayer> {
        let mut occurrences = HashMap::new();
        let cache_keys = scene
            .retained_layers
            .iter()
            .map(|layer| {
                let occurrence = occurrences.entry(layer.id.clone()).or_insert(0);
                let cache_key = RetainedLayerCacheKey {
                    id: layer.id.clone(),
                    occurrence: *occurrence,
                };
                *occurrence += 1;
                cache_key
            })
            .collect::<Vec<_>>();
        let prepared = Self::top_level_retained_layer_indices(&scene.retained_layers)
            .iter()
            .filter_map(|&index| {
                let layer = scene.retained_layers[index].clone();
                let cache_key = cache_keys[index].clone();
                let texture_size = Self::retained_layer_texture_size(&layer)?;
                if texture_size.width.0 as u32 > self.max_texture_size
                    || texture_size.height.0 as u32 > self.max_texture_size
                {
                    warn!(
                        "retained layer {} exceeds maximum texture dimension {}; drawing content normally",
                        layer.id, self.max_texture_size
                    );
                    return None;
                }

                let layer_scene = scene.clone_paint_range(layer.paint_range.clone());
                if Self::retained_scene_contains_backdrop_blurs(&layer_scene) {
                    return None;
                }

                let mut layer_scene = layer_scene;
                let draw_order = Self::first_draw_order(&layer_scene).unwrap_or(u32::MAX);
                Self::localize_scene(&mut layer_scene, layer.bounds.origin);
                let needs_render =
                    self.ensure_retained_layer_texture(&cache_key, &layer, texture_size);

                Some(PreparedRetainedLayer {
                    cache_key,
                    layer,
                    scene: layer_scene,
                    draw_order,
                    cache_valid: true,
                    needs_render,
                })
            })
            .collect::<Vec<_>>();

        let active_ids = prepared
            .iter()
            .map(|layer| layer.cache_key.clone())
            .collect::<HashSet<_>>();
        let stale_ids = self
            .retained_layers
            .keys()
            .filter(|id| !active_ids.contains(*id))
            .cloned()
            .collect::<Vec<_>>();
        for id in stale_ids {
            if let Some(layer) = self.retained_layers.remove(&id) {
                layer.texture.destroy();
            }
        }

        prepared
    }

    fn retained_scene_contains_backdrop_blurs(scene: &Scene) -> bool {
        !scene.backdrop_blurs.is_empty()
    }

    fn top_level_retained_layer_indices(layers: &[RetainedLayer]) -> Vec<usize> {
        let mut indices = (0..layers.len()).collect::<Vec<_>>();
        indices.sort_by_key(|&index| {
            let range = &layers[index].paint_range;
            (range.start, Reverse(range.end), index)
        });

        let mut accepted: Vec<usize> = Vec::new();
        for index in indices {
            let range = &layers[index].paint_range;
            if accepted
                .iter()
                .any(|&accepted| Self::paint_ranges_overlap(range, &layers[accepted].paint_range))
            {
                continue;
            }
            accepted.push(index);
        }
        accepted.sort_unstable();
        accepted
    }

    fn paint_ranges_overlap(a: &Range<usize>, b: &Range<usize>) -> bool {
        a.start < b.end && b.start < a.end
    }

    fn retained_layer_texture_size(layer: &RetainedLayer) -> Option<Size<DevicePixels>> {
        if layer.bounds.is_empty() {
            return None;
        }
        Some(Size {
            width: DevicePixels::from(layer.bounds.size.width).max(DevicePixels(1)),
            height: DevicePixels::from(layer.bounds.size.height).max(DevicePixels(1)),
        })
    }

    fn ensure_retained_layer_texture(
        &mut self,
        cache_key: &RetainedLayerCacheKey,
        layer: &RetainedLayer,
        texture_size: Size<DevicePixels>,
    ) -> bool {
        let needs_render = match self.retained_layers.get(cache_key) {
            Some(cache) => {
                !cache.valid
                    || layer.content_dirty
                    || cache.content_revision != layer.content_revision
                    || cache.texture_size != texture_size
            }
            None => true,
        };

        let needs_texture = self
            .retained_layers
            .get(cache_key)
            .is_none_or(|cache| cache.texture_size != texture_size);

        if needs_texture {
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("retained_layer_texture"),
                size: wgpu::Extent3d {
                    width: texture_size.width.0.max(1) as u32,
                    height: texture_size.height.0.max(1) as u32,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.surface_config.format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            if let Some(old) = self.retained_layers.insert(
                cache_key.clone(),
                WgpuRetainedLayer {
                    texture,
                    view,
                    content_revision: layer.content_revision,
                    texture_size,
                    valid: false,
                },
            ) {
                old.texture.destroy();
            }
        } else if needs_render && let Some(cache) = self.retained_layers.get_mut(cache_key) {
            cache.valid = false;
        }

        needs_render
    }

    fn localize_scene(scene: &mut Scene, origin: Point<ScaledPixels>) {
        for quad in &mut scene.quads {
            quad.bounds = quad.bounds - origin;
            Self::localize_content_mask(&mut quad.content_mask, origin);
        }
        for shadow in &mut scene.shadows {
            shadow.bounds = shadow.bounds - origin;
            shadow.element_bounds = shadow.element_bounds - origin;
            Self::localize_content_mask(&mut shadow.content_mask, origin);
        }
        for blur in &mut scene.backdrop_blurs {
            blur.bounds = blur.bounds - origin;
            Self::localize_content_mask(&mut blur.content_mask, origin);
        }
        for path in &mut scene.paths {
            path.bounds = path.bounds - origin;
            Self::localize_content_mask(&mut path.content_mask, origin);
            for vertex in &mut path.vertices {
                vertex.xy_position -= origin;
                Self::localize_content_mask(&mut vertex.content_mask, origin);
            }
        }
        for underline in &mut scene.underlines {
            underline.bounds = underline.bounds - origin;
            Self::localize_content_mask(&mut underline.content_mask, origin);
        }
        for sprite in &mut scene.monochrome_sprites {
            sprite.bounds = sprite.bounds - origin;
            Self::localize_content_mask(&mut sprite.content_mask, origin);
            sprite.transformation = Self::localize_transform(sprite.transformation, origin);
        }
        for sprite in &mut scene.subpixel_sprites {
            sprite.bounds = sprite.bounds - origin;
            Self::localize_content_mask(&mut sprite.content_mask, origin);
            sprite.transformation = Self::localize_transform(sprite.transformation, origin);
        }
        for sprite in &mut scene.polychrome_sprites {
            sprite.bounds = sprite.bounds - origin;
            Self::localize_content_mask(&mut sprite.content_mask, origin);
        }
        for surface in &mut scene.surfaces {
            surface.bounds = surface.bounds - origin;
            Self::localize_content_mask(&mut surface.content_mask, origin);
        }
    }

    fn localize_content_mask(
        content_mask: &mut ContentMask<ScaledPixels>,
        origin: Point<ScaledPixels>,
    ) {
        content_mask.bounds = content_mask.bounds - origin;
        content_mask.rounded_bounds = content_mask.rounded_bounds - origin;
    }

    fn localize_transform(
        transform: TransformationMatrix,
        origin: Point<ScaledPixels>,
    ) -> TransformationMatrix {
        let origin = [origin.x.0, origin.y.0];
        TransformationMatrix {
            rotation_scale: transform.rotation_scale,
            translation: [
                transform.rotation_scale[0][0] * origin[0]
                    + transform.rotation_scale[0][1] * origin[1]
                    + transform.translation[0]
                    - origin[0],
                transform.rotation_scale[1][0] * origin[0]
                    + transform.rotation_scale[1][1] * origin[1]
                    + transform.translation[1]
                    - origin[1],
            ],
        }
    }

    fn first_draw_order(scene: &Scene) -> Option<u32> {
        scene
            .batches()
            .next()
            .map(|batch| Self::batch_key(scene, &batch).0)
    }

    fn write_globals(&self, viewport_size: Size<DevicePixels>) {
        let gamma_params = GammaParams {
            gamma_ratios: self.rendering_params.gamma_ratios,
            grayscale_enhanced_contrast: self.rendering_params.grayscale_enhanced_contrast,
            subpixel_enhanced_contrast: self.rendering_params.subpixel_enhanced_contrast,
            is_bgr: self.desired.is_bgr as u32,
            _pad: 0,
        };
        let globals = GlobalParams {
            viewport_size: [viewport_size.width.0 as f32, viewport_size.height.0 as f32],
            premultiplied_alpha: if self.surface_config.alpha_mode
                == wgpu::CompositeAlphaMode::PreMultiplied
            {
                1
            } else {
                0
            },
            pad: 0,
        };
        let path_globals = GlobalParams {
            premultiplied_alpha: 0,
            ..globals
        };

        self.queue
            .write_buffer(&self.globals_buffer, 0, bytemuck::bytes_of(&globals));
        self.queue.write_buffer(
            &self.globals_buffer,
            self.path_globals_offset,
            bytemuck::bytes_of(&path_globals),
        );
        self.queue.write_buffer(
            &self.globals_buffer,
            self.gamma_offset,
            bytemuck::bytes_of(&gamma_params),
        );
    }

    fn draw_main_scene(
        &mut self,
        scene: &Scene,
        retained_layers: &[PreparedRetainedLayer],
        encoder: &mut wgpu::CommandEncoder,
        target_texture: &wgpu::Texture,
        target_view: &wgpu::TextureView,
        target_size: Size<DevicePixels>,
        target_supports_copy_src: bool,
        instance_offset: &mut u64,
    ) -> bool {
        let mut commands = scene
            .batches()
            .map(|batch| {
                let (order, kind) = Self::batch_key(scene, &batch);
                DrawCommand::Batch { batch, order, kind }
            })
            .collect::<Vec<_>>();

        for (layer_index, layer) in retained_layers.iter().enumerate() {
            if layer.cache_valid {
                commands.push(DrawCommand::RetainedLayer {
                    layer_index,
                    order: layer.draw_order,
                    kind: 9,
                });
            }
        }

        commands.sort_by_key(|command| match command {
            DrawCommand::Batch { order, kind, .. }
            | DrawCommand::RetainedLayer { order, kind, .. } => (*order, *kind),
        });

        self.draw_commands(
            scene,
            retained_layers,
            &commands,
            encoder,
            target_texture,
            target_view,
            target_size,
            target_supports_copy_src,
            wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
            instance_offset,
        )
    }

    fn draw_scene_batches(
        &mut self,
        scene: &Scene,
        encoder: &mut wgpu::CommandEncoder,
        target_texture: &wgpu::Texture,
        target_view: &wgpu::TextureView,
        target_size: Size<DevicePixels>,
        target_supports_copy_src: bool,
        load: wgpu::LoadOp<wgpu::Color>,
        instance_offset: &mut u64,
    ) -> bool {
        let commands = scene
            .batches()
            .map(|batch| {
                let (order, kind) = Self::batch_key(scene, &batch);
                DrawCommand::Batch { batch, order, kind }
            })
            .collect::<Vec<_>>();
        self.draw_commands(
            scene,
            &[],
            &commands,
            encoder,
            target_texture,
            target_view,
            target_size,
            target_supports_copy_src,
            load,
            instance_offset,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_commands(
        &mut self,
        scene: &Scene,
        retained_layers: &[PreparedRetainedLayer],
        commands: &[DrawCommand],
        encoder: &mut wgpu::CommandEncoder,
        target_texture: &wgpu::Texture,
        target_view: &wgpu::TextureView,
        target_size: Size<DevicePixels>,
        target_supports_copy_src: bool,
        load: wgpu::LoadOp<wgpu::Color>,
        instance_offset: &mut u64,
    ) -> bool {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("main_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load,
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            ..Default::default()
        });

        for command in commands {
            let ok = match command {
                DrawCommand::Batch { batch, .. } => match batch {
                    PrimitiveBatch::Quads(range) => {
                        self.draw_quads(&scene.quads[range.clone()], instance_offset, &mut pass)
                    }
                    PrimitiveBatch::Shadows(range) => {
                        self.draw_shadows(&scene.shadows[range.clone()], instance_offset, &mut pass)
                    }
                    PrimitiveBatch::BackdropBlurs(range) => {
                        let blurs = &scene.backdrop_blurs[range.clone()];
                        if blurs.is_empty() || !target_supports_copy_src {
                            continue;
                        }
                        let blur_clusters = backdrop_blur_clusters(blurs, target_size);
                        if blur_clusters.is_empty() {
                            continue;
                        }

                        let mut ok = true;
                        for blurs in blur_clusters {
                            let Some(mut scratch_bounds) =
                                backdrop_scratch_bounds(&blurs, target_size)
                            else {
                                continue;
                            };
                            let Some(texture_size) = self.ensure_backdrop_texture(
                                scratch_bounds.texture_size,
                                max_backdrop_texture_size(scratch_bounds, target_size),
                            ) else {
                                continue;
                            };
                            scratch_bounds = fit_backdrop_scratch_bounds(
                                scratch_bounds,
                                texture_size,
                                target_size,
                            );
                            let Some(backdrop_texture) = self.backdrop_texture.as_ref() else {
                                continue;
                            };
                            let prepared_blurs = prepare_backdrop_blurs(&blurs, scratch_bounds);
                            let plan_groups = backdrop_blur_plan_groups(
                                &blurs,
                                self.backdrop_blur_level_sizes.len().saturating_sub(1),
                            );
                            drop(pass);
                            encoder.copy_texture_to_texture(
                                wgpu::TexelCopyTextureInfo {
                                    texture: target_texture,
                                    mip_level: 0,
                                    origin: wgpu::Origin3d {
                                        x: scratch_bounds.bounds.origin.x.0.max(0.0) as u32,
                                        y: scratch_bounds.bounds.origin.y.0.max(0.0) as u32,
                                        z: 0,
                                    },
                                    aspect: wgpu::TextureAspect::All,
                                },
                                wgpu::TexelCopyTextureInfo {
                                    texture: backdrop_texture,
                                    mip_level: 0,
                                    origin: wgpu::Origin3d::ZERO,
                                    aspect: wgpu::TextureAspect::All,
                                },
                                wgpu::Extent3d {
                                    width: scratch_bounds.texture_size.width.0 as u32,
                                    height: scratch_bounds.texture_size.height.0 as u32,
                                    depth_or_array_layers: 1,
                                },
                            );

                            for (start, end, plan) in plan_groups {
                                if !self.render_backdrop_blur_for_plan(
                                    encoder,
                                    plan,
                                    instance_offset,
                                ) {
                                    ok = false;
                                    break;
                                }
                                let blur_view = if plan.passes == 0 {
                                    self.backdrop_view.as_ref()
                                } else {
                                    self.backdrop_blur_upsample_views.first()
                                };
                                let Some(blur_view) = blur_view else {
                                    ok = false;
                                    break;
                                };
                                let mut blur_pass =
                                    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                                        label: Some("main_pass_backdrop"),
                                        color_attachments: &[Some(
                                            wgpu::RenderPassColorAttachment {
                                                view: target_view,
                                                resolve_target: None,
                                                ops: wgpu::Operations {
                                                    load: wgpu::LoadOp::Load,
                                                    store: wgpu::StoreOp::Store,
                                                },
                                                depth_slice: None,
                                            },
                                        )],
                                        depth_stencil_attachment: None,
                                        ..Default::default()
                                    });
                                if !self.draw_backdrop_blurs(
                                    &prepared_blurs[start..end],
                                    blur_view,
                                    instance_offset,
                                    &mut blur_pass,
                                ) {
                                    ok = false;
                                    break;
                                }
                            }
                            pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                                label: Some("main_pass_after_backdrop"),
                                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                    view: target_view,
                                    resolve_target: None,
                                    ops: wgpu::Operations {
                                        load: wgpu::LoadOp::Load,
                                        store: wgpu::StoreOp::Store,
                                    },
                                    depth_slice: None,
                                })],
                                depth_stencil_attachment: None,
                                ..Default::default()
                            });
                            if !ok {
                                break;
                            }
                        }
                        ok
                    }
                    PrimitiveBatch::Paths(range) => {
                        let paths = &scene.paths[range.clone()];
                        if paths.is_empty() {
                            continue;
                        }

                        let viewport_size = target_size;
                        drop(pass);

                        let path_ranges: Vec<_> =
                            (0..paths.len()).map(|index| index..index + 1).collect();
                        let mut ok = true;

                        for path_range in path_ranges {
                            let paths = &paths[path_range];
                            let Some(mut scratch_bounds) =
                                Self::path_scratch_bounds(paths, viewport_size)
                            else {
                                continue;
                            };
                            let Some(texture_size) =
                                self.ensure_intermediate_textures(scratch_bounds.texture_size)
                            else {
                                ok = false;
                                break;
                            };
                            scratch_bounds.texture_size = texture_size;

                            let did_draw = self.draw_paths_to_intermediate(
                                encoder,
                                paths,
                                scratch_bounds,
                                instance_offset,
                            );

                            pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                                label: Some("main_pass_continued"),
                                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                    view: target_view,
                                    resolve_target: None,
                                    ops: wgpu::Operations {
                                        load: wgpu::LoadOp::Load,
                                        store: wgpu::StoreOp::Store,
                                    },
                                    depth_slice: None,
                                })],
                                depth_stencil_attachment: None,
                                ..Default::default()
                            });

                            ok = did_draw
                                && self.draw_paths_from_intermediate(
                                    paths,
                                    scratch_bounds,
                                    instance_offset,
                                    &mut pass,
                                );
                            drop(pass);
                            if !ok {
                                break;
                            }
                        }

                        pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: Some("main_pass_continued"),
                            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                view: target_view,
                                resolve_target: None,
                                ops: wgpu::Operations {
                                    load: wgpu::LoadOp::Load,
                                    store: wgpu::StoreOp::Store,
                                },
                                depth_slice: None,
                            })],
                            depth_stencil_attachment: None,
                            ..Default::default()
                        });
                        ok
                    }
                    PrimitiveBatch::Underlines(range) => self.draw_underlines(
                        &scene.underlines[range.clone()],
                        instance_offset,
                        &mut pass,
                    ),
                    PrimitiveBatch::MonochromeSprites { texture_id, range } => self
                        .draw_monochrome_sprites(
                            &scene.monochrome_sprites[range.clone()],
                            *texture_id,
                            instance_offset,
                            &mut pass,
                        ),
                    PrimitiveBatch::SubpixelSprites { texture_id, range } => self
                        .draw_subpixel_sprites(
                            &scene.subpixel_sprites[range.clone()],
                            *texture_id,
                            instance_offset,
                            &mut pass,
                        ),
                    PrimitiveBatch::PolychromeSprites { texture_id, range } => self
                        .draw_polychrome_sprites(
                            &scene.polychrome_sprites[range.clone()],
                            *texture_id,
                            instance_offset,
                            &mut pass,
                        ),
                    PrimitiveBatch::Surfaces(_surfaces) => true,
                },
                DrawCommand::RetainedLayer { layer_index, .. } => {
                    let layer = &retained_layers[*layer_index].layer;
                    let cache_key = &retained_layers[*layer_index].cache_key;
                    let Some(cache) = self.retained_layers.get(cache_key) else {
                        continue;
                    };
                    self.draw_retained_layer(layer, &cache.view, instance_offset, &mut pass)
                }
            };
            if !ok {
                return false;
            }
        }

        drop(pass);
        true
    }

    fn draw_retained_layer(
        &self,
        layer: &RetainedLayer,
        texture_view: &wgpu::TextureView,
        instance_offset: &mut u64,
        pass: &mut wgpu::RenderPass<'_>,
    ) -> bool {
        let sprite = RetainedLayerSprite {
            bounds: layer.bounds,
            content_mask: layer.content_mask.clone(),
            transformation: layer.transform,
            opacity: layer.opacity.clamp(0.0, 1.0),
            pad: [0.0; 3],
        };
        let data = unsafe { Self::instance_bytes(std::slice::from_ref(&sprite)) };
        self.draw_instances_with_texture(
            data,
            1,
            texture_view,
            &self.pipelines.retained_layers,
            instance_offset,
            pass,
        )
    }

    fn batch_key(scene: &Scene, batch: &PrimitiveBatch) -> (u32, u8) {
        match batch {
            PrimitiveBatch::Shadows(range) => (scene.shadows[range.start].order, 0),
            PrimitiveBatch::BackdropBlurs(range) => (scene.backdrop_blurs[range.start].order, 1),
            PrimitiveBatch::Quads(range) => (scene.quads[range.start].order, 2),
            PrimitiveBatch::Paths(range) => (scene.paths[range.start].order, 3),
            PrimitiveBatch::Underlines(range) => (scene.underlines[range.start].order, 4),
            PrimitiveBatch::MonochromeSprites { range, .. } => {
                (scene.monochrome_sprites[range.start].order, 5)
            }
            PrimitiveBatch::SubpixelSprites { range, .. } => {
                (scene.subpixel_sprites[range.start].order, 6)
            }
            PrimitiveBatch::PolychromeSprites { range, .. } => {
                (scene.polychrome_sprites[range.start].order, 7)
            }
            PrimitiveBatch::Surfaces(range) => (scene.surfaces[range.start].order, 8),
        }
    }

    fn draw_quads(
        &self,
        quads: &[Quad],
        instance_offset: &mut u64,
        pass: &mut wgpu::RenderPass<'_>,
    ) -> bool {
        let data = unsafe { Self::instance_bytes(quads) };
        self.draw_instances(
            data,
            quads.len() as u32,
            &self.pipelines.quads,
            instance_offset,
            pass,
        )
    }

    fn draw_shadows(
        &self,
        shadows: &[Shadow],
        instance_offset: &mut u64,
        pass: &mut wgpu::RenderPass<'_>,
    ) -> bool {
        let data = unsafe { Self::instance_bytes(shadows) };
        self.draw_instances(
            data,
            shadows.len() as u32,
            &self.pipelines.shadows,
            instance_offset,
            pass,
        )
    }

    fn draw_backdrop_blurs(
        &self,
        blurs: &[BackdropBlur],
        backdrop_view: &wgpu::TextureView,
        instance_offset: &mut u64,
        pass: &mut wgpu::RenderPass<'_>,
    ) -> bool {
        let data = unsafe { Self::instance_bytes(blurs) };
        self.draw_instances_with_texture(
            data,
            blurs.len() as u32,
            backdrop_view,
            &self.pipelines.backdrop_blurs,
            instance_offset,
            pass,
        )
    }

    fn render_backdrop_blur_for_plan(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        plan: BackdropBlurPlan,
        instance_offset: &mut u64,
    ) -> bool {
        if plan.passes == 0 {
            return self.backdrop_view.is_some();
        }
        if self.backdrop_blur_downsample_views.len() < plan.passes
            || self.backdrop_blur_upsample_views.is_empty()
        {
            return false;
        }

        for level in 0..plan.passes {
            let input_view = if level == 0 {
                let Some(view) = self.backdrop_blur_input_view.as_ref() else {
                    return false;
                };
                view
            } else {
                &self.backdrop_blur_downsample_views[level - 1]
            };
            let params = BackdropBlurPassParams {
                input_size: [
                    self.backdrop_blur_level_sizes[level].width.0 as f32,
                    self.backdrop_blur_level_sizes[level].height.0 as f32,
                ],
                sample_distance: plan.sample_distance,
                pad: 0.0,
            };
            if !self.draw_backdrop_blur_pass(
                encoder,
                &params,
                input_view,
                &self.backdrop_blur_downsample_views[level],
                &self.pipelines.backdrop_blur_downsample,
                instance_offset,
            ) {
                return false;
            }
        }

        for level in (0..plan.passes).rev() {
            let input_view = if level == plan.passes - 1 {
                &self.backdrop_blur_downsample_views[level]
            } else {
                &self.backdrop_blur_upsample_views[level + 1]
            };
            let params = BackdropBlurPassParams {
                input_size: [
                    self.backdrop_blur_level_sizes[level + 1].width.0 as f32,
                    self.backdrop_blur_level_sizes[level + 1].height.0 as f32,
                ],
                sample_distance: plan.sample_distance,
                pad: 0.0,
            };
            if !self.draw_backdrop_blur_pass(
                encoder,
                &params,
                input_view,
                &self.backdrop_blur_upsample_views[level],
                &self.pipelines.backdrop_blur_upsample,
                instance_offset,
            ) {
                return false;
            }
        }

        true
    }

    fn draw_backdrop_blur_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        params: &BackdropBlurPassParams,
        input_view: &wgpu::TextureView,
        output_view: &wgpu::TextureView,
        pipeline: &wgpu::RenderPipeline,
        instance_offset: &mut u64,
    ) -> bool {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("backdrop_blur_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: output_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            ..Default::default()
        });
        let data = unsafe { Self::instance_bytes(std::slice::from_ref(params)) };
        self.draw_instances_with_texture(data, 1, input_view, pipeline, instance_offset, &mut pass)
    }

    fn draw_underlines(
        &self,
        underlines: &[Underline],
        instance_offset: &mut u64,
        pass: &mut wgpu::RenderPass<'_>,
    ) -> bool {
        let data = unsafe { Self::instance_bytes(underlines) };
        self.draw_instances(
            data,
            underlines.len() as u32,
            &self.pipelines.underlines,
            instance_offset,
            pass,
        )
    }

    fn draw_monochrome_sprites(
        &self,
        sprites: &[MonochromeSprite],
        texture_id: AtlasTextureId,
        instance_offset: &mut u64,
        pass: &mut wgpu::RenderPass<'_>,
    ) -> bool {
        let tex_info = self.atlas.get_texture_info(texture_id);
        let data = unsafe { Self::instance_bytes(sprites) };
        self.draw_instances_with_texture(
            data,
            sprites.len() as u32,
            &tex_info.view,
            &self.pipelines.mono_sprites,
            instance_offset,
            pass,
        )
    }

    fn draw_subpixel_sprites(
        &self,
        sprites: &[SubpixelSprite],
        texture_id: AtlasTextureId,
        instance_offset: &mut u64,
        pass: &mut wgpu::RenderPass<'_>,
    ) -> bool {
        let tex_info = self.atlas.get_texture_info(texture_id);
        let data = unsafe { Self::instance_bytes(sprites) };
        let pipeline = self
            .pipelines
            .subpixel_sprites
            .as_ref()
            .unwrap_or(&self.pipelines.mono_sprites);
        self.draw_instances_with_texture(
            data,
            sprites.len() as u32,
            &tex_info.view,
            pipeline,
            instance_offset,
            pass,
        )
    }

    fn draw_polychrome_sprites(
        &self,
        sprites: &[PolychromeSprite],
        texture_id: AtlasTextureId,
        instance_offset: &mut u64,
        pass: &mut wgpu::RenderPass<'_>,
    ) -> bool {
        let tex_info = self.atlas.get_texture_info(texture_id);
        let data = unsafe { Self::instance_bytes(sprites) };
        self.draw_instances_with_texture(
            data,
            sprites.len() as u32,
            &tex_info.view,
            &self.pipelines.poly_sprites,
            instance_offset,
            pass,
        )
    }

    fn draw_instances(
        &self,
        data: &[u8],
        instance_count: u32,
        pipeline: &wgpu::RenderPipeline,
        instance_offset: &mut u64,
        pass: &mut wgpu::RenderPass<'_>,
    ) -> bool {
        if instance_count == 0 {
            return true;
        }
        let Some((offset, size)) = self.write_to_instance_buffer(instance_offset, data) else {
            return false;
        };
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.bind_group_layouts.instances,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: self.instance_binding(offset, size),
            }],
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &self.globals_bind_group, &[]);
        pass.set_bind_group(1, &bind_group, &[]);
        pass.draw(0..4, 0..instance_count);
        true
    }

    fn draw_instances_with_texture(
        &self,
        data: &[u8],
        instance_count: u32,
        texture_view: &wgpu::TextureView,
        pipeline: &wgpu::RenderPipeline,
        instance_offset: &mut u64,
        pass: &mut wgpu::RenderPass<'_>,
    ) -> bool {
        if instance_count == 0 {
            return true;
        }
        let Some((offset, size)) = self.write_to_instance_buffer(instance_offset, data) else {
            return false;
        };
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.bind_group_layouts.instances_with_texture,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.instance_binding(offset, size),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.atlas_sampler),
                },
            ],
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &self.globals_bind_group, &[]);
        pass.set_bind_group(1, &bind_group, &[]);
        pass.draw(0..4, 0..instance_count);
        true
    }

    unsafe fn instance_bytes<T>(instances: &[T]) -> &[u8] {
        unsafe {
            std::slice::from_raw_parts(
                instances.as_ptr() as *const u8,
                std::mem::size_of_val(instances),
            )
        }
    }

    fn path_scratch_bounds(
        paths: &[Path<ScaledPixels>],
        viewport_size: Size<DevicePixels>,
    ) -> Option<PathScratchBounds> {
        let mut bounds = paths.first()?.clipped_bounds();
        for path in paths.iter().skip(1) {
            bounds = bounds.union(&path.clipped_bounds());
        }

        let viewport_bounds = Bounds {
            origin: Point {
                x: ScaledPixels(0.0),
                y: ScaledPixels(0.0),
            },
            size: Size {
                width: ScaledPixels::from(viewport_size.width),
                height: ScaledPixels::from(viewport_size.height),
            },
        };
        bounds = bounds.dilate(ScaledPixels(1.0)).intersect(&viewport_bounds);
        if bounds.is_empty() {
            return None;
        }

        let origin = bounds.origin.map(|component| component.floor());
        let bottom_right = bounds.bottom_right().map(|component| component.ceil());
        let bounds = Bounds::from_corners(origin, bottom_right);
        Some(PathScratchBounds {
            texture_size: Size {
                width: DevicePixels::from(bounds.size.width),
                height: DevicePixels::from(bounds.size.height),
            },
            bounds,
        })
    }

    fn draw_paths_from_intermediate(
        &self,
        paths: &[Path<ScaledPixels>],
        scratch_bounds: PathScratchBounds,
        instance_offset: &mut u64,
        pass: &mut wgpu::RenderPass<'_>,
    ) -> bool {
        let first_path = &paths[0];
        let sprites: Vec<PathSprite> = if paths.last().map(|p| &p.order) == Some(&first_path.order)
        {
            paths
                .iter()
                .map(|p| PathSprite {
                    bounds: p.clipped_bounds(),
                    scratch_bounds: scratch_bounds.bounds,
                    texture_size: [
                        scratch_bounds.texture_size.width.0 as f32,
                        scratch_bounds.texture_size.height.0 as f32,
                    ],
                })
                .collect()
        } else {
            let mut bounds = first_path.clipped_bounds();
            for path in paths.iter().skip(1) {
                bounds = bounds.union(&path.clipped_bounds());
            }
            vec![PathSprite {
                bounds,
                scratch_bounds: scratch_bounds.bounds,
                texture_size: [
                    scratch_bounds.texture_size.width.0 as f32,
                    scratch_bounds.texture_size.height.0 as f32,
                ],
            }]
        };

        let Some(path_intermediate_view) = self.path_intermediate_view.as_ref() else {
            return true;
        };

        let sprite_data = unsafe { Self::instance_bytes(&sprites) };
        self.draw_instances_with_texture(
            sprite_data,
            sprites.len() as u32,
            path_intermediate_view,
            &self.pipelines.paths,
            instance_offset,
            pass,
        )
    }

    fn draw_paths_to_intermediate(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        paths: &[Path<ScaledPixels>],
        scratch_bounds: PathScratchBounds,
        instance_offset: &mut u64,
    ) -> bool {
        let mut vertices = Vec::new();
        for path in paths {
            let bounds = path.clipped_bounds();
            vertices.extend(path.vertices.iter().map(|v| PathRasterizationVertex {
                xy_position: v.xy_position,
                st_position: v.st_position,
                color: path.color,
                bounds,
                content_mask: path.content_mask.clone(),
                scratch_bounds: scratch_bounds.bounds,
                texture_size: [
                    scratch_bounds.texture_size.width.0 as f32,
                    scratch_bounds.texture_size.height.0 as f32,
                ],
            }));
        }

        if vertices.is_empty() {
            return true;
        }

        let vertex_data = unsafe { Self::instance_bytes(&vertices) };
        let Some((vertex_offset, vertex_size)) =
            self.write_to_instance_buffer(instance_offset, vertex_data)
        else {
            return false;
        };

        let data_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("path_rasterization_bind_group"),
            layout: &self.bind_group_layouts.instances,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: self.instance_binding(vertex_offset, vertex_size),
            }],
        });

        let Some(path_intermediate_view) = self.path_intermediate_view.as_ref() else {
            return true;
        };

        let (target_view, resolve_target) = if let Some(ref msaa_view) = self.path_msaa_view {
            (msaa_view, Some(path_intermediate_view))
        } else {
            (path_intermediate_view, None)
        };

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("path_rasterization_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target_view,
                    resolve_target,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                ..Default::default()
            });

            pass.set_pipeline(&self.pipelines.path_rasterization);
            pass.set_bind_group(0, &self.path_globals_bind_group, &[]);
            pass.set_bind_group(1, &data_bind_group, &[]);
            pass.draw(0..vertices.len() as u32, 0..1);
        }

        true
    }

    fn required_instance_buffer_size(&self, scene: &Scene) -> u64 {
        let mut size = 0;
        for batch in scene.batches() {
            match batch {
                PrimitiveBatch::Quads(range) => {
                    self.add_instance_bytes::<Quad>(&mut size, range.len());
                }
                PrimitiveBatch::Shadows(range) => {
                    self.add_instance_bytes::<Shadow>(&mut size, range.len());
                }
                PrimitiveBatch::BackdropBlurs(range) => {
                    self.add_instance_bytes::<BackdropBlur>(&mut size, range.len());
                    for _ in 0..range.len() * MAX_BACKDROP_BLUR_LEVELS * 2 {
                        self.add_instance_bytes::<BackdropBlurPassParams>(&mut size, 1);
                    }
                }
                PrimitiveBatch::Paths(range) => {
                    for path in &scene.paths[range] {
                        size = size.next_multiple_of(self.storage_buffer_alignment);
                        size += path.vertices.len() as u64
                            * std::mem::size_of::<PathRasterizationVertex>() as u64;
                        self.add_instance_bytes::<PathSprite>(&mut size, 1);
                    }
                }
                PrimitiveBatch::Underlines(range) => {
                    self.add_instance_bytes::<Underline>(&mut size, range.len());
                }
                PrimitiveBatch::MonochromeSprites { range, .. } => {
                    self.add_instance_bytes::<MonochromeSprite>(&mut size, range.len());
                }
                PrimitiveBatch::SubpixelSprites { range, .. } => {
                    self.add_instance_bytes::<SubpixelSprite>(&mut size, range.len());
                }
                PrimitiveBatch::PolychromeSprites { range, .. } => {
                    self.add_instance_bytes::<PolychromeSprite>(&mut size, range.len());
                }
                PrimitiveBatch::Surfaces(_) => {}
            }
        }
        size
    }

    fn add_instance_bytes<T>(&self, offset: &mut u64, count: usize) {
        if count == 0 {
            return;
        }
        *offset = (*offset).next_multiple_of(self.storage_buffer_alignment);
        *offset += (std::mem::size_of::<T>() as u64 * count as u64).max(16);
    }

    fn ensure_instance_buffer_capacity(&mut self, required_capacity: u64) {
        let max_capacity = MAX_INSTANCE_BUFFER_SIZE.min(self.device.limits().max_buffer_size);
        let mut new_capacity = required_capacity
            .next_multiple_of(INSTANCE_BUFFER_SIZE_BUCKET)
            .max(INITIAL_INSTANCE_BUFFER_SIZE);
        if new_capacity > max_capacity {
            log::error!(
                "required instance buffer size {} exceeds maximum {}; dropping frame may occur",
                new_capacity,
                max_capacity
            );
            new_capacity = max_capacity;
        }
        if new_capacity <= self.instance_buffer_capacity {
            return;
        }
        log::info!(
            "increased instance buffer size from {} to {}",
            self.instance_buffer_capacity,
            new_capacity
        );
        self.instance_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("instance_buffer"),
            size: new_capacity,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.instance_buffer_capacity = new_capacity;
    }

    fn write_to_instance_buffer(
        &self,
        instance_offset: &mut u64,
        data: &[u8],
    ) -> Option<(u64, NonZeroU64)> {
        let offset = (*instance_offset).next_multiple_of(self.storage_buffer_alignment);
        let size = (data.len() as u64).max(16);
        if offset + size > self.instance_buffer_capacity {
            return None;
        }
        self.queue.write_buffer(&self.instance_buffer, offset, data);
        *instance_offset = offset + size;
        Some((offset, NonZeroU64::new(size).expect("size is at least 16")))
    }

    fn instance_binding(&self, offset: u64, size: NonZeroU64) -> wgpu::BindingResource<'_> {
        wgpu::BindingResource::Buffer(wgpu::BufferBinding {
            buffer: &self.instance_buffer,
            offset,
            size: Some(size),
        })
    }

    pub fn destroy(&mut self) {
        // Release surface-bound GPU resources eagerly so the underlying native
        // window can be destroyed before the renderer itself is dropped.
        self.resources.take();
    }

    /// Returns true if the GPU device was lost and recovery is needed.
    pub fn device_lost(&self) -> bool {
        self.device_lost.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Returns and clears the flag indicating that cached GPU state was discarded.
    pub fn needs_redraw(&mut self) -> bool {
        std::mem::take(&mut self.needs_redraw)
    }

    /// Recreates this window's device-owned resource graph after device loss.
    /// The first window recreates the shared context; later windows adopt it.
    #[cfg(not(target_family = "wasm"))]
    pub fn recover(&mut self) -> anyhow::Result<()> {
        // The current scene was built against atlas IDs from the lost device.
        // Always request a fresh scene, including when this recovery attempt fails.
        self.needs_redraw = true;
        let gpu_context = Rc::clone(
            self.context
                .as_ref()
                .expect("native renderer recovery requires a shared context"),
        );
        let handles = self
            .surface_handles
            .expect("native renderer recovery requires surface handles");
        let needs_new_context = recovery_requires_new_context(
            gpu_context.borrow().as_ref().map(WgpuContext::device_lost),
        );

        // Drop every Arc/device child and the old surface before replacing the context.
        self.resources.take();
        let surface = if needs_new_context {
            *gpu_context.borrow_mut() = None;
            std::thread::sleep(std::time::Duration::from_millis(350));
            let instance = WgpuContext::instance();
            let surface = unsafe {
                instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                    raw_display_handle: Some(handles.display),
                    raw_window_handle: handles.window,
                })?
            };
            let context = WgpuContext::new_for_surface_recovery(
                instance,
                &surface,
                self.compositor_gpu,
                self.renderer_selection,
            )?;
            *gpu_context.borrow_mut() = Some(context);
            surface
        } else {
            let context_ref = gpu_context.borrow();
            let context = context_ref.as_ref().expect("context was recovered");
            unsafe {
                context
                    .instance
                    .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                        raw_display_handle: Some(handles.display),
                        raw_window_handle: handles.window,
                    })?
            }
        };
        let context_ref = gpu_context.borrow();
        let context = context_ref.as_ref().expect("context was recovered");
        let config = self.desired.renderer_config();
        let atlas = Arc::clone(&self.atlas);
        let is_bgr = self.desired.is_bgr;
        atlas.handle_device_lost(Arc::clone(&context.device), Arc::clone(&context.queue));

        let mut recovered = Self::from_surface_internal(
            context,
            surface,
            config,
            Some(Rc::clone(&gpu_context)),
            self.compositor_gpu,
            Some(atlas),
            Some(handles),
        )?;
        recovered.desired.is_bgr = is_bgr;
        self.frame_failure_streak
            .transfer_to(&mut recovered.frame_failure_streak);
        recovered.needs_redraw = true;
        recovered.first_presentation_observer = self.first_presentation_observer.take();
        *self = recovered;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::Corners;

    fn scene_with_retained_primitive(
        primitive: impl Into<gpui::Primitive>,
    ) -> (Scene, RetainedLayer) {
        let bounds = Bounds::new(
            Point::new(ScaledPixels(0.0), ScaledPixels(0.0)),
            Size::new(ScaledPixels(100.0), ScaledPixels(100.0)),
        );
        let mut scene = Scene::default();
        scene.insert_primitive(primitive);
        let layer = RetainedLayer {
            id: GlobalElementId::default(),
            content_revision: 1.into(),
            content_dirty: true,
            bounds,
            content_mask: ContentMask::new(bounds),
            transform: TransformationMatrix::unit(),
            opacity: 1.0,
            paint_range: 0..scene.paint_operation_count(),
        };
        (scene, layer)
    }

    #[test]
    fn backdrop_blur_pass_params_match_wgsl_storage_layout() {
        assert_eq!(std::mem::size_of::<BackdropBlurPassParams>(), 16);
        assert_eq!(std::mem::align_of::<BackdropBlurPassParams>(), 4);
    }

    #[test]
    fn backdrop_blur_intermediates_use_linear_formats() {
        assert_eq!(
            backdrop_blur_format(wgpu::TextureFormat::Bgra8UnormSrgb),
            wgpu::TextureFormat::Bgra8Unorm
        );
        assert_eq!(
            backdrop_blur_format(wgpu::TextureFormat::Rgba8UnormSrgb),
            wgpu::TextureFormat::Rgba8Unorm
        );
        assert_eq!(
            backdrop_blur_format(wgpu::TextureFormat::Bgra8Unorm),
            wgpu::TextureFormat::Bgra8Unorm
        );
        assert_eq!(
            backdrop_blur_format(wgpu::TextureFormat::Rgba8Unorm),
            wgpu::TextureFormat::Rgba8Unorm
        );
        assert_eq!(
            backdrop_blur_format(wgpu::TextureFormat::Rgba16Float),
            wgpu::TextureFormat::Rgba16Float
        );
    }

    #[test]
    fn shaders_parse_and_validate() {
        let module =
            wgpu::naga::front::wgsl::parse_str(include_str!("shaders.wgsl")).expect("parse WGSL");
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("validate WGSL");
    }

    #[test]
    fn backdrop_blur_shader_uses_fixed_downsample_and_weighted_tent_upsample() {
        let source = include_str!("shaders.wgsl");
        assert!(source.contains("let offset = texel * 0.75;"));
        assert!(source.contains("let offsets = array<vec2<f32>, 8>("));
        assert!(source.contains("let weights = array<f32, 8>("));
        assert!(source.contains("1.0 / 12.0,"));
        assert!(source.contains("1.0 / 6.0,"));
    }

    #[test]
    fn select_present_mode_accepts_auto_modes_without_surface_advertising_them() {
        assert_eq!(
            select_present_mode(
                Some(wgpu::PresentMode::AutoVsync),
                &[wgpu::PresentMode::Fifo]
            ),
            wgpu::PresentMode::AutoVsync
        );
        assert_eq!(
            select_present_mode(
                Some(wgpu::PresentMode::AutoNoVsync),
                &[wgpu::PresentMode::Fifo]
            ),
            wgpu::PresentMode::AutoNoVsync
        );
    }

    #[test]
    fn select_present_mode_falls_back_to_fifo_for_unsupported_explicit_mode() {
        assert_eq!(
            select_present_mode(Some(wgpu::PresentMode::Mailbox), &[wgpu::PresentMode::Fifo]),
            wgpu::PresentMode::Fifo
        );
    }

    #[test]
    fn recovery_state_preserves_requested_surface_and_subpixel_settings() {
        let desired = DesiredSurfaceState {
            size: Size::new(DevicePixels(1440), DevicePixels(900)),
            transparent: true,
            preferred_present_mode: Some(wgpu::PresentMode::Mailbox),
            is_bgr: true,
        };

        let config = desired.renderer_config();
        assert_eq!(config.size, desired.size);
        assert_eq!(config.transparent, desired.transparent);
        assert_eq!(
            config.preferred_present_mode,
            desired.preferred_present_mode
        );
        assert!(desired.is_bgr);
    }

    #[test]
    fn recovery_coordinator_recreates_only_missing_or_lost_shared_contexts() {
        assert!(recovery_requires_new_context(None));
        assert!(recovery_requires_new_context(Some(true)));
        assert!(!recovery_requires_new_context(Some(false)));
    }

    #[test]
    fn recovery_gap_does_not_reset_consecutive_frame_failures() {
        let mut failures = FrameFailureStreak::default();

        for expected in 1..=6 {
            assert_eq!(failures.record_error(), expected);
            if expected > 5 {
                failures.begin_recovery();
            }
        }

        let mut recovered_failures = FrameFailureStreak::default();
        failures.transfer_to(&mut recovered_failures);
        assert_eq!(failures.count, 0);
        recovered_failures.record_no_error();
        assert_eq!(recovered_failures.count, 6);

        for expected in 7..=21 {
            assert_eq!(recovered_failures.record_error(), expected);
            recovered_failures.begin_recovery();
            recovered_failures.record_no_error();
        }

        assert_eq!(recovered_failures.count, 21);
        recovered_failures.record_no_error();
        assert_eq!(recovered_failures.count, 0);
    }

    #[test]
    fn surface_usage_requests_copy_source_only_when_supported() {
        let (usage, supports_copy_src) =
            surface_usage(wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC);
        assert!(supports_copy_src);
        assert!(usage.contains(wgpu::TextureUsages::COPY_SRC));

        let (usage, supports_copy_src) = surface_usage(wgpu::TextureUsages::RENDER_ATTACHMENT);
        assert!(!supports_copy_src);
        assert!(!usage.contains(wgpu::TextureUsages::COPY_SRC));
        assert!(usage.contains(wgpu::TextureUsages::RENDER_ATTACHMENT));
    }

    #[test]
    fn retained_layer_with_backdrop_blur_is_excluded_from_wgpu_cache() {
        let bounds = Bounds::new(
            Point::new(ScaledPixels(0.0), ScaledPixels(0.0)),
            Size::new(ScaledPixels(100.0), ScaledPixels(100.0)),
        );
        let (scene, layer) = scene_with_retained_primitive(BackdropBlur {
            order: 0,
            pad: 0,
            bounds,
            content_mask: ContentMask::new(bounds),
            corner_radii: Corners::all(ScaledPixels(8.0)),
            blur_radius: ScaledPixels(16.0),
            source_origin_x: 0.0,
            source_origin_y: 0.0,
            source_width: 1.0,
            source_height: 1.0,
            opacity: 1.0,
        });

        let retained_scene = scene.clone_paint_range(layer.paint_range);
        assert!(WgpuRenderer::retained_scene_contains_backdrop_blurs(
            &retained_scene
        ));
    }

    #[test]
    fn retained_layer_without_backdrop_blur_remains_cacheable_by_wgpu() {
        let bounds = Bounds::new(
            Point::new(ScaledPixels(0.0), ScaledPixels(0.0)),
            Size::new(ScaledPixels(100.0), ScaledPixels(100.0)),
        );
        let (scene, layer) = scene_with_retained_primitive(Quad {
            order: 0,
            bounds,
            content_mask: ContentMask::new(bounds),
            ..Default::default()
        });

        let retained_scene = scene.clone_paint_range(layer.paint_range);
        assert!(!WgpuRenderer::retained_scene_contains_backdrop_blurs(
            &retained_scene
        ));
    }
}

struct RenderingParameters {
    path_sample_count: u32,
    gamma_ratios: [f32; 4],
    grayscale_enhanced_contrast: f32,
    subpixel_enhanced_contrast: f32,
}

impl RenderingParameters {
    fn new(adapter: &wgpu::Adapter, surface_format: wgpu::TextureFormat) -> Self {
        use std::env;

        let format_features = adapter.get_texture_format_features(surface_format);
        let path_sample_count = [4, 2, 1]
            .into_iter()
            .find(|&n| format_features.flags.sample_count_supported(n))
            .unwrap_or(1);

        let gamma = env::var("ZED_FONTS_GAMMA")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1.8_f32)
            .clamp(1.0, 2.2);
        let gamma_ratios = get_gamma_correction_ratios(gamma);

        let grayscale_enhanced_contrast = env::var("ZED_FONTS_GRAYSCALE_ENHANCED_CONTRAST")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1.0_f32)
            .max(0.0);

        let subpixel_enhanced_contrast = env::var("ZED_FONTS_SUBPIXEL_ENHANCED_CONTRAST")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.5_f32)
            .max(0.0);

        Self {
            path_sample_count,
            gamma_ratios,
            grayscale_enhanced_contrast,
            subpixel_enhanced_contrast,
        }
    }
}
