use crate::{DecodedVideoFrame, FrameEnqueueOutcome, VideoFrameQueue};

/// Basic transport state for a timestamp-scheduled video source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackState {
    Stopped,
    Playing,
    Paused,
    Ended,
}

/// Invalid time input or operation for the deterministic playback scheduler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackError {
    NegativeMonotonicTime,
    MonotonicTimeMovedBackwards,
    NegativeSeekPosition,
    InvalidFrameTiming,
    TimeOverflow,
    SourceAlreadyEnded,
}

impl std::fmt::Display for PlaybackError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::NegativeMonotonicTime => "monotonic playback time cannot be negative",
            Self::MonotonicTimeMovedBackwards => "monotonic playback time moved backwards",
            Self::NegativeSeekPosition => "seek position cannot be negative",
            Self::InvalidFrameTiming => "video frame timing is invalid",
            Self::TimeOverflow => "playback time exceeded the supported range",
            Self::SourceAlreadyEnded => "video source has already reached end of stream",
        })
    }
}

impl std::error::Error for PlaybackError {}

/// Maps monotonic wall-clock ticks to media time and selects due video frames.
///
/// All times use 100 ns units. Callers pass a monotonic clock value to each
/// operation; wall-clock/calendar time must not be used. The scheduler uses a
/// three-frame queue and returns a frame only when its PTS is due. A seek or
/// restart clears queued frames so samples from the previous timeline cannot
/// be presented.
#[derive(Debug)]
pub struct PlaybackScheduler {
    queue: VideoFrameQueue,
    state: PlaybackState,
    media_anchor_100ns: i64,
    wall_anchor_100ns: Option<i64>,
    last_wall_time_100ns: Option<i64>,
    source_ended: bool,
    last_presented_end_100ns: Option<i64>,
}

impl Default for PlaybackScheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl PlaybackScheduler {
    pub fn new() -> Self {
        Self {
            queue: VideoFrameQueue::new(),
            state: PlaybackState::Stopped,
            media_anchor_100ns: 0,
            wall_anchor_100ns: None,
            last_wall_time_100ns: None,
            source_ended: false,
            last_presented_end_100ns: None,
        }
    }

    pub const fn state(&self) -> PlaybackState {
        self.state
    }

    pub fn queued_frames(&self) -> usize {
        self.queue.len()
    }

    /// Return the media timestamp corresponding to a monotonic clock sample.
    pub fn media_time_100ns(&mut self, monotonic_time_100ns: i64) -> Result<i64, PlaybackError> {
        self.current_media_time(monotonic_time_100ns)
    }

    /// Start playback or resume from the paused media position.
    pub fn play(&mut self, monotonic_time_100ns: i64) -> Result<(), PlaybackError> {
        self.observe_monotonic_time(monotonic_time_100ns)?;
        if self.state == PlaybackState::Playing {
            return Ok(());
        }
        let restart_from_end = self.state == PlaybackState::Ended;
        if restart_from_end {
            self.clear_timeline(0);
        }
        self.state = PlaybackState::Playing;
        if restart_from_end {
            self.media_anchor_100ns = 0;
        }
        self.wall_anchor_100ns = Some(monotonic_time_100ns);
        Ok(())
    }

    /// Pause without advancing media time until playback resumes.
    pub fn pause(&mut self, monotonic_time_100ns: i64) -> Result<(), PlaybackError> {
        self.observe_monotonic_time(monotonic_time_100ns)?;
        if self.state == PlaybackState::Playing {
            let media_time = match self.media_time_from_anchor(monotonic_time_100ns) {
                Ok(media_time) => media_time,
                Err(PlaybackError::TimeOverflow) => i64::MAX,
                Err(error) => return Err(error),
            };
            self.state = PlaybackState::Paused;
            self.media_anchor_100ns = media_time;
            self.wall_anchor_100ns = None;
        }
        Ok(())
    }

    /// Stop and reset to the beginning, discarding queued and EOS state.
    pub fn stop(&mut self, monotonic_time_100ns: i64) -> Result<(), PlaybackError> {
        self.observe_monotonic_time(monotonic_time_100ns)?;
        self.state = PlaybackState::Stopped;
        self.clear_timeline(0);
        self.wall_anchor_100ns = None;
        Ok(())
    }

