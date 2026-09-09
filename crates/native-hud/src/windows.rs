//! Windows Direct2D, DirectWrite, Direct3D11, and DirectComposition backend.

use std::mem::ManuallyDrop;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use windows::core::{w, Interface, BOOL, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_FIGURE_BEGIN_HOLLOW, D2D1_FIGURE_END_OPEN,
    D2D1_GRADIENT_STOP, D2D1_PIXEL_FORMAT, D2D_RECT_F, D2D_SIZE_F,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1Device, ID2D1DeviceContext, ID2D1Factory, ID2D1Factory1,
    ID2D1LinearGradientBrush, ID2D1PathGeometry, ID2D1RenderTarget, ID2D1SolidColorBrush,
    ID2D1StrokeStyle, D2D1_ARC_SEGMENT, D2D1_ARC_SIZE_LARGE, D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
    D2D1_BITMAP_OPTIONS_TARGET, D2D1_BITMAP_PROPERTIES1, D2D1_CAP_STYLE_ROUND,
    D2D1_DEVICE_CONTEXT_OPTIONS_NONE, D2D1_DRAW_TEXT_OPTIONS_CLIP,
    D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT, D2D1_ELLIPSE, D2D1_EXTEND_MODE_CLAMP,
    D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_GAMMA_2_2, D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES,
    D2D1_LINE_JOIN_ROUND, D2D1_ROUNDED_RECT, D2D1_STROKE_STYLE_PROPERTIES,
    D2D1_SWEEP_DIRECTION_CLOCKWISE, D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE,
};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
    D3D11_SDK_VERSION,
};
use windows::Win32::Graphics::DirectComposition::{
    DCompositionCreateDevice, IDCompositionAnimation, IDCompositionDevice,
    IDCompositionEffectGroup, IDCompositionTarget, IDCompositionVisual,
};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, IDWriteInlineObject, IDWriteTextFormat,
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT_NORMAL,
    DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_LINE_SPACING_METHOD_UNIFORM,
    DWRITE_MEASURING_MODE_NATURAL, DWRITE_PARAGRAPH_ALIGNMENT_NEAR, DWRITE_TEXT_ALIGNMENT_LEADING,
    DWRITE_TRIMMING, DWRITE_TRIMMING_GRANULARITY_CHARACTER,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    IDXGIAdapter, IDXGIDevice, IDXGIFactory2, IDXGISurface, IDXGISwapChain1, DXGI_PRESENT,
    DXGI_SCALING_STRETCH, DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_EFFECT_FLIP_DISCARD,
    DXGI_USAGE_RENDER_TARGET_OUTPUT,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, GetMonitorInfoW, MonitorFromWindow, MONITORINFO,
    MONITOR_DEFAULTTOPRIMARY, PAINTSTRUCT,
};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    GetDpiForWindow, SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT,
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, KillTimer,
    PostMessageW, PostQuitMessage, RegisterClassExW, SetTimer, SetWindowDisplayAffinity,
    SetWindowPos, ShowWindow, SystemParametersInfoW, TranslateMessage, CS_HREDRAW, CS_VREDRAW,
    HTTRANSPARENT, MA_NOACTIVATE, MSG, SPI_GETCLIENTAREAANIMATION, SWP_HIDEWINDOW, SWP_NOACTIVATE,
    SWP_NOZORDER, SW_HIDE, SW_SHOWNOACTIVATE, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
    WDA_EXCLUDEFROMCAPTURE, WM_CLOSE, WM_DESTROY, WM_ERASEBKGND, WM_MOUSEACTIVATE, WM_NCHITTEST,
    WM_PAINT, WM_TIMER, WM_USER, WNDCLASSEXW, WS_EX_NOACTIVATE, WS_EX_NOREDIRECTIONBITMAP,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows_numerics::{Matrix3x2, Vector2};

use crate::format::{
    encode_truncated_utf16, format_duration_into_utf16, format_file_size_into_utf16,
    format_saved_subtitle_into_utf16, SUBTITLE_UTF16_BUF_SIZE, TITLE_UTF16_BUF_SIZE,
};
use crate::geometry::*;
use crate::types::*;

const WINDOW_CLASS_NAME: PCWSTR = w!("SilkNativeHudWindowClass");
const WM_HUD_COMMAND: u32 = WM_USER + 101;
const TIMER_ID_HOLD: usize = 1;
const TIMER_ID_EXIT: usize = 2;

#[inline]
const fn pt(x: f32, y: f32) -> Vector2 {
    Vector2 { X: x, Y: y }
}

#[inline]
const fn matrix3x2_identity() -> Matrix3x2 {
    Matrix3x2 {
        M11: 1.0,
        M12: 0.0,
        M21: 0.0,
        M22: 1.0,
        M31: 0.0,
        M32: 0.0,
    }
}

#[inline]
#[allow(dead_code)]
const fn matrix3x2_translation(x: f32, y: f32) -> Matrix3x2 {
    Matrix3x2 {
        M11: 1.0,
        M12: 0.0,
        M21: 0.0,
        M22: 1.0,
        M31: x,
        M32: y,
    }
}

enum HudCommand {
    Show(HudCue),
    Shutdown,
}

struct ComApartmentGuard {
    initialized: bool,
}

impl ComApartmentGuard {
    fn new() -> Result<Self, HudError> {
        let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        if hr.is_err() {
            return Err(HudError::InitializationFailed(format!(
                "CoInitializeEx failed: {hr:?}"
            )));
        }
        Ok(Self { initialized: true })
    }
}

impl Drop for ComApartmentGuard {
    fn drop(&mut self) {
        if self.initialized {
            unsafe { CoUninitialize() };
            self.initialized = false;
        }
    }
}

struct DpiAwarenessGuard {
    previous_context: DPI_AWARENESS_CONTEXT,
}

impl DpiAwarenessGuard {
    fn new() -> Self {
        let previous_context =
            unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        Self { previous_context }
    }
}

impl Drop for DpiAwarenessGuard {
    fn drop(&mut self) {
        if !self.previous_context.is_invalid() {
            unsafe {
                let _ = SetThreadDpiAwarenessContext(self.previous_context);
            }
        }
    }
}

/// Pre-allocated Direct2D brushes, vector geometries, and DirectComposition animation objects.
struct PreallocatedResources {
    // Background gradients
    queued_bg_brush: ID2D1LinearGradientBrush,
    saved_bg_brush: ID2D1LinearGradientBrush,
    failed_bg_brush: ID2D1LinearGradientBrush,
    compact_queued_bg_brush: ID2D1LinearGradientBrush,
    compact_saved_bg_brush: ID2D1LinearGradientBrush,
    compact_failed_bg_brush: ID2D1LinearGradientBrush,
    // Outer border brushes
    queued_border_brush: ID2D1SolidColorBrush,
    saved_border_brush: ID2D1SolidColorBrush,
    failed_border_brush: ID2D1SolidColorBrush,
    // Badge fills
    queued_badge_fill_brush: ID2D1SolidColorBrush,
    saved_badge_fill_brush: ID2D1SolidColorBrush,
    failed_badge_fill_brush: ID2D1SolidColorBrush,
    // Badge border
    queued_badge_border_brush: ID2D1SolidColorBrush,
    // Icon brushes
    queued_track_brush: ID2D1SolidColorBrush,
    queued_arc_brush: ID2D1SolidColorBrush,
    queued_dot_brush: ID2D1SolidColorBrush,
    saved_ink_brush: ID2D1SolidColorBrush,
    failed_ink_brush: ID2D1SolidColorBrush,
    // Text brushes
    queued_title_brush: ID2D1SolidColorBrush,
    saved_title_brush: ID2D1SolidColorBrush,
    failed_title_brush: ID2D1SolidColorBrush,
    queued_subtitle_brush: ID2D1SolidColorBrush,
    saved_subtitle_brush: ID2D1SolidColorBrush,
    failed_subtitle_brush: ID2D1SolidColorBrush,
    // Stroke styles
    round_stroke_style: ID2D1StrokeStyle,
    // Vector path geometries
    queued_arc_geometry: ID2D1PathGeometry,
    saved_checkmark_geometry: ID2D1PathGeometry,
    compact_queued_arc_geometry: ID2D1PathGeometry,
    compact_saved_checkmark_geometry: ID2D1PathGeometry,
    // DirectComposition Animations pre-allocated at startup
    entrance_opacity_anim: IDCompositionAnimation,
    entrance_offset_anim: IDCompositionAnimation,
    exit_opacity_anim: IDCompositionAnimation,
}

