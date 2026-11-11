#![cfg(windows)]

use std::time::{Duration, Instant};

use capture_api::{VideoCapture, VideoCaptureConfig, VideoCaptureEvent};
use capture_windows::{enumerate_displays, WgcDisplayCapture};
use encoder_api::{EncoderPreference, VideoEncoder, VideoEncoderConfig};
use encoder_media_foundation::new_fallback_encoder;
use media_types::{StreamId, TimeBase};

#[test]
#[ignore = "requires interactive desktop + WGC + a D3D11-aware H.264 MFT"]
fn media_foundation_encodes_wgc_gpu_frame() {
    let (frames, packets) = encode_native_gpu_frames(VideoEncoderConfig::default());
    assert!(frames >= 5, "expected WGC frames, got {frames}");
    assert!(
        !packets.is_empty(),
        "expected H.264 packets from WGC frames"
    );
    assert!(packets
        .iter()
        .all(|packet| packet.time_base == TimeBase::MILLISECOND));
}

#[test]
#[ignore = "requires interactive desktop + WGC + a D3D11-aware H.264 MFT"]
fn media_foundation_scales_wgc_gpu_frame_to_640x360() {
    let config = VideoEncoderConfig {
        width: 640,
        height: 360,
        fps: 30,
        ..VideoEncoderConfig::default()
    };
    let (frames, packets) = encode_native_gpu_frames(config);
    assert!(frames >= 5, "expected WGC frames, got {frames}");
    assert!(
        !packets.is_empty(),
        "expected H.264 packets from scaled WGC frames"
    );
}

#[test]
#[ignore = "requires interactive desktop + WGC + a D3D11-aware H.264 MFT"]
fn media_foundation_supersamples_wgc_gpu_frame_to_2x_if_bounded() {
    let sources = enumerate_displays().expect("enumeration");
    let primary = sources
        .iter()
        .find(|source| source.is_primary)
        .expect("primary display");
    let target_width = primary.width.saturating_mul(2);
    let target_height = primary.height.saturating_mul(2);
    // Bounded to typical MFT envelope (4096)
    if target_width <= 4096 && target_height <= 4096 {
        let config = VideoEncoderConfig {
            width: target_width,
            height: target_height,
            fps: 30,
            ..VideoEncoderConfig::default()
        };
        let (frames, packets) = encode_native_gpu_frames(config);
        assert!(frames >= 5, "expected WGC frames, got {frames}");
        assert!(
            !packets.is_empty(),
            "expected H.264 packets from 2x supersampled WGC frames"
        );
    }
}

fn encode_native_gpu_frames(config: VideoEncoderConfig) -> (u32, Vec<media_types::EncodedPacket>) {
    encode_with_capture(config, Box::new(WgcDisplayCapture::new(StreamId(0))))
}

#[test]
#[ignore = "requires interactive desktop + WGC + hardware H.264 encoder"]
fn wgc_replay_survives_multiple_keyframe_intervals() {
    // Match the failing Iris Xe report and run beyond its first GOP boundary.
    let config = VideoEncoderConfig {
        width: 1600,
        height: 900,
        fps: 60,
        ..VideoEncoderConfig::default()
    };
    let (frames, packets) =
        encode_with_capture_count(config, Box::new(WgcDisplayCapture::new(StreamId(0))), 360);
    assert_eq!(frames, 360);
    assert!(packets.len() >= 300, "encoder stopped producing video");
    assert!(packets.windows(2).all(|pair| pair[1].dts >= pair[0].dts));
}

#[test]
#[ignore = "requires interactive desktop + DXGI + a D3D11-aware H.264 MFT"]
fn media_foundation_encodes_dxgi_gpu_frames() {
    let config = VideoEncoderConfig {
        width: 1280,
        height: 720,
        fps: 60,
        ..VideoEncoderConfig::default()
    };
    let (frames, packets) = encode_with_capture(
        config,
        Box::new(capture_windows::DxgiDisplayCapture::new(StreamId(0))),
    );
    assert_eq!(frames, 120);
    assert!(!packets.is_empty());
    assert!(packets.iter().all(|p| p.time_base == TimeBase::MILLISECOND));
    assert!(packets.windows(2).all(|p| p[1].dts >= p[0].dts));
}

fn encode_with_capture(
    config: VideoEncoderConfig,
    capture: Box<dyn VideoCapture>,
) -> (u32, Vec<media_types::EncodedPacket>) {
    encode_with_capture_count(config, capture, 120)
}

