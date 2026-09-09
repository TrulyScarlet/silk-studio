//! Windows Media Foundation H.264 encoder backend (Segment S3).
//!
//! The backend owns all Media Foundation and COM interfaces on the video
//! encoding worker. The public encoder contract only exposes project media
//! types; Windows interfaces never cross that boundary. CPU frames are
//! converted to NV12 in the worker. WGC BGRA GPU surfaces use a same-device
//! D3D11 video processor and remain GPU-backed through the MF input sample.

mod audio;

pub use audio::new_aac_encoder;

use std::collections::VecDeque;
use std::mem::ManuallyDrop;
use std::ptr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use encoder_api::{
    EncoderCandidate, EncoderCapabilities, EncoderError, EncoderPreference, GpuFrameResolveError,
    Result, VideoEncoder, VideoEncoderConfig, VideoEncoderFactory,
};
use gpu_windows::{D3D11VideoConverter, GpuDeviceContext, GpuTextureSlot};
use media_types::{
    EncodedPacket, FramePayload, MediaType, PacketPayload, PixelFormat, StreamId, TimeBase,
    VideoFrame,
};
use windows::core::{IUnknown, Interface, GUID, PWSTR};
use windows::Win32::Foundation::{RPC_E_CHANGED_MODE, S_FALSE, S_OK};
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;
use windows::Win32::Media::MediaFoundation::{
    eAVEncAV1VProfile_Main_420_8, eAVEncH264VProfile, eAVEncH264VProfile_High, eAVEncH265VProfile,
    CODECAPI_AVEncCommonQualityVsSpeed, CODECAPI_AVEncH264CABACEnable, CODECAPI_AVEncMPVGOPSize,
    CODECAPI_AVEncVideoContentType, ICodecAPI, IMFActivate, IMFCollection, IMFDXGIDeviceManager,
    IMFMediaEventGenerator, IMFMediaType, IMFSample, IMFTransform, METransformDrainComplete,
    METransformHaveOutput, METransformNeedInput, MFCreateAlignedMemoryBuffer,
    MFCreateDXGIDeviceManager, MFCreateDXGISurfaceBuffer, MFCreateMediaType, MFCreateMemoryBuffer,
    MFCreateSample, MFMediaType_Video, MFNominalRange_16_235, MFSampleExtension_CleanPoint,
    MFShutdown, MFStartup, MFT_FRIENDLY_NAME_Attribute, MFVideoChromaSubsampling_MPEG2,
    MFVideoFormat_H264, MFVideoFormat_NV12, MFVideoInterlace_Progressive, MFVideoPrimaries_BT709,
    MFVideoTransFunc_709, MFVideoTransferMatrix_BT709, MFSTARTUP_FULL, MFT_CATEGORY_VIDEO_ENCODER,
    MFT_ENUM_FLAG, MFT_ENUM_FLAG_HARDWARE, MFT_ENUM_FLAG_LOCALMFT, MFT_ENUM_FLAG_SORTANDFILTER,
    MFT_ENUM_FLAG_SYNCMFT, MFT_MESSAGE_COMMAND_DRAIN, MFT_MESSAGE_NOTIFY_BEGIN_STREAMING,
    MFT_MESSAGE_NOTIFY_END_OF_STREAM, MFT_MESSAGE_NOTIFY_START_OF_STREAM,
    MFT_MESSAGE_SET_D3D_MANAGER, MFT_OUTPUT_DATA_BUFFER, MFT_OUTPUT_STREAM_INFO,
    MFT_OUTPUT_STREAM_PROVIDES_SAMPLES, MFT_REGISTER_TYPE_INFO, MF_EVENT_FLAG_NO_WAIT,
    MF_E_NOTACCEPTING, MF_E_NOT_FOUND, MF_E_NO_EVENTS_AVAILABLE, MF_E_TRANSFORM_NEED_MORE_INPUT,
    MF_MT_AVG_BITRATE, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE, MF_MT_INTERLACE_MODE, MF_MT_MAJOR_TYPE,
    MF_MT_MAX_KEYFRAME_SPACING, MF_MT_MPEG2_PROFILE, MF_MT_PIXEL_ASPECT_RATIO, MF_MT_SUBTYPE,
    MF_MT_TRANSFER_FUNCTION, MF_MT_VIDEO_CHROMA_SITING, MF_MT_VIDEO_NOMINAL_RANGE,
    MF_MT_VIDEO_PRIMARIES, MF_MT_YUV_MATRIX, MF_SA_D3D11_AWARE, MF_TRANSFORM_ASYNC,
    MF_TRANSFORM_ASYNC_UNLOCK, MF_VERSION,
};
use windows::Win32::System::Com::{
    CoInitializeEx, CoTaskMemFree, CoUninitialize, COINIT_MULTITHREADED,
};
use windows::Win32::System::Variant::VARIANT;

const INPUT_STREAM_ID: u32 = 0;
const OUTPUT_STREAM_ID: u32 = 0;
const MF_TIMEBASE: TimeBase = TimeBase::new(1, 10_000_000);
const MAX_SUPPORTED_WIDTH: u32 = 7_680;
const MAX_SUPPORTED_HEIGHT: u32 = 4_320;
const SUPPORTED_FPS: &[u32] = &[30, 60, 120];

#[allow(non_upper_case_globals)]
const MFVideoFormat_HEVC: GUID = GUID::from_u128(0x43564548_0000_0010_8000_00aa00389b71);
#[allow(non_upper_case_globals)]
const MFVideoFormat_AV1: GUID = GUID::from_u128(0x31305641_0000_0010_8000_00aa00389b71);
#[allow(non_upper_case_globals)]
const MFVideoFormat_AYUV: GUID = GUID::from_u128(0x56555941_0000_0010_8000_00aa00389b71);
#[allow(non_upper_case_globals)]
const eAVEncH265VProfile_Main_444: eAVEncH265VProfile = eAVEncH265VProfile(6);
#[allow(non_upper_case_globals)]
const eAVEncH264VProfile_High444: eAVEncH264VProfile = eAVEncH264VProfile(244);

fn backend_error(stage: &str, error: impl std::fmt::Display) -> EncoderError {
    EncoderError::InitializationFailed {
        backend: "media-foundation".to_string(),
        details: format!("{stage}: {error}"),
    }
}

fn encode_error(stage: &str, error: impl std::fmt::Display) -> EncoderError {
    EncoderError::EncodeFailed {
        details: format!("Media Foundation {stage}: {error}"),
    }
}

fn capability(name: String, codec: &str, hardware_accelerated: bool) -> EncoderCapabilities {
    EncoderCapabilities {
        backend_name: name,
        codec: codec.to_string(),
        hardware_accelerated,
        max_width: MAX_SUPPORTED_WIDTH,
        max_height: MAX_SUPPORTED_HEIGHT,
        supported_fps: SUPPORTED_FPS.to_vec(),
        supported_pixel_formats: vec![PixelFormat::Nv12],
    }
}

fn packed_pair(high: u32, low: u32) -> u64 {
    (u64::from(high) << 32) | u64::from(low)
}

fn friendly_name(activation: &IMFActivate) -> Option<String> {
    let mut value = PWSTR(std::ptr::null_mut());
    let mut length = 0_u32;
    unsafe {
        activation
            .GetAllocatedString(&MFT_FRIENDLY_NAME_Attribute, &mut value, &mut length)
            .ok()?;
        let text = if value.0.is_null() {
            String::new()
        } else {
            String::from_utf16_lossy(std::slice::from_raw_parts(value.0, length as usize))
        };
        CoTaskMemFree(Some(value.0.cast()));
        (!text.is_empty()).then_some(text)
    }
}

fn clean_output_parts(
    output: &mut MFT_OUTPUT_DATA_BUFFER,
) -> (Option<IMFSample>, Option<IMFCollection>) {
    // The generated binding uses ManuallyDrop because ownership is transferred
    // by the native ProcessOutput call.
    unsafe {
        (
            ManuallyDrop::take(&mut output.pSample),
            ManuallyDrop::take(&mut output.pEvents),
        )
    }
}

struct ActivationSpec {
    id: String,
    activation: IMFActivate,
    capabilities: EncoderCapabilities,
}

// IMFActivate is !Send in the generated bindings. The factory is created on
// the controller but is not populated until discovery runs on the encoder
// worker; it is then consumed on that same COM-initialized worker thread.
unsafe impl Send for ActivationSpec {}

#[derive(Default)]
struct FactoryState {
    activations: Vec<ActivationSpec>,
    com_initialized: bool,
    mf_started: bool,
}

/// Discovers Media Foundation H.264 MFTs and creates worker-owned encoders.
pub struct MediaFoundationFactory {
    state: Mutex<FactoryState>,
}

impl Default for MediaFoundationFactory {
    fn default() -> Self {
        Self {
            state: Mutex::new(FactoryState::default()),
        }
    }
}

impl MediaFoundationFactory {
    pub fn new() -> Self {
        Self::default()
    }

    fn ensure_runtime(state: &mut FactoryState) -> Result<()> {
        if !state.com_initialized {
            let result = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            if result != S_OK && result != S_FALSE && result != RPC_E_CHANGED_MODE {
                result
                    .ok()
                    .map_err(|error| backend_error("initialize COM", error))?;
            }
            state.com_initialized = true;
        }
        if !state.mf_started {
            unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL) }
                .map_err(|error| backend_error("start Media Foundation", error))?;
            state.mf_started = true;
        }
        Ok(())
    }

    fn enumerate_one(
        subtype: &GUID,
        codec_name: &str,
        flags: MFT_ENUM_FLAG,
        hardware_accelerated: bool,
        prefix: &str,
        next_id: &mut usize,
    ) -> Result<Vec<ActivationSpec>> {
        let output_type = MFT_REGISTER_TYPE_INFO {
            guidMajorType: MFMediaType_Video,
            guidSubtype: *subtype,
        };
        let mut raw_activations: *mut Option<IMFActivate> = ptr::null_mut();
        let mut count = 0_u32;
        let result = unsafe {
            windows::Win32::Media::MediaFoundation::MFTEnumEx(
                MFT_CATEGORY_VIDEO_ENCODER,
                flags,
                None,
                Some(&output_type),
                &mut raw_activations,
                &mut count,
            )
        };
        if let Err(error) = result {
            if !raw_activations.is_null() {
                unsafe {
                    CoTaskMemFree(Some(raw_activations.cast()));
                }
            }
            if error.code() == MF_E_NOT_FOUND {
                return Ok(Vec::new());
            }
            return Err(backend_error(
                &format!("enumerate {codec_name} MFTs"),
                error,
            ));
        }

        let mut found = Vec::with_capacity(count as usize);
        if !raw_activations.is_null() {
            for index in 0..count as usize {
                let activation = unsafe { ptr::read(raw_activations.add(index)) };
                let Some(activation) = activation else {
                    continue;
                };
                let id = format!("mf-{codec_name}-{prefix}-{}", *next_id);
                *next_id += 1;
                let name = friendly_name(&activation).unwrap_or_else(|| {
                    format!("media-foundation-{codec_name}-{prefix}-{}", *next_id)
                });
                found.push(ActivationSpec {
                    id,
                    capabilities: capability(name, codec_name, hardware_accelerated),
                    activation,
                });
            }
            unsafe {
                CoTaskMemFree(Some(raw_activations.cast()));
            }
        }
        Ok(found)
    }

    fn shutdown_runtime(&mut self) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        // Release activation objects before shutting down MF. This method is
        // called after the fallback wrapper has dropped its active transform.
        state.activations.clear();
        if state.mf_started {
            let _ = unsafe { MFShutdown() };
            state.mf_started = false;
        }
        if state.com_initialized {
            unsafe { CoUninitialize() };
            state.com_initialized = false;
        }
    }
}

