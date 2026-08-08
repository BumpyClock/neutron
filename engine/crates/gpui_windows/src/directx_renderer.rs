use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    slice,
    sync::{Arc, OnceLock},
};

use anyhow::{Context, Result};
use gpui_util::ResultExt;
use windows::{
    System::DispatcherQueueController,
    UI::Composition::{
        CompositionRoundedRectangleGeometry, Compositor, ContainerVisual,
        Desktop::DesktopWindowTarget, SpriteVisual,
    },
    Win32::{
        Foundation::HWND,
        Graphics::{
            Direct3D::*,
            Direct3D11::*,
            DirectComposition::*,
            DirectWrite::*,
            Dxgi::{Common::*, *},
        },
        System::WinRT::{
            Composition::{ICompositorDesktopInterop, ICompositorInterop},
            CreateDispatcherQueueController, DQTAT_COM_NONE, DQTYPE_THREAD_CURRENT,
            DispatcherQueueOptions,
        },
    },
    core::{IUnknown, Interface},
};
use windows_numerics::Vector2;

use crate::directx_renderer::shader_resources::{RawShaderBytes, ShaderModule, ShaderTarget};
use crate::*;
use gpui::*;

pub(crate) const DISABLE_DIRECT_COMPOSITION: &str = "GPUI_DISABLE_DIRECT_COMPOSITION";
const RENDER_TARGET_FORMAT: DXGI_FORMAT = DXGI_FORMAT_B8G8R8A8_UNORM;
// This configuration is used for MSAA rendering on paths only, and it's guaranteed to be supported by DirectX 11.
const PATH_MULTISAMPLE_COUNT: u32 = 4;
const MAX_BACKDROP_BLUR_LEVELS: usize = BackdropBlurPlan::MAX_PASSES;
const BACKDROP_TEXTURE_SIZE_QUANTUM: i32 = 64;
const MAX_STRUCTURED_BUFFER_BYTES: usize = 256 * 1024 * 1024;

fn rounded_backdrop_rebuild_requested(logical_radius: Option<f32>) -> bool {
    logical_radius.is_some()
}

pub(crate) struct FontInfo {
    pub gamma_ratios: [f32; 4],
    pub grayscale_enhanced_contrast: f32,
    pub subpixel_enhanced_contrast: f32,
    pub is_bgr: bool,
}

pub(crate) struct DirectXRenderer {
    hwnd: HWND,
    atlas: Arc<DirectXAtlas>,
    devices: Option<DirectXRendererDevices>,
    resources: Option<DirectXResources>,
    globals: DirectXGlobalElements,
    pipelines: DirectXRenderPipelines,
    direct_composition: Option<DirectComposition>,
    disable_direct_composition: bool,
    renderer_selection: RendererSelection,
    /// Windows.UI.Composition tree used for the rounded host-backdrop blur mode.
    /// When `Some`, `direct_composition` is `None` (only one composition target
    /// may exist per HWND) and the swap chain is presented through this tree.
    rounded_backdrop: Option<RoundedBackdrop>,
    /// Logical (DIP) corner radius for the rounded backdrop, retained so the
    /// device-pixel radius can be recomputed on resize / DPI change.
    rounded_backdrop_radius: Option<f32>,
    /// Last scale factor applied to the rounded backdrop, retained so the tree
    /// can be rebuilt with the correct device-pixel radius after device loss.
    rounded_backdrop_scale: f32,
    font_info: &'static FontInfo,

    width: u32,
    height: u32,

    /// Whether we want to skip drwaing due to device lost events.
    ///
    /// In that case we want to discard the first frame that we draw as we got reset in the middle of a frame
    /// meaning we lost all the allocated gpu textures and scene resources.
    skip_draws: bool,
    first_presentation_observer: Option<FirstPresentationObserver>,
}

/// Direct3D objects
#[derive(Clone)]
pub(crate) struct DirectXRendererDevices {
    pub(crate) adapter: IDXGIAdapter1,
    pub(crate) dxgi_factory: IDXGIFactory6,
    pub(crate) device: ID3D11Device,
    pub(crate) device_context: ID3D11DeviceContext,
    dxgi_device: Option<IDXGIDevice>,
}

struct DirectXResources {
    // Direct3D rendering objects
    swap_chain: IDXGISwapChain1,
    render_target: Option<ID3D11Texture2D>,
    render_target_view: Option<ID3D11RenderTargetView>,

    // Path intermediate textures (with MSAA)
    path_intermediate_texture: Option<ID3D11Texture2D>,
    path_intermediate_srv: Option<ID3D11ShaderResourceView>,
    path_intermediate_msaa_texture: Option<ID3D11Texture2D>,
    path_intermediate_msaa_view: Option<ID3D11RenderTargetView>,
    path_intermediate_size: Option<Size<DevicePixels>>,
    // Backdrop copy texture
    backdrop_texture: Option<ID3D11Texture2D>,
    backdrop_srv: Option<ID3D11ShaderResourceView>,
    backdrop_size: Option<Size<DevicePixels>>,
    backdrop_blur: Option<BackdropBlurResources>,

    // Cached viewport
    viewport: D3D11_VIEWPORT,
}

struct BackdropBlurResources {
    level_sizes: Vec<(u32, u32)>,
    downsample_textures: Vec<ID3D11Texture2D>,
    downsample_views: Vec<Option<ID3D11RenderTargetView>>,
    downsample_srvs: Vec<Option<ID3D11ShaderResourceView>>,
    upsample_textures: Vec<ID3D11Texture2D>,
    upsample_views: Vec<Option<ID3D11RenderTargetView>>,
    upsample_srvs: Vec<Option<ID3D11ShaderResourceView>>,
}

struct DirectXRenderPipelines {
    shadow_pipeline: PipelineState<Shadow>,
    backdrop_blur_pipeline: PipelineState<BackdropBlur>,
    backdrop_blur_downsample_pipeline: PipelineState<BackdropBlurParams>,
    backdrop_blur_upsample_pipeline: PipelineState<BackdropBlurParams>,
    quad_pipeline: PipelineState<Quad>,
    path_rasterization_pipeline: PipelineState<PathRasterizationSprite>,
    path_sprite_pipeline: PipelineState<PathSprite>,
    underline_pipeline: PipelineState<Underline>,
    mono_sprites: PipelineState<MonochromeSprite>,
    subpixel_sprites: PipelineState<SubpixelSprite>,
    poly_sprites: PipelineState<PolychromeSprite>,
}

#[derive(Clone, Copy)]
struct PathScratchBounds {
    bounds: Bounds<ScaledPixels>,
    texture_size: Size<DevicePixels>,
}

struct DirectXGlobalElements {
    global_params_buffer: Option<ID3D11Buffer>,
    sampler: Option<ID3D11SamplerState>,
    blur_sampler: Option<ID3D11SamplerState>,
}

struct DirectComposition {
    comp_device: IDCompositionDevice,
    comp_target: IDCompositionTarget,
    comp_visual: IDCompositionVisual,
    retained_compositor: RetainedCompositor,
    retained_layers_enabled: bool,
}

fn retained_layer_id(id: &GlobalElementId) -> RetainedLayerId {
    let mut hasher = DefaultHasher::new();
    id.hash(&mut hasher);
    RetainedLayerId(hasher.finish())
}

fn retained_layer_state(layer: &gpui::RetainedLayer, order: usize) -> RetainedLayerState {
    RetainedLayerState {
        order: order.min(u32::MAX as usize) as u32,
        transform: matrix_from_transformation(layer.transform),
        opacity: layer.opacity,
    }
}

fn retained_layer_clip(mask: &ContentMask<ScaledPixels>) -> Option<RetainedLayerClip> {
    if mask.corner_radii == Corners::default() {
        return None;
    }

    let bottom_right = mask.rounded_bounds.bottom_right();
    Some(RetainedLayerClip {
        left: mask.rounded_bounds.origin.x.0,
        top: mask.rounded_bounds.origin.y.0,
        right: bottom_right.x.0,
        bottom: bottom_right.y.0,
        top_left_radius: mask.corner_radii.top_left.0,
        top_right_radius: mask.corner_radii.top_right.0,
        bottom_right_radius: mask.corner_radii.bottom_right.0,
        bottom_left_radius: mask.corner_radii.bottom_left.0,
    })
}

fn matrix_from_transformation(transform: TransformationMatrix) -> windows_numerics::Matrix3x2 {
    windows_numerics::Matrix3x2 {
        M11: transform.rotation_scale[0][0],
        M12: transform.rotation_scale[1][0],
        M21: transform.rotation_scale[0][1],
        M22: transform.rotation_scale[1][1],
        M31: transform.translation[0],
        M32: transform.translation[1],
    }
}

impl DirectXRendererDevices {
    pub(crate) fn new(
        directx_devices: &DirectXDevices,
        disable_direct_composition: bool,
    ) -> Result<Self> {
        let DirectXDevices {
            adapter,
            dxgi_factory,
            device,
            device_context,
            ..
        } = directx_devices;
        let dxgi_device = if disable_direct_composition {
            None
        } else {
            Some(device.cast().context("Creating DXGI device")?)
        };

        Ok(Self {
            adapter: adapter.clone(),
            dxgi_factory: dxgi_factory.clone(),
            device: device.clone(),
            device_context: device_context.clone(),
            dxgi_device,
        })
    }
}

impl DirectXRenderer {
    pub(crate) fn new(
        hwnd: HWND,
        directx_devices: &DirectXDevices,
        disable_direct_composition: bool,
    ) -> Result<Self> {
        if disable_direct_composition {
            log::info!("Direct Composition is disabled.");
        }

        let devices = DirectXRendererDevices::new(directx_devices, disable_direct_composition)
            .context("Creating DirectX devices")?;
        let atlas = Arc::new(DirectXAtlas::new(&devices.device, &devices.device_context));

        let resources = DirectXResources::new(&devices, 1, 1, hwnd, disable_direct_composition)
            .context("Creating DirectX resources")?;
        let globals = DirectXGlobalElements::new(&devices.device)
            .context("Creating DirectX global elements")?;
        let pipelines = DirectXRenderPipelines::new(&devices.device)
            .context("Creating DirectX render pipelines")?;

        let direct_composition = if disable_direct_composition {
            None
        } else {
            let mut composition =
                DirectComposition::new(devices.dxgi_device.as_ref().unwrap(), hwnd)
                    .context("Creating DirectComposition")?;
            composition
                .set_swap_chain(&resources.swap_chain)
                .context("Setting swap chain for DirectComposition")?;
            Some(composition)
        };

        Ok(DirectXRenderer {
            hwnd,
            atlas,
            devices: Some(devices),
            resources: Some(resources),
            globals,
            pipelines,
            direct_composition,
            disable_direct_composition,
            renderer_selection: directx_devices.renderer_selection,
            rounded_backdrop: None,
            rounded_backdrop_radius: None,
            rounded_backdrop_scale: 1.0,
            font_info: Self::get_font_info(),
            width: 1,
            height: 1,
            skip_draws: false,
            first_presentation_observer: None,
        })
    }

    pub(crate) fn sprite_atlas(&self) -> Arc<dyn PlatformAtlas> {
        self.atlas.clone()
    }

    pub(crate) fn set_first_presentation_observer(&mut self, observer: FirstPresentationObserver) {
        self.first_presentation_observer = Some(observer);
    }

    fn pre_draw(&self, clear_color: &[f32; 4]) -> Result<()> {
        let resources = self.resources.as_ref().expect("resources missing");
        let device_context = &self
            .devices
            .as_ref()
            .expect("devices missing")
            .device_context;
        update_buffer(
            device_context,
            self.globals.global_params_buffer.as_ref().unwrap(),
            &[GlobalParams {
                gamma_ratios: self.font_info.gamma_ratios,
                viewport_size: [resources.viewport.Width, resources.viewport.Height],
                grayscale_enhanced_contrast: self.font_info.grayscale_enhanced_contrast,
                subpixel_enhanced_contrast: self.font_info.subpixel_enhanced_contrast,
                is_bgr: self.font_info.is_bgr as u32,
                _pad: [0; 3],
            }],
        )?;
        unsafe {
            device_context.ClearRenderTargetView(
                resources
                    .render_target_view
                    .as_ref()
                    .context("missing render target view")?,
                clear_color,
            );
            device_context
                .OMSetRenderTargets(Some(slice::from_ref(&resources.render_target_view)), None);
            device_context.RSSetViewports(Some(slice::from_ref(&resources.viewport)));
        }
        Ok(())
    }

    #[inline]
    fn present(&mut self) -> Result<()> {
        let result = unsafe {
            self.resources
                .as_ref()
                .expect("resources missing")
                .swap_chain
                .Present(0, DXGI_PRESENT(0))
        };
        result.ok().context("Presenting swap chain failed")?;
        if let Some(observer) = self.first_presentation_observer.take() {
            observer.record_presentation(PresentationEvidence::BackendAccepted);
        }
        Ok(())
    }

    fn update_clean_retained_layers(&mut self, scene: &Scene) -> Result<bool> {
        if !self.supports_retained_layer_scene(scene)
            || scene
                .retained_layers
                .iter()
                .any(|layer| layer.content_dirty)
        {
            return Ok(false);
        }

        let Some(direct_composition) = self.direct_composition.as_mut() else {
            return Ok(false);
        };
        let active_layers = Self::retained_layer_ids(scene);
        if !direct_composition
            .retained_compositor
            .contains_layers(&active_layers)
        {
            return Ok(false);
        }

        direct_composition.enable_retained_layers()?;
        for (order, layer) in scene.retained_layers.iter().enumerate() {
            direct_composition.retained_compositor.update_layer_state(
                retained_layer_id(&layer.id),
                retained_layer_state(layer, order),
            )?;
        }
        direct_composition
            .retained_compositor
            .retain_layers(&active_layers)?;
        direct_composition.commit()?;
        Ok(true)
    }

