//! Native Windows Graphics Capture backend (Segment S2).
//!
//! Implements `capture_api::VideoCapture` for one selected display using
//! Windows Graphics Capture over Direct3D 11 (spec VID-001…009, §13.2).
//!
//! Threading model (ADR 0002/0005): the dedicated capture thread owns session
//! lifecycle and cleanup. WGC's free-threaded callback uses cloned pool and
//! D3D references under the backend's explicit multithread-safety wrappers;
//! it never waits on the controller, disk, or UI. Frames are copied into a
//! fixed staging texture ring before being handed out, keeping GPU memory
//! bounded regardless of consumer behavior (exit criterion: no growing
//! GPU-resource usage).
//!
//! No code is injected into any process: WGC captures composited monitor
//! content via the OS (VID-011).

mod device;
mod enumerate;
mod session;

pub use enumerate::{
    enumerate_adapters, enumerate_displays, enumerate_windows, GraphicsAdapterInfo,
};
pub use gpu_windows::GpuTextureSlot;
pub use session::WgcDisplayCapture;

/// Feature probe used at startup/settings time: reports whether this OS
/// supports Windows Graphics Capture at all (spec §16.1 capability
/// discovery groundwork).
#[cfg(windows)]
pub fn is_wgc_supported() -> bool {
    use windows::Graphics::Capture::GraphicsCaptureSession;
    GraphicsCaptureSession::IsSupported().unwrap_or(false)
}

#[cfg(not(windows))]
pub fn is_wgc_supported() -> bool {
    false
}
