//! Native Windows completion sound cue for replay save completion.
//!
//! Plays an original embedded PCM WAV cue on `ControllerEvent::ClipSaved`
//! when enabled. Playback is non-blocking (`SND_ASYNC`) and failure is
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

/// Trigger the native Windows completion cue asynchronously.
///
/// Uses `windows-sys` `PlaySoundW` with `SND_ASYNC | SND_MEMORY | SND_NODEFAULT`.
/// The embedded byte slice has `'static` lifetime, guaranteeing memory validity
/// for background playback. Any OS playback error is ignored and does not affect
/// save results or caller execution.
pub fn play_clip_saved_sound() {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};

        // Static buffer pointer is valid for the lifetime of the process.
        let sound_ptr = CLIP_SAVED_WAV.as_ptr() as *const u16;
        unsafe {
            let _ = PlaySoundW(
                sound_ptr,
                std::ptr::null_mut(),
                SND_ASYNC | SND_MEMORY | SND_NODEFAULT,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