    fn update_retained_layer_cache(&mut self, scene: &Scene) -> Result<()> {
        let supports_retained_layer_scene = self.supports_retained_layer_scene(scene);
        let Some(direct_composition) = self.direct_composition.as_mut() else {
            return Ok(());
        };
        let swap_chain = self
            .resources
            .as_ref()
            .context("resources missing")?
            .swap_chain
            .clone();

        if !supports_retained_layer_scene {
            direct_composition.disable_retained_layers(&swap_chain)?;
            return Ok(());
        }

        direct_composition.enable_retained_layers()?;
        direct_composition
            .retained_compositor
            .set_root_clip(retained_layer_clip(&scene.retained_layers[0].content_mask))?;
        let active_layers = Self::retained_layer_ids(scene);
        for (order, layer) in scene.retained_layers.iter().enumerate() {
            direct_composition.retained_compositor.set_layer(
                retained_layer_id(&layer.id),
                retained_layer_state(layer, order),
                RetainedLayerContent::SwapChain(&swap_chain),
            )?;
        }
        direct_composition
            .retained_compositor
            .retain_layers(&active_layers)?;
        direct_composition.commit()
    }

    fn supports_retained_layer_scene(&self, scene: &Scene) -> bool {
        let [layer] = scene.retained_layers.as_slice() else {
            return false;
        };
        let viewport_bounds = Bounds::new(
            point(ScaledPixels(0.0), ScaledPixels(0.0)),
            size(
                ScaledPixels(self.width.max(1) as f32),
                ScaledPixels(self.height.max(1) as f32),
            ),
        );
        layer.paint_range == (0..scene.paint_operation_count())
            && layer.bounds == viewport_bounds
            && layer.content_mask.bounds == viewport_bounds
    }

    fn retained_layer_ids(scene: &Scene) -> Vec<RetainedLayerId> {
        scene
            .retained_layers
            .iter()
            .map(|layer| retained_layer_id(&layer.id))
            .collect()
    }

    pub(crate) fn handle_device_lost(&mut self, directx_devices: &DirectXDevices) -> Result<()> {
        try_to_recover_from_device_lost(|| {
            self.handle_device_lost_impl(directx_devices)
                .context("DirectXRenderer handling device lost")
        })
    }

    fn handle_device_lost_impl(&mut self, directx_devices: &DirectXDevices) -> Result<()> {
        // The live composition object is torn down before reconstruction and can
        // be absent after a failed attempt. Retained configuration is the source
        // of truth so a later device-loss retry preserves the requested mode.
        let rounded_requested = rounded_backdrop_rebuild_requested(self.rounded_backdrop_radius);
        // Rounded backdrop mode is rejected when DirectComposition is disabled,
        // so the stored configuration remains the source of truth here.
        let disable_direct_composition = self.disable_direct_composition;

        unsafe {
            #[cfg(debug_assertions)]
            if let Some(devices) = &self.devices {
                report_live_objects(&devices.device)
                    .context("Failed to report live objects after device lost")
                    .log_err();
            }

            self.resources.take();
            if let Some(devices) = &self.devices {
                devices.device_context.OMSetRenderTargets(None, None);
                devices.device_context.ClearState();
                devices.device_context.Flush();
                #[cfg(debug_assertions)]
                report_live_objects(&devices.device)
                    .context("Failed to report live objects after device lost")
                    .log_err();
            }

            self.direct_composition.take();
            self.rounded_backdrop.take();
            self.devices.take();
        }

        let devices = DirectXRendererDevices::new(directx_devices, disable_direct_composition)
            .context("Recreating DirectX devices")?;
        let resources = DirectXResources::new(
            &devices,
            self.width,
            self.height,
            self.hwnd,
            disable_direct_composition,
        )
        .context("Creating DirectX resources")?;
        let globals = DirectXGlobalElements::new(&devices.device)
            .context("Creating DirectXGlobalElements")?;
        let pipelines = DirectXRenderPipelines::new(&devices.device)
            .context("Creating DirectXRenderPipelines")?;

        let direct_composition = if disable_direct_composition || rounded_requested {
            None
        } else {
            let mut composition =
                DirectComposition::new(devices.dxgi_device.as_ref().unwrap(), self.hwnd)?;
            composition.set_swap_chain(&resources.swap_chain)?;
            Some(composition)
        };
        let rounded_backdrop = if rounded_requested {
            let device_radius =
                self.rounded_backdrop_radius.unwrap_or(0.0) * self.rounded_backdrop_scale;
            Some(
                RoundedBackdrop::new(
                    self.hwnd,
                    &resources.swap_chain,
                    self.width,
                    self.height,
                    device_radius,
                )
                .context("Rebuilding rounded backdrop after device lost")?,
            )
        } else {
            None
        };

        self.atlas
            .handle_device_lost(&devices.device, &devices.device_context);

        unsafe {
            devices
                .device_context
                .OMSetRenderTargets(Some(slice::from_ref(&resources.render_target_view)), None);
        }
        self.devices = Some(devices);
        self.resources = Some(resources);
        self.globals = globals;
        self.pipelines = pipelines;
        self.direct_composition = direct_composition;
        self.rounded_backdrop = rounded_backdrop;
        self.skip_draws = true;
        Ok(())
    }

    pub(crate) fn draw(
        &mut self,
        scene: &Scene,
        background_appearance: WindowBackgroundAppearance,
    ) -> Result<()> {
        if self.skip_draws {
            // skip drawing this frame, we just recovered from a device lost event
            // and so likely do not have the textures anymore that are required for drawing
            return Ok(());
        }
        let viewport_size = size(
            DevicePixels(self.width as i32),
            DevicePixels(self.height as i32),
        );
        if !scene
            .backdrop_blurs
            .iter()
            .any(|blur| backdrop_source_bounds(blur, viewport_size).is_some())
        {
            if let Some(resources) = self.resources.as_mut() {
                resources.discard_backdrop_resources();
            }
        }
        if self.update_clean_retained_layers(scene)? {
            return Ok(());
        }
        self.pre_draw(&match background_appearance {
            WindowBackgroundAppearance::Opaque => [1.0f32; 4],
            _ => [0.0f32; 4],
        })?;

        self.upload_scene_buffers(scene)?;

        for batch in scene.batches() {
            match batch {
                PrimitiveBatch::Shadows(range) => self.draw_shadows(range.start, range.len()),
                PrimitiveBatch::BackdropBlurs(range) => {
                    let blurs = &scene.backdrop_blurs[range];
                    if blurs.is_empty() {
                        Ok(())
                    } else {
                        let viewport_size = size(
                            DevicePixels(self.width as i32),
                            DevicePixels(self.height as i32),
                        );
                        for blurs in backdrop_blur_clusters(blurs, viewport_size) {
                            let Some(mut scratch_bounds) =
                                backdrop_scratch_bounds(&blurs, viewport_size)
                            else {
                                continue;
                            };
                            {
                                let devices = self.devices.as_ref().context("devices missing")?;
                                if let Some(texture_size) = self
                                    .resources
                                    .as_mut()
                                    .context("resources missing")?
                                    .ensure_backdrop_resources(
                                        devices,
                                        scratch_bounds.texture_size,
                                        max_backdrop_texture_size(scratch_bounds, viewport_size),
                                    )?
                                {
                                    scratch_bounds = fit_backdrop_scratch_bounds(
                                        scratch_bounds,
                                        texture_size,
                                        viewport_size,
                                    );
                                }
                            }
                            let prepared_blurs = prepare_backdrop_blurs(&blurs, scratch_bounds);
                            self.copy_render_target_to_backdrop(scratch_bounds)?;
                            let max_backdrop_blur_levels = self.max_backdrop_blur_levels();
                            let mut current_plan = None;
                            let mut current_blur_srv = None;
                            for (start, end, plan) in backdrop_blur_plan_groups(
                                &blurs,
                                max_backdrop_blur_levels,
                            ) {
                                if plan.passes == 0 {
                                    continue;
                                }
                                if current_plan != Some(plan) {
                                    current_blur_srv =
                                        self.run_backdrop_blur_passes(plan)?;
                                    current_plan = Some(plan);
                                }
                                self.draw_backdrop_blurs(
                                    &prepared_blurs[start..end],
                                    &current_blur_srv,
                                )?;
                            }
                        }
                        Ok(())
                    }
                }
                PrimitiveBatch::Quads(range) => self.draw_quads(range.start, range.len()),
                PrimitiveBatch::Paths(range) => {
                    let paths = &scene.paths[range];
                    if paths.is_empty() {
                        return Ok(());
                    }
                    let path_ranges: Vec<_> =
                        (0..paths.len()).map(|index| index..index + 1).collect();

                    for path_range in path_ranges {
                        let paths = &paths[path_range];
                        if let Some(mut scratch_bounds) =
                            Self::path_scratch_bounds(paths, self.width, self.height)
                        {
                            let devices = self.devices.as_ref().context("devices missing")?;
                            let texture_size = self
                                .resources
                                .as_mut()
                                .context("resources missing")?
                                .ensure_path_intermediate_resources(
                                    devices,
                                    scratch_bounds.texture_size,
                                )?;
                            let Some(texture_size) = texture_size else {
                                return Ok(());
                            };
                            scratch_bounds.texture_size = texture_size;
                            self.draw_paths_to_intermediate(paths, scratch_bounds)?;
                            self.draw_paths_from_intermediate(paths, scratch_bounds)?;
                        }
                    }
                    Ok(())
                }
                PrimitiveBatch::Underlines(range) => self.draw_underlines(range.start, range.len()),
                PrimitiveBatch::MonochromeSprites { texture_id, range } => {
                    self.draw_monochrome_sprites(texture_id, range.start, range.len())
                }
                PrimitiveBatch::SubpixelSprites { texture_id, range } => {
                    self.draw_subpixel_sprites(texture_id, range.start, range.len())
                }
                PrimitiveBatch::PolychromeSprites { texture_id, range } => {
                    self.draw_polychrome_sprites(texture_id, range.start, range.len())
                }
                PrimitiveBatch::Surfaces(range) => self.draw_surfaces(&scene.surfaces[range]),
            }
            .context(format!(
                "scene too large:\
                {} paths, {} shadows, {} blurs, {} quads, {} underlines, {} mono, {} subpixel, {} poly, {} surfaces",
                scene.paths.len(),
                scene.shadows.len(),
                scene.backdrop_blurs.len(),
                scene.quads.len(),
                scene.underlines.len(),
                scene.monochrome_sprites.len(),
                scene.subpixel_sprites.len(),
                scene.polychrome_sprites.len(),
                scene.surfaces.len(),
            ))?;
        }
        self.present()?;
        self.update_retained_layer_cache(scene)
    }

    pub(crate) fn resize(&mut self, new_size: Size<DevicePixels>) -> Result<()> {
        let width = new_size.width.0.max(1) as u32;
        let height = new_size.height.0.max(1) as u32;
        if self.width == width && self.height == height {
            return Ok(());
        }
        self.width = width;
        self.height = height;

        // Clear the render target before resizing
        let devices = self.devices.as_ref().context("devices missing")?;
        unsafe { devices.device_context.OMSetRenderTargets(None, None) };
        let resources = self.resources.as_mut().context("resources missing")?;
        resources.render_target.take();
        resources.render_target_view.take();

        // Resizing the swap chain requires a call to the underlying DXGI adapter, which can return the device removed error.
        // The app might have moved to a monitor that's attached to a different graphics device.
        // When a graphics device is removed or reset, the desktop resolution often changes, resulting in a window size change.
        // But here we just return the error, because we are handling device lost scenarios elsewhere.
        unsafe {
            resources
                .swap_chain
                .ResizeBuffers(
                    BUFFER_COUNT as u32,
                    width,
                    height,
                    RENDER_TARGET_FORMAT,
                    DXGI_SWAP_CHAIN_FLAG(0),
                )
                .context("Failed to resize swap chain")?;
        }

        resources.recreate_resources(devices, width, height)?;

        unsafe {
            devices
                .device_context
                .OMSetRenderTargets(Some(slice::from_ref(&resources.render_target_view)), None);
        }

        Ok(())
    }

    pub(crate) fn update_transparency(&mut self, transparent: bool) {
        if self.disable_direct_composition {
            return;
        }

        // The rounded backdrop mode already composes the swap chain with per-pixel
        // alpha through its own Windows.UI.Composition target; never create a
        // competing DirectComposition target for the same HWND.
        if self.rounded_backdrop.is_some() {
            return;
        }
        if !transparent || self.direct_composition.is_some() {
            return;
        }

        self.enable_direct_composition_for_transparency()
            .context("Failed to enable transparency in DirectXRenderer")
            .log_err();
    }

    /// Enables or disables the rounded host-backdrop blur mode.
    ///
    /// `Some(logical_radius)` builds (or updates) a Windows.UI.Composition tree
    /// for this HWND: a backdrop `SpriteVisual` painted with a host-backdrop
    /// brush and an antialiased rounded-rectangle clip, plus a content
    /// `SpriteVisual` whose brush is a composition surface wrapping the existing
    /// swap chain (clipped to the same rounded rect). Because only one
    /// composition target may exist per HWND, the legacy `DirectComposition`
    /// target is torn down while this mode is active.
    ///
    /// `None` tears the tree down and restores the plain `DirectComposition`
    /// transparency path (if it was previously in use).
    pub(crate) fn set_rounded_backdrop_blur(
        &mut self,
        logical_radius: Option<f32>,
        scale_factor: f32,
    ) -> Result<()> {
        match logical_radius {
            Some(radius) => {
                if self.disable_direct_composition {
                    anyhow::bail!("rounded backdrop blur requires DirectComposition");
                }

                self.rounded_backdrop_radius = Some(radius);
                self.rounded_backdrop_scale = scale_factor;
                let device_radius = radius * scale_factor;

                // Only one composition target per HWND: drop the DComp target.
                self.direct_composition.take();

                if let Some(backdrop) = self.rounded_backdrop.as_ref() {
                    backdrop.update_geometry(self.width, self.height, device_radius)?;
                } else {
                    let swap_chain = self
                        .resources
                        .as_ref()
                        .context("resources missing")?
                        .swap_chain
                        .clone();
                    let backdrop = RoundedBackdrop::new(
                        self.hwnd,
                        &swap_chain,
                        self.width,
                        self.height,
                        device_radius,
                    )
                    .context("Creating rounded backdrop")?;
                    self.rounded_backdrop = Some(backdrop);
                }
            }
            None => {
                self.rounded_backdrop_radius = None;
                if self.rounded_backdrop.take().is_some() && !self.disable_direct_composition {
                    // We tore down the DComp target when entering rounded mode;
                    // rebuild it so plain transparency keeps working.
                    self.rebuild_direct_composition()
                        .context("Restoring DirectComposition after rounded backdrop")?;
                }
            }
        }
        Ok(())
    }

