use std::collections::VecDeque;

use crate::DecodedVideoFrame;

const DEFAULT_CAPACITY: usize = 3;
const MIN_CAPACITY: usize = 2;
const MAX_CAPACITY: usize = 4;

/// Invalid capacity requested for a decoded video frame queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameQueueError {
    requested: usize,
}

impl FrameQueueError {
    pub const fn requested_capacity(self) -> usize {
        self.requested
    }
}

impl std::fmt::Display for FrameQueueError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "video frame queue capacity {} is outside {}..={}",
            self.requested, MIN_CAPACITY, MAX_CAPACITY
        )
    }
}

impl std::error::Error for FrameQueueError {}

/// Result of inserting one decoded frame into the bounded queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameEnqueueOutcome {
    /// The frame was inserted without evicting another frame.
    Queued,
    /// The frame had already passed its PTS-plus-duration deadline.
    DroppedLate,
    /// The frame was queued and the oldest queued frame was evicted at capacity.
    QueuedAndDroppedOldest,
    /// The queue was full and the incoming frame was older than its retained frames.
    DroppedByCapacity,
}

/// Timestamp-ordered queue for decoded frames, with a small validated capacity.
///
/// Enqueue drops frames whose presentation interval has expired. When full, it
/// retains the freshest timestamps. `pop_due` returns the newest frame whose
/// PTS is at or before the supplied playback time and whose presentation
/// interval has not expired, discarding older due frames that have been
/// superseded.
#[derive(Debug, Clone)]
pub struct VideoFrameQueue {
    frames: VecDeque<DecodedVideoFrame>,
    capacity: usize,
    dropped_frames: u64,
}

impl Default for VideoFrameQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl VideoFrameQueue {
    /// Creates a queue with the default capacity of three frames.
    pub fn new() -> Self {
        Self {
            frames: VecDeque::with_capacity(DEFAULT_CAPACITY),
            capacity: DEFAULT_CAPACITY,
            dropped_frames: 0,
        }
    }

    /// Creates a queue with capacity from two through four frames.
    pub fn with_capacity(capacity: usize) -> Result<Self, FrameQueueError> {
        if !(MIN_CAPACITY..=MAX_CAPACITY).contains(&capacity) {
            return Err(FrameQueueError {
                requested: capacity,
            });
        }
        Ok(Self {
            frames: VecDeque::with_capacity(capacity),
            capacity,
            dropped_frames: 0,
        })
    }

    pub const fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    pub const fn dropped_frames(&self) -> u64 {
        self.dropped_frames
    }

    /// Inserts by PTS, first removing frames whose presentation interval ended.
    pub fn enqueue(
        &mut self,
        frame: DecodedVideoFrame,
        playback_time_100ns: i64,
    ) -> FrameEnqueueOutcome {
        self.discard_expired(playback_time_100ns);
        if is_expired(&frame, playback_time_100ns) {
            self.dropped_frames = self.dropped_frames.saturating_add(1);
            return FrameEnqueueOutcome::DroppedLate;
        }

        if self.frames.len() == self.capacity
            && self
                .frames
                .front()
                .is_some_and(|oldest| frame.pts_100ns() <= oldest.pts_100ns())
        {
            self.dropped_frames = self.dropped_frames.saturating_add(1);
            return FrameEnqueueOutcome::DroppedByCapacity;
        }

        let insertion_index = self
            .frames
            .iter()
            .position(|queued| queued.pts_100ns() > frame.pts_100ns())
            .unwrap_or(self.frames.len());
        self.frames.insert(insertion_index, frame);

        if self.frames.len() > self.capacity {
            self.frames.pop_front();
            self.dropped_frames = self.dropped_frames.saturating_add(1);
            FrameEnqueueOutcome::QueuedAndDroppedOldest
        } else {
            FrameEnqueueOutcome::Queued
        }
    }

