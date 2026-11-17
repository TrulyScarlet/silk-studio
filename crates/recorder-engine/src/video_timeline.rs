use std::time::{Duration, Instant};

use media_types::{TimeBase, VideoFrame};

pub(crate) fn signed_elapsed_ms(at: Instant, origin: Instant) -> i64 {
    if at >= origin {
        at.duration_since(origin).as_millis().min(i64::MAX as u128) as i64
    } else {
        -(origin.duration_since(at).as_millis().min(i64::MAX as u128) as i64)
    }
}

pub(crate) fn map_capture_timestamp(frame: &mut VideoFrame, source: Instant, session: Instant) {
    let offset = signed_elapsed_ms(source, session);
    frame.pts = frame
        .pts
        .saturating_add(TimeBase::MILLISECOND.rescale(offset, frame.time_base))
        .max(0);
}

/// Keeps restarted capture sessions on the recording's existing timeline.
/// Audio workers keep running while a display is recovered or while auto-game
/// is waiting for focus. A new video source origin must therefore align with
/// the recording's elapsed timeline, rather than starting at zero or joining
/// directly onto an obsolete pre-recovery timestamp.
#[derive(Default)]
pub(crate) struct VideoTimeline {
    capture_started_at: Option<Instant>,
    offset_ms: i64,
    restart_pending: bool,
    explicit_restart_ms: Option<i64>,
    last_source_ms: Option<i64>,
    last_ms: Option<i64>,
}

impl VideoTimeline {
    /// Construct a timeline anchored to the recording start instant.
    /// The first arriving frame will align with the recording's elapsed time,
    /// ensuring audio and video share the exact same origin even after startup
    /// or window-focus delays.
    pub(crate) fn with_origin(capture_started_at: Instant) -> Self {
        Self {
            capture_started_at: Some(capture_started_at),
            offset_ms: 0,
            restart_pending: true,
            explicit_restart_ms: None,
            last_source_ms: None,
            last_ms: None,
        }
    }

    /// Request that the next arriving frame re-anchors to the recording's
    /// current elapsed timeline.
    pub(crate) fn request_restart(&mut self) {
        self.restart_pending = true;
        self.explicit_restart_ms = None;
    }

    /// Called immediately after capture startup succeeds, before encoder
    /// configuration. Queue/encoder latency must not accumulate per restart.
    #[allow(dead_code)]
    pub(crate) fn restarted(&mut self, elapsed: Duration) {
        let elapsed_ms = elapsed.as_millis().min(i64::MAX as u128) as i64;
        self.restart_pending = true;
        self.explicit_restart_ms = Some(
            self.last_ms
                .map_or(elapsed_ms, |last| elapsed_ms.max(last.saturating_add(1))),
        );
    }

    pub(crate) fn map(&mut self, frame: &mut VideoFrame) {
        let source_ms = frame.time_base.rescale(frame.pts, TimeBase::MILLISECOND);

        let current_elapsed = self
            .capture_started_at
            .map(|origin| origin.elapsed().as_millis().min(i64::MAX as u128) as i64);
        let elapsed_gap = match (current_elapsed, self.last_ms) {
            (Some(elapsed), Some(last)) => elapsed.saturating_sub(last) > 1000,
            _ => false,
        };
        let source_reset = self.last_source_ms.is_some_and(|last| source_ms < last);
        let should_anchor =
            self.restart_pending || self.last_ms.is_none() || source_reset || elapsed_gap;

        if should_anchor {
            self.restart_pending = false;
            let target_ms = if let Some(explicit) = self.explicit_restart_ms.take() {
                explicit
            } else if let Some(elapsed) = current_elapsed {
                self.last_ms
                    .map_or(elapsed, |last| elapsed.max(last.saturating_add(1)))
            } else {
                self.last_ms
                    .map_or(source_ms, |last| last.saturating_add(1))
            };
            self.offset_ms = target_ms.saturating_sub(source_ms);
        }

        frame.pts = frame
            .pts
            .saturating_add(TimeBase::MILLISECOND.rescale(self.offset_ms, frame.time_base));
        let mapped_ms = frame.time_base.rescale(frame.pts, TimeBase::MILLISECOND);
        self.last_source_ms = Some(source_ms);
        self.last_ms = Some(mapped_ms);
    }

