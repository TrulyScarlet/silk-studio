//! Shared Windows D3D11 resources and same-device video conversion.
//!
//! Capture owns the WGC session and publishes [`GpuTextureSlot`] values through
//! the encoder API's opaque resolver. The Media Foundation backend can use the
//! same resource type without depending on the capture crate, then convert the
//! BGRA texture to a scaled NV12 texture on the same D3D11 device.

use std::mem::ManuallyDrop;
use std::sync::Arc;

use thiserror::Error;
use windows::core::{s, Interface, PCSTR};
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::{
    ID3DBlob, D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST, D3D_SRV_DIMENSION_TEXTURE2D,
};
use windows::Win32::Graphics::Direct3D11::{
    ID3D11Device, ID3D11DeviceContext, ID3D11PixelShader, ID3D11RenderTargetView, ID3D11Resource,
    ID3D11ShaderResourceView, ID3D11Texture2D, ID3D11VertexShader, ID3D11VideoContext,
    ID3D11VideoDevice, ID3D11VideoProcessor, ID3D11VideoProcessorEnumerator,
    D3D11_BIND_RENDER_TARGET, D3D11_BIND_VIDEO_ENCODER, D3D11_RENDER_TARGET_VIEW_DESC,
    D3D11_RENDER_TARGET_VIEW_DESC_0, D3D11_RTV_DIMENSION_TEXTURE2DARRAY,
    D3D11_SHADER_RESOURCE_VIEW_DESC, D3D11_SHADER_RESOURCE_VIEW_DESC_0, D3D11_TEX2D_ARRAY_RTV,
    D3D11_TEX2D_SRV, D3D11_TEX2D_VPIV, D3D11_TEX2D_VPOV, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
    D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE, D3D11_VIDEO_PROCESSOR_COLOR_SPACE,
    D3D11_VIDEO_PROCESSOR_CONTENT_DESC, D3D11_VIDEO_PROCESSOR_FORMAT_SUPPORT_INPUT,
    D3D11_VIDEO_PROCESSOR_FORMAT_SUPPORT_OUTPUT, D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC,
    D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0, D3D11_VIDEO_PROCESSOR_NOMINAL_RANGE_0_255,
    D3D11_VIDEO_PROCESSOR_NOMINAL_RANGE_16_235, D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC,
    D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0, D3D11_VIDEO_PROCESSOR_STREAM, D3D11_VIEWPORT,
    D3D11_VPIV_DIMENSION_TEXTURE2D, D3D11_VPOV_DIMENSION_TEXTURE2D,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_NV12, DXGI_FORMAT_R8G8_UNORM, DXGI_FORMAT_R8_UNORM,
    DXGI_RATIONAL, DXGI_SAMPLE_DESC,
};

pub mod evidence;

/// A D3D11 device and immediate context that are protected for concurrent use.
///
/// The capture backend enables `ID3D11Multithread` protection before publishing
/// this value. The encoder uses the context only on its worker thread, while
/// WGC's callback uses the same protected context for resource copies.
#[derive(Clone)]
pub struct GpuDeviceContext {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
}

impl GpuDeviceContext {
    pub fn new(device: ID3D11Device, context: ID3D11DeviceContext) -> Self {
        Self { device, context }
    }

    pub fn device(&self) -> &ID3D11Device {
        &self.device
    }

    pub fn context(&self) -> &ID3D11DeviceContext {
        &self.context
    }
}

/// A captured BGRA texture and the device that owns it.
///
/// The resource is safe to retain across the capture/encode boundary only
/// because the owning device has multithread protection enabled. The lease
/// returned by `GpuFrameContext` keeps this value alive while the encoder uses
/// the texture.
#[derive(Clone)]
pub struct GpuTextureSlot {
    texture: ID3D11Texture2D,
    device_context: Arc<GpuDeviceContext>,
}

unsafe impl Send for GpuTextureSlot {}
unsafe impl Sync for GpuTextureSlot {}