impl PreallocatedResources {
    unsafe fn init(
        d2d_context: &ID2D1DeviceContext,
        d2d_factory: &ID2D1Factory,
        dcomp_device: &IDCompositionDevice,
    ) -> Result<Self, HudError> {
        let render_target: ID2D1RenderTarget = d2d_context
            .cast()
            .map_err(|e| HudError::Direct2DFailed(e.to_string()))?;

        // 1. Background Gradients
        let queued_stops = [
            D2D1_GRADIENT_STOP {
                position: 0.0,
                color: D2D1_COLOR_F {
                    r: 0.1255,
                    g: 0.1333,
                    b: 0.0902,
                    a: 0.960,
                },
            },
            D2D1_GRADIENT_STOP {
                position: 1.0,
                color: D2D1_COLOR_F {
                    r: 0.0627,
                    g: 0.0667,
                    b: 0.0471,
                    a: 0.970,
                },
            },
        ];
        let saved_stops = [
            D2D1_GRADIENT_STOP {
                position: 0.0,
                color: D2D1_COLOR_F {
                    r: 0.0706,
                    g: 0.1333,
                    b: 0.0863,
                    a: 0.960,
                },
            },
            D2D1_GRADIENT_STOP {
                position: 1.0,
                color: D2D1_COLOR_F {
                    r: 0.0392,
                    g: 0.0784,
                    b: 0.0510,
                    a: 0.970,
                },
            },
        ];
        let failed_stops = [
            D2D1_GRADIENT_STOP {
                position: 0.0,
                color: D2D1_COLOR_F {
                    r: 0.1490,
                    g: 0.0706,
                    b: 0.0627,
                    a: 0.960,
                },
            },
            D2D1_GRADIENT_STOP {
                position: 1.0,
                color: D2D1_COLOR_F {
                    r: 0.0784,
                    g: 0.0392,
                    b: 0.0353,
                    a: 0.970,
                },
            },
        ];

        let queued_stop_coll = render_target
            .CreateGradientStopCollection(&queued_stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP)
            .map_err(|e| HudError::Direct2DFailed(e.to_string()))?;
        let saved_stop_coll = render_target
            .CreateGradientStopCollection(&saved_stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP)
            .map_err(|e| HudError::Direct2DFailed(e.to_string()))?;
        let failed_stop_coll = render_target
            .CreateGradientStopCollection(&failed_stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP)
            .map_err(|e| HudError::Direct2DFailed(e.to_string()))?;

        let brush_props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
            startPoint: pt(CANVAS_WIDTH_DIP / 2.0, 0.0),
            endPoint: pt(CANVAS_WIDTH_DIP / 2.0, CANVAS_HEIGHT_DIP),
        };

        let queued_bg_brush = d2d_context
            .CreateLinearGradientBrush(&brush_props, None, &queued_stop_coll)
            .map_err(|e| HudError::Direct2DFailed(e.to_string()))?;
        let saved_bg_brush = d2d_context
            .CreateLinearGradientBrush(&brush_props, None, &saved_stop_coll)
            .map_err(|e| HudError::Direct2DFailed(e.to_string()))?;
        let failed_bg_brush = d2d_context
            .CreateLinearGradientBrush(&brush_props, None, &failed_stop_coll)
            .map_err(|e| HudError::Direct2DFailed(e.to_string()))?;

        let compact_brush_props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
            startPoint: pt(COMPACT_WIDTH_DIP / 2.0, 0.0),
            endPoint: pt(COMPACT_WIDTH_DIP / 2.0, COMPACT_HEIGHT_DIP),
        };

        let compact_queued_bg_brush = d2d_context
            .CreateLinearGradientBrush(&compact_brush_props, None, &queued_stop_coll)
            .map_err(|e| HudError::Direct2DFailed(e.to_string()))?;
        let compact_saved_bg_brush = d2d_context
            .CreateLinearGradientBrush(&compact_brush_props, None, &saved_stop_coll)
            .map_err(|e| HudError::Direct2DFailed(e.to_string()))?;
        let compact_failed_bg_brush = d2d_context
            .CreateLinearGradientBrush(&compact_brush_props, None, &failed_stop_coll)
            .map_err(|e| HudError::Direct2DFailed(e.to_string()))?;

        // 2. Outer Border Brushes
        let queued_border_brush = d2d_context
            .CreateSolidColorBrush(
                &D2D1_COLOR_F {
                    r: 1.0,
                    g: 1.0,
                    b: 0.702,
                    a: 0.40,
                },
                None,
            )
            .map_err(|e| HudError::Direct2DFailed(e.to_string()))?;
        let saved_border_brush = d2d_context
            .CreateSolidColorBrush(
                &D2D1_COLOR_F {
                    r: 0.7333,
                    g: 0.9686,
                    b: 0.8157,
                    a: 0.48,
                },
                None,
            )
            .map_err(|e| HudError::Direct2DFailed(e.to_string()))?;
        let failed_border_brush = d2d_context
            .CreateSolidColorBrush(
                &D2D1_COLOR_F {
                    r: 1.0,
                    g: 0.6667,
                    b: 0.6275,
                    a: 0.50,
                },
                None,
            )
            .map_err(|e| HudError::Direct2DFailed(e.to_string()))?;

        // 3. Badge Fills
        let queued_badge_fill_brush = d2d_context
            .CreateSolidColorBrush(
                &D2D1_COLOR_F {
                    r: 1.0,
                    g: 1.0,
                    b: 0.702,
                    a: 0.12,
                },
                None,
            )
            .map_err(|e| HudError::Direct2DFailed(e.to_string()))?;
        let saved_badge_fill_brush = d2d_context
            .CreateSolidColorBrush(
                &D2D1_COLOR_F {
                    r: 0.7333,
                    g: 0.9686,
                    b: 0.8157,
                    a: 1.0,
                },
                None,
            )
            .map_err(|e| HudError::Direct2DFailed(e.to_string()))?;
        let failed_badge_fill_brush = d2d_context
            .CreateSolidColorBrush(
                &D2D1_COLOR_F {
                    r: 1.0,
                    g: 0.6667,
                    b: 0.6275,
                    a: 1.0,
                },
                None,
            )
            .map_err(|e| HudError::Direct2DFailed(e.to_string()))?;

        // 4. Badge Border
        let queued_badge_border_brush = d2d_context
            .CreateSolidColorBrush(
                &D2D1_COLOR_F {
                    r: 1.0,
                    g: 1.0,
                    b: 0.702,
                    a: 0.24,
                },
                None,
            )
            .map_err(|e| HudError::Direct2DFailed(e.to_string()))?;

        // 5. Icon Brushes
        let queued_track_brush = d2d_context
            .CreateSolidColorBrush(
                &D2D1_COLOR_F {
                    r: 1.0,
                    g: 1.0,
                    b: 0.702,
                    a: 0.18,
                },
                None,
            )
            .map_err(|e| HudError::Direct2DFailed(e.to_string()))?;
        let queued_arc_brush = d2d_context
            .CreateSolidColorBrush(
                &D2D1_COLOR_F {
                    r: 1.0,
                    g: 1.0,
                    b: 0.702,
                    a: 0.92,
                },
                None,
            )
            .map_err(|e| HudError::Direct2DFailed(e.to_string()))?;
        let queued_dot_brush = d2d_context
            .CreateSolidColorBrush(
                &D2D1_COLOR_F {
                    r: 1.0,
                    g: 1.0,
                    b: 0.702,
                    a: 0.85,
                },
                None,
            )
            .map_err(|e| HudError::Direct2DFailed(e.to_string()))?;
        let saved_ink_brush = d2d_context
            .CreateSolidColorBrush(
                &D2D1_COLOR_F {
                    r: 0.0471,
                    g: 0.0980,
                    b: 0.0588,
                    a: 1.0,
                },
                None,
            )
            .map_err(|e| HudError::Direct2DFailed(e.to_string()))?;
        let failed_ink_brush = d2d_context
            .CreateSolidColorBrush(
                &D2D1_COLOR_F {
                    r: 0.1255,
                    g: 0.0392,
                    b: 0.0314,
                    a: 1.0,
                },
                None,
            )
            .map_err(|e| HudError::Direct2DFailed(e.to_string()))?;

        // 6. Text Brushes
        let queued_title_brush = d2d_context
            .CreateSolidColorBrush(
                &D2D1_COLOR_F {
                    r: 1.0,
                    g: 1.0,
                    b: 0.702,
                    a: 1.0,
                },
                None,
            )
            .map_err(|e| HudError::Direct2DFailed(e.to_string()))?;
        let saved_title_brush = d2d_context
            .CreateSolidColorBrush(
                &D2D1_COLOR_F {
                    r: 0.9020,
                    g: 1.0,
                    b: 0.9294,
                    a: 1.0,
                },
                None,
            )
            .map_err(|e| HudError::Direct2DFailed(e.to_string()))?;
        let failed_title_brush = d2d_context
            .CreateSolidColorBrush(
                &D2D1_COLOR_F {
                    r: 1.0,
                    g: 0.8941,
                    b: 0.8784,
                    a: 1.0,
                },
                None,
            )
            .map_err(|e| HudError::Direct2DFailed(e.to_string()))?;

        let queued_subtitle_brush = d2d_context
            .CreateSolidColorBrush(
                &D2D1_COLOR_F {
                    r: 1.0,
                    g: 1.0,
                    b: 0.702,
                    a: 0.76,
                },
                None,
            )
            .map_err(|e| HudError::Direct2DFailed(e.to_string()))?;
        let saved_subtitle_brush = d2d_context
            .CreateSolidColorBrush(
                &D2D1_COLOR_F {
                    r: 0.7333,
                    g: 0.9686,
                    b: 0.8157,
                    a: 0.80,
                },
                None,
            )
            .map_err(|e| HudError::Direct2DFailed(e.to_string()))?;
        let failed_subtitle_brush = d2d_context
            .CreateSolidColorBrush(
                &D2D1_COLOR_F {
                    r: 1.0,
                    g: 0.6667,
                    b: 0.6275,
                    a: 0.82,
                },
                None,
            )
            .map_err(|e| HudError::Direct2DFailed(e.to_string()))?;

        // 7. Stroke Style
        let stroke_props = D2D1_STROKE_STYLE_PROPERTIES {
            startCap: D2D1_CAP_STYLE_ROUND,
            endCap: D2D1_CAP_STYLE_ROUND,
            dashCap: D2D1_CAP_STYLE_ROUND,
            lineJoin: D2D1_LINE_JOIN_ROUND,
            miterLimit: 1.0,
            dashStyle: windows::Win32::Graphics::Direct2D::D2D1_DASH_STYLE_SOLID,
            dashOffset: 0.0,
        };
        let round_stroke_style = d2d_factory
            .CreateStrokeStyle(&stroke_props, None)
            .map_err(|e| HudError::Direct2DFailed(e.to_string()))?;

        // 8. Vector Geometries
        let queued_arc_geometry = d2d_factory
            .CreatePathGeometry()
            .map_err(|e| HudError::Direct2DFailed(e.to_string()))?;
        let sink = queued_arc_geometry
            .Open()
            .map_err(|e| HudError::Direct2DFailed(e.to_string()))?;
        sink.BeginFigure(pt(31.0, 20.0), D2D1_FIGURE_BEGIN_HOLLOW);
        sink.AddArc(&D2D1_ARC_SEGMENT {
            point: pt(24.07, 32.0),
            size: D2D_SIZE_F {
                width: 8.0,
                height: 8.0,
            },
            rotationAngle: 0.0,
            sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
            arcSize: D2D1_ARC_SIZE_LARGE,
        });
        sink.EndFigure(D2D1_FIGURE_END_OPEN);
        sink.Close()
            .map_err(|e| HudError::Direct2DFailed(e.to_string()))?;

        let saved_checkmark_geometry = d2d_factory
            .CreatePathGeometry()
            .map_err(|e| HudError::Direct2DFailed(e.to_string()))?;
        let sink = saved_checkmark_geometry
            .Open()
            .map_err(|e| HudError::Direct2DFailed(e.to_string()))?;
        sink.BeginFigure(pt(24.5, 28.0), D2D1_FIGURE_BEGIN_HOLLOW);
        sink.AddLine(pt(28.5, 32.0));
        sink.AddLine(pt(37.5, 23.0));
        sink.EndFigure(D2D1_FIGURE_END_OPEN);
        sink.Close()
            .map_err(|e| HudError::Direct2DFailed(e.to_string()))?;

        // Compact Queued Arc (Center: (19.0, 19.0), Radius: 5.5 DIP, Start: (19.0, 13.5), End: (14.23686, 21.75))
        let compact_queued_arc_geometry = d2d_factory
            .CreatePathGeometry()
            .map_err(|e| HudError::Direct2DFailed(e.to_string()))?;
        let sink = compact_queued_arc_geometry
            .Open()
            .map_err(|e| HudError::Direct2DFailed(e.to_string()))?;
        sink.BeginFigure(
            pt(COMPACT_BADGE_CENTER_X_DIP, COMPACT_BADGE_CENTER_Y_DIP - 5.5),
            D2D1_FIGURE_BEGIN_HOLLOW,
        );
        sink.AddArc(&D2D1_ARC_SEGMENT {
            point: pt(
                COMPACT_BADGE_CENTER_X_DIP - 4.7631397,
                COMPACT_BADGE_CENTER_Y_DIP + 2.75,
            ),
            size: D2D_SIZE_F {
                width: 5.5,
                height: 5.5,
            },
            rotationAngle: 0.0,
            sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
            arcSize: D2D1_ARC_SIZE_LARGE,
        });
        sink.EndFigure(D2D1_FIGURE_END_OPEN);
        sink.Close()
            .map_err(|e| HudError::Direct2DFailed(e.to_string()))?;

        // Compact Saved Checkmark: Points (Xc - 4.5, Yc), (Xc - 1.5, Yc + 3.0), (Xc + 5.0, Yc - 3.5)
        let compact_saved_checkmark_geometry = d2d_factory
            .CreatePathGeometry()
            .map_err(|e| HudError::Direct2DFailed(e.to_string()))?;
        let sink = compact_saved_checkmark_geometry
            .Open()
            .map_err(|e| HudError::Direct2DFailed(e.to_string()))?;
        sink.BeginFigure(
            pt(COMPACT_BADGE_CENTER_X_DIP - 4.5, COMPACT_BADGE_CENTER_Y_DIP),
            D2D1_FIGURE_BEGIN_HOLLOW,
        );
        sink.AddLine(pt(
            COMPACT_BADGE_CENTER_X_DIP - 1.5,
            COMPACT_BADGE_CENTER_Y_DIP + 3.0,
        ));
        sink.AddLine(pt(
            COMPACT_BADGE_CENTER_X_DIP + 5.0,
            COMPACT_BADGE_CENTER_Y_DIP - 3.5,
        ));
        sink.EndFigure(D2D1_FIGURE_END_OPEN);
        sink.Close()
            .map_err(|e| HudError::Direct2DFailed(e.to_string()))?;

        // 9. Pre-allocated DirectComposition Animations
        let entrance_opacity_anim = dcomp_device
            .CreateAnimation()
            .map_err(|e| HudError::CompositionFailed(e.to_string()))?;
        let entrance_offset_anim = dcomp_device
            .CreateAnimation()
            .map_err(|e| HudError::CompositionFailed(e.to_string()))?;
        let exit_opacity_anim = dcomp_device
            .CreateAnimation()
            .map_err(|e| HudError::CompositionFailed(e.to_string()))?;

        Ok(Self {
            queued_bg_brush,
            saved_bg_brush,
            failed_bg_brush,
            compact_queued_bg_brush,
            compact_saved_bg_brush,
            compact_failed_bg_brush,
            queued_border_brush,
            saved_border_brush,
            failed_border_brush,
            queued_badge_fill_brush,
            saved_badge_fill_brush,
            failed_badge_fill_brush,
            queued_badge_border_brush,
            queued_track_brush,
            queued_arc_brush,
            queued_dot_brush,
            saved_ink_brush,
            failed_ink_brush,
            queued_title_brush,
            saved_title_brush,
            failed_title_brush,
            queued_subtitle_brush,
            saved_subtitle_brush,
            failed_subtitle_brush,
            round_stroke_style,
            queued_arc_geometry,
            saved_checkmark_geometry,
            compact_queued_arc_geometry,
            compact_saved_checkmark_geometry,
            entrance_opacity_anim,
            entrance_offset_anim,
            exit_opacity_anim,
        })
    }
}

