//! Media Foundation AAC encoder backend for the dedicated audio workers.
//!
//! The Windows AAC MFT accepts interleaved 16-bit PCM and emits raw AAC
//! access units. Synchronized Silk audio is normally interleaved F32, so the
//! conversion happens here on the worker that owns the MFT.

use std::mem::ManuallyDrop;
use std::ptr;

use encoder_api::{AudioEncoder, AudioEncoderConfig, EncoderError, Result};
use media_types::{
    AudioFrame, EncodedPacket, MediaType, PacketPayload, SampleFormat, StreamId, TimeBase,
};
use windows::core::GUID;
use windows::Win32::Foundation::{RPC_E_CHANGED_MODE, S_FALSE, S_OK};
use windows::Win32::Media::MediaFoundation::{
    IMFActivate, IMFMediaType, IMFSample, IMFTransform, MFAudioFormat_AAC, MFAudioFormat_PCM,
    MFCreateAlignedMemoryBuffer, MFCreateMediaType, MFCreateMemoryBuffer, MFCreateSample,
    MFMediaType_Audio, MFSampleExtension_CleanPoint, MFStartup, MFT_CATEGORY_AUDIO_ENCODER,
    MFT_ENUM_FLAG, MFT_ENUM_FLAG_LOCALMFT, MFT_ENUM_FLAG_SORTANDFILTER, MFT_ENUM_FLAG_SYNCMFT,
    MFT_MESSAGE_COMMAND_DRAIN, MFT_MESSAGE_COMMAND_FLUSH, MFT_MESSAGE_NOTIFY_BEGIN_STREAMING,
    MFT_MESSAGE_NOTIFY_END_OF_STREAM, MFT_MESSAGE_NOTIFY_START_OF_STREAM, MFT_OUTPUT_DATA_BUFFER,
    MFT_OUTPUT_STREAM_INFO, MFT_OUTPUT_STREAM_PROVIDES_SAMPLES, MFT_REGISTER_TYPE_INFO,
    MF_E_NOTACCEPTING, MF_E_NOT_FOUND, MF_E_TRANSFORM_NEED_MORE_INPUT,
    MF_MT_AAC_AUDIO_PROFILE_LEVEL_INDICATION, MF_MT_AAC_PAYLOAD_TYPE,
    MF_MT_AUDIO_AVG_BYTES_PER_SECOND, MF_MT_AUDIO_BITS_PER_SAMPLE, MF_MT_AUDIO_BLOCK_ALIGNMENT,
    MF_MT_AUDIO_NUM_CHANNELS, MF_MT_AUDIO_SAMPLES_PER_SECOND, MF_MT_AVG_BITRATE, MF_MT_MAJOR_TYPE,
    MF_MT_SUBTYPE, MF_VERSION,
};
use windows::Win32::System::Com::{
    CoInitializeEx, CoTaskMemFree, CoUninitialize, COINIT_MULTITHREADED,
};

const INPUT_STREAM_ID: u32 = 0;
const OUTPUT_STREAM_ID: u32 = 0;
const MF_TIMEBASE: TimeBase = TimeBase::new(1, 10_000_000);
const AAC_FRAME_SAMPLES: u32 = 1_024;
const AAC_PROFILE_LEVEL_INDICATION: u32 = 0x29;
const AAC_OUTPUT_BUFFER_FLOOR: u32 = 4_096;

fn backend_error(stage: &str, error: impl std::fmt::Display) -> EncoderError {
    EncoderError::InitializationFailed {
        backend: "media-foundation-aac".to_string(),
        details: format!("{stage}: {error}"),
    }
}

fn encode_error(stage: &str, error: impl std::fmt::Display) -> EncoderError {
    EncoderError::EncodeFailed {
        details: format!("Media Foundation AAC {stage}: {error}"),
    }
}

struct MediaFoundationRuntime {
    com_initialized: bool,
    mf_started: bool,
}