fn encode_with_capture_count(
    config: VideoEncoderConfig,
    capture: Box<dyn VideoCapture>,
    frame_count: u32,
) -> (u32, Vec<media_types::EncodedPacket>) {
    encode_with_preference(config, capture, frame_count, EncoderPreference::Auto)
}

#[test]
#[ignore = "requires interactive desktop + WGC + Windows software H.264 MFT"]
fn software_encoder_accepts_scaled_wgc_gpu_frames() {
    let (frames, packets) = encode_with_preference(
        VideoEncoderConfig {
            width: 640,
            height: 360,
            fps: 30,
            ..VideoEncoderConfig::default()
        },
        Box::new(WgcDisplayCapture::new(StreamId(0))),
        90,
        EncoderPreference::SoftwareOnly,
    );
    assert_eq!(frames, 90);
    assert!(!packets.is_empty(), "software encoder produced no video");
    assert!(
        packets.last().unwrap().pts - packets.first().unwrap().pts >= 1000,
        "expected sustained software encoding; got {} packets",
        packets.len()
    );
    assert!(packets.windows(2).all(|pair| pair[1].dts >= pair[0].dts));
}

#[test]
#[ignore = "requires interactive desktop + WGC + Windows software H.264 MFT"]
fn software_encoder_accepts_reported_iris_xe_capture_settings() {
    let (frames, packets) = encode_with_preference(
        VideoEncoderConfig {
            width: 1600,
            height: 900,
            fps: 60,
            bitrate_kbps: Some(45000),
            ..VideoEncoderConfig::default()
        },
        Box::new(WgcDisplayCapture::new(StreamId(0))),
        60,
        EncoderPreference::SoftwareOnly,
    );
    assert_eq!(frames, 60);
    assert!(!packets.is_empty());
    assert!(packets.windows(2).all(|pair| pair[1].dts >= pair[0].dts));
}

#[test]
#[ignore = "requires interactive desktop, WGC, software H.264 MFT and SILK_TEST_FFMPEG"]
fn software_gpu_capture_bitstream_decodes() {
    let (_, packets) = encode_with_preference(
        VideoEncoderConfig {
            width: 640,
            height: 360,
            fps: 30,
            ..VideoEncoderConfig::default()
        },
        Box::new(WgcDisplayCapture::new(StreamId(0))),
        120,
        EncoderPreference::SoftwareOnly,
    );
    assert_bitstream_decodes(&packets);
}

#[test]
#[ignore = "requires interactive desktop, WGC, hardware H.264 MFT and SILK_TEST_FFMPEG"]
fn hardware_gpu_capture_bitstream_decodes() {
    let (_, packets) = encode_with_preference(
        VideoEncoderConfig {
            width: 640,
            height: 360,
            fps: 30,
            ..VideoEncoderConfig::default()
        },
        Box::new(WgcDisplayCapture::new(StreamId(0))),
        120,
        EncoderPreference::HardwareOnly,
    );
    assert_bitstream_decodes(&packets);
}

