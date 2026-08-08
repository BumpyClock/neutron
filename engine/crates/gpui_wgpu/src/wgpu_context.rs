#[cfg(not(target_family = "wasm"))]
use anyhow::Context as _;
use gpui::RendererSelection;
#[cfg(not(target_family = "wasm"))]
use gpui_util::ResultExt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub struct WgpuContext {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: Arc<wgpu::Device>,
    pub queue: Arc<wgpu::Queue>,
    dual_source_blending: bool,
    device_lost: Arc<AtomicBool>,
    renderer_selection: RendererSelection,
}

#[derive(Clone, Copy)]
pub struct CompositorGpuHint {
    pub vendor_id: u32,
    pub device_id: u32,
}

#[cfg(not(target_family = "wasm"))]
// Recovery must preserve startup eligibility. Explicit software selection remains constrained to
// Vulkan CPU adapters by `accepts_adapter`; default selection may still use CPU as a fallback.
fn recovery_rejects_software(_selection: RendererSelection) -> bool {
    false
}

#[cfg(not(target_family = "wasm"))]
fn accepts_adapter(
    selection: RendererSelection,
    reject_software: bool,
    backend: wgpu::Backend,
    device_type: wgpu::DeviceType,
) -> bool {
    if selection.requires_software_adapter() {
        backend == wgpu::Backend::Vulkan && device_type == wgpu::DeviceType::Cpu
    } else {
        !reject_software || device_type != wgpu::DeviceType::Cpu
    }
}

impl WgpuContext {
    #[cfg(not(target_family = "wasm"))]
    pub fn new(compositor_gpu: Option<CompositorGpuHint>) -> anyhow::Result<Self> {
        Self::new_internal(
            Self::instance(),
            None,
            compositor_gpu,
            RendererSelection::from_environment()?,
            false,
        )
    }

    #[cfg(not(target_family = "wasm"))]
    pub fn new_for_surface(
        instance: wgpu::Instance,
        surface: &wgpu::Surface<'_>,
        compositor_gpu: Option<CompositorGpuHint>,
    ) -> anyhow::Result<Self> {
        Self::new_internal(
            instance,
            Some(surface),
            compositor_gpu,
            RendererSelection::from_environment()?,
            false,
        )
    }

    #[cfg(not(target_family = "wasm"))]
    pub fn new_for_surface_rejecting_software(
        instance: wgpu::Instance,
        surface: &wgpu::Surface<'_>,
        compositor_gpu: Option<CompositorGpuHint>,
    ) -> anyhow::Result<Self> {
        Self::new_internal(
            instance,
            Some(surface),
            compositor_gpu,
            RendererSelection::Default,
            true,
        )
    }

    #[cfg(not(target_family = "wasm"))]
    pub(crate) fn new_for_surface_recovery(
        instance: wgpu::Instance,
        surface: &wgpu::Surface<'_>,
        compositor_gpu: Option<CompositorGpuHint>,
        renderer_selection: RendererSelection,
    ) -> anyhow::Result<Self> {
        Self::new_internal(
            instance,
            Some(surface),
            compositor_gpu,
            renderer_selection,
            recovery_rejects_software(renderer_selection),
        )
    }