impl VideoEncoderFactory for MediaFoundationFactory {
    fn discover(&self) -> Result<Vec<EncoderCandidate>> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| backend_error("lock factory state", "mutex poisoned"))?;
        Self::ensure_runtime(&mut state)?;
        state.activations.clear();
        let sync_flags =
            MFT_ENUM_FLAG_SYNCMFT | MFT_ENUM_FLAG_LOCALMFT | MFT_ENUM_FLAG_SORTANDFILTER;
        // The encoder contract uses the synchronous IMFTransform pull model;
        // asynchronous MFTs require an event-driven worker and must not be
        // activated by this backend.
        let hardware_flags = sync_flags | MFT_ENUM_FLAG_HARDWARE;
        let mut next_id = 0;
        let mut found = Vec::new();
        for (subtype, codec_name) in [
            (&MFVideoFormat_H264, "avc"),
            (&MFVideoFormat_HEVC, "hevc"),
            (&MFVideoFormat_AV1, "av1"),
        ] {
            found.extend(Self::enumerate_one(
                subtype,
                codec_name,
                sync_flags,
                false,
                "software",
                &mut next_id,
            )?);
            found.extend(Self::enumerate_one(
                subtype,
                codec_name,
                hardware_flags,
                true,
                "hardware",
                &mut next_id,
            )?);
        }

        let candidates = found
            .iter()
            .map(|spec| EncoderCandidate {
                id: spec.id.clone(),
                capabilities: spec.capabilities.clone(),
            })
            .collect();
        state.activations = found;
        Ok(candidates)
    }

    fn create(&self, candidate: &EncoderCandidate) -> Result<Box<dyn VideoEncoder>> {
        let state = self
            .state
            .lock()
            .map_err(|_| backend_error("lock factory state", "mutex poisoned"))?;
        let spec = state
            .activations
            .iter()
            .find(|spec| spec.id == candidate.id)
            .ok_or_else(|| backend_error("create encoder", "unknown discovery candidate"))?;
        let transform: IMFTransform = unsafe { spec.activation.ActivateObject() }
            .map_err(|error| backend_error("activate MFT", error))?;
        let mut async_mft = false;
        let mut event_generator = None;
        if let Ok(attributes) = unsafe { transform.GetAttributes() } {
            let is_async = unsafe { attributes.GetUINT32(&MF_TRANSFORM_ASYNC).unwrap_or(0) } != 0;
            if is_async {
                async_mft = true;
                unsafe {
                    attributes
                        .SetUINT32(&MF_TRANSFORM_ASYNC_UNLOCK, 1)
                        .map_err(|error| backend_error("unlock asynchronous MFT", error))?;
                }
                event_generator = Some(transform.cast::<IMFMediaEventGenerator>().map_err(
                    |error| backend_error("query asynchronous MFT event generator", error),
                )?);
            }
        }
        Ok(Box::new(MediaFoundationVideoEncoder::new(
            candidate.capabilities.clone(),
            transform,
            async_mft,
            event_generator,
        )))
    }

    fn shutdown(&mut self) {
        self.shutdown_runtime();
    }
}

impl Drop for MediaFoundationFactory {
    fn drop(&mut self) {
        self.shutdown_runtime();
    }
}