    /// Move to a media timestamp, clearing decoded frames from the old timeline.
    ///
    /// The caller is responsible for seeking the media decoder to the same
    /// position. A playing scheduler continues from the new position; an ended
    /// scheduler becomes paused so it can be resumed explicitly.
    pub fn seek(
        &mut self,
        position_100ns: i64,
        monotonic_time_100ns: i64,
    ) -> Result<(), PlaybackError> {
        if position_100ns < 0 {
            return Err(PlaybackError::NegativeSeekPosition);
        }
        self.observe_monotonic_time(monotonic_time_100ns)?;
        if self.state == PlaybackState::Playing {
            self.wall_anchor_100ns = Some(monotonic_time_100ns);
        } else if self.state == PlaybackState::Ended {
            self.state = PlaybackState::Paused;
            self.wall_anchor_100ns = None;
        }
        self.clear_timeline(position_100ns);
        Ok(())
    }

    /// Reset the timeline and begin playback from zero.
    pub fn restart(&mut self, monotonic_time_100ns: i64) -> Result<(), PlaybackError> {
        self.observe_monotonic_time(monotonic_time_100ns)?;
        self.clear_timeline(0);
        self.state = PlaybackState::Playing;
        self.wall_anchor_100ns = Some(monotonic_time_100ns);
        Ok(())
    }

    /// Add a decoded frame to the bounded queue using the current media time
    /// to discard frames that have already missed their presentation interval.
    pub fn enqueue_frame(
        &mut self,
        frame: DecodedVideoFrame,
        monotonic_time_100ns: i64,
    ) -> Result<FrameEnqueueOutcome, PlaybackError> {
        let media_time = self.current_media_time(monotonic_time_100ns)?;
        if self.source_ended || self.state == PlaybackState::Ended {
            return Err(PlaybackError::SourceAlreadyEnded);
        }
        if frame.pts_100ns() < 0 || frame.duration_100ns() <= 0 {
            return Err(PlaybackError::InvalidFrameTiming);
        }
        frame
            .pts_100ns()
            .checked_add(frame.duration_100ns())
            .ok_or(PlaybackError::TimeOverflow)?;
        Ok(self.queue.enqueue(frame, media_time))
    }

    /// Mark that no more frames will arrive from the current source timeline.
    /// The state becomes `Ended` only after queued/presented frames have elapsed.
    pub fn end_of_stream(&mut self, monotonic_time_100ns: i64) -> Result<(), PlaybackError> {
        self.observe_monotonic_time(monotonic_time_100ns)?;
        self.source_ended = true;
        Ok(())
    }

    /// Return the newest queued frame whose PTS is due, if any.
    ///
    /// `None` means no new frame should be presented at this clock sample.
    /// Late and superseded frames are discarded by the bounded queue.
    pub fn next_due_frame(
        &mut self,
        monotonic_time_100ns: i64,
    ) -> Result<Option<DecodedVideoFrame>, PlaybackError> {
        let media_time = self.current_media_time(monotonic_time_100ns)?;
        if self.state != PlaybackState::Playing {
            return Ok(None);
        }

        if let Some(frame) = self.queue.pop_due(media_time) {
            self.last_presented_end_100ns = Some(
                frame
                    .pts_100ns()
                    .checked_add(frame.duration_100ns())
                    .ok_or(PlaybackError::TimeOverflow)?,
            );
            return Ok(Some(frame));
        }

        let last_frame_elapsed = self
            .last_presented_end_100ns
            .is_none_or(|end| media_time >= end);
        if self.source_ended && self.queue.is_empty() && last_frame_elapsed {
            self.state = PlaybackState::Ended;
            self.media_anchor_100ns = media_time;
            self.wall_anchor_100ns = None;
        }
        Ok(None)
    }

    fn current_media_time(&mut self, monotonic_time_100ns: i64) -> Result<i64, PlaybackError> {
        self.observe_monotonic_time(monotonic_time_100ns)?;
        if self.state != PlaybackState::Playing {
            return Ok(self.media_anchor_100ns);
        }
        self.media_time_from_anchor(monotonic_time_100ns)
    }

