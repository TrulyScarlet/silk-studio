use serde::Serialize;
use std::borrow::Cow;
use std::convert::TryFrom;
use std::fmt;

use crate::mp4box::*;
use crate::*;

pub use bytes::Bytes;
pub use num_rational::Ratio;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct FixedPointU8(Ratio<u16>);

impl FixedPointU8 {
    pub fn new(val: u8) -> Self {
        Self(Ratio::new_raw(val as u16 * 0x100, 0x100))
    }

    pub fn new_raw(val: u16) -> Self {
        Self(Ratio::new_raw(val, 0x100))
    }

    pub fn value(&self) -> u8 {
        self.0.to_integer() as u8
    }

    pub fn raw_value(&self) -> u16 {
        *self.0.numer()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct FixedPointI8(Ratio<i16>);

impl FixedPointI8 {
    pub fn new(val: i8) -> Self {
        Self(Ratio::new_raw(val as i16 * 0x100, 0x100))
    }

    pub fn new_raw(val: i16) -> Self {
        Self(Ratio::new_raw(val, 0x100))
    }

    pub fn value(&self) -> i8 {
        self.0.to_integer() as i8
    }

    pub fn raw_value(&self) -> i16 {
        *self.0.numer()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct FixedPointU16(Ratio<u32>);

impl FixedPointU16 {
    pub fn new(val: u16) -> Self {
        Self(Ratio::new_raw(val as u32 * 0x10000, 0x10000))
    }

    pub fn new_raw(val: u32) -> Self {
        Self(Ratio::new_raw(val, 0x10000))
    }

    pub fn value(&self) -> u16 {
        self.0.to_integer() as u16
    }

    pub fn raw_value(&self) -> u32 {
        *self.0.numer()
    }
}

impl fmt::Debug for BoxType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let fourcc: FourCC = From::from(*self);
        write!(f, "{fourcc}")
    }
}

impl fmt::Display for BoxType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let fourcc: FourCC = From::from(*self);
        write!(f, "{fourcc}")
    }
}

#[derive(Default, PartialEq, Eq, Clone, Copy, Serialize)]
pub struct FourCC {
    pub value: [u8; 4],
}

impl std::str::FromStr for FourCC {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        if let [a, b, c, d] = s.as_bytes() {
            Ok(Self {
                value: [*a, *b, *c, *d],
            })
        } else {
            Err(Error::InvalidData("expected exactly four bytes in string"))
        }
    }
}

impl From<u32> for FourCC {
    fn from(number: u32) -> Self {
        FourCC {
            value: number.to_be_bytes(),
        }
    }
}

impl From<FourCC> for u32 {
    fn from(fourcc: FourCC) -> u32 {
        (&fourcc).into()
    }
}

impl From<&FourCC> for u32 {
    fn from(fourcc: &FourCC) -> u32 {
        u32::from_be_bytes(fourcc.value)
    }
}

impl From<[u8; 4]> for FourCC {
    fn from(value: [u8; 4]) -> FourCC {
        FourCC { value }
    }
}

impl From<BoxType> for FourCC {
    fn from(t: BoxType) -> FourCC {
        let box_num: u32 = Into::into(t);
        From::from(box_num)
    }
}

impl fmt::Debug for FourCC {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let code: u32 = self.into();
        let string = String::from_utf8_lossy(&self.value[..]);
        write!(f, "{string} / {code:#010X}")
    }
}

impl fmt::Display for FourCC {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", String::from_utf8_lossy(&self.value[..]))
    }
}

const DISPLAY_TYPE_VIDEO: &str = "Video";
const DISPLAY_TYPE_AUDIO: &str = "Audio";
const DISPLAY_TYPE_SUBTITLE: &str = "Subtitle";

const HANDLER_TYPE_VIDEO: &str = "vide";
const HANDLER_TYPE_VIDEO_FOURCC: [u8; 4] = *b"vide";

const HANDLER_TYPE_AUDIO: &str = "soun";
const HANDLER_TYPE_AUDIO_FOURCC: [u8; 4] = *b"soun";

const HANDLER_TYPE_SUBTITLE: &str = "sbtl";
const HANDLER_TYPE_SUBTITLE_FOURCC: [u8; 4] = *b"sbtl";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackType {
    Video,
    Audio,
    Subtitle,
}

impl fmt::Display for TrackType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let s = match self {
            TrackType::Video => DISPLAY_TYPE_VIDEO,
            TrackType::Audio => DISPLAY_TYPE_AUDIO,
            TrackType::Subtitle => DISPLAY_TYPE_SUBTITLE,
        };
        write!(f, "{s}")
    }
}

impl TryFrom<&str> for TrackType {
    type Error = Error;
    fn try_from(handler: &str) -> Result<TrackType> {
        match handler {
            HANDLER_TYPE_VIDEO => Ok(TrackType::Video),
            HANDLER_TYPE_AUDIO => Ok(TrackType::Audio),
            HANDLER_TYPE_SUBTITLE => Ok(TrackType::Subtitle),
            _ => Err(Error::InvalidData("unsupported handler type")),
        }
    }
}