/// Construct a fallback encoder using Media Foundation discovery.
pub fn new_fallback_encoder(preference: EncoderPreference) -> encoder_api::FallbackVideoEncoder {
    encoder_api::FallbackVideoEncoder::new(Box::new(MediaFoundationFactory::new()), preference)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct OutputTimeline {
    last_dts_ms: Option<i64>,
}

impl OutputTimeline {
    const fn new() -> Self {
        Self { last_dts_ms: None }
    }

    fn reset(&mut self) {
        self.last_dts_ms = None;
    }

    fn advance(&mut self, raw_pts_ms: i64) -> (i64, i64) {
        let dts = match self.last_dts_ms {
            None => raw_pts_ms,
            Some(prev_dts) => raw_pts_ms.max(prev_dts),
        };
        self.last_dts_ms = Some(dts);
        let pts = raw_pts_ms.max(dts);
        (pts, dts)
    }

    #[cfg(test)]
    fn last_dts_ms(&self) -> Option<i64> {
        self.last_dts_ms
    }
}

fn nominal_frame_duration_ms(fps: u32) -> i64 {
    TimeBase::from_hz(fps.max(1))
        .rescale(1, TimeBase::MILLISECOND)
        .max(1)
}

struct MediaFoundationVideoEncoder {
    capabilities: EncoderCapabilities,
    transform: Option<IMFTransform>,
    config: Option<VideoEncoderConfig>,
    extradata: Option<PacketPayload>,
    output_info: Option<MFT_OUTPUT_STREAM_INFO>,
    gpu_context: Option<encoder_api::GpuFrameContext>,
    gpu_device: Option<Arc<GpuDeviceContext>>,
    gpu_converter: Option<D3D11VideoConverter>,
    gpu_converter_input: Option<(u32, u32)>,
    dxgi_device_manager: Option<IMFDXGIDeviceManager>,
    async_mft: bool,
    event_generator: Option<IMFMediaEventGenerator>,
    async_input_ready: bool,
    async_pending_outputs: u32,
    stream_id: StreamId,
    last_frame_duration_hns: i64,
    timeline: OutputTimeline,
    nominal_frame_duration_ms: i64,
    input_pts_queue: VecDeque<i64>,
    encoded_outputs: u64,
    metrics: encoder_api::EncoderMetrics,
    started: bool,
}

// IMFTransform and its samples are intentionally confined to the worker by
// engine construction. `VideoEncoder` is the handoff marker for that worker;
// no MF value is ever accessed concurrently or sent through a channel.
unsafe impl Send for MediaFoundationVideoEncoder {}

impl MediaFoundationVideoEncoder {
    fn new(
        capabilities: EncoderCapabilities,
        transform: IMFTransform,
        async_mft: bool,
        event_generator: Option<IMFMediaEventGenerator>,
    ) -> Self {
        Self {
            capabilities,
            transform: Some(transform),
            config: None,
            extradata: None,
            output_info: None,
            gpu_context: None,
            gpu_device: None,
            gpu_converter: None,
            gpu_converter_input: None,
            dxgi_device_manager: None,
            async_mft,
            event_generator,
            async_input_ready: false,
            async_pending_outputs: 0,
            stream_id: StreamId(0),
            last_frame_duration_hns: 0,
            timeline: OutputTimeline::new(),
            nominal_frame_duration_ms: 1,
            input_pts_queue: VecDeque::new(),
            encoded_outputs: 0,
            metrics: encoder_api::EncoderMetrics::default(),
            started: false,
        }
    }

    fn transform(&self) -> Result<&IMFTransform> {
        self.transform.as_ref().ok_or(EncoderError::NotConfigured)
    }

    fn set_common_video_attributes(
        media_type: &IMFMediaType,
        config: &VideoEncoderConfig,
        subtype: &GUID,
        include_profile: bool,
    ) -> Result<()> {
        let bitrate = config
            .suggested_bitrate_kbps()
            .checked_mul(1_000)
            .ok_or_else(|| EncoderError::UnsupportedConfiguration {
                reason: "bitrate is too large".to_string(),
            })?;
        unsafe {
            media_type
                .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
                .map_err(|error| backend_error("set media major type", error))?;
            media_type
                .SetGUID(&MF_MT_SUBTYPE, subtype)
                .map_err(|error| backend_error("set media subtype", error))?;
            media_type
                .SetUINT64(&MF_MT_FRAME_SIZE, packed_pair(config.width, config.height))
                .map_err(|error| backend_error("set frame size", error))?;
            media_type
                .SetUINT64(&MF_MT_FRAME_RATE, packed_pair(config.fps, 1))
                .map_err(|error| backend_error("set frame rate", error))?;
            media_type
                .SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, packed_pair(1, 1))
                .map_err(|error| backend_error("set pixel aspect ratio", error))?;
            media_type
                .SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)
                .map_err(|error| backend_error("set progressive mode", error))?;
            media_type
                .SetUINT32(&MF_MT_YUV_MATRIX, MFVideoTransferMatrix_BT709.0 as u32)
                .map_err(|error| backend_error("set YUV transfer matrix", error))?;
            media_type
                .SetUINT32(&MF_MT_VIDEO_NOMINAL_RANGE, MFNominalRange_16_235.0 as u32)
                .map_err(|error| backend_error("set video nominal range", error))?;
            media_type
                .SetUINT32(&MF_MT_VIDEO_PRIMARIES, MFVideoPrimaries_BT709.0 as u32)
                .map_err(|error| backend_error("set video primaries", error))?;
            media_type
                .SetUINT32(&MF_MT_TRANSFER_FUNCTION, MFVideoTransFunc_709.0 as u32)
                .map_err(|error| backend_error("set video transfer function", error))?;
            if subtype != &MFVideoFormat_AYUV && config.pixel_format != PixelFormat::Ayuv {
                media_type
                    .SetUINT32(
                        &MF_MT_VIDEO_CHROMA_SITING,
                        MFVideoChromaSubsampling_MPEG2.0 as u32,
                    )
                    .map_err(|error| backend_error("set video chroma siting", error))?;
            }
            if subtype == &MFVideoFormat_H264
                || subtype == &MFVideoFormat_HEVC
                || subtype == &MFVideoFormat_AV1
            {
                media_type
                    .SetUINT32(&MF_MT_AVG_BITRATE, bitrate)
                    .map_err(|error| backend_error("set average bitrate", error))?;
                if subtype != &MFVideoFormat_AV1 {
                    media_type
                        .SetUINT32(
                            &MF_MT_MAX_KEYFRAME_SPACING,
                            config.keyframe_interval_frames(),
                        )
                        .map_err(|error| backend_error("set keyframe interval", error))?;
                }
                if include_profile {
                    if subtype == &MFVideoFormat_H264 {
                        let profile = if config.pixel_format == PixelFormat::Ayuv {
                            eAVEncH264VProfile_High444.0 as u32
                        } else {
                            eAVEncH264VProfile_High.0 as u32
                        };
                        media_type
                            .SetUINT32(&MF_MT_MPEG2_PROFILE, profile)
                            .map_err(|error| backend_error("set H.264 profile", error))?;
                    } else if subtype == &MFVideoFormat_HEVC {
                        if config.pixel_format == PixelFormat::Ayuv {
                            media_type
                                .SetUINT32(
                                    &MF_MT_MPEG2_PROFILE,
                                    eAVEncH265VProfile_Main_444.0 as u32,
                                )
                                .map_err(|error| backend_error("set HEVC 4:4:4 profile", error))?;
                        }
                    } else if subtype == &MFVideoFormat_AV1 {
                        media_type
                            .SetUINT32(&MF_MT_MPEG2_PROFILE, eAVEncAV1VProfile_Main_420_8.0 as u32)
                            .map_err(|error| backend_error("set AV1 profile", error))?;
                    }
                }
            }
        }
        Ok(())
    }

    fn configure_encoder_properties(&self, transform: &IMFTransform, config: &VideoEncoderConfig) {
        // Standard Media Foundation codec properties configuration via ICodecAPI.
        // Optional ICodecAPI properties vary by Windows version and certified
        // hardware drivers; this request is best-effort across all codecs.
        // If the transform does not expose ICodecAPI or rejects the property,
        // configuration proceeds without failure (H.264/HEVC media-type keyframe
        // spacing and driver defaults remain active).
        if let Ok(codec_api) = transform.cast::<ICodecAPI>() {
            let gop_variant = VARIANT::from(config.keyframe_interval_frames());
            unsafe {
                let _ = codec_api.SetValue(&CODECAPI_AVEncMPVGOPSize, &gop_variant);
            }

            // Soft-configure CODECAPI_AVEncH264CABACEnable (VARIANT_TRUE for H.264).
            let normalized_codec = encoder_api::normalize_codec(&self.capabilities.codec);
            if normalized_codec == "avc" || normalized_codec == "h264" {
                let cabac_variant = VARIANT::from(true);
                unsafe {
                    let _ = codec_api.SetValue(&CODECAPI_AVEncH264CABACEnable, &cabac_variant);
                }
            }

            // Soft-configure CODECAPI_AVEncVideoContentType (1 = Desktop).
            let content_type_variant = VARIANT::from(1u32);
            unsafe {
                let _ = codec_api.SetValue(&CODECAPI_AVEncVideoContentType, &content_type_variant);
            }

            // Soft-configure CODECAPI_AVEncCommonQualityVsSpeed (100).
            let quality_variant = VARIANT::from(100u32);
            unsafe {
                let _ = codec_api.SetValue(&CODECAPI_AVEncCommonQualityVsSpeed, &quality_variant);
            }
        }
    }

    fn configure_codec_properties(&self, transform: &IMFTransform, config: &VideoEncoderConfig) {
        self.configure_encoder_properties(transform, config);
    }

    fn configure_dxgi_device_manager(&mut self, transform: &IMFTransform) -> Result<()> {
        let Some(device) = self.gpu_device.as_ref() else {
            return Ok(());
        };
        let d3d11_aware = unsafe { transform.GetAttributes() }
            .ok()
            .and_then(|attributes| unsafe { attributes.GetUINT32(&MF_SA_D3D11_AWARE).ok() })
            .unwrap_or(0)
            != 0;
        if !d3d11_aware {
            return Err(EncoderError::UnsupportedConfiguration {
                reason: format!(
                    "MFT '{}' does not advertise MF_SA_D3D11_AWARE",
                    self.capabilities.backend_name
                ),
            });
        }
        let mut reset_token = 0_u32;
        let mut manager = None;
        unsafe { MFCreateDXGIDeviceManager(&mut reset_token, &mut manager) }
            .map_err(|error| backend_error("create DXGI device manager", error))?;
        let manager = manager.ok_or_else(|| {
            backend_error(
                "create DXGI device manager",
                "Media Foundation returned no manager",
            )
        })?;
        let unknown: IUnknown = device
            .device()
            .cast()
            .map_err(|error| backend_error("query DXGI device manager device", error))?;
        unsafe {
            manager
                .ResetDevice(&unknown, reset_token)
                .map_err(|error| backend_error("reset DXGI device manager", error))?;
            transform
                .ProcessMessage(MFT_MESSAGE_SET_D3D_MANAGER, manager.as_raw() as usize)
                .map_err(|error| backend_error("install DXGI device manager", error))?;
        }
        self.dxgi_device_manager = Some(manager);
        Ok(())
    }

    fn clear_dxgi_device_manager(&mut self, transform: &IMFTransform) {
        if self.dxgi_device_manager.take().is_some() {
            unsafe {
                let _ = transform.ProcessMessage(MFT_MESSAGE_SET_D3D_MANAGER, 0);
            }
        }
    }

    fn validate_frame_dimensions(frame: &VideoFrame, config: &VideoEncoderConfig) -> Result<()> {
        if frame.width != config.width || frame.height != config.height {
            return Err(EncoderError::UnsupportedConfiguration {
                reason: format!(
                    "frame dimensions {}x{} do not match encoder {}x{}",
                    frame.width, frame.height, config.width, config.height
                ),
            });
        }
        Ok(())
    }

    fn input_bytes(&self, frame: &VideoFrame, config: &VideoEncoderConfig) -> Result<Vec<u8>> {
        Self::validate_frame_dimensions(frame, config)?;
        match &frame.payload {
            FramePayload::Cpu(bytes) => match config.pixel_format {
                PixelFormat::Ayuv => {
                    convert_to_ayuv(bytes, frame.pixel_format, frame.width, frame.height)
                }
                _ => convert_to_nv12(bytes, frame.pixel_format, frame.width, frame.height),
            },
            FramePayload::Gpu(_) => Err(encode_error(
                "accept GPU frame",
                "GPU frames require a DXGI-backed input sample",
            )),
        }
    }

    fn make_sample(
        &self,
        buffer: &windows::Win32::Media::MediaFoundation::IMFMediaBuffer,
        frame: &VideoFrame,
    ) -> Result<IMFSample> {
        let sample = unsafe { MFCreateSample() }
            .map_err(|error| encode_error("create input sample", error))?;
        unsafe {
            sample
                .AddBuffer(buffer)
                .map_err(|error| encode_error("attach input buffer", error))?;
            sample
                .SetSampleTime(frame.time_base.rescale(frame.pts, MF_TIMEBASE))
                .map_err(|error| encode_error("set input timestamp", error))?;
            sample
                .SetSampleDuration(self.last_frame_duration_hns)
                .map_err(|error| encode_error("set input duration", error))?;
        }
        Ok(sample)
    }

    fn make_input_sample(&self, data: &[u8], frame: &VideoFrame) -> Result<IMFSample> {
        let length = u32::try_from(data.len())
            .map_err(|_| encode_error("create input buffer", "frame is too large"))?;
        let buffer = unsafe { MFCreateMemoryBuffer(length) }
            .map_err(|error| encode_error("create input buffer", error))?;
        let mut destination = ptr::null_mut();
        let mut max_length = 0_u32;
        unsafe {
            buffer
                .Lock(&mut destination, Some(&mut max_length), None)
                .map_err(|error| encode_error("lock input buffer", error))?;
            if max_length < length {
                let _ = buffer.Unlock();
                return Err(encode_error(
                    "fill input buffer",
                    "buffer capacity is too small",
                ));
            }
            ptr::copy_nonoverlapping(data.as_ptr(), destination, data.len());
            buffer
                .Unlock()
                .map_err(|error| encode_error("unlock input buffer", error))?;
            buffer
                .SetCurrentLength(length)
                .map_err(|error| encode_error("set input length", error))?;
        }
        self.make_sample(&buffer, frame)
    }

    fn make_gpu_input_sample(
        &mut self,
        frame: &VideoFrame,
        config: &VideoEncoderConfig,
    ) -> Result<Option<IMFSample>> {
        let context = self.gpu_context.as_ref().ok_or_else(|| {
            encode_error("resolve GPU frame", "capture GPU context is not installed")
        })?;
        let lease = match context.resolve(frame) {
            Ok(lease) => lease,
            Err(GpuFrameResolveError::StaleHandle { .. }) => return Ok(None),
            Err(error) => return Err(encode_error("resolve GPU frame", error)),
        };
        let slot = lease.downcast_ref::<GpuTextureSlot>().ok_or_else(|| {
            encode_error(
                "resolve GPU frame",
                "capture resource is not a supported D3D11 texture slot",
            )
        })?;
        let slot = slot.clone();
        let input_dimensions = slot.dimensions();
        if frame.width != input_dimensions.0 || frame.height != input_dimensions.1 {
            return Err(encode_error(
                "resolve GPU frame",
                format!(
                    "frame metadata {}x{} does not match captured texture {}x{}",
                    frame.width, frame.height, input_dimensions.0, input_dimensions.1
                ),
            ));
        }
        if let Some(device) = self.gpu_device.as_ref() {
            if !Arc::ptr_eq(device, slot.device_context()) {
                return Err(encode_error(
                    "convert GPU frame",
                    "captured texture belongs to a different D3D11 device",
                ));
            }
        } else {
            return Err(encode_error(
                "convert GPU frame",
                "DXGI device manager is not configured",
            ));
        }
        if self.gpu_converter_input != Some(input_dimensions) {
            self.gpu_converter = None;
            self.gpu_converter_input = None;
        }
        if self.gpu_converter.is_none() {
            let converter = D3D11VideoConverter::new(
                slot.device_context().clone(),
                input_dimensions.0,
                input_dimensions.1,
                config.width,
                config.height,
                config.fps,
            )
            .map_err(|error| encode_error("create GPU converter", error))?;
            self.gpu_converter = Some(converter);
            self.gpu_converter_input = Some(input_dimensions);
        }
        let converted = self
            .gpu_converter
            .as_ref()
            .ok_or_else(|| encode_error("convert GPU frame", "converter is unavailable"))?
            .convert(&slot)
            .map_err(|error| encode_error("convert GPU frame", error))?;
        let surface: IUnknown = converted
            .texture()
            .cast()
            .map_err(|error| encode_error("wrap converted GPU frame", error))?;
        let buffer =
            unsafe { MFCreateDXGISurfaceBuffer(&ID3D11Texture2D::IID, &surface, 0, false) }
                .map_err(|error| encode_error("create DXGI input buffer", error))?;
        self.make_sample(&buffer, frame).map(Some)
    }

    fn next_async_event(&self, wait: bool) -> Result<Option<u32>> {
        let Some(generator) = self.event_generator.as_ref() else {
            return Err(encode_error(
                "wait for MFT event",
                "asynchronous MFT has no event generator",
            ));
        };
        let flags = if wait {
            windows::Win32::Media::MediaFoundation::MF_EVENT_FLAG_NONE
        } else {
            MF_EVENT_FLAG_NO_WAIT
        };
        let event = match unsafe { generator.GetEvent(flags) } {
            Ok(event) => event,
            Err(error) if !wait && error.code() == MF_E_NO_EVENTS_AVAILABLE => return Ok(None),
            Err(error) => return Err(encode_error("wait for MFT event", error)),
        };
        let status = unsafe { event.GetStatus() }
            .map_err(|error| encode_error("read MFT event status", error))?;
        if status.0 < 0 {
            return Err(encode_error("MFT event", status));
        }
        let event_type = unsafe { event.GetType() }
            .map_err(|error| encode_error("read MFT event type", error))?;
        Ok(Some(event_type))
    }

    fn collect_async_events(&mut self) -> Result<()> {
        if !self.async_mft {
            return Ok(());
        }
        while let Some(event_type) = self.next_async_event(false)? {
            if event_type == METransformNeedInput.0 as u32 {
                self.async_input_ready = true;
            } else if event_type == METransformHaveOutput.0 as u32 {
                self.async_pending_outputs = self.async_pending_outputs.saturating_add(1);
            }
        }
        Ok(())
    }

    fn wait_for_async_input(&mut self, fallback_pts: i64) -> Result<Vec<EncodedPacket>> {
        if !self.async_mft {
            return Ok(Vec::new());
        }
        let mut packets = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let Some(event_type) = self.next_async_event(false)? else {
                if Instant::now() >= deadline {
                    return Err(encode_error(
                        "wait for MFT input",
                        "asynchronous MFT did not request input within 5 seconds",
                    ));
                }
                std::thread::sleep(Duration::from_millis(1));
                continue;
            };
            if event_type == METransformNeedInput.0 as u32 {
                self.async_input_ready = true;
                return Ok(packets);
            } else if event_type == METransformHaveOutput.0 as u32 {
                self.async_pending_outputs = self.async_pending_outputs.saturating_add(1);
                packets.extend(self.drain_pending_async_outputs(fallback_pts)?);
            }
        }
    }

    fn drain_pending_async_outputs(&mut self, fallback_pts: i64) -> Result<Vec<EncodedPacket>> {
        let mut packets = Vec::new();
        while self.async_pending_outputs > 0 {
            self.async_pending_outputs -= 1;
            packets.extend(self.drain_output(fallback_pts)?);
        }
        Ok(packets)
    }

    fn output_sample(&self) -> Result<(MFT_OUTPUT_DATA_BUFFER, Option<IMFSample>)> {
        let info = self.output_info.ok_or(EncoderError::NotConfigured)?;
        let provides_samples = (info.dwFlags & MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 as u32) != 0;
        let supplied = if provides_samples {
            None
        } else {
            let config = self.config.as_ref().ok_or(EncoderError::NotConfigured)?;
            let minimum = config.width.saturating_mul(config.height).max(1_024);
            let size = info.cbSize.max(minimum);
            let buffer = unsafe {
                if info.cbAlignment > 1 {
                    MFCreateAlignedMemoryBuffer(size, info.cbAlignment)
                } else {
                    MFCreateMemoryBuffer(size)
                }
            }
            .map_err(|error| encode_error("create output buffer", error))?;
            let sample = unsafe { MFCreateSample() }
                .map_err(|error| encode_error("create output sample", error))?;
            unsafe {
                sample
                    .AddBuffer(&buffer)
                    .map_err(|error| encode_error("attach output buffer", error))?;
            }
            Some(sample)
        };
        let output = MFT_OUTPUT_DATA_BUFFER {
            dwStreamID: OUTPUT_STREAM_ID,
            pSample: ManuallyDrop::new(supplied.clone()),
            dwStatus: 0,
            pEvents: ManuallyDrop::new(None),
        };
        Ok((output, supplied))
    }

    fn drain_output(&mut self, fallback_pts: i64) -> Result<Vec<EncodedPacket>> {
        let transform = self.transform()?.clone();
        let (mut output, _supplied) = self.output_sample()?;
        let mut status = 0_u32;
        let process =
            unsafe { transform.ProcessOutput(0, std::slice::from_mut(&mut output), &mut status) };
        let (sample, _events) = clean_output_parts(&mut output);
        if let Err(error) = process {
            if error.code() == MF_E_TRANSFORM_NEED_MORE_INPUT {
                return Ok(Vec::new());
            }
            return Err(encode_error("process output", error));
        }
        let Some(sample) = sample else {
            return Ok(Vec::new());
        };
        let buffer = unsafe { sample.ConvertToContiguousBuffer() }
            .map_err(|error| encode_error("read output buffer", error))?;
        let mut data = ptr::null_mut();
        let mut current_length = 0_u32;
        unsafe {
            buffer
                .Lock(&mut data, None, Some(&mut current_length))
                .map_err(|error| encode_error("lock output buffer", error))?;
            let bytes = std::slice::from_raw_parts(data, current_length as usize).to_vec();
            buffer
                .Unlock()
                .map_err(|error| encode_error("unlock output buffer", error))?;
            if bytes.is_empty() {
                return Ok(Vec::new());
            }
            if self.extradata.is_none() {
                if let Some(parameter_sets) =
                    parameter_sets_for_codec(&bytes, &self.capabilities.codec)
                {
                    self.extradata = Some(PacketPayload::from(parameter_sets));
                }
            }
            let sample_time = sample.GetSampleTime().unwrap_or(fallback_pts);
            let raw_pts_ms = self
                .input_pts_queue
                .pop_front()
                .unwrap_or_else(|| MF_TIMEBASE.rescale(sample_time, TimeBase::MILLISECOND));
            let (pts, dts) = self.timeline.advance(raw_pts_ms);
            let clean_point = sample.GetUINT32(&MFSampleExtension_CleanPoint).unwrap_or(0) != 0;
            let is_keyframe = self.encoded_outputs == 0 || clean_point;
            self.encoded_outputs = self.encoded_outputs.saturating_add(1);
            self.metrics.frames_encoded = self.metrics.frames_encoded.saturating_add(1);
            Ok(vec![EncodedPacket {
                stream_id: self.stream_id,
                media_type: MediaType::Video,
                pts,
                dts,
                duration: self.nominal_frame_duration_ms,
                time_base: TimeBase::MILLISECOND,
                is_keyframe,
                sequence: 0,
                payload: PacketPayload::from(bytes),
            }])
        }
    }

    fn drain_available_outputs(&mut self, fallback_pts: i64) -> Result<Vec<EncodedPacket>> {
        if self.async_mft {
            self.collect_async_events()?;
            return self.drain_pending_async_outputs(fallback_pts);
        }
        let mut packets = Vec::new();
        loop {
            let output = self.drain_output(fallback_pts)?;
            if output.is_empty() {
                break;
            }
            packets.extend(output);
        }
        Ok(packets)
    }
}

