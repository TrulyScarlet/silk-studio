//! Windows Audio Session API capture backends (Segment S4).
//!
//! Loopback and microphone capture are separate instances so a microphone
//! failure cannot stop desktop audio. Native objects are created, consumed,
//! and released on the worker thread that calls `start`, `next_event`, and
//! `stop`. Non-Windows builds retain a typed unavailable backend so the
//! workspace remains portable for deterministic tests.

#[cfg(windows)]
mod native;
#[cfg(not(windows))]
mod unsupported;

#[cfg(windows)]
pub use native::{enumerate_devices, WasapiAudioCapture, WasapiCaptureKind};
#[cfg(not(windows))]
pub use unsupported::{enumerate_devices, WasapiAudioCapture, WasapiCaptureKind};