    #[cfg(not(target_family = "wasm"))]
    fn new_internal(
        instance: wgpu::Instance,
        compatible_surface: Option<&wgpu::Surface<'_>>,
        compositor_gpu: Option<CompositorGpuHint>,
        renderer_selection: RendererSelection,
        reject_software: bool,
    ) -> anyhow::Result<Self> {
        let device_id_filter = match std::env::var("ZED_DEVICE_ID") {
            Ok(val) => parse_pci_id(&val)
                .context("Failed to parse device ID from `ZED_DEVICE_ID` environment variable")
                .log_err(),
            Err(std::env::VarError::NotPresent) => None,
            err => {
                err.context("Failed to read value of `ZED_DEVICE_ID` environment variable")
                    .log_err();
                None
            }
        };

        let (adapter, device, queue, dual_source_blending_available) =
            pollster::block_on(Self::select_adapter_and_device(
                &instance,
                device_id_filter,
                compositor_gpu.as_ref(),
                compatible_surface,
                renderer_selection,
                reject_software,
            ))?;

        let adapter_info = adapter.get_info();
        log::info!(
            "Selected GPU adapter: {} (backend={:?}, type={:?}, vendor={:#06x}, device={:#06x}, selection={renderer_selection:?})",
            adapter_info.name,
            adapter_info.backend,
            adapter_info.device_type,
            adapter_info.vendor,
            adapter_info.device,
        );

        let device_lost = Arc::new(AtomicBool::new(false));
        device.set_device_lost_callback({
            let device_lost = Arc::clone(&device_lost);
            move |reason, message| {
                log::error!("wgpu device lost: reason={reason:?}, message={message}");
                if reason != wgpu::DeviceLostReason::Destroyed {
                    device_lost.store(true, Ordering::Relaxed);
                }
            }
        });

        Ok(Self {
            instance,
            adapter,
            device: Arc::new(device),
            queue: Arc::new(queue),
            dual_source_blending: dual_source_blending_available,
            device_lost,
            renderer_selection,
        })
    }