fn assert_bitstream_decodes(packets: &[media_types::EncodedPacket]) {
    let ffmpeg = std::env::var_os("SILK_TEST_FFMPEG")
        .expect("set SILK_TEST_FFMPEG to the approved ffmpeg executable");
    assert!(!packets.is_empty());
    let name = format!(
        "silk-native-{}-{}.h264",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let path = std::env::temp_dir().join(name);
    let bytes: Vec<u8> = packets
        .iter()
        .flat_map(|packet| packet.payload.as_slice().iter().copied())
        .collect();
    std::fs::write(&path, bytes).unwrap();
    let result = std::process::Command::new(ffmpeg)
        .args(["-v", "error", "-xerror", "-f", "h264", "-i"])
        .arg(&path)
        .args(["-f", "null", "-"])
        .output();
    let output = result.expect("run approved FFmpeg decoder");
    if output.status.success() {
        std::fs::remove_file(&path).unwrap();
    } else {
        eprintln!("Retained failed bitstream at {}", path.display());
    }
    assert!(
        output.status.success(),
        "capture bitstream failed to decode: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn encode_with_preference(
    config: VideoEncoderConfig,
    mut capture: Box<dyn VideoCapture>,
    frame_count: u32,
    preference: EncoderPreference,
) -> (u32, Vec<media_types::EncodedPacket>) {
    let sources = enumerate_displays().expect("enumeration");
    let primary = sources
        .iter()
        .find(|source| source.is_primary)
        .expect("primary display");

    capture
        .start(VideoCaptureConfig {
            source_id: primary.id.clone(),
            target_fps: config.fps,
        })
        .expect("start capture");

    let context = capture.gpu_frame_context().expect("active GPU context");
    let mut encoder = new_fallback_encoder(preference);
    encoder
        .set_gpu_frame_context(Some(context))
        .expect("install GPU context");
    encoder
        .configure(config)
        .expect("configure Media Foundation encoder");
    if preference == EncoderPreference::SoftwareOnly {
        assert!(!encoder.capabilities().hardware_accelerated);
    }

    let deadline = Instant::now() + Duration::from_secs(15);
    let mut frames = 0_u32;
    let mut packets = Vec::new();
    while Instant::now() < deadline && frames < frame_count {
        match capture.next_event() {
            Ok(VideoCaptureEvent::Frame(frame)) => {
                packets.extend(encoder.encode(frame).expect("encode WGC GPU frame"));
                frames += 1;
            }
            Ok(VideoCaptureEvent::FormatChanged { .. }) => {}
            Ok(VideoCaptureEvent::SourceLost) => panic!("source lost during encode"),
            Err(error) => panic!("capture error during encode: {error}"),
        }
    }
    packets.extend(encoder.drain().expect("drain Media Foundation encoder"));
    eprintln!(
        "encoded {} packets from {frames} capture frames; metrics: {:?}",
        packets.len(),
        encoder.metrics()
    );
    capture.stop().expect("stop capture");

    (frames, packets)
}

#[test]
#[ignore = "probes Windows Media Foundation 4:4:4 encoder transforms"]
fn probe_media_foundation_444_capabilities() {
    use std::ptr;
    use windows::core::{GUID, PWSTR};
    use windows::Win32::Media::MediaFoundation::{
        IMFActivate, IMFMediaType, IMFTransform, MFCreateMediaType, MFMediaType_Video, MFShutdown,
        MFStartup, MFT_FRIENDLY_NAME_Attribute, MFVideoFormat_H264, MFVideoFormat_NV12,
        MFSTARTUP_FULL, MFT_CATEGORY_VIDEO_ENCODER, MFT_ENUM_FLAG_HARDWARE,
        MFT_ENUM_FLAG_SORTANDFILTER, MFT_ENUM_FLAG_SYNCMFT, MFT_REGISTER_TYPE_INFO,
        MF_MT_AVG_BITRATE, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE, MF_MT_INTERLACE_MODE,
        MF_MT_MAJOR_TYPE, MF_MT_MPEG2_PROFILE, MF_MT_SUBTYPE, MF_VERSION,
    };
    use windows::Win32::System::Com::{
        CoInitializeEx, CoTaskMemFree, CoUninitialize, COINIT_MULTITHREADED,
    };

    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    let _ = unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL) };

    #[allow(non_upper_case_globals)]
    const MFVideoFormat_HEVC: GUID = GUID::from_u128(0x43564548_0000_0010_8000_00aa00389b71);
    #[allow(non_upper_case_globals)]
    const MFVideoFormat_AV1: GUID = GUID::from_u128(0x31305641_0000_0010_8000_00aa00389b71);
    #[allow(non_upper_case_globals)]
    const MFVideoFormat_AYUV: GUID = GUID::from_u128(0x56555941_0000_0010_8000_00aa00389b71);
    #[allow(non_upper_case_globals)]
    const MFVideoFormat_Y410: GUID = GUID::from_u128(0x30313459_0000_0010_8000_00aa00389b71);
    #[allow(non_upper_case_globals)]
    const MFVideoFormat_ARGB32: GUID = GUID::from_u128(0x00000015_0000_0010_8000_00aa00389b71);

    println!("\n=== Windows Media Foundation 4:4:4 Capability Probe ===");

    for (codec_guid, codec_name) in [
        (&MFVideoFormat_H264, "H.264 / AVC"),
        (&MFVideoFormat_HEVC, "H.265 / HEVC"),
        (&MFVideoFormat_AV1, "AV1"),
    ] {
        let output_type = MFT_REGISTER_TYPE_INFO {
            guidMajorType: MFMediaType_Video,
            guidSubtype: *codec_guid,
        };
        let mut raw_activations: *mut Option<IMFActivate> = ptr::null_mut();
        let mut count = 0_u32;
        let flags = MFT_ENUM_FLAG_HARDWARE | MFT_ENUM_FLAG_SYNCMFT | MFT_ENUM_FLAG_SORTANDFILTER;
        let res = unsafe {
            windows::Win32::Media::MediaFoundation::MFTEnumEx(
                MFT_CATEGORY_VIDEO_ENCODER,
                flags,
                None,
                Some(&output_type),
                &mut raw_activations,
                &mut count,
            )
        };

        if res.is_err() || count == 0 {
            println!("\n[{codec_name}] No hardware encoder MFTs found.");
            continue;
        }

        println!("\n[{codec_name}] Found {count} hardware MFT(s):");
        for i in 0..count as usize {
            let activation = unsafe { ptr::read(raw_activations.add(i)) };
            let Some(activation) = activation else {
                continue;
            };

            let mut name_ptr = PWSTR(ptr::null_mut());
            let mut name_len = 0_u32;
            let friendly_name = unsafe {
                if activation
                    .GetAllocatedString(&MFT_FRIENDLY_NAME_Attribute, &mut name_ptr, &mut name_len)
                    .is_ok()
                    && !name_ptr.0.is_null()
                {
                    let slice = std::slice::from_raw_parts(name_ptr.0, name_len as usize);
                    let name = String::from_utf16_lossy(slice);
                    CoTaskMemFree(Some(name_ptr.0.cast()));
                    name
                } else {
                    "Unknown Hardware MFT".to_string()
                }
            };

            println!("  -> MFT #{i}: {friendly_name}");

            let transform: Result<IMFTransform, _> = unsafe { activation.ActivateObject() };
            let Ok(transform) = transform else {
                println!("     Could not activate MFT object.");
                continue;
            };

            // Query supported input types
            let mut input_subtypes = Vec::new();
            let mut type_idx = 0;
            while let Ok(in_type) = unsafe { transform.GetInputAvailableType(0, type_idx) } {
                type_idx += 1;
                if let Ok(subtype) = unsafe { in_type.GetGUID(&MF_MT_SUBTYPE) } {
                    let name = if subtype == MFVideoFormat_NV12 {
                        "NV12 (4:2:0 8b)"
                    } else if subtype == MFVideoFormat_AYUV {
                        "AYUV (4:4:4 8b)"
                    } else if subtype == MFVideoFormat_Y410 {
                        "Y410 (4:4:4 10b)"
                    } else if subtype == MFVideoFormat_ARGB32 {
                        "ARGB32 (RGB 8b)"
                    } else {
                        "Other/Unknown"
                    };
                    input_subtypes.push(format!("{name} ({:?})", subtype));
                }
            }
            println!("     Supported Input Types: {:?}", input_subtypes);

            // Test 4:4:4 Profiles
            let mut test_profiles = Vec::new();
            if *codec_guid == MFVideoFormat_H264 {
                test_profiles.push(("High 4:4:4 Predictive (244)", 244u32));
                test_profiles.push(("High (100)", 100u32));
            } else if *codec_guid == MFVideoFormat_HEVC {
                test_profiles.push(("Main 4:4:4 8-bit (6)", 6u32));
                test_profiles.push(("Main 4:4:4 10-bit (7)", 7u32));
                test_profiles.push(("Main 10 (2)", 2u32));
                test_profiles.push(("Main (1)", 1u32));
            }

            for (p_name, p_val) in test_profiles {
                let out_type: Result<IMFMediaType, _> = unsafe { MFCreateMediaType() };
                if let Ok(out_type) = out_type {
                    let _ = unsafe { out_type.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video) };
                    let _ = unsafe { out_type.SetGUID(&MF_MT_SUBTYPE, codec_guid) };
                    let _ = unsafe { out_type.SetUINT32(&MF_MT_MPEG2_PROFILE, p_val) };
                    let _ =
                        unsafe { out_type.SetUINT64(&MF_MT_FRAME_SIZE, (1920u64 << 32) | 1080u64) };
                    let _ = unsafe { out_type.SetUINT64(&MF_MT_FRAME_RATE, (60u64 << 32) | 1u64) };
                    let _ = unsafe { out_type.SetUINT32(&MF_MT_AVG_BITRATE, 20_000_000) };
                    let _ = unsafe { out_type.SetUINT32(&MF_MT_INTERLACE_MODE, 2) }; // Progressive

                    let set_res = unsafe { transform.SetOutputType(0, &out_type, 0) };
                    let status = if set_res.is_ok() {
                        "ACCEPTED"
                    } else {
                        "REJECTED"
                    };
                    println!("     Output Profile {p_name}: {status}");
                }
            }
        }

        unsafe {
            if !raw_activations.is_null() {
                CoTaskMemFree(Some(raw_activations.cast()));
            }
        }
    }
    println!("=======================================================\n");

    let _ = unsafe { MFShutdown() };
    unsafe { CoUninitialize() };
}
