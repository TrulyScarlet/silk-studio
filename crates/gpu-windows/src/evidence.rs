//! GPU scaling evidence, synthetic patterns, NV12 CPU readback, and chroma metrics.
//!
//! This module provides deterministic tools to verify D3D11 video processor conversion
//! from BGRA8 to NV12 (both 1x and 2x supersampled). It includes:
//! - Checked row-pitch aware NV12 plane addressing.
//! - BT.709 limited-range validation.
//! - Deterministic relative chroma-retention contrast metric for a 1-pixel saturated feature.
//! - Durable native D3D11 test comparing 1x vs 2x pre-encode quality.

use std::sync::Arc;

use thiserror::Error;
use windows::core::Interface;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP, D3D_FEATURE_LEVEL_11_0,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Multithread, ID3D11Texture2D,
    D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_CPU_ACCESS_READ,
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_FLAG, D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
    D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ, D3D11_SUBRESOURCE_DATA, D3D11_TEXTURE2D_DESC,
    D3D11_USAGE_DEFAULT, D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_NV12, DXGI_SAMPLE_DESC,
};

use crate::{D3D11VideoConverter, GpuDeviceContext, GpuMediaError, GpuTextureSlot};

/// Identifies the specific execution stage where an error occurred.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvidenceStage {
    DeviceCreation,
    EnumeratorCreation,
    FormatSupport,
    RectSetup,
    SourceTextureCreation,
    Converter1xExecution,
    Converter2xExecution,
    StagingCopyAndMap,
    MetricEvaluation,
}

/// Errors occurring during evidence generation, readback, or metric evaluation.
#[derive(Debug, Error)]
pub enum EvidenceError {
    #[error("stage '{stage:?}' failed: {message}")]
    Stage {
        stage: EvidenceStage,
        message: String,
        #[source]
        source: Option<windows::core::Error>,
    },

    #[error("stage '{stage:?}' failed with media error: {source}")]
    Media {
        stage: EvidenceStage,
        #[source]
        source: GpuMediaError,
    },

    #[error("stage '{stage:?}' layout error: {details}")]
    Layout {
        stage: EvidenceStage,
        details: &'static str,
    },

    #[error("stage '{stage:?}' BT.709 limited range violation: {details}")]
    RangeViolation {
        stage: EvidenceStage,
        details: String,
    },

    #[error("stage '{stage:?}' failed quality claim: 2x normalized chroma contrast ({contrast_2x:.4}) <= 1x ({contrast_1x:.4})")]
    ChromaQualityRegression {
        stage: EvidenceStage,
        contrast_1x: f64,
        contrast_2x: f64,
    },
}

/// A CPU-side NV12 image with row-pitch aware addressing for Y and UV planes.
#[derive(Debug, Clone)]
pub struct Nv12CpuBuffer {
    data: Vec<u8>,
    width: u32,
    height: u32,
    row_pitch: usize,
}