impl VideoEncoder for MediaFoundationVideoEncoder {
    fn capabilities(&self) -> EncoderCapabilities {
        self.capabilities.clone()
    }

    fn configure(&mut self, config: VideoEncoderConfig) -> Result<()> {
        config.validate()?;
        if config.pixel_format != PixelFormat::Nv12 && config.pixel_format != PixelFormat::Ayuv {
            return Err(EncoderError::UnsupportedConfiguration {
                reason: format!(
                    "Media Foundation video encoder only supports NV12 or AYUV input, got {:?}",
                    config.pixel_format
                ),
            });
        }
        let transform = self.transform()?.clone();
        self.clear_dxgi_device_manager(&transform);
        self.gpu_converter = None;
        self.gpu_converter_input = None;
        self.async_input_ready = false;
        self.async_pending_outputs = 0;
        self.input_pts_queue.clear();
        let output_subtype = match encoder_api::normalize_codec(&self.capabilities.codec) {
            "hevc" => MFVideoFormat_HEVC,
            "av1" => MFVideoFormat_AV1,
            _ => MFVideoFormat_H264,
        };
        let output_type = unsafe { MFCreateMediaType() }
            .map_err(|error| backend_error("create output media type", error))?;
        Self::set_common_video_attributes(&output_type, &config, &output_subtype, true)?;
        self.configure_codec_properties(&transform, &config);
        self.configure_dxgi_device_manager(&transform)?;
        if let Err(error) = unsafe { transform.SetOutputType(OUTPUT_STREAM_ID, &output_type, 0) } {
            if config.pixel_format == PixelFormat::Ayuv {
                return Err(EncoderError::UnsupportedConfiguration {
                    reason: "4:4:4 AYUV is not supported by this Media Foundation transform"
                        .to_string(),
                });
            }
            return Err(backend_error(
                &format!("set {} output type", self.capabilities.codec),
                error,
            ));
        }

        let input_subtype = if config.pixel_format == PixelFormat::Ayuv {
            MFVideoFormat_AYUV
        } else {
            MFVideoFormat_NV12
        };
        let input_type = unsafe { MFCreateMediaType() }
            .map_err(|error| backend_error("create input media type", error))?;
        Self::set_common_video_attributes(&input_type, &config, &input_subtype, false)?;
        if let Err(error) = unsafe { transform.SetInputType(INPUT_STREAM_ID, &input_type, 0) } {
            if config.pixel_format == PixelFormat::Ayuv {
                return Err(EncoderError::UnsupportedConfiguration {
                    reason: "4:4:4 AYUV is not supported by this Media Foundation transform"
                        .to_string(),
                });
            }
            return Err(backend_error("set input type", error));
        }
        unsafe {
            self.output_info = Some(
                transform
                    .GetOutputStreamInfo(OUTPUT_STREAM_ID)
                    .map_err(|error| backend_error("query output stream", error))?,
            );
            transform
                .ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)
                .map_err(|error| backend_error("begin streaming", error))?;
            transform
                .ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)
                .map_err(|error| backend_error("start stream", error))?;
        }
        self.stream_id = StreamId(0);
        self.extradata = None;
        self.last_frame_duration_hns = TimeBase::from_hz(config.fps).rescale(1, MF_TIMEBASE);
        self.nominal_frame_duration_ms = nominal_frame_duration_ms(config.fps);
        self.timeline.reset();
        self.encoded_outputs = 0;
        self.config = Some(config);
        self.metrics.active_backend = Some(self.capabilities.backend_name.clone());
        self.started = true;
        Ok(())
    }

    fn set_gpu_frame_context(
        &mut self,
        context: Option<encoder_api::GpuFrameContext>,
    ) -> Result<()> {
        let active_transform = if self.started {
            Some(self.transform()?.clone())
        } else {
            None
        };
        if let Some(transform) = active_transform.as_ref() {
            self.clear_dxgi_device_manager(transform);
        }
        self.gpu_context = context.clone();
        self.gpu_device = None;
        self.gpu_converter = None;
        self.gpu_converter_input = None;
        if let Some(context) = context {
            let resource = context.session_resource().ok_or_else(|| {
                EncoderError::UnsupportedConfiguration {
                    reason: "Media Foundation GPU encoding requires a D3D11 session resource"
                        .to_string(),
                }
            })?;
            self.gpu_device = Some(Arc::downcast::<GpuDeviceContext>(resource).map_err(|_| {
                EncoderError::UnsupportedConfiguration {
                    reason: "Media Foundation GPU encoding requires a D3D11 device context"
                        .to_string(),
                }
            })?);
        }
        if let Some(transform) = active_transform {
            self.configure_dxgi_device_manager(&transform)?;
        }
        Ok(())
    }

    fn encode(&mut self, frame: VideoFrame) -> Result<Vec<EncodedPacket>> {
        if !self.started {
            return Err(EncoderError::NotConfigured);
        }
        let config = self.config.clone().ok_or(EncoderError::NotConfigured)?;
        self.stream_id = frame.stream_id;
        self.metrics.frames_submitted = self.metrics.frames_submitted.saturating_add(1);
        let fallback_pts = frame.time_base.rescale(frame.pts, MF_TIMEBASE);
        self.collect_async_events()?;
        let mut packets = if self.async_mft {
            self.drain_pending_async_outputs(fallback_pts)?
        } else {
            Vec::new()
        };
        let sample = match &frame.payload {
            FramePayload::Cpu(_) => {
                let data = self.input_bytes(&frame, &config)?;
                Some(self.make_input_sample(&data, &frame)?)
            }
            FramePayload::Gpu(_) => self.make_gpu_input_sample(&frame, &config)?,
        };
        let Some(sample) = sample else {
            self.metrics.frames_dropped = self.metrics.frames_dropped.saturating_add(1);
            return Ok(packets);
        };
        self.input_pts_queue.push_back(frame.pts);
        let transform = self.transform()?.clone();
        loop {
            let result = unsafe { transform.ProcessInput(INPUT_STREAM_ID, &sample, 0) };
            match result {
                Ok(()) => break,
                Err(error) if self.async_mft && error.code() == MF_E_NOTACCEPTING => {
                    self.async_input_ready = false;
                    packets.extend(self.wait_for_async_input(fallback_pts)?);
                }
                Err(error) if error.code() == MF_E_NOTACCEPTING => {
                    self.metrics.overload_events = self.metrics.overload_events.saturating_add(1);
                    return Err(EncoderError::Overloaded {
                        backend: self.capabilities.backend_name.clone(),
                    });
                }
                Err(error) => return Err(encode_error("process input", error)),
            }
        }
        if self.async_mft {
            self.async_input_ready = false;
        }
        packets.extend(self.drain_available_outputs(fallback_pts)?);
        Ok(packets)
    }

    fn drain(&mut self) -> Result<Vec<EncodedPacket>> {
        if !self.started {
            return Ok(Vec::new());
        }
        let transform = self.transform()?.clone();
        unsafe {
            transform
                .ProcessMessage(MFT_MESSAGE_NOTIFY_END_OF_STREAM, 0)
                .map_err(|error| encode_error("end stream", error))?;
            transform
                .ProcessMessage(MFT_MESSAGE_COMMAND_DRAIN, 0)
                .map_err(|error| encode_error("drain stream", error))?;
        }
        let mut packets = Vec::new();
        if self.async_mft {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let Some(event_type) = self.next_async_event(false)? else {
                    if Instant::now() >= deadline {
                        return Err(encode_error(
                            "drain asynchronous MFT",
                            "asynchronous MFT did not signal drain completion within 5 seconds",
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(1));
                    continue;
                };
                if event_type == METransformHaveOutput.0 as u32 {
                    self.async_pending_outputs = self.async_pending_outputs.saturating_add(1);
                    packets.extend(self.drain_pending_async_outputs(0)?);
                } else if event_type == METransformDrainComplete.0 as u32 {
                    packets.extend(self.drain_pending_async_outputs(0)?);
                    break;
                }
            }
        } else {
            loop {
                let output = self.drain_output(0)?;
                if output.is_empty() {
                    break;
                }
                packets.extend(output);
            }
        }
        unsafe {
            let _ = transform.ProcessMessage(
                windows::Win32::Media::MediaFoundation::MFT_MESSAGE_COMMAND_FLUSH,
                0,
            );
        }
        self.clear_dxgi_device_manager(&transform);
        self.started = false;
        self.gpu_context = None;
        self.gpu_device = None;
        self.gpu_converter = None;
        self.gpu_converter_input = None;
        self.async_input_ready = false;
        self.async_pending_outputs = 0;
        self.timeline.reset();
        Ok(packets)
    }

    fn codec_extradata(&self) -> Option<PacketPayload> {
        self.extradata.clone()
    }

    fn metrics(&self) -> encoder_api::EncoderMetrics {
        self.metrics.clone()
    }
}

