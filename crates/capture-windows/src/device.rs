//! D3D11 device creation for the capture backend.

use windows::core::Interface;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Multithread,
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_FLAG, D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
};

use capture_api::{Result as CaptureResult, VideoCaptureError};

pub(crate) struct CapturingDevice {
    pub device: ID3D11Device,
    pub context: ID3D11DeviceContext,
}

/// Create a BGRA-capable D3D11 device: hardware first, WARP fallback
/// (keeps CI/headless machines functional at reduced performance).
pub(crate) fn create_capture_device() -> CaptureResult<CapturingDevice> {
    if let Ok(found) = try_create(D3D_DRIVER_TYPE_HARDWARE) {
        return Ok(found);
    }
    try_create(D3D_DRIVER_TYPE_WARP)
}

fn try_create(
    driver_type: windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE,
) -> CaptureResult<CapturingDevice> {
    unsafe {
        let mut device: Option<ID3D11Device> = None;
        let mut context: Option<ID3D11DeviceContext> = None;
        let feature_levels = [windows::Win32::Graphics::Direct3D::D3D_FEATURE_LEVEL_11_0];

        D3D11CreateDevice(
            None,
            driver_type,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_FLAG(
                D3D11_CREATE_DEVICE_BGRA_SUPPORT.0 | D3D11_CREATE_DEVICE_VIDEO_SUPPORT.0,
            ),
            Some(&feature_levels),
            7, // D3D11_SDK_VERSION
            Some(&mut device),
            None,
            Some(&mut context),
        )
        .map_err(|err| VideoCaptureError::Backend {
            details: format!("D3D11CreateDevice({driver_type:?}) failed: {err}"),
        })?;

        let device = device.ok_or_else(|| VideoCaptureError::Backend {
            details: "D3D11CreateDevice returned null device".into(),
        })?;
        let context = context.ok_or_else(|| VideoCaptureError::Backend {
            details: "D3D11CreateDevice returned null context".into(),
        })?;
        let multithread: ID3D11Multithread =
            context.cast().map_err(|err| VideoCaptureError::Backend {
                details: format!("D3D11 context has no multithread interface: {err}"),
            })?;
        let _ = multithread.SetMultithreadProtected(true);
        Ok(CapturingDevice { device, context })
    }
}