/// A handle to the native Windows HUD instance.
///
/// Automatically implements `Send` and `Sync` via safe Rust composition.
pub struct NativeHud {
    sender: SyncSender<HudCommand>,
    hwnd: isize,
    capabilities: HudCapabilities,
    thread_handle: Mutex<Option<JoinHandle<()>>>,
    cancelled: Arc<AtomicBool>,
    is_healthy: Arc<AtomicBool>,
}

impl NativeHud {
    /// Initializes and prewarms the Native HUD with AttachedHidden lifecycle (hidden during idle to protect game MPO/Independent Flip).
    pub fn try_new(config: HudConfig) -> Result<Self, HudError> {
        Self::try_new_internal(config, DiagnosticLifecyclePolicy::AttachedHidden)
    }

    /// Initializes and prewarms the Native HUD with a specified diagnostic lifecycle policy.
    #[cfg(feature = "diagnostic-lifecycle")]
    pub fn try_new_diagnostic(
        config: HudConfig,
        policy: DiagnosticLifecyclePolicy,
    ) -> Result<Self, HudError> {
        Self::try_new_internal(config, policy)
    }

    fn try_new_internal(
        config: HudConfig,
        policy: DiagnosticLifecyclePolicy,
    ) -> Result<Self, HudError> {
        let (cmd_tx, cmd_rx) = sync_channel(HUD_COMMAND_QUEUE_BOUND);
        let (init_tx, init_rx) = sync_channel(1);

        let cancelled = Arc::new(AtomicBool::new(false));
        let cancelled_thread = Arc::clone(&cancelled);

        let is_healthy = Arc::new(AtomicBool::new(true));
        let is_healthy_thread = Arc::clone(&is_healthy);

        let thread_handle = std::thread::Builder::new()
            .name("silk-native-hud".to_string())
            .spawn(move || {
                let com_guard = match ComApartmentGuard::new() {
                    Ok(g) => g,
                    Err(err) => {
                        let _ = init_tx.send(Err(err));
                        is_healthy_thread.store(false, Ordering::Release);
                        return;
                    }
                };

                let _dpi_guard = DpiAwarenessGuard::new();

                let init_result = unsafe { HudThreadContext::init(config, policy, cmd_rx) };
                match init_result {
                    Ok((mut context, caps, hwnd)) => {
                        if cancelled_thread.load(Ordering::Acquire) {
                            unsafe {
                                context.teardown();
                            }
                            drop(context);
                            drop(com_guard);
                            is_healthy_thread.store(false, Ordering::Release);
                            return;
                        }

                        let _ = init_tx.send(Ok((caps, hwnd.0 as isize)));
                        unsafe { context.run_message_loop(&is_healthy_thread) };
                        is_healthy_thread.store(false, Ordering::Release);
                        unsafe { context.teardown() };
                        drop(context);
                        drop(com_guard);
                    }
                    Err(err) => {
                        let _ = init_tx.send(Err(err));
                        drop(com_guard);
                        is_healthy_thread.store(false, Ordering::Release);
                    }
                }
            })
            .map_err(|e| HudError::ThreadSpawnFailed(e.to_string()))?;

        match init_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok((capabilities, hwnd))) => Ok(Self {
                sender: cmd_tx,
                hwnd,
                capabilities,
                thread_handle: Mutex::new(Some(thread_handle)),
                cancelled,
                is_healthy,
            }),
            Ok(Err(err)) => {
                cancelled.store(true, Ordering::Release);
                is_healthy.store(false, Ordering::Release);
                let _ = thread_handle.join();
                Err(err)
            }
            Err(_) => {
                cancelled.store(true, Ordering::Release);
                is_healthy.store(false, Ordering::Release);
                Err(HudError::InitializationTimeout)
            }
        }
    }

    /// Returns whether the HUD thread is healthy and accepting commands.
    pub fn is_healthy(&self) -> bool {
        self.is_healthy.load(Ordering::Acquire)
    }

    /// Submits a capture confirmation cue to the HUD without blocking callers.
    pub fn try_show(&self, cue: HudCue) -> Result<(), HudError> {
        if !self.is_healthy() {
            return Err(HudError::ShuttingDown);
        }

        self.sender
            .try_send(HudCommand::Show(cue))
            .map_err(|e| match e {
                std::sync::mpsc::TrySendError::Full(_) => HudError::QueueFull,
                std::sync::mpsc::TrySendError::Disconnected(_) => {
                    self.is_healthy.store(false, Ordering::Release);
                    HudError::ChannelDisconnected
                }
            })?;

        // Post wake message to the HWND pump
        let post_res = unsafe {
            PostMessageW(
                Some(HWND(self.hwnd as *mut _)),
                WM_HUD_COMMAND,
                WPARAM(0),
                LPARAM(0),
            )
        };
        if post_res.is_err() {
            self.is_healthy.store(false, Ordering::Release);
            return Err(HudError::ChannelDisconnected);
        }
        Ok(())
    }

    /// Returns the system capabilities detected during initialization.
    pub const fn capabilities(&self) -> &HudCapabilities {
        &self.capabilities
    }

    /// Cleanly and idempotently stops and joins the HUD message thread.
    pub fn shutdown(&self) {
        self.is_healthy.store(false, Ordering::Release);
        self.cancelled.store(true, Ordering::Release);

        let _ = self.sender.try_send(HudCommand::Shutdown);

        if self.hwnd != 0 {
            unsafe {
                let _ = PostMessageW(
                    Some(HWND(self.hwnd as *mut _)),
                    WM_CLOSE,
                    WPARAM(0),
                    LPARAM(0),
                );
            }
        }

        if let Ok(mut lock) = self.thread_handle.lock() {
            if let Some(handle) = lock.take() {
                let _ = handle.join();
            }
        }
    }
}

