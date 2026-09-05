use std::{collections::HashSet, ops::Range};

use super::*;

pub(super) struct DirectXRetainedLayer {
    texture: ID3D11Texture2D,
    view: Option<ID3D11RenderTargetView>,
    srv: Option<ID3D11ShaderResourceView>,
    bounds: Bounds<ScaledPixels>,
    input: RetainedLayerInput,
}

pub(super) struct RetainedBackdrop {
    _texture: ID3D11Texture2D,
    srv: Option<ID3D11ShaderResourceView>,
}

#[repr(C)]
pub(super) struct BackdropProjection {
    uvs: [Point<f32>; 4],
}

#[repr(C)]
pub(super) struct RetainedLayerSprite {
    positions: [Point<ScaledPixels>; 4],
    content_mask: ContentMask<ScaledPixels>,
    opacity: f32,
    _pad: [u32; 3],
}

impl DirectXRenderer {
    pub(super) fn prepare_retained_scene(
        &mut self,
        scene: &Scene,
        plan: &RetainedScenePlan,
    ) -> Result<Vec<bool>> {
        let active = plan
            .keys
            .iter()
            .enumerate()
            .filter_map(|(index, key)| {
                (!scene.retained_layers[index].paint_range.is_empty()).then_some(key)
            })
            .collect::<HashSet<_>>();
        self.retained_layers.retain(|key, _| active.contains(key));
        let dependent = retained_backdrop_layers(scene);
        for (index, &dependent) in dependent.iter().enumerate() {
            if direct_backdrop_layer(&scene.retained_layers[index], dependent) {
                self.retained_layers.remove(&plan.keys[index]);
            }
        }
        for &index in &plan.roots {
            if let Err(error) = self.prepare_retained_layer(scene, plan, &dependent, index) {
                self.retained_layers.clear();
                return Err(error);
            }
        }
        Ok(dependent)
    }

    fn prepare_retained_layer(
        &mut self,
        scene: &Scene,
        plan: &RetainedScenePlan,
        dependent: &[bool],
        index: usize,
    ) -> Result<()> {
        for &child in &plan.children[index] {
            self.prepare_retained_layer(scene, plan, dependent, child)?;
        }
        if dependent[index] {
            return Ok(());
        }
        self.render_retained_layer(scene, plan, dependent, index, Point::default())
    }

    fn render_retained_layer(
        &mut self,
        scene: &Scene,
        plan: &RetainedScenePlan,
        dependent: &[bool],
        index: usize,
        parent_origin: Point<ScaledPixels>,
    ) -> Result<()> {
        let layer = &scene.retained_layers[index];
        let (input, dirty) = RetainedLayerInput::new(scene, plan, index);
        let key = &plan.keys[index];
        if !dependent[index]
            && !dirty
            && self
                .retained_layers
                .get(key)
                .is_some_and(|cache| cache.input == input)
        {
            let cache = &self.retained_layers[key];
            if layer.opacity <= 0.0
                || transformed_bounds(cache.bounds, layer.transform)
                    .intersect(&layer.content_mask.bounds)
                    .is_empty()
            {
                self.retained_layers.remove(key);
            }
            return Ok(());
        }

        let Some(bounds) = retained_raster_bounds(scene, plan, index) else {
            self.retained_layers.remove(key);
            return Ok(());
        };
        let devices = self.devices.as_ref().context("devices missing")?;
        let limit = if matches!(
            unsafe { devices.device.GetFeatureLevel() },
            D3D_FEATURE_LEVEL_11_0 | D3D_FEATURE_LEVEL_11_1
        ) {
            D3D11_REQ_TEXTURE2D_U_OR_V_DIMENSION
        } else {
            8192
        };
        let bounds = texture_bounds(bounds, limit)?;
        let previous = self.retained_layers.remove(key);
        let cache =
            if let Some(mut cache) = previous.filter(|cache| cache.bounds.size == bounds.size) {
                cache.bounds = bounds;
                cache.input = input;
                cache
            } else {
                let (texture, view, srv) = create_backdrop_blur_texture_and_views(
                    &devices.device,
                    (bounds.size.width.0 as u32).max(1),
                    (bounds.size.height.0 as u32).max(1),
                )
                .context("Creating retained layer texture")?;
                DirectXRetainedLayer {
                    texture,
                    view,
                    srv,
                    bounds,
                    input,
                }
            };

        let backdrop = if dependent[index] {
            Some(self.capture_retained_backdrop(layer, bounds, parent_origin)?)
        } else {
            None
        };
        let previous_backdrop = std::mem::replace(&mut self.retained_backdrop, backdrop);
        let resources = self.resources.as_mut().context("resources missing")?;
        let previous_texture = resources.render_target.replace(cache.texture.clone());
        let previous_view =
            std::mem::replace(&mut resources.render_target_view, cache.view.clone());
        let previous_viewport = resources.viewport;
        resources.viewport.Width = bounds.size.width.0.max(1.0);
        resources.viewport.Height = bounds.size.height.0.max(1.0);
        let previous_isolation = self.isolated_layer;
        self.set_layer_isolation(true);
        let result = self
            .unbind_layer_textures()
            .and_then(|()| self.pre_draw(&[0.0; 4]))
            .and_then(|()| {
                self.draw_retained_range(
                    scene,
                    plan,
                    dependent,
                    layer.paint_range.clone(),
                    &plan.children[index],
                    bounds.origin,
                )
            });
        // Restore the parent target even when an upload or draw fails.
        let resources = self.resources.as_mut().context("resources missing")?;
        resources.render_target = previous_texture;
        resources.render_target_view = previous_view;
        resources.viewport = previous_viewport;
        self.retained_backdrop = previous_backdrop;
        self.set_layer_isolation(previous_isolation);
        let restore = self
            .unbind_layer_textures()
            .and_then(|()| self.bind_render_target(None));
        result?;
        restore?;
        self.retained_layers.insert(key.clone(), cache);
        Ok(())
    }

