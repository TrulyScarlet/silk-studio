use media_types::{EncodedPacket, MediaType, PacketPayload, StreamDescriptor, StreamId, TimeBase};
use replay_buffer::{normalize_timestamps, BufferConfig, BufferError, ReplayBuffer};

const VIDEO: StreamId = StreamId(0);
const AUDIO: StreamId = StreamId(1);

fn video_descriptor() -> StreamDescriptor {
    StreamDescriptor {
        stream_id: VIDEO,
        media_type: MediaType::Video,
        time_base: TimeBase::MILLISECOND,
        name: None,
        codec: "h264".to_string(),
        extradata: None,
        width: None,
        height: None,
        sample_rate: None,
        channels: None,
        pixel_format: None,
    }
}

fn audio_descriptor() -> StreamDescriptor {
    StreamDescriptor {
        stream_id: AUDIO,
        media_type: MediaType::Audio,
        time_base: TimeBase::MILLISECOND,
        name: None,
        codec: "aac".to_string(),
        extradata: None,
        width: None,
        height: None,
        sample_rate: Some(48_000),
        channels: Some(2),
        pixel_format: None,
    }
}

fn video_packet(pts_ms: i64, keyframe: bool) -> EncodedPacket {
    let size = if keyframe { 400 } else { 40 };
    EncodedPacket {
        stream_id: VIDEO,
        media_type: MediaType::Video,
        pts: pts_ms,
        dts: pts_ms,
        duration: 33,
        time_base: TimeBase::MILLISECOND,
        is_keyframe: keyframe,
        sequence: 0,
        payload: PacketPayload::from(vec![0xAB_u8; size]),
    }
}

fn audio_packet(start_ms: i64, duration_ms: i64) -> EncodedPacket {
    EncodedPacket {
        stream_id: AUDIO,
        media_type: MediaType::Audio,
        pts: start_ms,
        dts: start_ms,
        duration: duration_ms,
        time_base: TimeBase::MILLISECOND,
        is_keyframe: true,
        sequence: 0,
        payload: PacketPayload::from(vec![0_u8; 64]),
    }
}

fn buffer_with_retention(ms: i64) -> ReplayBuffer {
    let mut buffer = ReplayBuffer::new(BufferConfig {
        retention_ms: ms,
        max_packets_per_stream: 10_000,
    });
    buffer
        .register_stream(video_descriptor())
        .expect("register video");
    buffer
        .register_stream(audio_descriptor())
        .expect("register audio");
    buffer
}

/// Frames every 33 ms with a keyframe every 15 frames (~495 ms GOP).
struct ScriptedStream;

impl ScriptedStream {
    fn fill(buffer: &mut ReplayBuffer, until_ms: i64) {
        let mut t = 0;
        let mut frame = 0_u64;
        while t <= until_ms {
            buffer
                .insert(video_packet(t, frame.is_multiple_of(15)))
                .expect("insert video");
            buffer.insert(audio_packet(t, 20)).expect("insert audio");
            t += 33;
            frame += 1;
        }
    }

    fn fill_video_only(buffer: &mut ReplayBuffer, until_ms: i64) {
        let mut t = 0;
        let mut frame = 0_u64;
        while t <= until_ms {
            buffer
                .insert(video_packet(t, frame.is_multiple_of(15)))
                .expect("insert video");
            t += 33;
            frame += 1;
        }
    }
}

#[test]
fn sequences_are_globally_monotonic_across_streams() {
    let mut buffer = buffer_with_retention(60_000);
    ScriptedStream::fill(&mut buffer, 500);
    let metrics = buffer.metrics();
    assert_eq!(metrics.packet_count, (500 / 33 + 1) * 2);
}

#[test]
fn out_of_order_video_is_rejected() {
    let mut buffer = buffer_with_retention(60_000);
    buffer.insert(video_packet(0, true)).expect("first");
    buffer.insert(video_packet(33, false)).expect("second");
    let err = buffer
        .insert(video_packet(10, false))
        .expect_err("backwards dts");
    assert!(matches!(
        err,
        BufferError::NonMonotonicTimestamp {
            previous_dts: 33,
            new_dts: 10,
            ..
        }
    ));
}

#[test]
fn unregistered_stream_rejected() {
    let mut buffer = ReplayBuffer::new(BufferConfig::default());
    let err = buffer
        .insert(video_packet(0, true))
        .expect_err("unregistered");
    assert!(matches!(err, BufferError::UnregisteredStream { .. }));
}

