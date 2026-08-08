use crate::metal_atlas::MetalAtlas;
use anyhow::Result;
use block::ConcreteBlock;
use cocoa::{
    base::{NO, YES, id, nil},
    foundation::{NSAutoreleasePool, NSSize, NSUInteger},
    quartzcore::AutoresizingMask,
};
use gpui::{
    AtlasTextureId, BackdropBlur, BackdropBlurPlan, BackdropScratchBounds, Background, Bounds,
    ContentMask, DevicePixels, FirstPresentationObserver, GlobalElementId, MonochromeSprite,
    PaintSurface, Path, Point, PolychromeSprite, PresentationEvidence, PrimitiveBatch, Quad,
    RendererAdapterType, RendererInfo, RendererKind, RendererSelection, RetainedLayer,
    RetainedLayerContentRevision, ScaledPixels, Scene, Shadow, Size, Surface, TransformationMatrix,
    Underline, backdrop_blur_clusters, backdrop_blur_level_sizes_for, backdrop_blur_plan_groups,
    backdrop_scratch_bounds, can_reuse_backdrop_texture, fit_backdrop_scratch_bounds,
    max_backdrop_texture_size, point, prepare_backdrop_blurs, size,
};
#[cfg(any(test, feature = "test-support"))]
use image::RgbaImage;

use core_foundation::base::TCFType;
use core_video::{
    metal_texture::CVMetalTextureGetTexture, metal_texture_cache::CVMetalTextureCache,
    pixel_buffer::kCVPixelFormatType_420YpCbCr8BiPlanarFullRange,
};
use foreign_types::{ForeignType, ForeignTypeRef};
use metal::{
    CAMetalLayer, CommandQueue, MTLGPUFamily, MTLPixelFormat, MTLResourceOptions, NSRange,
    RenderPassColorAttachmentDescriptorRef,
};
use objc::{self, msg_send, sel, sel_impl};
use parking_lot::Mutex;

use std::{
    cell::Cell,
    collections::{HashMap, HashSet},
    ffi::c_void,
    mem, ptr,
    sync::Arc,
};

// Exported to metal
pub(crate) type PointF = gpui::Point<f32>;

#[cfg(not(feature = "runtime_shaders"))]
const SHADERS_METALLIB: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/shaders.metallib"));
#[cfg(feature = "runtime_shaders")]
const SHADERS_SOURCE_FILE: &str = include_str!(concat!(env!("OUT_DIR"), "/stitched_shaders.metal"));
// Use 4x MSAA, all devices support it.
// https://developer.apple.com/documentation/metal/mtldevice/1433355-supportstexturesamplecount
const PATH_SAMPLE_COUNT: u32 = 4;
const BACKDROP_TEXTURE_SIZE_QUANTUM: i32 = 64;

use crate::dispatch_semaphore::FrameSemaphore;

/// Maximum number of frames the CPU can submit ahead of the GPU.
/// Matches the drawable count so the CPU blocks on the semaphore instead of
/// queuing unbounded command buffers that pin GPU memory as non-purgeable.
/// See Apple Metal Best Practices for CPU-GPU synchronization.
const MAX_FRAMES_IN_FLIGHT: i64 = 2;

pub(crate) type Context = Arc<Mutex<InstanceBufferPool>>;
pub(crate) type Renderer = MetalRenderer;

struct ScopedAutoreleasePool(id);

impl ScopedAutoreleasePool {
    fn new() -> Self {
        Self(unsafe { NSAutoreleasePool::new(nil) })
    }
}

impl Drop for ScopedAutoreleasePool {
    fn drop(&mut self) {
        unsafe {
            self.0.drain();
        }
    }
}

pub(crate) fn new_renderer(context: self::Context, transparent: bool) -> Result<Renderer> {
    MetalRenderer::new(context, transparent)
}

pub(crate) struct InstanceBufferPool {
    buffer_size: usize,
    buffers: Vec<metal::Buffer>,
}

const INITIAL_INSTANCE_BUFFER_SIZE: usize = 2 * 1024 * 1024;
const INSTANCE_BUFFER_SIZE_BUCKET: usize = 1024 * 1024;
const MAX_INSTANCE_BUFFER_SIZE: usize = 256 * 1024 * 1024;

impl Default for InstanceBufferPool {
    fn default() -> Self {
        Self {
            buffer_size: INITIAL_INSTANCE_BUFFER_SIZE,
            buffers: Vec::new(),
        }
    }
}

pub(crate) struct InstanceBuffer {
    metal_buffer: metal::Buffer,
    size: usize,
    managed: bool,
}

fn dynamic_buffer_options(device: &metal::Device) -> (MTLResourceOptions, bool) {
    let managed = !device.has_unified_memory();
    let storage_mode = if managed {
        MTLResourceOptions::StorageModeManaged
    } else {
        MTLResourceOptions::StorageModeShared
    };

    (
        MTLResourceOptions::CPUCacheModeWriteCombined | storage_mode,
        managed,
    )
}

impl InstanceBufferPool {
    fn ensure_size(&mut self, required_size: usize) {
        let mut required_size = required_size
            .next_multiple_of(INSTANCE_BUFFER_SIZE_BUCKET)
            .max(INITIAL_INSTANCE_BUFFER_SIZE);
        if required_size > MAX_INSTANCE_BUFFER_SIZE {
            log::error!(
                "required instance buffer size {} exceeds maximum {}; dropping frame may occur",
                required_size,
                MAX_INSTANCE_BUFFER_SIZE
            );
            required_size = MAX_INSTANCE_BUFFER_SIZE;
        }
        if required_size > self.buffer_size {
            log::info!(
                "increased instance buffer size from {} to {}",
                self.buffer_size,
                required_size
            );
            self.buffer_size = required_size;
            self.buffers.clear();
        }
    }

    pub(crate) fn acquire(
        &mut self,
        device: &metal::Device,
        required_size: usize,
    ) -> InstanceBuffer {
        self.ensure_size(required_size);
        let (options, managed) = dynamic_buffer_options(device);
        let buffer = self
            .buffers
            .pop()
            .unwrap_or_else(|| device.new_buffer(self.buffer_size as u64, options));
        InstanceBuffer {
            metal_buffer: buffer,
            size: self.buffer_size,
            managed,
        }
    }

    pub(crate) fn release(&mut self, buffer: InstanceBuffer) {
        if buffer.size == self.buffer_size {
            self.buffers.push(buffer.metal_buffer)
        }
    }
}

pub(crate) struct MetalRenderer {
    device: metal::Device,
    layer: metal::MetalLayer,
    presents_with_transaction: bool,
    command_queue: CommandQueue,
    paths_rasterization_pipeline_state: metal::RenderPipelineState,
    path_sprites_pipeline_state: metal::RenderPipelineState,
    texture_copy_pipeline_state: metal::RenderPipelineState,
    shadows_pipeline_state: metal::RenderPipelineState,
    backdrop_blur_pipeline_state: metal::RenderPipelineState,
    backdrop_blur_downsample_pipeline_state: metal::RenderPipelineState,
    backdrop_blur_upsample_pipeline_state: metal::RenderPipelineState,
    quads_pipeline_state: metal::RenderPipelineState,
    underlines_pipeline_state: metal::RenderPipelineState,
    monochrome_sprites_pipeline_state: metal::RenderPipelineState,
    polychrome_sprites_pipeline_state: metal::RenderPipelineState,
    surfaces_pipeline_state: metal::RenderPipelineState,
    retained_layer_pipeline_state: metal::RenderPipelineState,
    unit_vertices: metal::Buffer,
    #[allow(clippy::arc_with_non_send_sync)]
    instance_buffer_pool: Arc<Mutex<InstanceBufferPool>>,
    sprite_atlas: Arc<MetalAtlas>,
    core_video_texture_cache: core_video::metal_texture_cache::CVMetalTextureCache,
    path_intermediate_texture: Option<metal::Texture>,
    path_intermediate_msaa_texture: Option<metal::Texture>,
    path_intermediate_size: Option<Size<DevicePixels>>,
    frame_texture: Option<metal::Texture>,
    frame_texture_size: Option<Size<DevicePixels>>,
    backdrop_texture_size: Option<Size<DevicePixels>>,
    backdrop_source_texture: Option<metal::Texture>,
    backdrop_blur_level_sizes: Vec<Size<DevicePixels>>,
    backdrop_blur_downsample_textures: Vec<metal::Texture>,
    backdrop_blur_upsample_textures: Vec<metal::Texture>,
    retained_layers: HashMap<RetainedLayerCacheKey, CachedRetainedLayer>,
    path_sample_count: u32,
    is_apple_gpu: bool,
    frame_semaphore: FrameSemaphore,
    first_presentation_observer: Option<FirstPresentationObserver>,
}

#[repr(C)]
pub struct PathRasterizationVertex {
    pub xy_position: Point<ScaledPixels>,
    pub st_position: Point<f32>,
    pub color: Background,
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub scratch_bounds: Bounds<ScaledPixels>,
    pub texture_size: Size<DevicePixels>,
}

#[derive(Clone, Copy)]
struct PathScratchBounds {
    bounds: Bounds<ScaledPixels>,
    texture_size: Size<DevicePixels>,
}

#[repr(C)]
struct BackdropBlurParams {
    input_origin: [f32; 2],
    input_size: Size<DevicePixels>,
    texture_size: Size<DevicePixels>,
    sample_distance: f32,
    pad: f32,
}

#[repr(C)]
struct TextureCopyParams {
    source_origin: [f32; 2],
    destination_size: [f32; 2],
}

struct CachedRetainedLayer {
    content_revision: RetainedLayerContentRevision,
    texture_size: Size<DevicePixels>,
    texture: metal::Texture,
}

#[derive(Clone, Hash, PartialEq, Eq)]
struct RetainedLayerCacheKey {
    id: GlobalElementId,
    occurrence: usize,
}

#[derive(Clone, Debug, PartialEq)]
#[repr(C)]
pub struct RetainedLayerSprite {
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub transform: TransformationMatrix,
    pub opacity: f32,
    pub pad: [f32; 3],
}

fn first_presentation_is_scheduled(status: metal::MTLCommandBufferStatus) -> bool {
    matches!(
        status,
        metal::MTLCommandBufferStatus::Scheduled | metal::MTLCommandBufferStatus::Completed
    )
}

fn record_first_presentation_status(
    observer: &FirstPresentationObserver,
    status: metal::MTLCommandBufferStatus,
) {
    if first_presentation_is_scheduled(status) {
        observer.record_presentation(PresentationEvidence::BackendAccepted);
    } else if status == metal::MTLCommandBufferStatus::Error {
        log::error!("first Metal presentation command buffer entered Error");
    }
}

fn presentation_requires_scheduling_wait(presents_with_transaction: bool) -> bool {
    presents_with_transaction
}