    /// Removes expired and superseded due frames, returning the newest due frame.
    pub fn pop_due(&mut self, playback_time_100ns: i64) -> Option<DecodedVideoFrame> {
        self.discard_expired(playback_time_100ns);
        let mut newest_due = None;
        while self
            .frames
            .front()
            .is_some_and(|frame| frame.pts_100ns() <= playback_time_100ns)
        {
            if newest_due.is_some() {
                self.dropped_frames = self.dropped_frames.saturating_add(1);
            }
            newest_due = self.frames.pop_front();
        }
        newest_due
    }

    /// Remove frames whose PTS-plus-duration interval has elapsed, returning
    /// the number removed. This allows transport completion to advance even
    /// when an output-FPS cap temporarily blocks presentation.
    pub fn discard_expired(&mut self, playback_time_100ns: i64) -> usize {
        let previous_len = self.frames.len();
        self.frames
            .retain(|frame| !is_expired(frame, playback_time_100ns));
        let dropped = previous_len - self.frames.len();
        self.dropped_frames = self.dropped_frames.saturating_add(dropped as u64);
        dropped
    }

    /// Clears all queued frames, for example when starting a seek timeline.
    pub fn clear(&mut self) {
        self.frames.clear();
    }
}

fn is_expired(frame: &DecodedVideoFrame, playback_time_100ns: i64) -> bool {
    frame
        .pts_100ns()
        .checked_add(frame.duration_100ns())
        .is_some_and(|deadline| deadline <= playback_time_100ns)
}

#[cfg(test)]
mod tests {
    use super::{FrameEnqueueOutcome, FrameQueueError, VideoFrameQueue};
    use crate::{DecodedVideoFrame, VideoPixelFormat};

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

    fn queued_pts(queue: &VideoFrameQueue) -> Vec<i64> {
        queue
            .frames
            .iter()
            .map(DecodedVideoFrame::pts_100ns)
            .collect()
    }

    #[test]
    fn capacity_defaults_to_three_and_accepts_only_two_through_four() {
        assert_eq!(VideoFrameQueue::new().capacity(), 3);
        for capacity in 2..=4 {
            assert_eq!(
                VideoFrameQueue::with_capacity(capacity).unwrap().capacity(),
                capacity
            );
        }
        assert_eq!(
            VideoFrameQueue::with_capacity(1).unwrap_err(),
            FrameQueueError { requested: 1 }
        );
        assert_eq!(
            VideoFrameQueue::with_capacity(5).unwrap_err(),
            FrameQueueError { requested: 5 }
        );
    }

    #[test]
    fn inserts_out_of_order_frames_and_pops_only_due_frames_in_pts_order() {
        let mut queue = VideoFrameQueue::new();
        assert_eq!(
            queue.enqueue(frame(20, 10, 2), 0),
            FrameEnqueueOutcome::Queued
        );
        assert_eq!(
            queue.enqueue(frame(0, 10, 0), 0),
            FrameEnqueueOutcome::Queued
        );
        assert_eq!(
            queue.enqueue(frame(10, 10, 1), 0),
            FrameEnqueueOutcome::Queued
        );
        assert_eq!(queued_pts(&queue), [0, 10, 20]);
        assert_eq!(queue.pop_due(9).unwrap().pts_100ns(), 0);
        assert_eq!(queue.pop_due(10).unwrap().pts_100ns(), 10);
        assert_eq!(queue.pop_due(19), None);
        assert_eq!(queue.pop_due(20).unwrap().pts_100ns(), 20);
        assert!(queue.is_empty());
    }

    #[test]
    fn sustained_input_never_exceeds_capacity_and_keeps_freshest_frames() {
        let mut queue = VideoFrameQueue::with_capacity(3).unwrap();
        for pts in 0..1_000 {
            let outcome = queue.enqueue(frame(pts * 10, 10, pts as u8), 0);
            assert!(matches!(
                outcome,
                FrameEnqueueOutcome::Queued | FrameEnqueueOutcome::QueuedAndDroppedOldest
            ));
            assert!(queue.len() <= queue.capacity());
        }
        assert_eq!(queued_pts(&queue), [9_970, 9_980, 9_990]);
    }

