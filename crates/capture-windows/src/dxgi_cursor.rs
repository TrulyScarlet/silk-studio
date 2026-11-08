//! DXGI pointer composition. Only the small cursor rectangle is read back;
//! desktop frames remain on the GPU. Sizes and buffer indexing are bounded.
use gpu_windows::GpuDeviceContext;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::*;

use super::dxgi::{backend, texture};
use capture_api::Result;

const MAX_SHAPE_BYTES: usize = 1024 * 1024;
const MAX_CURSOR_EDGE: u32 = 512;

#[derive(Default)]
pub(super) struct Cursor {
    shape: DXGI_OUTDUPL_POINTER_SHAPE_INFO,
    bytes: Vec<u8>,
    position: DXGI_OUTDUPL_POINTER_POSITION,
    staging: Option<ID3D11Texture2D>,
}

impl Cursor {
    pub fn update(
        &mut self,
        duplication: &IDXGIOutputDuplication,
        info: &DXGI_OUTDUPL_FRAME_INFO,
    ) -> Result<()> {
        if info.LastMouseUpdateTime != 0 {
            self.position = info.PointerPosition;
        }
        if info.PointerShapeBufferSize == 0 {
            return Ok(());
        }
        if info.PointerShapeBufferSize as usize > MAX_SHAPE_BYTES {
            return Err(backend("DXGI cursor exceeds size limit"));
        }
        self.bytes.resize(info.PointerShapeBufferSize as usize, 0);
        let mut needed = 0;
        let mut shape = DXGI_OUTDUPL_POINTER_SHAPE_INFO::default();
        unsafe {
            duplication.GetFramePointerShape(
                self.bytes.len() as u32,
                self.bytes.as_mut_ptr().cast(),
                &mut needed,
                &mut shape,
            )
        }
        .map_err(backend)?;
        if !valid_shape(&shape, self.bytes.len()) {
            return Err(backend("invalid DXGI cursor shape"));
        }
        if (shape.Width, shape_height(&shape)) != (self.shape.Width, shape_height(&self.shape)) {
            self.staging = None;
        }
        self.shape = shape;
        Ok(())
    }

    pub fn draw(
        &mut self,
        gpu: &GpuDeviceContext,
        target: &ID3D11Texture2D,
        width: u32,
        height: u32,
    ) -> Result<()> {
        if !self.position.Visible.as_bool() || self.bytes.is_empty() {
            return Ok(());
        }
        let cursor_height = shape_height(&self.shape);
        let Some((left, top, right, bottom, offset_x, offset_y)) = clipped_rect(
            self.position.Position.x,
            self.position.Position.y,
            self.shape.Width,
            cursor_height,
            width,
            height,
        ) else {
            return Ok(());
        };
        if self.staging.is_none() {
            self.staging = Some(texture(gpu, self.shape.Width, cursor_height, true)?);
        }
        let staging = self.staging.as_ref().expect("staging created");
        let region = D3D11_BOX {
            left,
            top,
            front: 0,
            right,
            bottom,
            back: 1,
        };
        unsafe {
            gpu.context()
                .CopySubresourceRegion(staging, 0, 0, 0, 0, target, 0, Some(&region));
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            gpu.context()
                .Map(staging, 0, D3D11_MAP_READ_WRITE, 0, Some(&mut mapped))
                .map_err(backend)?;
            // The mapped staging allocation has at least RowPitch * Height bytes.
            let pixels = std::slice::from_raw_parts_mut(
                mapped.pData.cast::<u8>(),
                mapped.RowPitch as usize * cursor_height as usize,
            );
            for y in 0..bottom - top {
                for x in 0..right - left {
                    let offset = y as usize * mapped.RowPitch as usize + x as usize * 4;
                    blend(
                        &self.shape,
                        &self.bytes,
                        x + offset_x,
                        y + offset_y,
                        &mut pixels[offset..offset + 4],
                    );
                }
            }
            gpu.context().Unmap(staging, 0);
            let patch = D3D11_BOX {
                left: 0,
                top: 0,
                front: 0,
                right: right - left,
                bottom: bottom - top,
                back: 1,
            };
            gpu.context()
                .CopySubresourceRegion(target, 0, left, top, 0, staging, 0, Some(&patch));
        }
        Ok(())
    }
}

fn shape_height(shape: &DXGI_OUTDUPL_POINTER_SHAPE_INFO) -> u32 {
    if shape.Type == 1 {
        shape.Height / 2
    } else {
        shape.Height
    }
}