impl MediaFoundationRuntime {
    fn start() -> Result<Self> {
        let result = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if result != S_OK && result != S_FALSE && result != RPC_E_CHANGED_MODE {
            result
                .ok()
                .map_err(|error| backend_error("initialize COM", error))?;
        }

        if let Err(error) = unsafe {
            MFStartup(
                MF_VERSION,
                windows::Win32::Media::MediaFoundation::MFSTARTUP_FULL,
            )
        } {
            if result == S_OK || result == S_FALSE {
                unsafe { CoUninitialize() };
            }
            return Err(backend_error("start Media Foundation", error));
        }

        Ok(Self {
            com_initialized: result == S_OK || result == S_FALSE,
            mf_started: true,
        })
    }
}

impl Drop for MediaFoundationRuntime {
    fn drop(&mut self) {
        if self.mf_started {
            let _ = unsafe { windows::Win32::Media::MediaFoundation::MFShutdown() };
            self.mf_started = false;
        }
        if self.com_initialized {
            unsafe { CoUninitialize() };
            self.com_initialized = false;
        }
    }
}

fn activate_aac_transform() -> Result<IMFTransform> {
    let output_type = MFT_REGISTER_TYPE_INFO {
        guidMajorType: MFMediaType_Audio,
        guidSubtype: MFAudioFormat_AAC,
    };
    let flags: MFT_ENUM_FLAG =
        MFT_ENUM_FLAG_SYNCMFT | MFT_ENUM_FLAG_LOCALMFT | MFT_ENUM_FLAG_SORTANDFILTER;
    let mut raw_activations: *mut Option<IMFActivate> = ptr::null_mut();
    let mut count = 0_u32;
    let result = unsafe {
        windows::Win32::Media::MediaFoundation::MFTEnumEx(
            MFT_CATEGORY_AUDIO_ENCODER,
            flags,
            None,
            Some(&output_type),
            &mut raw_activations,
            &mut count,
        )
    };
    if let Err(error) = result {
        if !raw_activations.is_null() {
            unsafe { CoTaskMemFree(Some(raw_activations.cast())) };
        }
        if error.code() == MF_E_NOT_FOUND {
            return Err(backend_error(
                "find AAC MFT",
                "no AAC encoder was registered",
            ));
        }
        return Err(backend_error("enumerate AAC MFTs", error));
    }

    let mut selected = None;
    if !raw_activations.is_null() {
        for index in 0..count as usize {
            let activation = unsafe { ptr::read(raw_activations.add(index)) };
            if selected.is_none() {
                selected = activation;
            }
        }
        unsafe { CoTaskMemFree(Some(raw_activations.cast())) };
    }

    let activation = selected.ok_or_else(|| {
        backend_error(
            "find AAC MFT",
            "no AAC encoder was registered for raw AAC output",
        )
    })?;
    unsafe { activation.ActivateObject() }.map_err(|error| backend_error("activate AAC MFT", error))
}

fn aac_bytes_per_second(config: &AudioEncoderConfig) -> Result<u32> {
    let bytes_per_second = config.bitrate_kbps.checked_mul(125).ok_or_else(|| {
        EncoderError::UnsupportedConfiguration {
            reason: "audio bitrate is too large".to_string(),
        }
    })?;
    Ok(bytes_per_second)
}

fn validate_aac_config(config: &AudioEncoderConfig) -> Result<u32> {
    config.validate()?;
    if !matches!(config.sample_rate, 44_100 | 48_000) {
        return Err(EncoderError::UnsupportedConfiguration {
            reason: format!(
                "Media Foundation AAC supports 44100 or 48000 Hz, got {}",
                config.sample_rate
            ),
        });
    }
    if !matches!(config.channels, 1 | 2 | 6) {
        return Err(EncoderError::UnsupportedConfiguration {
            reason: format!(
                "Media Foundation AAC supports 1, 2, or 6 channels, got {}",
                config.channels
            ),
        });
    }

    let bytes_per_second = aac_bytes_per_second(config)?;
    let supported_rate = if config.channels == 6 {
        bytes_per_second.is_multiple_of(6)
            && matches!(bytes_per_second / 6, 12_000 | 16_000 | 20_000 | 24_000)
    } else {
        matches!(bytes_per_second, 12_000 | 16_000 | 20_000 | 24_000)
    };
    if !supported_rate {
        return Err(EncoderError::UnsupportedConfiguration {
            reason: format!(
                "Media Foundation AAC does not support {} kbps for {} channels",
                config.bitrate_kbps, config.channels
            ),
        });
    }
    Ok(bytes_per_second)
}