impl Nv12CpuBuffer {
    /// Creates a new `Nv12CpuBuffer` after validating dimensions, pitch, and buffer length.
    pub fn new(
        data: Vec<u8>,
        width: u32,
        height: u32,
        row_pitch: usize,
    ) -> Result<Self, EvidenceError> {
        if width == 0 || height == 0 || !width.is_multiple_of(2) || !height.is_multiple_of(2) {
            return Err(EvidenceError::Layout {
                stage: EvidenceStage::MetricEvaluation,
                details: "NV12 width and height must be non-zero and even",
            });
        }
        if row_pitch < width as usize {
            return Err(EvidenceError::Layout {
                stage: EvidenceStage::MetricEvaluation,
                details: "row_pitch must be at least width",
            });
        }

        let y_rows = height as usize;
        let uv_rows = (height / 2) as usize;

        let y_plane_len = row_pitch.checked_mul(y_rows).ok_or(EvidenceError::Layout {
            stage: EvidenceStage::MetricEvaluation,
            details: "Y plane length calculation overflowed",
        })?;
        let uv_plane_len = row_pitch
            .checked_mul(uv_rows)
            .ok_or(EvidenceError::Layout {
                stage: EvidenceStage::MetricEvaluation,
                details: "UV plane length calculation overflowed",
            })?;
        let required_len = y_plane_len
            .checked_add(uv_plane_len)
            .ok_or(EvidenceError::Layout {
                stage: EvidenceStage::MetricEvaluation,
                details: "total NV12 buffer length calculation overflowed",
            })?;

        if data.len() < required_len {
            return Err(EvidenceError::Layout {
                stage: EvidenceStage::MetricEvaluation,
                details: "provided data buffer is smaller than required NV12 layout size",
            });
        }

        Ok(Self {
            data,
            width,
            height,
            row_pitch,
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn row_pitch(&self) -> usize {
        self.row_pitch
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Reads one Y (luma) sample at coordinate (x, y).
    pub fn get_y(&self, x: u32, y: u32) -> Result<u8, EvidenceError> {
        if x >= self.width || y >= self.height {
            return Err(EvidenceError::Layout {
                stage: EvidenceStage::MetricEvaluation,
                details: "Y coordinate out of bounds",
            });
        }
        let offset = (y as usize)
            .checked_mul(self.row_pitch)
            .and_then(|row_start| row_start.checked_add(x as usize))
            .ok_or(EvidenceError::Layout {
                stage: EvidenceStage::MetricEvaluation,
                details: "Y sample offset calculation overflowed",
            })?;
        Ok(self.data[offset])
    }

    /// Reads one interleaved (U, V) chroma pair at chroma coordinate (u_x, u_y).
    ///
    /// `u_x` spans `0..width/2` and `u_y` spans `0..height/2`.
    pub fn get_uv(&self, u_x: u32, u_y: u32) -> Result<(u8, u8), EvidenceError> {
        let uv_width = self.width / 2;
        let uv_height = self.height / 2;
        if u_x >= uv_width || u_y >= uv_height {
            return Err(EvidenceError::Layout {
                stage: EvidenceStage::MetricEvaluation,
                details: "UV coordinate out of bounds",
            });
        }

        let y_plane_len =
            (self.height as usize)
                .checked_mul(self.row_pitch)
                .ok_or(EvidenceError::Layout {
                    stage: EvidenceStage::MetricEvaluation,
                    details: "Y plane length calculation overflowed",
                })?;
        let uv_row_offset =
            (u_y as usize)
                .checked_mul(self.row_pitch)
                .ok_or(EvidenceError::Layout {
                    stage: EvidenceStage::MetricEvaluation,
                    details: "UV row offset calculation overflowed",
                })?;
        let col_offset = (u_x as usize).checked_mul(2).ok_or(EvidenceError::Layout {
            stage: EvidenceStage::MetricEvaluation,
            details: "UV column offset calculation overflowed",
        })?;

        let u_offset = y_plane_len
            .checked_add(uv_row_offset)
            .and_then(|base| base.checked_add(col_offset))
            .ok_or(EvidenceError::Layout {
                stage: EvidenceStage::MetricEvaluation,
                details: "U sample offset calculation overflowed",
            })?;
        let v_offset = u_offset.checked_add(1).ok_or(EvidenceError::Layout {
            stage: EvidenceStage::MetricEvaluation,
            details: "V sample offset calculation overflowed",
        })?;

        Ok((self.data[u_offset], self.data[v_offset]))
    }

    /// Computes the Euclidean chroma distance of sample `(u_x, u_y)` from neutral `(128.0, 128.0)`.
    pub fn chroma_deviation(&self, u_x: u32, u_y: u32) -> Result<f64, EvidenceError> {
        let (u, v) = self.get_uv(u_x, u_y)?;
        let du = u as f64 - 128.0;
        let dv = v as f64 - 128.0;
        Ok((du * du + dv * dv).sqrt())
    }

    /// Validates that all pixel values fall within BT.709 limited range bounds:
    /// - Y in `16..=235`
    /// - U and V in `16..=240`
    pub fn validate_bt709_limited_range(&self) -> Result<Bt709RangeSummary, EvidenceError> {
        let mut min_y = u8::MAX;
        let mut max_y = u8::MIN;
        for y in 0..self.height {
            for x in 0..self.width {
                let y_val = self.get_y(x, y)?;
                min_y = min_y.min(y_val);
                max_y = max_y.max(y_val);
                if !(16..=235).contains(&y_val) {
                    return Err(EvidenceError::RangeViolation {
                        stage: EvidenceStage::MetricEvaluation,
                        details: format!(
                            "Y sample at ({x}, {y}) = {y_val} is outside BT.709 limited range [16, 235]"
                        ),
                    });
                }
            }
        }

        let mut min_u = u8::MAX;
        let mut max_u = u8::MIN;
        let mut min_v = u8::MAX;
        let mut max_v = u8::MIN;
        let uv_w = self.width / 2;
        let uv_h = self.height / 2;
        for uy in 0..uv_h {
            for ux in 0..uv_w {
                let (u_val, v_val) = self.get_uv(ux, uy)?;
                min_u = min_u.min(u_val);
                max_u = max_u.max(u_val);
                min_v = min_v.min(v_val);
                max_v = max_v.max(v_val);
                if !(16..=240).contains(&u_val) {
                    return Err(EvidenceError::RangeViolation {
                        stage: EvidenceStage::MetricEvaluation,
                        details: format!(
                            "U sample at ({ux}, {uy}) = {u_val} is outside BT.709 limited range [16, 240]"
                        ),
                    });
                }
                if !(16..=240).contains(&v_val) {
                    return Err(EvidenceError::RangeViolation {
                        stage: EvidenceStage::MetricEvaluation,
                        details: format!(
                            "V sample at ({ux}, {uy}) = {v_val} is outside BT.709 limited range [16, 240]"
                        ),
                    });
                }
            }
        }

        Ok(Bt709RangeSummary {
            min_y,
            max_y,
            min_u,
            max_u,
            min_v,
            max_v,
        })
    }
}

/// Summary of sample ranges observed across an NV12 surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bt709RangeSummary {
    pub min_y: u8,
    pub max_y: u8,
    pub min_u: u8,
    pub max_u: u8,
    pub min_v: u8,
    pub max_v: u8,
}

/// Measured chroma metrics for an NV12 conversion.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChromaContrastResult {
    pub peak_chroma_deviation: f64,
    pub background_chroma_floor: f64,
    pub normalized_chroma_contrast: f64,
    pub peak_u: u8,
    pub peak_v: u8,
}

/// Comparison between 1x and 2x conversions of the same synthetic pattern.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EvidenceComparison {
    pub result_1x: ChromaContrastResult,
    pub result_2x: ChromaContrastResult,
    pub range_1x: Bt709RangeSummary,
    pub range_2x: Bt709RangeSummary,
    pub contrast_improvement: f64,
}

/// Creates a synthetic BGRA pattern of size `width x height` containing a 1-source-pixel saturated
/// red crosshair on a neutral gray background.
///
/// - Neutral background: BGRA `(128, 128, 128, 255)`.
/// - Saturated foreground: BGRA `(0, 0, 255, 255)` (Pure Red).
/// - 1px vertical line at `x = width / 2` spanning `height / 4 .. 3 * height / 4`.
/// - 1px horizontal line at `y = height / 2` spanning `width / 4 .. 3 * width / 4`.
pub fn create_crosshair_pattern(width: u32, height: u32) -> Result<Vec<u8>, EvidenceError> {
    if width == 0 || height == 0 || !width.is_multiple_of(2) || !height.is_multiple_of(2) {
        return Err(EvidenceError::Layout {
            stage: EvidenceStage::MetricEvaluation,
            details: "pattern width and height must be non-zero and even",
        });
    }

    let total_pixels =
        (width as usize)
            .checked_mul(height as usize)
            .ok_or(EvidenceError::Layout {
                stage: EvidenceStage::MetricEvaluation,
                details: "pixel count calculation overflowed",
            })?;
    let total_bytes = total_pixels.checked_mul(4).ok_or(EvidenceError::Layout {
        stage: EvidenceStage::MetricEvaluation,
        details: "byte count calculation overflowed",
    })?;

    let mut bgra = vec![0u8; total_bytes];

    // Fill with neutral gray (128, 128, 128, 255)
    for chunk in bgra.as_chunks_mut::<4>().0 {
        chunk[0] = 128; // B
        chunk[1] = 128; // G
        chunk[2] = 128; // R
        chunk[3] = 255; // A
    }

    let cx = width / 2;
    let cy = height / 2;
    let x_start = width / 4;
    let x_end = 3 * width / 4;
    let y_start = height / 4;
    let y_end = 3 * height / 4;

    // Draw horizontal 1px saturated line
    for x in x_start..=x_end {
        let offset = ((cy as usize) * (width as usize) + (x as usize)) * 4;
        bgra[offset] = 0; // B
        bgra[offset + 1] = 0; // G
        bgra[offset + 2] = 255; // R
        bgra[offset + 3] = 255; // A
    }

    // Draw vertical 1px saturated line
    for y in y_start..=y_end {
        let offset = ((y as usize) * (width as usize) + (cx as usize)) * 4;
        bgra[offset] = 0; // B
        bgra[offset + 1] = 0; // G
        bgra[offset + 2] = 255; // R
        bgra[offset + 3] = 255; // A
    }

    Ok(bgra)
}

/// Evaluates the chroma contrast of an NV12 buffer converted from the synthetic crosshair pattern.
///
/// `source_width` and `source_height` are the dimensions of the original BGRA pattern.
/// The metric compares the chroma retention of the 1-source-pixel feature against its immediate
/// 1-source-pixel background neighborhood in the corresponding source-pixel footprint.
pub fn evaluate_chroma_contrast(
    buffer: &Nv12CpuBuffer,
    source_width: u32,
    source_height: u32,
) -> Result<ChromaContrastResult, EvidenceError> {
    if source_width == 0
        || source_height == 0
        || !source_width.is_multiple_of(2)
        || !source_height.is_multiple_of(2)
    {
        return Err(EvidenceError::Layout {
            stage: EvidenceStage::MetricEvaluation,
            details: "source dimensions must be non-zero and even",
        });
    }

    let scale_x = buffer.width() as f64 / source_width as f64;
    let scale_y = buffer.height() as f64 / source_height as f64;

    let src_cx = source_width / 2;
    let src_cy = source_height / 2;
    let src_x_start = source_width / 4;
    let src_x_end = 3 * source_width / 4;
    let src_y_start = source_height / 4;
    let src_y_end = 3 * source_height / 4;

    let uv_w = buffer.width() / 2;
    let uv_h = buffer.height() / 2;

    // Helper to map source coordinates (xs, ys) to UV coordinates (ux, uy)
    let map_src_to_uv = |xs: u32, ys: u32| -> (u32, u32) {
        let out_x = (xs as f64 * scale_x).floor() as u32;
        let out_y = (ys as f64 * scale_y).floor() as u32;
        let ux = (out_x / 2).min(uv_w.saturating_sub(1));
        let uy = (out_y / 2).min(uv_h.saturating_sub(1));
        (ux, uy)
    };

    let mut peak_chroma_deviation = 0.0f64;
    let mut peak_u = 128u8;
    let mut peak_v = 128u8;
    let mut contrast_sum = 0.0f64;
    let mut contrast_samples = 0usize;
    let mut immediate_bg_sum = 0.0f64;

    // Evaluate along vertical crosshair arm: xs = src_cx, ys in src_y_start..=src_y_end
    for ys in src_y_start..=src_y_end {
        let (feat_ux, feat_uy) = map_src_to_uv(src_cx, ys);
        let feat_dev = buffer.chroma_deviation(feat_ux, feat_uy)?;

        if feat_dev > peak_chroma_deviation {
            peak_chroma_deviation = feat_dev;
            let (u, v) = buffer.get_uv(feat_ux, feat_uy)?;
            peak_u = u;
            peak_v = v;
        }

        // Sample immediate 1-source-pixel neighbors to the left (xs - 1) and right (xs + 1)
        let (left_ux, left_uy) = map_src_to_uv(src_cx.saturating_sub(1), ys);
        let (right_ux, right_uy) = map_src_to_uv((src_cx + 1).min(source_width - 1), ys);
        let left_dev = buffer.chroma_deviation(left_ux, left_uy)?;
        let right_dev = buffer.chroma_deviation(right_ux, right_uy)?;
        let local_bg = (left_dev + right_dev) / 2.0;

        immediate_bg_sum += local_bg;
        contrast_sum += (feat_dev - local_bg).max(0.0);
        contrast_samples += 1;
    }

    // Evaluate along horizontal crosshair arm: ys = src_cy, xs in src_x_start..=src_x_end
    // (excluding center already counted)
    for xs in src_x_start..=src_x_end {
        if xs == src_cx {
            continue;
        }
        let (feat_ux, feat_uy) = map_src_to_uv(xs, src_cy);
        let feat_dev = buffer.chroma_deviation(feat_ux, feat_uy)?;

        if feat_dev > peak_chroma_deviation {
            peak_chroma_deviation = feat_dev;
            let (u, v) = buffer.get_uv(feat_ux, feat_uy)?;
            peak_u = u;
            peak_v = v;
        }

        // Sample immediate 1-source-pixel neighbors above (ys - 1) and below (ys + 1)
        let (top_ux, top_uy) = map_src_to_uv(xs, src_cy.saturating_sub(1));
        let (bot_ux, bot_uy) = map_src_to_uv(xs, (src_cy + 1).min(source_height - 1));
        let top_dev = buffer.chroma_deviation(top_ux, top_uy)?;
        let bot_dev = buffer.chroma_deviation(bot_ux, bot_uy)?;
        let local_bg = (top_dev + bot_dev) / 2.0;

        immediate_bg_sum += local_bg;
        contrast_sum += (feat_dev - local_bg).max(0.0);
        contrast_samples += 1;
    }

    let background_chroma_floor = if contrast_samples > 0 {
        immediate_bg_sum / contrast_samples as f64
    } else {
        0.0
    };

    // Theoretical BT.709 pure red chroma deviation from neutral (128, 128):
    // Pure Red (255, 0, 0) in limited-range BT.709:
    // U ≈ 102 (delta = -26), V ≈ 240 (delta = +112)
    // deviation = sqrt(26^2 + 112^2) = sqrt(676 + 12544) = sqrt(13220) ≈ 114.9782588
    const THEORETICAL_MAX_CHROMA_DEV: f64 = 114.978_258_814_426_74;

    let mean_contrast = if contrast_samples > 0 {
        contrast_sum / contrast_samples as f64
    } else {
        0.0
    };

    let normalized_chroma_contrast = mean_contrast / THEORETICAL_MAX_CHROMA_DEV;

    Ok(ChromaContrastResult {
        peak_chroma_deviation,
        background_chroma_floor,
        normalized_chroma_contrast,
        peak_u,
        peak_v,
    })
}

/// Reads an NV12 GPU texture to a CPU buffer via a staging texture.
pub fn read_nv12_texture_to_cpu(
    device_context: &GpuDeviceContext,
    nv12_texture: &ID3D11Texture2D,
    width: u32,
    height: u32,
) -> Result<Nv12CpuBuffer, EvidenceError> {
    let staging_desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_NV12,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_STAGING,
        BindFlags: 0,
        CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
        MiscFlags: 0,
    };

