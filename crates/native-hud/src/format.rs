//! Formatting and text utilities for the Native HUD.

/// Fixed buffer capacity for title line UTF-16 code units (28 scalars * max 2 + ellipsis).
pub const TITLE_UTF16_BUF_SIZE: usize = 64;

/// Fixed buffer capacity for subtitle line UTF-16 code units (38 scalars * max 2 + ellipsis).
pub const SUBTITLE_UTF16_BUF_SIZE: usize = 128;

/// Formats a millisecond duration as `MMm SSs` per the visual specification (Section 4.2).
pub fn format_duration(duration_ms: u64) -> String {
    let total_secs = duration_ms / 1000;
    let mins = total_secs / 60;
    let secs = total_secs % 60;
    format!("{mins:02}m {secs:02}s")
}

/// Formats a byte size bounded to 1 decimal place (e.g., `94.2 MB`, `1.2 GB`).
pub fn format_file_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    const TB: f64 = GB * 1024.0;

    let b = bytes as f64;
    if b < KB {
        format!("{bytes} B")
    } else if b < MB {
        format!("{:.1} KB", b / KB)
    } else if b < GB {
        format!("{:.1} MB", b / MB)
    } else if b < TB {
        format!("{:.1} GB", b / GB)
    } else {
        format!("{:.1} TB", b / TB)
    }
}

/// Formats duration and file size with the standard bullet separator (` • `).
pub fn format_saved_subtitle(duration_ms: u64, bytes: u64) -> String {
    format!(
        "{} \u{2022} {}",
        format_duration(duration_ms),
        format_file_size(bytes)
    )
}

/// Truncates text with an ellipsis (`…`) if it exceeds `max_chars` UTF-8 scalar values.
pub fn truncate_with_ellipsis(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_string();
    }
    if max_chars == 0 {
        return String::new();
    }
    if max_chars == 1 {
        return "…".to_string();
    }

    let mut result = String::with_capacity(max_chars * 4);
    for (i, ch) in text.chars().enumerate() {
        if i >= max_chars - 1 {
            break;
        }
        result.push(ch);
    }
    result.push('…');
    result
}

/// Encodes `text` into `out_buf` in UTF-16, truncating to at most `max_chars` Unicode scalar values.
/// If truncated, appends the ellipsis character `…` (U+2026) as the final scalar.
/// Never splits UTF-16 surrogate pairs.
/// Returns the number of `u16` code units written.
pub fn encode_truncated_utf16(text: &str, max_chars: usize, out_buf: &mut [u16]) -> usize {
    if max_chars == 0 || out_buf.is_empty() {
        return 0;
    }

    let char_count = text.chars().count();
    if char_count <= max_chars {
        let mut written = 0;
        for ch in text.chars() {
            let mut tmp = [0u16; 2];
            let encoded = ch.encode_utf16(&mut tmp);
            if written + encoded.len() > out_buf.len() {
                break;
            }
            for &u in encoded.iter() {
                out_buf[written] = u;
                written += 1;
            }
        }
        return written;
    }

    // Truncate to (max_chars - 1) scalar values, then append '…' (0x2026)
    let take_chars = max_chars.saturating_sub(1);
    let mut written = 0;
    for (i, ch) in text.chars().enumerate() {
        if i >= take_chars {
            break;
        }
        let mut tmp = [0u16; 2];
        let encoded = ch.encode_utf16(&mut tmp);
        if written + encoded.len() + 1 > out_buf.len() {
            break;
        }
        for &u in encoded.iter() {
            out_buf[written] = u;
            written += 1;
        }
    }

    if written < out_buf.len() {
        out_buf[written] = 0x2026; // '…'
        written += 1;
    }
    written
}

/// Writes `val` as decimal ASCII digits in UTF-16 into `buf`.
/// Returns the number of `u16` code units written.
pub fn format_u64_into_utf16(mut val: u64, buf: &mut [u16]) -> usize {
    if val == 0 {
        if !buf.is_empty() {
            buf[0] = b'0' as u16;
            return 1;
        }
        return 0;
    }

    let mut temp = [0u16; 20];
    let mut len = 0;
    while val > 0 && len < temp.len() {
        temp[len] = (b'0' + (val % 10) as u8) as u16;
        val /= 10;
        len += 1;
    }

    let to_write = len.min(buf.len());
    for i in 0..to_write {
        buf[i] = temp[len - 1 - i];
    }
    to_write
}