impl TryFrom<&FourCC> for TrackType {
    type Error = Error;
    fn try_from(fourcc: &FourCC) -> Result<TrackType> {
        match fourcc.value {
            HANDLER_TYPE_VIDEO_FOURCC => Ok(TrackType::Video),
            HANDLER_TYPE_AUDIO_FOURCC => Ok(TrackType::Audio),
            HANDLER_TYPE_SUBTITLE_FOURCC => Ok(TrackType::Subtitle),
            _ => Err(Error::InvalidData("unsupported handler type")),
        }
    }
}

impl From<TrackType> for FourCC {
    fn from(t: TrackType) -> FourCC {
        match t {
            TrackType::Video => HANDLER_TYPE_VIDEO_FOURCC.into(),
            TrackType::Audio => HANDLER_TYPE_AUDIO_FOURCC.into(),
            TrackType::Subtitle => HANDLER_TYPE_SUBTITLE_FOURCC.into(),
        }
    }
}

const MEDIA_TYPE_H264: &str = "h264";
const MEDIA_TYPE_H265: &str = "h265";
const MEDIA_TYPE_AV1: &str = "av1";
const MEDIA_TYPE_VP9: &str = "vp9";
const MEDIA_TYPE_AAC: &str = "aac";
const MEDIA_TYPE_TTXT: &str = "ttxt";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaType {
    H264,
    H265,
    AV1,
    VP9,
    AAC,
    TTXT,
}

impl fmt::Display for MediaType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let s: &str = self.into();
        write!(f, "{s}")
    }
}

impl TryFrom<&str> for MediaType {
    type Error = Error;
    fn try_from(media: &str) -> Result<MediaType> {
        match media {
            MEDIA_TYPE_H264 => Ok(MediaType::H264),
            MEDIA_TYPE_H265 => Ok(MediaType::H265),
            MEDIA_TYPE_AV1 => Ok(MediaType::AV1),
            MEDIA_TYPE_VP9 => Ok(MediaType::VP9),
            MEDIA_TYPE_AAC => Ok(MediaType::AAC),
            MEDIA_TYPE_TTXT => Ok(MediaType::TTXT),
            _ => Err(Error::InvalidData("unsupported media type")),
        }
    }
}

impl From<MediaType> for &str {
    fn from(t: MediaType) -> &'static str {
        match t {
            MediaType::H264 => MEDIA_TYPE_H264,
            MediaType::H265 => MEDIA_TYPE_H265,
            MediaType::AV1 => MEDIA_TYPE_AV1,
            MediaType::VP9 => MEDIA_TYPE_VP9,
            MediaType::AAC => MEDIA_TYPE_AAC,
            MediaType::TTXT => MEDIA_TYPE_TTXT,
        }
    }
}