fn aac_audio_specific_config(config: &AudioEncoderConfig) -> Result<PacketPayload> {
    let _ = validate_aac_config(config)?;
    let sample_rate_index = match config.sample_rate {
        44_100 => 4_u16,
        48_000 => 3_u16,
        _ => unreachable!("validate_aac_config checked the sample rate"),
    };
    let channel_configuration = match config.channels {
        1 => 1_u16,
        2 => 2_u16,
        6 => 6_u16,
        _ => unreachable!("validate_aac_config checked the channel count"),
    };
    // ISO/IEC 14496-3 AudioSpecificConfig: AAC-LC, explicit sample-rate
    // index, and the standard channel configuration for the supported layouts.
    let bits = (2_u16 << 11) | (sample_rate_index << 7) | (channel_configuration << 3);
    Ok(PacketPayload::from(vec![(bits >> 8) as u8, bits as u8]))
}

fn sample_duration_hns(sample_count: u32, sample_rate: u32) -> i64 {
    TimeBase::from_hz(sample_rate).rescale(i64::from(sample_count), MF_TIMEBASE)
}

fn expected_audio_bytes(frame: &AudioFrame, bytes_per_sample: usize) -> Result<usize> {
    if frame.sample_count == 0 || frame.channels == 0 || frame.sample_rate == 0 {
        return Err(EncoderError::UnsupportedConfiguration {
            reason: "audio frames must contain a positive sample count, rate, and channel count"
                .to_string(),
        });
    }
    usize::try_from(frame.sample_count)
        .ok()
        .and_then(|samples| samples.checked_mul(usize::from(frame.channels)))
        .and_then(|samples| samples.checked_mul(bytes_per_sample))
        .ok_or_else(|| EncoderError::UnsupportedConfiguration {
            reason: "audio frame dimensions overflow host size".to_string(),
        })
}

fn f32_to_s16(sample: f32) -> Result<i16> {
    if !sample.is_finite() {
        return Err(encode_error(
            "convert input",
            "audio contains a non-finite sample",
        ));
    }
    let sample = sample.clamp(-1.0, 1.0);
    Ok(if sample <= -1.0 {
        i16::MIN
    } else if sample >= 1.0 {
        i16::MAX
    } else {
        (sample * 32_768.0).round() as i16
    })
}

fn pcm_input_bytes(frame: &AudioFrame, config: &AudioEncoderConfig) -> Result<Vec<u8>> {
    if frame.sample_rate != config.sample_rate {
        return Err(EncoderError::UnsupportedConfiguration {
            reason: format!(
                "audio frame rate {} does not match AAC encoder rate {}",
                frame.sample_rate, config.sample_rate
            ),
        });
    }
    if frame.channels != config.channels {
        return Err(EncoderError::UnsupportedConfiguration {
            reason: format!(
                "audio frame channels {} do not match AAC encoder channels {}",
                frame.channels, config.channels
            ),
        });
    }

    match frame.sample_format {
        SampleFormat::S16 => {
            let expected = expected_audio_bytes(frame, 2)?;
            if frame.data.len() != expected {
                return Err(encode_error(
                    "convert input",
                    format!(
                        "S16 payload has {} bytes, expected {expected}",
                        frame.data.len()
                    ),
                ));
            }
            Ok(frame.data.to_vec())
        }
        SampleFormat::F32 => {
            let expected = expected_audio_bytes(frame, 4)?;
            if frame.data.len() != expected {
                return Err(encode_error(
                    "convert input",
                    format!(
                        "F32 payload has {} bytes, expected {expected}",
                        frame.data.len()
                    ),
                ));
            }
            let mut output = Vec::with_capacity(expected / 2);
            for bytes in frame.data.as_ref().as_chunks::<4>().0 {
                let sample = f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                output.extend_from_slice(&f32_to_s16(sample)?.to_le_bytes());
            }
            Ok(output)
        }
    }
}