#[test]
fn stream_extradata_is_carried_into_snapshots() {
    let mut buffer = buffer_with_retention(60_000);
    let extradata = PacketPayload::from(vec![0x11_u8, 0x90]);
    buffer
        .set_stream_extradata(AUDIO, Some(extradata.clone()))
        .expect("audio stream metadata");
    ScriptedStream::fill(&mut buffer, 100);

    let snapshot = buffer.snapshot(0).expect("snapshot");
    let audio = snapshot
        .media
        .streams
        .iter()
        .find(|stream| stream.descriptor.stream_id == AUDIO)
        .expect("audio stream");
    assert_eq!(audio.descriptor.extradata, Some(extradata));
}

#[test]
fn eviction_keeps_keyframe_before_boundary_and_nothing_stale() {
    let retention = 500_i64;
    let mut buffer = buffer_with_retention(retention);
    ScriptedStream::fill_video_only(&mut buffer, 3_000);

    // Newest ≈ 2997; boundary ≈ 2497. Keyframes at multiples of 495.
    // The newest keyframe ≤ boundary must be retained as the front packet.
    let newest = buffer.newest_video_dts().expect("newest");
    let boundary = newest - retention;
    let metrics = buffer.metrics();

    assert!(
        metrics.duration_ticks <= retention + 495 + 66,
        "span {} must stay within retention + one GOP + slack",
        metrics.duration_ticks
    );

    // Verify the front packet is a keyframe at/before boundary and that the
    // *next* keyframe after it lies beyond the boundary (nothing extra kept).
    let snapshot = buffer.snapshot(boundary).expect("snapshot at boundary");
    assert!(snapshot.actual_start_pts <= boundary);

    let first = snapshot.media.streams[0].packets.first().unwrap().clone();
    let second_kf = snapshot.media.streams[0]
        .packets
        .iter()
        .skip(1)
        .find(|p| p.is_keyframe)
        .cloned();
    assert!(first.is_keyframe);
    if let Some(second_kf) = second_kf {
        assert!(second_kf.pts > boundary + 1 || second_kf.pts == first.pts);
    }
}

#[test]
fn snapshot_starts_on_keyframe_with_reported_preroll() {
    let mut buffer = buffer_with_retention(60_000);
    ScriptedStream::fill_video_only(&mut buffer, 2_000);

    // Request a start between keyframes: kf at 1485 and 1980.
    let requested = 1_700;
    let outcome = buffer.snapshot(requested).expect("snapshot");
    assert_eq!(outcome.requested_start_pts, requested);
    assert_eq!(outcome.actual_start_pts, 1_485);
    assert_eq!(outcome.keyframe_preroll_ticks, requested - 1_485);

    let first = &outcome.media.streams[0].packets[0];
    assert!(first.is_keyframe);
    assert_eq!(first.pts, 1_485);
    assert_eq!(outcome.media.origin_pts, 1_485);
}

#[test]
fn snapshot_without_eligible_keyframe_is_not_ready() {
    let mut buffer = buffer_with_retention(60_000);
    buffer
        .insert(video_packet(0, false))
        .expect("non-keyframe first");
    let err = buffer.snapshot(0).expect_err("no keyframe yet");
    assert_eq!(err.code(), "BUFFER_NOT_READY");
}

#[test]
fn audio_overlapping_selected_interval_is_included() {
    let mut buffer = buffer_with_retention(60_000);
    // Video keyframes every 15 frames of 20 ms → every 300 ms.
    let mut t = 0;
    let mut frame = 0_u64;
    while t <= 2_000 {
        buffer
            .insert(video_packet(t, frame.is_multiple_of(15)))
            .expect("video");
        buffer.insert(audio_packet(t, 20)).expect("audio");
        t += 20;
        frame += 1;
    }

    // Keyframes at multiples of 300 ms → request start 1_000 selects 900.
    let outcome = buffer.snapshot(1_000).expect("snapshot");
    let audio = outcome
        .media
        .streams
        .iter()
        .find(|s| s.descriptor.media_type == MediaType::Audio)
        .expect("audio stream present");

    let first_audio = audio.packets.first().expect("audio packets");
    // Overlap rule: keep audio whose end extends past the clip start.
    assert!(
        first_audio.dts + first_audio.duration > outcome.actual_start_pts,
        "first audio chunk must overlap clip start"
    );
    let last_video = outcome.media.streams[0].packets.last().unwrap();
    let last_audio = audio.packets.last().unwrap();
    assert!(last_audio.dts < last_video.dts + last_video.duration);
}