    fn media_time_from_anchor(&self, monotonic_time_100ns: i64) -> Result<i64, PlaybackError> {
        let wall_anchor = self.wall_anchor_100ns.ok_or(PlaybackError::TimeOverflow)?;
        let elapsed = monotonic_time_100ns
            .checked_sub(wall_anchor)
            .ok_or(PlaybackError::TimeOverflow)?;
        self.media_anchor_100ns
            .checked_add(elapsed)
            .ok_or(PlaybackError::TimeOverflow)
    }

    fn observe_monotonic_time(&mut self, now: i64) -> Result<(), PlaybackError> {
        if now < 0 {
            return Err(PlaybackError::NegativeMonotonicTime);
        }
        if self.last_wall_time_100ns.is_some_and(|last| now < last) {
            return Err(PlaybackError::MonotonicTimeMovedBackwards);
        }
        self.last_wall_time_100ns = Some(now);
        Ok(())
    }

    fn clear_timeline(&mut self, media_time_100ns: i64) {
        self.queue.clear();
        self.media_anchor_100ns = media_time_100ns;
        self.source_ended = false;
        self.last_presented_end_100ns = None;
    }
}

#[cfg(test)]
mod tests {
    use super::{PlaybackError, PlaybackScheduler, PlaybackState};
    use crate::{DecodedVideoFrame, FrameEnqueueOutcome, VideoPixelFormat};

    fn frame(pts_100ns: i64, duration_100ns: i64, marker: u8) -> DecodedVideoFrame {
        DecodedVideoFrame {
            width: 1,
            height: 1,
            stride: 4,
            pixel_format: VideoPixelFormat::Rgba8,
            rgba8: vec![marker, 0, 0, 255],
            pts_100ns,
            duration_100ns,
        }
    }

    #[test]
    fn playback_clock_uses_pts_and_does_not_jump_across_pause_resume() {
        let mut scheduler = PlaybackScheduler::new();
        scheduler.play(1_000).unwrap();
        assert_eq!(scheduler.media_time_100ns(1_050).unwrap(), 50);
        scheduler.pause(1_050).unwrap();
        assert_eq!(scheduler.media_time_100ns(9_000).unwrap(), 50);
        scheduler.play(9_000).unwrap();
        assert_eq!(scheduler.media_time_100ns(9_025).unwrap(), 75);

        scheduler.enqueue_frame(frame(125, 100, 1), 9_025).unwrap();
        assert_eq!(scheduler.next_due_frame(9_074).unwrap(), None);
        assert_eq!(
            scheduler
                .next_due_frame(9_075)
                .unwrap()
                .unwrap()
                .pts_100ns(),
            125
        );
    }

    #[test]
    fn late_frames_are_rejected_and_queue_never_exceeds_default_capacity() {
        let mut scheduler = PlaybackScheduler::new();
        scheduler.play(0).unwrap();
        assert_eq!(
            scheduler.enqueue_frame(frame(0, 10, 0), 10).unwrap(),
            FrameEnqueueOutcome::DroppedLate
        );
        for pts in [20, 30, 40, 50] {
            scheduler
                .enqueue_frame(frame(pts, 100, pts as u8), 10)
                .unwrap();
            assert!(scheduler.queued_frames() <= 3);
        }
        assert_eq!(scheduler.queued_frames(), 3);
        assert_eq!(
            scheduler.next_due_frame(50).unwrap().unwrap().pts_100ns(),
            50
        );
    }

    #[test]
    fn seek_and_restart_flush_frames_from_the_previous_timeline() {
        let mut scheduler = PlaybackScheduler::new();
        scheduler.play(0).unwrap();
        scheduler.enqueue_frame(frame(100, 100, 1), 0).unwrap();
        scheduler.seek(0, 10).unwrap();
        assert_eq!(scheduler.queued_frames(), 0);
        scheduler.enqueue_frame(frame(0, 100, 2), 10).unwrap();
        assert_eq!(scheduler.next_due_frame(10).unwrap().unwrap().rgba8()[0], 2);

        scheduler.enqueue_frame(frame(100, 100, 3), 10).unwrap();
        scheduler.restart(20).unwrap();
        assert_eq!(scheduler.state(), PlaybackState::Playing);
        assert_eq!(scheduler.queued_frames(), 0);
        assert_eq!(scheduler.media_time_100ns(20).unwrap(), 0);
        scheduler.enqueue_frame(frame(0, 100, 4), 20).unwrap();
        assert_eq!(scheduler.next_due_frame(20).unwrap().unwrap().rgba8()[0], 4);
    }

