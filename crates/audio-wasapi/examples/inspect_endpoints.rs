//! Read-only Windows routing diagnostic. Does not record audio to disk.
#[cfg(windows)]
fn main() {
    use audio_api::{AudioCapture, AudioCaptureConfig, AudioCaptureEvent};
    use audio_wasapi::{enumerate_devices, WasapiAudioCapture, WasapiCaptureKind};
    use std::time::{Duration, Instant};
    use windows::core::{Interface, PCWSTR};
    use windows::Win32::Media::Audio::{
        IAudioSessionControl2, IAudioSessionManager2, IMMDeviceEnumerator, MMDeviceEnumerator,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED,
    };

    let devices = enumerate_devices(WasapiCaptureKind::Loopback).unwrap();
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok().unwrap();
        {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).unwrap();
            for info in devices {
                let id: Vec<_> = info.id.encode_utf16().chain(Some(0)).collect();
                let device = enumerator.GetDevice(PCWSTR(id.as_ptr())).unwrap();
                let manager: IAudioSessionManager2 = device.Activate(CLSCTX_ALL, None).unwrap();
                let sessions = manager.GetSessionEnumerator().unwrap();
                let mut processes = Vec::new();
                for i in 0..sessions.GetCount().unwrap() {
                    let control: IAudioSessionControl2 =
                        sessions.GetSession(i).unwrap().cast().unwrap();
                    processes.push((
                        control.GetProcessId().unwrap_or(0),
                        control.GetState().unwrap().0,
                    ));
                }
                println!(
                    "{} [{}] default={} sessions(pid,state)={processes:?}",
                    info.name, info.id, info.is_default
                );
                if std::env::args().any(|arg| arg == "--list") || !info.name.contains("GoXLR") {
                    continue;
                }
                let mut capture = WasapiAudioCapture::loopback();
                capture
                    .start(AudioCaptureConfig {
                        device_id: Some(info.id),
                    })
                    .unwrap();
                let started = Instant::now();
                let mut peak = 0.0_f64;
                let mut sample_bytes = 0;
                while started.elapsed() < Duration::from_secs(2) {
                    if let Ok(AudioCaptureEvent::Frames(frame)) = capture.next_event() {
                        sample_bytes += frame.data.len();
                        match frame.sample_format {
                            media_types::SampleFormat::F32 => {
                                for sample in frame.data.as_chunks::<4>().0 {
                                    peak = peak.max(f32::from_le_bytes(*sample).abs() as f64);
                                }
                            }
                            media_types::SampleFormat::S16 => {
                                for sample in frame.data.as_chunks::<2>().0 {
                                    peak = peak.max(
                                        f64::from(i16::from_le_bytes(*sample)).abs() / 32768.0,
                                    );
                                }
                            }
                        }
                    }
                }
                capture.stop().unwrap();
                println!("  loopback peak={peak:.6}, bytes={sample_bytes}");
            }
        }
        CoUninitialize();
    }
}

#[cfg(not(windows))]
fn main() {}
