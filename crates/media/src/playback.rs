use crate::{DecodedVideoFrame, FrameEnqueueOutcome, VideoFrameQueue};
use wallpaper_core::playback_settings::{LoopMode, SegmentRange};

const HUNDRED_NANOSECONDS_PER_SECOND: u32 = 10_000_000;
const MAX_PRESENTATION_FPS: u16 = 120;

/// Supported common output-FPS presets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoFpsPreset {
    Fps15,
    Fps24,
    Fps30,
    Fps60,
    Fps120,
}

impl VideoFpsPreset {
    pub const ALL: [Self; 5] = [
        Self::Fps15,
        Self::Fps24,
        Self::Fps30,
        Self::Fps60,
        Self::Fps120,
    ];

    pub const fn frames_per_second(self) -> u16 {
        match self {
            Self::Fps15 => 15,
            Self::Fps24 => 24,
            Self::Fps30 => 30,
            Self::Fps60 => 60,
            Self::Fps120 => 120,
        }
    }
}

/// Validated maximum output presentation rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoFpsLimit {
    frames_per_second: u16,
}

impl VideoFpsLimit {
    pub const fn preset(preset: VideoFpsPreset) -> Self {
        Self {
            frames_per_second: preset.frames_per_second(),
        }
    }

    /// Create a custom cap from 1 through 120 FPS.
    pub fn custom(frames_per_second: u16) -> Result<Self, VideoFpsLimitError> {
        if !(1..=MAX_PRESENTATION_FPS).contains(&frames_per_second) {
            return Err(VideoFpsLimitError {
                requested: frames_per_second,
            });
        }
        Ok(Self { frames_per_second })
    }

    pub const fn frames_per_second(self) -> u16 {
        self.frames_per_second
    }

    /// Smallest whole 100 ns interval that never exceeds this output rate.
    pub const fn minimum_interval_100ns(self) -> i64 {
        HUNDRED_NANOSECONDS_PER_SECOND.div_ceil(self.frames_per_second as u32) as i64
    }
}

impl Default for VideoFpsLimit {
    fn default() -> Self {
        Self::preset(VideoFpsPreset::Fps60)
    }
}

/// A custom presentation FPS value outside the supported 1–120 range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoFpsLimitError {
    requested: u16,
}

impl VideoFpsLimitError {
    pub const fn requested_fps(self) -> u16 {
        self.requested
    }
}

impl std::fmt::Display for VideoFpsLimitError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "custom FPS {} is outside 1..=120",
            self.requested
        )
    }
}

impl std::error::Error for VideoFpsLimitError {}

/// Documented playback speeds for the video media timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VideoPlaybackSpeed {
    Quarter,
    Half,
    ThreeQuarters,
    #[default]
    Normal,
    FiveQuarters,
    OneAndHalf,
    Double,
}

impl VideoPlaybackSpeed {
    pub const ALL: [Self; 7] = [
        Self::Quarter,
        Self::Half,
        Self::ThreeQuarters,
        Self::Normal,
        Self::FiveQuarters,
        Self::OneAndHalf,
        Self::Double,
    ];

    pub const fn from_percent(percent: u16) -> Result<Self, VideoPlaybackSpeedError> {
        match percent {
            25 => Ok(Self::Quarter),
            50 => Ok(Self::Half),
            75 => Ok(Self::ThreeQuarters),
            100 => Ok(Self::Normal),
            125 => Ok(Self::FiveQuarters),
            150 => Ok(Self::OneAndHalf),
            200 => Ok(Self::Double),
            _ => Err(VideoPlaybackSpeedError { requested: percent }),
        }
    }

    pub const fn percent(self) -> u16 {
        match self {
            Self::Quarter => 25,
            Self::Half => 50,
            Self::ThreeQuarters => 75,
            Self::Normal => 100,
            Self::FiveQuarters => 125,
            Self::OneAndHalf => 150,
            Self::Double => 200,
        }
    }