    /// Last shared stamp accepted on this timeline.
    #[allow(dead_code)]
    pub(crate) fn last_ms(&self) -> Option<i64> {
        self.last_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use media_types::{FramePayload, PixelFormat, StreamId};
    use std::sync::Arc;

    #[test]
    fn correlated_streams_preserve_startup_queue_and_recovery_time() {
        let epoch = Instant::now();
        let session = epoch + Duration::from_secs(2);
        for event_ms in [2500, 30_000, 150_000] {
            let mut video = frame(event_ms, TimeBase::MILLISECOND);
            map_capture_timestamp(&mut video, epoch, session);
            let audio = signed_elapsed_ms(epoch + Duration::from_millis(event_ms as u64), session);
            assert_eq!(video.pts, audio);
            // Delivery time is intentionally not an input: queued packets and
            // independent worker startup must not change the capture timestamp.
            assert_eq!(video.pts, event_ms - 2000);
        }
    }

    fn frame(pts: i64, time_base: TimeBase) -> VideoFrame {
        VideoFrame {
            stream_id: StreamId(0),
            width: 2,
            height: 2,
            pixel_format: PixelFormat::Nv12,
            pts,
            time_base,
            payload: FramePayload::Cpu(Arc::from([0_u8; 6])),
        }
    }

    #[test]
    fn recovery_keeps_downtime_and_does_not_accumulate_previous_offsets() {
        let mut timeline = VideoTimeline::default();
        for (restart_ms, source_pts, expected) in [
            (None, 0, 0),
            (None, 10_000, 10_000),
            (Some(10_500), 0, 10_500),
            (None, 33, 10_533),
            (Some(12_000), 0, 12_000),
            (None, 33, 12_033),
        ] {
            if let Some(ms) = restart_ms {
                timeline.restarted(Duration::from_millis(ms));
            }
            let mut frame = frame(source_pts, TimeBase::MILLISECOND);
            timeline.map(&mut frame);
            assert_eq!(frame.pts, expected);
        }
    }

    #[test]
    fn recovery_accepts_new_source_origin_and_time_base_without_moving_backwards() {
        let mut timeline = VideoTimeline::default();
        let mut first = frame(10_000, TimeBase::MILLISECOND);
        timeline.map(&mut first);
        timeline.restarted(Duration::from_secs(9));
        let mut next = frame(90_000, TimeBase::from_hz(90_000));
        timeline.map(&mut next);
        assert_eq!(next.pts, 900_090);
        next.pts = 93_000;
        timeline.map(&mut next);
        assert_eq!(next.pts, 903_090);
    }

    #[test]
    fn origin_timeline_anchors_first_frame_to_elapsed_time() {
        let start = Instant::now() - Duration::from_millis(500);
        let mut timeline = VideoTimeline::with_origin(start);
        let mut first = frame(0, TimeBase::MILLISECOND);
        timeline.map(&mut first);
        // First frame should be placed at or after 500ms (the elapsed time since recording started)
        assert!(first.pts >= 500, "expected >= 500ms, got {}", first.pts);

        let mut second = frame(16, TimeBase::MILLISECOND);
        timeline.map(&mut second);
        assert_eq!(second.pts, first.pts + 16);
    }

    #[test]
    fn source_clock_reset_reanchors_without_moving_backwards() {
        let mut timeline = VideoTimeline::default();
        let mut f1 = frame(0, TimeBase::MILLISECOND);
        timeline.map(&mut f1);
        let mut f2 = frame(10_000, TimeBase::MILLISECOND);
        timeline.map(&mut f2);
        assert_eq!(f2.pts, 10_000);

        // Underlying capture source restarts with pts=0 (e.g. new window selected)
        let mut f3 = frame(0, TimeBase::MILLISECOND);
        timeline.map(&mut f3);
        assert!(
            f3.pts >= 10_001,
            "expected monotonic advance, got {}",
            f3.pts
        );

        let mut f4 = frame(16, TimeBase::MILLISECOND);
        timeline.map(&mut f4);
        assert_eq!(f4.pts, f3.pts + 16);
    }
}