impl Drop for MediaFoundationVideoEncoder {
    fn drop(&mut self) {
        // Engine stop calls drain first. Dropping the interface here is still
        // deterministic for failed initialization paths.
        self.transform.take();
    }
}

fn checked_frame_len(width: u32, height: u32, bytes_per_pixel: usize) -> Result<usize> {
    usize::try_from(width)
        .ok()
        .and_then(|width| usize::try_from(height).ok().map(|height| (width, height)))
        .and_then(|(width, height)| width.checked_mul(height))
        .and_then(|pixels| pixels.checked_mul(bytes_per_pixel))
        .ok_or_else(|| EncoderError::UnsupportedConfiguration {
            reason: "frame dimensions overflow host size".to_string(),
        })
}

fn clamp_byte(value: i32) -> u8 {
    value.clamp(0, 255) as u8
}

fn yuv_from_rgb(red: u8, green: u8, blue: u8) -> (u8, u8, u8) {
    let red = i32::from(red);
    let green = i32::from(green);
    let blue = i32::from(blue);
    // ITU-R BT.709 Studio Range (Y: 16..235, U/V: 16..240 centered at 128)
    let y = ((47 * red + 157 * green + 16 * blue + 128) >> 8) + 16;
    let u = ((-26 * red - 87 * green + 112 * blue + 128) >> 8) + 128;
    let v = ((112 * red - 102 * green - 10 * blue + 128) >> 8) + 128;
    (clamp_byte(y), clamp_byte(u), clamp_byte(v))
}

fn convert_bgra_to_nv12(bytes: &[u8], width: u32, height: u32, rgba: bool) -> Result<Vec<u8>> {
    let expected = checked_frame_len(width, height, 4)?;
    if bytes.len() < expected {
        return Err(encode_error(
            "convert frame",
            "BGRA/RGBA payload is truncated",
        ));
    }
    let width = width as usize;
    let height = height as usize;
    let mut output = vec![0_u8; width * height + (width * height / 2)];
    for y in 0..height {
        for x in 0..width {
            let offset = (y * width + x) * 4;
            let (red, green, blue) = if rgba {
                (bytes[offset], bytes[offset + 1], bytes[offset + 2])
            } else {
                (bytes[offset + 2], bytes[offset + 1], bytes[offset])
            };
            output[y * width + x] = yuv_from_rgb(red, green, blue).0;
        }
    }
    let chroma_offset = width * height;
    for y in (0..height).step_by(2) {
        for x in (0..width).step_by(2) {
            let mut u = 0_i32;
            let mut v = 0_i32;
            for dy in 0..2 {
                for dx in 0..2 {
                    let offset = ((y + dy) * width + x + dx) * 4;
                    let (red, green, blue) = if rgba {
                        (bytes[offset], bytes[offset + 1], bytes[offset + 2])
                    } else {
                        (bytes[offset + 2], bytes[offset + 1], bytes[offset])
                    };
                    let (_, sample_u, sample_v) = yuv_from_rgb(red, green, blue);
                    u += i32::from(sample_u);
                    v += i32::from(sample_v);
                }
            }
            let offset = chroma_offset + (y / 2) * width + x;
            output[offset] = clamp_byte((u + 2) / 4);
            output[offset + 1] = clamp_byte((v + 2) / 4);
        }
    }
    Ok(output)
}

fn convert_i420_to_nv12(bytes: &[u8], width: u32, height: u32) -> Result<Vec<u8>> {
    let width = width as usize;
    let height = height as usize;
    let y_len = width
        .checked_mul(height)
        .ok_or_else(|| encode_error("convert frame", "I420 dimensions overflow"))?;
    let plane_len = y_len / 4;
    let expected = y_len + plane_len * 2;
    if bytes.len() < expected {
        return Err(encode_error("convert frame", "I420 payload is truncated"));
    }
    let mut output = vec![0_u8; y_len + y_len / 2];
    output[..y_len].copy_from_slice(&bytes[..y_len]);
    let u_plane = &bytes[y_len..y_len + plane_len];
    let v_plane = &bytes[y_len + plane_len..expected];
    for index in 0..plane_len {
        output[y_len + index * 2] = u_plane[index];
        output[y_len + index * 2 + 1] = v_plane[index];
    }
    Ok(output)
}

fn convert_to_nv12(bytes: &[u8], format: PixelFormat, width: u32, height: u32) -> Result<Vec<u8>> {
    if width == 0 || height == 0 || !width.is_multiple_of(2) || !height.is_multiple_of(2) {
        return Err(EncoderError::UnsupportedConfiguration {
            reason: "NV12 conversion requires positive even dimensions".to_string(),
        });
    }
    match format {
        PixelFormat::Nv12 => {
            let expected = (width as usize) * (height as usize) * 3 / 2;
            if bytes.len() < expected {
                return Err(encode_error("convert frame", "NV12 payload is truncated"));
            }
            Ok(bytes[..expected].to_vec())
        }
        PixelFormat::Bgra8 => convert_bgra_to_nv12(bytes, width, height, false),
        PixelFormat::Rgba8 => convert_bgra_to_nv12(bytes, width, height, true),
        PixelFormat::I420 => convert_i420_to_nv12(bytes, width, height),
        PixelFormat::Ayuv => Err(EncoderError::UnsupportedConfiguration {
            reason: "AYUV is not supported by Media Foundation NV12 converter".to_string(),
        }),
    }
}

pub fn convert_bgra_to_ayuv(
    bytes: &[u8],
    width: u32,
    height: u32,
    is_rgba: bool,
) -> Result<Vec<u8>> {
    if width == 0 || height == 0 || !width.is_multiple_of(2) || !height.is_multiple_of(2) {
        return Err(EncoderError::UnsupportedConfiguration {
            reason: "AYUV conversion requires positive even dimensions".to_string(),
        });
    }
    let pixel_count = (width as usize) * (height as usize);
    let expected_in = pixel_count * 4;
    if bytes.len() < expected_in {
        return Err(encode_error("convert frame", "BGRA payload is truncated"));
    }
    let mut output = vec![0_u8; pixel_count * 4];
    for (i, chunk) in bytes[..expected_in].as_chunks::<4>().0.iter().enumerate() {
        let (r, g, b) = if is_rgba {
            (chunk[0] as f32, chunk[1] as f32, chunk[2] as f32)
        } else {
            (chunk[2] as f32, chunk[1] as f32, chunk[0] as f32)
        };
        let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        let u = (b - y) / 1.8556;
        let v = (r - y) / 1.5748;
        let y_lim = ((16.0 + 219.0 * (y / 255.0).clamp(0.0, 1.0)).round() as u8).clamp(16, 235);
        let u_lim = ((128.0 + 224.0 * (u / 255.0).clamp(-0.5, 0.5)).round() as u8).clamp(16, 240);
        let v_lim = ((128.0 + 224.0 * (v / 255.0).clamp(-0.5, 0.5)).round() as u8).clamp(16, 240);
        let a = chunk[3];
        // AYUV in Windows little-endian DWORD / byte stream: [V, U, Y, A]
        let out_idx = i * 4;
        output[out_idx] = v_lim;
        output[out_idx + 1] = u_lim;
        output[out_idx + 2] = y_lim;
        output[out_idx + 3] = a;
    }
    Ok(output)
}

fn convert_to_ayuv(bytes: &[u8], format: PixelFormat, width: u32, height: u32) -> Result<Vec<u8>> {
    if width == 0 || height == 0 || !width.is_multiple_of(2) || !height.is_multiple_of(2) {
        return Err(EncoderError::UnsupportedConfiguration {
            reason: "AYUV conversion requires positive even dimensions".to_string(),
        });
    }
    match format {
        PixelFormat::Ayuv => {
            let expected = (width as usize) * (height as usize) * 4;
            if bytes.len() < expected {
                return Err(encode_error("convert frame", "AYUV payload is truncated"));
            }
            Ok(bytes[..expected].to_vec())
        }
        PixelFormat::Bgra8 => convert_bgra_to_ayuv(bytes, width, height, false),
        PixelFormat::Rgba8 => convert_bgra_to_ayuv(bytes, width, height, true),
        PixelFormat::Nv12 | PixelFormat::I420 => Err(EncoderError::UnsupportedConfiguration {
            reason: format!("{format:?} to AYUV conversion is not supported"),
        }),
    }
}

fn parameter_sets_for_codec(data: &[u8], codec: &str) -> Option<Vec<u8>> {
    match encoder_api::normalize_codec(codec) {
        "hevc" => hevc_parameter_sets(data),
        "av1" => av1_sequence_header(data),
        _ => h264_parameter_sets(data),
    }
}

fn hevc_parameter_sets(data: &[u8]) -> Option<Vec<u8>> {
    let nals = h264_nal_units(data)?;
    let mut output = Vec::new();
    let mut has_vps = false;
    let mut has_sps = false;
    let mut has_pps = false;
    for nal in nals {
        let nal_type = nal.first().map(|byte| (byte >> 1) & 0x3F)?;
        if nal_type == 32 || nal_type == 33 || nal_type == 34 {
            output.extend_from_slice(&[0, 0, 0, 1]);
            output.extend_from_slice(nal);
            has_vps |= nal_type == 32;
            has_sps |= nal_type == 33;
            has_pps |= nal_type == 34;
        }
    }
    (has_vps && has_sps && has_pps).then_some(output)
}

fn encode_leb128(mut value: u64) -> Vec<u8> {
    let mut bytes = Vec::new();
    loop {
        let mut byte = (value & 0x7F) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
            bytes.push(byte);
        } else {
            bytes.push(byte);
            break;
        }
    }
    bytes
}

