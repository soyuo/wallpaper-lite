# Bounded decoded video frame queue

`wallpaper-media::VideoFrameQueue` keeps decoded frames ordered by their
100 ns presentation timestamps. Its default capacity is three frames; callers
may select two, three, or four. Other capacities are rejected.

On enqueue, the queue first discards frames whose presentation interval has
ended (`PTS + duration <= playback time`). An already-expired incoming frame is
discarded. Frames are inserted in stable PTS order. When full, the queue keeps
the freshest timestamps: an incoming frame older than the oldest retained
frame is dropped; otherwise it is inserted and the oldest frame is evicted.

`pop_due(playback_time)` discards expired frames and returns the newest frame
whose PTS is at or before the supplied playback time. Older due frames are
superseded and discarded, so a delayed consumer does not have to render stale
frames one by one. Future frames remain queued for the scheduler. `clear()`
removes the entire queue when the playback timeline changes, such as after a
seek.

The queue stores only decoded frame objects and uses no GPU, UI, clock, or
scheduler dependencies. Unit tests cover capacity validation, ordering,
sustained overflow, late-frame removal, due-time and delayed-consumer behavior,
and seek clearing.
