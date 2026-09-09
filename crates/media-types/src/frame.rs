use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::packet::StreamId;
use crate::time_base::TimeBase;

/// Pixel layout of a video frame surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PixelFormat {
    Bgra8,
    Rgba8,
    Nv12,
    I420,
    Ayuv,
}

/// Chroma subsampling layout for video formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChromaSubsampling {
    Yuv420,
    Yuv444,
}

impl PixelFormat {
    /// Returns the chroma subsampling representation of this pixel format, if applicable.
    pub const fn chroma_subsampling(self) -> Option<ChromaSubsampling> {
        match self {
            Self::Nv12 | Self::I420 => Some(ChromaSubsampling::Yuv420),
            Self::Ayuv => Some(ChromaSubsampling::Yuv444),
            Self::Bgra8 | Self::Rgba8 => None,
        }
    }
}

/// Sample encoding of interleaved audio bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SampleFormat {
    S16,
    F32,
}

/// Opaque GPU-side frame handle. Real GPU surfaces arrive with native
/// capture (Segment S2); mocks only need a stable identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GpuFrameHandle(pub u64);

/// Backing storage of a video frame.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FramePayload {
    Gpu(GpuFrameHandle),
    Cpu(Arc<[u8]>),
}

/// One captured video frame on the shared timeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoFrame {
    pub stream_id: StreamId,
    pub width: u32,
    pub height: u32,
    pub pixel_format: PixelFormat,
    pub pts: i64,
    pub time_base: TimeBase,
    pub payload: FramePayload,
}

/// One captured chunk of interleaved audio samples.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioFrame {
    pub stream_id: StreamId,
    pub sample_format: SampleFormat,
    pub sample_rate: u32,
    pub channels: u16,
    /// Per-channel sample count contained in `data`.
    pub sample_count: u32,
    pub pts: i64,
    pub time_base: TimeBase,
    pub data: Arc<[u8]>,
}

/// Describes a display available for capture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoSourceInfo {
    pub id: String,
    pub name: String,
    pub is_primary: bool,
    pub width: u32,
    pub height: u32,
}

/// Describes a top-level application window available for capture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowSourceInfo {
    pub id: String,
    pub title: String,
    pub app_name: Option<String>,
}

/// Class of audio endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioDeviceKind {
    Output,
    Input,
}

/// Describes an audio endpoint available for capture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioDeviceInfo {
    pub id: String,
    pub name: String,
    pub kind: AudioDeviceKind,
    pub is_default: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixel_format_chroma_subsampling_mapping() {
        assert_eq!(
            PixelFormat::Nv12.chroma_subsampling(),
            Some(ChromaSubsampling::Yuv420)
        );
        assert_eq!(
            PixelFormat::I420.chroma_subsampling(),
            Some(ChromaSubsampling::Yuv420)
        );
        assert_eq!(
            PixelFormat::Ayuv.chroma_subsampling(),
            Some(ChromaSubsampling::Yuv444)
        );
        assert_eq!(PixelFormat::Bgra8.chroma_subsampling(), None);
        assert_eq!(PixelFormat::Rgba8.chroma_subsampling(), None);
    }

    #[test]
    fn pixel_format_and_chroma_subsampling_serde() {
        let formats = [
            (PixelFormat::Bgra8, "\"bgra8\""),
            (PixelFormat::Rgba8, "\"rgba8\""),
            (PixelFormat::Nv12, "\"nv12\""),
            (PixelFormat::I420, "\"i420\""),
            (PixelFormat::Ayuv, "\"ayuv\""),
        ];
        for (fmt, json) in formats {
            assert_eq!(serde_json::to_string(&fmt).unwrap(), json);
            let parsed: PixelFormat = serde_json::from_str(json).unwrap();
            assert_eq!(parsed, fmt);
        }

        let subsamplings = [
            (ChromaSubsampling::Yuv420, "\"yuv420\""),
            (ChromaSubsampling::Yuv444, "\"yuv444\""),
        ];
        for (sub, json) in subsamplings {
            assert_eq!(serde_json::to_string(&sub).unwrap(), json);
            let parsed: ChromaSubsampling = serde_json::from_str(json).unwrap();
            assert_eq!(parsed, sub);
        }
    }
}