    /// Recomputes the rounded backdrop clip geometry for the current device size
    /// and scale factor. Called on resize / DPI change. No-op unless rounded
    /// mode is active.
    pub(crate) fn update_rounded_backdrop(&mut self, scale_factor: f32) -> Result<()> {
        let Some(radius) = self.rounded_backdrop_radius else {
            return Ok(());
        };
        self.rounded_backdrop_scale = scale_factor;
        let device_radius = radius * scale_factor;
        let (width, height) = (self.width, self.height);
        if let Some(backdrop) = self.rounded_backdrop.as_ref() {
            backdrop.update_geometry(width, height, device_radius)?;
        }
        Ok(())
    }

    fn rebuild_direct_composition(&mut self) -> Result<()> {
        let devices = self.devices.as_ref().context("devices missing")?;
        let dxgi_device = devices
            .dxgi_device
            .as_ref()
            .context("dxgi device missing for DirectComposition")?;
        let swap_chain = self
            .resources
            .as_ref()
            .context("resources missing")?
            .swap_chain
            .clone();
        let mut composition =
            DirectComposition::new(dxgi_device, self.hwnd).context("Creating DirectComposition")?;
        composition
            .set_swap_chain(&swap_chain)
            .context("Setting swap chain for DirectComposition")?;
        self.direct_composition = Some(composition);
        Ok(())
    }

    fn enable_direct_composition_for_transparency(&mut self) -> Result<()> {
        let devices = self.devices.as_ref().context("devices missing")?;
        let dxgi_device = devices
            .dxgi_device
            .as_ref()
            .context("dxgi device missing for DirectComposition")?;
        let width = self.width.max(1);
        let height = self.height.max(1);

        unsafe { devices.device_context.OMSetRenderTargets(None, None) };

        let swap_chain = create_swap_chain_for_composition(
            &devices.dxgi_factory,
            &devices.device,
            width,
            height,
        )
        .context("Failed to create composition swap chain")?;
        let mut direct_composition =
            DirectComposition::new(dxgi_device, self.hwnd).context("Creating DirectComposition")?;
        direct_composition
            .set_swap_chain(&swap_chain)
            .context("Setting swap chain for DirectComposition")?;

        let resources = self.resources.as_mut().context("resources missing")?;
        resources.render_target.take();
        resources.render_target_view.take();
        resources.swap_chain = swap_chain;
        resources
            .recreate_resources(devices, width, height)
            .context("Recreating DirectX resources for transparency")?;

        unsafe {
            devices
                .device_context
                .OMSetRenderTargets(Some(slice::from_ref(&resources.render_target_view)), None);
        }

        self.direct_composition = Some(direct_composition);
        Ok(())
    }

    fn upload_scene_buffers(&mut self, scene: &Scene) -> Result<()> {
        let devices = self.devices.as_ref().context("devices missing")?;

        if !scene.shadows.is_empty() {
            self.pipelines.shadow_pipeline.update_buffer(
                &devices.device,
                &devices.device_context,
                &scene.shadows,
            )?;
        }

        if !scene.quads.is_empty() {
            self.pipelines.quad_pipeline.update_buffer(
                &devices.device,
                &devices.device_context,
                &scene.quads,
            )?;
        }

        if !scene.underlines.is_empty() {
            self.pipelines.underline_pipeline.update_buffer(
                &devices.device,
                &devices.device_context,
                &scene.underlines,
            )?;
        }

        if !scene.monochrome_sprites.is_empty() {
            self.pipelines.mono_sprites.update_buffer(
                &devices.device,
                &devices.device_context,
                &scene.monochrome_sprites,
            )?;
        }

        if !scene.subpixel_sprites.is_empty() {
            self.pipelines.subpixel_sprites.update_buffer(
                &devices.device,
                &devices.device_context,
                &scene.subpixel_sprites,
            )?;
        }

        if !scene.polychrome_sprites.is_empty() {
            self.pipelines.poly_sprites.update_buffer(
                &devices.device,
                &devices.device_context,
                &scene.polychrome_sprites,
            )?;
        }

        Ok(())
    }

    fn draw_shadows(&mut self, start: usize, len: usize) -> Result<()> {
        if len == 0 {
            return Ok(());
        }
        let devices = self.devices.as_ref().context("devices missing")?;
        self.pipelines.shadow_pipeline.draw_range(
            &devices.device,
            &devices.device_context,
            slice::from_ref(
                &self
                    .resources
                    .as_ref()
                    .context("resources missing")?
                    .viewport,
            ),
            slice::from_ref(&self.globals.global_params_buffer),
            4,
            start as u32,
            len as u32,
        )
    }

    fn copy_render_target_to_backdrop(
        &mut self,
        scratch_bounds: BackdropScratchBounds,
    ) -> Result<()> {
        let devices = self.devices.as_ref().context("devices missing")?;
        let resources = self.resources.as_ref().context("resources missing")?;
        let render_target = resources
            .render_target
            .as_ref()
            .context("render target missing")?;
        let backdrop_texture = resources
            .backdrop_texture
            .as_ref()
            .context("backdrop texture missing")?;
        let source_box = D3D11_BOX {
            left: scratch_bounds.bounds.origin.x.0 as u32,
            top: scratch_bounds.bounds.origin.y.0 as u32,
            front: 0,
            right: (scratch_bounds.bounds.origin.x.0 as u32)
                + scratch_bounds.texture_size.width.0 as u32,
            bottom: (scratch_bounds.bounds.origin.y.0 as u32)
                + scratch_bounds.texture_size.height.0 as u32,
            back: 1,
        };
        unsafe {
            devices.device_context.OMSetRenderTargets(None, None);
            devices.device_context.CopySubresourceRegion(
                backdrop_texture,
                0,
                0,
                0,
                0,
                render_target,
                0,
                Some(&source_box),
            );
            if let Some(ref render_target_view) = resources.render_target_view {
                devices
                    .device_context
                    .OMSetRenderTargets(Some(&[Some(render_target_view.clone())]), None);
            }
        }
        Ok(())
    }

    fn max_backdrop_blur_levels(&self) -> usize {
        self.resources
            .as_ref()
            .and_then(|resources| resources.backdrop_blur.as_ref())
            .map(|blur| blur.level_sizes.len().saturating_sub(1))
            .unwrap_or(0)
            .min(MAX_BACKDROP_BLUR_LEVELS)
    }

    fn draw_backdrop_blurs(
        &mut self,
        blurs: &[BackdropBlur],
        source_srv: &Option<ID3D11ShaderResourceView>,
    ) -> Result<()> {
        if blurs.is_empty() {
            return Ok(());
        }
        let devices = self.devices.as_ref().context("devices missing")?;
        let resources = self.resources.as_ref().context("resources missing")?;
        self.pipelines.backdrop_blur_pipeline.update_buffer(
            &devices.device,
            &devices.device_context,
            blurs,
        )?;
        self.pipelines
            .backdrop_blur_pipeline
            .draw_range_with_texture(
                &devices.device,
                &devices.device_context,
                slice::from_ref(source_srv),
                slice::from_ref(&resources.viewport),
                slice::from_ref(&self.globals.global_params_buffer),
                slice::from_ref(&self.globals.blur_sampler),
                0,
                blurs.len() as u32,
            )
    }

    fn run_backdrop_blur_passes(
        &mut self,
        plan: BackdropBlurPlan,
    ) -> Result<Option<ID3D11ShaderResourceView>> {
        let devices = self.devices.as_ref().context("devices missing")?;
        let resources = self.resources.as_ref().context("resources missing")?;
        let backdrop_blur = resources
            .backdrop_blur
            .as_ref()
            .context("backdrop blur resources missing")?;
        if plan.passes == 0 {
            return Ok(resources.backdrop_srv.clone());
        }
        if backdrop_blur.downsample_srvs.len() < plan.passes
            || backdrop_blur.upsample_srvs.is_empty()
        {
            return Ok(resources.backdrop_srv.clone());
        }

        let mut input_srv = resources.backdrop_srv.clone();
        for level in 0..plan.passes {
            let input_size = backdrop_blur.level_sizes[level];
            let output_size = backdrop_blur.level_sizes[level + 1];
            let output_view = backdrop_blur.downsample_views[level].as_ref();
            let viewport = D3D11_VIEWPORT {
                TopLeftX: 0.0,
                TopLeftY: 0.0,
                Width: output_size.0 as f32,
                Height: output_size.1 as f32,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            };
            let params = BackdropBlurParams {
                input_size: [input_size.0 as f32, input_size.1 as f32],
                sample_distance: plan.sample_distance,
                pad: 0.0,
            };
            Self::draw_backdrop_blur_pass(
                &devices.device,
                &devices.device_context,
                &mut self.pipelines.backdrop_blur_downsample_pipeline,
                &self.globals,
                &input_srv,
                output_view,
                &viewport,
                params,
            )?;
            input_srv = backdrop_blur.downsample_srvs[level].clone();
        }

        let mut input_srv = backdrop_blur.downsample_srvs[plan.passes - 1].clone();
        for level in (0..plan.passes).rev() {
            let input_size = backdrop_blur.level_sizes[level + 1];
            let output_size = backdrop_blur.level_sizes[level];
            let output_view = backdrop_blur.upsample_views[level].as_ref();
            let viewport = D3D11_VIEWPORT {
                TopLeftX: 0.0,
                TopLeftY: 0.0,
                Width: output_size.0 as f32,
                Height: output_size.1 as f32,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            };
            let params = BackdropBlurParams {
                input_size: [input_size.0 as f32, input_size.1 as f32],
                sample_distance: plan.sample_distance,
                pad: 0.0,
            };
            Self::draw_backdrop_blur_pass(
                &devices.device,
                &devices.device_context,
                &mut self.pipelines.backdrop_blur_upsample_pipeline,
                &self.globals,
                &input_srv,
                output_view,
                &viewport,
                params,
            )?;
            input_srv = backdrop_blur.upsample_srvs[level].clone();
        }

        unsafe {
            if let Some(ref render_target_view) = resources.render_target_view {
                devices
                    .device_context
                    .OMSetRenderTargets(Some(&[Some(render_target_view.clone())]), None);
            }
            devices
                .device_context
                .RSSetViewports(Some(slice::from_ref(&resources.viewport)));
        }

        Ok(backdrop_blur.upsample_srvs.first().cloned().unwrap_or(None))
    }

    fn draw_backdrop_blur_pass(
        device: &ID3D11Device,
        device_context: &ID3D11DeviceContext,
        pipeline: &mut PipelineState<BackdropBlurParams>,
        globals: &DirectXGlobalElements,
        input_srv: &Option<ID3D11ShaderResourceView>,
        output_view: Option<&ID3D11RenderTargetView>,
        viewport: &D3D11_VIEWPORT,
        params: BackdropBlurParams,
    ) -> Result<()> {
        pipeline.update_buffer(device, device_context, &[params])?;
        unsafe {
            if let Some(view) = output_view {
                device_context.ClearRenderTargetView(view, &[0.0; 4]);
                device_context.OMSetRenderTargets(Some(&[Some(view.clone())]), None);
            } else {
                device_context.OMSetRenderTargets(None, None);
            }
            device_context.RSSetViewports(Some(slice::from_ref(viewport)));
        }
        pipeline.draw_with_texture(
            device_context,
            slice::from_ref(input_srv),
            slice::from_ref(viewport),
            slice::from_ref(&globals.global_params_buffer),
            slice::from_ref(&globals.blur_sampler),
            1,
        )
    }

    fn draw_quads(&mut self, start: usize, len: usize) -> Result<()> {
        if len == 0 {
            return Ok(());
        }
        let devices = self.devices.as_ref().context("devices missing")?;
        self.pipelines.quad_pipeline.draw_range(
            &devices.device,
            &devices.device_context,
            slice::from_ref(
                &self
                    .resources
                    .as_ref()
                    .context("resources missing")?
                    .viewport,
            ),
            slice::from_ref(&self.globals.global_params_buffer),
            4,
            start as u32,
            len as u32,
        )
    }