    const fn ratio(self) -> (i64, i64) {
        match self {
            Self::Quarter => (1, 4),
            Self::Half => (1, 2),
            Self::ThreeQuarters => (3, 4),
            Self::Normal => (1, 1),
            Self::FiveQuarters => (5, 4),
            Self::OneAndHalf => (3, 2),
            Self::Double => (2, 1),
        }
    }
}

/// Unsupported percentage passed to `VideoPlaybackSpeed::from_percent`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoPlaybackSpeedError {
    requested: u16,
}

impl VideoPlaybackSpeedError {
    pub const fn requested_percent(self) -> u16 {
        self.requested
    }
}

impl std::fmt::Display for VideoPlaybackSpeedError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "unsupported video playback speed {}%",
            self.requested
        )
    }
}

impl std::error::Error for VideoPlaybackSpeedError {}

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
    fps_limit: VideoFpsLimit,
    last_presented_wall_time_100ns: Option<i64>,
    speed: VideoPlaybackSpeed,
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
            fps_limit: VideoFpsLimit::default(),
            last_presented_wall_time_100ns: None,
            speed: VideoPlaybackSpeed::default(),
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

    pub fn dropped_frames(&self) -> u64 {
        self.queue.dropped_frames()
    }

    pub const fn fps_limit(&self) -> VideoFpsLimit {
        self.fps_limit
    }

    /// Change the presentation cap without re-anchoring the media clock.
    pub fn set_fps_limit(&mut self, fps_limit: VideoFpsLimit) {
        self.fps_limit = fps_limit;
    }

    pub const fn speed(&self) -> VideoPlaybackSpeed {
        self.speed
    }

    /// Change playback rate continuously, without changing the source PTS or
    /// configured output-FPS limit.
    pub fn set_speed(
        &mut self,
        speed: VideoPlaybackSpeed,
        monotonic_time_100ns: i64,
    ) -> Result<(), PlaybackError> {
        self.observe_monotonic_time(monotonic_time_100ns)?;
        if self.speed == speed {
            return Ok(());
        }
        let media_time = if self.state == PlaybackState::Playing {
            self.media_time_from_anchor(monotonic_time_100ns)?
        } else {
            self.media_anchor_100ns
        };
        self.speed = speed;
        self.media_anchor_100ns = media_time;
        if self.state == PlaybackState::Playing {
            self.wall_anchor_100ns = Some(monotonic_time_100ns);
        }
        Ok(())
    }

    /// Return the media timestamp corresponding to a monotonic clock sample.
    pub fn media_time_100ns(&mut self, monotonic_time_100ns: i64) -> Result<i64, PlaybackError> {
        self.current_media_time(monotonic_time_100ns)
    }

    /// Re-anchor a playing timeline to the audio engine's authoritative clock.
    pub fn synchronize_media_time(
        &mut self,
        media_time_100ns: i64,
        monotonic_time_100ns: i64,
    ) -> Result<(), PlaybackError> {
        if media_time_100ns < 0 {
            return Err(PlaybackError::NegativeSeekPosition);
        }
        self.observe_monotonic_time(monotonic_time_100ns)?;
        if self.state == PlaybackState::Playing {
            self.media_anchor_100ns = media_time_100ns;
            self.wall_anchor_100ns = Some(monotonic_time_100ns);
        }
        Ok(())
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

    /// Return the seek target when the configured loop boundary is reached.
    /// The caller seeks the decoder and scheduler together, then resumes the
    /// source. Segment end is exclusive.
    pub fn loop_seek_target(
        &mut self,
        mode: LoopMode,
        segment: Option<SegmentRange>,
        monotonic_time_100ns: i64,
    ) -> Result<Option<i64>, PlaybackError> {
        match mode {
            LoopMode::Disabled => Ok(None),
            LoopMode::Full if self.state == PlaybackState::Ended => Ok(Some(0)),
            LoopMode::Full => Ok(None),
            LoopMode::Segment => {
                let Some(segment) = segment else {
                    return Ok(None);
                };
                let start = segment
                    .start_ms()
                    .checked_mul(10_000)
                    .and_then(|value| i64::try_from(value).ok())
                    .ok_or(PlaybackError::TimeOverflow)?;
                let end = segment
                    .end_ms()
                    .checked_mul(10_000)
                    .and_then(|value| i64::try_from(value).ok())
                    .ok_or(PlaybackError::TimeOverflow)?;
                if self.state == PlaybackState::Ended
                    || (self.state == PlaybackState::Playing
                        && self.media_time_100ns(monotonic_time_100ns)? >= end)
                {
                    Ok(Some(start))
                } else {
                    Ok(None)
                }
            }
        }
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
        self.queue.discard_expired(media_time);
        let last_frame_elapsed = self
            .last_presented_end_100ns
            .is_none_or(|end| media_time >= end);
        if self.source_ended && self.queue.is_empty() && last_frame_elapsed {
            self.state = PlaybackState::Ended;
            self.media_anchor_100ns = media_time;
            self.wall_anchor_100ns = None;
            return Ok(None);
        }
        if self.last_presented_wall_time_100ns.is_some_and(|last| {
            monotonic_time_100ns - last < self.fps_limit.minimum_interval_100ns()
        }) {
            return Ok(None);
        }

        if let Some(frame) = self.queue.pop_due(media_time) {
            self.last_presented_end_100ns = Some(
                frame
                    .pts_100ns()
                    .checked_add(frame.duration_100ns())
                    .ok_or(PlaybackError::TimeOverflow)?,
            );
            self.last_presented_wall_time_100ns = Some(monotonic_time_100ns);
            return Ok(Some(frame));
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
        let (speed_numerator, speed_denominator) = self.speed.ratio();
        let scaled_elapsed = i128::from(elapsed)
            .checked_mul(i128::from(speed_numerator))
            .ok_or(PlaybackError::TimeOverflow)?
            / i128::from(speed_denominator);
        let media_time = i128::from(self.media_anchor_100ns)
            .checked_add(scaled_elapsed)
            .ok_or(PlaybackError::TimeOverflow)?;
        i64::try_from(media_time).map_err(|_| PlaybackError::TimeOverflow)
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
    use super::{
        PlaybackError, PlaybackScheduler, PlaybackState, VideoFpsLimit, VideoFpsPreset,
        VideoPlaybackSpeed,
    };
    use crate::{DecodedVideoFrame, FrameEnqueueOutcome, VideoPixelFormat};
    use wallpaper_core::playback_settings::{LoopMode, SegmentRange};

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
    fn loop_targets_respect_disabled_full_and_exclusive_segment_boundaries() {
        let segment = SegmentRange::new(500, 1_500).unwrap();
        let mut scheduler = PlaybackScheduler::new();
        scheduler.restart(0).unwrap();

        assert_eq!(
            scheduler
                .loop_seek_target(LoopMode::Disabled, None, 20_000_000)
                .unwrap(),
            None
        );
        assert_eq!(
            scheduler
                .loop_seek_target(LoopMode::Full, None, 20_000_000)
                .unwrap(),
            None,
            "full loop waits for source EOS and final-frame expiry"
        );
        assert_eq!(
            scheduler
                .loop_seek_target(LoopMode::Segment, Some(segment), 14_999_999)
                .unwrap(),
            None
        );
        assert_eq!(
            scheduler
                .loop_seek_target(LoopMode::Segment, Some(segment), 15_000_000)
                .unwrap(),
            Some(5_000_000),
            "the exclusive end timestamp wraps to the segment start"
        );

        scheduler.end_of_stream(15_000_000).unwrap();
        scheduler
            .next_due_frame(15_000_000)
            .expect("end-of-stream evaluation");
        assert_eq!(scheduler.state(), PlaybackState::Ended);
        assert_eq!(
            scheduler
                .loop_seek_target(LoopMode::Full, None, 15_000_000)
                .unwrap(),
            Some(0)
        );
        assert_eq!(
            scheduler
                .loop_seek_target(LoopMode::Segment, Some(segment), 15_000_000)
                .unwrap(),
            Some(5_000_000)
        );
    }

    #[test]
    fn repeated_full_loop_targets_restart_after_each_end_of_stream() {
        let mut scheduler = PlaybackScheduler::new();
        let mut now = 0;
        for _ in 0..5 {
            scheduler.restart(now).unwrap();
            scheduler.end_of_stream(now).unwrap();
            scheduler.next_due_frame(now).unwrap();
            assert_eq!(scheduler.state(), PlaybackState::Ended);
            assert_eq!(
                scheduler
                    .loop_seek_target(LoopMode::Full, None, now)
                    .unwrap(),
                Some(0)
            );
            scheduler.restart(now + 1).unwrap();
            now += 2;
            assert_eq!(scheduler.state(), PlaybackState::Playing);
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
        scheduler.set_fps_limit(VideoFpsLimit::preset(VideoFpsPreset::Fps120));
        scheduler.play(0).unwrap();
        scheduler.enqueue_frame(frame(100, 100, 1), 0).unwrap();
        scheduler.seek(0, 10).unwrap();
        assert_eq!(scheduler.queued_frames(), 0);
        scheduler.enqueue_frame(frame(0, 100, 2), 10).unwrap();
        assert_eq!(scheduler.next_due_frame(10).unwrap().unwrap().rgba8()[0], 2);

        let restart_time = 83_344;
        scheduler
            .enqueue_frame(frame(100, 100, 3), restart_time)
            .unwrap();
        scheduler.restart(restart_time).unwrap();
        assert_eq!(scheduler.state(), PlaybackState::Playing);
        assert_eq!(scheduler.queued_frames(), 0);
        assert_eq!(scheduler.media_time_100ns(restart_time).unwrap(), 0);
        scheduler
            .enqueue_frame(frame(0, 100, 4), restart_time)
            .unwrap();
        assert_eq!(
            scheduler
                .next_due_frame(restart_time)
                .unwrap()
                .unwrap()
                .rgba8()[0],
            4
        );
    }

    #[test]
    fn stop_and_end_of_stream_have_deterministic_transitions() {
        let mut scheduler = PlaybackScheduler::new();
        scheduler.set_fps_limit(VideoFpsLimit::preset(VideoFpsPreset::Fps120));
        scheduler.play(0).unwrap();
        scheduler.enqueue_frame(frame(0, 100_000, 1), 0).unwrap();
        assert_eq!(scheduler.next_due_frame(0).unwrap().unwrap().pts_100ns(), 0);
        scheduler.end_of_stream(0).unwrap();
        assert_eq!(scheduler.next_due_frame(83_333).unwrap(), None);
        assert_eq!(scheduler.state(), PlaybackState::Playing);
        assert_eq!(scheduler.next_due_frame(100_000).unwrap(), None);
        assert_eq!(scheduler.state(), PlaybackState::Ended);
        assert_eq!(
            scheduler.enqueue_frame(frame(100_000, 100_000, 2), 100_000),
            Err(PlaybackError::SourceAlreadyEnded)
        );

        scheduler.play(100_000).unwrap();
        assert_eq!(scheduler.state(), PlaybackState::Playing);
        assert_eq!(scheduler.media_time_100ns(100_000).unwrap(), 0);
        scheduler.stop(100_000).unwrap();
        assert_eq!(scheduler.state(), PlaybackState::Stopped);
        assert_eq!(scheduler.media_time_100ns(100_001).unwrap(), 0);
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

    #[test]
    fn fps_presets_and_custom_limits_are_validated() {
        let expected = [
            (15, 666_667),
            (24, 416_667),
            (30, 333_334),
            (60, 166_667),
            (120, 83_334),
        ];
        for (preset, (fps, interval)) in VideoFpsPreset::ALL.into_iter().zip(expected) {
            let limit = VideoFpsLimit::preset(preset);
            assert_eq!(preset.frames_per_second(), fps);
            assert_eq!(limit.frames_per_second(), fps);
            assert_eq!(limit.minimum_interval_100ns(), interval);
        }
        assert_eq!(VideoFpsLimit::default().frames_per_second(), 60);
        assert_eq!(VideoFpsLimit::custom(1).unwrap().frames_per_second(), 1);
        assert_eq!(VideoFpsLimit::custom(90).unwrap().frames_per_second(), 90);
        assert_eq!(VideoFpsLimit::custom(120).unwrap().frames_per_second(), 120);
        assert_eq!(VideoFpsLimit::custom(0).unwrap_err().requested_fps(), 0);
        assert_eq!(VideoFpsLimit::custom(121).unwrap_err().requested_fps(), 121);
    }

    fn simulate_one_second(source_fps: u16, cap_fps: u16) -> (usize, usize) {
        let mut scheduler = PlaybackScheduler::new();
        scheduler.set_fps_limit(VideoFpsLimit::custom(cap_fps).unwrap());
        scheduler.play(0).unwrap();
        let source_interval = (10_000_000_i64 + i64::from(source_fps) - 1) / i64::from(source_fps);
        let minimum_output_interval = VideoFpsLimit::custom(cap_fps)
            .unwrap()
            .minimum_interval_100ns();
        let mut presented = 0;
        let mut max_queued = 0;
        let mut last_output_time = None;
        for frame_index in 0..source_fps {
            let pts = i64::from(frame_index) * source_interval;
            scheduler
                .enqueue_frame(frame(pts, source_interval, frame_index as u8), pts)
                .unwrap();
            max_queued = max_queued.max(scheduler.queued_frames());
            if scheduler.next_due_frame(pts).unwrap().is_some() {
                if let Some(previous) = last_output_time {
                    assert!(pts - previous >= minimum_output_interval);
                }
                last_output_time = Some(pts);
                presented += 1;
            }
        }
        (presented, max_queued)
    }

    #[test]
    fn fps_cap_limits_output_without_changing_the_media_clock_or_synthesizing_frames() {
        assert_eq!(simulate_one_second(30, 30).0, 30);
        assert_eq!(simulate_one_second(60, 30).0, 30);
        assert_eq!(simulate_one_second(30, 60).0, 30);
        assert_eq!(simulate_one_second(60, 60).0, 60);
        let (presented, max_queued) = simulate_one_second(60, 15);
        assert_eq!(presented, 15);
        assert!(max_queued <= 3);

        let mut scheduler = PlaybackScheduler::new();
        scheduler.play(100).unwrap();
        assert_eq!(scheduler.media_time_100ns(1_000_100).unwrap(), 1_000_000);
        scheduler.set_fps_limit(VideoFpsLimit::preset(VideoFpsPreset::Fps15));
        assert_eq!(scheduler.media_time_100ns(1_000_100).unwrap(), 1_000_000);
    }

    #[test]
    fn end_of_stream_advances_without_waiting_for_the_next_fps_slot() {
        let mut scheduler = PlaybackScheduler::new();
        scheduler.set_fps_limit(VideoFpsLimit::custom(1).unwrap());
        scheduler.play(0).unwrap();
        scheduler.enqueue_frame(frame(0, 1_000_000, 1), 0).unwrap();
        assert_eq!(scheduler.next_due_frame(0).unwrap().unwrap().pts_100ns(), 0);

        scheduler
            .enqueue_frame(frame(1_000_000, 10_000_000, 2), 1_000_000)
            .unwrap();
        scheduler.end_of_stream(1_000_000).unwrap();
        assert_eq!(scheduler.next_due_frame(1_000_000).unwrap(), None);
        assert_eq!(scheduler.queued_frames(), 1);
        assert_eq!(scheduler.state(), PlaybackState::Playing);

        assert_eq!(
            scheduler
                .next_due_frame(10_000_000)
                .unwrap()
                .unwrap()
                .pts_100ns(),
            1_000_000
        );
        assert_eq!(scheduler.next_due_frame(11_000_000).unwrap(), None);
        assert_eq!(scheduler.state(), PlaybackState::Ended);
    }

    #[test]
    fn documented_playback_speeds_map_media_time_and_reject_unsupported_values() {
        let expected = [
            (25, 2_500_000),
            (50, 5_000_000),
            (75, 7_500_000),
            (100, 10_000_000),
            (125, 12_500_000),
            (150, 15_000_000),
            (200, 20_000_000),
        ];
        for (speed, (percent, media_time)) in VideoPlaybackSpeed::ALL.into_iter().zip(expected) {
            assert_eq!(speed.percent(), percent);
            assert_eq!(VideoPlaybackSpeed::from_percent(percent), Ok(speed));
            let mut scheduler = PlaybackScheduler::new();
            scheduler.set_speed(speed, 0).unwrap();
            scheduler.play(0).unwrap();
            assert_eq!(scheduler.media_time_100ns(10_000_000).unwrap(), media_time);
        }
        for unsupported in [0, 1, 90, 110, 201] {
            assert_eq!(
                VideoPlaybackSpeed::from_percent(unsupported)
                    .unwrap_err()
                    .requested_percent(),
                unsupported
            );
        }
    }

    #[test]
    fn speed_changes_pause_resume_and_seek_keep_continuous_media_time_and_fps_cap() {
        let mut scheduler = PlaybackScheduler::new();
        scheduler.set_fps_limit(VideoFpsLimit::preset(VideoFpsPreset::Fps30));
        scheduler.play(100).unwrap();
        assert_eq!(scheduler.media_time_100ns(1_000_100).unwrap(), 1_000_000);

        scheduler
            .set_speed(VideoPlaybackSpeed::Double, 1_000_100)
            .unwrap();
        assert_eq!(scheduler.media_time_100ns(1_000_100).unwrap(), 1_000_000);
        assert_eq!(scheduler.fps_limit().frames_per_second(), 30);
        assert_eq!(scheduler.media_time_100ns(1_500_100).unwrap(), 2_000_000);

        scheduler.pause(1_500_100).unwrap();
        assert_eq!(scheduler.media_time_100ns(10_000_000).unwrap(), 2_000_000);
        scheduler
            .set_speed(VideoPlaybackSpeed::Half, 10_000_000)
            .unwrap();
        scheduler.play(10_000_000).unwrap();
        assert_eq!(scheduler.media_time_100ns(12_000_000).unwrap(), 3_000_000);

        scheduler.seek(5_000_000, 12_000_000).unwrap();
        assert_eq!(scheduler.media_time_100ns(12_000_000).unwrap(), 5_000_000);
        assert_eq!(scheduler.media_time_100ns(14_000_000).unwrap(), 6_000_000);
        assert_eq!(scheduler.fps_limit().frames_per_second(), 30);
    }

    #[test]
    fn accelerated_media_clock_selects_source_frames_by_unchanged_pts() {
        let mut scheduler = PlaybackScheduler::new();
        scheduler.set_speed(VideoPlaybackSpeed::Double, 0).unwrap();
        scheduler.play(0).unwrap();
        scheduler
            .enqueue_frame(frame(1_000_000, 250_000, 7), 500_000)
            .unwrap();
        let selected = scheduler.next_due_frame(500_000).unwrap().unwrap();
        assert_eq!(selected.pts_100ns(), 1_000_000);
        assert_eq!(selected.rgba8()[0], 7);
    }
}