impl GpuTextureSlot {
    pub fn new(texture: ID3D11Texture2D, device_context: Arc<GpuDeviceContext>) -> Self {
        Self {
            texture,
            device_context,
        }
    }

    pub fn texture(&self) -> &ID3D11Texture2D {
        &self.texture
    }

    pub fn device_context(&self) -> &Arc<GpuDeviceContext> {
        &self.device_context
    }

    pub fn dimensions(&self) -> (u32, u32) {
        let mut description = D3D11_TEXTURE2D_DESC::default();
        unsafe {
            self.texture.GetDesc(&mut description);
        }
        (description.Width, description.Height)
    }
}

/// One same-device NV12 output surface produced by [`D3D11VideoConverter`].
pub struct GpuNv12Texture {
    texture: ID3D11Texture2D,
    width: u32,
    height: u32,
}

impl GpuNv12Texture {
    pub fn texture(&self) -> &ID3D11Texture2D {
        &self.texture
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }
}

#[derive(Debug, Error)]
pub enum GpuMediaError {
    #[error("D3D11 interface query failed during {stage}: {source}")]
    Interface {
        stage: &'static str,
        #[source]
        source: windows::core::Error,
    },

    #[error("D3D11 operation failed during {stage}: {source}")]
    Operation {
        stage: &'static str,
        #[source]
        source: windows::core::Error,
    },

    #[error("D3D11 video processor does not support {format} as {direction}")]
    UnsupportedFormat {
        format: &'static str,
        direction: &'static str,
    },

    #[error("invalid video conversion dimensions: {width}x{height}")]
    InvalidDimensions { width: u32, height: u32 },

    #[error("captured texture format is not BGRA8")]
    UnexpectedInputFormat,

    #[error("captured texture dimensions changed from {expected_width}x{expected_height} to {actual_width}x{actual_height}")]
    InputDimensionsChanged {
        expected_width: u32,
        expected_height: u32,
        actual_width: u32,
        actual_height: u32,
    },