    fn path_scratch_bounds(
        paths: &[Path<ScaledPixels>],
        width: u32,
        height: u32,
    ) -> Option<PathScratchBounds> {
        let mut bounds = paths.first()?.clipped_bounds();
        for path in paths.iter().skip(1) {
            bounds = bounds.union(&path.clipped_bounds());
        }

        let viewport_bounds = Bounds {
            origin: point(ScaledPixels(0.0), ScaledPixels(0.0)),
            size: size(
                ScaledPixels::from(DevicePixels(width as i32)),
                ScaledPixels::from(DevicePixels(height as i32)),
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
        &mut self,
        paths: &[Path<ScaledPixels>],
        scratch_bounds: PathScratchBounds,
    ) -> Result<()> {
        if paths.is_empty() {
            return Ok(());
        }

        let devices = self.devices.as_ref().context("devices missing")?;
        let resources = self.resources.as_ref().context("resources missing")?;
        let path_intermediate_msaa_view = resources
            .path_intermediate_msaa_view
            .as_ref()
            .context("path intermediate MSAA view missing")?;
        let path_intermediate_texture = resources
            .path_intermediate_texture
            .as_ref()
            .context("path intermediate texture missing")?;
        let path_intermediate_msaa_texture = resources
            .path_intermediate_msaa_texture
            .as_ref()
            .context("path intermediate MSAA texture missing")?;
        // Clear intermediate MSAA texture
        unsafe {
            devices
                .device_context
                .ClearRenderTargetView(path_intermediate_msaa_view, &[0.0; 4]);
            // Set intermediate MSAA texture as render target
            devices
                .device_context
                .OMSetRenderTargets(Some(&[Some(path_intermediate_msaa_view.clone())]), None);
        }

        // Collect all vertices and sprites for a single draw call
        let mut vertices = Vec::new();

        for path in paths {
            vertices.extend(path.vertices.iter().map(|v| PathRasterizationSprite {
                xy_position: v.xy_position,
                st_position: v.st_position,
                color: path.color,
                bounds: path.clipped_bounds(),
                content_mask: path.content_mask.clone(),
                scratch_bounds: scratch_bounds.bounds,
                texture_size: [
                    scratch_bounds.texture_size.width.0 as f32,
                    scratch_bounds.texture_size.height.0 as f32,
                ],
            }));
        }

        self.pipelines.path_rasterization_pipeline.update_buffer(
            &devices.device,
            &devices.device_context,
            &vertices,
        )?;

        let scratch_viewport = D3D11_VIEWPORT {
            Width: scratch_bounds.texture_size.width.0 as f32,
            Height: scratch_bounds.texture_size.height.0 as f32,
            MaxDepth: 1.0,
            ..Default::default()
        };
        self.pipelines.path_rasterization_pipeline.draw(
            &devices.device_context,
            slice::from_ref(&scratch_viewport),
            slice::from_ref(&self.globals.global_params_buffer),
            D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST,
            vertices.len() as u32,
            1,
        )?;

        // Resolve MSAA to non-MSAA intermediate texture
        unsafe {
            devices.device_context.ResolveSubresource(
                path_intermediate_texture,
                0,
                path_intermediate_msaa_texture,
                0,
                RENDER_TARGET_FORMAT,
            );
            // Restore main render target
            devices
                .device_context
                .OMSetRenderTargets(Some(slice::from_ref(&resources.render_target_view)), None);
        }

        Ok(())
    }

    fn draw_paths_from_intermediate(
        &mut self,
        paths: &[Path<ScaledPixels>],
        scratch_bounds: PathScratchBounds,
    ) -> Result<()> {
        let Some(first_path) = paths.first() else {
            return Ok(());
        };

        // When copying paths from the intermediate texture to the drawable,
        // each pixel must only be copied once, in case of transparent paths.
        //
        // If all paths have the same draw order, then their bounds are all
        // disjoint, so we can copy each path's bounds individually. If this
        // batch combines different draw orders, we perform a single copy
        // for a minimal spanning rect.
        let sprites = if paths.last().unwrap().order == first_path.order {
            paths
                .iter()
                .map(|path| PathSprite {
                    bounds: path.clipped_bounds(),
                    scratch_bounds: scratch_bounds.bounds,
                    texture_size: [
                        scratch_bounds.texture_size.width.0 as f32,
                        scratch_bounds.texture_size.height.0 as f32,
                    ],
                })
                .collect::<Vec<_>>()
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

        let devices = self.devices.as_ref().context("devices missing")?;
        let resources = self.resources.as_ref().context("resources missing")?;
        let path_intermediate_srv = resources
            .path_intermediate_srv
            .as_ref()
            .context("path intermediate shader resource missing")?;
        self.pipelines.path_sprite_pipeline.update_buffer(
            &devices.device,
            &devices.device_context,
            &sprites,
        )?;

        // Draw the sprites with the path texture
        self.pipelines.path_sprite_pipeline.draw_with_texture(
            &devices.device_context,
            &[Some(path_intermediate_srv.clone())],
            slice::from_ref(&resources.viewport),
            slice::from_ref(&self.globals.global_params_buffer),
            slice::from_ref(&self.globals.sampler),
            sprites.len() as u32,
        )
    }

    fn draw_underlines(&mut self, start: usize, len: usize) -> Result<()> {
        if len == 0 {
            return Ok(());
        }
        let devices = self.devices.as_ref().context("devices missing")?;
        let resources = self.resources.as_ref().context("resources missing")?;
        self.pipelines.underline_pipeline.draw_range(
            &devices.device,
            &devices.device_context,
            slice::from_ref(&resources.viewport),
            slice::from_ref(&self.globals.global_params_buffer),
            4,
            start as u32,
            len as u32,
        )
    }

    fn draw_monochrome_sprites(
        &mut self,
        texture_id: AtlasTextureId,
        start: usize,
        len: usize,
    ) -> Result<()> {
        if len == 0 {
            return Ok(());
        }
        let devices = self.devices.as_ref().context("devices missing")?;
        let resources = self.resources.as_ref().context("resources missing")?;
        let texture_view = self.atlas.get_texture_view(texture_id);
        self.pipelines.mono_sprites.draw_range_with_texture(
            &devices.device,
            &devices.device_context,
            &texture_view,
            slice::from_ref(&resources.viewport),
            slice::from_ref(&self.globals.global_params_buffer),
            slice::from_ref(&self.globals.sampler),
            start as u32,
            len as u32,
        )
    }

    fn draw_subpixel_sprites(
        &mut self,
        texture_id: AtlasTextureId,
        start: usize,
        len: usize,
    ) -> Result<()> {
        if len == 0 {
            return Ok(());
        }
        let devices = self.devices.as_ref().context("devices missing")?;
        let resources = self.resources.as_ref().context("resources missing")?;
        let texture_view = self.atlas.get_texture_view(texture_id);
        self.pipelines.subpixel_sprites.draw_range_with_texture(
            &devices.device,
            &devices.device_context,
            &texture_view,
            slice::from_ref(&resources.viewport),
            slice::from_ref(&self.globals.global_params_buffer),
            slice::from_ref(&self.globals.sampler),
            start as u32,
            len as u32,
        )
    }

    fn draw_polychrome_sprites(
        &mut self,
        texture_id: AtlasTextureId,
        start: usize,
        len: usize,
    ) -> Result<()> {
        if len == 0 {
            return Ok(());
        }
        let devices = self.devices.as_ref().context("devices missing")?;
        let resources = self.resources.as_ref().context("resources missing")?;
        let texture_view = self.atlas.get_texture_view(texture_id);
        self.pipelines.poly_sprites.draw_range_with_texture(
            &devices.device,
            &devices.device_context,
            &texture_view,
            slice::from_ref(&resources.viewport),
            slice::from_ref(&self.globals.global_params_buffer),
            slice::from_ref(&self.globals.sampler),
            start as u32,
            len as u32,
        )
    }

    fn draw_surfaces(&mut self, surfaces: &[PaintSurface]) -> Result<()> {
        if surfaces.is_empty() {
            return Ok(());
        }
        Ok(())
    }

    pub(crate) fn renderer_info(&self) -> Result<RendererInfo> {
        let devices = self.devices.as_ref().context("devices missing")?;
        let desc = unsafe { devices.adapter.GetDesc1() }?;
        let adapter_type = if (desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32) != 0 {
            RendererAdapterType::Software
        } else {
            RendererAdapterType::Hardware
        };
        Ok(RendererInfo {
            selection: self.renderer_selection,
            renderer: RendererKind::Direct3d11,
            backend: "Direct3D11".to_string(),
            adapter_name: String::from_utf16_lossy(&desc.Description)
                .trim_matches(char::from(0))
                .to_string(),
            adapter_type,
            vendor_id: Some(desc.VendorId),
            device_id: Some(desc.DeviceId),
        })
    }

    pub(crate) fn gpu_specs(&self) -> Result<GpuSpecs> {
        let devices = self.devices.as_ref().context("devices missing")?;
        let desc = unsafe { devices.adapter.GetDesc1() }?;
        let is_software_emulated = (desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32) != 0;
        let device_name = String::from_utf16_lossy(&desc.Description)
            .trim_matches(char::from(0))
            .to_string();
        let driver_name = match desc.VendorId {
            0x10DE => "NVIDIA Corporation".to_string(),
            0x1002 => "AMD Corporation".to_string(),
            0x8086 => "Intel Corporation".to_string(),
            id => format!("Unknown Vendor (ID: {:#X})", id),
        };
        let driver_version = match desc.VendorId {
            0x10DE => nvidia::get_driver_version(),
            0x1002 => amd::get_driver_version(),
            // For Intel and other vendors, we use the DXGI API to get the driver version.
            _ => dxgi::get_driver_version(&devices.adapter),
        }
        .context("Failed to get gpu driver info")
        .log_err()
        .unwrap_or("Unknown Driver".to_string());
        Ok(GpuSpecs {
            is_software_emulated,
            device_name,
            driver_name,
            driver_info: driver_version,
        })
    }

    pub(crate) fn get_font_info() -> &'static FontInfo {
        static CACHED_FONT_INFO: OnceLock<FontInfo> = OnceLock::new();
        CACHED_FONT_INFO.get_or_init(|| unsafe {
            let factory: IDWriteFactory5 = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED).unwrap();
            let render_params: IDWriteRenderingParams1 =
                factory.CreateRenderingParams().unwrap().cast().unwrap();
            FontInfo {
                gamma_ratios: gpui::get_gamma_correction_ratios(render_params.GetGamma()),
                grayscale_enhanced_contrast: render_params.GetGrayscaleEnhancedContrast(),
                subpixel_enhanced_contrast: render_params.GetEnhancedContrast(),
                is_bgr: render_params.GetPixelGeometry() == DWRITE_PIXEL_GEOMETRY_BGR,
            }
        })
    }

    pub(crate) fn mark_drawable(&mut self) {
        self.skip_draws = false;
    }
}

impl DirectXResources {
    pub fn new(
        devices: &DirectXRendererDevices,
        width: u32,
        height: u32,
        hwnd: HWND,
        disable_direct_composition: bool,
    ) -> Result<Self> {
        let swap_chain = if disable_direct_composition {
            create_swap_chain(&devices.dxgi_factory, &devices.device, hwnd, width, height)?
        } else {
            create_swap_chain_for_composition(
                &devices.dxgi_factory,
                &devices.device,
                width,
                height,
            )?
        };

        let (render_target, render_target_view, viewport) =
            create_resources(devices, &swap_chain, width, height)?;
        set_rasterizer_state(&devices.device, &devices.device_context)?;

        Ok(Self {
            swap_chain,
            render_target: Some(render_target),
            render_target_view,
            path_intermediate_texture: None,
            path_intermediate_msaa_texture: None,
            path_intermediate_msaa_view: None,
            path_intermediate_srv: None,
            path_intermediate_size: None,
            backdrop_texture: None,
            backdrop_srv: None,
            backdrop_size: None,
            backdrop_blur: None,
            viewport,
        })
    }

    #[inline]
    fn recreate_resources(
        &mut self,
        devices: &DirectXRendererDevices,
        width: u32,
        height: u32,
    ) -> Result<()> {
        let (render_target, render_target_view, viewport) =
            create_resources(devices, &self.swap_chain, width, height)?;
        self.render_target = Some(render_target);
        self.render_target_view = render_target_view;
        self.discard_path_intermediate_resources();
        self.discard_backdrop_resources();
        self.viewport = viewport;
        Ok(())
    }

    fn discard_path_intermediate_resources(&mut self) {
        self.path_intermediate_texture = None;
        self.path_intermediate_srv = None;
        self.path_intermediate_msaa_texture = None;
        self.path_intermediate_msaa_view = None;
        self.path_intermediate_size = None;
    }

    fn discard_backdrop_resources(&mut self) {
        self.backdrop_texture = None;
        self.backdrop_srv = None;
        self.backdrop_size = None;
        self.backdrop_blur = None;
    }

    fn ensure_path_intermediate_resources(
        &mut self,
        devices: &DirectXRendererDevices,
        size: Size<DevicePixels>,
    ) -> Result<Option<Size<DevicePixels>>> {
        if let Some(current_size) = self.path_intermediate_size {
            if current_size.width >= size.width && current_size.height >= size.height {
                return Ok(Some(current_size));
            }
            return self.create_path_intermediate_resources(
                devices,
                Size {
                    width: current_size.width.max(size.width),
                    height: current_size.height.max(size.height),
                },
            );
        }

        self.create_path_intermediate_resources(devices, size)
    }

    fn create_path_intermediate_resources(
        &mut self,
        devices: &DirectXRendererDevices,
        size: Size<DevicePixels>,
    ) -> Result<Option<Size<DevicePixels>>> {
        if size.width.0 <= 0 || size.height.0 <= 0 {
            self.discard_path_intermediate_resources();
            return Ok(None);
        }

        self.discard_path_intermediate_resources();
        let width = size.width.0 as u32;
        let height = size.height.0 as u32;
        let (path_intermediate_texture, path_intermediate_srv) =
            create_path_intermediate_texture(&devices.device, width, height)?;
        let (path_intermediate_msaa_texture, path_intermediate_msaa_view) =
            create_path_intermediate_msaa_texture_and_view(&devices.device, width, height)?;
        self.path_intermediate_texture = Some(path_intermediate_texture);
        self.path_intermediate_srv = path_intermediate_srv;
        self.path_intermediate_msaa_texture = Some(path_intermediate_msaa_texture);
        self.path_intermediate_msaa_view = path_intermediate_msaa_view;
        self.path_intermediate_size = Some(size);
        Ok(Some(size))
    }

