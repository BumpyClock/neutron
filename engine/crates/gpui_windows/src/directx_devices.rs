use anyhow::{Context, Result};
use gpui::RendererSelection;
use gpui_util::ResultExt;
use itertools::Itertools;
use windows::Win32::{
    Foundation::HMODULE,
    Graphics::{
        Direct3D::{
            D3D_DRIVER_TYPE, D3D_DRIVER_TYPE_UNKNOWN, D3D_DRIVER_TYPE_WARP, D3D_FEATURE_LEVEL,
            D3D_FEATURE_LEVEL_10_1, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1,
        },
        Direct3D11::{
            D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_DEBUG,
            D3D11_FEATURE_D3D10_X_HARDWARE_OPTIONS, D3D11_FEATURE_DATA_D3D10_X_HARDWARE_OPTIONS,
            D3D11_SDK_VERSION, D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext,
        },
        Dxgi::{
            CreateDXGIFactory2, DXGI_ADAPTER_FLAG_SOFTWARE, DXGI_CREATE_FACTORY_DEBUG,
            DXGI_CREATE_FACTORY_FLAGS, IDXGIAdapter, IDXGIAdapter1, IDXGIFactory4, IDXGIFactory6,
        },
    },
};
use windows::core::Interface;

pub(crate) fn try_to_recover_from_device_lost<T>(mut f: impl FnMut() -> Result<T>) -> Result<T> {
    (0..5)
        .map(|i| {
            if i > 0 {
                // Add a small delay before retrying
                std::thread::sleep(std::time::Duration::from_millis(100 + i * 10));
            }
            f()
        })
        .find_or_last(Result::is_ok)
        .unwrap()
        .context("DirectXRenderer failed to recover from lost device after multiple attempts")
}

#[derive(Clone)]
pub(crate) struct DirectXDevices {
    pub(crate) adapter: IDXGIAdapter1,
    pub(crate) dxgi_factory: IDXGIFactory6,
    pub(crate) renderer_selection: RendererSelection,
    pub(crate) device: ID3D11Device,
    pub(crate) device_context: ID3D11DeviceContext,
}

impl DirectXDevices {
    pub(crate) fn new() -> Result<Self> {
        let renderer_selection =
            RendererSelection::from_environment().context("Reading renderer selection")?;
        let debug_layer_available = check_debug_layer_available();
        let dxgi_factory =
            get_dxgi_factory(debug_layer_available).context("Creating DXGI factory")?;
        let (adapter, device, device_context, feature_level) =
            get_adapter(&dxgi_factory, debug_layer_available, renderer_selection)
                .context("Getting DXGI adapter")?;
        match feature_level {
            D3D_FEATURE_LEVEL_11_1 => {
                log::info!("Created device with Direct3D 11.1 feature level.")
            }
            D3D_FEATURE_LEVEL_11_0 => {
                log::info!("Created device with Direct3D 11.0 feature level.")
            }
            D3D_FEATURE_LEVEL_10_1 => {
                log::info!("Created device with Direct3D 10.1 feature level.")
            }
            _ => unreachable!(),
        }

        let desc = unsafe { adapter.GetDesc1() }.context("Reading DXGI adapter description")?;
        let name = String::from_utf16_lossy(&desc.Description)
            .trim_matches(char::from(0))
            .to_string();
        log::info!(
            "Selected Direct3D adapter: {name} (vendor={:#06x}, device={:#06x}, software={}, selection={renderer_selection:?})",
            desc.VendorId,
            desc.DeviceId,
            (desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32) != 0,
        );

        Ok(Self {
            adapter,
            dxgi_factory,
            renderer_selection,
            device,
            device_context,
        })
    }
}

#[inline]
fn check_debug_layer_available() -> bool {
    #[cfg(debug_assertions)]
    {
        use windows::Win32::Graphics::Dxgi::{DXGIGetDebugInterface1, IDXGIInfoQueue};

        unsafe { DXGIGetDebugInterface1::<IDXGIInfoQueue>(0) }
            .log_err()
            .is_some()
    }
    #[cfg(not(debug_assertions))]
    {
        false
    }
}

#[inline]
fn get_dxgi_factory(debug_layer_available: bool) -> Result<IDXGIFactory6> {
    let factory_flag = if debug_layer_available {
        DXGI_CREATE_FACTORY_DEBUG
    } else {
        #[cfg(debug_assertions)]
        log::warn!(
            "Failed to get DXGI debug interface. DirectX debugging features will be disabled."
        );
        DXGI_CREATE_FACTORY_FLAGS::default()
    };
    unsafe { Ok(CreateDXGIFactory2(factory_flag)?) }
}