fn set_audio_type_attributes(
    media_type: &IMFMediaType,
    subtype: &GUID,
    config: &AudioEncoderConfig,
    output: bool,
    bytes_per_second: u32,
) -> Result<()> {
    let block_alignment = if output {
        1
    } else {
        u32::from(config.channels).checked_mul(2).ok_or_else(|| {
            EncoderError::UnsupportedConfiguration {
                reason: "audio block alignment is too large".to_string(),
            }
        })?
    };
    let pcm_bytes_per_second =
        config
            .sample_rate
            .checked_mul(block_alignment)
            .ok_or_else(|| EncoderError::UnsupportedConfiguration {
                reason: "audio sample rate is too large".to_string(),
            })?;
    let bitrate = config.bitrate_kbps.checked_mul(1_000).ok_or_else(|| {
        EncoderError::UnsupportedConfiguration {
            reason: "audio bitrate is too large".to_string(),
        }
    })?;
    unsafe {
        media_type
            .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)
            .map_err(|error| backend_error("set AAC media major type", error))?;
        media_type
            .SetGUID(&MF_MT_SUBTYPE, subtype)
            .map_err(|error| backend_error("set AAC media subtype", error))?;
        media_type
            .SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)
            .map_err(|error| backend_error("set AAC sample width", error))?;
        media_type
            .SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, config.sample_rate)
            .map_err(|error| backend_error("set AAC sample rate", error))?;
        media_type
            .SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, u32::from(config.channels))
            .map_err(|error| backend_error("set AAC channel count", error))?;
        media_type
            .SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, block_alignment)
            .map_err(|error| backend_error("set AAC block alignment", error))?;
        media_type
            .SetUINT32(
                &MF_MT_AUDIO_AVG_BYTES_PER_SECOND,
                if output {
                    bytes_per_second
                } else {
                    pcm_bytes_per_second
                },
            )
            .map_err(|error| backend_error("set AAC byte rate", error))?;
        media_type
            .SetUINT32(
                &MF_MT_AVG_BITRATE,
                if output {
                    bitrate
                } else {
                    pcm_bytes_per_second * 8
                },
            )
            .map_err(|error| backend_error("set AAC bitrate", error))?;
        if output {
            media_type
                .SetUINT32(&MF_MT_AAC_PAYLOAD_TYPE, 0)
                .map_err(|error| backend_error("set AAC raw payload type", error))?;
            media_type
                .SetUINT32(
                    &MF_MT_AAC_AUDIO_PROFILE_LEVEL_INDICATION,
                    AAC_PROFILE_LEVEL_INDICATION,
                )
                .map_err(|error| backend_error("set AAC profile", error))?;
        }
    }
    Ok(())
}

/// Construct a worker-owned native AAC encoder.
pub fn new_aac_encoder() -> Box<dyn AudioEncoder> {
    Box::new(MediaFoundationAudioEncoder::default())
}

struct MediaFoundationAudioEncoder {
    runtime: Option<MediaFoundationRuntime>,
    transform: Option<IMFTransform>,
    config: Option<AudioEncoderConfig>,
    extradata: Option<PacketPayload>,
    output_info: Option<MFT_OUTPUT_STREAM_INFO>,
    stream_id: StreamId,
    default_output_duration_hns: i64,
    last_output_pts_ms: Option<i64>,
    started: bool,
}

// The MFT is created, configured, and consumed by one audio worker. No MF
// interface is sent through a media channel or accessed concurrently.
unsafe impl Send for MediaFoundationAudioEncoder {}

impl Default for MediaFoundationAudioEncoder {
    fn default() -> Self {
        Self {
            runtime: None,
            transform: None,
            config: None,
            extradata: None,
            output_info: None,
            stream_id: StreamId(0),
            default_output_duration_hns: 0,
            last_output_pts_ms: None,
            started: false,
        }
    }
}

impl MediaFoundationAudioEncoder {
    fn transform(&self) -> Result<&IMFTransform> {
        self.transform.as_ref().ok_or(EncoderError::NotConfigured)
    }

    fn discard_session(&mut self) {
        self.started = false;
        self.output_info = None;
        self.config = None;
        self.extradata = None;
        self.last_output_pts_ms = None;
        let _ = self.transform.take();
        let _ = self.runtime.take();
    }