    fn ensure_backdrop_resources(
        &mut self,
        devices: &DirectXRendererDevices,
        size: Size<DevicePixels>,
        max_size: Size<DevicePixels>,
    ) -> Result<Option<Size<DevicePixels>>> {
        let size = Self::quantize_backdrop_texture_size(size, max_size);
        if let Some(current_size) = self.backdrop_size
            && self.backdrop_texture.is_some()
        {
            if can_reuse_backdrop_texture(current_size, size) {
                return Ok(Some(current_size));
            }
            return self.create_backdrop_resources(
                devices,
                Size {
                    width: current_size.width.max(size.width).min(max_size.width),
                    height: current_size.height.max(size.height).min(max_size.height),
                },
            );
        }

        self.create_backdrop_resources(devices, size)
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
        devices: &DirectXRendererDevices,
        size: Size<DevicePixels>,
    ) -> Result<Option<Size<DevicePixels>>> {
        if size.width.0 <= 0 || size.height.0 <= 0 {
            self.discard_backdrop_resources();
            return Ok(None);
        }

        self.discard_backdrop_resources();
        let width = size.width.0 as u32;
        let height = size.height.0 as u32;
        let (backdrop_texture, backdrop_srv) =
            create_backdrop_texture_and_srv(&devices.device, width, height)?;
        let backdrop_blur = create_backdrop_blur_resources(&devices.device, width, height)?;
        self.backdrop_texture = Some(backdrop_texture);
        self.backdrop_srv = backdrop_srv;
        self.backdrop_size = Some(size);
        self.backdrop_blur = Some(backdrop_blur);
        Ok(Some(size))
    }
}

impl DirectXRenderPipelines {
    pub fn new(device: &ID3D11Device) -> Result<Self> {
        let shadow_pipeline = PipelineState::new(
            device,
            "shadow_pipeline",
            ShaderModule::Shadow,
            4,
            create_blend_state(device)?,
        )?;
        let backdrop_blur_pipeline = PipelineState::new(
            device,
            "backdrop_blur_pipeline",
            ShaderModule::BackdropBlur,
            4,
            create_blend_state(device)?,
        )?;
        let backdrop_blur_downsample_pipeline = PipelineState::new(
            device,
            "backdrop_blur_downsample_pipeline",
            ShaderModule::BackdropBlurDownsample,
            4,
            create_blend_state_without_blending(device)?,
        )?;
        let backdrop_blur_upsample_pipeline = PipelineState::new(
            device,
            "backdrop_blur_upsample_pipeline",
            ShaderModule::BackdropBlurUpsample,
            4,
            create_blend_state_without_blending(device)?,
        )?;
        let quad_pipeline = PipelineState::new(
            device,
            "quad_pipeline",
            ShaderModule::Quad,
            64,
            create_blend_state(device)?,
        )?;
        let path_rasterization_pipeline = PipelineState::new(
            device,
            "path_rasterization_pipeline",
            ShaderModule::PathRasterization,
            32,
            create_blend_state_for_path_rasterization(device)?,
        )?;
        let path_sprite_pipeline = PipelineState::new(
            device,
            "path_sprite_pipeline",
            ShaderModule::PathSprite,
            4,
            create_blend_state_for_path_sprite(device)?,
        )?;
        let underline_pipeline = PipelineState::new(
            device,
            "underline_pipeline",
            ShaderModule::Underline,
            4,
            create_blend_state(device)?,
        )?;
        let mono_sprites = PipelineState::new(
            device,
            "monochrome_sprite_pipeline",
            ShaderModule::MonochromeSprite,
            512,
            create_blend_state(device)?,
        )?;
        let subpixel_sprites = PipelineState::new(
            device,
            "subpixel_sprite_pipeline",
            ShaderModule::SubpixelSprite,
            512,
            create_blend_state_for_subpixel_rendering(device)?,
        )?;
        let poly_sprites = PipelineState::new(
            device,
            "polychrome_sprite_pipeline",
            ShaderModule::PolychromeSprite,
            16,
            create_blend_state(device)?,
        )?;

        Ok(Self {
            shadow_pipeline,
            backdrop_blur_pipeline,
            backdrop_blur_downsample_pipeline,
            backdrop_blur_upsample_pipeline,
            quad_pipeline,
            path_rasterization_pipeline,
            path_sprite_pipeline,
            underline_pipeline,
            mono_sprites,
            subpixel_sprites,
            poly_sprites,
        })
    }
}

impl DirectComposition {
    pub fn new(dxgi_device: &IDXGIDevice, hwnd: HWND) -> Result<Self> {
        let comp_device = get_comp_device(dxgi_device)?;
        let comp_target = unsafe { comp_device.CreateTargetForHwnd(hwnd, true) }?;
        let comp_visual = unsafe { comp_device.CreateVisual() }?;
        let retained_compositor = RetainedCompositor::new(comp_device.clone(), comp_visual.clone());

        Ok(Self {
            comp_device,
            comp_target,
            comp_visual,
            retained_compositor,
            retained_layers_enabled: false,
        })
    }

    pub fn set_swap_chain(&mut self, swap_chain: &IDXGISwapChain1) -> Result<()> {
        unsafe {
            self.retained_compositor.retain_layers(&[])?;
            self.retained_compositor.set_root_clip(None)?;
            self.comp_visual.SetContent(swap_chain)?;
            self.comp_target.SetRoot(&self.comp_visual)?;
            self.comp_device.Commit()?;
        }
        self.retained_layers_enabled = false;
        Ok(())
    }

    pub fn enable_retained_layers(&mut self) -> Result<()> {
        if self.retained_layers_enabled {
            return Ok(());
        }

        unsafe {
            self.comp_visual.SetContent(None::<&IUnknown>)?;
            self.comp_target.SetRoot(&self.comp_visual)?;
        }
        self.retained_layers_enabled = true;
        Ok(())
    }

    pub fn disable_retained_layers(&mut self, swap_chain: &IDXGISwapChain1) -> Result<()> {
        if !self.retained_layers_enabled {
            return Ok(());
        }

        self.retained_compositor.retain_layers(&[])?;
        self.retained_compositor.set_root_clip(None)?;
        unsafe {
            self.comp_visual.SetContent(swap_chain)?;
            self.comp_target.SetRoot(&self.comp_visual)?;
        }
        self.retained_layers_enabled = false;
        self.commit()
    }

    pub fn commit(&self) -> Result<()> {
        self.retained_compositor.commit()
    }
}

thread_local! {
    /// A `DispatcherQueue` must exist on the current thread before a
    /// `Compositor` can be constructed. We create one lazily per thread and keep
    /// it alive for the life of the thread (windows run their message loop on the
    /// same thread, which pumps the queue).
    static DISPATCHER_QUEUE_CONTROLLER: std::cell::RefCell<Option<DispatcherQueueController>> =
        const { std::cell::RefCell::new(None) };
}

/// Ensures a `DispatcherQueue` exists for the current thread so a `Compositor`
/// can be created. Non-fatal: if a queue already exists (e.g. created elsewhere)
/// the call fails and we proceed, since `Compositor::new` will still succeed.
fn ensure_dispatcher_queue() {
    DISPATCHER_QUEUE_CONTROLLER.with(|cell| {
        let mut cell = cell.borrow_mut();
        if cell.is_some() {
            return;
        }
        let options = DispatcherQueueOptions {
            dwSize: std::mem::size_of::<DispatcherQueueOptions>() as u32,
            threadType: DQTYPE_THREAD_CURRENT,
            apartmentType: DQTAT_COM_NONE,
        };
        match unsafe { CreateDispatcherQueueController(options) } {
            Ok(controller) => *cell = Some(controller),
            Err(e) => {
                log::warn!(
                    "CreateDispatcherQueueController failed (a queue may already exist): {e}"
                )
            }
        }
    });
}

/// A Windows.UI.Composition tree that hosts the swap chain content above a
/// host-backdrop blur, both clipped to an antialiased rounded rectangle.
struct RoundedBackdrop {
    // Held to keep the composition tree alive; not otherwise read.
    _compositor: Compositor,
    _target: DesktopWindowTarget,
    _root: ContainerVisual,
    _backdrop_visual: SpriteVisual,
    _content_visual: SpriteVisual,
    geometry: CompositionRoundedRectangleGeometry,
}

impl RoundedBackdrop {
    fn new(
        hwnd: HWND,
        swap_chain: &IDXGISwapChain1,
        width: u32,
        height: u32,
        device_radius: f32,
    ) -> Result<Self> {
        ensure_dispatcher_queue();

        let compositor = Compositor::new().context("Creating WinComp Compositor")?;

        let desktop_interop: ICompositorDesktopInterop = compositor
            .cast()
            .context("Casting Compositor to ICompositorDesktopInterop")?;
        let target = unsafe { desktop_interop.CreateDesktopWindowTarget(hwnd, false) }
            .context("Creating DesktopWindowTarget")?;

        let root = compositor
            .CreateContainerVisual()
            .context("Creating root ContainerVisual")?;
        root.SetRelativeSizeAdjustment(Vector2 { X: 1.0, Y: 1.0 })?;
        target.SetRoot(&root)?;

        // Shared rounded-rect geometry in physical pixels (DesktopWindowTarget
        // visuals are not DPI-scaled).
        let geometry = compositor
            .CreateRoundedRectangleGeometry()
            .context("Creating rounded rectangle geometry")?;
        geometry.SetSize(Vector2 {
            X: width as f32,
            Y: height as f32,
        })?;
        geometry.SetCornerRadius(Vector2 {
            X: device_radius,
            Y: device_radius,
        })?;

        let backdrop_clip = compositor
            .CreateGeometricClipWithGeometry(&geometry)
            .context("Creating backdrop geometric clip")?;
        let content_clip = compositor
            .CreateGeometricClipWithGeometry(&geometry)
            .context("Creating content geometric clip")?;

        // Backdrop visual: host-backdrop blur clipped to the rounded rect.
        let backdrop_visual = compositor
            .CreateSpriteVisual()
            .context("Creating backdrop SpriteVisual")?;
        backdrop_visual.SetRelativeSizeAdjustment(Vector2 { X: 1.0, Y: 1.0 })?;
        let host_brush = compositor
            .CreateHostBackdropBrush()
            .context("Creating host backdrop brush")?;
        backdrop_visual.SetBrush(&host_brush)?;
        backdrop_visual.SetClip(&backdrop_clip)?;

        // Content visual: the GPUI swap chain, also clipped to the rounded rect.
        let content_visual = compositor
            .CreateSpriteVisual()
            .context("Creating content SpriteVisual")?;
        content_visual.SetRelativeSizeAdjustment(Vector2 { X: 1.0, Y: 1.0 })?;
        let surface_interop: ICompositorInterop = compositor
            .cast()
            .context("Casting Compositor to ICompositorInterop")?;
        let surface = unsafe { surface_interop.CreateCompositionSurfaceForSwapChain(swap_chain) }
            .context("Creating composition surface for swap chain")?;
        let surface_brush = compositor
            .CreateSurfaceBrushWithSurface(&surface)
            .context("Creating surface brush")?;
        content_visual.SetBrush(&surface_brush)?;
        content_visual.SetClip(&content_clip)?;

        let children = root.Children()?;
        children.InsertAtTop(&backdrop_visual)?;
        children.InsertAtTop(&content_visual)?;

        Ok(Self {
            _compositor: compositor,
            _target: target,
            _root: root,
            _backdrop_visual: backdrop_visual,
            _content_visual: content_visual,
            geometry,
        })
    }

    fn update_geometry(&self, width: u32, height: u32, device_radius: f32) -> Result<()> {
        self.geometry.SetSize(Vector2 {
            X: width as f32,
            Y: height as f32,
        })?;
        self.geometry.SetCornerRadius(Vector2 {
            X: device_radius,
            Y: device_radius,
        })?;
        Ok(())
    }
}

impl DirectXGlobalElements {
    pub fn new(device: &ID3D11Device) -> Result<Self> {
        let global_params_buffer = unsafe {
            let desc = D3D11_BUFFER_DESC {
                ByteWidth: std::mem::size_of::<GlobalParams>() as u32,
                Usage: D3D11_USAGE_DYNAMIC,
                BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
                CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
                ..Default::default()
            };
            let mut buffer = None;
            device.CreateBuffer(&desc, None, Some(&mut buffer))?;
            buffer
        };

        let sampler = unsafe {
            let desc = D3D11_SAMPLER_DESC {
                Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
                AddressU: D3D11_TEXTURE_ADDRESS_WRAP,
                AddressV: D3D11_TEXTURE_ADDRESS_WRAP,
                AddressW: D3D11_TEXTURE_ADDRESS_WRAP,
                MipLODBias: 0.0,
                MaxAnisotropy: 1,
                ComparisonFunc: D3D11_COMPARISON_ALWAYS,
                BorderColor: [0.0; 4],
                MinLOD: 0.0,
                MaxLOD: D3D11_FLOAT32_MAX,
            };
            let mut output = None;
            device.CreateSamplerState(&desc, Some(&mut output))?;
            output
        };

        let blur_sampler = unsafe {
            let desc = D3D11_SAMPLER_DESC {
                Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
                AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
                AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
                AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
                MipLODBias: 0.0,
                MaxAnisotropy: 1,
                ComparisonFunc: D3D11_COMPARISON_ALWAYS,
                BorderColor: [0.0; 4],
                MinLOD: 0.0,
                MaxLOD: D3D11_FLOAT32_MAX,
            };
            let mut output = None;
            device.CreateSamplerState(&desc, Some(&mut output))?;
            output
        };

        Ok(Self {
            global_params_buffer,
            sampler,
            blur_sampler,
        })
    }
}

#[derive(Debug, Default)]
#[repr(C)]
struct GlobalParams {
    gamma_ratios: [f32; 4],
    viewport_size: [f32; 2],
    grayscale_enhanced_contrast: f32,
    subpixel_enhanced_contrast: f32,
    is_bgr: u32,
    _pad: [u32; 3],
}

#[derive(Debug, Default)]
#[repr(C)]
struct BackdropBlurParams {
    input_size: [f32; 2],
    sample_distance: f32,
    pad: f32,
}

struct PipelineState<T> {
    label: &'static str,
    vertex: ID3D11VertexShader,
    fragment: ID3D11PixelShader,
    buffer: ID3D11Buffer,
    buffer_size: usize,
    view: Option<ID3D11ShaderResourceView>,
    blend_state: ID3D11BlendState,
    _marker: std::marker::PhantomData<T>,
}

impl<T> PipelineState<T> {
    fn new(
        device: &ID3D11Device,
        label: &'static str,
        shader_module: ShaderModule,
        buffer_size: usize,
        blend_state: ID3D11BlendState,
    ) -> Result<Self> {
        let vertex = {
            let raw_shader = RawShaderBytes::new(shader_module, ShaderTarget::Vertex)?;
            create_vertex_shader(device, raw_shader.as_bytes())?
        };
        let fragment = {
            let raw_shader = RawShaderBytes::new(shader_module, ShaderTarget::Fragment)?;
            create_fragment_shader(device, raw_shader.as_bytes())?
        };
        let buffer = create_buffer(device, std::mem::size_of::<T>(), buffer_size)?;
        let view = create_buffer_view(device, &buffer)?;

        Ok(PipelineState {
            label,
            vertex,
            fragment,
            buffer,
            buffer_size,
            view,
            blend_state,
            _marker: std::marker::PhantomData,
        })
    }