    #[test]
    fn drops_expired_frames_and_rejects_late_incoming_frames() {
        let mut queue = VideoFrameQueue::new();
        assert_eq!(
            queue.enqueue(frame(0, 10, 0), 0),
            FrameEnqueueOutcome::Queued
        );
        assert_eq!(
            queue.enqueue(frame(10, 10, 1), 0),
            FrameEnqueueOutcome::Queued
        );
        assert_eq!(
            queue.enqueue(frame(20, 10, 2), 30),
            FrameEnqueueOutcome::DroppedLate
        );
        assert!(queued_pts(&queue).is_empty());
        assert_eq!(
            queue.enqueue(frame(25, 10, 3), 30),
            FrameEnqueueOutcome::Queued
        );
        assert_eq!(queue.pop_due(30).unwrap().pts_100ns(), 25);
    }

    #[test]
    fn full_queue_drops_incoming_frame_if_it_is_older_than_retained_frames() {
        let mut queue = VideoFrameQueue::with_capacity(2).unwrap();
        queue.enqueue(frame(20, 10, 2), 0);
        queue.enqueue(frame(30, 10, 3), 0);
        assert_eq!(
            queue.enqueue(frame(10, 10, 1), 0),
            FrameEnqueueOutcome::DroppedByCapacity
        );
        assert_eq!(queued_pts(&queue), [20, 30]);
    }

    #[test]
    fn playback_time_jump_returns_newest_due_frame_and_discards_superseded_frames() {
        let mut queue = VideoFrameQueue::new();
        queue.enqueue(frame(0, 100, 0), 0);
        queue.enqueue(frame(20, 100, 1), 0);
        queue.enqueue(frame(40, 100, 2), 0);
        queue.enqueue(frame(60, 100, 3), 0);

        assert_eq!(queue.pop_due(50).unwrap().pts_100ns(), 40);
        assert_eq!(queued_pts(&queue), [60]);
        assert_eq!(queue.pop_due(60).unwrap().pts_100ns(), 60);
        assert!(queue.is_empty());
    }

    #[test]
    fn clear_removes_frames_from_the_old_seek_timeline() {
        let mut queue = VideoFrameQueue::new();
        queue.enqueue(frame(100, 10, 1), 90);
        queue.enqueue(frame(110, 10, 2), 90);
        queue.clear();
        assert!(queue.is_empty());
        assert_eq!(
            queue.enqueue(frame(0, 10, 3), 0),
            FrameEnqueueOutcome::Queued
        );
        assert_eq!(queued_pts(&queue), [0]);
    }

    #[test]
    fn cumulative_drop_counter_tracks_capacity_late_and_superseded_frames() {
        let mut queue = VideoFrameQueue::with_capacity(2).unwrap();
        queue.enqueue(frame(10, 100, 1), 0);
        queue.enqueue(frame(20, 100, 2), 0);
        assert_eq!(
            queue.enqueue(frame(30, 100, 3), 0),
            FrameEnqueueOutcome::QueuedAndDroppedOldest
        );
        assert_eq!(queue.dropped_frames(), 1);
        assert_eq!(
            queue.enqueue(frame(20, 100, 4), 0),
            FrameEnqueueOutcome::DroppedByCapacity
        );
        assert_eq!(queue.dropped_frames(), 2);
        queue.clear();

        queue.enqueue(frame(0, 100, 0), 0);
        queue.enqueue(frame(20, 100, 1), 0);
        queue.enqueue(frame(40, 100, 2), 0);
        assert_eq!(queue.pop_due(50).unwrap().pts_100ns(), 40);
        assert_eq!(queue.dropped_frames(), 4);
        assert_eq!(
            queue.enqueue(frame(0, 10, 0), 50),
            FrameEnqueueOutcome::DroppedLate
        );
        assert_eq!(queue.dropped_frames(), 5);
    }
}