impl From<&MediaType> for &str {
    fn from(t: &MediaType) -> &'static str {
        match t {
            MediaType::H264 => MEDIA_TYPE_H264,
            MediaType::H265 => MEDIA_TYPE_H265,
            MediaType::AV1 => MEDIA_TYPE_AV1,
            MediaType::VP9 => MEDIA_TYPE_VP9,
            MediaType::AAC => MEDIA_TYPE_AAC,
            MediaType::TTXT => MEDIA_TYPE_TTXT,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum AvcProfile {
    AvcConstrainedBaseline, // 66 with constraint set 1
    AvcBaseline,            // 66,
    AvcMain,                // 77,
    AvcExtended,            // 88,
    AvcHigh,                // 100
                            // TODO Progressive High Profile, Constrained High Profile, ...
}

impl TryFrom<(u8, u8)> for AvcProfile {
    type Error = Error;
    fn try_from(value: (u8, u8)) -> Result<AvcProfile> {
        let profile = value.0;
        let constraint_set1_flag = (value.1 & 0x40) >> 7;
        match (profile, constraint_set1_flag) {
            (66, 1) => Ok(AvcProfile::AvcConstrainedBaseline),
            (66, 0) => Ok(AvcProfile::AvcBaseline),
            (77, _) => Ok(AvcProfile::AvcMain),
            (88, _) => Ok(AvcProfile::AvcExtended),
            (100, _) => Ok(AvcProfile::AvcHigh),
            _ => Err(Error::InvalidData("unsupported avc profile")),
        }
    }
}

impl fmt::Display for AvcProfile {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let profile = match self {
            AvcProfile::AvcConstrainedBaseline => "Constrained Baseline",
            AvcProfile::AvcBaseline => "Baseline",
            AvcProfile::AvcMain => "Main",
            AvcProfile::AvcExtended => "Extended",
            AvcProfile::AvcHigh => "High",
        };
        write!(f, "{profile}")
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum AudioObjectType {
    AacMain = 1,                                       // AAC Main Profile
    AacLowComplexity = 2,                              // AAC Low Complexity
    AacScalableSampleRate = 3,                         // AAC Scalable Sample Rate
    AacLongTermPrediction = 4,                         // AAC Long Term Predictor
    SpectralBandReplication = 5,                       // Spectral band Replication
    AACScalable = 6,                                   // AAC Scalable
    TwinVQ = 7,                                        // Twin VQ
    CodeExcitedLinearPrediction = 8,                   // CELP
    HarmonicVectorExcitationCoding = 9,                // HVXC
    TextToSpeechtInterface = 12,                       // TTSI
    MainSynthetic = 13,                                // Main Synthetic
    WavetableSynthesis = 14,                           // Wavetable Synthesis
    GeneralMIDI = 15,                                  // General MIDI
    AlgorithmicSynthesis = 16,                         // Algorithmic Synthesis
    ErrorResilientAacLowComplexity = 17,               // ER AAC LC
    ErrorResilientAacLongTermPrediction = 19,          // ER AAC LTP
    ErrorResilientAacScalable = 20,                    // ER AAC Scalable
    ErrorResilientAacTwinVQ = 21,                      // ER AAC TwinVQ
    ErrorResilientAacBitSlicedArithmeticCoding = 22,   // ER Bit Sliced Arithmetic Coding
    ErrorResilientAacLowDelay = 23,                    // ER AAC Low Delay
    ErrorResilientCodeExcitedLinearPrediction = 24,    // ER CELP
    ErrorResilientHarmonicVectorExcitationCoding = 25, // ER HVXC
    ErrorResilientHarmonicIndividualLinesNoise = 26,   // ER HILN
    ErrorResilientParametric = 27,                     // ER Parametric
    SinuSoidalCoding = 28,                             // SSC
    ParametricStereo = 29,                             // PS
    MpegSurround = 30,                                 // MPEG Surround
    MpegLayer1 = 32,                                   // MPEG Layer 1
    MpegLayer2 = 33,                                   // MPEG Layer 2
    MpegLayer3 = 34,                                   // MPEG Layer 3
    DirectStreamTransfer = 35,                         // DST Direct Stream Transfer
    AudioLosslessCoding = 36,                          // ALS Audio Lossless Coding
    ScalableLosslessCoding = 37,                       // SLC Scalable Lossless Coding
    ScalableLosslessCodingNoneCore = 38,               // SLC non-core
    ErrorResilientAacEnhancedLowDelay = 39,            // ER AAC ELD
    SymbolicMusicRepresentationSimple = 40,            // SMR Simple
    SymbolicMusicRepresentationMain = 41,              // SMR Main
    UnifiedSpeechAudioCoding = 42,                     // USAC
    SpatialAudioObjectCoding = 43,                     // SAOC
    LowDelayMpegSurround = 44,                         // LD MPEG Surround
    SpatialAudioObjectCodingDialogueEnhancement = 45,  // SAOC-DE
    AudioSync = 46,                                    // Audio Sync
}

impl TryFrom<u8> for AudioObjectType {
    type Error = Error;
    fn try_from(value: u8) -> Result<AudioObjectType> {
        match value {
            1 => Ok(AudioObjectType::AacMain),
            2 => Ok(AudioObjectType::AacLowComplexity),
            3 => Ok(AudioObjectType::AacScalableSampleRate),
            4 => Ok(AudioObjectType::AacLongTermPrediction),
            5 => Ok(AudioObjectType::SpectralBandReplication),
            6 => Ok(AudioObjectType::AACScalable),
            7 => Ok(AudioObjectType::TwinVQ),
            8 => Ok(AudioObjectType::CodeExcitedLinearPrediction),
            9 => Ok(AudioObjectType::HarmonicVectorExcitationCoding),
            12 => Ok(AudioObjectType::TextToSpeechtInterface),
            13 => Ok(AudioObjectType::MainSynthetic),
            14 => Ok(AudioObjectType::WavetableSynthesis),
            15 => Ok(AudioObjectType::GeneralMIDI),
            16 => Ok(AudioObjectType::AlgorithmicSynthesis),
            17 => Ok(AudioObjectType::ErrorResilientAacLowComplexity),
            19 => Ok(AudioObjectType::ErrorResilientAacLongTermPrediction),
            20 => Ok(AudioObjectType::ErrorResilientAacScalable),
            21 => Ok(AudioObjectType::ErrorResilientAacTwinVQ),
            22 => Ok(AudioObjectType::ErrorResilientAacBitSlicedArithmeticCoding),
            23 => Ok(AudioObjectType::ErrorResilientAacLowDelay),
            24 => Ok(AudioObjectType::ErrorResilientCodeExcitedLinearPrediction),
            25 => Ok(AudioObjectType::ErrorResilientHarmonicVectorExcitationCoding),
            26 => Ok(AudioObjectType::ErrorResilientHarmonicIndividualLinesNoise),
            27 => Ok(AudioObjectType::ErrorResilientParametric),
            28 => Ok(AudioObjectType::SinuSoidalCoding),
            29 => Ok(AudioObjectType::ParametricStereo),
            30 => Ok(AudioObjectType::MpegSurround),
            32 => Ok(AudioObjectType::MpegLayer1),
            33 => Ok(AudioObjectType::MpegLayer2),
            34 => Ok(AudioObjectType::MpegLayer3),
            35 => Ok(AudioObjectType::DirectStreamTransfer),
            36 => Ok(AudioObjectType::AudioLosslessCoding),
            37 => Ok(AudioObjectType::ScalableLosslessCoding),
            38 => Ok(AudioObjectType::ScalableLosslessCodingNoneCore),
            39 => Ok(AudioObjectType::ErrorResilientAacEnhancedLowDelay),
            40 => Ok(AudioObjectType::SymbolicMusicRepresentationSimple),
            41 => Ok(AudioObjectType::SymbolicMusicRepresentationMain),
            42 => Ok(AudioObjectType::UnifiedSpeechAudioCoding),
            43 => Ok(AudioObjectType::SpatialAudioObjectCoding),
            44 => Ok(AudioObjectType::LowDelayMpegSurround),
            45 => Ok(AudioObjectType::SpatialAudioObjectCodingDialogueEnhancement),
            46 => Ok(AudioObjectType::AudioSync),
            _ => Err(Error::InvalidData("invalid audio object type")),
        }
    }
}

impl fmt::Display for AudioObjectType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let type_str = match self {
            AudioObjectType::AacMain => "AAC Main",
            AudioObjectType::AacLowComplexity => "LC",
            AudioObjectType::AacScalableSampleRate => "SSR",
            AudioObjectType::AacLongTermPrediction => "LTP",
            AudioObjectType::SpectralBandReplication => "SBR",
            AudioObjectType::AACScalable => "Scalable",
            AudioObjectType::TwinVQ => "TwinVQ",
            AudioObjectType::CodeExcitedLinearPrediction => "CELP",
            AudioObjectType::HarmonicVectorExcitationCoding => "HVXC",
            AudioObjectType::TextToSpeechtInterface => "TTSI",
            AudioObjectType::MainSynthetic => "Main Synthetic",
            AudioObjectType::WavetableSynthesis => "Wavetable Synthesis",
            AudioObjectType::GeneralMIDI => "General MIDI",
            AudioObjectType::AlgorithmicSynthesis => "Algorithmic Synthesis",
            AudioObjectType::ErrorResilientAacLowComplexity => "ER AAC LC",
            AudioObjectType::ErrorResilientAacLongTermPrediction => "ER AAC LTP",
            AudioObjectType::ErrorResilientAacScalable => "ER AAC scalable",
            AudioObjectType::ErrorResilientAacTwinVQ => "ER AAC TwinVQ",
            AudioObjectType::ErrorResilientAacBitSlicedArithmeticCoding => "ER AAC BSAC",
            AudioObjectType::ErrorResilientAacLowDelay => "ER AAC LD",
            AudioObjectType::ErrorResilientCodeExcitedLinearPrediction => "ER CELP",
            AudioObjectType::ErrorResilientHarmonicVectorExcitationCoding => "ER HVXC",
            AudioObjectType::ErrorResilientHarmonicIndividualLinesNoise => "ER HILN",
            AudioObjectType::ErrorResilientParametric => "ER Parametric",
            AudioObjectType::SinuSoidalCoding => "SSC",
            AudioObjectType::ParametricStereo => "Parametric Stereo",
            AudioObjectType::MpegSurround => "MPEG surround",
            AudioObjectType::MpegLayer1 => "MPEG Layer 1",
            AudioObjectType::MpegLayer2 => "MPEG Layer 2",
            AudioObjectType::MpegLayer3 => "MPEG Layer 3",
            AudioObjectType::DirectStreamTransfer => "DST",
            AudioObjectType::AudioLosslessCoding => "ALS",
            AudioObjectType::ScalableLosslessCoding => "SLS",
            AudioObjectType::ScalableLosslessCodingNoneCore => "SLS Non-core",
            AudioObjectType::ErrorResilientAacEnhancedLowDelay => "ER AAC ELD",
            AudioObjectType::SymbolicMusicRepresentationSimple => "SMR Simple",
            AudioObjectType::SymbolicMusicRepresentationMain => "SMR Main",
            AudioObjectType::UnifiedSpeechAudioCoding => "USAC",
            AudioObjectType::SpatialAudioObjectCoding => "SAOC",
            AudioObjectType::LowDelayMpegSurround => "LD MPEG Surround",
            AudioObjectType::SpatialAudioObjectCodingDialogueEnhancement => "SAOC-DE",
            AudioObjectType::AudioSync => "Audio Sync",
        };
        write!(f, "{type_str}")
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum SampleFreqIndex {
    Freq96000 = 0x0,
    Freq88200 = 0x1,
    Freq64000 = 0x2,
    Freq48000 = 0x3,
    Freq44100 = 0x4,
    Freq32000 = 0x5,
    Freq24000 = 0x6,
    Freq22050 = 0x7,
    Freq16000 = 0x8,
    Freq12000 = 0x9,
    Freq11025 = 0xa,
    Freq8000 = 0xb,
    Freq7350 = 0xc,
}

impl TryFrom<u8> for SampleFreqIndex {
    type Error = Error;
    fn try_from(value: u8) -> Result<SampleFreqIndex> {
        match value {
            0x0 => Ok(SampleFreqIndex::Freq96000),
            0x1 => Ok(SampleFreqIndex::Freq88200),
            0x2 => Ok(SampleFreqIndex::Freq64000),
            0x3 => Ok(SampleFreqIndex::Freq48000),
            0x4 => Ok(SampleFreqIndex::Freq44100),
            0x5 => Ok(SampleFreqIndex::Freq32000),
            0x6 => Ok(SampleFreqIndex::Freq24000),
            0x7 => Ok(SampleFreqIndex::Freq22050),
            0x8 => Ok(SampleFreqIndex::Freq16000),
            0x9 => Ok(SampleFreqIndex::Freq12000),
            0xa => Ok(SampleFreqIndex::Freq11025),
            0xb => Ok(SampleFreqIndex::Freq8000),
            0xc => Ok(SampleFreqIndex::Freq7350),
            _ => Err(Error::InvalidData("invalid sampling frequency index")),
        }
    }
}

impl SampleFreqIndex {
    pub fn freq(&self) -> u32 {
        match *self {
            SampleFreqIndex::Freq96000 => 96000,
            SampleFreqIndex::Freq88200 => 88200,
            SampleFreqIndex::Freq64000 => 64000,
            SampleFreqIndex::Freq48000 => 48000,
            SampleFreqIndex::Freq44100 => 44100,
            SampleFreqIndex::Freq32000 => 32000,
            SampleFreqIndex::Freq24000 => 24000,
            SampleFreqIndex::Freq22050 => 22050,
            SampleFreqIndex::Freq16000 => 16000,
            SampleFreqIndex::Freq12000 => 12000,
            SampleFreqIndex::Freq11025 => 11025,
            SampleFreqIndex::Freq8000 => 8000,
            SampleFreqIndex::Freq7350 => 7350,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum ChannelConfig {
    Mono = 0x1,
    Stereo = 0x2,
    Three = 0x3,
    Four = 0x4,
    Five = 0x5,
    FiveOne = 0x6,
    SevenOne = 0x7,
}

impl TryFrom<u8> for ChannelConfig {
    type Error = Error;
    fn try_from(value: u8) -> Result<ChannelConfig> {
        match value {
            0x1 => Ok(ChannelConfig::Mono),
            0x2 => Ok(ChannelConfig::Stereo),
            0x3 => Ok(ChannelConfig::Three),
            0x4 => Ok(ChannelConfig::Four),
            0x5 => Ok(ChannelConfig::Five),
            0x6 => Ok(ChannelConfig::FiveOne),
            0x7 => Ok(ChannelConfig::SevenOne),
            _ => Err(Error::InvalidData("invalid channel configuration")),
        }
    }
}

impl fmt::Display for ChannelConfig {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let s = match self {
            ChannelConfig::Mono => "mono",
            ChannelConfig::Stereo => "stereo",
            ChannelConfig::Three => "three",
            ChannelConfig::Four => "four",
            ChannelConfig::Five => "five",
            ChannelConfig::FiveOne => "five.one",
            ChannelConfig::SevenOne => "seven.one",
        };
        write!(f, "{s}")
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Default)]
pub struct AvcConfig {
    pub width: u16,
    pub height: u16,
    pub seq_param_set: Vec<u8>,
    pub pic_param_set: Vec<u8>,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct HevcConfig {
    pub width: u16,
    pub height: u16,
    pub general_profile_space: u8,
    pub general_tier_flag: bool,
    pub general_profile_idc: u8,
    pub general_profile_compatibility_flags: u32,
    pub general_constraint_indicator_flags: u64,
    pub general_level_idc: u8,
    pub min_spatial_segmentation_idc: u16,
    pub parallelism_type: u8,
    pub chroma_format_idc: u8,
    pub bit_depth_luma_minus8: u8,
    pub bit_depth_chroma_minus8: u8,
    pub avg_frame_rate: u16,
    pub constant_frame_rate: u8,
    pub num_temporal_layers: u8,
    pub temporal_id_nested: bool,
    pub length_size_minus_one: u8,
    pub vps: Vec<Vec<u8>>,
    pub sps: Vec<Vec<u8>>,
    pub pps: Vec<Vec<u8>>,
}

impl Default for HevcConfig {
    fn default() -> Self {
        Self {
            width: 0,
            height: 0,
            general_profile_space: 0,
            general_tier_flag: false,
            general_profile_idc: 1,
            general_profile_compatibility_flags: 0x6000_0000,
            general_constraint_indicator_flags: 0,
            general_level_idc: 120,
            min_spatial_segmentation_idc: 0,
            parallelism_type: 0,
            chroma_format_idc: 1,
            bit_depth_luma_minus8: 0,
            bit_depth_chroma_minus8: 0,
            avg_frame_rate: 0,
            constant_frame_rate: 0,
            num_temporal_layers: 1,
            temporal_id_nested: true,
            length_size_minus_one: 3,
            vps: Vec::new(),
            sps: Vec::new(),
            pps: Vec::new(),
        }
    }
}

impl HevcConfig {
    pub fn validate(&self) -> Result<()> {
        if self.width == 0 || self.height == 0 {
            return Err(Error::InvalidData("width and height must be non-zero"));
        }
        if self.general_profile_space > 3 {
            return Err(Error::InvalidData(
                "general_profile_space must be <= 3 (2 bits)",
            ));
        }
        if self.general_profile_idc > 31 {
            return Err(Error::InvalidData(
                "general_profile_idc must be <= 31 (5 bits)",
            ));
        }
        if self.general_constraint_indicator_flags > 0x0000_FFFF_FFFF_FFFF {
            return Err(Error::InvalidData(
                "general_constraint_indicator_flags must be 48-bit",
            ));
        }
        if self.min_spatial_segmentation_idc > 4095 {
            return Err(Error::InvalidData(
                "min_spatial_segmentation_idc must be <= 4095 (12 bits)",
            ));
        }
        if self.parallelism_type > 3 {
            return Err(Error::InvalidData("parallelism_type must be <= 3 (2 bits)"));
        }
        if self.chroma_format_idc > 3 {
            return Err(Error::InvalidData(
                "chroma_format_idc must be <= 3 (2 bits)",
            ));
        }
        if self.bit_depth_luma_minus8 > 7 {
            return Err(Error::InvalidData(
                "bit_depth_luma_minus8 must be <= 7 (3 bits)",
            ));
        }
        if self.bit_depth_chroma_minus8 > 7 {
            return Err(Error::InvalidData(
                "bit_depth_chroma_minus8 must be <= 7 (3 bits)",
            ));
        }
        if self.constant_frame_rate > 3 {
            return Err(Error::InvalidData(
                "constant_frame_rate must be <= 3 (2 bits)",
            ));
        }
        if self.num_temporal_layers > 7 {
            return Err(Error::InvalidData(
                "num_temporal_layers must be <= 7 (3 bits)",
            ));
        }
        if self.length_size_minus_one != 3 {
            return Err(Error::InvalidData(
                "length_size_minus_one must be 3 for 4-byte NAL length prefix",
            ));
        }
        if self.vps.is_empty() {
            return Err(Error::InvalidData("VPS array must not be empty"));
        }
        if self.sps.is_empty() {
            return Err(Error::InvalidData("SPS array must not be empty"));
        }
        if self.pps.is_empty() {
            return Err(Error::InvalidData("PPS array must not be empty"));
        }
        if self.vps.len() > u16::MAX as usize {
            return Err(Error::InvalidData("too many VPS NAL units"));
        }
        if self.sps.len() > u16::MAX as usize {
            return Err(Error::InvalidData("too many SPS NAL units"));
        }
        if self.pps.len() > u16::MAX as usize {
            return Err(Error::InvalidData("too many PPS NAL units"));
        }
        for nal in &self.vps {
            if nal.is_empty() {
                return Err(Error::InvalidData("VPS NAL unit must not be empty"));
            }
            if nal.len() > u16::MAX as usize {
                return Err(Error::InvalidData(
                    "VPS NAL unit length exceeds 65535 bytes",
                ));
            }
        }
        for nal in &self.sps {
            if nal.is_empty() {
                return Err(Error::InvalidData("SPS NAL unit must not be empty"));
            }
            if nal.len() > u16::MAX as usize {
                return Err(Error::InvalidData(
                    "SPS NAL unit length exceeds 65535 bytes",
                ));
            }
        }
        for nal in &self.pps {
            if nal.is_empty() {
                return Err(Error::InvalidData("PPS NAL unit must not be empty"));
            }
            if nal.len() > u16::MAX as usize {
                return Err(Error::InvalidData(
                    "PPS NAL unit length exceeds 65535 bytes",
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Av1Config {
    pub width: u16,
    pub height: u16,
    pub seq_profile: u8,
    pub seq_level_idx_0: u8,
    pub seq_tier_0: bool,
    pub high_bitdepth: bool,
    pub twelve_bit: bool,
    pub monochrome: bool,
    pub chroma_subsampling_x: bool,
    pub chroma_subsampling_y: bool,
    pub chroma_sample_position: u8,
    pub initial_presentation_delay_minus_one: Option<u8>,
    pub config_obus: Vec<u8>,
}

impl Default for Av1Config {
    fn default() -> Self {
        Self {
            width: 0,
            height: 0,
            seq_profile: 0,
            seq_level_idx_0: 0,
            seq_tier_0: false,
            high_bitdepth: false,
            twelve_bit: false,
            monochrome: false,
            chroma_subsampling_x: true,
            chroma_subsampling_y: true,
            chroma_sample_position: 0,
            initial_presentation_delay_minus_one: None,
            config_obus: Vec::new(),
        }
    }
}

fn minimal_leb128_len(mut val: usize) -> usize {
    let mut count = 0;
    loop {
        val >>= 7;
        count += 1;
        if val == 0 {
            break;
        }
    }
    count
}

pub fn parse_and_validate_av1_obus(data: &[u8]) -> Result<bool> {
    if data.is_empty() {
        return Err(Error::InvalidData("AV1 config OBUs cannot be empty"));
    }
    let mut offset = 0;
    let mut obu_index = 0;

    while offset < data.len() {
        let header_byte = data[offset];
        offset += 1;

        if (header_byte & 0x80) != 0 {
            return Err(Error::InvalidData("AV1 OBU forbidden bit is set"));
        }
        if (header_byte & 0x01) != 0 {
            return Err(Error::InvalidData("AV1 OBU reserved bit is set"));
        }

        let obu_type = (header_byte >> 3) & 0x0F;
        let obu_extension_flag = (header_byte & 0x04) != 0;
        let obu_has_size_field = (header_byte & 0x02) != 0;

        if !obu_has_size_field {
            return Err(Error::InvalidData(
                "AV1 OBU must have size field (obu_has_size_field=1)",
            ));
        }

        if obu_extension_flag {
            if offset >= data.len() {
                return Err(Error::InvalidData("truncated AV1 OBU extension header"));
            }
            let ext_byte = data[offset];
            offset += 1;
            if (ext_byte & 0x07) != 0 {
                return Err(Error::InvalidData(
                    "AV1 extension header reserved bits are set",
                ));
            }
        }

        let mut obu_size: usize = 0;
        let mut leb128_bytes = 0;
        loop {
            if offset >= data.len() {
                return Err(Error::InvalidData("truncated AV1 LEB128 size field"));
            }
            if leb128_bytes >= 8 {
                return Err(Error::InvalidData("AV1 LEB128 size field exceeds 8 bytes"));
            }
            let byte = data[offset];
            offset += 1;
            let val = (byte & 0x7F) as usize;
            let shift = leb128_bytes * 7;
            if shift >= (usize::BITS as usize) {
                return Err(Error::InvalidData("AV1 LEB128 size overflow"));
            }
            obu_size = obu_size
                .checked_add(val << shift)
                .ok_or(Error::InvalidData("AV1 LEB128 size overflow"))?;
            leb128_bytes += 1;

            if (byte & 0x80) == 0 {
                break;
            }
        }

        if obu_size > u32::MAX as usize {
            return Err(Error::InvalidData("AV1 OBU size exceeds 32-bit limit"));
        }

        if leb128_bytes != minimal_leb128_len(obu_size) {
            return Err(Error::InvalidData(
                "AV1 config OBU uses non-minimal LEB128 size encoding",
            ));
        }

        if offset
            .checked_add(obu_size)
            .is_none_or(|end| end > data.len())
        {
            return Err(Error::InvalidData(
                "AV1 OBU payload exceeds available data (truncated)",
            ));
        }

        if obu_index == 0 {
            if obu_type != 1 {
                return Err(Error::InvalidData(
                    "first AV1 config OBU must be Sequence Header (type 1)",
                ));
            }
        } else {
            if obu_type == 1 {
                return Err(Error::InvalidData(
                    "duplicate Sequence Header OBU in AV1 config OBUs",
                ));
            }
            if obu_type != 5 {
                return Err(Error::InvalidData(
                    "only Sequence Header (type 1) and Metadata (type 5) OBUs allowed in config OBUs",
                ));
            }
        }

        obu_index += 1;
        offset += obu_size;
    }

    if obu_index == 0 {
        return Err(Error::InvalidData(
            "AV1 config OBUs missing Sequence Header OBU (type 1)",
        ));
    }

    Ok(true)
}

impl Av1Config {
    pub fn validate(&self) -> Result<()> {
        if self.width == 0 || self.height == 0 {
            return Err(Error::InvalidData("width and height must be non-zero"));
        }
        if self.seq_profile > 2 {
            return Err(Error::InvalidData("seq_profile must be 0..=2"));
        }
        if !((0..=19).contains(&self.seq_level_idx_0) || self.seq_level_idx_0 == 31) {
            return Err(Error::InvalidData("seq_level_idx_0 must be 0..=19 or 31"));
        }
        if self.seq_tier_0 && self.seq_level_idx_0 <= 7 {
            return Err(Error::InvalidData(
                "seq_tier_0 cannot be high tier for level <= 7",
            ));
        }
        if self.chroma_sample_position > 2 {
            return Err(Error::InvalidData("chroma_sample_position must be 0..=2"));
        }
        if (!self.chroma_subsampling_x || !self.chroma_subsampling_y)
            && self.chroma_sample_position != 0
        {
            return Err(Error::InvalidData(
                "chroma_sample_position must be 0 (unknown) when not 4:2:0",
            ));
        }
        if self.monochrome
            && (!self.chroma_subsampling_x
                || !self.chroma_subsampling_y
                || self.chroma_sample_position != 0)
        {
            return Err(Error::InvalidData(
                "monochrome must have chroma_subsampling_x=true, chroma_subsampling_y=true, chroma_sample_position=0",
            ));
        }
        if self.twelve_bit && (!self.high_bitdepth || self.seq_profile != 2) {
            return Err(Error::InvalidData(
                "twelve_bit requires high_bitdepth and seq_profile=2",
            ));
        }
        if self.seq_profile == 0 && (!self.chroma_subsampling_x || !self.chroma_subsampling_y) {
            return Err(Error::InvalidData(
                "seq_profile 0 requires 4:2:0 chroma subsampling (x=true, y=true)",
            ));
        }
        if self.seq_profile == 1
            && (self.monochrome || self.chroma_subsampling_x || self.chroma_subsampling_y)
        {
            return Err(Error::InvalidData(
                "seq_profile 1 requires non-monochrome 4:4:4 (x=false, y=false)",
            ));
        }
        if self.seq_profile == 2
            && !self.monochrome
            && !self.twelve_bit
            && (!self.chroma_subsampling_x || self.chroma_subsampling_y)
        {
            return Err(Error::InvalidData(
                "seq_profile 2 (non-12-bit) requires 4:2:2 (x=true, y=false)",
            ));
        }
        if self.seq_profile == 2
            && !self.monochrome
            && self.twelve_bit
            && (!self.chroma_subsampling_x && self.chroma_subsampling_y)
        {
            return Err(Error::InvalidData(
                "seq_profile 2 12-bit cannot have x=false, y=true",
            ));
        }
        if let Some(delay) = self.initial_presentation_delay_minus_one {
            if delay > 15 {
                return Err(Error::InvalidData(
                    "initial_presentation_delay_minus_one must be <= 15 (4 bits)",
                ));
            }
        }
        if self.config_obus.is_empty() {
            return Err(Error::InvalidData("config_obus must not be empty"));
        }
        parse_and_validate_av1_obus(&self.config_obus)?;
        Ok(())
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Default)]
pub struct Vp9Config {
    pub width: u16,
    pub height: u16,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct AacConfig {
    pub bitrate: u32,
    pub profile: AudioObjectType,
    pub freq_index: SampleFreqIndex,
    pub chan_conf: ChannelConfig,
}

impl Default for AacConfig {
    fn default() -> Self {
        Self {
            bitrate: 0,
            profile: AudioObjectType::AacLowComplexity,
            freq_index: SampleFreqIndex::Freq48000,
            chan_conf: ChannelConfig::Stereo,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Default)]
pub struct TtxtConfig {}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum MediaConfig {
    AvcConfig(AvcConfig),
    HevcConfig(HevcConfig),
    Av1Config(Av1Config),
    Vp9Config(Vp9Config),
    AacConfig(AacConfig),
    TtxtConfig(TtxtConfig),
}

#[derive(Debug)]
pub struct Mp4Sample {
    pub start_time: u64,
    pub duration: u32,
    pub rendering_offset: i32,
    pub is_sync: bool,
    pub bytes: Bytes,
}

impl PartialEq for Mp4Sample {
    fn eq(&self, other: &Self) -> bool {
        self.start_time == other.start_time
            && self.duration == other.duration
            && self.rendering_offset == other.rendering_offset
            && self.is_sync == other.is_sync
            && self.bytes.len() == other.bytes.len() // XXX for easy check
    }
}

impl fmt::Display for Mp4Sample {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "start_time {}, duration {}, rendering_offset {}, is_sync {}, length {}",
            self.start_time,
            self.duration,
            self.rendering_offset,
            self.is_sync,
            self.bytes.len()
        )
    }
}

pub fn creation_time(creation_time: u64) -> u64 {
    // convert from MP4 epoch (1904-01-01) to Unix epoch (1970-01-01)
    if creation_time >= 2082844800 {
        creation_time - 2082844800
    } else {
        creation_time
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
pub enum DataType {
    #[default]
    Binary = 0x000000,
    Text = 0x000001,
    Image = 0x00000D,
    TempoCpil = 0x000015,
}

impl TryFrom<u32> for DataType {
    type Error = Error;
    fn try_from(value: u32) -> Result<DataType> {
        match value {
            0x000000 => Ok(DataType::Binary),
            0x000001 => Ok(DataType::Text),
            0x00000D => Ok(DataType::Image),
            0x000015 => Ok(DataType::TempoCpil),
            _ => Err(Error::InvalidData("invalid data type")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub enum MetadataKey {
    Title,
    Year,
    Poster,
    Summary,
}

pub trait Metadata<'a> {
    /// The video's title
    fn title(&self) -> Option<Cow<'_, str>>;
    /// The video's release year
    fn year(&self) -> Option<u32>;
    /// The video's poster (cover art)
    fn poster(&self) -> Option<&[u8]>;
    /// The video's summary
    fn summary(&self) -> Option<Cow<'_, str>>;
}

impl<'a, T: Metadata<'a>> Metadata<'a> for &'a T {
    fn title(&self) -> Option<Cow<'_, str>> {
        (**self).title()
    }

    fn year(&self) -> Option<u32> {
        (**self).year()
    }

    fn poster(&self) -> Option<&[u8]> {
        (**self).poster()
    }

    fn summary(&self) -> Option<Cow<'_, str>> {
        (**self).summary()
    }
}

impl<'a, T: Metadata<'a>> Metadata<'a> for Option<T> {
    fn title(&self) -> Option<Cow<'_, str>> {
        self.as_ref().and_then(|t| t.title())
    }

    fn year(&self) -> Option<u32> {
        self.as_ref().and_then(|t| t.year())
    }

    fn poster(&self) -> Option<&[u8]> {
        self.as_ref().and_then(|t| t.poster())
    }

    fn summary(&self) -> Option<Cow<'_, str>> {
        self.as_ref().and_then(|t| t.summary())
    }
}