    fn update_buffer(
        &mut self,
        device: &ID3D11Device,
        device_context: &ID3D11DeviceContext,
        data: &[T],
    ) -> Result<()> {
        if self.buffer_size < data.len() {
            let new_buffer_size = data.len().next_power_of_two();
            log::debug!(
                "Updating {} buffer size from {} to {}",
                self.label,
                self.buffer_size,
                new_buffer_size
            );
            let buffer = create_buffer(device, std::mem::size_of::<T>(), new_buffer_size)?;
            let view = create_buffer_view(device, &buffer)?;
            self.buffer = buffer;
            self.view = view;
            self.buffer_size = new_buffer_size;
        }
        update_buffer(device_context, &self.buffer, data)
    }

    fn draw(
        &self,
        device_context: &ID3D11DeviceContext,
        viewport: &[D3D11_VIEWPORT],
        global_params: &[Option<ID3D11Buffer>],
        topology: D3D_PRIMITIVE_TOPOLOGY,
        vertex_count: u32,
        instance_count: u32,
    ) -> Result<()> {
        set_pipeline_state(
            device_context,
            slice::from_ref(&self.view),
            topology,
            viewport,
            &self.vertex,
            &self.fragment,
            global_params,
            &self.blend_state,
        );
        unsafe {
            device_context.DrawInstanced(vertex_count, instance_count, 0, 0);
        }
        Ok(())
    }

    fn draw_with_texture(
        &self,
        device_context: &ID3D11DeviceContext,
        texture: &[Option<ID3D11ShaderResourceView>],
        viewport: &[D3D11_VIEWPORT],
        global_params: &[Option<ID3D11Buffer>],
        sampler: &[Option<ID3D11SamplerState>],
        instance_count: u32,
    ) -> Result<()> {
        set_pipeline_state(
            device_context,
            slice::from_ref(&self.view),
            D3D_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP,
            viewport,
            &self.vertex,
            &self.fragment,
            global_params,
            &self.blend_state,
        );
        unsafe {
            device_context.PSSetSamplers(0, Some(sampler));
            device_context.VSSetShaderResources(0, Some(texture));
            device_context.PSSetShaderResources(0, Some(texture));

            device_context.DrawInstanced(4, instance_count, 0, 0);
        }
        Ok(())
    }

    fn draw_range(
        &self,
        device: &ID3D11Device,
        device_context: &ID3D11DeviceContext,
        viewport: &[D3D11_VIEWPORT],
        global_params: &[Option<ID3D11Buffer>],
        vertex_count: u32,
        first_instance: u32,
        instance_count: u32,
    ) -> Result<()> {
        let view = create_buffer_view_range(device, &self.buffer, first_instance, instance_count)?;
        set_pipeline_state(
            device_context,
            slice::from_ref(&view),
            D3D_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP,
            viewport,
            &self.vertex,
            &self.fragment,
            global_params,
            &self.blend_state,
        );
        unsafe {
            device_context.DrawInstanced(vertex_count, instance_count, 0, 0);
        }
        Ok(())
    }

    fn draw_range_with_texture(
        &self,
        device: &ID3D11Device,
        device_context: &ID3D11DeviceContext,
        texture: &[Option<ID3D11ShaderResourceView>],
        viewport: &[D3D11_VIEWPORT],
        global_params: &[Option<ID3D11Buffer>],
        sampler: &[Option<ID3D11SamplerState>],
        first_instance: u32,
        instance_count: u32,
    ) -> Result<()> {
        let view = create_buffer_view_range(device, &self.buffer, first_instance, instance_count)?;
        set_pipeline_state(
            device_context,
            slice::from_ref(&view),
            D3D_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP,
            viewport,
            &self.vertex,
            &self.fragment,
            global_params,
            &self.blend_state,
        );
        unsafe {
            device_context.PSSetSamplers(0, Some(sampler));
            device_context.VSSetShaderResources(0, Some(texture));
            device_context.PSSetShaderResources(0, Some(texture));
            device_context.DrawInstanced(4, instance_count, 0, 0);
        }
        Ok(())
    }
}

#[derive(Clone)]
#[repr(C)]
struct PathRasterizationSprite {
    xy_position: Point<ScaledPixels>,
    st_position: Point<f32>,
    color: Background,
    bounds: Bounds<ScaledPixels>,
    content_mask: ContentMask<ScaledPixels>,
    scratch_bounds: Bounds<ScaledPixels>,
    texture_size: [f32; 2],
}

#[derive(Clone, Copy)]
#[repr(C)]
struct PathSprite {
    bounds: Bounds<ScaledPixels>,
    scratch_bounds: Bounds<ScaledPixels>,
    texture_size: [f32; 2],
}

impl Drop for DirectXRenderer {
    fn drop(&mut self) {
        #[cfg(debug_assertions)]
        if let Some(devices) = &self.devices {
            report_live_objects(&devices.device).ok();
        }
    }
}

#[inline]
fn get_comp_device(dxgi_device: &IDXGIDevice) -> Result<IDCompositionDevice> {
    Ok(unsafe { DCompositionCreateDevice(dxgi_device)? })
}

fn create_swap_chain_for_composition(
    dxgi_factory: &IDXGIFactory6,
    device: &ID3D11Device,
    width: u32,
    height: u32,
) -> Result<IDXGISwapChain1> {
    let desc = DXGI_SWAP_CHAIN_DESC1 {
        Width: width,
        Height: height,
        Format: RENDER_TARGET_FORMAT,
        Stereo: false.into(),
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
        BufferCount: BUFFER_COUNT as u32,
        // Composition SwapChains only support the DXGI_SCALING_STRETCH Scaling.
        Scaling: DXGI_SCALING_STRETCH,
        SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
        AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
        Flags: 0,
    };
    Ok(unsafe { dxgi_factory.CreateSwapChainForComposition(device, &desc, None)? })
}

fn create_swap_chain(
    dxgi_factory: &IDXGIFactory6,
    device: &ID3D11Device,
    hwnd: HWND,
    width: u32,
    height: u32,
) -> Result<IDXGISwapChain1> {
    use windows::Win32::Graphics::Dxgi::DXGI_MWA_NO_ALT_ENTER;

    let desc = DXGI_SWAP_CHAIN_DESC1 {
        Width: width,
        Height: height,
        Format: RENDER_TARGET_FORMAT,
        Stereo: false.into(),
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
        BufferCount: BUFFER_COUNT as u32,
        Scaling: DXGI_SCALING_NONE,
        SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
        AlphaMode: DXGI_ALPHA_MODE_IGNORE,
        Flags: 0,
    };
    let swap_chain =
        unsafe { dxgi_factory.CreateSwapChainForHwnd(device, hwnd, &desc, None, None) }?;
    unsafe { dxgi_factory.MakeWindowAssociation(hwnd, DXGI_MWA_NO_ALT_ENTER) }?;
    Ok(swap_chain)
}

#[inline]
fn create_resources(
    devices: &DirectXRendererDevices,
    swap_chain: &IDXGISwapChain1,
    width: u32,
    height: u32,
) -> Result<(
    ID3D11Texture2D,
    Option<ID3D11RenderTargetView>,
    D3D11_VIEWPORT,
)> {
    let (render_target, render_target_view) =
        create_render_target_and_its_view(swap_chain, &devices.device)?;
    let viewport = set_viewport(&devices.device_context, width as f32, height as f32);
    Ok((render_target, render_target_view, viewport))
}

#[inline]
fn create_render_target_and_its_view(
    swap_chain: &IDXGISwapChain1,
    device: &ID3D11Device,
) -> Result<(ID3D11Texture2D, Option<ID3D11RenderTargetView>)> {
    let render_target: ID3D11Texture2D = unsafe { swap_chain.GetBuffer(0) }?;
    let mut render_target_view = None;
    unsafe { device.CreateRenderTargetView(&render_target, None, Some(&mut render_target_view))? };
    Ok((render_target, render_target_view))
}

#[inline]
fn create_path_intermediate_texture(
    device: &ID3D11Device,
    width: u32,
    height: u32,
) -> Result<(ID3D11Texture2D, Option<ID3D11ShaderResourceView>)> {
    let texture = unsafe {
        let mut output = None;
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: RENDER_TARGET_FORMAT,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        device.CreateTexture2D(&desc, None, Some(&mut output))?;
        output.unwrap()
    };

    let mut shader_resource_view = None;
    unsafe { device.CreateShaderResourceView(&texture, None, Some(&mut shader_resource_view))? };

    Ok((texture, Some(shader_resource_view.unwrap())))
}

#[inline]
fn create_backdrop_texture_and_srv(
    device: &ID3D11Device,
    width: u32,
    height: u32,
) -> Result<(ID3D11Texture2D, Option<ID3D11ShaderResourceView>)> {
    let texture = unsafe {
        let mut output = None;
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: RENDER_TARGET_FORMAT,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        device.CreateTexture2D(&desc, None, Some(&mut output))?;
        output.unwrap()
    };

    let mut srv = None;
    unsafe { device.CreateShaderResourceView(&texture, None, Some(&mut srv))? };

    Ok((texture, srv))
}

#[inline]
fn create_backdrop_blur_texture_and_views(
    device: &ID3D11Device,
    width: u32,
    height: u32,
) -> Result<(
    ID3D11Texture2D,
    Option<ID3D11RenderTargetView>,
    Option<ID3D11ShaderResourceView>,
)> {
    let texture = unsafe {
        let mut output = None;
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: RENDER_TARGET_FORMAT,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        device.CreateTexture2D(&desc, None, Some(&mut output))?;
        output.unwrap()
    };

    let mut rtv = None;
    let mut srv = None;
    unsafe {
        device.CreateRenderTargetView(&texture, None, Some(&mut rtv))?;
        device.CreateShaderResourceView(&texture, None, Some(&mut srv))?;
    }

    Ok((texture, rtv, srv))
}

fn create_backdrop_blur_resources(
    device: &ID3D11Device,
    width: u32,
    height: u32,
) -> Result<BackdropBlurResources> {
    let level_sizes = backdrop_blur_level_sizes_for(size(
        DevicePixels(width as i32),
        DevicePixels(height as i32),
    ))
    .into_iter()
    .map(|level_size| (level_size.width.0 as u32, level_size.height.0 as u32))
    .collect::<Vec<_>>();
    let mut downsample_textures = Vec::new();
    let mut downsample_views = Vec::new();
    let mut downsample_srvs = Vec::new();
    let mut upsample_textures = Vec::new();
    let mut upsample_views = Vec::new();
    let mut upsample_srvs = Vec::new();

    for &(level_width, level_height) in level_sizes.iter().skip(1) {
        let (texture, view, srv) =
            create_backdrop_blur_texture_and_views(device, level_width, level_height)?;
        downsample_textures.push(texture);
        downsample_views.push(view);
        downsample_srvs.push(srv);
    }

    for &(level_width, level_height) in &level_sizes {
        let (texture, view, srv) =
            create_backdrop_blur_texture_and_views(device, level_width, level_height)?;
        upsample_textures.push(texture);
        upsample_views.push(view);
        upsample_srvs.push(srv);
    }

    Ok(BackdropBlurResources {
        level_sizes,
        downsample_textures,
        downsample_views,
        downsample_srvs,
        upsample_textures,
        upsample_views,
        upsample_srvs,
    })
}

#[inline]
fn create_path_intermediate_msaa_texture_and_view(
    device: &ID3D11Device,
    width: u32,
    height: u32,
) -> Result<(ID3D11Texture2D, Option<ID3D11RenderTargetView>)> {
    let msaa_texture = unsafe {
        let mut output = None;
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: RENDER_TARGET_FORMAT,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: PATH_MULTISAMPLE_COUNT,
                Quality: D3D11_STANDARD_MULTISAMPLE_PATTERN.0 as u32,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        device.CreateTexture2D(&desc, None, Some(&mut output))?;
        output.unwrap()
    };
    let mut msaa_view = None;
    unsafe { device.CreateRenderTargetView(&msaa_texture, None, Some(&mut msaa_view))? };
    Ok((msaa_texture, Some(msaa_view.unwrap())))
}

#[inline]
fn set_viewport(device_context: &ID3D11DeviceContext, width: f32, height: f32) -> D3D11_VIEWPORT {
    let viewport = [D3D11_VIEWPORT {
        TopLeftX: 0.0,
        TopLeftY: 0.0,
        Width: width,
        Height: height,
        MinDepth: 0.0,
        MaxDepth: 1.0,
    }];
    unsafe { device_context.RSSetViewports(Some(&viewport)) };
    viewport[0]
}

#[inline]
fn set_rasterizer_state(device: &ID3D11Device, device_context: &ID3D11DeviceContext) -> Result<()> {
    let desc = D3D11_RASTERIZER_DESC {
        FillMode: D3D11_FILL_SOLID,
        CullMode: D3D11_CULL_NONE,
        FrontCounterClockwise: false.into(),
        DepthBias: 0,
        DepthBiasClamp: 0.0,
        SlopeScaledDepthBias: 0.0,
        DepthClipEnable: true.into(),
        ScissorEnable: false.into(),
        MultisampleEnable: true.into(),
        AntialiasedLineEnable: false.into(),
    };
    let rasterizer_state = unsafe {
        let mut state = None;
        device.CreateRasterizerState(&desc, Some(&mut state))?;
        state.unwrap()
    };
    unsafe { device_context.RSSetState(&rasterizer_state) };
    Ok(())
}

// https://learn.microsoft.com/en-us/windows/win32/api/d3d11/ns-d3d11-d3d11_blend_desc
#[inline]
fn create_blend_state(device: &ID3D11Device) -> Result<ID3D11BlendState> {
    let mut desc = D3D11_BLEND_DESC::default();
    desc.RenderTarget[0].BlendEnable = true.into();
    desc.RenderTarget[0].BlendOp = D3D11_BLEND_OP_ADD;
    desc.RenderTarget[0].BlendOpAlpha = D3D11_BLEND_OP_ADD;
    desc.RenderTarget[0].SrcBlend = D3D11_BLEND_SRC_ALPHA;
    desc.RenderTarget[0].SrcBlendAlpha = D3D11_BLEND_ONE;
    desc.RenderTarget[0].DestBlend = D3D11_BLEND_INV_SRC_ALPHA;
    desc.RenderTarget[0].DestBlendAlpha = D3D11_BLEND_ONE;
    desc.RenderTarget[0].RenderTargetWriteMask = D3D11_COLOR_WRITE_ENABLE_ALL.0 as u8;
    unsafe {
        let mut state = None;
        device.CreateBlendState(&desc, Some(&mut state))?;
        Ok(state.unwrap())
    }
}

#[inline]
fn create_blend_state_without_blending(device: &ID3D11Device) -> Result<ID3D11BlendState> {
    let mut desc = D3D11_BLEND_DESC::default();
    desc.RenderTarget[0].RenderTargetWriteMask = D3D11_COLOR_WRITE_ENABLE_ALL.0 as u8;
    unsafe {
        let mut state = None;
        device.CreateBlendState(&desc, Some(&mut state))?;
        Ok(state.unwrap())
    }
}

#[inline]
fn create_blend_state_for_subpixel_rendering(device: &ID3D11Device) -> Result<ID3D11BlendState> {
    let mut desc = D3D11_BLEND_DESC::default();
    desc.RenderTarget[0].BlendEnable = true.into();
    desc.RenderTarget[0].BlendOp = D3D11_BLEND_OP_ADD;
    desc.RenderTarget[0].BlendOpAlpha = D3D11_BLEND_OP_ADD;
    desc.RenderTarget[0].SrcBlend = D3D11_BLEND_SRC1_COLOR;
    desc.RenderTarget[0].DestBlend = D3D11_BLEND_INV_SRC1_COLOR;
    // It does not make sense to draw transparent subpixel-rendered text, since it cannot be meaningfully alpha-blended onto anything else.
    desc.RenderTarget[0].SrcBlendAlpha = D3D11_BLEND_ONE;
    desc.RenderTarget[0].DestBlendAlpha = D3D11_BLEND_ZERO;
    desc.RenderTarget[0].RenderTargetWriteMask =
        D3D11_COLOR_WRITE_ENABLE_ALL.0 as u8 & !D3D11_COLOR_WRITE_ENABLE_ALPHA.0 as u8;

    unsafe {
        let mut state = None;
        device.CreateBlendState(&desc, Some(&mut state))?;
        Ok(state.unwrap())
    }
}

#[inline]
fn create_blend_state_for_path_rasterization(device: &ID3D11Device) -> Result<ID3D11BlendState> {
    // If the feature level is set to greater than D3D_FEATURE_LEVEL_9_3, the display
    // device performs the blend in linear space, which is ideal.
    let mut desc = D3D11_BLEND_DESC::default();
    desc.RenderTarget[0].BlendEnable = true.into();
    desc.RenderTarget[0].BlendOp = D3D11_BLEND_OP_ADD;
    desc.RenderTarget[0].BlendOpAlpha = D3D11_BLEND_OP_ADD;
    desc.RenderTarget[0].SrcBlend = D3D11_BLEND_ONE;
    desc.RenderTarget[0].SrcBlendAlpha = D3D11_BLEND_ONE;
    desc.RenderTarget[0].DestBlend = D3D11_BLEND_INV_SRC_ALPHA;
    desc.RenderTarget[0].DestBlendAlpha = D3D11_BLEND_INV_SRC_ALPHA;
    desc.RenderTarget[0].RenderTargetWriteMask = D3D11_COLOR_WRITE_ENABLE_ALL.0 as u8;
    unsafe {
        let mut state = None;
        device.CreateBlendState(&desc, Some(&mut state))?;
        Ok(state.unwrap())
    }
}

#[inline]
fn create_blend_state_for_path_sprite(device: &ID3D11Device) -> Result<ID3D11BlendState> {
    // If the feature level is set to greater than D3D_FEATURE_LEVEL_9_3, the display
    // device performs the blend in linear space, which is ideal.
    let mut desc = D3D11_BLEND_DESC::default();
    desc.RenderTarget[0].BlendEnable = true.into();
    desc.RenderTarget[0].BlendOp = D3D11_BLEND_OP_ADD;
    desc.RenderTarget[0].BlendOpAlpha = D3D11_BLEND_OP_ADD;
    desc.RenderTarget[0].SrcBlend = D3D11_BLEND_ONE;
    desc.RenderTarget[0].SrcBlendAlpha = D3D11_BLEND_ONE;
    desc.RenderTarget[0].DestBlend = D3D11_BLEND_INV_SRC_ALPHA;
    desc.RenderTarget[0].DestBlendAlpha = D3D11_BLEND_ONE;
    desc.RenderTarget[0].RenderTargetWriteMask = D3D11_COLOR_WRITE_ENABLE_ALL.0 as u8;
    unsafe {
        let mut state = None;
        device.CreateBlendState(&desc, Some(&mut state))?;
        Ok(state.unwrap())
    }
}

#[inline]
fn create_vertex_shader(device: &ID3D11Device, bytes: &[u8]) -> Result<ID3D11VertexShader> {
    unsafe {
        let mut shader = None;
        device.CreateVertexShader(bytes, None, Some(&mut shader))?;
        Ok(shader.unwrap())
    }
}

#[inline]
fn create_fragment_shader(device: &ID3D11Device, bytes: &[u8]) -> Result<ID3D11PixelShader> {
    unsafe {
        let mut shader = None;
        device.CreatePixelShader(bytes, None, Some(&mut shader))?;
        Ok(shader.unwrap())
    }
}

#[inline]
fn create_buffer(
    device: &ID3D11Device,
    element_size: usize,
    buffer_size: usize,
) -> Result<ID3D11Buffer> {
    let byte_width = element_size
        .checked_mul(buffer_size)
        .context("structured buffer size overflow")?;
    if byte_width > MAX_STRUCTURED_BUFFER_BYTES {
        anyhow::bail!(
            "structured buffer size {} exceeds maximum {}",
            byte_width,
            MAX_STRUCTURED_BUFFER_BYTES
        );
    }
    let desc = D3D11_BUFFER_DESC {
        ByteWidth: u32::try_from(byte_width).context("structured buffer size exceeds u32")?,
        Usage: D3D11_USAGE_DYNAMIC,
        BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
        CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
        MiscFlags: D3D11_RESOURCE_MISC_BUFFER_STRUCTURED.0 as u32,
        StructureByteStride: element_size as u32,
    };
    let mut buffer = None;
    unsafe { device.CreateBuffer(&desc, None, Some(&mut buffer)) }?;
    Ok(buffer.unwrap())
}

#[inline]
fn create_buffer_view(
    device: &ID3D11Device,
    buffer: &ID3D11Buffer,
) -> Result<Option<ID3D11ShaderResourceView>> {
    let mut view = None;
    unsafe { device.CreateShaderResourceView(buffer, None, Some(&mut view)) }?;
    Ok(view)
}

#[inline]
fn create_buffer_view_range(
    device: &ID3D11Device,
    buffer: &ID3D11Buffer,
    first_element: u32,
    num_elements: u32,
) -> Result<Option<ID3D11ShaderResourceView>> {
    let desc = D3D11_SHADER_RESOURCE_VIEW_DESC {
        Format: DXGI_FORMAT_UNKNOWN,
        ViewDimension: D3D11_SRV_DIMENSION_BUFFER,
        Anonymous: D3D11_SHADER_RESOURCE_VIEW_DESC_0 {
            Buffer: D3D11_BUFFER_SRV {
                Anonymous1: D3D11_BUFFER_SRV_0 {
                    FirstElement: first_element,
                },
                Anonymous2: D3D11_BUFFER_SRV_1 {
                    NumElements: num_elements,
                },
            },
        },
    };
    let mut view = None;
    unsafe { device.CreateShaderResourceView(buffer, Some(&desc), Some(&mut view)) }?;
    Ok(view)
}

#[inline]
fn update_buffer<T>(
    device_context: &ID3D11DeviceContext,
    buffer: &ID3D11Buffer,
    data: &[T],
) -> Result<()> {
    unsafe {
        let mut dest = std::mem::zeroed();
        device_context.Map(buffer, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut dest))?;
        std::ptr::copy_nonoverlapping(data.as_ptr(), dest.pData as _, data.len());
        device_context.Unmap(buffer, 0);
    }
    Ok(())
}