    #[test]
    fn stop_and_end_of_stream_have_deterministic_transitions() {
        let mut scheduler = PlaybackScheduler::new();
        scheduler.play(0).unwrap();
        scheduler.enqueue_frame(frame(0, 100, 1), 0).unwrap();
        assert_eq!(scheduler.next_due_frame(0).unwrap().unwrap().pts_100ns(), 0);
        scheduler.end_of_stream(0).unwrap();
        assert_eq!(scheduler.next_due_frame(99).unwrap(), None);
        assert_eq!(scheduler.state(), PlaybackState::Playing);
        assert_eq!(scheduler.next_due_frame(100).unwrap(), None);
        assert_eq!(scheduler.state(), PlaybackState::Ended);
        assert_eq!(
            scheduler.enqueue_frame(frame(100, 100, 2), 100),
            Err(PlaybackError::SourceAlreadyEnded)
        );

        scheduler.play(100).unwrap();
        assert_eq!(scheduler.state(), PlaybackState::Playing);
        assert_eq!(scheduler.media_time_100ns(100).unwrap(), 0);
        scheduler.stop(100).unwrap();
        assert_eq!(scheduler.state(), PlaybackState::Stopped);
        assert_eq!(scheduler.media_time_100ns(10_000).unwrap(), 0);
        assert_eq!(scheduler.queued_frames(), 0);
    }

    #[test]
    fn monotonic_clock_and_seek_inputs_are_validated() {
        let mut scheduler = PlaybackScheduler::new();
        assert_eq!(
            scheduler.play(-1),
            Err(PlaybackError::NegativeMonotonicTime)
        );
        scheduler.play(100).unwrap();
        assert_eq!(
            scheduler.media_time_100ns(99),
            Err(PlaybackError::MonotonicTimeMovedBackwards)
        );
        assert_eq!(
            scheduler.seek(-1, 100),
            Err(PlaybackError::NegativeSeekPosition)
        );
    }

    #[test]
    fn stop_pause_and_seek_recover_after_media_time_overflow() {
        let mut stopped = PlaybackScheduler::new();
        stopped.play(0).unwrap();
        stopped.seek(i64::MAX - 10, 0).unwrap();
        assert_eq!(
            stopped.media_time_100ns(11),
            Err(PlaybackError::TimeOverflow)
        );
        stopped.stop(11).unwrap();
        assert_eq!(stopped.state(), PlaybackState::Stopped);
        assert_eq!(stopped.media_time_100ns(12).unwrap(), 0);

        let mut paused = PlaybackScheduler::new();
        paused.play(0).unwrap();
        paused.seek(i64::MAX - 10, 0).unwrap();
        assert_eq!(
            paused.media_time_100ns(11),
            Err(PlaybackError::TimeOverflow)
        );
        paused.pause(11).unwrap();
        assert_eq!(paused.state(), PlaybackState::Paused);
        assert_eq!(paused.media_time_100ns(12).unwrap(), i64::MAX);
        paused.seek(50, 12).unwrap();
        paused.play(12).unwrap();
        assert_eq!(paused.media_time_100ns(13).unwrap(), 51);
    }

    #[test]
    fn overflowing_frame_interval_is_rejected_before_queueing() {
        let mut scheduler = PlaybackScheduler::new();
        scheduler.play(0).unwrap();
        assert_eq!(
            scheduler.enqueue_frame(frame(i64::MAX - 5, 10, 1), 0),
            Err(PlaybackError::TimeOverflow)
        );
        assert_eq!(scheduler.queued_frames(), 0);
        assert_eq!(
            scheduler.enqueue_frame(frame(0, 0, 2), 0),
            Err(PlaybackError::InvalidFrameTiming)
        );
        assert_eq!(scheduler.queued_frames(), 0);
    }
}