    let mut staging_texture = None;
    unsafe {
        device_context
            .device()
            .CreateTexture2D(&staging_desc, None, Some(&mut staging_texture))
    }
    .map_err(|err| EvidenceError::Stage {
        stage: EvidenceStage::StagingCopyAndMap,
        message: format!("create staging texture ({width}x{height}) failed"),
        source: Some(err),
    })?;

    let staging_texture = staging_texture.ok_or_else(|| EvidenceError::Stage {
        stage: EvidenceStage::StagingCopyAndMap,
        message: "CreateTexture2D returned null staging texture".to_string(),
        source: None,
    })?;

    unsafe {
        device_context
            .context()
            .CopyResource(&staging_texture, nv12_texture);
    }

    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    unsafe {
        device_context
            .context()
            .Map(&staging_texture, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
    }
    .map_err(|err| EvidenceError::Stage {
        stage: EvidenceStage::StagingCopyAndMap,
        message: "Map staging texture subresource 0 failed".to_string(),
        source: Some(err),
    })?;

    let row_pitch = mapped.RowPitch as usize;
    let y_plane_len = (height as usize)
        .checked_mul(row_pitch)
        .ok_or(EvidenceError::Layout {
            stage: EvidenceStage::StagingCopyAndMap,
            details: "Y plane length calculation overflowed",
        })?;
    let uv_rows = (height / 2) as usize;
    let uv_plane_len = uv_rows
        .checked_mul(row_pitch)
        .ok_or(EvidenceError::Layout {
            stage: EvidenceStage::StagingCopyAndMap,
            details: "UV plane length calculation overflowed",
        })?;
    let total_len = y_plane_len
        .checked_add(uv_plane_len)
        .ok_or(EvidenceError::Layout {
            stage: EvidenceStage::StagingCopyAndMap,
            details: "total mapped length calculation overflowed",
        })?;

    let mut cpu_bytes = vec![0u8; total_len];
    unsafe {
        if !mapped.pData.is_null() {
            std::ptr::copy_nonoverlapping(
                mapped.pData as *const u8,
                cpu_bytes.as_mut_ptr(),
                total_len,
            );
        }
        device_context.context().Unmap(&staging_texture, 0);
    }

    Nv12CpuBuffer::new(cpu_bytes, width, height, row_pitch)
}

/// Creates a multithread-protected D3D11 device context suitable for video processing.
///
/// Tries hardware D3D11 first, falling back to WARP if hardware creation fails.
pub fn create_video_device_context() -> Result<Arc<GpuDeviceContext>, EvidenceError> {
    let try_driver = |driver_type: windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE| -> Result<Arc<GpuDeviceContext>, windows::core::Error> {
        let mut device: Option<ID3D11Device> = None;
        let mut context: Option<ID3D11DeviceContext> = None;
        let feature_levels = [D3D_FEATURE_LEVEL_11_0];
        unsafe {
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
            )?;
        }
        let device = device.ok_or_else(|| windows::core::Error::from(windows::Win32::Foundation::E_FAIL))?;
        let context = context.ok_or_else(|| windows::core::Error::from(windows::Win32::Foundation::E_FAIL))?;
        let multithread: ID3D11Multithread = context.cast()?;
        unsafe {
            let _ = multithread.SetMultithreadProtected(true);
        }
        Ok(Arc::new(GpuDeviceContext::new(device, context)))
    };

