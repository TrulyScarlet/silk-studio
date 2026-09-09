//! Native Windows HUD implementation for capture confirmations.
//!
//! Provides a zero-window-mutation, Direct2D/DirectComposition HUD surface for
//! nonblocking queued/saved/failed gameplay feedback.

pub mod format;
pub mod geometry;
pub mod types;

#[cfg(windows)]
mod windows;

#[cfg(not(windows))]
mod unsupported;

pub use format::*;
pub use geometry::*;
pub use types::*;

#[cfg(windows)]
pub use windows::NativeHud;

#[cfg(not(windows))]
pub use unsupported::NativeHud;