impl Drop for NativeHud {
    fn drop(&mut self) {
        self.shutdown();
    }
}

struct HudThreadContext {
    hwnd: HWND,
    cmd_rx: Receiver<HudCommand>,
    _dpi: u32,
    reduced_motion: bool,
    mode: HudMode,
    anchor: HudAnchor,
    policy: DiagnosticLifecyclePolicy,
    // DirectComposition
    dcomp_device: IDCompositionDevice,
    dcomp_target: IDCompositionTarget,
    dcomp_visual: IDCompositionVisual,
    dcomp_effect_group: IDCompositionEffectGroup,
    // Direct2D & DXGI
    swapchain: IDXGISwapChain1,
    d2d_context: ID2D1DeviceContext,
    _dwrite_factory: IDWriteFactory,
    title_format: IDWriteTextFormat,
    subtitle_format: IDWriteTextFormat,
    compact_title_format: IDWriteTextFormat,
    resources: PreallocatedResources,
    // State
    _current_generation: u64,
    is_visible: bool,
    root_attached: bool,
    window_shown: bool,
}

impl HudThreadContext {
    unsafe fn init(
        config: HudConfig,
        policy: DiagnosticLifecyclePolicy,
        cmd_rx: Receiver<HudCommand>,
    ) -> Result<(Self, HudCapabilities, HWND), HudError> {
        let instance = GetModuleHandleW(None)
            .map_err(|e| HudError::InitializationFailed(format!("GetModuleHandleW failed: {e}")))?;

        let wnd_class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: instance.into(),
            hIcon: windows::Win32::UI::WindowsAndMessaging::HICON::default(),
            hCursor: windows::Win32::UI::WindowsAndMessaging::HCURSOR::default(),
            hbrBackground: windows::Win32::Graphics::Gdi::HBRUSH::default(),
            lpszMenuName: PCWSTR::null(),
            lpszClassName: WINDOW_CLASS_NAME,
            hIconSm: windows::Win32::UI::WindowsAndMessaging::HICON::default(),
        };

        let _ = RegisterClassExW(&wnd_class);

        // Extended styles: topmost, noactivate, noredirectionbitmap, toolwindow, transparent
        let ex_style = WS_EX_TOPMOST
            | WS_EX_NOACTIVATE
            | WS_EX_NOREDIRECTIONBITMAP
            | WS_EX_TOOLWINDOW
            | WS_EX_TRANSPARENT;
        let style = WS_POPUP;

        let hwnd = CreateWindowExW(
            ex_style,
            WINDOW_CLASS_NAME,
            w!("Silk Native HUD"),
            style,
            0,
            0,
            320,
            56,
            Some(HWND::default()),
            Some(windows::Win32::UI::WindowsAndMessaging::HMENU::default()),
            Some(instance.into()),
            None,
        )
        .map_err(|e| HudError::WindowCreationFailed(e.to_string()))?;

        let dpi = GetDpiForWindow(hwnd);
        let dpi = if dpi == 0 { 96 } else { dpi };

