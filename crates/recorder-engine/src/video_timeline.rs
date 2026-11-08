use std::time::Duration;

use media_types::{TimeBase, VideoFrame};

/// Keeps restarted capture sessions on the recording's existing timeline.
/// Audio workers keep running while a display is recovered. A new video
/// source origin must therefore include the elapsed recovery time, rather
/// than starting at zero or joining directly onto the preceding frame.
#[derive(Default)]
pub(crate) struct VideoTimeline {
    offset_ms: i64,
    restart_at_ms: Option<i64>,
    last_ms: Option<i64>,
}

impl VideoTimeline {
    /// Called immediately after capture startup succeeds, before encoder
    /// configuration. Queue/encoder latency must not accumulate per restart.
    pub(crate) fn restarted(&mut self, elapsed: Duration) {
        let elapsed_ms = elapsed.as_millis().min(i64::MAX as u128) as i64;
        self.restart_at_ms = Some(
            self.last_ms
                .map_or(elapsed_ms, |last| elapsed_ms.max(last.saturating_add(1))),
        );
    }

    pub(crate) fn map(&mut self, frame: &mut VideoFrame) {
        if let Some(restart_ms) = self.restart_at_ms.take() {
            let source_ms = frame.time_base.rescale(frame.pts, TimeBase::MILLISECOND);
            self.offset_ms = restart_ms.saturating_sub(source_ms);
        }
        frame.pts = frame
            .pts
            .saturating_add(TimeBase::MILLISECOND.rescale(self.offset_ms, frame.time_base));
        self.last_ms = Some(frame.time_base.rescale(frame.pts, TimeBase::MILLISECOND));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use media_types::{FramePayload, PixelFormat, StreamId};
    use std::sync::Arc;

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
}