#[inline]
fn set_pipeline_state(
    device_context: &ID3D11DeviceContext,
    buffer_view: &[Option<ID3D11ShaderResourceView>],
    topology: D3D_PRIMITIVE_TOPOLOGY,
    viewport: &[D3D11_VIEWPORT],
    vertex_shader: &ID3D11VertexShader,
    fragment_shader: &ID3D11PixelShader,
    global_params: &[Option<ID3D11Buffer>],
    blend_state: &ID3D11BlendState,
) {
    unsafe {
        device_context.VSSetShaderResources(1, Some(buffer_view));
        device_context.PSSetShaderResources(1, Some(buffer_view));
        device_context.IASetPrimitiveTopology(topology);
        device_context.RSSetViewports(Some(viewport));
        device_context.VSSetShader(vertex_shader, None);
        device_context.PSSetShader(fragment_shader, None);
        device_context.VSSetConstantBuffers(0, Some(global_params));
        device_context.PSSetConstantBuffers(0, Some(global_params));
        device_context.OMSetBlendState(blend_state, None, 0xFFFFFFFF);
    }
}

#[cfg(debug_assertions)]
fn report_live_objects(device: &ID3D11Device) -> Result<()> {
    let debug_device: ID3D11Debug = device.cast()?;
    unsafe {
        debug_device.ReportLiveDeviceObjects(D3D11_RLDO_DETAIL)?;
    }
    Ok(())
}

const BUFFER_COUNT: usize = 3;

pub(crate) mod shader_resources {
    use anyhow::Result;

    #[cfg(debug_assertions)]
    use windows::{
        Win32::Graphics::Direct3D::{
            Fxc::{D3DCOMPILE_DEBUG, D3DCOMPILE_SKIP_OPTIMIZATION, D3DCompileFromFile},
            ID3DBlob,
        },
        core::{HSTRING, PCSTR},
    };

    #[derive(Copy, Clone, Debug, Eq, PartialEq)]
    pub(crate) enum ShaderModule {
        Quad,
        Shadow,
        BackdropBlur,
        BackdropBlurDownsample,
        BackdropBlurUpsample,
        Underline,
        PathRasterization,
        PathSprite,
        MonochromeSprite,
        SubpixelSprite,
        PolychromeSprite,
        EmojiRasterization,
    }

    #[derive(Copy, Clone, Debug, Eq, PartialEq)]
    pub(crate) enum ShaderTarget {
        Vertex,
        Fragment,
    }

    pub(crate) struct RawShaderBytes<'t> {
        inner: &'t [u8],

        #[cfg(debug_assertions)]
        _blob: ID3DBlob,
    }

    impl<'t> RawShaderBytes<'t> {
        pub(crate) fn new(module: ShaderModule, target: ShaderTarget) -> Result<Self> {
            #[cfg(not(debug_assertions))]
            {
                Ok(Self::from_bytes(module, target))
            }
            #[cfg(debug_assertions)]
            {
                let blob = build_shader_blob(module, target)?;
                let inner = unsafe {
                    std::slice::from_raw_parts(
                        blob.GetBufferPointer() as *const u8,
                        blob.GetBufferSize(),
                    )
                };
                Ok(Self { inner, _blob: blob })
            }
        }

        pub(crate) fn as_bytes(&'t self) -> &'t [u8] {
            self.inner
        }

        #[cfg(not(debug_assertions))]
        fn from_bytes(module: ShaderModule, target: ShaderTarget) -> Self {
            let bytes = match module {
                ShaderModule::Quad => match target {
                    ShaderTarget::Vertex => QUAD_VERTEX_BYTES,
                    ShaderTarget::Fragment => QUAD_FRAGMENT_BYTES,
                },
                ShaderModule::Shadow => match target {
                    ShaderTarget::Vertex => SHADOW_VERTEX_BYTES,
                    ShaderTarget::Fragment => SHADOW_FRAGMENT_BYTES,
                },
                ShaderModule::BackdropBlur => match target {
                    ShaderTarget::Vertex => BACKDROP_BLUR_VERTEX_BYTES,
                    ShaderTarget::Fragment => BACKDROP_BLUR_FRAGMENT_BYTES,
                },
                ShaderModule::BackdropBlurDownsample => match target {
                    ShaderTarget::Vertex => BACKDROP_BLUR_DOWNSAMPLE_VERTEX_BYTES,
                    ShaderTarget::Fragment => BACKDROP_BLUR_DOWNSAMPLE_FRAGMENT_BYTES,
                },
                ShaderModule::BackdropBlurUpsample => match target {
                    ShaderTarget::Vertex => BACKDROP_BLUR_UPSAMPLE_VERTEX_BYTES,
                    ShaderTarget::Fragment => BACKDROP_BLUR_UPSAMPLE_FRAGMENT_BYTES,
                },
                ShaderModule::Underline => match target {
                    ShaderTarget::Vertex => UNDERLINE_VERTEX_BYTES,
                    ShaderTarget::Fragment => UNDERLINE_FRAGMENT_BYTES,
                },
                ShaderModule::PathRasterization => match target {
                    ShaderTarget::Vertex => PATH_RASTERIZATION_VERTEX_BYTES,
                    ShaderTarget::Fragment => PATH_RASTERIZATION_FRAGMENT_BYTES,
                },
                ShaderModule::PathSprite => match target {
                    ShaderTarget::Vertex => PATH_SPRITE_VERTEX_BYTES,
                    ShaderTarget::Fragment => PATH_SPRITE_FRAGMENT_BYTES,
                },
                ShaderModule::MonochromeSprite => match target {
                    ShaderTarget::Vertex => MONOCHROME_SPRITE_VERTEX_BYTES,
                    ShaderTarget::Fragment => MONOCHROME_SPRITE_FRAGMENT_BYTES,
                },
                ShaderModule::SubpixelSprite => match target {
                    ShaderTarget::Vertex => SUBPIXEL_SPRITE_VERTEX_BYTES,
                    ShaderTarget::Fragment => SUBPIXEL_SPRITE_FRAGMENT_BYTES,
                },
                ShaderModule::PolychromeSprite => match target {
                    ShaderTarget::Vertex => POLYCHROME_SPRITE_VERTEX_BYTES,
                    ShaderTarget::Fragment => POLYCHROME_SPRITE_FRAGMENT_BYTES,
                },
                ShaderModule::EmojiRasterization => match target {
                    ShaderTarget::Vertex => EMOJI_RASTERIZATION_VERTEX_BYTES,
                    ShaderTarget::Fragment => EMOJI_RASTERIZATION_FRAGMENT_BYTES,
                },
            };
            Self { inner: bytes }
        }
    }

    #[cfg(debug_assertions)]
    pub(super) fn build_shader_blob(entry: ShaderModule, target: ShaderTarget) -> Result<ID3DBlob> {
        unsafe {
            use windows::Win32::Graphics::{
                Direct3D::ID3DInclude, Hlsl::D3D_COMPILE_STANDARD_FILE_INCLUDE,
            };

            let shader_name = if matches!(entry, ShaderModule::EmojiRasterization) {
                "color_text_raster.hlsl"
            } else {
                "shaders.hlsl"
            };

            let entry = format!(
                "{}_{}\0",
                entry.as_str(),
                match target {
                    ShaderTarget::Vertex => "vertex",
                    ShaderTarget::Fragment => "fragment",
                }
            );
            let target = match target {
                ShaderTarget::Vertex => "vs_4_1\0",
                ShaderTarget::Fragment => "ps_4_1\0",
            };

            let mut compile_blob = None;
            let mut error_blob = None;
            let shader_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join(&format!("src/{}", shader_name))
                .canonicalize()?;

            let entry_point = PCSTR::from_raw(entry.as_ptr());
            let target_cstr = PCSTR::from_raw(target.as_ptr());

            // really dirty trick because winapi bindings are unhappy otherwise
            let include_handler = &std::mem::transmute::<usize, ID3DInclude>(
                D3D_COMPILE_STANDARD_FILE_INCLUDE as usize,
            );

            let ret = D3DCompileFromFile(
                &HSTRING::from(shader_path.to_str().unwrap()),
                None,
                include_handler,
                entry_point,
                target_cstr,
                D3DCOMPILE_DEBUG | D3DCOMPILE_SKIP_OPTIMIZATION,
                0,
                &mut compile_blob,
                Some(&mut error_blob),
            );
            if ret.is_err() {
                let Some(error_blob) = error_blob else {
                    return Err(anyhow::anyhow!("{ret:?}"));
                };

                let error_string =
                    std::ffi::CStr::from_ptr(error_blob.GetBufferPointer() as *const i8)
                        .to_string_lossy();
                log::error!("Shader compile error: {}", error_string);
                return Err(anyhow::anyhow!("Compile error: {}", error_string));
            }
            Ok(compile_blob.unwrap())
        }
    }

    #[cfg(not(debug_assertions))]
    include!(concat!(env!("OUT_DIR"), "/shaders_bytes.rs"));

    #[cfg(debug_assertions)]
    impl ShaderModule {
        pub fn as_str(self) -> &'static str {
            match self {
                ShaderModule::Quad => "quad",
                ShaderModule::Shadow => "shadow",
                ShaderModule::BackdropBlur => "backdrop_blur",
                ShaderModule::BackdropBlurDownsample => "backdrop_blur_downsample",
                ShaderModule::BackdropBlurUpsample => "backdrop_blur_upsample",
                ShaderModule::Underline => "underline",
                ShaderModule::PathRasterization => "path_rasterization",
                ShaderModule::PathSprite => "path_sprite",
                ShaderModule::MonochromeSprite => "monochrome_sprite",
                ShaderModule::SubpixelSprite => "subpixel_sprite",
                ShaderModule::PolychromeSprite => "polychrome_sprite",
                ShaderModule::EmojiRasterization => "emoji_rasterization",
            }
        }
    }
}