    fn output_sample(&self) -> Result<(MFT_OUTPUT_DATA_BUFFER, Option<IMFSample>)> {
        let info = self.output_info.ok_or(EncoderError::NotConfigured)?;
        let provides_samples = (info.dwFlags & MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 as u32) != 0;
        let supplied = if provides_samples {
            None
        } else {
            let size = info.cbSize.max(AAC_OUTPUT_BUFFER_FLOOR);
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
        Ok((
            MFT_OUTPUT_DATA_BUFFER {
                dwStreamID: OUTPUT_STREAM_ID,
                pSample: ManuallyDrop::new(supplied.clone()),
                dwStatus: 0,
                pEvents: ManuallyDrop::new(None),
            },
            supplied,
        ))
    }

    fn drain_output(&mut self, fallback_pts_hns: i64) -> Result<Vec<EncodedPacket>> {
        let transform = self.transform()?.clone();
        let (mut output, _supplied) = self.output_sample()?;
        let mut status = 0_u32;
        let process =
            unsafe { transform.ProcessOutput(0, std::slice::from_mut(&mut output), &mut status) };
        let (sample, _events) = unsafe {
            (
                ManuallyDrop::take(&mut output.pSample),
                ManuallyDrop::take(&mut output.pEvents),
            )
        };
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
        let bytes = unsafe {
            buffer
                .Lock(&mut data, None, Some(&mut current_length))
                .map_err(|error| encode_error("lock output buffer", error))?;
            let bytes = if data.is_null() || current_length == 0 {
                Vec::new()
            } else {
                std::slice::from_raw_parts(data, current_length as usize).to_vec()
            };
            buffer
                .Unlock()
                .map_err(|error| encode_error("unlock output buffer", error))?;
            bytes
        };
        if bytes.is_empty() {
            return Ok(Vec::new());
        }

        let sample_time = unsafe { sample.GetSampleTime().unwrap_or(fallback_pts_hns) };
        let sample_duration = unsafe {
            sample
                .GetSampleDuration()
                .unwrap_or(self.default_output_duration_hns)
                .max(1)
        };
        let mut pts = MF_TIMEBASE.rescale(sample_time, TimeBase::MILLISECOND);
        if let Some(previous) = self.last_output_pts_ms {
            pts = pts.max(previous.saturating_add(1));
        }
        self.last_output_pts_ms = Some(pts);
        let duration = MF_TIMEBASE
            .rescale(sample_duration, TimeBase::MILLISECOND)
            .max(1);
        let is_keyframe =
            unsafe { sample.GetUINT32(&MFSampleExtension_CleanPoint).unwrap_or(1) != 0 };
        Ok(vec![EncodedPacket {
            stream_id: self.stream_id,
            media_type: MediaType::Audio,
            pts,
            dts: pts,
            duration,
            time_base: TimeBase::MILLISECOND,
            is_keyframe,
            sequence: 0,
            payload: PacketPayload::from(bytes),
        }])
    }

    fn drain_available_outputs(&mut self, fallback_pts_hns: i64) -> Result<Vec<EncodedPacket>> {
        let mut packets = Vec::new();
        loop {
            let output = self.drain_output(fallback_pts_hns)?;
            if output.is_empty() {
                break;
            }
            packets.extend(output);
        }
        Ok(packets)
    }
}

impl AudioEncoder for MediaFoundationAudioEncoder {
    fn configure(&mut self, config: AudioEncoderConfig) -> Result<()> {
        if self.started {
            return Err(EncoderError::UnsupportedConfiguration {
                reason: "AAC encoder cannot be reconfigured while streaming".to_string(),
            });
        }
        let bytes_per_second = validate_aac_config(&config)?;
        let extradata = aac_audio_specific_config(&config)?;
        self.discard_session();

        let runtime = MediaFoundationRuntime::start()?;
        let transform = activate_aac_transform()?;
        let output_type = unsafe { MFCreateMediaType() }
            .map_err(|error| backend_error("create AAC output media type", error))?;
        set_audio_type_attributes(
            &output_type,
            &MFAudioFormat_AAC,
            &config,
            true,
            bytes_per_second,
        )?;
        unsafe {
            transform
                .SetOutputType(OUTPUT_STREAM_ID, &output_type, 0)
                .map_err(|error| backend_error("set AAC output type", error))?;
        }

        let input_type = unsafe { MFCreateMediaType() }
            .map_err(|error| backend_error("create AAC input media type", error))?;
        set_audio_type_attributes(
            &input_type,
            &MFAudioFormat_PCM,
            &config,
            false,
            bytes_per_second,
        )?;
        unsafe {
            transform
                .SetInputType(INPUT_STREAM_ID, &input_type, 0)
                .map_err(|error| backend_error("set AAC input type", error))?;
        }

        let output_info = unsafe { transform.GetOutputStreamInfo(OUTPUT_STREAM_ID) }
            .map_err(|error| backend_error("query AAC output stream", error))?;
        unsafe {
            transform
                .ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)
                .map_err(|error| backend_error("begin AAC streaming", error))?;
            transform
                .ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)
                .map_err(|error| backend_error("start AAC stream", error))?;
        }