#[inline]
fn get_adapter(
    dxgi_factory: &IDXGIFactory6,
    debug_layer_available: bool,
    renderer_selection: RendererSelection,
) -> Result<(
    IDXGIAdapter1,
    ID3D11Device,
    ID3D11DeviceContext,
    D3D_FEATURE_LEVEL,
)> {
    if renderer_selection.requires_software_adapter() {
        let adapter = get_warp_adapter(dxgi_factory)?;
        let mut context: Option<ID3D11DeviceContext> = None;
        let mut feature_level = D3D_FEATURE_LEVEL::default();
        let device = get_device(
            None,
            D3D_DRIVER_TYPE_WARP,
            Some(&mut context),
            Some(&mut feature_level),
            debug_layer_available,
        )
        .context("Creating D3D11 WARP device")?;
        return Ok((
            adapter,
            device,
            context.expect("D3D11 WARP device creation returned no context"),
            feature_level,
        ));
    }

    for adapter_index in 0.. {
        let adapter: IDXGIAdapter1 = unsafe { dxgi_factory.EnumAdapters(adapter_index)?.cast()? };
        if let Ok(desc) = unsafe { adapter.GetDesc1() } {
            let gpu_name = String::from_utf16_lossy(&desc.Description)
                .trim_matches(char::from(0))
                .to_string();
            log::info!("Using GPU: {}", gpu_name);
        }
        // Check to see whether the adapter supports Direct3D 11 and create
        // the device if it does.
        let mut context: Option<ID3D11DeviceContext> = None;
        let mut feature_level = D3D_FEATURE_LEVEL::default();
        let d3d_adapter: IDXGIAdapter = adapter.cast().context("Getting DXGI adapter")?;
        if let Some(device) = get_device(
            Some(&d3d_adapter),
            D3D_DRIVER_TYPE_UNKNOWN,
            Some(&mut context),
            Some(&mut feature_level),
            debug_layer_available,
        )
        .log_err()
        {
            return Ok((adapter, device, context.unwrap(), feature_level));
        }
    }

    unreachable!()
}

fn get_warp_adapter(dxgi_factory: &IDXGIFactory6) -> Result<IDXGIAdapter1> {
    let factory: IDXGIFactory4 = dxgi_factory.cast().context("Getting IDXGIFactory4")?;
    let adapter: IDXGIAdapter1 =
        unsafe { factory.EnumWarpAdapter() }.context("Getting WARP adapter")?;
    let desc = unsafe { adapter.GetDesc1() }.context("Reading WARP adapter description")?;
    anyhow::ensure!(
        (desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32) != 0,
        "DXGI WARP adapter was not marked as software"
    );
    Ok(adapter)
}

#[inline]
fn get_device(
    adapter: Option<&IDXGIAdapter>,
    driver_type: D3D_DRIVER_TYPE,
    context: Option<*mut Option<ID3D11DeviceContext>>,
    feature_level: Option<*mut D3D_FEATURE_LEVEL>,
    debug_layer_available: bool,
) -> Result<ID3D11Device> {
    let mut device: Option<ID3D11Device> = None;
    let device_flags = if debug_layer_available {
        D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_DEBUG
    } else {
        D3D11_CREATE_DEVICE_BGRA_SUPPORT
    };
    unsafe {
        D3D11CreateDevice(
            adapter,
            driver_type,
            HMODULE::default(),
            device_flags,
            // 4x MSAA is required for Direct3D Feature Level 10.1 or better
            Some(&[
                D3D_FEATURE_LEVEL_11_1,
                D3D_FEATURE_LEVEL_11_0,
                D3D_FEATURE_LEVEL_10_1,
            ]),
            D3D11_SDK_VERSION,
            Some(&mut device),
            feature_level,
            context,
        )?;
    }
    let device = device.unwrap();
    let mut data = D3D11_FEATURE_DATA_D3D10_X_HARDWARE_OPTIONS::default();
    unsafe {
        device
            .CheckFeatureSupport(
                D3D11_FEATURE_D3D10_X_HARDWARE_OPTIONS,
                &mut data as *mut _ as _,
                std::mem::size_of::<D3D11_FEATURE_DATA_D3D10_X_HARDWARE_OPTIONS>() as u32,
            )
            .context("Checking GPU device feature support")?;
    }
    if data
        .ComputeShaders_Plus_RawAndStructuredBuffers_Via_Shader_4_x
        .as_bool()
    {
        Ok(device)
    } else {
        Err(anyhow::anyhow!(
            "Required feature StructuredBuffer is not supported by GPU/driver"
        ))
    }
}
