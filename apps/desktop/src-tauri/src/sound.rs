//! Native Windows completion sound cue for replay save completion.
//!
//! Plays an original embedded PCM WAV cue on `ControllerEvent::ClipSaved`
//! when enabled. Dispatch uses a bounded worker queue and failure is
//! completely non-fatal.

use app_controller::ControllerEvent;

/// Original generated subtle completion chime embedded statically into the binary.
pub const CLIP_SAVED_WAV: &[u8] = include_bytes!("clip_saved.wav");

/// Pure decision seam determining whether a completion sound cue should be dispatched.
/// Dispatches strictly on `ControllerEvent::ClipSaved` when `clip_sound_enabled` is true.
#[must_use]
pub fn should_play_clip_sound(event: &ControllerEvent, clip_sound_enabled: bool) -> bool {
    clip_sound_enabled && matches!(event, ControllerEvent::ClipSaved { .. })
}

/// One playback worker, with at most one pending cue. OS audio work and PCM
/// scaling never execute on the controller or capture threads.
static SOUND_QUEUE: std::sync::OnceLock<Option<std::sync::mpsc::SyncSender<u16>>> =
    std::sync::OnceLock::new();

pub fn initialize() {
    SOUND_QUEUE.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("silk-chime".into())
            .spawn(move || {
                let mut wav = CLIP_SAVED_WAV.to_vec();
                while let Ok(volume) = rx.recv() {
                    scale_chime(&mut wav, volume);
                    #[cfg(windows)]
                    unsafe {
                        use windows_sys::Win32::Media::Audio::{
                            PlaySoundW, SND_MEMORY, SND_NODEFAULT, SND_SYNC,
                        };
                        // Buffer remains owned and unchanged until playback finishes.
                        // Synchronous only on this worker; no system volume changes.
                        let _ = PlaySoundW(
                            wav.as_ptr().cast(),
                            std::ptr::null_mut(),
                            SND_MEMORY | SND_NODEFAULT | SND_SYNC,
                        );
                    }
                }
            })
            .ok()
            .map(|_| tx)
    });
}

pub fn play_clip_saved_sound(volume: u16) -> Result<(), String> {
    if volume > 300 {
        return Err("Chime volume must be between 0 and 300 percent".into());
    }
    if volume == 0 {
        return Ok(());
    }
    SOUND_QUEUE
        .get()
        .and_then(Option::as_ref)
        .ok_or_else(|| "Chime playback is unavailable".to_string())?
        .try_send(volume)
        .map_err(|_| "Chime playback is busy or unavailable".to_string())
}