    #[error("D3D11 operation did not return a resource during {stage}")]
    MissingResource { stage: &'static str },
}

fn interface_error(stage: &'static str, source: windows::core::Error) -> GpuMediaError {
    GpuMediaError::Interface { stage, source }
}

fn operation_error(stage: &'static str, source: windows::core::Error) -> GpuMediaError {
    GpuMediaError::Operation { stage, source }
}

const VS_SOURCE: &str = r#"
struct VS_OUTPUT {
    float4 pos : SV_Position;
    float2 tex : TEXCOORD0;
};
VS_OUTPUT VS_Main(uint id : SV_VertexID) {
    VS_OUTPUT output;
    output.tex = float2((id << 1) & 2, id & 2);
    output.pos = float4(output.tex * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
    return output;
}
"#;

const PS_LUMA_SOURCE: &str = r#"
struct VS_OUTPUT {
    float4 pos : SV_Position;
    float2 tex : TEXCOORD0;
};
Texture2D<float4> g_Input : register(t0);
float4 PS_Luma(VS_OUTPUT input) : SV_Target {
    int2 coord = int2(input.pos.xy);
    float3 rgb = g_Input.Load(int3(coord, 0)).rgb;
    float y = dot(rgb, float3(0.2126, 0.7152, 0.0722));
    float y_limited = (16.0 + 219.0 * saturate(y)) / 255.0;
    return float4(y_limited, 0.0, 0.0, 1.0);
}
"#;

const PS_CHROMA_SOURCE: &str = r#"
struct VS_OUTPUT {
    float4 pos : SV_Position;
    float2 tex : TEXCOORD0;
};
Texture2D<float4> g_Input : register(t0);
float3 RGBtoYUV(float3 rgb) {
    float y = dot(rgb, float3(0.2126, 0.7152, 0.0722));
    float u = (rgb.b - y) / 1.8556;
    float v = (rgb.r - y) / 1.5748;
    return float3(y, u, v);
}
float2 PS_Chroma(VS_OUTPUT input) : SV_Target {
    int2 baseCoord = int2(input.pos.xy) * 2;
    float3 p00 = g_Input.Load(int3(baseCoord + int2(0, 0), 0)).rgb;
    float3 p10 = g_Input.Load(int3(baseCoord + int2(1, 0), 0)).rgb;
    float3 p01 = g_Input.Load(int3(baseCoord + int2(0, 1), 0)).rgb;
    float3 p11 = g_Input.Load(int3(baseCoord + int2(1, 1), 0)).rgb;
    
    float3 yuv00 = RGBtoYUV(p00);
    float3 yuv10 = RGBtoYUV(p10);
    float3 yuv01 = RGBtoYUV(p01);
    float3 yuv11 = RGBtoYUV(p11);
    
    float sat00 = dot(yuv00.yz, yuv00.yz);
    float sat10 = dot(yuv10.yz, yuv10.yz);
    float sat01 = dot(yuv01.yz, yuv01.yz);
    float sat11 = dot(yuv11.yz, yuv11.yz);
    
    float maxSat = max(max(sat00, sat10), max(sat01, sat11));
    float lumaVariance = abs(yuv00.x - yuv10.x) + abs(yuv01.x - yuv11.x) + abs(yuv00.x - yuv01.x) + abs(yuv10.x - yuv11.x);
    
    const float eps = 1e-4;
    float w00 = pow(sat00 + eps, 1.5);
    float w10 = pow(sat10 + eps, 1.5);
    float w01 = pow(sat01 + eps, 1.5);
    float w11 = pow(sat11 + eps, 1.5);
    
    if (lumaVariance > 0.05 && maxSat > 0.01) {
        w00 *= (sat00 / (maxSat + eps));
        w10 *= (sat10 / (maxSat + eps));
        w01 *= (sat01 / (maxSat + eps));
        w11 *= (sat11 / (maxSat + eps));
    }
    
    float totalWeight = w00 + w10 + w01 + w11;
    float2 filteredUV = (w00 * yuv00.yz + w10 * yuv10.yz + w01 * yuv01.yz + w11 * yuv11.yz) / totalWeight;
    
    // Convex hull clamp
    float minU = min(min(yuv00.y, yuv10.y), min(yuv01.y, yuv11.y));
    float maxU = max(max(yuv00.y, yuv10.y), max(yuv01.y, yuv11.y));
    float minV = min(min(yuv00.z, yuv10.z), min(yuv01.z, yuv11.z));
    float maxV = max(max(yuv00.z, yuv10.z), max(yuv01.z, yuv11.z));
    filteredUV.x = clamp(filteredUV.x, minU, maxU);
    filteredUV.y = clamp(filteredUV.y, minV, maxV);
    
    // Quantize to limited range [16, 240] / 255.0
    float2 uv_limited = clamp((128.0 + 224.0 * filteredUV) / 255.0, 16.0 / 255.0, 240.0 / 255.0);
    return uv_limited;
}
"#;

fn compile_hlsl(source: &str, entry: PCSTR, target: PCSTR) -> Result<ID3DBlob, GpuMediaError> {
    let mut shader_blob = None;
    let mut error_blob = None;
    unsafe {
        D3DCompile(
            source.as_ptr() as *const _,
            source.len(),
            PCSTR::null(),
            None,
            None,
            entry,
            target,
            0,
            0,
            &mut shader_blob,
            Some(&mut error_blob),
        )
    }
    .map_err(|error| {
        let message = if let Some(ref err) = error_blob {
            let slice = unsafe {
                std::slice::from_raw_parts(err.GetBufferPointer() as *const u8, err.GetBufferSize())
            };
            String::from_utf8_lossy(slice).to_string()
        } else {
            error.to_string()
        };
        operation_error(
            "compile HLSL shader",
            windows::core::Error::new(error.code(), message),
        )
    })?;

    shader_blob.ok_or(GpuMediaError::MissingResource {
        stage: "compile HLSL shader",
    })
}

/// Custom planar D3D11 shader converter for full-screen triangle BGRA8 -> NV12 conversion
/// with saliency-guided bilateral chroma downsampling.
pub struct D3D11PlanarShaderConverter {
    device_context: Arc<GpuDeviceContext>,
    vertex_shader: ID3D11VertexShader,
    luma_pixel_shader: ID3D11PixelShader,
    chroma_pixel_shader: ID3D11PixelShader,
    width: u32,
    height: u32,
}

impl D3D11PlanarShaderConverter {
    pub fn new(
        device_context: Arc<GpuDeviceContext>,
        width: u32,
        height: u32,
    ) -> Result<Self, GpuMediaError> {
        validate_dimensions(width, height)?;

        let device = device_context.device();

        let vs_blob = compile_hlsl(VS_SOURCE, s!("VS_Main"), s!("vs_5_0"))?;
        let vs_slice = unsafe {
            std::slice::from_raw_parts(
                vs_blob.GetBufferPointer() as *const u8,
                vs_blob.GetBufferSize(),
            )
        };
        let mut vertex_shader = None;
        unsafe { device.CreateVertexShader(vs_slice, None, Some(&mut vertex_shader)) }
            .map_err(|error| operation_error("create vertex shader", error))?;
        let vertex_shader = vertex_shader.ok_or(GpuMediaError::MissingResource {
            stage: "create vertex shader",
        })?;

        let luma_blob = compile_hlsl(PS_LUMA_SOURCE, s!("PS_Luma"), s!("ps_5_0"))?;
        let luma_slice = unsafe {
            std::slice::from_raw_parts(
                luma_blob.GetBufferPointer() as *const u8,
                luma_blob.GetBufferSize(),
            )
        };
        let mut luma_pixel_shader = None;
        unsafe { device.CreatePixelShader(luma_slice, None, Some(&mut luma_pixel_shader)) }
            .map_err(|error| operation_error("create luma pixel shader", error))?;
        let luma_pixel_shader = luma_pixel_shader.ok_or(GpuMediaError::MissingResource {
            stage: "create luma pixel shader",
        })?;

        let chroma_blob = compile_hlsl(PS_CHROMA_SOURCE, s!("PS_Chroma"), s!("ps_5_0"))?;
        let chroma_slice = unsafe {
            std::slice::from_raw_parts(
                chroma_blob.GetBufferPointer() as *const u8,
                chroma_blob.GetBufferSize(),
            )
        };
        let mut chroma_pixel_shader = None;
        unsafe { device.CreatePixelShader(chroma_slice, None, Some(&mut chroma_pixel_shader)) }
            .map_err(|error| operation_error("create chroma pixel shader", error))?;
        let chroma_pixel_shader = chroma_pixel_shader.ok_or(GpuMediaError::MissingResource {
            stage: "create chroma pixel shader",
        })?;

        Ok(Self {
            device_context,
            vertex_shader,
            luma_pixel_shader,
            chroma_pixel_shader,
            width,
            height,
        })
    }

    pub fn convert(
        &self,
        input: &ID3D11Texture2D,
        output: &ID3D11Texture2D,
    ) -> Result<(), GpuMediaError> {
        let device = self.device_context.device();
        let context = self.device_context.context();

        // 1. Create SRV on input (DXGI_FORMAT_B8G8R8A8_UNORM)
        let srv_desc = D3D11_SHADER_RESOURCE_VIEW_DESC {
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            ViewDimension: D3D_SRV_DIMENSION_TEXTURE2D,
            Anonymous: D3D11_SHADER_RESOURCE_VIEW_DESC_0 {
                Texture2D: D3D11_TEX2D_SRV {
                    MostDetailedMip: 0,
                    MipLevels: 1,
                },
            },
        };
        let mut input_srv = None;
        unsafe { device.CreateShaderResourceView(input, Some(&srv_desc), Some(&mut input_srv)) }
            .map_err(|error| {
                operation_error("create input SRV for planar shader conversion", error)
            })?;
        let input_srv = input_srv.ok_or(GpuMediaError::MissingResource {
            stage: "create input SRV for planar shader conversion",
        })?;

        // 2. Create Planar RTV for Subresource 0 (DXGI_FORMAT_R8_UNORM) on output
        let luma_rtv_desc = D3D11_RENDER_TARGET_VIEW_DESC {
            Format: DXGI_FORMAT_R8_UNORM,
            ViewDimension: D3D11_RTV_DIMENSION_TEXTURE2DARRAY,
            Anonymous: D3D11_RENDER_TARGET_VIEW_DESC_0 {
                Texture2DArray: D3D11_TEX2D_ARRAY_RTV {
                    MipSlice: 0,
                    FirstArraySlice: 0,
                    ArraySize: 1,
                },
            },
        };
        let mut luma_rtv = None;
        unsafe { device.CreateRenderTargetView(output, Some(&luma_rtv_desc), Some(&mut luma_rtv)) }
            .map_err(|error| {
                operation_error("create luma RTV for planar shader conversion", error)
            })?;
        let luma_rtv = luma_rtv.ok_or(GpuMediaError::MissingResource {
            stage: "create luma RTV for planar shader conversion",
        })?;

        // 3. Create Planar RTV for Subresource 1 (DXGI_FORMAT_R8G8_UNORM) on output
        let chroma_rtv_desc = D3D11_RENDER_TARGET_VIEW_DESC {
            Format: DXGI_FORMAT_R8G8_UNORM,
            ViewDimension: D3D11_RTV_DIMENSION_TEXTURE2DARRAY,
            Anonymous: D3D11_RENDER_TARGET_VIEW_DESC_0 {
                Texture2DArray: D3D11_TEX2D_ARRAY_RTV {
                    MipSlice: 0,
                    FirstArraySlice: 1,
                    ArraySize: 1,
                },
            },
        };
        let mut chroma_rtv = None;
        unsafe {
            device.CreateRenderTargetView(output, Some(&chroma_rtv_desc), Some(&mut chroma_rtv))
        }
        .map_err(|error| {
            operation_error("create chroma RTV for planar shader conversion", error)
        })?;
        let chroma_rtv = chroma_rtv.ok_or(GpuMediaError::MissingResource {
            stage: "create chroma RTV for planar shader conversion",
        })?;

        unsafe {
            context.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            context.IASetInputLayout(None);
            context.VSSetShader(&self.vertex_shader, None);
            context.PSSetShaderResources(0, Some(&[Some(input_srv.clone())]));

            // Pass 1: Luma (Y) - Viewport (0, 0, W, H)
            let luma_viewport = D3D11_VIEWPORT {
                TopLeftX: 0.0,
                TopLeftY: 0.0,
                Width: self.width as f32,
                Height: self.height as f32,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            };
            context.PSSetShader(&self.luma_pixel_shader, None);
            context.OMSetRenderTargets(Some(&[Some(luma_rtv)]), None);
            context.RSSetViewports(Some(&[luma_viewport]));
            context.Draw(3, 0);

            // Pass 2: Chroma (UV) - Viewport (0, 0, W/2, H/2)
            let chroma_viewport = D3D11_VIEWPORT {
                TopLeftX: 0.0,
                TopLeftY: 0.0,
                Width: (self.width / 2) as f32,
                Height: (self.height / 2) as f32,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            };
            context.PSSetShader(&self.chroma_pixel_shader, None);
            context.OMSetRenderTargets(Some(&[Some(chroma_rtv)]), None);
            context.RSSetViewports(Some(&[chroma_viewport]));
            context.Draw(3, 0);

            // Clean up: Unbind RTVs, SRVs, and Shaders immediately
            let null_srv: [Option<ID3D11ShaderResourceView>; 1] = [None];
            context.PSSetShaderResources(0, Some(&null_srv));
            let null_rtv: [Option<ID3D11RenderTargetView>; 1] = [None];
            context.OMSetRenderTargets(Some(&null_rtv), None);
            context.VSSetShader(None, None);
            context.PSSetShader(None, None);
        }

        Ok(())
    }
}

/// D3D11 video-processor conversion from BGRA8 to scaled NV12.
///
/// The processor and its enumerator are tied to one input/output geometry. A
/// new converter is required when WGC reports a different source size.
pub struct D3D11VideoConverter {
    device_context: Arc<GpuDeviceContext>,
    video_device: ID3D11VideoDevice,
    video_context: ID3D11VideoContext,
    enumerator: ID3D11VideoProcessorEnumerator,
    processor: ID3D11VideoProcessor,
    planar_shader: Option<D3D11PlanarShaderConverter>,
    input_width: u32,
    input_height: u32,
    output_width: u32,
    output_height: u32,
}

impl D3D11VideoConverter {
    pub fn new(
        device_context: Arc<GpuDeviceContext>,
        input_width: u32,
        input_height: u32,
        output_width: u32,
        output_height: u32,
        fps: u32,
    ) -> Result<Self, GpuMediaError> {
        validate_dimensions(input_width, input_height)?;
        validate_dimensions(output_width, output_height)?;
        if fps == 0 {
            return Err(GpuMediaError::InvalidDimensions {
                width: output_width,
                height: output_height,
            });
        }

        let video_device: ID3D11VideoDevice = device_context
            .device()
            .cast()
            .map_err(|error| interface_error("query ID3D11VideoDevice", error))?;
        let video_context: ID3D11VideoContext = device_context
            .context()
            .cast()
            .map_err(|error| interface_error("query ID3D11VideoContext", error))?;
        let content = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
            InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
            InputFrameRate: DXGI_RATIONAL {
                Numerator: fps,
                Denominator: 1,
            },
            InputWidth: input_width,
            InputHeight: input_height,
            OutputFrameRate: DXGI_RATIONAL {
                Numerator: fps,
                Denominator: 1,
            },
            OutputWidth: output_width,
            OutputHeight: output_height,
            Usage: windows::Win32::Graphics::Direct3D11::D3D11_VIDEO_USAGE_PLAYBACK_NORMAL,
        };
        let enumerator = unsafe { video_device.CreateVideoProcessorEnumerator(&content) }
            .map_err(|error| operation_error("create video processor enumerator", error))?;

        let input_flags =
            unsafe { enumerator.CheckVideoProcessorFormat(DXGI_FORMAT_B8G8R8A8_UNORM) }
                .map_err(|error| operation_error("check BGRA video processor support", error))?;
        if input_flags & D3D11_VIDEO_PROCESSOR_FORMAT_SUPPORT_INPUT.0 as u32 == 0 {
            return Err(GpuMediaError::UnsupportedFormat {
                format: "BGRA8",
                direction: "input",
            });
        }
        let output_flags = unsafe { enumerator.CheckVideoProcessorFormat(DXGI_FORMAT_NV12) }
            .map_err(|error| operation_error("check NV12 video processor support", error))?;
        if output_flags & D3D11_VIDEO_PROCESSOR_FORMAT_SUPPORT_OUTPUT.0 as u32 == 0 {
            return Err(GpuMediaError::UnsupportedFormat {
                format: "NV12",
                direction: "output",
            });
        }

        let processor = unsafe { video_device.CreateVideoProcessor(&enumerator, 0) }
            .map_err(|error| operation_error("create video processor", error))?;

        let source_right =
            i32::try_from(input_width).map_err(|_| GpuMediaError::InvalidDimensions {
                width: input_width,
                height: input_height,
            })?;
        let source_bottom =
            i32::try_from(input_height).map_err(|_| GpuMediaError::InvalidDimensions {
                width: input_width,
                height: input_height,
            })?;
        let dest_right =
            i32::try_from(output_width).map_err(|_| GpuMediaError::InvalidDimensions {
                width: output_width,
                height: output_height,
            })?;
        let dest_bottom =
            i32::try_from(output_height).map_err(|_| GpuMediaError::InvalidDimensions {
                width: output_width,
                height: output_height,
            })?;

        let source_rect = RECT {
            left: 0,
            top: 0,
            right: source_right,
            bottom: source_bottom,
        };
        let dest_rect = RECT {
            left: 0,
            top: 0,
            right: dest_right,
            bottom: dest_bottom,
        };

        // Explicitly configure BT.709 full-range RGB input and BT.709
        // limited-range NV12 output. The D3D11 nominal-range enum differs from
        // Media Foundation's MFNominalRange values: 1 is 16-235 and 2 is 0-255.
        let (input_color_space, output_color_space) = bt709_rgb_to_nv12_color_spaces();
        unsafe {
            video_context.VideoProcessorSetStreamColorSpace(&processor, 0, &input_color_space);
            video_context.VideoProcessorSetOutputColorSpace(&processor, &output_color_space);
            video_context.VideoProcessorSetStreamSourceRect(
                &processor,
                0,
                true,
                Some(&source_rect),
            );
            video_context.VideoProcessorSetStreamDestRect(&processor, 0, true, Some(&dest_rect));
            video_context.VideoProcessorSetOutputTargetRect(&processor, true, Some(&dest_rect));
        }

        let planar_shader = if input_width == output_width && input_height == output_height {
            D3D11PlanarShaderConverter::new(device_context.clone(), output_width, output_height)
                .ok()
        } else {
            None
        };

        Ok(Self {
            device_context,
            video_device,
            video_context,
            enumerator,
            processor,
            planar_shader,
            input_width,
            input_height,
            output_width,
            output_height,
        })
    }

    pub fn convert(&self, source: &GpuTextureSlot) -> Result<GpuNv12Texture, GpuMediaError> {
        let mut source_desc = D3D11_TEXTURE2D_DESC::default();
        unsafe {
            source.texture().GetDesc(&mut source_desc);
        }
        if source_desc.Format != DXGI_FORMAT_B8G8R8A8_UNORM {
            return Err(GpuMediaError::UnexpectedInputFormat);
        }
        if source_desc.Width != self.input_width || source_desc.Height != self.input_height {
            return Err(GpuMediaError::InputDimensionsChanged {
                expected_width: self.input_width,
                expected_height: self.input_height,
                actual_width: source_desc.Width,
                actual_height: source_desc.Height,
            });
        }

        let output_desc = D3D11_TEXTURE2D_DESC {
            Width: self.output_width,
            Height: self.output_height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_NV12,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_VIDEO_ENCODER.0) as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let mut output_texture = None;
        unsafe {
            self.device_context.device().CreateTexture2D(
                &output_desc,
                None,
                Some(&mut output_texture),
            )
        }
        .map_err(|error| operation_error("create NV12 output texture", error))?;
        let output_texture = output_texture.ok_or(GpuMediaError::MissingResource {
            stage: "create NV12 output texture",
        })?;

        if let Some(ref planar) = self.planar_shader {
            if planar.convert(source.texture(), &output_texture).is_ok() {
                return Ok(GpuNv12Texture {
                    texture: output_texture,
                    width: self.output_width,
                    height: self.output_height,
                });
            }
        }

        let source_resource: ID3D11Resource = source
            .texture()
            .cast()
            .map_err(|error| interface_error("query source D3D11 resource", error))?;
        let input_desc = D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
            FourCC: 0,
            ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
            Anonymous: D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0 {
                Texture2D: D3D11_TEX2D_VPIV {
                    MipSlice: 0,
                    ArraySlice: 0,
                },
            },
        };
        let mut input_view = None;
        unsafe {
            self.video_device.CreateVideoProcessorInputView(
                &source_resource,
                &self.enumerator,
                &input_desc,
                Some(&mut input_view),
            )
        }
        .map_err(|error| operation_error("create BGRA processor input view", error))?;
        let input_view = input_view.ok_or(GpuMediaError::MissingResource {
            stage: "create BGRA processor input view",
        })?;

        let output_desc = D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
            ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
            Anonymous: D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 {
                Texture2D: D3D11_TEX2D_VPOV { MipSlice: 0 },
            },
        };
        let mut output_view = None;
        unsafe {
            self.video_device.CreateVideoProcessorOutputView(
                &output_texture,
                &self.enumerator,
                &output_desc,
                Some(&mut output_view),
            )
        }
        .map_err(|error| operation_error("create NV12 processor output view", error))?;
        let output_view = output_view.ok_or(GpuMediaError::MissingResource {
            stage: "create NV12 processor output view",
        })?;