fn av1_sequence_header(data: &[u8]) -> Option<Vec<u8>> {
    if data.is_empty() {
        return None;
    }
    let mut cursor = 0_usize;
    while cursor < data.len() {
        let header_byte = *data.get(cursor)?;
        // Bit 7: obu_forbidden_bit (must be 0)
        if (header_byte & 0x80) != 0 {
            return None;
        }
        // Bits 6..3: obu_type
        let obu_type = (header_byte >> 3) & 0x0F;
        // Allowed types: 1..=7 or 15. Reject 0, 8 (Tile List), and 9..=14.
        if !((1..=7).contains(&obu_type) || obu_type == 15) {
            return None;
        }
        // Bit 2: obu_extension_flag
        let extension_flag = (header_byte & 0x04) != 0;
        // Bit 1: obu_has_size_field
        let has_size_field = (header_byte & 0x02) != 0;
        // Bit 0: obu_reserved_1bit (must be 0)
        if (header_byte & 0x01) != 0 {
            return None;
        }

        let header_len = if extension_flag { 2 } else { 1 };
        if cursor.checked_add(header_len)? > data.len() {
            return None;
        }

        if extension_flag {
            let ext_byte = *data.get(cursor + 1)?;
            // Extension header bits 2..0: extension_header_reserved_3bits (must be 0)
            if (ext_byte & 0x07) != 0 {
                return None;
            }
        }

        if has_size_field {
            let leb_start = cursor + header_len;
            let mut payload_size: u64 = 0;
            let mut leb_len = 0_usize;
            let mut leb_done = false;

            for i in 0..8 {
                let leb_byte = *data.get(leb_start + i)?;
                let value_bits = (leb_byte & 0x7F) as u64;
                let shifted = value_bits.checked_shl((i * 7) as u32)?;
                payload_size = payload_size.checked_add(shifted)?;
                if (leb_byte & 0x80) == 0 {
                    leb_len = i + 1;
                    leb_done = true;
                    break;
                }
            }

            if !leb_done {
                return None; // LEB128 exceeded 8 bytes
            }
            if payload_size >= (1u64 << 32) {
                return None; // payload size >= 2^32
            }
            let payload_len = usize::try_from(payload_size).ok()?;
            let payload_start = leb_start.checked_add(leb_len)?;
            let payload_end = payload_start.checked_add(payload_len)?;
            if payload_end > data.len() {
                return None; // payload extends beyond buffer
            }

            if obu_type == 1 {
                // OBU_SEQUENCE_HEADER: canonicalize into a size-delimited OBU
                let payload = &data[payload_start..payload_end];
                let leb_bytes = encode_leb128(payload_size);
                let mut obu = Vec::with_capacity(header_len + leb_bytes.len() + payload_len);
                obu.push(header_byte | 0x02);
                if extension_flag {
                    obu.push(*data.get(cursor + 1)?);
                }
                obu.extend_from_slice(&leb_bytes);
                obu.extend_from_slice(payload);
                return Some(obu);
            }

            cursor = payload_end;
        } else {
            // Unsized OBU
            if obu_type == 1 {
                // Unsized Sequence Header: valid only when it unambiguously occupies the packet remainder
                let payload_start = cursor + header_len;
                let payload = &data[payload_start..];
                let payload_len = payload.len();
                let leb_bytes = encode_leb128(payload_len as u64);
                let mut obu = Vec::with_capacity(header_len + leb_bytes.len() + payload_len);
                obu.push(header_byte | 0x02);
                if extension_flag {
                    obu.push(*data.get(cursor + 1)?);
                }
                obu.extend_from_slice(&leb_bytes);
                obu.extend_from_slice(payload);
                return Some(obu);
            }

            // An unsized non-Sequence-Header OBU means we cannot determine where it ends
            // to find any subsequent Sequence Header.
            return None;
        }
    }
    None
}

fn h264_parameter_sets(data: &[u8]) -> Option<Vec<u8>> {
    let nals = h264_nal_units(data)?;
    let mut output = Vec::new();
    let mut has_sps = false;
    let mut has_pps = false;
    for nal in nals {
        let nal_type = nal.first().map(|byte| byte & 0x1F)?;
        if nal_type == 7 || nal_type == 8 {
            output.extend_from_slice(&[0, 0, 0, 1]);
            output.extend_from_slice(nal);
            has_sps |= nal_type == 7;
            has_pps |= nal_type == 8;
        }
    }
    (has_sps && has_pps).then_some(output)
}

fn h264_nal_units(data: &[u8]) -> Option<Vec<&[u8]>> {
    annex_b_nals(data).or_else(|| avcc_nals(data))
}

fn annex_b_nals(data: &[u8]) -> Option<Vec<&[u8]>> {
    let first = find_start_code(data, 0)?;
    let mut result = Vec::new();
    let mut cursor = first;
    while let Some(code_len) = start_code_len(data, cursor) {
        let nal_start = cursor + code_len;
        let next = find_start_code(data, nal_start);
        let mut nal_end = next.unwrap_or(data.len());
        while nal_end > nal_start && data[nal_end - 1] == 0 {
            nal_end -= 1;
        }
        if nal_start < nal_end {
            result.push(&data[nal_start..nal_end]);
        }
        let Some(next) = next else { break };
        cursor = next;
    }
    (!result.is_empty()).then_some(result)
}

fn avcc_nals(data: &[u8]) -> Option<Vec<&[u8]>> {
    let mut offset = 0_usize;
    let mut result = Vec::new();
    while offset < data.len() {
        let end_of_length = offset.checked_add(4)?;
        if end_of_length > data.len() {
            return None;
        }
        let length = u32::from_be_bytes(data[offset..end_of_length].try_into().ok()?) as usize;
        if length == 0 {
            return None;
        }
        let nal_start = end_of_length;
        let nal_end = nal_start.checked_add(length)?;
        if nal_end > data.len() {
            return None;
        }
        result.push(&data[nal_start..nal_end]);
        offset = nal_end;
    }
    (!result.is_empty()).then_some(result)
}

fn find_start_code(data: &[u8], from: usize) -> Option<usize> {
    (from..data.len()).find(|index| start_code_len(data, *index).is_some())
}