    #[cfg(not(target_family = "wasm"))]
    pub fn instance() -> wgpu::Instance {
        wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN | wgpu::Backends::GL,
            flags: wgpu::InstanceFlags::default(),
            backend_options: wgpu::BackendOptions::default(),
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
            display: None,
        })
    }

    #[cfg(target_family = "wasm")]
    pub async fn new_web() -> anyhow::Result<Self> {
        let renderer_selection = RendererSelection::from_environment()?;
        anyhow::ensure!(
            !renderer_selection.requires_software_adapter(),
            "software renderer selection is not supported on web"
        );
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::BROWSER_WEBGPU | wgpu::Backends::GL,
            flags: wgpu::InstanceFlags::default(),
            backend_options: wgpu::BackendOptions::default(),
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
            display: None,
        });

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
            })
            .await
            .map_err(|e| anyhow::anyhow!("Failed to request GPU adapter: {e}"))?;

        let adapter_info = adapter.get_info();
        log::info!(
            "Selected GPU adapter: {} (backend={:?}, type={:?}, vendor={:#06x}, device={:#06x}, selection={renderer_selection:?})",
            adapter_info.name,
            adapter_info.backend,
            adapter_info.device_type,
            adapter_info.vendor,
            adapter_info.device,
        );

        let dual_source_blending_available = adapter
            .features()
            .contains(wgpu::Features::DUAL_SOURCE_BLENDING);

        let mut required_features = wgpu::Features::empty();
        if dual_source_blending_available {
            required_features |= wgpu::Features::DUAL_SOURCE_BLENDING;
        } else {
            log::warn!(
                "Dual-source blending not available on this GPU. \
                Subpixel text antialiasing will be disabled."
            );
        }

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("gpui_device"),
                required_features,
                required_limits: wgpu::Limits::downlevel_defaults()
                    .using_resolution(adapter.limits())
                    .using_alignment(adapter.limits()),
                memory_hints: wgpu::MemoryHints::MemoryUsage,
                trace: wgpu::Trace::Off,
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
            })
            .await
            .map_err(|e| anyhow::anyhow!("Failed to create wgpu device: {e}"))?;

        let device_lost = Arc::new(AtomicBool::new(false));
        device.set_device_lost_callback({
            let device_lost = Arc::clone(&device_lost);
            move |reason, message| {
                log::error!("wgpu device lost: reason={reason:?}, message={message}");
                if reason != wgpu::DeviceLostReason::Destroyed {
                    device_lost.store(true, Ordering::Relaxed);
                }
            }
        });

        Ok(Self {
            instance,
            adapter,
            device: Arc::new(device),
            queue: Arc::new(queue),
            dual_source_blending: dual_source_blending_available,
            device_lost,
            renderer_selection,
        })
    }

    #[cfg(not(target_family = "wasm"))]
    async fn select_adapter_and_device(
        instance: &wgpu::Instance,
        device_id_filter: Option<u32>,
        compositor_gpu: Option<&CompositorGpuHint>,
        compatible_surface: Option<&wgpu::Surface<'_>>,
        renderer_selection: RendererSelection,
        reject_software: bool,
    ) -> anyhow::Result<(wgpu::Adapter, wgpu::Device, wgpu::Queue, bool)> {
        let backends = if renderer_selection.requires_software_adapter() {
            wgpu::Backends::VULKAN
        } else {
            wgpu::Backends::all()
        };
        let mut adapters: Vec<_> = instance.enumerate_adapters(backends).await;

        if adapters.is_empty() {
            if renderer_selection.requires_software_adapter() {
                anyhow::bail!("No Vulkan GPU adapters found for software renderer selection");
            }
            anyhow::bail!("No GPU adapters found");
        }

        if let Some(device_id) = device_id_filter {
            log::info!("ZED_DEVICE_ID filter: {:#06x}", device_id);
        }

        // Sort adapters into a single priority order. Tiers (from highest to lowest):
        //
        // 1. ZED_DEVICE_ID match — explicit user override
        // 2. Compositor GPU match — the GPU the display server is rendering on
        // 3. Device type — WGPU HighPerformance order (Discrete > Integrated >
        //    Other > Virtual > Cpu). "Other" ranks above "Virtual" because
        //    backends like OpenGL may report real hardware as "Other".
        // 4. Backend — prefer Vulkan/Metal/Dx12 over GL/etc.
        adapters.sort_by_key(|adapter| {
            let info = adapter.get_info();

            // Backends like OpenGL report device=0 for all adapters, so
            // device-based matching is only meaningful when non-zero.
            let device_known = info.device != 0;

            let user_override: u8 = match device_id_filter {
                Some(id) if device_known && info.device == id => 0,
                _ => 1,
            };

            let compositor_match: u8 = match compositor_gpu {
                Some(hint)
                    if device_known
                        && info.vendor == hint.vendor_id
                        && info.device == hint.device_id =>
                {
                    0
                }
                _ => 1,
            };

            let type_priority: u8 = match info.device_type {
                wgpu::DeviceType::DiscreteGpu => 0,
                wgpu::DeviceType::IntegratedGpu => 1,
                wgpu::DeviceType::Other => 2,
                wgpu::DeviceType::VirtualGpu => 3,
                wgpu::DeviceType::Cpu => 4,
            };

            let backend_priority: u8 = match info.backend {
                wgpu::Backend::Vulkan => 0,
                wgpu::Backend::Metal => 0,
                wgpu::Backend::Dx12 => 0,
                _ => 1,
            };

            (
                user_override,
                compositor_match,
                type_priority,
                backend_priority,
            )
        });

        // Log all available adapters (in sorted order)
        log::info!("Found {} GPU adapter(s):", adapters.len());
        for adapter in &adapters {
            let info = adapter.get_info();
            log::info!(
                "  - {} (vendor={:#06x}, device={:#06x}, backend={:?}, type={:?})",
                info.name,
                info.vendor,
                info.device,
                info.backend,
                info.device_type,
            );
        }

        for adapter in adapters {
            let info = adapter.get_info();
            if !accepts_adapter(
                renderer_selection,
                reject_software,
                info.backend,
                info.device_type,
            ) {
                let reason = if renderer_selection.requires_software_adapter() {
                    "software selection requires a Vulkan CPU adapter"
                } else {
                    "software adapter rejected during recovery"
                };
                log::info!(
                    "Skipping GPU adapter {} ({:?}, {:?}): {reason}",
                    info.name,
                    info.backend,
                    info.device_type,
                );
                continue;
            }

            let result = if let Some(surface) = compatible_surface {
                Self::try_adapter_with_surface(&adapter, surface).await
            } else {
                Self::create_device(&adapter).await
            };
            match result {
                Ok((device, queue, dual_source_blending)) => {
                    if !dual_source_blending {
                        log::warn!(
                            "Dual-source blending not available on this GPU. \
                             Subpixel text antialiasing will be disabled."
                        );
                    }
                    anyhow::ensure!(
                        !renderer_selection.requires_software_adapter()
                            || (info.backend == wgpu::Backend::Vulkan
                                && info.device_type == wgpu::DeviceType::Cpu),
                        "software renderer selection chose a non-Vulkan CPU adapter"
                    );
                    return Ok((adapter, device, queue, dual_source_blending));
                }
                Err(error) => {
                    log::info!(
                        "Adapter {} ({:?}) failed: {error:#}; trying next",
                        info.name,
                        info.backend
                    );
                }
            }
        }

        if renderer_selection.requires_software_adapter() {
            anyhow::bail!("No Vulkan CPU GPU adapter found that can configure the display surface")
        }
        anyhow::bail!("No GPU adapter found that can configure the display surface")
    }

    #[cfg(not(target_family = "wasm"))]
    async fn create_device(
        adapter: &wgpu::Adapter,
    ) -> anyhow::Result<(wgpu::Device, wgpu::Queue, bool)> {
        let dual_source_blending = adapter
            .features()
            .contains(wgpu::Features::DUAL_SOURCE_BLENDING);
        let required_features = if dual_source_blending {
            wgpu::Features::DUAL_SOURCE_BLENDING
        } else {
            wgpu::Features::empty()
        };
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("gpui_device"),
                required_features,
                required_limits: wgpu::Limits::downlevel_defaults()
                    .using_resolution(adapter.limits())
                    .using_alignment(adapter.limits()),
                memory_hints: wgpu::MemoryHints::MemoryUsage,
                trace: wgpu::Trace::Off,
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
            })
            .await
            .map_err(|error| anyhow::anyhow!("Failed to create wgpu device: {error}"))?;
        Ok((device, queue, dual_source_blending))
    }

    #[cfg(not(target_family = "wasm"))]
    async fn try_adapter_with_surface(
        adapter: &wgpu::Adapter,
        surface: &wgpu::Surface<'_>,
    ) -> anyhow::Result<(wgpu::Device, wgpu::Queue, bool)> {
        let capabilities = surface.get_capabilities(adapter);
        let format = capabilities
            .formats
            .first()
            .copied()
            .ok_or_else(|| anyhow::anyhow!("no compatible surface formats"))?;
        let alpha_mode = capabilities
            .alpha_modes
            .first()
            .copied()
            .ok_or_else(|| anyhow::anyhow!("no compatible alpha modes"))?;
        let present_mode = if capabilities
            .present_modes
            .contains(&wgpu::PresentMode::Fifo)
        {
            wgpu::PresentMode::Fifo
        } else {
            capabilities
                .present_modes
                .first()
                .copied()
                .ok_or_else(|| anyhow::anyhow!("no compatible present modes"))?
        };

        let (device, queue, dual_source_blending) = Self::create_device(adapter).await?;
        let error_scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        surface.configure(
            &device,
            &wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format,
                width: 64,
                height: 64,
                present_mode,
                desired_maximum_frame_latency: 2,
                alpha_mode,
                view_formats: vec![],
            },
        );
        if let Some(error) = error_scope.pop().await {
            anyhow::bail!("surface configuration failed: {error}");
        }
        Ok((device, queue, dual_source_blending))
    }

    pub fn supports_dual_source_blending(&self) -> bool {
        self.dual_source_blending
    }

    pub fn renderer_selection(&self) -> RendererSelection {
        self.renderer_selection
    }

    pub fn check_compatible_with_surface(&self, surface: &wgpu::Surface<'_>) -> anyhow::Result<()> {
        let capabilities = surface.get_capabilities(&self.adapter);
        anyhow::ensure!(
            !capabilities.formats.is_empty() && !capabilities.alpha_modes.is_empty(),
            "shared GPU adapter is incompatible with the window surface"
        );
        Ok(())
    }

    /// Returns true if the GPU device was lost (e.g., due to driver crash, suspend/resume).
    /// When this returns true, the context should be recreated.
    pub fn device_lost(&self) -> bool {
        self.device_lost.load(Ordering::Relaxed)
    }

    /// Returns a clone of the device_lost flag for sharing with renderers.
    pub(crate) fn device_lost_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.device_lost)
    }
}