mod nvidia {
    use std::{
        ffi::CStr,
        os::raw::{c_char, c_int, c_uint},
    };

    use anyhow::Result;
    use windows::{Win32::System::LibraryLoader::GetProcAddress, core::s};

    use crate::with_dll_library;

    // https://github.com/NVIDIA/nvapi/blob/7cb76fce2f52de818b3da497af646af1ec16ce27/nvapi_lite_common.h#L180
    const NVAPI_SHORT_STRING_MAX: usize = 64;

    // https://github.com/NVIDIA/nvapi/blob/7cb76fce2f52de818b3da497af646af1ec16ce27/nvapi_lite_common.h#L235
    #[allow(non_camel_case_types)]
    type NvAPI_ShortString = [c_char; NVAPI_SHORT_STRING_MAX];

    // https://github.com/NVIDIA/nvapi/blob/7cb76fce2f52de818b3da497af646af1ec16ce27/nvapi_lite_common.h#L447
    #[allow(non_camel_case_types)]
    type NvAPI_SYS_GetDriverAndBranchVersion_t = unsafe extern "C" fn(
        driver_version: *mut c_uint,
        build_branch_string: *mut NvAPI_ShortString,
    ) -> c_int;

    pub(super) fn get_driver_version() -> Result<String> {
        #[cfg(target_pointer_width = "64")]
        let nvidia_dll_name = s!("nvapi64.dll");
        #[cfg(target_pointer_width = "32")]
        let nvidia_dll_name = s!("nvapi.dll");

        with_dll_library(nvidia_dll_name, |nvidia_dll| unsafe {
            let nvapi_query_addr = GetProcAddress(nvidia_dll, s!("nvapi_QueryInterface"))
                .ok_or_else(|| anyhow::anyhow!("Failed to get nvapi_QueryInterface address"))?;
            let nvapi_query: extern "C" fn(u32) -> *mut () = std::mem::transmute(nvapi_query_addr);

            // https://github.com/NVIDIA/nvapi/blob/7cb76fce2f52de818b3da497af646af1ec16ce27/nvapi_interface.h#L41
            let nvapi_get_driver_version_ptr = nvapi_query(0x2926aaad);
            if nvapi_get_driver_version_ptr.is_null() {
                anyhow::bail!("Failed to get NVIDIA driver version function pointer");
            }
            let nvapi_get_driver_version: NvAPI_SYS_GetDriverAndBranchVersion_t =
                std::mem::transmute(nvapi_get_driver_version_ptr);

            let mut driver_version: c_uint = 0;
            let mut build_branch_string: NvAPI_ShortString = [0; NVAPI_SHORT_STRING_MAX];
            let result = nvapi_get_driver_version(
                &mut driver_version as *mut c_uint,
                &mut build_branch_string as *mut NvAPI_ShortString,
            );

            if result != 0 {
                anyhow::bail!(
                    "Failed to get NVIDIA driver version, error code: {}",
                    result
                );
            }
            let major = driver_version / 100;
            let minor = driver_version % 100;
            let branch_string = CStr::from_ptr(build_branch_string.as_ptr());
            Ok(format!(
                "{}.{} {}",
                major,
                minor,
                branch_string.to_string_lossy()
            ))
        })
    }
}

mod amd {
    use std::os::raw::{c_char, c_int, c_void};

    use anyhow::Result;
    use windows::{Win32::System::LibraryLoader::GetProcAddress, core::s};

    use crate::with_dll_library;

    // https://github.com/GPUOpen-LibrariesAndSDKs/AGS_SDK/blob/5d8812d703d0335741b6f7ffc37838eeb8b967f7/ags_lib/inc/amd_ags.h#L145
    const AGS_CURRENT_VERSION: i32 = (6 << 22) | (3 << 12);

    // https://github.com/GPUOpen-LibrariesAndSDKs/AGS_SDK/blob/5d8812d703d0335741b6f7ffc37838eeb8b967f7/ags_lib/inc/amd_ags.h#L204
    // This is an opaque type, using struct to represent it properly for FFI
    #[repr(C)]
    struct AGSContext {
        _private: [u8; 0],
    }

    #[repr(C)]
    pub struct AGSGPUInfo {
        pub driver_version: *const c_char,
        pub radeon_software_version: *const c_char,
        pub num_devices: c_int,
        pub devices: *mut c_void,
    }

    // https://github.com/GPUOpen-LibrariesAndSDKs/AGS_SDK/blob/5d8812d703d0335741b6f7ffc37838eeb8b967f7/ags_lib/inc/amd_ags.h#L429
    #[allow(non_camel_case_types)]
    type agsInitialize_t = unsafe extern "C" fn(
        version: c_int,
        config: *const c_void,
        context: *mut *mut AGSContext,
        gpu_info: *mut AGSGPUInfo,
    ) -> c_int;

    // https://github.com/GPUOpen-LibrariesAndSDKs/AGS_SDK/blob/5d8812d703d0335741b6f7ffc37838eeb8b967f7/ags_lib/inc/amd_ags.h#L436
    #[allow(non_camel_case_types)]
    type agsDeInitialize_t = unsafe extern "C" fn(context: *mut AGSContext) -> c_int;

    pub(super) fn get_driver_version() -> Result<String> {
        #[cfg(target_pointer_width = "64")]
        let amd_dll_name = s!("amd_ags_x64.dll");
        #[cfg(target_pointer_width = "32")]
        let amd_dll_name = s!("amd_ags_x86.dll");

        with_dll_library(amd_dll_name, |amd_dll| unsafe {
            let ags_initialize_addr = GetProcAddress(amd_dll, s!("agsInitialize"))
                .ok_or_else(|| anyhow::anyhow!("Failed to get agsInitialize address"))?;
            let ags_deinitialize_addr = GetProcAddress(amd_dll, s!("agsDeInitialize"))
                .ok_or_else(|| anyhow::anyhow!("Failed to get agsDeInitialize address"))?;

            let ags_initialize: agsInitialize_t = std::mem::transmute(ags_initialize_addr);
            let ags_deinitialize: agsDeInitialize_t = std::mem::transmute(ags_deinitialize_addr);

            let mut context: *mut AGSContext = std::ptr::null_mut();
            let mut gpu_info: AGSGPUInfo = AGSGPUInfo {
                driver_version: std::ptr::null(),
                radeon_software_version: std::ptr::null(),
                num_devices: 0,
                devices: std::ptr::null_mut(),
            };

            let result = ags_initialize(
                AGS_CURRENT_VERSION,
                std::ptr::null(),
                &mut context,
                &mut gpu_info,
            );
            if result != 0 {
                anyhow::bail!("Failed to initialize AMD AGS, error code: {}", result);
            }

            // Vulkan actually returns this as the driver version
            let software_version = if !gpu_info.radeon_software_version.is_null() {
                std::ffi::CStr::from_ptr(gpu_info.radeon_software_version)
                    .to_string_lossy()
                    .into_owned()
            } else {
                "Unknown Radeon Software Version".to_string()
            };

            let driver_version = if !gpu_info.driver_version.is_null() {
                std::ffi::CStr::from_ptr(gpu_info.driver_version)
                    .to_string_lossy()
                    .into_owned()
            } else {
                "Unknown Radeon Driver Version".to_string()
            };

            ags_deinitialize(context);
            Ok(format!("{} ({})", software_version, driver_version))
        })
    }
}

#[cfg(test)]
mod tests {
    use gpui::{BackdropBlur, Bounds, ContentMask, Corners, ScaledPixels, point, size};

    use super::{
        BackdropBlurParams, GlobalParams, RetainedLayerClip, retained_layer_clip,
        rounded_backdrop_rebuild_requested,
    };

    #[test]
    fn global_params_preserve_hlsl_constant_buffer_alignment() {
        assert_eq!(std::mem::size_of::<GlobalParams>(), 48);
    }

    #[test]
    fn backdrop_blur_params_preserve_hlsl_buffer_layout() {
        assert_eq!(std::mem::size_of::<BackdropBlurParams>(), 16);
    }

    /// `struct BackdropBlur` in `shaders.hlsl` is hand-written, unlike the
    /// Metal one that `gpui_macos/build.rs` generates from `scene.rs`, so
    /// nothing else catches it drifting from the Rust type. The structured
    /// buffer's `StructureByteStride` comes from this size, and a shader-side
    /// struct of a different size skews every instance after the first.
    #[test]
    fn backdrop_blur_preserves_hlsl_structured_buffer_stride() {
        assert_eq!(std::mem::size_of::<BackdropBlur>(), 112);
    }

    #[test]
    fn rounded_backdrop_rebuild_uses_retained_configuration() {
        assert!(!rounded_backdrop_rebuild_requested(None));
        assert!(rounded_backdrop_rebuild_requested(Some(0.0)));
        assert!(rounded_backdrop_rebuild_requested(Some(12.0)));
    }

    #[test]
    fn retained_layer_clip_is_disabled_for_rectangular_masks() {
        let mask = ContentMask::new(Bounds::new(
            point(ScaledPixels(0.0), ScaledPixels(0.0)),
            size(ScaledPixels(100.0), ScaledPixels(80.0)),
        ));

        assert_eq!(retained_layer_clip(&mask), None);
    }

    #[test]
    fn retained_layer_clip_preserves_rounded_mask_geometry() {
        let mask = ContentMask::rounded(
            Bounds::new(
                point(ScaledPixels(2.0), ScaledPixels(3.0)),
                size(ScaledPixels(100.0), ScaledPixels(80.0)),
            ),
            Corners {
                top_left: ScaledPixels(4.0),
                top_right: ScaledPixels(5.0),
                bottom_right: ScaledPixels(6.0),
                bottom_left: ScaledPixels(7.0),
            },
        );

        assert_eq!(
            retained_layer_clip(&mask),
            Some(RetainedLayerClip {
                left: 2.0,
                top: 3.0,
                right: 102.0,
                bottom: 83.0,
                top_left_radius: 4.0,
                top_right_radius: 5.0,
                bottom_right_radius: 6.0,
                bottom_left_radius: 7.0,
            })
        );
    }
}

mod dxgi {
    use windows::{
        Win32::Graphics::Dxgi::{IDXGIAdapter1, IDXGIDevice},
        core::Interface,
    };

    pub(super) fn get_driver_version(adapter: &IDXGIAdapter1) -> anyhow::Result<String> {
        let number = unsafe { adapter.CheckInterfaceSupport(&IDXGIDevice::IID as _) }?;
        Ok(format!(
            "{}.{}.{}.{}",
            number >> 48,
            (number >> 32) & 0xFFFF,
            (number >> 16) & 0xFFFF,
            number & 0xFFFF
        ))
    }
}