        self.runtime = Some(runtime);
        self.transform = Some(transform);
        self.config = Some(config.clone());
        self.extradata = Some(extradata);
        self.output_info = Some(output_info);
        self.stream_id = StreamId(0);
        self.default_output_duration_hns =
            sample_duration_hns(AAC_FRAME_SAMPLES, config.sample_rate);
        self.last_output_pts_ms = None;
        self.started = true;
        Ok(())
    }

    fn encode(&mut self, frame: AudioFrame) -> Result<Vec<EncodedPacket>> {
        if !self.started {
            return Err(EncoderError::NotConfigured);
        }
        let config = self.config.clone().ok_or(EncoderError::NotConfigured)?;
        let data = pcm_input_bytes(&frame, &config)?;
        let duration_hns = sample_duration_hns(frame.sample_count, config.sample_rate);
        if duration_hns <= 0 {
            return Err(encode_error(
                "create input sample",
                "audio duration is zero",
            ));
        }
        self.stream_id = frame.stream_id;
        let sample = {
            let sample = unsafe { MFCreateSample() }
                .map_err(|error| encode_error("create input sample", error))?;
            let length = u32::try_from(data.len())
                .map_err(|_| encode_error("create input buffer", "audio frame is too large"))?;
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
                sample
                    .AddBuffer(&buffer)
                    .map_err(|error| encode_error("attach input buffer", error))?;
                sample
                    .SetSampleTime(frame.time_base.rescale(frame.pts, MF_TIMEBASE))
                    .map_err(|error| encode_error("set input timestamp", error))?;
                sample
                    .SetSampleDuration(duration_hns)
                    .map_err(|error| encode_error("set input duration", error))?;
            }
            sample
        };

        let process = unsafe { self.transform()?.ProcessInput(INPUT_STREAM_ID, &sample, 0) };
        if let Err(error) = process {
            if error.code() == MF_E_NOTACCEPTING {
                return Err(EncoderError::Overloaded {
                    backend: "media-foundation-aac".to_string(),
                });
            }
            return Err(encode_error("process input", error));
        }
        self.drain_available_outputs(frame.time_base.rescale(frame.pts, MF_TIMEBASE))
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
        let packets = self.drain_available_outputs(0)?;
        unsafe {
            let _ = transform.ProcessMessage(MFT_MESSAGE_COMMAND_FLUSH, 0);
        }
        self.started = false;
        Ok(packets)
    }

    fn codec_extradata(&self) -> Option<PacketPayload> {
        self.extradata.clone()
    }
}