fn valid_shape(s: &DXGI_OUTDUPL_POINTER_SHAPE_INFO, size: usize) -> bool {
    let row = match s.Type {
        1 => s.Width.div_ceil(8),
        2 | 4 => s.Width.saturating_mul(4),
        _ => return false,
    };
    s.Width > 0
        && s.Width <= MAX_CURSOR_EDGE
        && shape_height(s) > 0
        && shape_height(s) <= MAX_CURSOR_EDGE
        && (s.Type != 1 || s.Height.is_multiple_of(2))
        && s.Pitch >= row
        && u64::from(s.Pitch) * u64::from(s.Height) <= size as u64
}

// DXGI reports the top-left position relative to this output, not the hotspot.
fn clipped_rect(
    x: i32,
    y: i32,
    cw: u32,
    ch: u32,
    width: u32,
    height: u32,
) -> Option<(u32, u32, u32, u32, u32, u32)> {
    let left = i64::from(x).max(0);
    let top = i64::from(y).max(0);
    let right = (i64::from(x) + i64::from(cw)).min(i64::from(width));
    let bottom = (i64::from(y) + i64::from(ch)).min(i64::from(height));
    (right > left && bottom > top).then_some((
        left as u32,
        top as u32,
        right as u32,
        bottom as u32,
        (left - i64::from(x)) as u32,
        (top - i64::from(y)) as u32,
    ))
}

fn blend(s: &DXGI_OUTDUPL_POINTER_SHAPE_INFO, bytes: &[u8], x: u32, y: u32, pixel: &mut [u8]) {
    if s.Type == 1 {
        let offset = y as usize * s.Pitch as usize + (x / 8) as usize;
        let mask = 0x80 >> (x % 8);
        let and = if bytes[offset] & mask != 0 { 255 } else { 0 };
        let xor = if bytes[offset + (s.Height / 2 * s.Pitch) as usize] & mask != 0 {
            255
        } else {
            0
        };
        for component in &mut pixel[..3] {
            *component = (*component & and) ^ xor;
        }
    } else {
        let offset = y as usize * s.Pitch as usize + x as usize * 4;
        let source = &bytes[offset..offset + 4];
        for channel in 0..3 {
            pixel[channel] = if s.Type == 4 {
                if source[3] == 0 {
                    source[channel]
                } else {
                    pixel[channel] ^ source[channel]
                }
            } else {
                // DXGI color cursor channels use premultiplied alpha.
                (u16::from(source[channel])
                    + (u16::from(pixel[channel]) * (255 - u16::from(source[3])) + 127) / 255)
                    .min(255) as u8
            };
        }
    }
    pixel[3] = 255;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cursor_clips_at_each_monitor_edge() {
        assert_eq!(
            clipped_rect(-2, -3, 8, 8, 100, 100),
            Some((0, 0, 6, 5, 2, 3))
        );
        assert_eq!(
            clipped_rect(98, 99, 8, 8, 100, 100),
            Some((98, 99, 100, 100, 0, 0))
        );
        assert_eq!(clipped_rect(101, 10, 8, 8, 100, 100), None);
    }
    #[test]
    fn monochrome_and_xor_masks_cover_all_operations() {
        let shape = DXGI_OUTDUPL_POINTER_SHAPE_INFO {
            Type: 1,
            Width: 4,
            Height: 2,
            Pitch: 1,
            ..Default::default()
        };
        let bytes = [0b00110000, 0b01010000];
        for (x, expected) in [0, 255, 30, 225].into_iter().enumerate() {
            let mut pixel = [30, 30, 30, 255];
            blend(&shape, &bytes, x as u32, 0, &mut pixel);
            assert_eq!(pixel, [expected, expected, expected, 255]);
        }
    }
    #[test]
    fn color_and_masked_color_keep_correct_channels() {
        let mut shape = DXGI_OUTDUPL_POINTER_SHAPE_INFO {
            Type: 2,
            Width: 1,
            Height: 1,
            Pitch: 4,
            ..Default::default()
        };
        let mut pixel = [100, 100, 100, 255];
        blend(&shape, &[50, 0, 0, 128], 0, 0, &mut pixel);
        assert_eq!(pixel, [100, 50, 50, 255]);
        shape.Type = 4;
        blend(&shape, &[1, 2, 3, 0], 0, 0, &mut pixel);
        assert_eq!(pixel, [1, 2, 3, 255]);
        blend(&shape, &[255, 255, 255, 255], 0, 0, &mut pixel);
        assert_eq!(pixel, [254, 253, 252, 255]);
        assert!(valid_shape(&shape, 4));
        assert!(!valid_shape(&shape, 3));
        shape.Width = 513;
        assert!(!valid_shape(&shape, MAX_SHAPE_BYTES));
    }
}