    pub(super) fn draw_retained_range(
        &mut self,
        scene: &Scene,
        plan: &RetainedScenePlan,
        dependent: &[bool],
        range: Range<usize>,
        children: &[usize],
        origin: Point<ScaledPixels>,
    ) -> Result<()> {
        let children = composite_children(scene, plan, dependent, children);
        for (range, child) in paint_segments(range, &children, &scene.retained_layers) {
            if !range.is_empty() {
                let mut content = scene.clone_paint_range(range);
                localize_scene(&mut content, origin);
                self.draw_scene(&content)?;
            }
            if let Some(child) = child {
                let layer = &scene.retained_layers[child];
                if dependent[child] {
                    self.render_retained_layer(scene, plan, dependent, child, origin)?;
                }
                // Successful preparation leaves no texture for empty or fully clipped output.
                let Some(cache) = self.retained_layers.get(&plan.keys[child]) else {
                    continue;
                };
                let bounds = cache.bounds;
                let srv = cache.srv.clone();
                self.draw_retained_layer(layer, bounds, &srv, origin)?;
            }
        }
        Ok(())
    }

    fn draw_retained_layer(
        &mut self,
        layer: &gpui::RetainedLayer,
        bounds: Bounds<ScaledPixels>,
        srv: &Option<ID3D11ShaderResourceView>,
        origin: Point<ScaledPixels>,
    ) -> Result<()> {
        let positions = [
            bounds.origin,
            point(bounds.right(), bounds.top()),
            point(bounds.left(), bounds.bottom()),
            bounds.bottom_right(),
        ]
        .map(|position| layer.transform.apply_scaled(position) - origin);
        let mut content_mask = if layer.transform.is_unit() {
            // Primitive shaders already applied this inherited rounded coverage.
            ContentMask::new(layer.content_mask.bounds)
        } else {
            layer.content_mask.clone()
        };
        localize_mask(&mut content_mask, origin);
        let sprite = RetainedLayerSprite {
            positions,
            content_mask,
            opacity: layer.opacity.clamp(0.0, 1.0),
            _pad: [0; 3],
        };
        let devices = self.devices.as_ref().context("devices missing")?;
        let resources = self.resources.as_ref().context("resources missing")?;
        self.pipelines.retained_layer.update_buffer(
            &devices.device,
            &devices.device_context,
            &[sprite],
        )?;
        self.pipelines.retained_layer.draw_with_texture(
            &devices.device_context,
            slice::from_ref(srv),
            slice::from_ref(&resources.viewport),
            slice::from_ref(&self.globals.global_params_buffer),
            slice::from_ref(&self.globals.blur_sampler),
            1,
        )?;
        self.unbind_layer_textures()
    }