impl MetalRenderer {
    pub fn new(
        instance_buffer_pool: Arc<Mutex<InstanceBufferPool>>,
        transparent: bool,
    ) -> Result<Self> {
        // Prefer low‐power integrated GPUs on Intel Mac. On Apple
        // Silicon, there is only ever one GPU, so this is equivalent to
        // `metal::Device::system_default()`.
        let device = if let Some(d) = metal::Device::all()
            .into_iter()
            .min_by_key(|d| (d.is_removable(), !d.is_low_power()))
        {
            d
        } else {
            // For some reason `all()` can return an empty list, see https://github.com/zed-industries/zed/issues/37689.
            // In that case, fall back to the system default device.
            log::error!(
                "Unable to enumerate Metal devices; attempting to use system default device"
            );
            metal::Device::system_default()
                .ok_or_else(|| anyhow::anyhow!("unable to access a compatible graphics device"))?
        };

        let layer = metal::MetalLayer::new();
        layer.set_device(&device);
        layer.set_pixel_format(MTLPixelFormat::BGRA8Unorm);
        // Support direct-to-display rendering if the window is not transparent
        // https://developer.apple.com/documentation/metal/managing-your-game-window-for-metal-in-macos
        layer.set_opaque(!transparent);
        layer.set_maximum_drawable_count(2);
        // Visual tests read pixels back from the drawable.
        #[cfg(any(test, feature = "test-support"))]
        layer.set_framebuffer_only(false);
        #[cfg(not(any(test, feature = "test-support")))]
        layer.set_framebuffer_only(true);
        unsafe {
            let _: () = msg_send![&*layer, setAllowsNextDrawableTimeout: NO];
            let _: () = msg_send![&*layer, setNeedsDisplayOnBoundsChange: YES];
            let _: () = msg_send![
                &*layer,
                setAutoresizingMask: AutoresizingMask::WIDTH_SIZABLE
                    | AutoresizingMask::HEIGHT_SIZABLE
            ];
        }
        #[cfg(feature = "runtime_shaders")]
        let library = device
            .new_library_with_source(&SHADERS_SOURCE_FILE, &metal::CompileOptions::new())
            .expect("error building metal library");
        #[cfg(not(feature = "runtime_shaders"))]
        let library = device
            .new_library_with_data(SHADERS_METALLIB)
            .expect("error building metal library");

        fn to_float2_bits(point: PointF) -> u64 {
            let mut output = point.y.to_bits() as u64;
            output <<= 32;
            output |= point.x.to_bits() as u64;
            output
        }

        let unit_vertices = [
            to_float2_bits(point(0., 0.)),
            to_float2_bits(point(1., 0.)),
            to_float2_bits(point(0., 1.)),
            to_float2_bits(point(0., 1.)),
            to_float2_bits(point(1., 0.)),
            to_float2_bits(point(1., 1.)),
        ];
        let unit_vertices = device.new_buffer_with_data(
            unit_vertices.as_ptr() as *const c_void,
            mem::size_of_val(&unit_vertices) as u64,
            dynamic_buffer_options(&device).0,
        );

        let paths_rasterization_pipeline_state = build_path_rasterization_pipeline_state(
            &device,
            &library,
            "paths_rasterization",
            "path_rasterization_vertex",
            "path_rasterization_fragment",
            MTLPixelFormat::BGRA8Unorm,
            PATH_SAMPLE_COUNT,
        );
        let path_sprites_pipeline_state = build_path_sprite_pipeline_state(
            &device,
            &library,
            "path_sprites",
            "path_sprite_vertex",
            "path_sprite_fragment",
            MTLPixelFormat::BGRA8Unorm,
        );
        let texture_copy_pipeline_state = build_texture_copy_pipeline_state(
            &device,
            &library,
            "texture_copy",
            "texture_copy_vertex",
            "texture_copy_fragment",
            MTLPixelFormat::BGRA8Unorm,
        );
        let shadows_pipeline_state = build_pipeline_state(
            &device,
            &library,
            "shadows",
            "shadow_vertex",
            "shadow_fragment",
            MTLPixelFormat::BGRA8Unorm,
        );
        let backdrop_blur_pipeline_state = build_pipeline_state(
            &device,
            &library,
            "backdrop_blur",
            "backdrop_blur_vertex",
            "backdrop_blur_fragment",
            MTLPixelFormat::BGRA8Unorm,
        );
        let backdrop_blur_downsample_pipeline_state = build_no_blend_pipeline_state(
            &device,
            &library,
            "backdrop_blur_downsample",
            "backdrop_blur_downsample_vertex",
            "backdrop_blur_downsample_fragment",
            MTLPixelFormat::BGRA8Unorm,
        );
        let backdrop_blur_upsample_pipeline_state = build_no_blend_pipeline_state(
            &device,
            &library,
            "backdrop_blur_upsample",
            "backdrop_blur_upsample_vertex",
            "backdrop_blur_upsample_fragment",
            MTLPixelFormat::BGRA8Unorm,
        );
        let quads_pipeline_state = build_pipeline_state(
            &device,
            &library,
            "quads",
            "quad_vertex",
            "quad_fragment",
            MTLPixelFormat::BGRA8Unorm,
        );
        let underlines_pipeline_state = build_pipeline_state(
            &device,
            &library,
            "underlines",
            "underline_vertex",
            "underline_fragment",
            MTLPixelFormat::BGRA8Unorm,
        );
        let monochrome_sprites_pipeline_state = build_pipeline_state(
            &device,
            &library,
            "monochrome_sprites",
            "monochrome_sprite_vertex",
            "monochrome_sprite_fragment",
            MTLPixelFormat::BGRA8Unorm,
        );
        let polychrome_sprites_pipeline_state = build_pipeline_state(
            &device,
            &library,
            "polychrome_sprites",
            "polychrome_sprite_vertex",
            "polychrome_sprite_fragment",
            MTLPixelFormat::BGRA8Unorm,
        );
        let surfaces_pipeline_state = build_pipeline_state(
            &device,
            &library,
            "surfaces",
            "surface_vertex",
            "surface_fragment",
            MTLPixelFormat::BGRA8Unorm,
        );
        let retained_layer_pipeline_state = build_path_sprite_pipeline_state(
            &device,
            &library,
            "retained_layers",
            "retained_layer_vertex",
            "retained_layer_fragment",
            MTLPixelFormat::BGRA8Unorm,
        );

        let command_queue = device.new_command_queue();
        let sprite_atlas = Arc::new(MetalAtlas::new(device.clone()));
        let core_video_texture_cache =
            CVMetalTextureCache::new(None, device.clone(), None).unwrap();
        let is_apple_gpu = device.supports_family(MTLGPUFamily::Apple1);

        Ok(Self {
            device,
            layer,
            presents_with_transaction: false,
            command_queue,
            paths_rasterization_pipeline_state,
            path_sprites_pipeline_state,
            texture_copy_pipeline_state,
            shadows_pipeline_state,
            backdrop_blur_pipeline_state,
            backdrop_blur_downsample_pipeline_state,
            backdrop_blur_upsample_pipeline_state,
            quads_pipeline_state,
            underlines_pipeline_state,
            monochrome_sprites_pipeline_state,
            polychrome_sprites_pipeline_state,
            surfaces_pipeline_state,
            retained_layer_pipeline_state,
            unit_vertices,
            instance_buffer_pool,
            sprite_atlas,
            core_video_texture_cache,
            path_intermediate_texture: None,
            path_intermediate_msaa_texture: None,
            path_intermediate_size: None,
            frame_texture: None,
            frame_texture_size: None,
            backdrop_texture_size: None,
            backdrop_source_texture: None,
            backdrop_blur_level_sizes: Vec::new(),
            backdrop_blur_downsample_textures: Vec::new(),
            backdrop_blur_upsample_textures: Vec::new(),
            retained_layers: HashMap::default(),
            path_sample_count: PATH_SAMPLE_COUNT,
            is_apple_gpu,
            frame_semaphore: FrameSemaphore::new(MAX_FRAMES_IN_FLIGHT),
            first_presentation_observer: None,
        })
    }

    pub fn layer(&self) -> &metal::MetalLayerRef {
        &self.layer
    }

    pub fn layer_ptr(&self) -> *mut CAMetalLayer {
        self.layer.as_ptr()
    }

    pub fn sprite_atlas(&self) -> &Arc<MetalAtlas> {
        &self.sprite_atlas
    }

    pub fn set_presents_with_transaction(&mut self, presents_with_transaction: bool) {
        self.presents_with_transaction = presents_with_transaction;
        self.layer
            .set_presents_with_transaction(presents_with_transaction);
    }

    pub fn update_drawable_size(&mut self, size: Size<DevicePixels>) {
        let size = NSSize {
            width: size.width.0 as f64,
            height: size.height.0 as f64,
        };
        unsafe {
            let _: () = msg_send![
                self.layer(),
                setDrawableSize: size
            ];
        }
        self.discard_path_intermediate_textures();
        self.discard_frame_texture();
        self.discard_backdrop_textures();
        self.retained_layers.clear();
    }

    fn discard_path_intermediate_textures(&mut self) {
        self.path_intermediate_texture = None;
        self.path_intermediate_msaa_texture = None;
        self.path_intermediate_size = None;
    }

    fn discard_frame_texture(&mut self) {
        self.frame_texture = None;
        self.frame_texture_size = None;
    }

    fn ensure_frame_texture(&mut self, size: Size<DevicePixels>) -> Option<()> {
        if let Some(current_size) = self.frame_texture_size
            && self.frame_texture.is_some()
            && current_size.width >= size.width
            && current_size.height >= size.height
        {
            return Some(());
        }

        if size.width.0 <= 0 || size.height.0 <= 0 {
            self.discard_frame_texture();
            return None;
        }

        let texture_descriptor = metal::TextureDescriptor::new();
        texture_descriptor.set_width(size.width.0 as u64);
        texture_descriptor.set_height(size.height.0 as u64);
        texture_descriptor.set_pixel_format(metal::MTLPixelFormat::BGRA8Unorm);
        texture_descriptor.set_storage_mode(metal::MTLStorageMode::Private);
        texture_descriptor
            .set_usage(metal::MTLTextureUsage::RenderTarget | metal::MTLTextureUsage::ShaderRead);
        self.frame_texture = Some(self.device.new_texture(&texture_descriptor));
        self.frame_texture_size = Some(size);
        Some(())
    }

    fn discard_backdrop_textures(&mut self) {
        self.backdrop_texture_size = None;
        self.backdrop_source_texture = None;
        self.backdrop_blur_level_sizes.clear();
        self.backdrop_blur_downsample_textures.clear();
        self.backdrop_blur_upsample_textures.clear();
    }

    fn ensure_path_intermediate_textures(
        &mut self,
        size: Size<DevicePixels>,
    ) -> Option<Size<DevicePixels>> {
        if let Some(current_size) = self.path_intermediate_size {
            if current_size.width >= size.width && current_size.height >= size.height {
                return Some(current_size);
            }
            return self.create_path_intermediate_textures(Size {
                width: current_size.width.max(size.width),
                height: current_size.height.max(size.height),
            });
        }

        self.create_path_intermediate_textures(size)
    }

    fn create_path_intermediate_textures(
        &mut self,
        size: Size<DevicePixels>,
    ) -> Option<Size<DevicePixels>> {
        // We are uncertain when this happens, but sometimes size can be 0 here. Most likely before
        // the layout pass on window creation. Zero-sized texture creation causes SIGABRT.
        // https://github.com/zed-industries/zed/issues/36229
        if size.width.0 <= 0 || size.height.0 <= 0 {
            self.discard_path_intermediate_textures();
            return None;
        }

        self.discard_path_intermediate_textures();

        let texture_descriptor = metal::TextureDescriptor::new();
        texture_descriptor.set_width(size.width.0 as u64);
        texture_descriptor.set_height(size.height.0 as u64);
        texture_descriptor.set_pixel_format(metal::MTLPixelFormat::BGRA8Unorm);
        texture_descriptor.set_storage_mode(metal::MTLStorageMode::Private);
        texture_descriptor
            .set_usage(metal::MTLTextureUsage::RenderTarget | metal::MTLTextureUsage::ShaderRead);
        self.path_intermediate_texture = Some(self.device.new_texture(&texture_descriptor));

        if self.path_sample_count > 1 {
            let msaa_descriptor = texture_descriptor;
            msaa_descriptor.set_texture_type(metal::MTLTextureType::D2Multisample);
            let storage_mode = if self.is_apple_gpu {
                metal::MTLStorageMode::Memoryless
            } else {
                metal::MTLStorageMode::Private
            };
            msaa_descriptor.set_storage_mode(storage_mode);
            msaa_descriptor.set_sample_count(self.path_sample_count as _);
            self.path_intermediate_msaa_texture = Some(self.device.new_texture(&msaa_descriptor));
        } else {
            self.path_intermediate_msaa_texture = None;
        }
        self.path_intermediate_size = Some(size);
        Some(size)
    }