        // Position window per anchor and monitor work area
        let hmonitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTOPRIMARY);
        let mut minfo = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            rcMonitor: RECT::default(),
            rcWork: RECT::default(),
            dwFlags: 0,
        };
        let gmi_res = GetMonitorInfoW(hmonitor, &mut minfo);
        if !gmi_res.as_bool()
            || (minfo.rcWork.right <= minfo.rcWork.left)
            || (minfo.rcWork.bottom <= minfo.rcWork.top)
        {
            let _ = DestroyWindow(hwnd);
            return Err(HudError::WindowCreationFailed(
                "Invalid or unavailable monitor work area".to_string(),
            ));
        }

        let work_area = WorkArea::new(
            minfo.rcWork.left,
            minfo.rcWork.top,
            (minfo.rcWork.right - minfo.rcWork.left) as u32,
            (minfo.rcWork.bottom - minfo.rcWork.top) as u32,
        );
        let (phys_w, phys_h) = physical_size_for_mode(config.mode, dpi);
        let (phys_x, phys_y) =
            calculate_anchor_position_for_mode(config.anchor, config.mode, work_area, dpi);

        let swp_res = SetWindowPos(
            hwnd,
            Some(HWND::default()),
            phys_x,
            phys_y,
            phys_w as i32,
            phys_h as i32,
            SWP_NOACTIVATE | SWP_NOZORDER | SWP_HIDEWINDOW,
        );
        if swp_res.is_err() {
            let _ = DestroyWindow(hwnd);
            return Err(HudError::WindowCreationFailed(
                "SetWindowPos geometry initialization failed".to_string(),
            ));
        }

        // Truthful capture exclusion: only reported if requested and succeeded
        let mut capture_exclusion_applied = false;
        let mut capture_exclusion_supported = false;
        if config.exclude_from_capture {
            let result = SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE);
            if result.is_ok() {
                capture_exclusion_applied = true;
                capture_exclusion_supported = true;
            }
        }

        // Reduced motion check
        let mut anim_enabled = BOOL(1);
        let _ = SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some(&mut anim_enabled as *mut BOOL as *mut _),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        );
        let reduced_motion = !anim_enabled.as_bool();

        // 1. D3D11 hardware device with BGRA support
        let feature_levels = [D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0];
        let mut d3d_device: Option<ID3D11Device> = None;
        let mut d3d_context: Option<ID3D11DeviceContext> = None;
        let mut obtained_feature_level = D3D_FEATURE_LEVEL::default();

        let d3d_res = D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            windows::Win32::Foundation::HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            Some(&feature_levels),
            D3D11_SDK_VERSION,
            Some(&mut d3d_device),
            Some(&mut obtained_feature_level),
            Some(&mut d3d_context),
        );
        if let Err(e) = d3d_res {
            let _ = DestroyWindow(hwnd);
            return Err(HudError::DeviceCreationFailed(format!(
                "D3D11CreateDevice failed: {e}"
            )));
        }

        let d3d_device = match d3d_device {
            Some(dev) => dev,
            None => {
                let _ = DestroyWindow(hwnd);
                return Err(HudError::DeviceCreationFailed(
                    "D3D11 device missing".to_string(),
                ));
            }
        };

        // 2. DXGI swapchain for composition
        let dxgi_device: IDXGIDevice = match d3d_device.cast() {
            Ok(d) => d,
            Err(e) => {
                let _ = DestroyWindow(hwnd);
                return Err(HudError::DeviceCreationFailed(e.to_string()));
            }
        };
        let dxgi_adapter: IDXGIAdapter = match dxgi_device.GetAdapter() {
            Ok(a) => a,
            Err(e) => {
                let _ = DestroyWindow(hwnd);
                return Err(HudError::DeviceCreationFailed(e.to_string()));
            }
        };
        let dxgi_factory: IDXGIFactory2 = match dxgi_adapter.GetParent() {
            Ok(f) => f,
            Err(e) => {
                let _ = DestroyWindow(hwnd);
                return Err(HudError::DeviceCreationFailed(e.to_string()));
            }
        };

        let swapchain_desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: phys_w,
            Height: phys_h,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            Stereo: BOOL(0),
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            Scaling: DXGI_SCALING_STRETCH,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
            AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
            Flags: 0,
        };

        let swapchain =
            match dxgi_factory.CreateSwapChainForComposition(&d3d_device, &swapchain_desc, None) {
                Ok(sc) => sc,
                Err(e) => {
                    let _ = DestroyWindow(hwnd);
                    return Err(HudError::DeviceCreationFailed(e.to_string()));
                }
            };

        // 3. Direct2D Device Context & Target Bitmap
        let d2d_factory1: ID2D1Factory1 =
            match D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None) {
                Ok(f) => f,
                Err(e) => {
                    let _ = DestroyWindow(hwnd);
                    return Err(HudError::Direct2DFailed(e.to_string()));
                }
            };
        let d2d_factory: ID2D1Factory = match d2d_factory1.cast() {
            Ok(f) => f,
            Err(e) => {
                let _ = DestroyWindow(hwnd);
                return Err(HudError::Direct2DFailed(e.to_string()));
            }
        };
        let d2d_device: ID2D1Device = match d2d_factory1.CreateDevice(&dxgi_device) {
            Ok(dev) => dev,
            Err(e) => {
                let _ = DestroyWindow(hwnd);
                return Err(HudError::Direct2DFailed(e.to_string()));
            }
        };
        let d2d_context = match d2d_device.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE) {
            Ok(ctx) => ctx,
            Err(e) => {
                let _ = DestroyWindow(hwnd);
                return Err(HudError::Direct2DFailed(e.to_string()));
            }
        };

        let dxgi_surface: IDXGISurface = match swapchain.GetBuffer(0) {
            Ok(s) => s,
            Err(e) => {
                let _ = DestroyWindow(hwnd);
                return Err(HudError::Direct2DFailed(e.to_string()));
            }
        };

        let bitmap_props = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: dpi as f32,
            dpiY: dpi as f32,
            bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
            colorContext: ManuallyDrop::new(None),
        };

        let target_bitmap =
            match d2d_context.CreateBitmapFromDxgiSurface(&dxgi_surface, Some(&bitmap_props)) {
                Ok(b) => b,
                Err(e) => {
                    let _ = DestroyWindow(hwnd);
                    return Err(HudError::Direct2DFailed(e.to_string()));
                }
            };

        d2d_context.SetTarget(&target_bitmap);
        d2d_context.SetDpi(dpi as f32, dpi as f32);
        d2d_context.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);

        // 4. DirectComposition visual tree
        let dcomp_device: IDCompositionDevice = match DCompositionCreateDevice(Some(&dxgi_device)) {
            Ok(d) => d,
            Err(e) => {
                d2d_context.SetTarget(None);
                let _ = DestroyWindow(hwnd);
                return Err(HudError::CompositionFailed(e.to_string()));
            }
        };
        let dcomp_target = match dcomp_device.CreateTargetForHwnd(hwnd, true) {
            Ok(t) => t,
            Err(e) => {
                d2d_context.SetTarget(None);
                let _ = DestroyWindow(hwnd);
                return Err(HudError::CompositionFailed(e.to_string()));
            }
        };
        let dcomp_visual = match dcomp_device.CreateVisual() {
            Ok(v) => v,
            Err(e) => {
                d2d_context.SetTarget(None);
                let _ = DestroyWindow(hwnd);
                return Err(HudError::CompositionFailed(e.to_string()));
            }
        };
        let dcomp_effect_group = match dcomp_device.CreateEffectGroup() {
            Ok(eg) => eg,
            Err(e) => {
                d2d_context.SetTarget(None);
                let _ = DestroyWindow(hwnd);
                return Err(HudError::CompositionFailed(e.to_string()));
            }
        };

        if let Err(e) = dcomp_visual.SetContent(&swapchain) {
            d2d_context.SetTarget(None);
            let _ = DestroyWindow(hwnd);
            return Err(HudError::CompositionFailed(e.to_string()));
        }

        // Initialize with opacity 0.0 (idle state)
        let _ = dcomp_effect_group.SetOpacity2(0.0);
        let _ = dcomp_visual.SetOffsetY2(0.0);
        let _ = dcomp_visual.SetEffect(&dcomp_effect_group);

        let mut root_attached = false;
        if matches!(
            policy,
            DiagnosticLifecyclePolicy::AttachedShown | DiagnosticLifecyclePolicy::AttachedHidden
        ) {
            if let Err(e) = dcomp_target.SetRoot(&dcomp_visual) {
                d2d_context.SetTarget(None);
                let _ = DestroyWindow(hwnd);
                return Err(HudError::CompositionFailed(e.to_string()));
            }
            root_attached = true;
        }
        if let Err(e) = dcomp_device.Commit() {
            d2d_context.SetTarget(None);
            let _ = DestroyWindow(hwnd);
            return Err(HudError::CompositionFailed(e.to_string()));
        }

        // Pre-allocate all static Direct2D brushes, geometries, stroke styles, and DComp animations
        let resources = match PreallocatedResources::init(&d2d_context, &d2d_factory, &dcomp_device)
        {
            Ok(r) => r,
            Err(e) => {
                d2d_context.SetTarget(None);
                let _ = DestroyWindow(hwnd);
                return Err(e);
            }
        };

        // 5. DirectWrite Text Formats
        let dwrite_factory: IDWriteFactory = match DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED) {
            Ok(f) => f,
            Err(e) => {
                d2d_context.SetTarget(None);
                let _ = DestroyWindow(hwnd);
                return Err(HudError::DirectWriteFailed(e.to_string()));
            }
        };

        let title_format = dwrite_factory
            .CreateTextFormat(
                w!("Segoe UI Variable Text"),
                None,
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
                DWRITE_FONT_STYLE_NORMAL,
                windows::Win32::Graphics::DirectWrite::DWRITE_FONT_STRETCH_NORMAL,
                TITLE_FONT_SIZE_DIP,
                w!("en-us"),
            )
            .or_else(|_| {
                dwrite_factory.CreateTextFormat(
                    w!("Segoe UI"),
                    None,
                    DWRITE_FONT_WEIGHT_SEMI_BOLD,
                    DWRITE_FONT_STYLE_NORMAL,
                    windows::Win32::Graphics::DirectWrite::DWRITE_FONT_STRETCH_NORMAL,
                    TITLE_FONT_SIZE_DIP,
                    w!("en-us"),
                )
            })
            .map_err(|e| {
                d2d_context.SetTarget(None);
                let _ = DestroyWindow(hwnd);
                HudError::DirectWriteFailed(e.to_string())
            })?;

        let _ = title_format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_LEADING);
        let _ = title_format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_NEAR);
        let _ = title_format.SetLineSpacing(
            DWRITE_LINE_SPACING_METHOD_UNIFORM,
            TITLE_LINE_HEIGHT_DIP,
            TITLE_FONT_SIZE_DIP,
        );

        let subtitle_format = dwrite_factory
            .CreateTextFormat(
                w!("Segoe UI Variable Text"),
                None,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_FONT_STYLE_NORMAL,
                windows::Win32::Graphics::DirectWrite::DWRITE_FONT_STRETCH_NORMAL,
                SUBTITLE_FONT_SIZE_DIP,
                w!("en-us"),
            )
            .or_else(|_| {
                dwrite_factory.CreateTextFormat(
                    w!("Segoe UI"),
                    None,
                    DWRITE_FONT_WEIGHT_NORMAL,
                    DWRITE_FONT_STYLE_NORMAL,
                    windows::Win32::Graphics::DirectWrite::DWRITE_FONT_STRETCH_NORMAL,
                    SUBTITLE_FONT_SIZE_DIP,
                    w!("en-us"),
                )
            })
            .map_err(|e| {
                d2d_context.SetTarget(None);
                let _ = DestroyWindow(hwnd);
                HudError::DirectWriteFailed(e.to_string())
            })?;

        let _ = subtitle_format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_LEADING);
        let _ = subtitle_format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_NEAR);
        let _ = subtitle_format.SetLineSpacing(
            DWRITE_LINE_SPACING_METHOD_UNIFORM,
            SUBTITLE_LINE_HEIGHT_DIP,
            SUBTITLE_FONT_SIZE_DIP,
        );

        let trimming = DWRITE_TRIMMING {
            granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
            delimiter: 0,
            delimiterCount: 0,
        };
        let ellipsis_sign: Option<IDWriteInlineObject> = dwrite_factory
            .CreateEllipsisTrimmingSign(&subtitle_format)
            .ok();
        if let Some(ref sign) = ellipsis_sign {
            let _ = subtitle_format.SetTrimming(&trimming, sign);
        }

        let compact_title_format = dwrite_factory
            .CreateTextFormat(
                w!("Segoe UI Variable Text"),
                None,
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
                DWRITE_FONT_STYLE_NORMAL,
                windows::Win32::Graphics::DirectWrite::DWRITE_FONT_STRETCH_NORMAL,
                COMPACT_TITLE_FONT_SIZE_DIP,
                w!("en-us"),
            )
            .or_else(|_| {
                dwrite_factory.CreateTextFormat(
                    w!("Segoe UI"),
                    None,
                    DWRITE_FONT_WEIGHT_SEMI_BOLD,
                    DWRITE_FONT_STYLE_NORMAL,
                    windows::Win32::Graphics::DirectWrite::DWRITE_FONT_STRETCH_NORMAL,
                    COMPACT_TITLE_FONT_SIZE_DIP,
                    w!("en-us"),
                )
            })
            .map_err(|e| {
                d2d_context.SetTarget(None);
                let _ = DestroyWindow(hwnd);
                HudError::DirectWriteFailed(e.to_string())
            })?;

        let _ = compact_title_format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_LEADING);
        let _ = compact_title_format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_NEAR);
        let _ = compact_title_format.SetLineSpacing(
            DWRITE_LINE_SPACING_METHOD_UNIFORM,
            COMPACT_TITLE_LINE_SPACING_DIP,
            COMPACT_TITLE_FONT_SIZE_DIP,
        );

        let compact_trimming = DWRITE_TRIMMING {
            granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
            delimiter: 0,
            delimiterCount: 0,
        };
        let compact_ellipsis_sign: Option<IDWriteInlineObject> = dwrite_factory
            .CreateEllipsisTrimmingSign(&compact_title_format)
            .ok();
        if let Some(ref sign) = compact_ellipsis_sign {
            let _ = compact_title_format.SetTrimming(&compact_trimming, sign);
        }

        // Render initial transparent frame to backbuffer
        d2d_context.BeginDraw();
        d2d_context.Clear(Some(&D2D1_COLOR_F {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.0,
        }));
        let _ = d2d_context.EndDraw(None, None);
        let _ = swapchain.Present(0, DXGI_PRESENT(0));

        // Show window if policy requires it to be shown while idle
        let mut window_shown = false;
        if matches!(
            policy,
            DiagnosticLifecyclePolicy::AttachedShown | DiagnosticLifecyclePolicy::DetachedShown
        ) {
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            window_shown = true;
        }

        let capabilities = HudCapabilities {
            is_supported: true,
            capture_exclusion_supported,
            capture_exclusion_applied,
            reduced_motion,
        };

        let context = Self {
            hwnd,
            cmd_rx,
            _dpi: dpi,
            reduced_motion,
            mode: config.mode,
            anchor: config.anchor,
            policy,
            dcomp_device,
            dcomp_target,
            dcomp_visual,
            dcomp_effect_group,
            swapchain,
            d2d_context,
            _dwrite_factory: dwrite_factory,
            title_format,
            subtitle_format,
            compact_title_format,
            resources,
            _current_generation: 0,
            is_visible: false,
            root_attached,
            window_shown,
        };

        Ok((context, capabilities, hwnd))
    }

    unsafe fn teardown(&mut self) {
        let _ = KillTimer(Some(self.hwnd), TIMER_ID_HOLD);
        let _ = KillTimer(Some(self.hwnd), TIMER_ID_EXIT);

        // Reset visual opacity to 0.0 and detach DComp root
        let _ = self.dcomp_effect_group.SetOpacity2(0.0);
        let _ = self.dcomp_visual.SetOffsetY2(0.0);
        let _ = self.dcomp_target.SetRoot(None);
        let _ = self.dcomp_device.Commit();
        self.root_attached = false;

        // Unbind target bitmap from Direct2D device context
        self.d2d_context.SetTarget(None);

        if !self.hwnd.0.is_null() {
            let _ = DestroyWindow(self.hwnd);
            self.hwnd = HWND::default();
            self.window_shown = false;
        }
    }

    unsafe fn run_message_loop(&mut self, is_healthy: &Arc<AtomicBool>) {
        let mut msg = MSG::default();
        loop {
            let res = GetMessageW(&mut msg, Some(HWND::default()), 0, 0).0;
            if res <= 0 {
                if res == -1 {
                    // Win32 GetMessageW error
                    is_healthy.store(false, Ordering::Release);
                }
                break;
            }

            if msg.message == WM_HUD_COMMAND {
                if !self.drain_commands() {
                    break;
                }
            } else if msg.message == WM_TIMER {
                let timer_res = if msg.wParam.0 == TIMER_ID_HOLD {
                    self.on_hold_timer_expired()
                } else if msg.wParam.0 == TIMER_ID_EXIT {
                    self.on_exit_timer_expired()
                } else {
                    Ok(())
                };

                if timer_res.is_err() {
                    let _ = self.recover_to_idle();
                    is_healthy.store(false, Ordering::Release);
                    break;
                }
            } else if msg.message == WM_CLOSE {
                break;
            } else {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }

    unsafe fn recover_to_idle(&mut self) -> Result<(), HudError> {
        let _ = KillTimer(Some(self.hwnd), TIMER_ID_HOLD);
        let _ = KillTimer(Some(self.hwnd), TIMER_ID_EXIT);
        self.is_visible = false;
        let _ = self.dcomp_effect_group.SetOpacity2(0.0);
        let _ = self.dcomp_visual.SetOffsetY2(0.0);

        if self.policy == DiagnosticLifecyclePolicy::DetachedShown && self.root_attached {
            let _ = self.dcomp_target.SetRoot(None);
            self.root_attached = false;
        }

        let commit_res = self.dcomp_device.Commit();

        if self.policy == DiagnosticLifecyclePolicy::AttachedHidden && self.window_shown {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
            self.window_shown = false;
        }

        commit_res.map_err(|e| HudError::CompositionFailed(e.to_string()))
    }

    unsafe fn drain_commands(&mut self) -> bool {
        while let Ok(cmd) = self.cmd_rx.try_recv() {
            match cmd {
                HudCommand::Show(cue) => {
                    if self.handle_show_cue(cue).is_err() {
                        let _ = self.recover_to_idle();
                        return false;
                    }
                }
                HudCommand::Shutdown => {
                    return false;
                }
            }
        }
        true
    }

    unsafe fn handle_show_cue(&mut self, cue: HudCue) -> Result<(), HudError> {
        self._current_generation += 1;
        let _ = KillTimer(Some(self.hwnd), TIMER_ID_HOLD);
        let _ = KillTimer(Some(self.hwnd), TIMER_ID_EXIT);

        // AttachedHidden: on first cue call ShowWindow(SW_SHOWNOACTIVATE) on worker before cue presentation
        if !self.window_shown {
            let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
            self.window_shown = true;
        }

        // Render cue into the DXGI swapchain backbuffer (Zero heap allocations)
        self.render_cue(&cue)?;
        self.swapchain
            .Present(0, DXGI_PRESENT(0))
            .ok()
            .map_err(|e| HudError::DeviceCreationFailed(e.to_string()))?;

        let hold_duration = cue.hold_duration_ms() as u32;

        if !self.is_visible {
            // DetachedShown: on first cue attach existing visual before/in the same transaction as entrance commit
            if !self.root_attached {
                self.dcomp_target
                    .SetRoot(&self.dcomp_visual)
                    .map_err(|e| HudError::CompositionFailed(e.to_string()))?;
                self.root_attached = true;
            }

            // Full entrance transition
            self.start_entrance_animation()?;
            self.is_visible = true;
            let entrance_ms = if self.reduced_motion {
                REDUCED_MOTION_FADE_DURATION_MS as u32
            } else {
                ENTRANCE_DURATION_MS as u32
            };
            let timer_res = SetTimer(
                Some(self.hwnd),
                TIMER_ID_HOLD,
                entrance_ms + hold_duration,
                None,
            );
            if timer_res == 0 {
                return self.recover_to_idle();
            }
        } else {
            // Already visible: updated in place via flip swapchain
            self.dcomp_effect_group
                .SetOpacity2(1.0)
                .map_err(|e| HudError::CompositionFailed(e.to_string()))?;
            self.dcomp_visual
                .SetOffsetY2(0.0)
                .map_err(|e| HudError::CompositionFailed(e.to_string()))?;
            self.dcomp_device
                .Commit()
                .map_err(|e| HudError::CompositionFailed(e.to_string()))?;

            let timer_res = SetTimer(Some(self.hwnd), TIMER_ID_HOLD, hold_duration, None);
            if timer_res == 0 {
                return self.recover_to_idle();
            }
        }
        Ok(())
    }

    unsafe fn start_entrance_animation(&mut self) -> Result<(), HudError> {
        if self.reduced_motion {
            self.dcomp_effect_group
                .SetOpacity2(1.0)
                .map_err(|e| HudError::CompositionFailed(e.to_string()))?;
            self.dcomp_visual
                .SetOffsetY2(0.0)
                .map_err(|e| HudError::CompositionFailed(e.to_string()))?;
            self.dcomp_device
                .Commit()
                .map_err(|e| HudError::CompositionFailed(e.to_string()))?;
            return Ok(());
        }

        let t = ENTRANCE_DURATION_MS as f64 / 1000.0;
        let inv_t = (1.0 / t) as f32;
        let inv_t2 = inv_t * inv_t;
        let inv_t3 = inv_t2 * inv_t;

        // Reuse pre-allocated entrance opacity animation
        self.resources
            .entrance_opacity_anim
            .Reset()
            .map_err(|e| HudError::CompositionFailed(e.to_string()))?;
        self.resources
            .entrance_opacity_anim
            .AddCubic(0.0, 0.0, 3.0 * inv_t, -3.0 * inv_t2, inv_t3)
            .map_err(|e| HudError::CompositionFailed(e.to_string()))?;
        self.resources
            .entrance_opacity_anim
            .End(t, 1.0)
            .map_err(|e| HudError::CompositionFailed(e.to_string()))?;
        self.dcomp_effect_group
            .SetOpacity(&self.resources.entrance_opacity_anim)
            .map_err(|e| HudError::CompositionFailed(e.to_string()))?;

        let slide_offset = self.anchor.slide_offset_dip();
        if slide_offset != 0.0 {
            // Reuse pre-allocated entrance offset animation
            self.resources
                .entrance_offset_anim
                .Reset()
                .map_err(|e| HudError::CompositionFailed(e.to_string()))?;
            self.resources
                .entrance_offset_anim
                .AddCubic(
                    0.0,
                    slide_offset,
                    -slide_offset * 3.0 * inv_t,
                    slide_offset * 3.0 * inv_t2,
                    -slide_offset * inv_t3,
                )
                .map_err(|e| HudError::CompositionFailed(e.to_string()))?;
            self.resources
                .entrance_offset_anim
                .End(t, 0.0)
                .map_err(|e| HudError::CompositionFailed(e.to_string()))?;
            self.dcomp_visual
                .SetOffsetY(&self.resources.entrance_offset_anim)
                .map_err(|e| HudError::CompositionFailed(e.to_string()))?;
        } else {
            self.dcomp_visual
                .SetOffsetY2(0.0)
                .map_err(|e| HudError::CompositionFailed(e.to_string()))?;
        }

        self.dcomp_device
            .Commit()
            .map_err(|e| HudError::CompositionFailed(e.to_string()))
    }

    unsafe fn on_hold_timer_expired(&mut self) -> Result<(), HudError> {
        let _ = KillTimer(Some(self.hwnd), TIMER_ID_HOLD);
        self.start_exit_animation()?;
        let exit_duration = if self.reduced_motion {
            REDUCED_MOTION_FADE_DURATION_MS as u32
        } else {
            EXIT_DURATION_MS as u32
        };
        let timer_res = SetTimer(Some(self.hwnd), TIMER_ID_EXIT, exit_duration, None);
        if timer_res == 0 {
            return self.recover_to_idle();
        }
        Ok(())
    }

    unsafe fn start_exit_animation(&mut self) -> Result<(), HudError> {
        let exit_duration_ms = if self.reduced_motion {
            REDUCED_MOTION_FADE_DURATION_MS
        } else {
            EXIT_DURATION_MS
        };

        let t = exit_duration_ms as f64 / 1000.0;
        let inv_t = (1.0 / t) as f32;
        let inv_t2 = inv_t * inv_t;

        // Reuse pre-allocated exit opacity animation
        self.resources
            .exit_opacity_anim
            .Reset()
            .map_err(|e| HudError::CompositionFailed(e.to_string()))?;
        self.resources
            .exit_opacity_anim
            .AddCubic(0.0, 1.0, 0.0, -inv_t2, 0.0)
            .map_err(|e| HudError::CompositionFailed(e.to_string()))?;
        self.resources
            .exit_opacity_anim
            .End(t, 0.0)
            .map_err(|e| HudError::CompositionFailed(e.to_string()))?;
        self.dcomp_effect_group
            .SetOpacity(&self.resources.exit_opacity_anim)
            .map_err(|e| HudError::CompositionFailed(e.to_string()))?;

        self.dcomp_device
            .Commit()
            .map_err(|e| HudError::CompositionFailed(e.to_string()))
    }

    unsafe fn on_exit_timer_expired(&mut self) -> Result<(), HudError> {
        let _ = KillTimer(Some(self.hwnd), TIMER_ID_EXIT);
        self.is_visible = false;
        self.dcomp_effect_group
            .SetOpacity2(0.0)
            .map_err(|e| HudError::CompositionFailed(e.to_string()))?;
        self.dcomp_visual
            .SetOffsetY2(0.0)
            .map_err(|e| HudError::CompositionFailed(e.to_string()))?;

        // DetachedShown: detach root + Commit
        if self.policy == DiagnosticLifecyclePolicy::DetachedShown && self.root_attached {
            self.dcomp_target
                .SetRoot(None)
                .map_err(|e| HudError::CompositionFailed(e.to_string()))?;
            self.root_attached = false;
        }

        self.dcomp_device
            .Commit()
            .map_err(|e| HudError::CompositionFailed(e.to_string()))?;

        // AttachedHidden: reset opacity/offset + Commit, then ShowWindow(SW_HIDE)
        if self.policy == DiagnosticLifecyclePolicy::AttachedHidden && self.window_shown {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
            self.window_shown = false;
        }

        Ok(())
    }

    /// Formats the cue's title for compact mode (max 20 chars) into `title_buf` without heap allocations.
    fn write_compact_cue_title<'a>(
        &self,
        cue: &HudCue,
        title_buf: &'a mut [u16; TITLE_UTF16_BUF_SIZE],
    ) -> &'a [u16] {
        let raw_title = cue.title();
        let len = encode_truncated_utf16(raw_title, COMPACT_TITLE_MAX_CHARS, title_buf);
        &title_buf[..len]
    }

    /// Formats the cue's title into `title_buf` without heap allocations.
    fn write_cue_title<'a>(
        &self,
        cue: &HudCue,
        title_buf: &'a mut [u16; TITLE_UTF16_BUF_SIZE],
    ) -> &'a [u16] {
        let raw_title = cue.title();
        let len = encode_truncated_utf16(raw_title, TITLE_MAX_CHARS, title_buf);
        &title_buf[..len]
    }

    /// Formats the cue's subtitle into `subtitle_buf` without heap allocations.
    fn write_cue_subtitle<'a>(
        &self,
        cue: &HudCue,
        subtitle_buf: &'a mut [u16; SUBTITLE_UTF16_BUF_SIZE],
    ) -> &'a [u16] {
        let len = match cue {
            HudCue::Queued {
                subtitle: Some(s), ..
            } => encode_truncated_utf16(s.as_str(), SUBTITLE_MAX_CHARS, subtitle_buf),
            HudCue::Queued { subtitle: None, .. } => {
                encode_truncated_utf16("Writing clip to disk", SUBTITLE_MAX_CHARS, subtitle_buf)
            }
            HudCue::Saved {
                custom_subtitle: Some(s),
                ..
            } => encode_truncated_utf16(s.as_str(), SUBTITLE_MAX_CHARS, subtitle_buf),
            HudCue::Saved {
                duration_ms: Some(dur),
                bytes: Some(b),
                ..
            } => format_saved_subtitle_into_utf16(*dur, *b, subtitle_buf),
            HudCue::Saved {
                duration_ms: Some(dur),
                bytes: None,
                ..
            } => format_duration_into_utf16(*dur, subtitle_buf),
            HudCue::Saved {
                duration_ms: None,
                bytes: Some(b),
                ..
            } => format_file_size_into_utf16(*b, subtitle_buf),
            HudCue::Saved {
                duration_ms: None,
                bytes: None,
                custom_subtitle: None,
            } => encode_truncated_utf16("Clip saved", SUBTITLE_MAX_CHARS, subtitle_buf),
            HudCue::Failed { reason } | HudCue::Rejected { reason } => {
                encode_truncated_utf16(reason.as_str(), SUBTITLE_MAX_CHARS, subtitle_buf)
            }
        };
        &subtitle_buf[..len]
    }

    unsafe fn render_full_cue(&mut self, cue: &HudCue) {
        let state_kind = cue.state_kind();

        // 1. Container Background Gradient & Border (Using pre-allocated resources)
        let (bg_brush, border_brush) = match state_kind {
            HudStateKind::Queued => (
                &self.resources.queued_bg_brush,
                &self.resources.queued_border_brush,
            ),
            HudStateKind::Saved => (
                &self.resources.saved_bg_brush,
                &self.resources.saved_border_brush,
            ),
            HudStateKind::Failed => (
                &self.resources.failed_bg_brush,
                &self.resources.failed_border_brush,
            ),
        };

        let hud_rect = D2D1_ROUNDED_RECT {
            rect: D2D_RECT_F {
                left: 0.5,
                top: 0.5,
                right: CANVAS_WIDTH_DIP - 0.5,
                bottom: CANVAS_HEIGHT_DIP - 0.5,
            },
            radiusX: CONTAINER_RADIUS_DIP,
            radiusY: CONTAINER_RADIUS_DIP,
        };
        self.d2d_context.FillRoundedRectangle(&hud_rect, bg_brush);
        self.d2d_context.DrawRoundedRectangle(
            &hud_rect,
            border_brush,
            CONTAINER_BORDER_STROKE_DIP,
            None,
        );

        // 2. Badge Container (Using pre-allocated resources)
        let badge_rect = D2D1_ROUNDED_RECT {
            rect: D2D_RECT_F {
                left: BADGE_LEFT_DIP,
                top: BADGE_TOP_DIP,
                right: BADGE_LEFT_DIP + BADGE_WIDTH_DIP,
                bottom: BADGE_TOP_DIP + BADGE_HEIGHT_DIP,
            },
            radiusX: BADGE_RADIUS_DIP,
            radiusY: BADGE_RADIUS_DIP,
        };

        match state_kind {
            HudStateKind::Queued => {
                self.d2d_context
                    .FillRoundedRectangle(&badge_rect, &self.resources.queued_badge_fill_brush);
                self.d2d_context.DrawRoundedRectangle(
                    &badge_rect,
                    &self.resources.queued_badge_border_brush,
                    1.0,
                    None,
                );
            }
            HudStateKind::Saved => {
                self.d2d_context
                    .FillRoundedRectangle(&badge_rect, &self.resources.saved_badge_fill_brush);
            }
            HudStateKind::Failed => {
                self.d2d_context
                    .FillRoundedRectangle(&badge_rect, &self.resources.failed_badge_fill_brush);
            }
        }

        // 3. Badge Icon Geometries (Using pre-allocated geometries and brushes)
        match state_kind {
            HudStateKind::Queued => {
                let track_ellipse = D2D1_ELLIPSE {
                    point: pt(BADGE_CENTER_X_DIP, BADGE_CENTER_Y_DIP),
                    radiusX: 8.0,
                    radiusY: 8.0,
                };
                self.d2d_context.DrawEllipse(
                    &track_ellipse,
                    &self.resources.queued_track_brush,
                    1.5,
                    None,
                );

                self.d2d_context.DrawGeometry(
                    &self.resources.queued_arc_geometry,
                    &self.resources.queued_arc_brush,
                    2.0,
                    Some(&self.resources.round_stroke_style),
                );

                let dot_ellipse = D2D1_ELLIPSE {
                    point: pt(BADGE_CENTER_X_DIP, BADGE_CENTER_Y_DIP),
                    radiusX: 2.0,
                    radiusY: 2.0,
                };
                self.d2d_context
                    .FillEllipse(&dot_ellipse, &self.resources.queued_dot_brush);
            }
            HudStateKind::Saved => {
                self.d2d_context.DrawGeometry(
                    &self.resources.saved_checkmark_geometry,
                    &self.resources.saved_ink_brush,
                    2.5,
                    Some(&self.resources.round_stroke_style),
                );
            }
            HudStateKind::Failed => {
                self.d2d_context.DrawLine(
                    pt(31.0, 21.5),
                    pt(31.0, 29.5),
                    &self.resources.failed_ink_brush,
                    2.5,
                    Some(&self.resources.round_stroke_style),
                );

                let dot_ellipse = D2D1_ELLIPSE {
                    point: pt(31.0, 34.5),
                    radiusX: 1.35,
                    radiusY: 1.35,
                };
                self.d2d_context
                    .FillEllipse(&dot_ellipse, &self.resources.failed_ink_brush);
            }
        }

        // 4. Title and Subtitle Text (Zero allocation from stack UTF-16 buffers)
        let (title_brush, subtitle_brush) = match state_kind {
            HudStateKind::Queued => (
                &self.resources.queued_title_brush,
                &self.resources.queued_subtitle_brush,
            ),
            HudStateKind::Saved => (
                &self.resources.saved_title_brush,
                &self.resources.saved_subtitle_brush,
            ),
            HudStateKind::Failed => (
                &self.resources.failed_title_brush,
                &self.resources.failed_subtitle_brush,
            ),
        };

        let mut title_buf = [0u16; TITLE_UTF16_BUF_SIZE];
        let title_utf16 = self.write_cue_title(cue, &mut title_buf);
        let title_rect = D2D_RECT_F {
            left: TEXT_BLOCK_LEFT_DIP,
            top: 11.0,
            right: TEXT_BLOCK_RIGHT_DIP,
            bottom: 28.0,
        };

        self.d2d_context.DrawText(
            title_utf16,
            &self.title_format,
            &title_rect,
            title_brush,
            D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT | D2D1_DRAW_TEXT_OPTIONS_CLIP,
            DWRITE_MEASURING_MODE_NATURAL,
        );

        let mut subtitle_buf = [0u16; SUBTITLE_UTF16_BUF_SIZE];
        let subtitle_utf16 = self.write_cue_subtitle(cue, &mut subtitle_buf);
        let subtitle_rect = D2D_RECT_F {
            left: TEXT_BLOCK_LEFT_DIP,
            top: 29.0,
            right: TEXT_BLOCK_RIGHT_DIP,
            bottom: 44.0,
        };

        self.d2d_context.DrawText(
            subtitle_utf16,
            &self.subtitle_format,
            &subtitle_rect,
            subtitle_brush,
            D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT | D2D1_DRAW_TEXT_OPTIONS_CLIP,
            DWRITE_MEASURING_MODE_NATURAL,
        );
    }

    unsafe fn render_compact_cue(&mut self, cue: &HudCue) {
        let identity = matrix3x2_identity();
        self.d2d_context.SetTransform(&identity);

        let state_kind = cue.state_kind();

        // 1. Compact Container Background Gradient & Border (Section 6.4)
        let (bg_brush, border_brush) = match state_kind {
            HudStateKind::Queued => (
                &self.resources.compact_queued_bg_brush,
                &self.resources.queued_border_brush,
            ),
            HudStateKind::Saved => (
                &self.resources.compact_saved_bg_brush,
                &self.resources.saved_border_brush,
            ),
            HudStateKind::Failed => (
                &self.resources.compact_failed_bg_brush,
                &self.resources.failed_border_brush,
            ),
        };

        let pill_rect = D2D1_ROUNDED_RECT {
            rect: D2D_RECT_F {
                left: 0.5,
                top: 0.5,
                right: COMPACT_WIDTH_DIP - 0.5,
                bottom: COMPACT_HEIGHT_DIP - 0.5,
            },
            radiusX: COMPACT_RADIUS_DIP,
            radiusY: COMPACT_RADIUS_DIP,
        };
        self.d2d_context.FillRoundedRectangle(&pill_rect, bg_brush);
        self.d2d_context.DrawRoundedRectangle(
            &pill_rect,
            border_brush,
            COMPACT_BORDER_STROKE_DIP,
            None,
        );

        // 2. Compact Badge Container (24x24 DIP, radius 6.0 DIP, margin [7, 7, 31, 31])
        let badge_rect = D2D1_ROUNDED_RECT {
            rect: D2D_RECT_F {
                left: COMPACT_BADGE_MARGIN_DIP,
                top: COMPACT_BADGE_MARGIN_DIP,
                right: COMPACT_BADGE_MARGIN_DIP + COMPACT_BADGE_SIZE_DIP,
                bottom: COMPACT_BADGE_MARGIN_DIP + COMPACT_BADGE_SIZE_DIP,
            },
            radiusX: COMPACT_BADGE_RADIUS_DIP,
            radiusY: COMPACT_BADGE_RADIUS_DIP,
        };

        match state_kind {
            HudStateKind::Queued => {
                self.d2d_context
                    .FillRoundedRectangle(&badge_rect, &self.resources.queued_badge_fill_brush);
                self.d2d_context.DrawRoundedRectangle(
                    &badge_rect,
                    &self.resources.queued_badge_border_brush,
                    1.0,
                    None,
                );
            }
            HudStateKind::Saved => {
                self.d2d_context
                    .FillRoundedRectangle(&badge_rect, &self.resources.saved_badge_fill_brush);
            }
            HudStateKind::Failed => {
                self.d2d_context
                    .FillRoundedRectangle(&badge_rect, &self.resources.failed_badge_fill_brush);
            }
        }

        // 3. Compact Icon Geometries (Local center: (19.0, 19.0))
        match state_kind {
            HudStateKind::Queued => {
                let track_ellipse = D2D1_ELLIPSE {
                    point: pt(COMPACT_BADGE_CENTER_X_DIP, COMPACT_BADGE_CENTER_Y_DIP),
                    radiusX: 5.5,
                    radiusY: 5.5,
                };
                self.d2d_context.DrawEllipse(
                    &track_ellipse,
                    &self.resources.queued_track_brush,
                    1.2,
                    None,
                );

                self.d2d_context.DrawGeometry(
                    &self.resources.compact_queued_arc_geometry,
                    &self.resources.queued_arc_brush,
                    1.6,
                    Some(&self.resources.round_stroke_style),
                );

                let dot_ellipse = D2D1_ELLIPSE {
                    point: pt(COMPACT_BADGE_CENTER_X_DIP, COMPACT_BADGE_CENTER_Y_DIP),
                    radiusX: 1.4,
                    radiusY: 1.4,
                };
                self.d2d_context
                    .FillEllipse(&dot_ellipse, &self.resources.queued_dot_brush);
            }
            HudStateKind::Saved => {
                self.d2d_context.DrawGeometry(
                    &self.resources.compact_saved_checkmark_geometry,
                    &self.resources.saved_ink_brush,
                    2.0,
                    Some(&self.resources.round_stroke_style),
                );
            }
            HudStateKind::Failed => {
                self.d2d_context.DrawLine(
                    pt(COMPACT_BADGE_CENTER_X_DIP, COMPACT_BADGE_CENTER_Y_DIP - 4.5),
                    pt(COMPACT_BADGE_CENTER_X_DIP, COMPACT_BADGE_CENTER_Y_DIP + 1.5),
                    &self.resources.failed_ink_brush,
                    2.0,
                    Some(&self.resources.round_stroke_style),
                );

                let dot_ellipse = D2D1_ELLIPSE {
                    point: pt(COMPACT_BADGE_CENTER_X_DIP, COMPACT_BADGE_CENTER_Y_DIP + 4.5),
                    radiusX: 1.0,
                    radiusY: 1.0,
                };
                self.d2d_context
                    .FillEllipse(&dot_ellipse, &self.resources.failed_ink_brush);
            }
        }

        // 4. Compact Title Text Only (Max 20 Unicode scalars, local bounds [38, 11, 168, 27], no subtitle)
        let title_brush = match state_kind {
            HudStateKind::Queued => &self.resources.queued_title_brush,
            HudStateKind::Saved => &self.resources.saved_title_brush,
            HudStateKind::Failed => &self.resources.failed_title_brush,
        };

        let mut title_buf = [0u16; TITLE_UTF16_BUF_SIZE];
        let title_utf16 = self.write_compact_cue_title(cue, &mut title_buf);
        let text_rect = D2D_RECT_F {
            left: COMPACT_TEXT_LEFT_DIP,
            top: COMPACT_TEXT_TOP_DIP,
            right: COMPACT_TEXT_RIGHT_DIP,
            bottom: COMPACT_TEXT_BOTTOM_DIP,
        };

        self.d2d_context.DrawText(
            title_utf16,
            &self.compact_title_format,
            &text_rect,
            title_brush,
            D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT | D2D1_DRAW_TEXT_OPTIONS_CLIP,
            DWRITE_MEASURING_MODE_NATURAL,
        );
    }

    unsafe fn render_cue(&mut self, cue: &HudCue) -> Result<(), HudError> {
        self.d2d_context.BeginDraw();
        self.d2d_context.Clear(Some(&D2D1_COLOR_F {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.0,
        }));

        match self.mode {
            HudMode::Full => self.render_full_cue(cue),
            HudMode::Compact => self.render_compact_cue(cue),
        }

        // Ensure context transform is restored deterministically before EndDraw
        let identity = matrix3x2_identity();
        self.d2d_context.SetTransform(&identity);

        self.d2d_context
            .EndDraw(None, None)
            .map_err(|e| HudError::Direct2DFailed(e.to_string()))
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let _hdc = BeginPaint(hwnd, &mut ps);
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_CLOSE => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