fn start_code_len(data: &[u8], index: usize) -> Option<usize> {
    if index + 4 <= data.len() && data[index..index + 4] == [0, 0, 0, 1] {
        Some(4)
    } else if index + 3 <= data.len() && data[index..index + 3] == [0, 0, 1] {
        Some(3)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bgra_conversion_produces_nv12_shape() {
        let input = vec![
            0_u8, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255,
        ];
        let output = convert_to_nv12(&input, PixelFormat::Bgra8, 2, 2).expect("NV12");
        assert_eq!(output.len(), 6);
        assert!(output[..4].iter().all(|value| *value > 0));
    }

    fn solid_2x2(pixel: [u8; 4]) -> Vec<u8> {
        pixel.repeat(4)
    }

    #[test]
    fn bgra_to_ayuv_uses_bt709_limited_and_correct_packing() {
        let cases = [
            ([0, 0, 0, 255], [128, 128, 16, 255]),
            ([255, 255, 255, 255], [128, 128, 235, 255]),
            ([0, 0, 255, 255], [240, 102, 63, 255]),
            ([255, 0, 0, 255], [118, 240, 32, 255]),
        ];

        for (pixel, expected_pixel) in cases {
            let bgra = solid_2x2(pixel);
            let ayuv = convert_bgra_to_ayuv(&bgra, 2, 2, false).expect("convert BGRA to AYUV");
            assert_eq!(ayuv.len(), 16);
            for chunk in ayuv.as_chunks::<4>().0 {
                assert_eq!(chunk, &expected_pixel, "unexpected AYUV for BGRA {pixel:?}");
            }
        }

        // Test RGBA red
        let rgba_red = solid_2x2([255, 0, 0, 255]);
        let ayuv = convert_bgra_to_ayuv(&rgba_red, 2, 2, true).expect("convert RGBA to AYUV");
        for chunk in ayuv.as_chunks::<4>().0 {
            assert_eq!(chunk, &[240, 102, 63, 255]);
        }
    }

    #[test]
    fn ayuv_conversion_rejects_invalid_dimensions_and_truncated_payload() {
        assert!(convert_bgra_to_ayuv(&[0; 16], 3, 2, false).is_err());
        assert!(convert_bgra_to_ayuv(&[0; 16], 2, 3, false).is_err());
        assert!(convert_bgra_to_ayuv(&[0; 16], 0, 2, false).is_err());
        assert!(convert_bgra_to_ayuv(&[0; 12], 2, 2, false).is_err());
    }

    #[test]
    fn bgra_to_nv12_uses_bt709_limited_reference_codes() {
        let cases = [
            ([0, 0, 0, 255], [16, 16, 16, 16, 128, 128]),
            // The fixed-point BT.709 approximation produces Cb 127 for white.
            ([255, 255, 255, 255], [235, 235, 235, 235, 127, 128]),
            ([0, 0, 255, 255], [63, 63, 63, 63, 102, 240]),
            ([0, 255, 0, 255], [172, 172, 172, 172, 41, 26]),
            ([255, 0, 0, 255], [32, 32, 32, 32, 240, 118]),
        ];

        for (pixel, expected) in cases {
            let output = convert_to_nv12(&solid_2x2(pixel), PixelFormat::Bgra8, 2, 2)
                .expect("convert solid BGRA reference color");
            assert_eq!(output, expected, "unexpected NV12 for BGRA {pixel:?}");
        }
    }

    #[test]
    fn rgba_to_nv12_uses_the_same_bt709_limited_codes() {
        let red_rgba = solid_2x2([255, 0, 0, 255]);
        let output =
            convert_to_nv12(&red_rgba, PixelFormat::Rgba8, 2, 2).expect("convert solid RGBA red");
        assert_eq!(output, vec![63, 63, 63, 63, 102, 240]);
    }

    #[test]
    fn one_pixel_crosshair_pattern_has_deterministic_2x2_chroma_average() {
        // One red pixel and three black pixels model a one-pixel colored HUD
        // detail within one 4:2:0 chroma footprint.
        let input = vec![
            0, 0, 255, 255, // red
            0, 0, 0, 255, // black
            0, 0, 0, 255, // black
            0, 0, 0, 255, // black
        ];
        let output = convert_to_nv12(&input, PixelFormat::Bgra8, 2, 2)
            .expect("convert one-pixel BGRA pattern");

        assert_eq!(output, vec![63, 16, 16, 16, 122, 156]);
    }

    #[test]
    fn i420_conversion_interleaves_chroma() {
        let input = vec![16, 17, 18, 19, 100, 150];
        let output = convert_to_nv12(&input, PixelFormat::I420, 2, 2).expect("NV12");
        assert_eq!(output, vec![16, 17, 18, 19, 100, 150]);
    }

    #[test]
    fn nominal_frame_durations_for_supported_fps() {
        assert_eq!(nominal_frame_duration_ms(30), 33);
        assert_eq!(nominal_frame_duration_ms(60), 16);
        assert_eq!(nominal_frame_duration_ms(120), 8);
        assert!(nominal_frame_duration_ms(30) > 0);
        assert!(nominal_frame_duration_ms(60) > 0);
        assert!(nominal_frame_duration_ms(120) > 0);
        assert!(nominal_frame_duration_ms(0) > 0);
    }

    #[test]
    fn output_timeline_handles_variable_cadence() {
        let mut timeline = OutputTimeline::new();
        let input_pts = [0, 16, 33, 50, 100];
        let mut outputs = Vec::new();
        for pts in input_pts {
            outputs.push(timeline.advance(pts));
        }

        // DTS directly tracks raw PTS across variable capture cadence
        assert_eq!(
            outputs,
            vec![(0, 0), (16, 16), (33, 33), (50, 50), (100, 100)]
        );
        // Total elapsed DTS accurately spans full capture interval
        let first_dts = outputs.first().unwrap().1;
        let last_dts = outputs.last().unwrap().1;
        assert_eq!(last_dts - first_dts, 100);
        // DTS is monotonically nondecreasing
        for pair in outputs.windows(2) {
            assert!(pair[1].1 >= pair[0].1);
        }
    }

    #[test]
    fn output_timeline_enforces_nondecreasing_dts_on_backward_jitter() {
        let mut timeline = OutputTimeline::new();
        // Backward jitter: timestamp regresses from 33 to 32
        let input_pts = [0, 33, 32, 66];
        let mut outputs = Vec::new();
        for pts in input_pts {
            outputs.push(timeline.advance(pts));
        }

        assert_eq!(outputs, vec![(0, 0), (33, 33), (33, 33), (66, 66)]);
        // Ensure nondecreasing DTS and PTS >= DTS for every packet
        for pair in outputs.windows(2) {
            assert!(pair[1].1 >= pair[0].1, "DTS must be nondecreasing");
        }
        for (pts, dts) in &outputs {
            assert!(*pts >= *dts, "PTS must be >= DTS");
        }
    }

    #[test]
    fn output_timeline_handles_large_forward_gap() {
        let mut timeline = OutputTimeline::new();
        let input_pts = [0, 16, 5000, 5016];
        let mut outputs = Vec::new();
        for pts in input_pts {
            outputs.push(timeline.advance(pts));
        }

        assert_eq!(outputs, vec![(0, 0), (16, 16), (5000, 5000), (5016, 5016)]);
        // Large jump is reflected immediately in DTS without synthetic duration lag
        assert_eq!(outputs[2].1 - outputs[1].1, 4984);
    }

    #[test]
    fn output_timeline_resets_properly_on_reconfigure() {
        let mut timeline = OutputTimeline::new();
        timeline.advance(500);
        timeline.advance(1000);
        assert_eq!(timeline.last_dts_ms(), Some(1000));

        timeline.reset();
        assert_eq!(timeline.last_dts_ms(), None);

        // After reset, a new stream starting at 0 is not clamped to previous 1000
        let (pts, dts) = timeline.advance(0);
        assert_eq!((pts, dts), (0, 0));
    }

    #[test]
    fn output_timeline_ignores_mft_sample_duration_by_design() {
        // Output timeline only consumes raw PTS derived from sample time.
        // Bogus MFT output sample durations (e.g. 0.35ms from AMD hardware MFTs)
        // are never read or integrated, preventing replay buffer starvation.
        let mut timeline = OutputTimeline::new();
        let nominal_duration = nominal_frame_duration_ms(120);
        assert_eq!(nominal_duration, 8);

        // Simulating 10 frames at ~60 Hz actual capture cadence (16-17ms apart)
        // despite 120 FPS configuration.
        let mut packets = Vec::new();
        for i in 0..10 {
            let raw_pts_ms = i * 16;
            let (pts, dts) = timeline.advance(raw_pts_ms);
            packets.push((pts, dts, nominal_duration));
        }

        // Elapsed DTS spans actual 144ms, nominal packet duration is always 8ms
        let elapsed_dts = packets.last().unwrap().1 - packets.first().unwrap().1;
        assert_eq!(elapsed_dts, 144);
        assert!(packets.iter().all(|(_, _, dur)| *dur == 8));
    }

    #[test]
    fn input_pts_queue_preserves_capture_timestamps() {
        let mut queue = VecDeque::new();
        // 3 frames with 60 Hz timestamps (0, 16, 33)
        queue.push_back(0);
        queue.push_back(16);
        queue.push_back(33);

        let mut timeline = OutputTimeline::new();
        // Even if AMD MFT returns synthetic 0, 8, 16, the queue pops the real capture timestamps
        let p0 = queue.pop_front().unwrap_or(0);
        assert_eq!(timeline.advance(p0), (0, 0));

        let p1 = queue.pop_front().unwrap_or(8);
        assert_eq!(timeline.advance(p1), (16, 16));

        let p2 = queue.pop_front().unwrap_or(16);
        assert_eq!(timeline.advance(p2), (33, 33));

        // When queue is empty, falls back gracefully
        let p3 = queue.pop_front().unwrap_or(50);
        assert_eq!(timeline.advance(p3), (50, 50));
    }

    #[test]
    fn av1_sized_sequence_header_extraction() {
        let payload = vec![0x0A, 0x0B, 0x0C, 0x0D];
        let mut data = vec![
            // Header: obu_type=1 (Sequence Header), obu_has_size_field=1 -> (1 << 3) | 0x02 = 0x0A
            0x0A, // LEB128 size = 4
            0x04,
        ];
        data.extend_from_slice(&payload);

        let extracted = av1_sequence_header(&data).expect("extract sized sequence header");
        assert_eq!(extracted, data);
    }

    #[test]
    fn av1_temporal_delimiter_followed_by_sequence_header() {
        let seq_payload = vec![0x01, 0x02, 0x03, 0x04, 0x05];
        let mut data = vec![
            // Temporal Delimiter: obu_type=2, obu_has_size_field=1, size=0 -> (2 << 3) | 0x02 = 0x12, 0x00
            0x12, 0x00,
            // Sequence Header: obu_type=1, obu_has_size_field=1, size=5 -> (1 << 3) | 0x02 = 0x0A, 0x05
            0x0A, 0x05,
        ];
        data.extend_from_slice(&seq_payload);

        let extracted = av1_sequence_header(&data).expect("extract sequence header after TD");
        assert_eq!(extracted, vec![0x0A, 0x05, 0x01, 0x02, 0x03, 0x04, 0x05]);
    }

    #[test]
    fn av1_unsized_sequence_header_canonicalization() {
        let seq_payload = vec![0xAA, 0xBB, 0xCC];
        // Unsized Sequence Header: obu_type=1, obu_has_size_field=0 -> (1 << 3) = 0x08
        let mut data = vec![0x08];
        data.extend_from_slice(&seq_payload);

        let extracted = av1_sequence_header(&data).expect("canonicalize unsized sequence header");
        // Canonicalized output has obu_has_size_field=1 (0x0A), LEB128 size (3), and exact payload
        assert_eq!(extracted, vec![0x0A, 0x03, 0xAA, 0xBB, 0xCC]);
    }

    #[test]
    fn av1_sequence_header_byte_identical_payload() {
        let seq_payload = (0..64_u8).collect::<Vec<_>>();
        let mut data = vec![0x0A, 64];
        data.extend_from_slice(&seq_payload);

        let extracted = av1_sequence_header(&data).expect("extract exact sequence header");
        assert_eq!(&extracted[2..], seq_payload.as_slice());
    }

    #[test]
    fn av1_sequence_header_with_extension_header() {
        let seq_payload = vec![0x10, 0x20];
        // Extension flag=1, size_field=1 -> (1 << 3) | 0x04 | 0x02 = 0x0E
        // Extension header byte: temporal_id=1, spatial_id=2, reserved=0 -> (1 << 5) | (2 << 3) = 0x30
        let mut data = vec![0x0E, 0x30, 0x02];
        data.extend_from_slice(&seq_payload);

        let extracted = av1_sequence_header(&data).expect("extract with extension");
        assert_eq!(extracted, vec![0x0E, 0x30, 0x02, 0x10, 0x20]);
    }

    #[test]
    fn av1_missing_sequence_header_returns_none() {
        // Temporal Delimiter followed by Frame Header (obu_type=3) -> no Sequence Header
        let data = vec![0x12, 0x00, 0x1A, 0x02, 0x55, 0x66];
        assert_eq!(av1_sequence_header(&data), None);
        assert_eq!(av1_sequence_header(&[]), None);
    }

    #[test]
    fn av1_truncated_leb_returns_none() {
        // Size field indicates more bytes (0x80) but buffer ends
        let data = vec![0x0A, 0x80];
        assert_eq!(av1_sequence_header(&data), None);

        // Size says 10 bytes, but only 2 provided
        let data2 = vec![0x0A, 0x0A, 0x01, 0x02];
        assert_eq!(av1_sequence_header(&data2), None);
    }

    #[test]
    fn av1_oversized_leb_returns_none() {
        // LEB128 with >8 continuation bytes
        let data = vec![
            0x0A, 0x81, 0x81, 0x81, 0x81, 0x81, 0x81, 0x81, 0x81, 0x01, 0xAA,
        ];
        assert_eq!(av1_sequence_header(&data), None);
    }

    #[test]
    fn av1_reserved_bits_and_forbidden_bit_rejects() {
        // Forbidden bit set (0x80)
        let forbidden = vec![0x8A, 0x01, 0x00];
        assert_eq!(av1_sequence_header(&forbidden), None);

        // Header reserved 1-bit set (0x01)
        let reserved_header = vec![0x0B, 0x01, 0x00];
        assert_eq!(av1_sequence_header(&reserved_header), None);

        // Extension header reserved 3-bits set (0x01)
        let reserved_ext = vec![0x0E, 0x01, 0x01, 0x00];
        assert_eq!(av1_sequence_header(&reserved_ext), None);
    }

    #[test]
    fn av1_disallowed_obu_types_rejects() {
        // Type 0 (reserved)
        assert_eq!(av1_sequence_header(&[0x02, 0x00]), None);
        // Type 8 (Tile List - disallowed)
        assert_eq!(av1_sequence_header(&[0x42, 0x00]), None);
        // Types 9..=14 (reserved)
        for t in 9..=14 {
            let header = (t << 3) | 0x02;
            assert_eq!(av1_sequence_header(&[header, 0x00]), None);
        }
    }

    #[test]
    fn hevc_parameter_sets_requires_vps_sps_pps() {
        let vps_nal = [0x40, 0x01, 0x0C, 0x01]; // type 32: (0x40 >> 1) & 0x3F = 32
        let sps_nal = [0x42, 0x01, 0x01, 0x02]; // type 33: (0x42 >> 1) & 0x3F = 33
        let pps_nal = [0x44, 0x01, 0xC0]; // type 34: (0x44 >> 1) & 0x3F = 34

        // Helper to construct Annex B stream
        let make_annex_b = |nals: &[&[u8]]| -> Vec<u8> {
            let mut out = Vec::new();
            for nal in nals {
                out.extend_from_slice(&[0, 0, 0, 1]);
                out.extend_from_slice(nal);
            }
            out
        };

        // All three present -> success
        let all_three = make_annex_b(&[&vps_nal, &sps_nal, &pps_nal]);
        let extracted = hevc_parameter_sets(&all_three).expect("VPS+SPS+PPS must succeed");
        assert_eq!(extracted, all_three);

        // Missing VPS -> None
        let missing_vps = make_annex_b(&[&sps_nal, &pps_nal]);
        assert_eq!(hevc_parameter_sets(&missing_vps), None);

        // Missing SPS -> None
        let missing_sps = make_annex_b(&[&vps_nal, &pps_nal]);
        assert_eq!(hevc_parameter_sets(&missing_sps), None);

        // Missing PPS -> None
        let missing_pps = make_annex_b(&[&vps_nal, &sps_nal]);
        assert_eq!(hevc_parameter_sets(&missing_pps), None);
    }

    #[test]
    fn h264_parameter_sets_requires_sps_pps() {
        let sps_nal = [0x67, 0x42, 0x00, 0x1E]; // type 7: 0x67 & 0x1F = 7
        let pps_nal = [0x68, 0xCE, 0x38, 0x80]; // type 8: 0x68 & 0x1F = 8

        let mut all_two = vec![0, 0, 0, 1];
        all_two.extend_from_slice(&sps_nal);
        all_two.extend_from_slice(&[0, 0, 0, 1]);
        all_two.extend_from_slice(&pps_nal);

        let extracted = h264_parameter_sets(&all_two).expect("SPS+PPS must succeed");
        assert_eq!(extracted, all_two);

        // Missing SPS -> None
        let mut missing_sps = vec![0, 0, 0, 1];
        missing_sps.extend_from_slice(&pps_nal);
        assert_eq!(h264_parameter_sets(&missing_sps), None);

        // Missing PPS -> None
        let mut missing_pps = vec![0, 0, 0, 1];
        missing_pps.extend_from_slice(&sps_nal);
        assert_eq!(h264_parameter_sets(&missing_pps), None);
    }

    #[test]
    fn parameter_sets_for_codec_routes_aliases() {
        let h264_data = vec![0, 0, 0, 1, 0x67, 0x42, 0, 0, 0, 1, 0x68, 0xCE];
        for codec in ["h264", "H264", "avc", "AVC", "avc1", "AVC1"] {
            assert!(
                parameter_sets_for_codec(&h264_data, codec).is_some(),
                "failed for {codec}"
            );
        }

        let hevc_data = vec![
            0, 0, 0, 1, 0x40, 0x01, 0, 0, 0, 1, 0x42, 0x01, 0, 0, 0, 1, 0x44, 0x01,
        ];
        for codec in ["hevc", "HEVC", "h265", "H265", "hvc1", "HVC1"] {
            assert!(
                parameter_sets_for_codec(&hevc_data, codec).is_some(),
                "failed for {codec}"
            );
        }

        let av1_data = vec![0x0A, 0x02, 0x11, 0x22];
        for codec in ["av1", "AV1", "av01", "AV01"] {
            assert!(
                parameter_sets_for_codec(&av1_data, codec).is_some(),
                "failed for {codec}"
            );
        }
    }

    struct TestMfRuntime {
        com_initialized: bool,
        mf_started: bool,
    }

    impl TestMfRuntime {
        fn new() -> Self {
            let result = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            let com_initialized = result == S_OK || result == S_FALSE;
            let mf_started = unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL) }.is_ok();
            Self {
                com_initialized,
                mf_started,
            }
        }
    }

    impl Drop for TestMfRuntime {
        fn drop(&mut self) {
            if self.mf_started {
                let _ = unsafe { MFShutdown() };
            }
            if self.com_initialized {
                unsafe { CoUninitialize() };
            }
        }
    }

    fn assert_bt709_limited_attributes(media_type: &IMFMediaType) {
        assert_eq!(
            unsafe { media_type.GetUINT32(&MF_MT_YUV_MATRIX) }.ok(),
            Some(MFVideoTransferMatrix_BT709.0 as u32)
        );
        assert_eq!(
            unsafe { media_type.GetUINT32(&MF_MT_VIDEO_NOMINAL_RANGE) }.ok(),
            Some(MFNominalRange_16_235.0 as u32)
        );
        assert_eq!(
            unsafe { media_type.GetUINT32(&MF_MT_VIDEO_PRIMARIES) }.ok(),
            Some(MFVideoPrimaries_BT709.0 as u32)
        );
        assert_eq!(
            unsafe { media_type.GetUINT32(&MF_MT_TRANSFER_FUNCTION) }.ok(),
            Some(MFVideoTransFunc_709.0 as u32)
        );
        assert_eq!(
            unsafe { media_type.GetUINT32(&MF_MT_VIDEO_CHROMA_SITING) }.ok(),
            Some(MFVideoChromaSubsampling_MPEG2.0 as u32)
        );
    }

    #[test]
    fn common_video_attributes_omits_keyframe_spacing_only_for_av1() {
        let _runtime = TestMfRuntime::new();
        let config = VideoEncoderConfig {
            width: 1920,
            height: 1080,
            fps: 60,
            bitrate_kbps: Some(6_000),
            keyframe_interval_seconds: 2.0,
            ..VideoEncoderConfig::default()
        };

        // H.264: average bitrate, keyframe spacing, color matrix, nominal range, and High profile present
        let h264_type = unsafe { MFCreateMediaType() }.expect("create H.264 media type");
        MediaFoundationVideoEncoder::set_common_video_attributes(
            &h264_type,
            &config,
            &MFVideoFormat_H264,
            true,
        )
        .expect("set H.264 attributes");
        assert_bt709_limited_attributes(&h264_type);
        assert_eq!(
            unsafe { h264_type.GetUINT32(&MF_MT_AVG_BITRATE) }.ok(),
            Some(6_000_000)
        );
        assert_eq!(
            unsafe { h264_type.GetUINT32(&MF_MT_MAX_KEYFRAME_SPACING) }.ok(),
            Some(120)
        );
        assert_eq!(
            unsafe { h264_type.GetUINT32(&MF_MT_MPEG2_PROFILE) }.ok(),
            Some(eAVEncH264VProfile_High.0 as u32)
        );
        assert_eq!(
            unsafe { h264_type.GetUINT32(&MF_MT_YUV_MATRIX) }.ok(),
            Some(MFVideoTransferMatrix_BT709.0 as u32)
        );
        assert_eq!(
            unsafe { h264_type.GetUINT32(&MF_MT_VIDEO_NOMINAL_RANGE) }.ok(),
            Some(MFNominalRange_16_235.0 as u32)
        );

        // HEVC: average bitrate and keyframe spacing present, no H.264 profile
        let hevc_type = unsafe { MFCreateMediaType() }.expect("create HEVC media type");
        MediaFoundationVideoEncoder::set_common_video_attributes(
            &hevc_type,
            &config,
            &MFVideoFormat_HEVC,
            true,
        )
        .expect("set HEVC attributes");
        assert_bt709_limited_attributes(&hevc_type);
        assert_eq!(
            unsafe { hevc_type.GetUINT32(&MF_MT_AVG_BITRATE) }.ok(),
            Some(6_000_000)
        );
        assert_eq!(
            unsafe { hevc_type.GetUINT32(&MF_MT_MAX_KEYFRAME_SPACING) }.ok(),
            Some(120)
        );
        assert_eq!(
            unsafe { hevc_type.GetUINT32(&MF_MT_YUV_MATRIX) }.ok(),
            Some(MFVideoTransferMatrix_BT709.0 as u32)
        );
        assert_eq!(
            unsafe { hevc_type.GetUINT32(&MF_MT_VIDEO_NOMINAL_RANGE) }.ok(),
            Some(MFNominalRange_16_235.0 as u32)
        );
        assert!(unsafe { hevc_type.GetUINT32(&MF_MT_MPEG2_PROFILE) }.is_err());

        // AV1: average bitrate present, keyframe spacing absent, AV1 main profile present
        let av1_type = unsafe { MFCreateMediaType() }.expect("create AV1 media type");
        MediaFoundationVideoEncoder::set_common_video_attributes(
            &av1_type,
            &config,
            &MFVideoFormat_AV1,
            true,
        )
        .expect("set AV1 attributes");
        assert_bt709_limited_attributes(&av1_type);
        assert_eq!(
            unsafe { av1_type.GetUINT32(&MF_MT_AVG_BITRATE) }.ok(),
            Some(6_000_000)
        );
        assert!(
            unsafe { av1_type.GetUINT32(&MF_MT_MAX_KEYFRAME_SPACING) }.is_err(),
            "MF_MT_MAX_KEYFRAME_SPACING must be omitted for AV1"
        );
        assert_eq!(
            unsafe { av1_type.GetUINT32(&MF_MT_MPEG2_PROFILE) }.ok(),
            Some(eAVEncAV1VProfile_Main_420_8.0 as u32)
        );
        assert_eq!(
            unsafe { av1_type.GetUINT32(&MF_MT_YUV_MATRIX) }.ok(),
            Some(MFVideoTransferMatrix_BT709.0 as u32)
        );
        assert_eq!(
            unsafe { av1_type.GetUINT32(&MF_MT_VIDEO_NOMINAL_RANGE) }.ok(),
            Some(MFNominalRange_16_235.0 as u32)
        );

        // The same signaling is applied to the NV12 input media type.
        let input_type = unsafe { MFCreateMediaType() }.expect("create NV12 input media type");
        MediaFoundationVideoEncoder::set_common_video_attributes(
            &input_type,
            &config,
            &MFVideoFormat_NV12,
            false,
        )
        .expect("set NV12 input attributes");
        assert_bt709_limited_attributes(&input_type);
    }

    #[test]
    #[ignore = "requires a Windows Media Foundation H.264 transform"]
    fn media_foundation_encodes_synthetic_bgra_frames() {
        let config = VideoEncoderConfig {
            width: 64,
            height: 64,
            fps: 30,
            bitrate_kbps: Some(1_000),
            ..VideoEncoderConfig::default()
        };
        let mut encoder = new_fallback_encoder(EncoderPreference::Auto);
        encoder.configure(config).expect("H.264 MFT configuration");
        let mut encoded = Vec::new();
        for index in 0..8_i64 {
            let mut bytes = vec![0_u8; 64 * 64 * 4];
            for pixel in bytes.as_chunks_mut::<4>().0 {
                pixel.copy_from_slice(&[32, 96, (index * 16) as u8, 255]);
            }
            encoded.extend(
                encoder
                    .encode(VideoFrame {
                        stream_id: StreamId(0),
                        width: 64,
                        height: 64,
                        pixel_format: PixelFormat::Bgra8,
                        pts: index * 33,
                        time_base: TimeBase::MILLISECOND,
                        payload: FramePayload::Cpu(bytes.into()),
                    })
                    .expect("encode synthetic frame"),
            );
        }
        encoded.extend(encoder.drain().expect("drain H.264 MFT"));
        assert!(!encoded.is_empty(), "MFT must produce at least one packet");
        assert!(encoded.iter().any(|packet| packet.is_keyframe));
        assert!(encoded.iter().all(|packet| !packet.payload.is_empty()));
        assert!(
            encoder.codec_extradata().is_some(),
            "MFT must expose SPS/PPS data"
        );
        assert!(encoded
            .windows(2)
            .all(|packets| packets[1].dts >= packets[0].dts));
    }

    #[test]
    #[ignore = "requires a Windows Media Foundation H.264 transform"]
    fn media_foundation_encodes_1080p_frames() {
        let config = VideoEncoderConfig {
            width: 1_920,
            height: 1_080,
            fps: 60,
            bitrate_kbps: Some(8_000),
            ..VideoEncoderConfig::default()
        };
        let mut encoder = new_fallback_encoder(EncoderPreference::Auto);
        encoder
            .configure(config)
            .expect("1080p H.264 MFT configuration");
        let mut encoded = Vec::new();
        for index in 0..20_i64 {
            let mut bytes = vec![0_u8; 1_920 * 1_080 * 4];
            for pixel in bytes.as_chunks_mut::<4>().0 {
                pixel.copy_from_slice(&[48, 112, (index * 8) as u8, 255]);
            }
            encoded.extend(
                encoder
                    .encode(VideoFrame {
                        stream_id: StreamId(0),
                        width: 1_920,
                        height: 1_080,
                        pixel_format: PixelFormat::Bgra8,
                        pts: index * 16,
                        time_base: TimeBase::MILLISECOND,
                        payload: FramePayload::Cpu(bytes.into()),
                    })
                    .expect("encode 1080p frame"),
            );
        }
        encoded.extend(encoder.drain().expect("drain 1080p H.264 MFT"));
        assert!(!encoded.is_empty(), "1080p MFT must produce packets");
        assert!(encoded.len() >= 10, "MFT must emit most submitted frames");
        assert!(encoded.iter().any(|packet| packet.is_keyframe));
        assert!(
            encoder.codec_extradata().is_some(),
            "MFT must expose SPS/PPS data"
        );
        assert!(encoded
            .windows(2)
            .all(|packets| packets[1].dts >= packets[0].dts));
    }
}