/// Formats duration as `MMm SSs` directly into `buf` with zero allocations.
/// Returns the number of `u16` code units written.
pub fn format_duration_into_utf16(duration_ms: u64, buf: &mut [u16]) -> usize {
    let total_secs = duration_ms / 1000;
    let mins = total_secs / 60;
    let secs = total_secs % 60;

    let mut written = 0;

    // Minutes (at least 2 digits)
    if mins < 10 {
        if written < buf.len() {
            buf[written] = b'0' as u16;
            written += 1;
        }
        written += format_u64_into_utf16(mins, &mut buf[written..]);
    } else {
        written += format_u64_into_utf16(mins, &mut buf[written..]);
    }

    if written < buf.len() {
        buf[written] = b'm' as u16;
        written += 1;
    }
    if written < buf.len() {
        buf[written] = b' ' as u16;
        written += 1;
    }

    // Seconds (2 digits)
    if secs < 10 {
        if written < buf.len() {
            buf[written] = b'0' as u16;
            written += 1;
        }
        written += format_u64_into_utf16(secs, &mut buf[written..]);
    } else {
        written += format_u64_into_utf16(secs, &mut buf[written..]);
    }

    if written < buf.len() {
        buf[written] = b's' as u16;
        written += 1;
    }

    written
}

/// Formats byte size bounded to 1 decimal place directly into `buf` with zero allocations.
/// Returns the number of `u16` code units written.
pub fn format_file_size_into_utf16(bytes: u64, buf: &mut [u16]) -> usize {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    const TB: f64 = GB * 1024.0;

    let b = bytes as f64;
    let mut written = 0;

    if b < KB {
        written += format_u64_into_utf16(bytes, buf);
        if written < buf.len() {
            buf[written] = b' ' as u16;
            written += 1;
        }
        if written < buf.len() {
            buf[written] = b'B' as u16;
            written += 1;
        }
        return written;
    }

    let (val, unit) = if b < MB {
        (b / KB, "KB")
    } else if b < GB {
        (b / MB, "MB")
    } else if b < TB {
        (b / GB, "GB")
    } else {
        (b / TB, "TB")
    };

    // Calculate whole and single decimal fraction digit with rounding
    let mut whole = val.floor() as u64;
    let mut frac = ((val - whole as f64) * 10.0 + 0.5).floor() as u64;
    if frac >= 10 {
        whole += 1;
        frac = 0;
    }

    written += format_u64_into_utf16(whole, buf);
    if written < buf.len() {
        buf[written] = b'.' as u16;
        written += 1;
    }
    if written < buf.len() {
        buf[written] = (b'0' + frac as u8) as u16;
        written += 1;
    }
    if written < buf.len() {
        buf[written] = b' ' as u16;
        written += 1;
    }
    for byte in unit.bytes() {
        if written < buf.len() {
            buf[written] = byte as u16;
            written += 1;
        }
    }

    written
}