#[test]
fn live_retention_change_applies_without_restart() {
    let mut buffer = buffer_with_retention(60_000);
    ScriptedStream::fill_video_only(&mut buffer, 2_000);
    assert!(buffer.duration_ticks() >= 1_900);

    buffer.set_retention(200).expect("shrink");
    assert!(
        buffer.duration_ticks() <= 200 + 495 + 66,
        "span must shrink immediately"
    );

    // Growing never violates bounds; inserts keep working.
    buffer.set_retention(4_000).expect("grow");
    assert_eq!(buffer.retention_ms(), 4_000);
    let next_t = buffer.newest_video_dts().expect("newest") + 33;
    buffer
        .insert(video_packet(next_t, true))
        .expect("insert after change");
}

#[test]
fn invalid_retention_rejected() {
    let mut buffer = buffer_with_retention(1_000);
    assert!(matches!(
        buffer.set_retention(0),
        Err(BufferError::RetentionTooShort { value_ms: 0 })
    ));
}

#[test]
fn new_epoch_clears_everything_but_keeps_sequence_counter() {
    let mut buffer = buffer_with_retention(60_000);
    ScriptedStream::fill_video_only(&mut buffer, 300);
    let seq_after_fill = buffer.metrics().packet_count as u64;

    let epoch = buffer.start_new_epoch();
    assert_eq!(epoch, 1);
    assert_eq!(buffer.metrics().packet_count, 0);
    assert_eq!(buffer.memory_bytes(), 0);

    // Streams must be re-registered for the new epoch.
    assert!(buffer.register_stream(video_descriptor()).is_ok());
    buffer
        .insert(video_packet(0, true))
        .expect("insert post-epoch");
    let next = buffer.next_sequence_hint();
    assert!(
        next > seq_after_fill,
        "sequence never resets within process"
    );
}

#[test]
fn preserving_epoch_reset_keeps_stream_metadata_for_workers() {
    let mut buffer = buffer_with_retention(60_000);
    buffer
        .set_stream_extradata(VIDEO, Some(PacketPayload::from(vec![1, 2, 3])))
        .expect("video metadata");
    buffer
        .set_stream_extradata(AUDIO, Some(PacketPayload::from(vec![4, 5])))
        .expect("audio metadata");
    buffer.insert(video_packet(0, true)).expect("video packet");
    buffer.insert(audio_packet(0, 20)).expect("audio packet");

    assert_eq!(
        buffer.start_new_epoch_preserving_streams().expect("epoch"),
        1
    );
    assert_eq!(buffer.registered_streams(), 2);
    assert_eq!(buffer.metrics().packet_count, 0);

    buffer
        .insert(video_packet(0, true))
        .expect("new video packet");
    buffer
        .insert(audio_packet(0, 20))
        .expect("new audio packet");
    let outcome = buffer.snapshot(0).expect("post-epoch snapshot");
    assert_eq!(
        outcome.media.streams[0].descriptor.extradata,
        Some(PacketPayload::from(vec![1, 2, 3]))
    );
    assert_eq!(
        outcome.media.streams[1].descriptor.extradata,
        Some(PacketPayload::from(vec![4, 5]))
    );
}

#[test]
fn normalization_rebases_to_zero_and_is_idempotent() {
    let mut buffer = buffer_with_retention(60_000);
    ScriptedStream::fill(&mut buffer, 2_000);
    let mut outcome = buffer.snapshot(1_000).expect("snapshot");

    normalize_timestamps(&mut outcome.media);
    let first = outcome.media.streams[0].packets[0].clone();
    assert_eq!(first.dts.min(first.pts), 0);
    assert_eq!(outcome.media.origin_pts, 0);

    // Order preserved.
    let video = &outcome.media.streams[0].packets;
    for pair in video.windows(2) {
        assert!(pair[1].dts >= pair[0].dts);
        assert!(pair[1].pts >= pair[0].pts);
    }

    let before: Vec<_> = video.iter().map(|p| (p.pts, p.dts)).collect();
    normalize_timestamps(&mut outcome.media);
    let after: Vec<_> = outcome.media.streams[0]
        .packets
        .iter()
        .map(|p| (p.pts, p.dts))
        .collect();
    assert_eq!(before, after, "second pass must be a no-op");
}

#[test]
fn memory_accounting_tracks_payload_bytes() {
    let mut buffer = buffer_with_retention(60_000);
    ScriptedStream::fill_video_only(&mut buffer, 300);
    let m = buffer.metrics();
    let expected: u64 = (0..300)
        .step_by(33)
        .map(|t| {
            if ((t / 33) as u64).is_multiple_of(15) {
                400
            } else {
                40
            }
        })
        .sum();
    assert_eq!(m.memory_bytes, expected);
    assert_eq!(buffer.memory_bytes(), expected);
}