impl Drop for MediaFoundationAudioEncoder {
    fn drop(&mut self) {
        // Release the transform before MFShutdown on all paths.
        let _ = self.transform.take();
        let _ = self.runtime.take();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame_with_f32(samples: &[f32], sample_count: u32) -> AudioFrame {
        let mut data = Vec::with_capacity(samples.len() * 4);
        for sample in samples {
            data.extend_from_slice(&sample.to_le_bytes());
        }
        AudioFrame {
            stream_id: StreamId(4),
            sample_format: SampleFormat::F32,
            sample_rate: 48_000,
            channels: 2,
            sample_count,
            pts: 0,
            time_base: TimeBase::from_hz(48_000),
            data: data.into(),
        }
    }

    #[test]
    fn accepts_default_aac_configuration() {
        assert!(matches!(
            validate_aac_config(&AudioEncoderConfig::default()),
            Ok(24_000)
        ));
    }

    #[test]
    fn rejects_unsupported_aac_rate_channels_and_bitrate() {
        let mut config = AudioEncoderConfig {
            sample_rate: 32_000,
            ..AudioEncoderConfig::default()
        };
        assert!(matches!(
            validate_aac_config(&config),
            Err(EncoderError::UnsupportedConfiguration { .. })
        ));
        config.sample_rate = 48_000;
        config.channels = 4;
        assert!(matches!(
            validate_aac_config(&config),
            Err(EncoderError::UnsupportedConfiguration { .. })
        ));
        config.channels = 2;
        config.bitrate_kbps = 200;
        assert!(matches!(
            validate_aac_config(&config),
            Err(EncoderError::UnsupportedConfiguration { .. })
        ));
    }

    #[test]
    fn converts_f32_samples_to_little_endian_s16() {
        let frame = frame_with_f32(&[-1.0, -0.5, 0.0, 0.5, 1.0, 0.25], 3);
        let output = pcm_input_bytes(&frame, &AudioEncoderConfig::default()).expect("PCM");
        let values: Vec<i16> = output
            .as_chunks::<2>()
            .0
            .iter()
            .map(|bytes| i16::from_le_bytes(*bytes))
            .collect();
        assert_eq!(values, vec![i16::MIN, -16_384, 0, 16_384, i16::MAX, 8_192]);
    }

    #[test]
    fn builds_aac_lc_audio_specific_config() {
        let stereo_48k = aac_audio_specific_config(&AudioEncoderConfig::default()).expect("ASC");
        assert_eq!(stereo_48k.as_slice(), &[0x11, 0x90]);

        let mono_44k = aac_audio_specific_config(&AudioEncoderConfig {
            bitrate_kbps: 96,
            sample_rate: 44_100,
            channels: 1,
        })
        .expect("ASC");
        assert_eq!(mono_44k.as_slice(), &[0x12, 0x08]);
    }

    #[test]
    fn rejects_truncated_or_non_finite_audio() {
        let mut frame = frame_with_f32(&[0.0, 0.0], 1);
        frame.data = std::sync::Arc::from([0_u8; 4]);
        assert!(matches!(
            pcm_input_bytes(&frame, &AudioEncoderConfig::default()),
            Err(EncoderError::EncodeFailed { .. })
        ));

        let frame = frame_with_f32(&[f32::NAN, 0.0], 1);
        assert!(matches!(
            pcm_input_bytes(&frame, &AudioEncoderConfig::default()),
            Err(EncoderError::EncodeFailed { .. })
        ));
    }

    #[test]
    #[ignore = "requires the Windows Media Foundation AAC MFT"]
    fn media_foundation_encodes_synthetic_audio() {
        let mut encoder = new_aac_encoder();
        encoder
            .configure(AudioEncoderConfig::default())
            .expect("AAC MFT configuration");

        let mut encoded = Vec::new();
        for index in 0..8_i64 {
            let sample_count = 1_024;
            let mut samples = Vec::with_capacity(sample_count as usize * 2);
            for sample in 0..sample_count {
                let value = (((sample + index as u32) % 32) as f32 / 32.0 - 0.5) * 0.5;
                samples.extend([value, -value]);
            }
            let mut frame = frame_with_f32(&samples, sample_count);
            frame.pts = index * i64::from(sample_count);
            encoded.extend(encoder.encode(frame).expect("encode AAC frame"));
        }
        encoded.extend(encoder.drain().expect("drain AAC MFT"));

        assert!(!encoded.is_empty(), "AAC MFT must produce packets");
        assert!(encoded.iter().all(|packet| {
            packet.media_type == MediaType::Audio
                && packet.duration > 0
                && !packet.payload.is_empty()
        }));
        assert!(encoded
            .windows(2)
            .all(|packets| packets[1].dts >= packets[0].dts));
    }
}