        let mut stream = D3D11_VIDEO_PROCESSOR_STREAM {
            Enable: true.into(),
            pInputSurface: ManuallyDrop::new(Some(input_view)),
            ..Default::default()
        };
        unsafe {
            self.video_context.VideoProcessorSetStreamFrameFormat(
                &self.processor,
                0,
                D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
            );
        }
        let result = unsafe {
            self.video_context.VideoProcessorBlt(
                &self.processor,
                &output_view,
                0,
                std::slice::from_ref(&stream),
            )
        };
        unsafe {
            drop(ManuallyDrop::take(&mut stream.pInputSurface));
        }
        result.map_err(|error| operation_error("convert BGRA to NV12", error))?;

        Ok(GpuNv12Texture {
            texture: output_texture,
            width: self.output_width,
            height: self.output_height,
        })
    }
}

fn bt709_rgb_to_nv12_color_spaces() -> (
    D3D11_VIDEO_PROCESSOR_COLOR_SPACE,
    D3D11_VIDEO_PROCESSOR_COLOR_SPACE,
) {
    const BT709_MATRIX_BIT: u32 = 1 << 2;
    const NOMINAL_RANGE_SHIFT: u32 = 4;

    let input = D3D11_VIDEO_PROCESSOR_COLOR_SPACE {
        // RGB_Range remains zero (full range). Nominal_Range is also marked
        // full for drivers that inspect it while converting an RGB stream.
        _bitfield: BT709_MATRIX_BIT
            | ((D3D11_VIDEO_PROCESSOR_NOMINAL_RANGE_0_255.0 as u32) << NOMINAL_RANGE_SHIFT),
    };
    let output = D3D11_VIDEO_PROCESSOR_COLOR_SPACE {
        _bitfield: BT709_MATRIX_BIT
            | ((D3D11_VIDEO_PROCESSOR_NOMINAL_RANGE_16_235.0 as u32) << NOMINAL_RANGE_SHIFT),
    };
    (input, output)
}

fn validate_dimensions(width: u32, height: u32) -> Result<(), GpuMediaError> {
    if width == 0 || height == 0 || !width.is_multiple_of(2) || !height.is_multiple_of(2) {
        return Err(GpuMediaError::InvalidDimensions { width, height });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bt709_conversion_uses_full_rgb_and_limited_nv12_ranges() {
        let (input, output) = bt709_rgb_to_nv12_color_spaces();

        assert_eq!(
            (input._bitfield >> 1) & 1,
            0,
            "RGB input must be full range"
        );
        assert_eq!((input._bitfield >> 2) & 1, 1, "input matrix must be BT.709");
        assert_eq!(
            (input._bitfield >> 4) & 0b11,
            D3D11_VIDEO_PROCESSOR_NOMINAL_RANGE_0_255.0 as u32
        );
        assert_eq!(
            (output._bitfield >> 2) & 1,
            1,
            "output matrix must be BT.709"
        );
        assert_eq!(
            (output._bitfield >> 4) & 0b11,
            D3D11_VIDEO_PROCESSOR_NOMINAL_RANGE_16_235.0 as u32
        );
    }
}