#[cfg(not(target_family = "wasm"))]
fn parse_pci_id(id: &str) -> anyhow::Result<u32> {
    let mut id = id.trim();

    if id.starts_with("0x") || id.starts_with("0X") {
        id = &id[2..];
    }
    let is_hex_string = id.chars().all(|c| c.is_ascii_hexdigit());
    let is_4_chars = id.len() == 4;
    anyhow::ensure!(
        is_4_chars && is_hex_string,
        "Expected a 4 digit PCI ID in hexadecimal format"
    );

    u32::from_str_radix(id, 16).context("parsing PCI ID as hex")
}

#[cfg(all(test, not(target_family = "wasm")))]
mod tests {
    use super::{accepts_adapter, parse_pci_id, recovery_rejects_software};
    use gpui::RendererSelection;

    #[test]
    fn software_selection_requires_a_vulkan_cpu_adapter() {
        assert!(accepts_adapter(
            RendererSelection::Software,
            false,
            wgpu::Backend::Vulkan,
            wgpu::DeviceType::Cpu,
        ));
        assert!(accepts_adapter(
            RendererSelection::Software,
            true,
            wgpu::Backend::Vulkan,
            wgpu::DeviceType::Cpu,
        ));
        assert!(!accepts_adapter(
            RendererSelection::Software,
            false,
            wgpu::Backend::Gl,
            wgpu::DeviceType::Cpu,
        ));
        assert!(!accepts_adapter(
            RendererSelection::Software,
            false,
            wgpu::Backend::Vulkan,
            wgpu::DeviceType::DiscreteGpu,
        ));
        assert!(!accepts_adapter(
            RendererSelection::Default,
            true,
            wgpu::Backend::Vulkan,
            wgpu::DeviceType::Cpu,
        ));
    }

    #[test]
    fn default_recovery_preserves_initial_cpu_fallback_policy() {
        let selection = RendererSelection::Default;
        assert!(accepts_adapter(
            selection,
            false,
            wgpu::Backend::Vulkan,
            wgpu::DeviceType::Cpu,
        ));
        assert!(accepts_adapter(
            selection,
            recovery_rejects_software(selection),
            wgpu::Backend::Vulkan,
            wgpu::DeviceType::Cpu,
        ));
    }

    #[test]
    fn test_parse_device_id() {
        assert!(parse_pci_id("0xABCD").is_ok());
        assert!(parse_pci_id("ABCD").is_ok());
        assert!(parse_pci_id("abcd").is_ok());
        assert!(parse_pci_id("1234").is_ok());
        assert!(parse_pci_id("123").is_err());
        assert_eq!(
            parse_pci_id(&format!("{:x}", 0x1234)).unwrap(),
            parse_pci_id(&format!("{:X}", 0x1234)).unwrap(),
        );

        assert_eq!(
            parse_pci_id(&format!("{:#x}", 0x1234)).unwrap(),
            parse_pci_id(&format!("{:#X}", 0x1234)).unwrap(),
        );
    }
}