/// Formats `{duration} • {file_size}` directly into `buf` with zero allocations.
/// Returns the number of `u16` code units written.
pub fn format_saved_subtitle_into_utf16(duration_ms: u64, bytes: u64, buf: &mut [u16]) -> usize {
    let mut written = format_duration_into_utf16(duration_ms, buf);

    // Append " • "
    if written < buf.len() {
        buf[written] = b' ' as u16;
        written += 1;
    }
    if written < buf.len() {
        buf[written] = 0x2022; // '•' (U+2022)
        written += 1;
    }
    if written < buf.len() {
        buf[written] = b' ' as u16;
        written += 1;
    }

    if written < buf.len() {
        written += format_file_size_into_utf16(bytes, &mut buf[written..]);
    }

    written
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_formatting() {
        assert_eq!(format_duration(0), "00m 00s");
        assert_eq!(format_duration(45_000), "00m 45s");
        assert_eq!(format_duration(60_000), "01m 00s");
        assert_eq!(format_duration(90_000), "01m 30s");
        assert_eq!(format_duration(3_665_000), "61m 05s");
    }

    #[test]
    fn fixed_buffer_duration_formatting_matches_allocating() {
        let test_cases = [0, 45_000, 60_000, 90_000, 3_665_000, 7_200_000];
        for &dur in &test_cases {
            let mut buf = [0u16; 32];
            let len = format_duration_into_utf16(dur, &mut buf);
            let result_str = String::from_utf16(&buf[..len]).unwrap();
            assert_eq!(result_str, format_duration(dur));
        }
    }

    #[test]
    fn file_size_formatting() {
        assert_eq!(format_file_size(0), "0 B");
        assert_eq!(format_file_size(512), "512 B");
        assert_eq!(format_file_size(1024), "1.0 KB");
        assert_eq!(format_file_size(1536), "1.5 KB");
        assert_eq!(format_file_size(94_200_000), "89.8 MB");
        assert_eq!(format_file_size(1_073_741_824), "1.0 GB");
        assert_eq!(format_file_size(1_288_490_188), "1.2 GB");
    }

    #[test]
    fn fixed_buffer_file_size_formatting_matches_allocating() {
        let test_cases = [
            0,
            512,
            1024,
            1536,
            94_200_000,
            1_073_741_824,
            1_288_490_188,
            2_147_483_648,
        ];
        for &b in &test_cases {
            let mut buf = [0u16; 32];
            let len = format_file_size_into_utf16(b, &mut buf);
            let result_str = String::from_utf16(&buf[..len]).unwrap();
            assert_eq!(result_str, format_file_size(b));
        }
    }

    #[test]
    fn saved_subtitle_formatting() {
        assert_eq!(
            format_saved_subtitle(60_000, 98_775_000),
            "01m 00s • 94.2 MB"
        );
    }

    #[test]
    fn fixed_buffer_saved_subtitle_matches_allocating() {
        let mut buf = [0u16; 64];
        let len = format_saved_subtitle_into_utf16(60_000, 98_775_000, &mut buf);
        let result_str = String::from_utf16(&buf[..len]).unwrap();
        assert_eq!(result_str, format_saved_subtitle(60_000, 98_775_000));
    }

    #[test]
    fn text_truncation() {
        assert_eq!(truncate_with_ellipsis("Hello", 10), "Hello");
        assert_eq!(truncate_with_ellipsis("Hello World", 5), "Hell…");
        assert_eq!(truncate_with_ellipsis("A", 1), "A");
        assert_eq!(truncate_with_ellipsis("AB", 1), "…");
        assert_eq!(truncate_with_ellipsis("AB", 0), "");
    }

    #[test]
    fn fixed_buffer_unicode_surrogate_pairs_truncation() {
        // String with 4-byte surrogate pair emojis: 🎮 (U+1F3AE), 🦀 (U+1F980), 🚀 (U+1F680)
        let text = "🎮🦀🚀 Clip Captured";
        // 3 emojis + 1 space + 13 chars = 17 scalar characters
        assert_eq!(text.chars().count(), 17);

        let mut buf = [0u16; 64];

        // 1. No truncation (max_chars >= 17)
        let len = encode_truncated_utf16(text, 20, &mut buf);
        let str_full = String::from_utf16(&buf[..len]).unwrap();
        assert_eq!(str_full, text);

        // 2. Truncation at 3 characters: should take 2 emojis + ellipsis
        let len = encode_truncated_utf16(text, 3, &mut buf);
        let str_trunc = String::from_utf16(&buf[..len]).unwrap();
        assert_eq!(str_trunc, "🎮🦀…");

        // Verify surrogate pairs were NOT split
        for code_unit in &buf[..len] {
            assert!(*code_unit != 0);
        }
        assert!(String::from_utf16(&buf[..len]).is_ok());

        // 3. Truncation at 1 character
        let len = encode_truncated_utf16(text, 1, &mut buf);
        let str_one = String::from_utf16(&buf[..len]).unwrap();
        assert_eq!(str_one, "…");
    }
}