    fn unbind_layer_textures(&self) -> Result<()> {
        let context = &self
            .devices
            .as_ref()
            .context("devices missing")?
            .device_context;
        unsafe {
            context.VSSetShaderResources(0, Some(&[None]));
            context.PSSetShaderResources(0, Some(&[None]));
        }
        Ok(())
    }

    fn set_layer_isolation(&mut self, isolated: bool) {
        self.isolated_layer = isolated;
        self.pipelines.shadow_pipeline.isolated = isolated;
        self.pipelines.backdrop_blur_pipeline.isolated = isolated;
        self.pipelines.quad_pipeline.isolated = isolated;
        self.pipelines.path_sprite_pipeline.isolated = isolated;
        self.pipelines.underline_pipeline.isolated = isolated;
        self.pipelines.mono_sprites.isolated = isolated;
        self.pipelines.subpixel_sprites.isolated = isolated;
        self.pipelines.poly_sprites.isolated = isolated;
    }

    fn capture_retained_backdrop(
        &mut self,
        layer: &gpui::RetainedLayer,
        bounds: Bounds<ScaledPixels>,
        parent_origin: Point<ScaledPixels>,
    ) -> Result<RetainedBackdrop> {
        let devices = self.devices.as_ref().context("devices missing")?;
        let resources = self.resources.as_ref().context("resources missing")?;
        let parent_size = size(
            ScaledPixels(resources.viewport.Width),
            ScaledPixels(resources.viewport.Height),
        );
        let source = resources
            .render_target
            .as_ref()
            .context("render target missing")?;
        let (snapshot, snapshot_view, snapshot_srv) = create_backdrop_blur_texture_and_views(
            &devices.device,
            parent_size.width.0 as u32,
            parent_size.height.0 as u32,
        )?;
        let (texture, view, srv) = create_backdrop_blur_texture_and_views(
            &devices.device,
            (bounds.size.width.0 as u32).max(1),
            (bounds.size.height.0 as u32).max(1),
        )?;
        let external = self
            .retained_backdrop
            .as_ref()
            .and_then(|backdrop| backdrop.srv.clone());
        let params = BackdropProjection {
            uvs: backdrop_projection_uvs(
                bounds,
                layer.transform,
                Bounds::new(parent_origin, parent_size),
            ),
        };
        unsafe {
            devices.device_context.OMSetRenderTargets(None, None);
            devices.device_context.CopyResource(&snapshot, source);
        }
        let viewport = D3D11_VIEWPORT {
            Width: bounds.size.width.0.max(1.0),
            Height: bounds.size.height.0.max(1.0),
            MaxDepth: 1.0,
            ..Default::default()
        };
        let result = (|| {
            if external.is_some() {
                // Compose in parent pixels before resampling through the child transform.
                draw_backdrop_projection(
                    devices,
                    &self.globals,
                    &mut self.pipelines.backdrop_underlay,
                    &snapshot_view,
                    &resources.viewport,
                    &external,
                    BackdropProjection {
                        uvs: [
                            point(0.0, 0.0),
                            point(1.0, 0.0),
                            point(0.0, 1.0),
                            point(1.0, 1.0),
                        ],
                    },
                )?;
            }
            draw_backdrop_projection(
                devices,
                &self.globals,
                &mut self.pipelines.backdrop_projection,
                &view,
                &viewport,
                &snapshot_srv,
                params,
            )
        })();
        let restore = self
            .unbind_layer_textures()
            .and_then(|()| self.bind_render_target(None));
        result?;
        restore?;
        Ok(RetainedBackdrop {
            _texture: texture,
            srv,
        })
    }

    pub(super) fn fill_retained_backdrop(&mut self, scratch: BackdropScratchBounds) -> Result<()> {
        let devices = self.devices.as_ref().context("devices missing")?;
        let resources = self.resources.as_ref().context("resources missing")?;
        let backdrop = self
            .retained_backdrop
            .as_ref()
            .context("retained backdrop missing")?;
        let params = BackdropProjection {
            uvs: backdrop_projection_uvs(
                Bounds::new(
                    scratch.bounds.origin,
                    size(
                        ScaledPixels(scratch.texture_size.width.0 as f32),
                        ScaledPixels(scratch.texture_size.height.0 as f32),
                    ),
                ),
                TransformationMatrix::unit(),
                Bounds::new(
                    Point::default(),
                    size(
                        ScaledPixels(resources.viewport.Width),
                        ScaledPixels(resources.viewport.Height),
                    ),
                ),
            ),
        };
        let viewport = D3D11_VIEWPORT {
            Width: scratch.texture_size.width.0 as f32,
            Height: scratch.texture_size.height.0 as f32,
            MaxDepth: 1.0,
            ..Default::default()
        };
        // The copied local content stays above the external parent snapshot.
        let result = draw_backdrop_projection(
            devices,
            &self.globals,
            &mut self.pipelines.backdrop_underlay,
            &resources.backdrop_view,
            &viewport,
            &backdrop.srv,
            params,
        );
        let restore = self
            .unbind_layer_textures()
            .and_then(|()| self.bind_render_target(None));
        result?;
        restore
    }
}