/// Tests validate the embedded asset as canonical 16-bit PCM.
/// Scale from the original samples, never from a previous volume.
fn scale_chime(wav: &mut [u8], volume: u16) {
    for (source, output) in CLIP_SAVED_WAV[44..]
        .as_chunks::<2>()
        .0
        .iter()
        .zip(wav[44..].as_chunks_mut::<2>().0.iter_mut())
    {
        let sample = i32::from(i16::from_le_bytes([source[0], source[1]]));
        let scaled = (sample * i32::from(volume.min(300)) / 100)
            .clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16;
        output.copy_from_slice(&scaled.to_le_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_scales_original_pcm_without_changing_the_header_or_accumulating_gain() {
        let mut wav = CLIP_SAVED_WAV.to_vec();
        for volume in [0, 50, 100, 200, 300, 100] {
            scale_chime(&mut wav, volume);
            assert_eq!(&wav[..44], &CLIP_SAVED_WAV[..44]);
            for (source, actual) in CLIP_SAVED_WAV[44..]
                .as_chunks::<2>()
                .0
                .iter()
                .zip(wav[44..].as_chunks::<2>().0.iter())
            {
                let source = i32::from(i16::from_le_bytes(*source));
                let actual = i32::from(i16::from_le_bytes(*actual));
                // This particular asset has enough headroom for the full boost.
                let expected = source * i32::from(volume) / 100;
                assert!((i32::from(i16::MIN)..=i32::from(i16::MAX)).contains(&expected));
                assert_eq!(actual, expected);
            }
        }
        assert_eq!(wav, CLIP_SAVED_WAV);
    }

    #[test]
    fn mute_and_invalid_volume_do_not_need_a_playback_worker() {
        assert!(play_clip_saved_sound(0).is_ok());
        assert!(play_clip_saved_sound(301).is_err());
        assert!(play_clip_saved_sound(u16::MAX).is_err());
    }

    #[test]
    fn should_play_clip_sound_only_on_clip_saved_when_enabled() {
        let clip_saved = ControllerEvent::ClipSaved {
            path: "C:/clips/test.mp4".to_string(),
            duration_ms: 5000,
            size_bytes: 1024,
        };
        let save_queued = ControllerEvent::SaveQueued {
            path: "C:/clips/test.mp4".to_string(),
        };
        let save_failed = ControllerEvent::SaveFailed {
            code: "DISK_FULL".to_string(),
            message: "no space".to_string(),
        };
        let status_changed = ControllerEvent::StatusChanged {
            state: "Buffering".to_string(),
        };
        let command_rejected = ControllerEvent::CommandRejected {
            command: "save_replay".to_string(),
            reason: "not ready".to_string(),
        };

        // Enabled: only ClipSaved triggers sound
        assert!(should_play_clip_sound(&clip_saved, true));
        assert!(!should_play_clip_sound(&save_queued, true));
        assert!(!should_play_clip_sound(&save_failed, true));
        assert!(!should_play_clip_sound(&status_changed, true));
        assert!(!should_play_clip_sound(&command_rejected, true));

        // Disabled: ClipSaved does not trigger sound
        assert!(!should_play_clip_sound(&clip_saved, false));
        assert!(!should_play_clip_sound(&save_queued, false));
        assert!(!should_play_clip_sound(&save_failed, false));
        assert!(!should_play_clip_sound(&status_changed, false));
        assert!(!should_play_clip_sound(&command_rejected, false));
    }

    #[test]
    fn embedded_wav_has_valid_riff_wave_pcm_structure() {
        let wav = CLIP_SAVED_WAV;
        assert!(
            wav.len() >= 44,
            "WAV data must be at least 44 bytes for standard header"
        );

        // RIFF header
        assert_eq!(&wav[0..4], b"RIFF", "magic must be RIFF");
        let riff_size = u32::from_le_bytes([wav[4], wav[5], wav[6], wav[7]]);
        assert_eq!(
            riff_size as usize + 8,
            wav.len(),
            "RIFF chunk size must equal total length minus 8"
        );
        assert_eq!(&wav[8..12], b"WAVE", "format must be WAVE");

        // fmt chunk
        assert_eq!(&wav[12..16], b"fmt ", "expected fmt chunk");
        let fmt_size = u32::from_le_bytes([wav[16], wav[17], wav[18], wav[19]]);
        assert_eq!(fmt_size, 16, "fmt subchunk size must be 16 for PCM");
        let audio_format = u16::from_le_bytes([wav[20], wav[21]]);
        assert_eq!(audio_format, 1, "audio format must be 1 (PCM)");
        let num_channels = u16::from_le_bytes([wav[22], wav[23]]);
        assert!(
            num_channels == 1 || num_channels == 2,
            "channels must be 1 or 2"
        );
        let sample_rate = u32::from_le_bytes([wav[24], wav[25], wav[26], wav[27]]);
        assert!(sample_rate >= 22050, "sample rate must be >= 22050 Hz");
        let byte_rate = u32::from_le_bytes([wav[28], wav[29], wav[30], wav[31]]);
        let block_align = u16::from_le_bytes([wav[32], wav[33]]);
        let bits_per_sample = u16::from_le_bytes([wav[34], wav[35]]);
        assert_eq!(bits_per_sample, 16, "must be 16-bit PCM");
        assert_eq!(
            block_align,
            num_channels * (bits_per_sample / 8),
            "block align mismatch"
        );
        assert_eq!(
            byte_rate,
            sample_rate * u32::from(block_align),
            "byte rate mismatch"
        );

        // data chunk
        assert_eq!(&wav[36..40], b"data", "expected data chunk");
        let data_size = u32::from_le_bytes([wav[40], wav[41], wav[42], wav[43]]) as usize;
        assert_eq!(
            44 + data_size,
            wav.len(),
            "data size plus header must equal total WAV byte length"
        );
        assert!(data_size > 0, "audio data must not be empty");

        // Inspect PCM samples
        let sample_bytes = &wav[44..];
        let mut non_zero_samples = 0_usize;
        let mut max_abs_sample = 0_i16;
        for chunk in sample_bytes.as_chunks::<2>().0 {
            let sample = i16::from_le_bytes([chunk[0], chunk[1]]);
            if sample != 0 {
                non_zero_samples += 1;
            }
            max_abs_sample = max_abs_sample.max(sample.saturating_abs());
        }

        assert!(
            non_zero_samples > 100,
            "sound cue must contain actual audible samples"
        );
        assert!(
            max_abs_sample > 1000,
            "peak audio amplitude must be audible (got {max_abs_sample})"
        );
    }
}