    fn ensure_backdrop_textures(
        &mut self,
        size: Size<DevicePixels>,
        max_size: Size<DevicePixels>,
    ) -> Option<Size<DevicePixels>> {
        let size = Self::quantize_backdrop_texture_size(size, max_size);
        if let Some(current_size) = self.backdrop_texture_size {
            if can_reuse_backdrop_texture(current_size, size) {
                return Some(current_size);
            }
            return self.create_backdrop_textures(Size {
                width: current_size.width.max(size.width).min(max_size.width),
                height: current_size.height.max(size.height).min(max_size.height),
            });
        }

        self.create_backdrop_textures(size)
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

    fn create_backdrop_textures(&mut self, size: Size<DevicePixels>) -> Option<Size<DevicePixels>> {
        if size.width.0 <= 0 || size.height.0 <= 0 {
            self.discard_backdrop_textures();
            return None;
        }

        self.update_backdrop_blur_textures(size);
        self.backdrop_texture_size = Some(size);
        Some(size)
    }

    fn ensure_backdrop_source_texture(&mut self) -> Option<metal::Texture> {
        let size = self.backdrop_texture_size?;
        if let Some(texture) = &self.backdrop_source_texture {
            return Some(texture.clone());
        }

        let texture_descriptor = metal::TextureDescriptor::new();
        texture_descriptor.set_width(size.width.0 as u64);
        texture_descriptor.set_height(size.height.0 as u64);
        texture_descriptor.set_pixel_format(metal::MTLPixelFormat::BGRA8Unorm);
        texture_descriptor.set_storage_mode(metal::MTLStorageMode::Private);
        texture_descriptor
            .set_usage(metal::MTLTextureUsage::ShaderRead | metal::MTLTextureUsage::RenderTarget);
        let texture = self.device.new_texture(&texture_descriptor);
        self.backdrop_source_texture = Some(texture.clone());
        Some(texture)
    }

    fn update_backdrop_blur_textures(&mut self, size: Size<DevicePixels>) {
        self.backdrop_source_texture = None;
        self.backdrop_blur_level_sizes.clear();
        self.backdrop_blur_downsample_textures.clear();
        self.backdrop_blur_upsample_textures.clear();

        self.backdrop_blur_level_sizes = backdrop_blur_level_sizes_for(size);
        if self.backdrop_blur_level_sizes.is_empty() {
            return;
        }

        let texture_descriptor = metal::TextureDescriptor::new();
        texture_descriptor.set_pixel_format(metal::MTLPixelFormat::BGRA8Unorm);
        texture_descriptor.set_storage_mode(metal::MTLStorageMode::Private);
        texture_descriptor
            .set_usage(metal::MTLTextureUsage::ShaderRead | metal::MTLTextureUsage::RenderTarget);

        for level_size in self.backdrop_blur_level_sizes.iter().skip(1) {
            texture_descriptor.set_width(level_size.width.0 as u64);
            texture_descriptor.set_height(level_size.height.0 as u64);
            self.backdrop_blur_downsample_textures
                .push(self.device.new_texture(&texture_descriptor));
        }

        for level_size in &self.backdrop_blur_level_sizes {
            texture_descriptor.set_width(level_size.width.0 as u64);
            texture_descriptor.set_height(level_size.height.0 as u64);
            self.backdrop_blur_upsample_textures
                .push(self.device.new_texture(&texture_descriptor));
        }
    }

    pub fn update_transparency(&self, transparent: bool) {
        self.layer.set_opaque(!transparent);
    }

    pub fn set_first_presentation_observer(&mut self, observer: FirstPresentationObserver) {
        self.first_presentation_observer = Some(observer);
    }

    fn observe_first_presentation(&self, command_buffer: &metal::CommandBufferRef) {
        let Some(observer) = &self.first_presentation_observer else {
            return;
        };
        if observer.presentation_count() != 0 {
            return;
        }

        let observer = observer.clone();
        let handler = ConcreteBlock::new(move |command_buffer: &metal::CommandBufferRef| {
            record_first_presentation_status(&observer, command_buffer.status());
        });
        let handler = handler.copy();
        command_buffer.add_scheduled_handler(&handler);
    }

    pub fn renderer_info(&self) -> RendererInfo {
        RendererInfo {
            selection: RendererSelection::Default,
            renderer: RendererKind::Metal,
            backend: "Metal".to_string(),
            adapter_name: self.device.name().to_string(),
            adapter_type: RendererAdapterType::Hardware,
            vendor_id: None,
            device_id: None,
        }
    }

    pub fn destroy(&self) {
        // nothing to do
    }

    pub fn draw(&mut self, scene: &Scene) {
        // Throttle CPU submission: block until a previous frame completes.
        // This limits in-flight command buffers to MAX_FRAMES_IN_FLIGHT, preventing
        // the Metal driver from pinning GPU memory for many queued frames.
        self.frame_semaphore.wait();
        let _autorelease_pool = ScopedAutoreleasePool::new();
        let layer = self.layer.clone();
        let viewport_size = layer.drawable_size();
        let viewport_size: Size<DevicePixels> = size(
            (viewport_size.width.ceil() as i32).into(),
            (viewport_size.height.ceil() as i32).into(),
        );
        let has_backdrop_blurs = Self::scene_has_backdrop_blurs(scene);
        let drawable = if let Some(drawable) = layer.next_drawable() {
            drawable
        } else {
            log::error!(
                "failed to retrieve next drawable, drawable size: {:?}",
                viewport_size
            );
            self.frame_semaphore.signal();
            return;
        };
        let required_instance_buffer_size = self.required_instance_buffer_size(scene);
        let mut instance_buffer = self
            .instance_buffer_pool
            .lock()
            .acquire(&self.device, required_instance_buffer_size);

        if !has_backdrop_blurs && self.backdrop_texture_size.is_some() {
            self.discard_backdrop_textures();
        }

        let target_texture = if has_backdrop_blurs {
            if self.ensure_frame_texture(viewport_size).is_none() {
                self.instance_buffer_pool.lock().release(instance_buffer);
                self.frame_semaphore.signal();
                return;
            }
            self.frame_texture.as_ref().unwrap().clone()
        } else {
            drawable.texture().to_owned()
        };

        let command_buffer =
            self.draw_scene(scene, &mut instance_buffer, &target_texture, viewport_size);

        match command_buffer {
            Ok(command_buffer) => {
                let instance_buffer_pool = self.instance_buffer_pool.clone();
                let instance_buffer = Cell::new(Some(instance_buffer));
                let frame_semaphore = self.frame_semaphore.clone();
                let block = ConcreteBlock::new(move |_| {
                    if let Some(instance_buffer) = instance_buffer.take() {
                        instance_buffer_pool.lock().release(instance_buffer);
                    }
                    frame_semaphore.signal();
                });
                let block = block.copy();
                command_buffer.add_completed_handler(&block);

                if target_texture.as_ptr() != drawable.texture().as_ptr() {
                    self.render_texture_copy(
                        &command_buffer,
                        &target_texture,
                        drawable.texture(),
                        0.,
                        0.,
                        viewport_size,
                    );
                }

                if presentation_requires_scheduling_wait(self.presents_with_transaction) {
                    command_buffer.commit();
                    command_buffer.wait_until_scheduled();
                    drawable.present();
                    if let Some(observer) = &self.first_presentation_observer {
                        record_first_presentation_status(observer, command_buffer.status());
                    }
                } else {
                    command_buffer.present_drawable(drawable);
                    self.observe_first_presentation(&command_buffer);
                    command_buffer.commit();
                }
            }
            Err(err) => {
                self.instance_buffer_pool.lock().release(instance_buffer);
                self.frame_semaphore.signal();
                log::error!("failed to render: {}", err);
            }
        }
    }

    /// Renders the scene to a texture and returns the pixel data as an RGBA image.
    /// This does not present the frame to screen - useful for visual testing
    /// where we want to capture what would be rendered without displaying it.
    #[cfg(any(test, feature = "test-support"))]
    pub fn render_to_image(&mut self, scene: &Scene) -> Result<RgbaImage> {
        let _autorelease_pool = ScopedAutoreleasePool::new();
        let layer = self.layer.clone();
        let viewport_size = layer.drawable_size();
        let viewport_size: Size<DevicePixels> = size(
            (viewport_size.width.ceil() as i32).into(),
            (viewport_size.height.ceil() as i32).into(),
        );
        let drawable = layer
            .next_drawable()
            .ok_or_else(|| anyhow::anyhow!("Failed to get drawable for render_to_image"))?;

        let required_instance_buffer_size = self.required_instance_buffer_size(scene);
        let mut instance_buffer = self
            .instance_buffer_pool
            .lock()
            .acquire(&self.device, required_instance_buffer_size);

        let target_texture = if Self::scene_has_backdrop_blurs(scene) {
            if self.ensure_frame_texture(viewport_size).is_none() {
                self.instance_buffer_pool.lock().release(instance_buffer);
                anyhow::bail!("failed to create offscreen frame texture");
            }
            self.frame_texture.as_ref().unwrap().clone()
        } else {
            drawable.texture().to_owned()
        };

        let command_buffer =
            match self.draw_scene(scene, &mut instance_buffer, &target_texture, viewport_size) {
                Ok(command_buffer) => command_buffer,
                Err(err) => {
                    self.instance_buffer_pool.lock().release(instance_buffer);
                    anyhow::bail!("failed to render: {}", err);
                }
            };
        let instance_buffer_pool = self.instance_buffer_pool.clone();
        let instance_buffer = Cell::new(Some(instance_buffer));
        let block = ConcreteBlock::new(move |_| {
            if let Some(instance_buffer) = instance_buffer.take() {
                instance_buffer_pool.lock().release(instance_buffer);
            }
        });
        let block = block.copy();
        command_buffer.add_completed_handler(&block);

        if target_texture.as_ptr() != drawable.texture().as_ptr() {
            self.render_texture_copy(
                &command_buffer,
                &target_texture,
                drawable.texture(),
                0.,
                0.,
                viewport_size,
            );
        }

        // Commit and wait for completion without presenting
        command_buffer.commit();
        command_buffer.wait_until_completed();

        // Read pixels from the texture
        let texture = drawable.texture();
        let width = texture.width() as u32;
        let height = texture.height() as u32;
        let bytes_per_row = width as usize * 4;
        let buffer_size = height as usize * bytes_per_row;

        let mut pixels = vec![0u8; buffer_size];

        let region = metal::MTLRegion {
            origin: metal::MTLOrigin { x: 0, y: 0, z: 0 },
            size: metal::MTLSize {
                width: width as u64,
                height: height as u64,
                depth: 1,
            },
        };

        texture.get_bytes(
            pixels.as_mut_ptr() as *mut std::ffi::c_void,
            bytes_per_row as u64,
            region,
            0,
        );

        // Convert BGRA to RGBA (swap B and R channels)
        for chunk in pixels.chunks_exact_mut(4) {
            chunk.swap(0, 2);
        }

        RgbaImage::from_raw(width, height, pixels)
            .ok_or_else(|| anyhow::anyhow!("Failed to create RgbaImage from pixel data"))
    }

    fn required_instance_buffer_size(&self, scene: &Scene) -> usize {
        let mut size = 0;
        for batch in scene.batches() {
            match batch {
                PrimitiveBatch::Shadows(range) => {
                    add_instance_bytes::<Shadow>(&mut size, range.len());
                }
                PrimitiveBatch::BackdropBlurs(range) => {
                    for _ in range {
                        add_instance_bytes::<BackdropBlur>(&mut size, 1);
                    }
                }
                PrimitiveBatch::Quads(range) => {
                    add_instance_bytes::<Quad>(&mut size, range.len());
                }
                PrimitiveBatch::Paths(range) => {
                    for path in &scene.paths[range] {
                        align_offset(&mut size);
                        size += path.vertices.len() * mem::size_of::<PathRasterizationVertex>();
                        add_instance_bytes::<PathSprite>(&mut size, 1);
                    }
                }
                PrimitiveBatch::Underlines(range) => {
                    add_instance_bytes::<Underline>(&mut size, range.len());
                }
                PrimitiveBatch::MonochromeSprites { range, .. } => {
                    add_instance_bytes::<MonochromeSprite>(&mut size, range.len());
                }
                PrimitiveBatch::PolychromeSprites { range, .. } => {
                    add_instance_bytes::<PolychromeSprite>(&mut size, range.len());
                }
                PrimitiveBatch::Surfaces(range) => {
                    for _ in range {
                        add_instance_bytes::<Surface>(&mut size, 1);
                    }
                }
                PrimitiveBatch::SubpixelSprites { .. } => unreachable!(),
            }
        }
        for _ in &scene.retained_layers {
            add_instance_bytes::<RetainedLayerSprite>(&mut size, 1);
        }
        size
    }

    fn scene_has_backdrop_blurs(scene: &Scene) -> bool {
        scene.batches().any(|batch| match batch {
            PrimitiveBatch::BackdropBlurs(range) => !scene.backdrop_blurs[range].is_empty(),
            _ => false,
        })
    }

    fn draw_scene(
        &mut self,
        scene: &Scene,
        instance_buffer: &mut InstanceBuffer,
        target_texture: &metal::TextureRef,
        viewport_size: Size<DevicePixels>,
    ) -> Result<metal::CommandBuffer> {
        let command_queue = self.command_queue.clone();
        let command_buffer = command_queue.new_command_buffer();
        let alpha = if self.layer.is_opaque() { 1. } else { 0. };
        let mut instance_offset = 0;

        let retained_layers = Self::retained_layers_for_scene(scene);
        if retained_layers.is_empty() {
            self.retained_layers.clear();
            self.draw_primitives(
                &command_buffer,
                target_texture,
                scene,
                instance_buffer,
                &mut instance_offset,
                viewport_size,
                metal::MTLLoadAction::Clear,
                alpha,
            )?;
            self.flush_instance_buffer(instance_buffer, instance_offset);
            return Ok(command_buffer.to_owned());
        }

        let mut cursor = 0;
        let mut load_action = metal::MTLLoadAction::Clear;
        for (cache_key, layer) in &retained_layers {
            if cursor < layer.paint_range.start {
                self.draw_scene_range(
                    &command_buffer,
                    target_texture,
                    scene,
                    cursor..layer.paint_range.start,
                    instance_buffer,
                    &mut instance_offset,
                    viewport_size,
                    load_action,
                    alpha,
                )?;
                load_action = metal::MTLLoadAction::Load;
            }

            let texture = self.retained_layer_texture(
                &command_buffer,
                scene,
                cache_key,
                layer,
                instance_buffer,
                &mut instance_offset,
            )?;
            self.draw_retained_layer(
                &command_buffer,
                target_texture,
                layer,
                &texture,
                instance_buffer,
                &mut instance_offset,
                viewport_size,
                load_action,
                alpha,
            )?;
            load_action = metal::MTLLoadAction::Load;
            cursor = layer.paint_range.end;
        }

        if cursor < scene.len() {
            self.draw_scene_range(
                &command_buffer,
                target_texture,
                scene,
                cursor..scene.len(),
                instance_buffer,
                &mut instance_offset,
                viewport_size,
                load_action,
                alpha,
            )?;
        }

        let active_layer_keys = retained_layers
            .iter()
            .map(|(cache_key, _)| cache_key.clone())
            .collect::<HashSet<_>>();
        self.retained_layers
            .retain(|cache_key, _| active_layer_keys.contains(cache_key));

        self.flush_instance_buffer(instance_buffer, instance_offset);
        Ok(command_buffer.to_owned())
    }

    fn flush_instance_buffer(&self, instance_buffer: &InstanceBuffer, length: usize) {
        if instance_buffer.managed {
            instance_buffer.metal_buffer.did_modify_range(NSRange {
                location: 0,
                length: length as NSUInteger,
            });
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_scene_range(
        &mut self,
        command_buffer: &metal::CommandBufferRef,
        target_texture: &metal::TextureRef,
        scene: &Scene,
        range: std::ops::Range<usize>,
        instance_buffer: &mut InstanceBuffer,
        instance_offset: &mut usize,
        viewport_size: Size<DevicePixels>,
        load_action: metal::MTLLoadAction,
        clear_alpha: f64,
    ) -> Result<()> {
        if range.is_empty() {
            return Ok(());
        }

        let mut range_scene = Scene::default();
        range_scene.replay(range, scene);
        range_scene.finish();
        self.draw_primitives(
            command_buffer,
            target_texture,
            &range_scene,
            instance_buffer,
            instance_offset,
            viewport_size,
            load_action,
            clear_alpha,
        )
    }

    fn retained_layers_for_scene(scene: &Scene) -> Vec<(RetainedLayerCacheKey, &RetainedLayer)> {
        let mut occurrences = HashMap::new();
        let mut layers = scene
            .retained_layers
            .iter()
            .map(|layer| {
                let occurrence = occurrences.entry(layer.id.clone()).or_insert(0);
                let cache_key = RetainedLayerCacheKey {
                    id: layer.id.clone(),
                    occurrence: *occurrence,
                };
                *occurrence += 1;
                (cache_key, layer)
            })
            .filter(|(_, layer)| layer.paint_range.start < layer.paint_range.end)
            .filter(|(_, layer)| layer.paint_range.end <= scene.len())
            .filter(|(_, layer)| !layer.bounds.is_empty())
            .filter(|(_, layer)| !Self::retained_layer_contains_backdrop_blurs(scene, layer))
            .collect::<Vec<_>>();
        layers.sort_by(|a, b| {
            a.1.paint_range
                .start
                .cmp(&b.1.paint_range.start)
                .then_with(|| b.1.paint_range.end.cmp(&a.1.paint_range.end))
        });

        let mut cursor = 0;
        layers
            .into_iter()
            .filter(|(_, layer)| {
                if layer.paint_range.start < cursor {
                    return false;
                }
                cursor = layer.paint_range.end;
                true
            })
            .collect()
    }

    fn retained_layer_contains_backdrop_blurs(scene: &Scene, layer: &RetainedLayer) -> bool {
        !Self::retained_layer_scene(scene, layer)
            .backdrop_blurs
            .is_empty()
    }

    fn retained_layer_scene(scene: &Scene, layer: &RetainedLayer) -> Scene {
        let mut layer_scene = Scene::default();
        layer_scene.replay(layer.paint_range.clone(), scene);
        layer_scene.finish();
        layer_scene
    }

    fn retained_layer_texture_size(layer: &RetainedLayer) -> Size<DevicePixels> {
        size(
            layer.bounds.size.width.into(),
            layer.bounds.size.height.into(),
        )
    }

    fn localize_retained_layer_scene(scene: &mut Scene, origin: Point<ScaledPixels>) {
        for shadow in &mut scene.shadows {
            Self::localize_bounds(&mut shadow.bounds, origin);
            Self::localize_content_mask(&mut shadow.content_mask, origin);
        }
        for blur in &mut scene.backdrop_blurs {
            Self::localize_bounds(&mut blur.bounds, origin);
            Self::localize_content_mask(&mut blur.content_mask, origin);
        }
        for quad in &mut scene.quads {
            Self::localize_bounds(&mut quad.bounds, origin);
            Self::localize_content_mask(&mut quad.content_mask, origin);
        }
        for path in &mut scene.paths {
            Self::localize_bounds(&mut path.bounds, origin);
            Self::localize_content_mask(&mut path.content_mask, origin);
            for vertex in &mut path.vertices {
                vertex.xy_position = vertex.xy_position - origin;
                Self::localize_content_mask(&mut vertex.content_mask, origin);
            }
        }
        for underline in &mut scene.underlines {
            Self::localize_bounds(&mut underline.bounds, origin);
            Self::localize_content_mask(&mut underline.content_mask, origin);
        }
        for sprite in &mut scene.monochrome_sprites {
            Self::localize_bounds(&mut sprite.bounds, origin);
            Self::localize_content_mask(&mut sprite.content_mask, origin);
            sprite.transformation = Self::localize_transformation(sprite.transformation, origin);
        }
        for sprite in &mut scene.subpixel_sprites {
            Self::localize_bounds(&mut sprite.bounds, origin);
            Self::localize_content_mask(&mut sprite.content_mask, origin);
            sprite.transformation = Self::localize_transformation(sprite.transformation, origin);
        }
        for sprite in &mut scene.polychrome_sprites {
            Self::localize_bounds(&mut sprite.bounds, origin);
            Self::localize_content_mask(&mut sprite.content_mask, origin);
        }
        for surface in &mut scene.surfaces {
            Self::localize_bounds(&mut surface.bounds, origin);
            Self::localize_content_mask(&mut surface.content_mask, origin);
        }
    }

    fn localize_bounds(bounds: &mut Bounds<ScaledPixels>, origin: Point<ScaledPixels>) {
        *bounds = *bounds - origin;
    }

    fn localize_content_mask(
        content_mask: &mut ContentMask<ScaledPixels>,
        origin: Point<ScaledPixels>,
    ) {
        Self::localize_bounds(&mut content_mask.bounds, origin);
        Self::localize_bounds(&mut content_mask.rounded_bounds, origin);
    }

    fn localize_transformation(
        mut transformation: TransformationMatrix,
        origin: Point<ScaledPixels>,
    ) -> TransformationMatrix {
        let x = origin.x.0;
        let y = origin.y.0;
        transformation.translation[0] +=
            transformation.rotation_scale[0][0] * x + transformation.rotation_scale[0][1] * y - x;
        transformation.translation[1] +=
            transformation.rotation_scale[1][0] * x + transformation.rotation_scale[1][1] * y - y;
        transformation
    }

    #[allow(clippy::too_many_arguments)]
    fn retained_layer_texture(
        &mut self,
        command_buffer: &metal::CommandBufferRef,
        scene: &Scene,
        cache_key: &RetainedLayerCacheKey,
        layer: &RetainedLayer,
        instance_buffer: &mut InstanceBuffer,
        instance_offset: &mut usize,
    ) -> Result<metal::Texture> {
        let texture_size = Self::retained_layer_texture_size(layer);
        let cache_is_valid = self
            .retained_layers
            .get(cache_key)
            .map(|cached| {
                !layer.content_dirty
                    && cached.content_revision == layer.content_revision
                    && cached.texture_size == texture_size
            })
            .unwrap_or(false);

        if !cache_is_valid {
            let texture = if let Some(cached) = self.retained_layers.get(cache_key)
                && cached.texture_size == texture_size
            {
                cached.texture.clone()
            } else {
                self.create_retained_layer_texture(texture_size)?
            };
            let mut layer_scene = Self::retained_layer_scene(scene, layer);
            Self::localize_retained_layer_scene(&mut layer_scene, layer.bounds.origin);
            self.draw_primitives(
                command_buffer,
                &texture,
                &layer_scene,
                instance_buffer,
                instance_offset,
                texture_size,
                metal::MTLLoadAction::Clear,
                0.0,
            )?;
            self.retained_layers.insert(
                cache_key.clone(),
                CachedRetainedLayer {
                    content_revision: layer.content_revision,
                    texture_size,
                    texture,
                },
            );
        }

        Ok(self
            .retained_layers
            .get(cache_key)
            .expect("retained layer cache should exist")
            .texture
            .clone())
    }

    fn create_retained_layer_texture(&self, size: Size<DevicePixels>) -> Result<metal::Texture> {
        if size.width.0 <= 0 || size.height.0 <= 0 {
            anyhow::bail!("retained layer texture size must be non-zero");
        }

        let texture_descriptor = metal::TextureDescriptor::new();
        texture_descriptor.set_width(size.width.0 as u64);
        texture_descriptor.set_height(size.height.0 as u64);
        texture_descriptor.set_pixel_format(metal::MTLPixelFormat::BGRA8Unorm);
        texture_descriptor.set_storage_mode(metal::MTLStorageMode::Private);
        texture_descriptor
            .set_usage(metal::MTLTextureUsage::RenderTarget | metal::MTLTextureUsage::ShaderRead);
        Ok(self.device.new_texture(&texture_descriptor))
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_retained_layer(
        &mut self,
        command_buffer: &metal::CommandBufferRef,
        target_texture: &metal::TextureRef,
        layer: &RetainedLayer,
        texture: &metal::TextureRef,
        instance_buffer: &mut InstanceBuffer,
        instance_offset: &mut usize,
        viewport_size: Size<DevicePixels>,
        load_action: metal::MTLLoadAction,
        clear_alpha: f64,
    ) -> Result<()> {
        let command_encoder = new_command_encoder(
            command_buffer,
            target_texture,
            viewport_size,
            |color_attachment| {
                color_attachment.set_load_action(load_action);
                if matches!(load_action, metal::MTLLoadAction::Clear) {
                    color_attachment.set_clear_color(metal::MTLClearColor::new(
                        0.,
                        0.,
                        0.,
                        clear_alpha,
                    ));
                }
            },
        );

        let ok = self.draw_retained_layer_texture(
            layer,
            texture,
            instance_buffer,
            instance_offset,
            viewport_size,
            command_encoder,
        );
        command_encoder.end_encoding();
        if !ok {
            anyhow::bail!("scene too large for retained layer composite");
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_primitives(
        &mut self,
        command_buffer: &metal::CommandBufferRef,
        target_texture: &metal::TextureRef,
        scene: &Scene,
        instance_buffer: &mut InstanceBuffer,
        instance_offset: &mut usize,
        viewport_size: Size<DevicePixels>,
        load_action: metal::MTLLoadAction,
        clear_alpha: f64,
    ) -> Result<()> {
        let mut command_encoder = new_command_encoder(
            command_buffer,
            target_texture,
            viewport_size,
            |color_attachment| {
                color_attachment.set_load_action(load_action);
                if matches!(load_action, metal::MTLLoadAction::Clear) {
                    color_attachment.set_clear_color(metal::MTLClearColor::new(
                        0.,
                        0.,
                        0.,
                        clear_alpha,
                    ));
                }
            },
        );
        for batch in scene.batches() {
            let ok = match batch {
                PrimitiveBatch::Shadows(range) => self.draw_shadows(
                    &scene.shadows[range],
                    instance_buffer,
                    instance_offset,
                    viewport_size,
                    command_encoder,
                ),
                PrimitiveBatch::BackdropBlurs(range) => {
                    let blurs = &scene.backdrop_blurs[range];
                    command_encoder.end_encoding();
                    let mut ok = true;
                    for blurs in backdrop_blur_clusters(blurs, viewport_size) {
                        let Some(mut scratch_bounds) =
                            backdrop_scratch_bounds(&blurs, viewport_size)
                        else {
                            continue;
                        };
                        let Some(texture_size) = self.ensure_backdrop_textures(
                            scratch_bounds.texture_size,
                            max_backdrop_texture_size(scratch_bounds, viewport_size),
                        ) else {
                            continue;
                        };
                        scratch_bounds = fit_backdrop_scratch_bounds(
                            scratch_bounds,
                            texture_size,
                            viewport_size,
                        );
                        let level_sizes = self.backdrop_blur_level_sizes.clone();
                        let prepared_blurs = prepare_backdrop_blurs(&blurs, scratch_bounds);
                        let plan_groups =
                            backdrop_blur_plan_groups(&blurs, level_sizes.len().saturating_sub(1));

                        let source_snapshot =
                            if Self::backdrop_blur_needs_source_snapshot(&plan_groups) {
                                let Some(texture) = self.ensure_backdrop_source_texture() else {
                                    anyhow::bail!("failed to create backdrop source texture");
                                };
                                self.render_texture_copy(
                                    command_buffer,
                                    target_texture,
                                    &texture,
                                    scratch_bounds.bounds.origin.x.0,
                                    scratch_bounds.bounds.origin.y.0,
                                    scratch_bounds.texture_size,
                                );
                                Some(texture)
                            } else {
                                None
                            };

                        let source_texture = source_snapshot
                            .as_ref()
                            .map(|texture| texture.as_ref())
                            .unwrap_or(target_texture);
                        let source_texture_size = if source_snapshot.is_some() {
                            scratch_bounds.texture_size
                        } else {
                            viewport_size
                        };
                        let mut input_scratch_bounds = scratch_bounds;
                        if source_snapshot.is_some() {
                            input_scratch_bounds.bounds.origin =
                                point(ScaledPixels(0.0), ScaledPixels(0.0));
                        }

                        for (start, end, plan) in plan_groups {
                            let blur_texture = self.render_backdrop_blur_texture_for_plan(
                                command_buffer,
                                source_texture,
                                source_texture_size,
                                input_scratch_bounds,
                                plan,
                                &level_sizes,
                                source_snapshot.is_none(),
                            );
                            command_encoder = new_command_encoder(
                                command_buffer,
                                target_texture,
                                viewport_size,
                                |color_attachment| {
                                    color_attachment.set_load_action(metal::MTLLoadAction::Load);
                                },
                            );

                            if let Some(blur_texture) = blur_texture {
                                ok = self.draw_backdrop_blurs(
                                    &prepared_blurs[start..end],
                                    instance_buffer,
                                    instance_offset,
                                    viewport_size,
                                    command_encoder,
                                    blur_texture,
                                );
                            }
                            command_encoder.end_encoding();
                            if !ok {
                                break;
                            }
                        }
                        if !ok {
                            break;
                        }
                    }
                    command_encoder = new_command_encoder(
                        command_buffer,
                        target_texture,
                        viewport_size,
                        |color_attachment| {
                            color_attachment.set_load_action(metal::MTLLoadAction::Load);
                        },
                    );
                    ok
                }
                PrimitiveBatch::Quads(range) => self.draw_quads(
                    &scene.quads[range],
                    instance_buffer,
                    instance_offset,
                    viewport_size,
                    command_encoder,
                ),
                PrimitiveBatch::Paths(range) => {
                    let paths = &scene.paths[range];
                    command_encoder.end_encoding();
                    if paths.is_empty() {
                        command_encoder = new_command_encoder(
                            command_buffer,
                            target_texture,
                            viewport_size,
                            |color_attachment| {
                                color_attachment.set_load_action(metal::MTLLoadAction::Load);
                            },
                        );
                        continue;
                    }

                    let mut ok = true;
                    let path_ranges: Vec<_> =
                        (0..paths.len()).map(|index| index..index + 1).collect();

                    for path_range in path_ranges {
                        let paths = &paths[path_range];
                        let Some(mut scratch_bounds) =
                            Self::path_scratch_bounds(paths, viewport_size)
                        else {
                            continue;
                        };
                        let did_draw = if let Some(texture_size) =
                            self.ensure_path_intermediate_textures(scratch_bounds.texture_size)
                        {
                            scratch_bounds.texture_size = texture_size;
                            self.draw_paths_to_intermediate(
                                paths,
                                scratch_bounds,
                                instance_buffer,
                                instance_offset,
                                command_buffer,
                            )
                        } else {
                            false
                        };

                        command_encoder = new_command_encoder(
                            command_buffer,
                            target_texture,
                            viewport_size,
                            |color_attachment| {
                                color_attachment.set_load_action(metal::MTLLoadAction::Load);
                            },
                        );

                        ok = did_draw
                            && self.draw_paths_from_intermediate(
                                paths,
                                scratch_bounds,
                                instance_buffer,
                                instance_offset,
                                viewport_size,
                                command_encoder,
                            );
                        if !ok {
                            break;
                        }

                        command_encoder.end_encoding();
                    }
                    command_encoder = new_command_encoder(
                        command_buffer,
                        target_texture,
                        viewport_size,
                        |color_attachment| {
                            color_attachment.set_load_action(metal::MTLLoadAction::Load);
                        },
                    );
                    ok
                }
                PrimitiveBatch::Underlines(range) => self.draw_underlines(
                    &scene.underlines[range],
                    instance_buffer,
                    instance_offset,
                    viewport_size,
                    command_encoder,
                ),
                PrimitiveBatch::MonochromeSprites { texture_id, range } => self
                    .draw_monochrome_sprites(
                        texture_id,
                        &scene.monochrome_sprites[range],
                        instance_buffer,
                        instance_offset,
                        viewport_size,
                        command_encoder,
                    ),
                PrimitiveBatch::PolychromeSprites { texture_id, range } => self
                    .draw_polychrome_sprites(
                        texture_id,
                        &scene.polychrome_sprites[range],
                        instance_buffer,
                        instance_offset,
                        viewport_size,
                        command_encoder,
                    ),
                PrimitiveBatch::Surfaces(range) => self.draw_surfaces(
                    &scene.surfaces[range],
                    instance_buffer,
                    instance_offset,
                    viewport_size,
                    command_encoder,
                ),
                PrimitiveBatch::SubpixelSprites { .. } => unreachable!(),
            };
            if !ok {
                command_encoder.end_encoding();
                anyhow::bail!(
                    "scene too large: {} paths, {} shadows, {} blurs, {} quads, {} underlines, {} mono, {} poly, {} surfaces",
                    scene.paths.len(),
                    scene.shadows.len(),
                    scene.backdrop_blurs.len(),
                    scene.quads.len(),
                    scene.underlines.len(),
                    scene.monochrome_sprites.len(),
                    scene.polychrome_sprites.len(),
                    scene.surfaces.len(),
                );
            }
        }

        command_encoder.end_encoding();

        if instance_buffer.managed {
            instance_buffer.metal_buffer.did_modify_range(NSRange {
                location: 0,
                length: *instance_offset as NSUInteger,
            });
        }
        Ok(())
    }

    fn render_texture_copy(
        &self,
        command_buffer: &metal::CommandBufferRef,
        source_texture: &metal::TextureRef,
        destination_texture: &metal::TextureRef,
        source_origin_x: f32,
        source_origin_y: f32,
        destination_size: Size<DevicePixels>,
    ) {
        let params = TextureCopyParams {
            source_origin: [source_origin_x, source_origin_y],
            destination_size: [
                destination_size.width.0 as f32,
                destination_size.height.0 as f32,
            ],
        };

        let command_encoder = new_command_encoder(
            command_buffer,
            destination_texture,
            destination_size,
            |color_attachment| {
                color_attachment.set_load_action(metal::MTLLoadAction::Clear);
                color_attachment.set_clear_color(metal::MTLClearColor::new(0., 0., 0., 0.));
            },
        );
        command_encoder.set_render_pipeline_state(&self.texture_copy_pipeline_state);
        command_encoder.set_vertex_buffer(0, Some(&self.unit_vertices), 0);
        command_encoder.set_vertex_bytes(
            1,
            mem::size_of_val(&params) as u64,
            &params as *const TextureCopyParams as *const _,
        );
        command_encoder.set_fragment_bytes(
            1,
            mem::size_of_val(&params) as u64,
            &params as *const TextureCopyParams as *const _,
        );
        command_encoder.set_fragment_texture(0, Some(source_texture));
        command_encoder.draw_primitives(metal::MTLPrimitiveType::Triangle, 0, 6);
        command_encoder.end_encoding();
    }

    fn backdrop_blur_needs_source_snapshot(
        plan_groups: &[(usize, usize, BackdropBlurPlan)],
    ) -> bool {
        plan_groups.len() > 1 || plan_groups.iter().any(|(_, _, plan)| plan.passes == 0)
    }

    fn render_backdrop_blur_texture_for_plan<'a>(
        &'a self,
        command_buffer: &metal::CommandBufferRef,
        source_texture: &'a metal::TextureRef,
        source_texture_size: Size<DevicePixels>,
        scratch_bounds: BackdropScratchBounds,
        plan: BackdropBlurPlan,
        level_sizes: &[Size<DevicePixels>],
        source_texture_is_target: bool,
    ) -> Option<&'a metal::TextureRef> {
        if plan.passes == 0 {
            return (!source_texture_is_target).then_some(source_texture);
        }

        if self.backdrop_blur_downsample_textures.len() < plan.passes
            || self.backdrop_blur_upsample_textures.is_empty()
        {
            return None;
        }

        let mut input_texture = source_texture;
        for level in 0..plan.passes {
            let output_texture: &metal::TextureRef = &self.backdrop_blur_downsample_textures[level];
            let input_size = level_sizes[level];
            let output_size = level_sizes[level + 1];
            let (input_origin, texture_size) = if level == 0 {
                (
                    [
                        scratch_bounds.bounds.origin.x.0,
                        scratch_bounds.bounds.origin.y.0,
                    ],
                    source_texture_size,
                )
            } else {
                ([0.0, 0.0], input_size)
            };
            self.draw_backdrop_blur_pass(
                command_buffer,
                &self.backdrop_blur_downsample_pipeline_state,
                input_texture,
                output_texture,
                input_origin,
                input_size,
                texture_size,
                output_size,
                plan.sample_distance,
            );
            input_texture = output_texture;
        }

        let mut input_texture: &metal::TextureRef =
            &self.backdrop_blur_downsample_textures[plan.passes - 1];
        for level in (0..plan.passes).rev() {
            let output_texture: &metal::TextureRef = &self.backdrop_blur_upsample_textures[level];
            let input_size = level_sizes[level + 1];
            let output_size = level_sizes[level];
            self.draw_backdrop_blur_pass(
                command_buffer,
                &self.backdrop_blur_upsample_pipeline_state,
                input_texture,
                output_texture,
                [0.0, 0.0],
                input_size,
                input_size,
                output_size,
                plan.sample_distance,
            );
            input_texture = output_texture;
        }

        self.backdrop_blur_upsample_textures
            .first()
            .map(|texture| texture.as_ref())
    }

    fn draw_backdrop_blur_pass(
        &self,
        command_buffer: &metal::CommandBufferRef,
        pipeline_state: &metal::RenderPipelineStateRef,
        input_texture: &metal::TextureRef,
        output_texture: &metal::TextureRef,
        input_origin: [f32; 2],
        input_size: Size<DevicePixels>,
        texture_size: Size<DevicePixels>,
        output_size: Size<DevicePixels>,
        sample_distance: f32,
    ) {
        let render_pass_descriptor = metal::RenderPassDescriptor::new();
        let color_attachment = render_pass_descriptor
            .color_attachments()
            .object_at(0)
            .unwrap();
        color_attachment.set_texture(Some(output_texture));
        color_attachment.set_load_action(metal::MTLLoadAction::Clear);
        color_attachment.set_clear_color(metal::MTLClearColor::new(0., 0., 0., 0.));
        color_attachment.set_store_action(metal::MTLStoreAction::Store);

        let command_encoder = command_buffer.new_render_command_encoder(render_pass_descriptor);
        command_encoder.set_viewport(metal::MTLViewport {
            originX: 0.0,
            originY: 0.0,
            width: i32::from(output_size.width) as f64,
            height: i32::from(output_size.height) as f64,
            znear: 0.0,
            zfar: 1.0,
        });
        command_encoder.set_render_pipeline_state(pipeline_state);
        command_encoder.set_vertex_buffer(
            BackdropBlurPassInputIndex::Vertices as u64,
            Some(&self.unit_vertices),
            0,
        );

        let params = BackdropBlurParams {
            input_origin,
            input_size,
            texture_size,
            sample_distance,
            pad: 0.0,
        };
        command_encoder.set_fragment_bytes(
            BackdropBlurPassInputIndex::Params as u64,
            mem::size_of_val(&params) as u64,
            &params as *const BackdropBlurParams as *const _,
        );
        command_encoder.set_fragment_texture(
            BackdropBlurPassInputIndex::SourceTexture as u64,
            Some(input_texture),
        );

        command_encoder.draw_primitives(metal::MTLPrimitiveType::Triangle, 0, 6);
        command_encoder.end_encoding();
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
            origin: point(ScaledPixels(0.0), ScaledPixels(0.0)),
            size: size(
                ScaledPixels::from(viewport_size.width),
                ScaledPixels::from(viewport_size.height),
            ),
        };
        bounds = bounds.dilate(ScaledPixels(1.0)).intersect(&viewport_bounds);
        if bounds.is_empty() {
            return None;
        }

        let origin = bounds.origin.map(|component| component.floor());
        let bottom_right = bounds.bottom_right().map(|component| component.ceil());
        let bounds = Bounds::from_corners(origin, bottom_right);
        Some(PathScratchBounds {
            texture_size: size(
                DevicePixels::from(bounds.size.width),
                DevicePixels::from(bounds.size.height),
            ),
            bounds,
        })
    }

    fn draw_paths_to_intermediate(
        &self,
        paths: &[Path<ScaledPixels>],
        scratch_bounds: PathScratchBounds,
        instance_buffer: &mut InstanceBuffer,
        instance_offset: &mut usize,
        command_buffer: &metal::CommandBufferRef,
    ) -> bool {
        if paths.is_empty() {
            return true;
        }
        let Some(intermediate_texture) = &self.path_intermediate_texture else {
            return false;
        };

        let render_pass_descriptor = metal::RenderPassDescriptor::new();
        let color_attachment = render_pass_descriptor
            .color_attachments()
            .object_at(0)
            .unwrap();
        color_attachment.set_load_action(metal::MTLLoadAction::Clear);
        color_attachment.set_clear_color(metal::MTLClearColor::new(0., 0., 0., 0.));

        if let Some(msaa_texture) = &self.path_intermediate_msaa_texture {
            color_attachment.set_texture(Some(msaa_texture));
            color_attachment.set_resolve_texture(Some(intermediate_texture));
            color_attachment.set_store_action(metal::MTLStoreAction::MultisampleResolve);
        } else {
            color_attachment.set_texture(Some(intermediate_texture));
            color_attachment.set_store_action(metal::MTLStoreAction::Store);
        }

        let command_encoder = command_buffer.new_render_command_encoder(render_pass_descriptor);
        command_encoder.set_render_pipeline_state(&self.paths_rasterization_pipeline_state);

        align_offset(instance_offset);
        let mut vertices = Vec::new();
        for path in paths {
            vertices.extend(path.vertices.iter().map(|v| PathRasterizationVertex {
                xy_position: v.xy_position,
                st_position: v.st_position,
                color: path.color,
                bounds: path.bounds.intersect(&path.content_mask.bounds),
                content_mask: path.content_mask.clone(),
                scratch_bounds: scratch_bounds.bounds,
                texture_size: scratch_bounds.texture_size,
            }));
        }
        let vertices_bytes_len = mem::size_of_val(vertices.as_slice());
        let next_offset = *instance_offset + vertices_bytes_len;
        if next_offset > instance_buffer.size {
            command_encoder.end_encoding();
            return false;
        }
        command_encoder.set_vertex_buffer(
            PathRasterizationInputIndex::Vertices as u64,
            Some(&instance_buffer.metal_buffer),
            *instance_offset as u64,
        );
        command_encoder.set_fragment_buffer(
            PathRasterizationInputIndex::Vertices as u64,
            Some(&instance_buffer.metal_buffer),
            *instance_offset as u64,
        );
        let buffer_contents =
            unsafe { (instance_buffer.metal_buffer.contents() as *mut u8).add(*instance_offset) };
        unsafe {
            ptr::copy_nonoverlapping(
                vertices.as_ptr() as *const u8,
                buffer_contents,
                vertices_bytes_len,
            );
        }
        command_encoder.draw_primitives(
            metal::MTLPrimitiveType::Triangle,
            0,
            vertices.len() as u64,
        );
        *instance_offset = next_offset;

        command_encoder.end_encoding();
        true
    }

    fn draw_shadows(
        &self,
        shadows: &[Shadow],
        instance_buffer: &mut InstanceBuffer,
        instance_offset: &mut usize,
        viewport_size: Size<DevicePixels>,
        command_encoder: &metal::RenderCommandEncoderRef,
    ) -> bool {
        if shadows.is_empty() {
            return true;
        }
        align_offset(instance_offset);

        command_encoder.set_render_pipeline_state(&self.shadows_pipeline_state);
        command_encoder.set_vertex_buffer(
            ShadowInputIndex::Vertices as u64,
            Some(&self.unit_vertices),
            0,
        );
        command_encoder.set_vertex_buffer(
            ShadowInputIndex::Shadows as u64,
            Some(&instance_buffer.metal_buffer),
            *instance_offset as u64,
        );
        command_encoder.set_fragment_buffer(
            ShadowInputIndex::Shadows as u64,
            Some(&instance_buffer.metal_buffer),
            *instance_offset as u64,
        );

        command_encoder.set_vertex_bytes(
            ShadowInputIndex::ViewportSize as u64,
            mem::size_of_val(&viewport_size) as u64,
            &viewport_size as *const Size<DevicePixels> as *const _,
        );

        let shadow_bytes_len = mem::size_of_val(shadows);
        let buffer_contents =
            unsafe { (instance_buffer.metal_buffer.contents() as *mut u8).add(*instance_offset) };

        let next_offset = *instance_offset + shadow_bytes_len;
        if next_offset > instance_buffer.size {
            return false;
        }

        unsafe {
            ptr::copy_nonoverlapping(
                shadows.as_ptr() as *const u8,
                buffer_contents,
                shadow_bytes_len,
            );
        }

        command_encoder.draw_primitives_instanced(
            metal::MTLPrimitiveType::Triangle,
            0,
            6,
            shadows.len() as u64,
        );
        *instance_offset = next_offset;
        true
    }

    fn draw_quads(
        &self,
        quads: &[Quad],
        instance_buffer: &mut InstanceBuffer,
        instance_offset: &mut usize,
        viewport_size: Size<DevicePixels>,
        command_encoder: &metal::RenderCommandEncoderRef,
    ) -> bool {
        if quads.is_empty() {
            return true;
        }
        align_offset(instance_offset);

        command_encoder.set_render_pipeline_state(&self.quads_pipeline_state);
        command_encoder.set_vertex_buffer(
            QuadInputIndex::Vertices as u64,
            Some(&self.unit_vertices),
            0,
        );
        command_encoder.set_vertex_buffer(
            QuadInputIndex::Quads as u64,
            Some(&instance_buffer.metal_buffer),
            *instance_offset as u64,
        );
        command_encoder.set_fragment_buffer(
            QuadInputIndex::Quads as u64,
            Some(&instance_buffer.metal_buffer),
            *instance_offset as u64,
        );

        command_encoder.set_vertex_bytes(
            QuadInputIndex::ViewportSize as u64,
            mem::size_of_val(&viewport_size) as u64,
            &viewport_size as *const Size<DevicePixels> as *const _,
        );

        let quad_bytes_len = mem::size_of_val(quads);
        let buffer_contents =
            unsafe { (instance_buffer.metal_buffer.contents() as *mut u8).add(*instance_offset) };

        let next_offset = *instance_offset + quad_bytes_len;
        if next_offset > instance_buffer.size {
            return false;
        }

        unsafe {
            ptr::copy_nonoverlapping(quads.as_ptr() as *const u8, buffer_contents, quad_bytes_len);
        }

        command_encoder.draw_primitives_instanced(
            metal::MTLPrimitiveType::Triangle,
            0,
            6,
            quads.len() as u64,
        );
        *instance_offset = next_offset;
        true
    }

    fn draw_backdrop_blurs(
        &self,
        blurs: &[BackdropBlur],
        instance_buffer: &mut InstanceBuffer,
        instance_offset: &mut usize,
        viewport_size: Size<DevicePixels>,
        command_encoder: &metal::RenderCommandEncoderRef,
        source_texture: &metal::TextureRef,
    ) -> bool {
        if blurs.is_empty() {
            return true;
        }
        align_offset(instance_offset);

        command_encoder.set_render_pipeline_state(&self.backdrop_blur_pipeline_state);
        command_encoder.set_vertex_buffer(
            BackdropBlurInputIndex::Vertices as u64,
            Some(&self.unit_vertices),
            0,
        );
        command_encoder.set_vertex_buffer(
            BackdropBlurInputIndex::Blurs as u64,
            Some(&instance_buffer.metal_buffer),
            *instance_offset as u64,
        );
        command_encoder.set_fragment_buffer(
            BackdropBlurInputIndex::Blurs as u64,
            Some(&instance_buffer.metal_buffer),
            *instance_offset as u64,
        );
        command_encoder.set_vertex_bytes(
            BackdropBlurInputIndex::ViewportSize as u64,
            mem::size_of_val(&viewport_size) as u64,
            &viewport_size as *const Size<DevicePixels> as *const _,
        );
        command_encoder.set_fragment_bytes(
            BackdropBlurInputIndex::ViewportSize as u64,
            mem::size_of_val(&viewport_size) as u64,
            &viewport_size as *const Size<DevicePixels> as *const _,
        );
        command_encoder.set_fragment_texture(
            BackdropBlurInputIndex::BackdropTexture as u64,
            Some(source_texture),
        );

        let blur_bytes_len = mem::size_of_val(blurs);
        let buffer_contents =
            unsafe { (instance_buffer.metal_buffer.contents() as *mut u8).add(*instance_offset) };
        let next_offset = *instance_offset + blur_bytes_len;
        if next_offset > instance_buffer.size {
            return false;
        }

        unsafe {
            ptr::copy_nonoverlapping(blurs.as_ptr() as *const u8, buffer_contents, blur_bytes_len);
        }

        command_encoder.draw_primitives_instanced(
            metal::MTLPrimitiveType::Triangle,
            0,
            6,
            blurs.len() as u64,
        );
        *instance_offset = next_offset;
        true
    }

    fn draw_paths_from_intermediate(
        &self,
        paths: &[Path<ScaledPixels>],
        scratch_bounds: PathScratchBounds,
        instance_buffer: &mut InstanceBuffer,
        instance_offset: &mut usize,
        viewport_size: Size<DevicePixels>,
        command_encoder: &metal::RenderCommandEncoderRef,
    ) -> bool {
        let Some(first_path) = paths.first() else {
            return true;
        };

        let Some(ref intermediate_texture) = self.path_intermediate_texture else {
            return false;
        };

        command_encoder.set_render_pipeline_state(&self.path_sprites_pipeline_state);
        command_encoder.set_vertex_buffer(
            SpriteInputIndex::Vertices as u64,
            Some(&self.unit_vertices),
            0,
        );
        command_encoder.set_vertex_bytes(
            SpriteInputIndex::ViewportSize as u64,
            mem::size_of_val(&viewport_size) as u64,
            &viewport_size as *const Size<DevicePixels> as *const _,
        );

        command_encoder.set_fragment_texture(
            SpriteInputIndex::AtlasTexture as u64,
            Some(intermediate_texture),
        );

        // When copying paths from the intermediate texture to the drawable,
        // each pixel must only be copied once, in case of transparent paths.
        //
        // If all paths have the same draw order, then their bounds are all
        // disjoint, so we can copy each path's bounds individually. If this
        // batch combines different draw orders, we perform a single copy
        // for a minimal spanning rect.
        let sprites;
        if paths.last().unwrap().order == first_path.order {
            sprites = paths
                .iter()
                .map(|path| PathSprite {
                    bounds: path.clipped_bounds(),
                    scratch_bounds: scratch_bounds.bounds,
                    texture_size: scratch_bounds.texture_size,
                })
                .collect();
        } else {
            let mut bounds = first_path.clipped_bounds();
            for path in paths.iter().skip(1) {
                bounds = bounds.union(&path.clipped_bounds());
            }
            sprites = vec![PathSprite {
                bounds,
                scratch_bounds: scratch_bounds.bounds,
                texture_size: scratch_bounds.texture_size,
            }];
        }

        align_offset(instance_offset);
        let sprite_bytes_len = mem::size_of_val(sprites.as_slice());
        let next_offset = *instance_offset + sprite_bytes_len;
        if next_offset > instance_buffer.size {
            return false;
        }

        command_encoder.set_vertex_buffer(
            SpriteInputIndex::Sprites as u64,
            Some(&instance_buffer.metal_buffer),
            *instance_offset as u64,
        );

        let buffer_contents =
            unsafe { (instance_buffer.metal_buffer.contents() as *mut u8).add(*instance_offset) };
        unsafe {
            ptr::copy_nonoverlapping(
                sprites.as_ptr() as *const u8,
                buffer_contents,
                sprite_bytes_len,
            );
        }

        command_encoder.draw_primitives_instanced(
            metal::MTLPrimitiveType::Triangle,
            0,
            6,
            sprites.len() as u64,
        );
        *instance_offset = next_offset;

        true
    }

    fn draw_underlines(
        &self,
        underlines: &[Underline],
        instance_buffer: &mut InstanceBuffer,
        instance_offset: &mut usize,
        viewport_size: Size<DevicePixels>,
        command_encoder: &metal::RenderCommandEncoderRef,
    ) -> bool {
        if underlines.is_empty() {
            return true;
        }
        align_offset(instance_offset);

        command_encoder.set_render_pipeline_state(&self.underlines_pipeline_state);
        command_encoder.set_vertex_buffer(
            UnderlineInputIndex::Vertices as u64,
            Some(&self.unit_vertices),
            0,
        );
        command_encoder.set_vertex_buffer(
            UnderlineInputIndex::Underlines as u64,
            Some(&instance_buffer.metal_buffer),
            *instance_offset as u64,
        );
        command_encoder.set_fragment_buffer(
            UnderlineInputIndex::Underlines as u64,
            Some(&instance_buffer.metal_buffer),
            *instance_offset as u64,
        );

        command_encoder.set_vertex_bytes(
            UnderlineInputIndex::ViewportSize as u64,
            mem::size_of_val(&viewport_size) as u64,
            &viewport_size as *const Size<DevicePixels> as *const _,
        );

        let underline_bytes_len = mem::size_of_val(underlines);
        let buffer_contents =
            unsafe { (instance_buffer.metal_buffer.contents() as *mut u8).add(*instance_offset) };

        let next_offset = *instance_offset + underline_bytes_len;
        if next_offset > instance_buffer.size {
            return false;
        }

        unsafe {
            ptr::copy_nonoverlapping(
                underlines.as_ptr() as *const u8,
                buffer_contents,
                underline_bytes_len,
            );
        }

        command_encoder.draw_primitives_instanced(
            metal::MTLPrimitiveType::Triangle,
            0,
            6,
            underlines.len() as u64,
        );
        *instance_offset = next_offset;
        true
    }

    fn draw_monochrome_sprites(
        &self,
        texture_id: AtlasTextureId,
        sprites: &[MonochromeSprite],
        instance_buffer: &mut InstanceBuffer,
        instance_offset: &mut usize,
        viewport_size: Size<DevicePixels>,
        command_encoder: &metal::RenderCommandEncoderRef,
    ) -> bool {
        if sprites.is_empty() {
            return true;
        }
        align_offset(instance_offset);

        let sprite_bytes_len = mem::size_of_val(sprites);
        let buffer_contents =
            unsafe { (instance_buffer.metal_buffer.contents() as *mut u8).add(*instance_offset) };

        let next_offset = *instance_offset + sprite_bytes_len;
        if next_offset > instance_buffer.size {
            return false;
        }

        let texture = self.sprite_atlas.metal_texture(texture_id);
        let texture_size = size(
            DevicePixels(texture.width() as i32),
            DevicePixels(texture.height() as i32),
        );
        command_encoder.set_render_pipeline_state(&self.monochrome_sprites_pipeline_state);
        command_encoder.set_vertex_buffer(
            SpriteInputIndex::Vertices as u64,
            Some(&self.unit_vertices),
            0,
        );
        command_encoder.set_vertex_buffer(
            SpriteInputIndex::Sprites as u64,
            Some(&instance_buffer.metal_buffer),
            *instance_offset as u64,
        );
        command_encoder.set_vertex_bytes(
            SpriteInputIndex::ViewportSize as u64,
            mem::size_of_val(&viewport_size) as u64,
            &viewport_size as *const Size<DevicePixels> as *const _,
        );
        command_encoder.set_vertex_bytes(
            SpriteInputIndex::AtlasTextureSize as u64,
            mem::size_of_val(&texture_size) as u64,
            &texture_size as *const Size<DevicePixels> as *const _,
        );
        command_encoder.set_fragment_buffer(
            SpriteInputIndex::Sprites as u64,
            Some(&instance_buffer.metal_buffer),
            *instance_offset as u64,
        );
        command_encoder.set_fragment_texture(SpriteInputIndex::AtlasTexture as u64, Some(&texture));

        unsafe {
            ptr::copy_nonoverlapping(
                sprites.as_ptr() as *const u8,
                buffer_contents,
                sprite_bytes_len,
            );
        }

        command_encoder.draw_primitives_instanced(
            metal::MTLPrimitiveType::Triangle,
            0,
            6,
            sprites.len() as u64,
        );
        *instance_offset = next_offset;
        true
    }

    fn draw_polychrome_sprites(
        &self,
        texture_id: AtlasTextureId,
        sprites: &[PolychromeSprite],
        instance_buffer: &mut InstanceBuffer,
        instance_offset: &mut usize,
        viewport_size: Size<DevicePixels>,
        command_encoder: &metal::RenderCommandEncoderRef,
    ) -> bool {
        if sprites.is_empty() {
            return true;
        }
        align_offset(instance_offset);

        let texture = self.sprite_atlas.metal_texture(texture_id);
        let texture_size = size(
            DevicePixels(texture.width() as i32),
            DevicePixels(texture.height() as i32),
        );
        command_encoder.set_render_pipeline_state(&self.polychrome_sprites_pipeline_state);
        command_encoder.set_vertex_buffer(
            SpriteInputIndex::Vertices as u64,
            Some(&self.unit_vertices),
            0,
        );
        command_encoder.set_vertex_buffer(
            SpriteInputIndex::Sprites as u64,
            Some(&instance_buffer.metal_buffer),
            *instance_offset as u64,
        );
        command_encoder.set_vertex_bytes(
            SpriteInputIndex::ViewportSize as u64,
            mem::size_of_val(&viewport_size) as u64,
            &viewport_size as *const Size<DevicePixels> as *const _,
        );
        command_encoder.set_vertex_bytes(
            SpriteInputIndex::AtlasTextureSize as u64,
            mem::size_of_val(&texture_size) as u64,
            &texture_size as *const Size<DevicePixels> as *const _,
        );
        command_encoder.set_fragment_buffer(
            SpriteInputIndex::Sprites as u64,
            Some(&instance_buffer.metal_buffer),
            *instance_offset as u64,
        );
        command_encoder.set_fragment_texture(SpriteInputIndex::AtlasTexture as u64, Some(&texture));

        let sprite_bytes_len = mem::size_of_val(sprites);
        let buffer_contents =
            unsafe { (instance_buffer.metal_buffer.contents() as *mut u8).add(*instance_offset) };

        let next_offset = *instance_offset + sprite_bytes_len;
        if next_offset > instance_buffer.size {
            return false;
        }

        unsafe {
            ptr::copy_nonoverlapping(
                sprites.as_ptr() as *const u8,
                buffer_contents,
                sprite_bytes_len,
            );
        }

        command_encoder.draw_primitives_instanced(
            metal::MTLPrimitiveType::Triangle,
            0,
            6,
            sprites.len() as u64,
        );
        *instance_offset = next_offset;
        true
    }

    fn draw_surfaces(
        &mut self,
        surfaces: &[PaintSurface],
        instance_buffer: &mut InstanceBuffer,
        instance_offset: &mut usize,
        viewport_size: Size<DevicePixels>,
        command_encoder: &metal::RenderCommandEncoderRef,
    ) -> bool {
        command_encoder.set_render_pipeline_state(&self.surfaces_pipeline_state);
        command_encoder.set_vertex_buffer(
            SurfaceInputIndex::Vertices as u64,
            Some(&self.unit_vertices),
            0,
        );
        command_encoder.set_vertex_bytes(
            SurfaceInputIndex::ViewportSize as u64,
            mem::size_of_val(&viewport_size) as u64,
            &viewport_size as *const Size<DevicePixels> as *const _,
        );

        for surface in surfaces {
            let texture_size = size(
                DevicePixels::from(surface.image_buffer.get_width() as i32),
                DevicePixels::from(surface.image_buffer.get_height() as i32),
            );

            assert_eq!(
                surface.image_buffer.get_pixel_format(),
                kCVPixelFormatType_420YpCbCr8BiPlanarFullRange
            );

            let y_texture = self
                .core_video_texture_cache
                .create_texture_from_image(
                    surface.image_buffer.as_concrete_TypeRef(),
                    None,
                    MTLPixelFormat::R8Unorm,
                    surface.image_buffer.get_width_of_plane(0),
                    surface.image_buffer.get_height_of_plane(0),
                    0,
                )
                .unwrap();
            let cb_cr_texture = self
                .core_video_texture_cache
                .create_texture_from_image(
                    surface.image_buffer.as_concrete_TypeRef(),
                    None,
                    MTLPixelFormat::RG8Unorm,
                    surface.image_buffer.get_width_of_plane(1),
                    surface.image_buffer.get_height_of_plane(1),
                    1,
                )
                .unwrap();

            align_offset(instance_offset);
            let next_offset = *instance_offset + mem::size_of::<Surface>();
            if next_offset > instance_buffer.size {
                return false;
            }

            command_encoder.set_vertex_buffer(
                SurfaceInputIndex::Surfaces as u64,
                Some(&instance_buffer.metal_buffer),
                *instance_offset as u64,
            );
            command_encoder.set_vertex_bytes(
                SurfaceInputIndex::TextureSize as u64,
                mem::size_of_val(&texture_size) as u64,
                &texture_size as *const Size<DevicePixels> as *const _,
            );
            // let y_texture = y_texture.get_texture().unwrap().
            command_encoder.set_fragment_texture(SurfaceInputIndex::YTexture as u64, unsafe {
                let texture = CVMetalTextureGetTexture(y_texture.as_concrete_TypeRef());
                Some(metal::TextureRef::from_ptr(texture as *mut _))
            });
            command_encoder.set_fragment_texture(SurfaceInputIndex::CbCrTexture as u64, unsafe {
                let texture = CVMetalTextureGetTexture(cb_cr_texture.as_concrete_TypeRef());
                Some(metal::TextureRef::from_ptr(texture as *mut _))
            });

            unsafe {
                let buffer_contents = (instance_buffer.metal_buffer.contents() as *mut u8)
                    .add(*instance_offset)
                    as *mut SurfaceBounds;
                ptr::write(
                    buffer_contents,
                    SurfaceBounds {
                        bounds: surface.bounds,
                        content_mask: surface.content_mask.clone(),
                    },
                );
            }

            command_encoder.draw_primitives(metal::MTLPrimitiveType::Triangle, 0, 6);
            *instance_offset = next_offset;
        }
        true
    }

    fn draw_retained_layer_texture(
        &mut self,
        layer: &RetainedLayer,
        texture: &metal::TextureRef,
        instance_buffer: &mut InstanceBuffer,
        instance_offset: &mut usize,
        viewport_size: Size<DevicePixels>,
        command_encoder: &metal::RenderCommandEncoderRef,
    ) -> bool {
        align_offset(instance_offset);
        let next_offset = *instance_offset + mem::size_of::<RetainedLayerSprite>();
        if next_offset > instance_buffer.size {
            return false;
        }

        command_encoder.set_render_pipeline_state(&self.retained_layer_pipeline_state);
        command_encoder.set_vertex_buffer(
            RetainedLayerInputIndex::Vertices as u64,
            Some(&self.unit_vertices),
            0,
        );
        command_encoder.set_vertex_buffer(
            RetainedLayerInputIndex::Layer as u64,
            Some(&instance_buffer.metal_buffer),
            *instance_offset as u64,
        );
        command_encoder.set_vertex_bytes(
            RetainedLayerInputIndex::ViewportSize as u64,
            mem::size_of_val(&viewport_size) as u64,
            &viewport_size as *const Size<DevicePixels> as *const _,
        );
        command_encoder
            .set_fragment_texture(RetainedLayerInputIndex::LayerTexture as u64, Some(texture));

        unsafe {
            let buffer_contents = (instance_buffer.metal_buffer.contents() as *mut u8)
                .add(*instance_offset)
                as *mut RetainedLayerSprite;
            ptr::write(
                buffer_contents,
                RetainedLayerSprite {
                    bounds: layer.bounds,
                    content_mask: layer.content_mask.clone(),
                    transform: layer.transform,
                    opacity: layer.opacity.clamp(0.0, 1.0),
                    pad: [0.0; 3],
                },
            );
        }

        command_encoder.draw_primitives(metal::MTLPrimitiveType::Triangle, 0, 6);
        *instance_offset = next_offset;
        true
    }
}

fn new_command_encoder<'a>(
    command_buffer: &'a metal::CommandBufferRef,
    target_texture: &'a metal::TextureRef,
    viewport_size: Size<DevicePixels>,
    configure_color_attachment: impl Fn(&RenderPassColorAttachmentDescriptorRef),
) -> &'a metal::RenderCommandEncoderRef {
    let render_pass_descriptor = metal::RenderPassDescriptor::new();
    let color_attachment = render_pass_descriptor
        .color_attachments()
        .object_at(0)
        .unwrap();
    color_attachment.set_texture(Some(target_texture));
    color_attachment.set_store_action(metal::MTLStoreAction::Store);
    configure_color_attachment(color_attachment);

    let command_encoder = command_buffer.new_render_command_encoder(render_pass_descriptor);
    command_encoder.set_viewport(metal::MTLViewport {
        originX: 0.0,
        originY: 0.0,
        width: i32::from(viewport_size.width) as f64,
        height: i32::from(viewport_size.height) as f64,
        znear: 0.0,
        zfar: 1.0,
    });
    command_encoder
}

fn build_pipeline_state(
    device: &metal::DeviceRef,
    library: &metal::LibraryRef,
    label: &str,
    vertex_fn_name: &str,
    fragment_fn_name: &str,
    pixel_format: metal::MTLPixelFormat,
) -> metal::RenderPipelineState {
    let vertex_fn = library
        .get_function(vertex_fn_name, None)
        .expect("error locating vertex function");
    let fragment_fn = library
        .get_function(fragment_fn_name, None)
        .expect("error locating fragment function");

    let descriptor = metal::RenderPipelineDescriptor::new();
    descriptor.set_label(label);
    descriptor.set_vertex_function(Some(vertex_fn.as_ref()));
    descriptor.set_fragment_function(Some(fragment_fn.as_ref()));
    let color_attachment = descriptor.color_attachments().object_at(0).unwrap();
    color_attachment.set_pixel_format(pixel_format);
    color_attachment.set_blending_enabled(true);
    color_attachment.set_rgb_blend_operation(metal::MTLBlendOperation::Add);
    color_attachment.set_alpha_blend_operation(metal::MTLBlendOperation::Add);
    color_attachment.set_source_rgb_blend_factor(metal::MTLBlendFactor::SourceAlpha);
    color_attachment.set_source_alpha_blend_factor(metal::MTLBlendFactor::One);
    color_attachment.set_destination_rgb_blend_factor(metal::MTLBlendFactor::OneMinusSourceAlpha);
    color_attachment.set_destination_alpha_blend_factor(metal::MTLBlendFactor::One);

    device
        .new_render_pipeline_state(&descriptor)
        .expect("could not create render pipeline state")
}

fn build_no_blend_pipeline_state(
    device: &metal::DeviceRef,
    library: &metal::LibraryRef,
    label: &str,
    vertex_fn_name: &str,
    fragment_fn_name: &str,
    pixel_format: metal::MTLPixelFormat,
) -> metal::RenderPipelineState {
    let vertex_fn = library
        .get_function(vertex_fn_name, None)
        .expect("error locating vertex function");
    let fragment_fn = library
        .get_function(fragment_fn_name, None)
        .expect("error locating fragment function");

    let descriptor = metal::RenderPipelineDescriptor::new();
    descriptor.set_label(label);
    descriptor.set_vertex_function(Some(vertex_fn.as_ref()));
    descriptor.set_fragment_function(Some(fragment_fn.as_ref()));
    descriptor
        .color_attachments()
        .object_at(0)
        .unwrap()
        .set_pixel_format(pixel_format);

    device
        .new_render_pipeline_state(&descriptor)
        .expect("could not create render pipeline state")
}

fn build_path_sprite_pipeline_state(
    device: &metal::DeviceRef,
    library: &metal::LibraryRef,
    label: &str,
    vertex_fn_name: &str,
    fragment_fn_name: &str,
    pixel_format: metal::MTLPixelFormat,
) -> metal::RenderPipelineState {
    let vertex_fn = library
        .get_function(vertex_fn_name, None)
        .expect("error locating vertex function");
    let fragment_fn = library
        .get_function(fragment_fn_name, None)
        .expect("error locating fragment function");

    let descriptor = metal::RenderPipelineDescriptor::new();
    descriptor.set_label(label);
    descriptor.set_vertex_function(Some(vertex_fn.as_ref()));
    descriptor.set_fragment_function(Some(fragment_fn.as_ref()));
    let color_attachment = descriptor.color_attachments().object_at(0).unwrap();
    color_attachment.set_pixel_format(pixel_format);
    color_attachment.set_blending_enabled(true);
    color_attachment.set_rgb_blend_operation(metal::MTLBlendOperation::Add);
    color_attachment.set_alpha_blend_operation(metal::MTLBlendOperation::Add);
    color_attachment.set_source_rgb_blend_factor(metal::MTLBlendFactor::One);
    color_attachment.set_source_alpha_blend_factor(metal::MTLBlendFactor::One);
    color_attachment.set_destination_rgb_blend_factor(metal::MTLBlendFactor::OneMinusSourceAlpha);
    color_attachment.set_destination_alpha_blend_factor(metal::MTLBlendFactor::One);

    device
        .new_render_pipeline_state(&descriptor)
        .expect("could not create render pipeline state")
}

fn build_texture_copy_pipeline_state(
    device: &metal::DeviceRef,
    library: &metal::LibraryRef,
    label: &str,
    vertex_fn_name: &str,
    fragment_fn_name: &str,
    pixel_format: metal::MTLPixelFormat,
) -> metal::RenderPipelineState {
    let vertex_fn = library
        .get_function(vertex_fn_name, None)
        .expect("error locating vertex function");
    let fragment_fn = library
        .get_function(fragment_fn_name, None)
        .expect("error locating fragment function");

    let descriptor = metal::RenderPipelineDescriptor::new();
    descriptor.set_label(label);
    descriptor.set_vertex_function(Some(vertex_fn.as_ref()));
    descriptor.set_fragment_function(Some(fragment_fn.as_ref()));
    let color_attachment = descriptor.color_attachments().object_at(0).unwrap();
    color_attachment.set_pixel_format(pixel_format);

    device
        .new_render_pipeline_state(&descriptor)
        .expect("could not create render pipeline state")
}

fn build_path_rasterization_pipeline_state(
    device: &metal::DeviceRef,
    library: &metal::LibraryRef,
    label: &str,
    vertex_fn_name: &str,
    fragment_fn_name: &str,
    pixel_format: metal::MTLPixelFormat,
    path_sample_count: u32,
) -> metal::RenderPipelineState {
    let vertex_fn = library
        .get_function(vertex_fn_name, None)
        .expect("error locating vertex function");
    let fragment_fn = library
        .get_function(fragment_fn_name, None)
        .expect("error locating fragment function");

    let descriptor = metal::RenderPipelineDescriptor::new();
    descriptor.set_label(label);
    descriptor.set_vertex_function(Some(vertex_fn.as_ref()));
    descriptor.set_fragment_function(Some(fragment_fn.as_ref()));
    if path_sample_count > 1 {
        descriptor.set_raster_sample_count(path_sample_count as _);
        descriptor.set_alpha_to_coverage_enabled(false);
    }
    let color_attachment = descriptor.color_attachments().object_at(0).unwrap();
    color_attachment.set_pixel_format(pixel_format);
    color_attachment.set_blending_enabled(true);
    color_attachment.set_rgb_blend_operation(metal::MTLBlendOperation::Add);
    color_attachment.set_alpha_blend_operation(metal::MTLBlendOperation::Add);
    color_attachment.set_source_rgb_blend_factor(metal::MTLBlendFactor::One);
    color_attachment.set_source_alpha_blend_factor(metal::MTLBlendFactor::One);
    color_attachment.set_destination_rgb_blend_factor(metal::MTLBlendFactor::OneMinusSourceAlpha);
    color_attachment.set_destination_alpha_blend_factor(metal::MTLBlendFactor::OneMinusSourceAlpha);

    device
        .new_render_pipeline_state(&descriptor)
        .expect("could not create render pipeline state")
}

// Align to multiples of 256 make Metal happy.
fn align_offset(offset: &mut usize) {
    *offset = (*offset).div_ceil(256) * 256;
}

fn add_instance_bytes<T>(offset: &mut usize, count: usize) {
    if count == 0 {
        return;
    }
    align_offset(offset);
    *offset += mem::size_of::<T>() * count;
}

#[repr(C)]
enum ShadowInputIndex {
    Vertices = 0,
    Shadows = 1,
    ViewportSize = 2,
}

#[repr(C)]
enum BackdropBlurInputIndex {
    Vertices = 0,
    Blurs = 1,
    ViewportSize = 2,
    BackdropTexture = 3,
}

#[repr(C)]
enum BackdropBlurPassInputIndex {
    Vertices = 0,
    Params = 1,
    SourceTexture = 2,
}

#[repr(C)]
enum QuadInputIndex {
    Vertices = 0,
    Quads = 1,
    ViewportSize = 2,
}

#[repr(C)]
enum UnderlineInputIndex {
    Vertices = 0,
    Underlines = 1,
    ViewportSize = 2,
}

#[repr(C)]
enum SpriteInputIndex {
    Vertices = 0,
    Sprites = 1,
    ViewportSize = 2,
    AtlasTextureSize = 3,
    AtlasTexture = 4,
}

#[repr(C)]
enum SurfaceInputIndex {
    Vertices = 0,
    Surfaces = 1,
    ViewportSize = 2,
    TextureSize = 3,
    YTexture = 4,
    CbCrTexture = 5,
}

#[repr(C)]
enum RetainedLayerInputIndex {
    Vertices = 0,
    Layer = 1,
    ViewportSize = 2,
    LayerTexture = 3,
}

#[repr(C)]
enum PathRasterizationInputIndex {
    Vertices = 0,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct PathSprite {
    pub bounds: Bounds<ScaledPixels>,
    pub scratch_bounds: Bounds<ScaledPixels>,
    pub texture_size: Size<DevicePixels>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct SurfaceBounds {
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::Corners;

    #[test]
    fn first_presentation_requires_scheduled_or_completed() {
        use metal::MTLCommandBufferStatus::{
            Committed, Completed, Enqueued, Error, NotEnqueued, Scheduled,
        };

        assert!(first_presentation_is_scheduled(Scheduled));
        assert!(first_presentation_is_scheduled(Completed));
        for status in [NotEnqueued, Enqueued, Committed, Error] {
            assert!(!first_presentation_is_scheduled(status));
        }
    }

    #[test]
    fn first_presentation_status_does_not_record_errors() {
        let observer = FirstPresentationObserver::default();
        let mut notification = observer.subscribe();

        record_first_presentation_status(&observer, metal::MTLCommandBufferStatus::Error);
        assert_eq!(observer.presentation_count(), 0);

        record_first_presentation_status(&observer, metal::MTLCommandBufferStatus::Scheduled);
        assert_eq!(
            notification.try_recv(),
            Ok(Some(PresentationEvidence::BackendAccepted))
        );
    }

    #[test]
    fn normal_presentation_does_not_wait_for_scheduling() {
        assert!(!presentation_requires_scheduling_wait(false));
        assert!(presentation_requires_scheduling_wait(true));
    }

    fn scene_with_retained_primitive(primitive: impl Into<gpui::Primitive>) -> Scene {
        let bounds = Bounds::new(
            Point::new(ScaledPixels(0.0), ScaledPixels(0.0)),
            Size::new(ScaledPixels(100.0), ScaledPixels(100.0)),
        );
        let mut scene = Scene::default();
        scene.insert_primitive(primitive);
        scene.retained_layers.push(RetainedLayer {
            id: GlobalElementId::default(),
            content_revision: 1.into(),
            content_dirty: true,
            bounds,
            content_mask: ContentMask::new(bounds),
            transform: TransformationMatrix::unit(),
            opacity: 1.0,
            paint_range: 0..scene.paint_operation_count(),
        });
        scene
    }

    #[test]
    fn backdrop_blur_snapshot_predicate_preserves_fast_path() {
        let positive_plan = BackdropBlurPlan {
            passes: 1,
            sample_distance: 1.0,
        };

        assert!(!MetalRenderer::backdrop_blur_needs_source_snapshot(&[(
            0,
            1,
            positive_plan,
        )]));
        assert!(MetalRenderer::backdrop_blur_needs_source_snapshot(&[(
            0,
            1,
            BackdropBlurPlan::IDENTITY,
        )]));
        assert!(MetalRenderer::backdrop_blur_needs_source_snapshot(&[
            (0, 1, positive_plan),
            (1, 2, BackdropBlurPlan::IDENTITY),
        ]));
    }

    #[test]
    fn fading_backdrop_blurs_keep_the_snapshot_fast_path() {
        let bounds = Bounds::new(
            Point::new(ScaledPixels(0.0), ScaledPixels(0.0)),
            Size::new(ScaledPixels(100.0), ScaledPixels(100.0)),
        );
        let blur = |order, opacity| BackdropBlur {
            order,
            pad: 0,
            bounds,
            content_mask: ContentMask::new(bounds),
            corner_radii: Corners::all(ScaledPixels(8.0)),
            blur_radius: ScaledPixels(16.0),
            source_origin_x: 0.0,
            source_origin_y: 0.0,
            source_width: 1.0,
            source_height: 1.0,
            opacity,
        };

        // Two surfaces mid-fade at different opacities still share one plan
        // group, so the renderer keeps blurring in place instead of taking a
        // source snapshot.
        let plan_groups =
            backdrop_blur_plan_groups(&[blur(0, 0.9), blur(1, 0.8)], BackdropBlurPlan::MAX_PASSES);

        assert_eq!(plan_groups.len(), 1);
        assert!(!MetalRenderer::backdrop_blur_needs_source_snapshot(
            &plan_groups
        ));
    }

    #[test]
    fn retained_layer_with_backdrop_blur_is_excluded_from_metal_cache() {
        let bounds = Bounds::new(
            Point::new(ScaledPixels(0.0), ScaledPixels(0.0)),
            Size::new(ScaledPixels(100.0), ScaledPixels(100.0)),
        );
        let scene = scene_with_retained_primitive(BackdropBlur {
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

        assert!(MetalRenderer::retained_layers_for_scene(&scene).is_empty());
    }

    #[test]
    fn retained_layer_without_backdrop_blur_remains_cacheable_by_metal() {
        let bounds = Bounds::new(
            Point::new(ScaledPixels(0.0), ScaledPixels(0.0)),
            Size::new(ScaledPixels(100.0), ScaledPixels(100.0)),
        );
        let scene = scene_with_retained_primitive(Quad {
            order: 0,
            bounds,
            content_mask: ContentMask::new(bounds),
            ..Default::default()
        });

        assert_eq!(MetalRenderer::retained_layers_for_scene(&scene).len(), 1);
    }
}