fn draw_backdrop_projection(
    devices: &DirectXRendererDevices,
    globals: &DirectXGlobalElements,
    pipeline: &mut PipelineState<BackdropProjection>,
    view: &Option<ID3D11RenderTargetView>,
    viewport: &D3D11_VIEWPORT,
    source: &Option<ID3D11ShaderResourceView>,
    params: BackdropProjection,
) -> Result<()> {
    pipeline.update_buffer(&devices.device, &devices.device_context, &[params])?;
    unsafe {
        devices
            .device_context
            .OMSetRenderTargets(Some(slice::from_ref(view)), None);
    }
    pipeline.draw_with_texture(
        &devices.device_context,
        slice::from_ref(source),
        slice::from_ref(viewport),
        slice::from_ref(&globals.global_params_buffer),
        slice::from_ref(&globals.blur_sampler),
        1,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::{
        Win32::{
            Foundation::HMODULE,
            UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, WS_POPUP},
        },
        core::w,
    };

    struct TestWindow(HWND);

    impl Drop for TestWindow {
        fn drop(&mut self) {
            unsafe { DestroyWindow(self.0) }.expect("destroy test window");
        }
    }

    fn warp_renderer() -> (TestWindow, DirectXRenderer) {
        let hwnd = unsafe {
            CreateWindowExW(
                Default::default(),
                w!("STATIC"),
                w!("GPUI retained regression"),
                WS_POPUP,
                0,
                0,
                64,
                64,
                None,
                None,
                None,
                None,
            )
        }
        .expect("create hidden test window");
        let window = TestWindow(hwnd);
        let mut device = None;
        let mut context = None;
        unsafe {
            D3D11CreateDevice(
                None::<&IDXGIAdapter>,
                D3D_DRIVER_TYPE_WARP,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&[D3D_FEATURE_LEVEL_11_0]),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
        }
        .expect("create WARP device");
        let dxgi_factory: IDXGIFactory6 =
            unsafe { CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS::default()) }.unwrap();
        let adapter = unsafe { dxgi_factory.EnumWarpAdapter() }.unwrap();
        let devices = DirectXDevices {
            adapter,
            dxgi_factory,
            device: device.unwrap(),
            device_context: context.unwrap(),
            renderer_selection: RendererSelection::Software,
        };
        let mut renderer = DirectXRenderer::new(hwnd, &devices, true).unwrap();
        renderer
            .resize(size(DevicePixels(64), DevicePixels(64)))
            .unwrap();
        (window, renderer)
    }

    fn bounds(x: f32, y: f32, width: f32, height: f32) -> Bounds<ScaledPixels> {
        Bounds::new(
            point(ScaledPixels(x), ScaledPixels(y)),
            size(ScaledPixels(width), ScaledPixels(height)),
        )
    }

    fn quad(scene: &mut Scene, bounds: Bounds<ScaledPixels>, color: u32) {
        scene.insert_primitive(Quad {
            bounds,
            content_mask: ContentMask::new(self::bounds(0.0, 0.0, 64.0, 64.0)),
            background: gpui::rgb(color).into(),
            ..Default::default()
        });
    }

    fn layer(
        id: &'static str,
        bounds: Bounds<ScaledPixels>,
        range: Range<usize>,
        opacity: f32,
    ) -> gpui::RetainedLayer {
        let mut global_id = GlobalElementId::default();
        *global_id = vec![id.into()].into();
        gpui::RetainedLayer {
            id: global_id,
            bounds,
            content_mask: ContentMask::new(self::bounds(0.0, 0.0, 64.0, 64.0)),
            paint_range: range,
            opacity,
            content_revision: 1.into(),
            content_dirty: false,
            transform: TransformationMatrix::unit(),
        }
    }

    fn pixels(renderer: &DirectXRenderer) -> Vec<[u8; 4]> {
        let devices = renderer.devices.as_ref().unwrap();
        let source = renderer
            .resources
            .as_ref()
            .unwrap()
            .render_target
            .as_ref()
            .unwrap();
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        unsafe { source.GetDesc(&mut desc) };
        desc.Usage = D3D11_USAGE_STAGING;
        desc.BindFlags = 0;
        desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
        desc.MiscFlags = 0;
        let mut staging = None;
        unsafe {
            devices
                .device
                .CreateTexture2D(&desc, None, Some(&mut staging))
        }
        .unwrap();
        let staging = staging.unwrap();
        unsafe {
            devices.device_context.OMSetRenderTargets(None, None);
            devices.device_context.CopyResource(&staging, source);
        }
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        unsafe {
            devices
                .device_context
                .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
        }
        .unwrap();
        let mut pixels = Vec::new();
        for y in 0..desc.Height {
            for x in 0..desc.Width {
                let offset = (y * mapped.RowPitch + x * 4) as usize;
                pixels.push(unsafe { *(mapped.pData.cast::<u8>().add(offset).cast::<[u8; 4]>()) });
            }
        }
        unsafe { devices.device_context.Unmap(&staging, 0) };
        pixels
    }

    fn assert_pixel(pixels: &[[u8; 4]], x: usize, y: usize, expected: [u8; 4]) {
        let actual = pixels[y * 64 + x];
        assert!(
            actual
                .iter()
                .zip(expected)
                .all(|(&actual, expected)| actual.abs_diff(expected) <= 2),
            "pixel ({x}, {y}): actual {actual:?}, expected {expected:?}"
        );
    }

    #[test]
    fn warp_retained_cards_preserve_nested_opacity_order_and_warm_transforms() {
        let (_window, mut renderer) = warp_renderer();
        let mut scene = Scene::default();
        quad(&mut scene, bounds(0.0, 0.0, 64.0, 64.0), 0x0000ff);
        quad(&mut scene, bounds(8.0, 8.0, 20.0, 20.0), 0xff0000);
        quad(&mut scene, bounds(12.0, 12.0, 8.0, 8.0), 0xffff00);
        quad(&mut scene, bounds(44.0, 8.0, 12.0, 20.0), 0x00ff00);
        quad(&mut scene, bounds(27.0, 14.0, 2.0, 2.0), 0xffffff);
        scene.retained_layers = vec![
            layer("child", bounds(12.0, 12.0, 8.0, 8.0), 2..3, 0.5),
            layer("parent", bounds(8.0, 8.0, 20.0, 20.0), 1..3, 0.5),
            layer("sibling", bounds(44.0, 8.0, 12.0, 20.0), 3..4, 0.25),
        ];
        scene.retained_layers[1].transform.translation = [12.0, 0.0];
        scene.finish();
        renderer
            .draw_frame(&scene, WindowBackgroundAppearance::Opaque)
            .unwrap();
        let cold = pixels(&renderer);
        assert_pixel(&cold, 10, 10, [255, 0, 0, 255]);
        assert_pixel(&cold, 22, 10, [127, 0, 128, 255]);
        assert_pixel(&cold, 25, 15, [127, 64, 128, 255]);
        assert_pixel(&cold, 27, 15, [255, 255, 255, 255]);
        assert_pixel(&cold, 48, 15, [191, 64, 0, 255]);

        let plan = RetainedScenePlan::new(&scene).unwrap();
        let texture = renderer.retained_layers[&plan.keys[1]].texture.as_raw();
        scene.retained_layers[1].transform.translation = [16.0, 0.0];
        scene.retained_layers[1].opacity = 0.25;
        renderer
            .draw_frame(&scene, WindowBackgroundAppearance::Opaque)
            .unwrap();
        assert_eq!(
            renderer.retained_layers[&plan.keys[1]].texture.as_raw(),
            texture
        );
        let warm = pixels(&renderer);
        assert_pixel(&warm, 22, 10, [255, 0, 0, 255]);
        assert_pixel(&warm, 25, 10, [191, 0, 64, 255]);
        assert_pixel(&warm, 30, 16, [191, 32, 64, 255]);

        scene.retained_layers[0].opacity = 0.0;
        renderer
            .draw_frame(&scene, WindowBackgroundAppearance::Opaque)
            .unwrap();
        assert_pixel(&pixels(&renderer), 30, 16, [191, 0, 64, 255]);
        scene.retained_layers[0].opacity = 1.0;
        scene.retained_layers[0].transform.translation = [4.0, 0.0];
        renderer
            .draw_frame(&scene, WindowBackgroundAppearance::Opaque)
            .unwrap();
        let nested = pixels(&renderer);
        assert_pixel(&nested, 30, 16, [191, 0, 64, 255]);
        assert_pixel(&nested, 34, 16, [191, 64, 64, 255]);
        scene.retained_layers[1].content_mask = ContentMask::new(bounds(0.0, 0.0, 30.0, 64.0));
        renderer
            .draw_frame(&scene, WindowBackgroundAppearance::Opaque)
            .unwrap();
        assert_pixel(&pixels(&renderer), 32, 16, [255, 0, 0, 255]);
        renderer
            .resize(size(DevicePixels(80), DevicePixels(64)))
            .unwrap();
        assert!(renderer.retained_layers.is_empty());
    }

    #[test]
    fn retained_sprite_preserves_hlsl_stride() {
        assert_eq!(std::mem::size_of::<RetainedLayerSprite>(), 96);
        assert_eq!(std::mem::size_of::<BackdropProjection>(), 32);
    }

    #[test]
    fn warp_retained_group_alpha_uses_source_over_before_group_opacity() {
        let (_window, mut renderer) = warp_renderer();
        let mut scene = Scene::default();
        quad(&mut scene, bounds(0.0, 0.0, 64.0, 64.0), 0x0000ff);
        for _ in 0..2 {
            scene.insert_primitive(Quad {
                bounds: bounds(8.0, 8.0, 20.0, 20.0),
                content_mask: ContentMask::new(bounds(0.0, 0.0, 64.0, 64.0)),
                background: gpui::rgba(0xff000080).into(),
                ..Default::default()
            });
        }
        scene.retained_layers = vec![layer("alpha", bounds(8.0, 8.0, 20.0, 20.0), 1..3, 0.5)];
        scene.finish();
        renderer
            .draw_frame(&scene, WindowBackgroundAppearance::Opaque)
            .unwrap();
        assert_pixel(&pixels(&renderer), 15, 15, [159, 0, 96, 255]);
        scene.clear();
        quad(&mut scene, bounds(0.0, 0.0, 64.0, 64.0), 0x0000ff);
        renderer
            .draw_frame(&scene, WindowBackgroundAppearance::Opaque)
            .unwrap();
        assert!(renderer.retained_layers.is_empty());
        assert_pixel(&pixels(&renderer), 15, 15, [255, 0, 0, 255]);
    }

    #[test]
    fn warp_retained_backdrop_groups_sample_the_current_parent_scene() {
        fn scene(left_color: u32, retained: bool) -> Scene {
            let mut scene = Scene::default();
            quad(&mut scene, bounds(0.0, 0.0, 32.0, 64.0), left_color);
            quad(&mut scene, bounds(32.0, 0.0, 32.0, 64.0), 0x0000ff);
            let blur_bounds = bounds(16.0, 16.0, 32.0, 32.0);
            scene.insert_primitive(BackdropBlur {
                order: 0,
                pad: 0,
                bounds: blur_bounds,
                content_mask: ContentMask::new(bounds(0.0, 0.0, 64.0, 64.0)),
                corner_radii: Corners::default(),
                blur_radius: ScaledPixels(8.0),
                source_origin_x: 0.0,
                source_origin_y: 0.0,
                source_width: 0.0,
                source_height: 0.0,
                opacity: 1.0,
            });
            let mut second_blur = scene.backdrop_blurs[0].clone();
            second_blur.bounds = bounds(24.0, 16.0, 24.0, 32.0);
            second_blur.blur_radius = ScaledPixels(12.0);
            second_blur.opacity = 0.5;
            scene.insert_primitive(second_blur);
            if retained {
                scene.retained_layers = vec![
                    layer("blur", blur_bounds, 2..3, 1.0),
                    layer("second-blur", blur_bounds, 3..4, 1.0),
                    layer("blur-parent", blur_bounds, 2..4, 1.0),
                ];
            }
            scene.finish();
            scene
        }

        let (_window, mut renderer) = warp_renderer();
        let colors = [0xff0000, 0x00ff00];
        let expected = colors.map(|color| {
            renderer
                .draw_frame(&scene(color, false), WindowBackgroundAppearance::Opaque)
                .unwrap();
            pixels(&renderer)
        });
        let boundary = expected[0][32 * 64 + 30];
        assert!(boundary[0] > 0 && boundary[2] > 0);
        assert_ne!(expected[0], expected[1]);
        for (color, expected) in colors.into_iter().zip(expected) {
            renderer
                .draw_frame(&scene(color, true), WindowBackgroundAppearance::Opaque)
                .unwrap();
            assert_eq!(pixels(&renderer), expected);
            assert!(renderer.retained_layers.is_empty());
        }
    }

    #[test]
    fn warp_retained_backdrop_affine_groups_preserve_parent_color_and_opacity() {
        fn scene(right_color: u32, transform: TransformationMatrix, nested: bool) -> Scene {
            let mut scene = Scene::default();
            quad(&mut scene, bounds(0.0, 0.0, 32.0, 64.0), 0xff0000);
            quad(&mut scene, bounds(32.0, 0.0, 32.0, 64.0), right_color);
            let group_bounds = bounds(0.0, 8.0, 32.0, 48.0);
            scene.insert_primitive(Quad {
                bounds: group_bounds,
                content_mask: ContentMask::new(bounds(-64.0, -64.0, 192.0, 192.0)),
                background: gpui::rgba(0x00ff0080).into(),
                ..Default::default()
            });
            scene.insert_primitive(BackdropBlur {
                order: 0,
                pad: 0,
                bounds: bounds(8.0, 16.0, 16.0, 32.0),
                content_mask: ContentMask::new(bounds(-64.0, -64.0, 192.0, 192.0)),
                corner_radii: Corners::default(),
                blur_radius: ScaledPixels(8.0),
                source_origin_x: 0.0,
                source_origin_y: 0.0,
                source_width: 0.0,
                source_height: 0.0,
                opacity: 1.0,
            });
            let mut child = layer("glass", group_bounds, 2..4, 0.5);
            child.transform = transform;
            if nested {
                child.transform.translation = [16.0, 0.0];
                let mut parent = layer("glass-parent", group_bounds, 2..4, 0.5);
                parent.transform.translation = [16.0, 0.0];
                scene.retained_layers = vec![child, parent];
            } else {
                scene.retained_layers = vec![child];
            }
            quad(&mut scene, bounds(46.0, 28.0, 4.0, 3.0), 0xffffff);
            scene.finish();
            scene
        }

        let (_window, mut renderer) = warp_renderer();
        // All transforms map the uniform interior to blue parent pixels around (48, 32).
        // The local green alpha is 1/2. Group opacity 1/2 gives 1/4 green plus 3/4 blue.
        let transforms = [
            TransformationMatrix {
                rotation_scale: [[1.0, 0.0], [0.0, 1.0]],
                translation: [32.0, 0.0],
            },
            TransformationMatrix {
                rotation_scale: [[-1.0, 0.0], [0.0, 1.0]],
                translation: [64.0, 0.0],
            },
            TransformationMatrix {
                rotation_scale: [[0.0, -1.0], [1.0, 0.0]],
                translation: [80.0, 16.0],
            },
            TransformationMatrix {
                rotation_scale: [[1.5, 0.0], [0.0, 0.5]],
                translation: [24.0, 16.0],
            },
        ];
        let mut ordinary = scene(0x0000ff, transforms[0], false);
        ordinary.retained_layers.clear();
        renderer
            .draw_frame(&ordinary, WindowBackgroundAppearance::Opaque)
            .unwrap();
        for transform in transforms {
            renderer
                .draw_frame(
                    &scene(0x0000ff, transform, false),
                    WindowBackgroundAppearance::Opaque,
                )
                .unwrap();
            let actual = pixels(&renderer);
            assert_pixel(&actual, 48, 32, [191, 64, 0, 255]);
            assert_pixel(&actual, 10, 10, [0, 0, 255, 255]);
            assert_pixel(&actual, 48, 29, [255, 255, 255, 255]);
        }
        renderer
            .draw_frame(
                &scene(0xff00ff, transforms[0], false),
                WindowBackgroundAppearance::Opaque,
            )
            .unwrap();
        assert_pixel(&pixels(&renderer), 48, 32, [191, 64, 191, 255]);
        renderer
            .draw_frame(
                &scene(0x0000ff, transforms[0], true),
                WindowBackgroundAppearance::Opaque,
            )
            .unwrap();
        assert_pixel(&pixels(&renderer), 48, 32, [223, 32, 0, 255]);
        assert!(!renderer.isolated_layer);
        assert!(renderer.retained_backdrop.is_none());
    }

    #[test]
    fn warp_retained_identity_and_nested_masks_preserve_rounded_coverage() {
        fn assert_image(actual: &[[u8; 4]], expected: &[[u8; 4]]) {
            for (index, (actual, expected)) in actual.iter().zip(expected).enumerate() {
                assert!(
                    actual
                        .iter()
                        .zip(expected)
                        .all(|(&actual, &expected)| actual.abs_diff(expected) <= 1),
                    "pixel {index}: actual {actual:?}, expected {expected:?}"
                );
            }
        }
        let (_window, mut renderer) = warp_renderer();
        let mut scene = Scene::default();
        quad(&mut scene, bounds(0.0, 0.0, 64.0, 64.0), 0x000000);
        let mask = ContentMask::rounded(
            bounds(8.0, 8.0, 48.0, 48.0),
            Corners::all(ScaledPixels(12.0)),
        );
        scene.insert_primitive(Quad {
            bounds: bounds(0.0, 0.0, 64.0, 64.0),
            content_mask: mask.clone(),
            background: gpui::rgb(0xffffff).into(),
            ..Default::default()
        });
        scene.finish();
        renderer
            .draw_frame(&scene, WindowBackgroundAppearance::Opaque)
            .unwrap();
        let expected = pixels(&renderer);
        assert!((1..255).contains(&expected[16 * 64 + 55][0]));
        let mut child = layer("rounded-child", mask.bounds, 1..2, 1.0);
        child.content_mask = mask.clone();
        let mut parent = layer("rounded-parent", mask.bounds, 1..2, 1.0);
        parent.content_mask = mask;
        scene.retained_layers = vec![child.clone()];
        renderer
            .draw_frame(&scene, WindowBackgroundAppearance::Opaque)
            .unwrap();
        assert_image(&pixels(&renderer), &expected);
        scene.retained_layers.push(parent.clone());
        renderer
            .draw_frame(&scene, WindowBackgroundAppearance::Opaque)
            .unwrap();
        assert_image(&pixels(&renderer), &expected);

        child.transform.translation = [8.0, 0.0];
        scene.retained_layers = vec![child];
        renderer
            .draw_frame(&scene, WindowBackgroundAppearance::Opaque)
            .unwrap();
        let transformed = pixels(&renderer);
        assert_pixel(&transformed, 55, 16, expected[16 * 64 + 55]);
        assert_pixel(&transformed, 58, 32, [0, 0, 0, 255]);
        scene.retained_layers.push(parent);
        renderer
            .draw_frame(&scene, WindowBackgroundAppearance::Opaque)
            .unwrap();
        assert_image(&pixels(&renderer), &transformed);
    }

    #[test]
    fn warp_retained_fully_clipped_descendant_does_not_fail_the_frame() {
        let (_window, mut renderer) = warp_renderer();
        let mut scene = Scene::default();
        quad(&mut scene, bounds(0.0, 0.0, 64.0, 64.0), 0x0000ff);
        quad(&mut scene, bounds(8.0, 8.0, 16.0, 16.0), 0xff0000);
        scene.retained_layers = vec![
            layer("offscreen-child", bounds(8.0, 8.0, 16.0, 16.0), 1..2, 1.0),
            layer("parent", bounds(8.0, 8.0, 16.0, 16.0), 1..2, 1.0),
        ];
        scene.retained_layers[0].transform.translation = [20_000.0, 0.0];
        scene.finish();
        renderer
            .draw_frame(&scene, WindowBackgroundAppearance::Opaque)
            .unwrap();
        assert!(renderer.retained_layers.is_empty());
        assert!(
            pixels(&renderer)
                .iter()
                .all(|pixel| *pixel == [255, 0, 0, 255])
        );
        scene.retained_layers[0].transform = TransformationMatrix::unit();
        renderer
            .draw_frame(&scene, WindowBackgroundAppearance::Opaque)
            .unwrap();
        assert_pixel(&pixels(&renderer), 12, 12, [0, 0, 255, 255]);
        scene.retained_layers[0].transform.translation = [20_000.0, 0.0];
        renderer
            .draw_frame(&scene, WindowBackgroundAppearance::Opaque)
            .unwrap();
        assert!(renderer.retained_layers.is_empty());
        assert!(
            pixels(&renderer)
                .iter()
                .all(|pixel| *pixel == [255, 0, 0, 255])
        );
    }
}