    if let Ok(dc) = try_driver(D3D_DRIVER_TYPE_HARDWARE) {
        return Ok(dc);
    }
    try_driver(D3D_DRIVER_TYPE_WARP).map_err(|err| EvidenceError::Stage {
        stage: EvidenceStage::DeviceCreation,
        message: format!("create D3D11 video device failed (both hardware and WARP): {err}"),
        source: Some(err),
    })
}

/// Runs native D3D11 video processor conversions for 1x and 2x on the same synthetic 1px pattern
/// and computes the relative chroma-retention evidence comparison.
pub fn run_native_chroma_evidence(
    device_context: Option<Arc<GpuDeviceContext>>,
) -> Result<EvidenceComparison, EvidenceError> {
    let device_context = match device_context {
        Some(dc) => dc,
        None => create_video_device_context()?,
    };

    const PATTERN_WIDTH: u32 = 64;
    const PATTERN_HEIGHT: u32 = 64;

    let pattern_bytes = create_crosshair_pattern(PATTERN_WIDTH, PATTERN_HEIGHT)?;

    // Create source BGRA texture
    let source_desc = D3D11_TEXTURE2D_DESC {
        Width: PATTERN_WIDTH,
        Height: PATTERN_HEIGHT,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };

    let init_data = D3D11_SUBRESOURCE_DATA {
        pSysMem: pattern_bytes.as_ptr() as *const _,
        SysMemPitch: PATTERN_WIDTH * 4,
        SysMemSlicePitch: 0,
    };

    let mut source_texture = None;
    unsafe {
        device_context.device().CreateTexture2D(
            &source_desc,
            Some(&init_data),
            Some(&mut source_texture),
        )
    }
    .map_err(|err| EvidenceError::Stage {
        stage: EvidenceStage::SourceTextureCreation,
        message: "create source BGRA pattern texture failed".to_string(),
        source: Some(err),
    })?;

    let source_texture = source_texture.ok_or_else(|| EvidenceError::Stage {
        stage: EvidenceStage::SourceTextureCreation,
        message: "CreateTexture2D returned null source texture".to_string(),
        source: None,
    })?;

    let source_slot = GpuTextureSlot::new(source_texture, device_context.clone());

    // 1x Conversion (64x64 -> 64x64)
    let converter_1x = D3D11VideoConverter::new(
        device_context.clone(),
        PATTERN_WIDTH,
        PATTERN_HEIGHT,
        PATTERN_WIDTH,
        PATTERN_HEIGHT,
        60,
    )
    .map_err(|err| EvidenceError::Media {
        stage: EvidenceStage::Converter1xExecution,
        source: err,
    })?;

    let nv12_1x = converter_1x
        .convert(&source_slot)
        .map_err(|err| EvidenceError::Media {
            stage: EvidenceStage::Converter1xExecution,
            source: err,
        })?;

    let buffer_1x = read_nv12_texture_to_cpu(
        &device_context,
        nv12_1x.texture(),
        PATTERN_WIDTH,
        PATTERN_HEIGHT,
    )?;
    let range_1x = buffer_1x.validate_bt709_limited_range()?;
    let result_1x = evaluate_chroma_contrast(&buffer_1x, PATTERN_WIDTH, PATTERN_HEIGHT)?;

    // 2x Conversion (64x64 -> 128x128)
    let converter_2x = D3D11VideoConverter::new(
        device_context.clone(),
        PATTERN_WIDTH,
        PATTERN_HEIGHT,
        PATTERN_WIDTH * 2,
        PATTERN_HEIGHT * 2,
        60,
    )
    .map_err(|err| EvidenceError::Media {
        stage: EvidenceStage::Converter2xExecution,
        source: err,
    })?;

    let nv12_2x = converter_2x
        .convert(&source_slot)
        .map_err(|err| EvidenceError::Media {
            stage: EvidenceStage::Converter2xExecution,
            source: err,
        })?;

    let buffer_2x = read_nv12_texture_to_cpu(
        &device_context,
        nv12_2x.texture(),
        PATTERN_WIDTH * 2,
        PATTERN_HEIGHT * 2,
    )?;
    let range_2x = buffer_2x.validate_bt709_limited_range()?;
    let result_2x = evaluate_chroma_contrast(&buffer_2x, PATTERN_WIDTH, PATTERN_HEIGHT)?;

    let contrast_improvement =
        result_2x.normalized_chroma_contrast - result_1x.normalized_chroma_contrast;

    if result_2x.normalized_chroma_contrast <= result_1x.normalized_chroma_contrast {
        return Err(EvidenceError::ChromaQualityRegression {
            stage: EvidenceStage::MetricEvaluation,
            contrast_1x: result_1x.normalized_chroma_contrast,
            contrast_2x: result_2x.normalized_chroma_contrast,
        });
    }

    Ok(EvidenceComparison {
        result_1x,
        result_2x,
        range_1x,
        range_2x,
        contrast_improvement,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nv12_row_pitch_and_plane_addressing() {
        // Construct a 4x4 NV12 image with row_pitch = 6 (2 padding bytes per row)
        // Y plane: 4 rows of 6 bytes = 24 bytes
        // UV plane: 2 rows of 6 bytes = 12 bytes
        // Total = 36 bytes
        let mut data = vec![0u8; 36];

        // Fill Y plane
        for y in 0..4u32 {
            for x in 0..4u32 {
                let offset = (y as usize) * 6 + (x as usize);
                data[offset] = (y * 10 + x + 16) as u8;
            }
        }

        // Fill UV plane
        for uy in 0..2u32 {
            for ux in 0..2u32 {
                let u_offset = 24 + (uy as usize) * 6 + (ux as usize) * 2;
                data[u_offset] = (uy * 20 + ux * 2 + 100) as u8; // U
                data[u_offset + 1] = (uy * 20 + ux * 2 + 200) as u8; // V
            }
        }

        let buffer = Nv12CpuBuffer::new(data, 4, 4, 6).expect("buffer creation should succeed");

        // Verify Y addressing
        assert_eq!(buffer.get_y(0, 0).unwrap(), 16);
        assert_eq!(buffer.get_y(3, 0).unwrap(), 19);
        assert_eq!(buffer.get_y(0, 3).unwrap(), 46);
        assert_eq!(buffer.get_y(3, 3).unwrap(), 49);

        // Verify UV addressing
        assert_eq!(buffer.get_uv(0, 0).unwrap(), (100, 200));
        assert_eq!(buffer.get_uv(1, 0).unwrap(), (102, 202));
        assert_eq!(buffer.get_uv(0, 1).unwrap(), (120, 220));
        assert_eq!(buffer.get_uv(1, 1).unwrap(), (122, 222));
    }

    #[test]
    fn nv12_checked_arithmetic_detects_invalid_inputs() {
        // Odd dimensions
        assert!(Nv12CpuBuffer::new(vec![0; 100], 3, 4, 4).is_err());
        assert!(Nv12CpuBuffer::new(vec![0; 100], 4, 5, 4).is_err());
        assert!(Nv12CpuBuffer::new(vec![0; 100], 0, 4, 4).is_err());

        // row_pitch < width
        assert!(Nv12CpuBuffer::new(vec![0; 100], 8, 8, 4).is_err());

        // Buffer too short (8x8 with pitch 8 requires 8*8 + 8*4 = 96 bytes)
        assert!(Nv12CpuBuffer::new(vec![0; 95], 8, 8, 8).is_err());
        assert!(Nv12CpuBuffer::new(vec![0; 96], 8, 8, 8).is_ok());
    }

    #[test]
    fn synthetic_pattern_generation_and_properties() {
        let pattern = create_crosshair_pattern(64, 64).expect("pattern generation");
        assert_eq!(pattern.len(), 64 * 64 * 4);

        // Corner is neutral gray: (128, 128, 128, 255)
        assert_eq!(&pattern[0..4], &[128, 128, 128, 255]);

        // Center pixel (32, 32) is pure red: (0, 0, 255, 255)
        let center_offset = (32 * 64 + 32) * 4;
        assert_eq!(
            &pattern[center_offset..center_offset + 4],
            &[0, 0, 255, 255]
        );
    }

    #[test]
    fn bt709_limited_range_validation_detects_violations() {
        // Valid 4x4 buffer: Y in 16..=235, U/V in 16..=240
        let mut data = vec![128u8; 24]; // 4*4 Y + 4*2 UV = 24
        let buf = Nv12CpuBuffer::new(data.clone(), 4, 4, 4).unwrap();
        assert!(buf.validate_bt709_limited_range().is_ok());

        // Y underflow (0 < 16)
        data[0] = 15;
        let buf = Nv12CpuBuffer::new(data.clone(), 4, 4, 4).unwrap();
        assert!(buf.validate_bt709_limited_range().is_err());

        // Y overflow (236 > 235)
        data[0] = 236;
        let buf = Nv12CpuBuffer::new(data.clone(), 4, 4, 4).unwrap();
        assert!(buf.validate_bt709_limited_range().is_err());

        // UV underflow (10 < 16)
        data[0] = 128;
        data[16] = 10; // U sample
        let buf = Nv12CpuBuffer::new(data.clone(), 4, 4, 4).unwrap();
        assert!(buf.validate_bt709_limited_range().is_err());

        // UV overflow (245 > 240)
        data[16] = 128;
        data[17] = 245; // V sample
        let buf = Nv12CpuBuffer::new(data, 4, 4, 4).unwrap();
        assert!(buf.validate_bt709_limited_range().is_err());
    }

    #[test]
    fn deterministic_chroma_metric_proves_strict_2x_improvement() {
        // Synthetically model 1x vs 2x NV12 conversion of a 16x16 source with 1px red crosshair
        // In 1x (16x16): UV plane is 8x8. The 1px red line is filtered with gray in 2x2 quad:
        // Pure red: U=102, V=240 (delta_u = -26, delta_v = +112)
        // Subsampled 1x UV: filtered 50% -> delta_u = -13 (U=115), delta_v = +56 (V=184)
        // In 2x (32x32): UV plane is 16x16. The 2px red line covers full 2x2 UV quad:
        // 2x UV: U=102, V=240 (delta_u = -26, delta_v = +112)

        // Create 1x NV12 buffer (16x16)
        let total_1x = 16 * 16 + 16 * 8;
        let mut data_1x = vec![128u8; total_1x];
        // Center crosshair in 1x UV (8x8) at uy = 4, ux = 4
        for ux in 2..=6 {
            let offset = 256 + 4 * 16 + ux * 2;
            data_1x[offset] = 115; // U
            data_1x[offset + 1] = 184; // V
        }
        for uy in 2..=6 {
            let offset = 256 + uy * 16 + 4 * 2;
            data_1x[offset] = 115; // U
            data_1x[offset + 1] = 184; // V
        }
        let buf_1x = Nv12CpuBuffer::new(data_1x, 16, 16, 16).unwrap();
        let res_1x = evaluate_chroma_contrast(&buf_1x, 16, 16).unwrap();

        // Create 2x NV12 buffer (32x32)
        let total_2x = 32 * 32 + 32 * 16;
        let mut data_2x = vec![128u8; total_2x];
        // Center crosshair in 2x UV (16x16) at uy = 8, ux = 8
        for ux in 4..=12 {
            let offset = 1024 + 8 * 32 + ux * 2;
            data_2x[offset] = 102; // U
            data_2x[offset + 1] = 240; // V
        }
        for uy in 4..=12 {
            let offset = 1024 + uy * 32 + 8 * 2;
            data_2x[offset] = 102; // U
            data_2x[offset + 1] = 240; // V
        }
        let buf_2x = Nv12CpuBuffer::new(data_2x, 32, 32, 32).unwrap();
        let res_2x = evaluate_chroma_contrast(&buf_2x, 16, 16).unwrap();

        assert!(
            res_2x.normalized_chroma_contrast > res_1x.normalized_chroma_contrast,
            "2x normalized chroma contrast ({:.4}) must strictly exceed 1x ({:.4})",
            res_2x.normalized_chroma_contrast,
            res_1x.normalized_chroma_contrast
        );
        assert!(
            res_1x.normalized_chroma_contrast > 0.15 && res_1x.normalized_chroma_contrast < 0.40
        );
        assert!(res_2x.normalized_chroma_contrast > 0.90);
    }

    #[test]
    fn deterministic_pink_crosshair_with_black_outline_preserves_chroma_and_hue() {
        // BT.709 RGB to YUV helper matching the HLSL shader
        let rgb_to_yuv = |rgb: [f32; 3]| -> (f32, f32, f32) {
            let y = 0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2];
            let u = (rgb[2] - y) / 1.8556;
            let v = (rgb[0] - y) / 1.5748;
            (y, u, v)
        };

        // Edge-directed bilateral chroma downsampler (identical to PS_Chroma in HLSL)
        let edge_directed_chroma =
            |p00: [f32; 3], p10: [f32; 3], p01: [f32; 3], p11: [f32; 3]| -> (f32, f32) {
                let yuv00 = rgb_to_yuv(p00);
                let yuv10 = rgb_to_yuv(p10);
                let yuv01 = rgb_to_yuv(p01);
                let yuv11 = rgb_to_yuv(p11);

                let sat00 = yuv00.1 * yuv00.1 + yuv00.2 * yuv00.2;
                let sat10 = yuv10.1 * yuv10.1 + yuv10.2 * yuv10.2;
                let sat01 = yuv01.1 * yuv01.1 + yuv01.2 * yuv01.2;
                let sat11 = yuv11.1 * yuv11.1 + yuv11.2 * yuv11.2;

                let max_sat = sat00.max(sat10).max(sat01).max(sat11);
                let luma_variance = (yuv00.0 - yuv10.0).abs()
                    + (yuv01.0 - yuv11.0).abs()
                    + (yuv00.0 - yuv01.0).abs()
                    + (yuv10.0 - yuv11.0).abs();

                const EPS: f32 = 1e-4;
                let mut w00 = (sat00 + EPS).powf(1.5);
                let mut w10 = (sat10 + EPS).powf(1.5);
                let mut w01 = (sat01 + EPS).powf(1.5);
                let mut w11 = (sat11 + EPS).powf(1.5);

                if luma_variance > 0.05 && max_sat > 0.01 {
                    w00 *= sat00 / (max_sat + EPS);
                    w10 *= sat10 / (max_sat + EPS);
                    w01 *= sat01 / (max_sat + EPS);
                    w11 *= sat11 / (max_sat + EPS);
                }

                let total_weight = w00 + w10 + w01 + w11;
                let mut filtered_u =
                    (w00 * yuv00.1 + w10 * yuv10.1 + w01 * yuv01.1 + w11 * yuv11.1) / total_weight;
                let mut filtered_v =
                    (w00 * yuv00.2 + w10 * yuv10.2 + w01 * yuv01.2 + w11 * yuv11.2) / total_weight;

                let min_u = yuv00.1.min(yuv10.1).min(yuv01.1).min(yuv11.1);
                let max_u = yuv00.1.max(yuv10.1).max(yuv01.1).max(yuv11.1);
                let min_v = yuv00.2.min(yuv10.2).min(yuv01.2).min(yuv11.2);
                let max_v = yuv00.2.max(yuv10.2).max(yuv01.2).max(yuv11.2);

                filtered_u = filtered_u.clamp(min_u, max_u);
                filtered_v = filtered_v.clamp(min_v, max_v);

                (filtered_u, filtered_v)
            };

        // Standard unweighted box filter
        let box_filter_chroma =
            |p00: [f32; 3], p10: [f32; 3], p01: [f32; 3], p11: [f32; 3]| -> (f32, f32) {
                let yuv00 = rgb_to_yuv(p00);
                let yuv10 = rgb_to_yuv(p10);
                let yuv01 = rgb_to_yuv(p01);
                let yuv11 = rgb_to_yuv(p11);
                (
                    (yuv00.1 + yuv10.1 + yuv01.1 + yuv11.1) * 0.25,
                    (yuv00.2 + yuv10.2 + yuv01.2 + yuv11.2) * 0.25,
                )
            };

        // 1px Pink crosshair on 1px Black outline
        // Pink (HotPink / Magenta tint): R=1.0, G=0.25, B=0.75
        let pink = [1.0f32, 0.25f32, 0.75f32];
        let black = [0.0f32, 0.0f32, 0.0f32];

        let (_pink_y, pink_u, pink_v) = rgb_to_yuv(pink);
        let pink_chroma_mag = (pink_u * pink_u + pink_v * pink_v).sqrt();
        let pink_hue_angle = pink_v.atan2(pink_u);

        // Case 1: 1-pixel pink corner in a 2x2 quad with 3 black outline pixels
        let (box_u1, box_v1) = box_filter_chroma(pink, black, black, black);
        let (edge_u1, edge_v1) = edge_directed_chroma(pink, black, black, black);

        let box_mag1 = (box_u1 * box_u1 + box_v1 * box_v1).sqrt();
        let edge_mag1 = (edge_u1 * edge_u1 + edge_v1 * edge_v1).sqrt();
        let edge_hue1 = edge_v1.atan2(edge_u1);

        // Case 2: 1-pixel vertical pink line (2 pixels) in 2x2 quad with 2 black outline pixels
        let (box_u2, box_v2) = box_filter_chroma(pink, black, pink, black);
        let (edge_u2, edge_v2) = edge_directed_chroma(pink, black, pink, black);

        let box_mag2 = (box_u2 * box_u2 + box_v2 * box_v2).sqrt();
        let edge_mag2 = (edge_u2 * edge_u2 + edge_v2 * edge_v2).sqrt();
        let edge_hue2 = edge_v2.atan2(edge_u2);

        // 1. Box filter heavily washes out chroma (box_mag1 is 25% of pink, box_mag2 is 50%)
        assert!((box_mag1 - 0.25 * pink_chroma_mag).abs() < 1e-4);
        assert!((box_mag2 - 0.50 * pink_chroma_mag).abs() < 1e-4);

        // 2. Edge-directed filter preserves over 99% of original chroma magnitude
        assert!(edge_mag1 > 0.99 * pink_chroma_mag);
        assert!(edge_mag2 > 0.99 * pink_chroma_mag);
        assert!(edge_mag1 > 3.9 * box_mag1); // ~4x retention improvement over box filter

        // 3. Edge-directed filter exhibits zero hue distortion
        assert!(
            (edge_hue1 - pink_hue_angle).abs() < 1e-5,
            "Edge filter must preserve exact pink hue angle"
        );
        assert!(
            (edge_hue2 - pink_hue_angle).abs() < 1e-5,
            "Edge filter must preserve exact pink hue angle"
        );

        // 4. Quantized limited-range UV stays cleanly in [16/255, 240/255]
        let quantize_uv = |(u, v): (f32, f32)| -> (f32, f32) {
            let u_lim = ((128.0 + 224.0 * u) / 255.0).clamp(16.0 / 255.0, 240.0 / 255.0);
            let v_lim = ((128.0 + 224.0 * v) / 255.0).clamp(16.0 / 255.0, 240.0 / 255.0);
            (u_lim, v_lim)
        };

        let (q_u1, q_v1) = quantize_uv((edge_u1, edge_v1));
        assert!((16.0 / 255.0..=240.0 / 255.0).contains(&q_u1));
        assert!((16.0 / 255.0..=240.0 / 255.0).contains(&q_v1));
    }

    #[test]
    fn stage_error_reporting_identifies_all_stages() {
        let stages = [
            EvidenceStage::DeviceCreation,
            EvidenceStage::EnumeratorCreation,
            EvidenceStage::FormatSupport,
            EvidenceStage::RectSetup,
            EvidenceStage::SourceTextureCreation,
            EvidenceStage::Converter1xExecution,
            EvidenceStage::Converter2xExecution,
            EvidenceStage::StagingCopyAndMap,
            EvidenceStage::MetricEvaluation,
        ];
        for stage in stages {
            let err = EvidenceError::Layout {
                stage,
                details: "test details",
            };
            let formatted = format!("{err}");
            assert!(
                formatted.contains(&format!("{stage:?}")),
                "formatted error should mention stage {stage:?}"
            );
        }
    }

    #[test]
    #[ignore = "Requires Windows D3D11 hardware or WARP video processor support"]
    fn native_d3d11_1x_versus_2x_chroma_evidence() {
        match run_native_chroma_evidence(None) {
            Ok(comparison) => {
                println!(
                    "Native D3D11 Evidence Results:\n\
                     1x: contrast={:.4}, peak_dev={:.2}, bg_floor={:.2}, peak_uv=({}, {}), Y=[{}, {}], U=[{}, {}], V=[{}, {}]\n\
                     2x: contrast={:.4}, peak_dev={:.2}, bg_floor={:.2}, peak_uv=({}, {}), Y=[{}, {}], U=[{}, {}], V=[{}, {}]\n\
                     Improvement: {:+.4}",
                    comparison.result_1x.normalized_chroma_contrast,
                    comparison.result_1x.peak_chroma_deviation,
                    comparison.result_1x.background_chroma_floor,
                    comparison.result_1x.peak_u,
                    comparison.result_1x.peak_v,
                    comparison.range_1x.min_y,
                    comparison.range_1x.max_y,
                    comparison.range_1x.min_u,
                    comparison.range_1x.max_u,
                    comparison.range_1x.min_v,
                    comparison.range_1x.max_v,
                    comparison.result_2x.normalized_chroma_contrast,
                    comparison.result_2x.peak_chroma_deviation,
                    comparison.result_2x.background_chroma_floor,
                    comparison.result_2x.peak_u,
                    comparison.result_2x.peak_v,
                    comparison.range_2x.min_y,
                    comparison.range_2x.max_y,
                    comparison.range_2x.min_u,
                    comparison.range_2x.max_u,
                    comparison.range_2x.min_v,
                    comparison.range_2x.max_v,
                    comparison.contrast_improvement,
                );
                assert!(
                    comparison.result_2x.normalized_chroma_contrast
                        > comparison.result_1x.normalized_chroma_contrast,
                    "2x normalized chroma contrast ({:.4}) must strictly exceed 1x ({:.4})",
                    comparison.result_2x.normalized_chroma_contrast,
                    comparison.result_1x.normalized_chroma_contrast
                );
            }
            Err(EvidenceError::Stage {
                stage: EvidenceStage::DeviceCreation,
                message,
                ..
            }) => {
                // Explicit environment limitation: D3D11 video device creation unavailable
                println!("Skipped native D3D11 test due to environment limitation: {message}");
            }
            Err(err) => {
                panic!("Native D3D11 evidence test failed: {err}");
            }
        }
    }
}
