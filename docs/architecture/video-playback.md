# Video playback clock and scheduler

`wallpaper_media::PlaybackScheduler` maps caller-supplied monotonic time to a
100 ns media timeline. Callers must pass monotonic ticks, such as elapsed time
from `std::time::Instant`; calendar time is invalid. The scheduler rejects
negative or backward clock samples and checks timeline arithmetic for overflow.
If media-time arithmetic reaches its representable limit, a time query returns
an error, while pause/seek/stop/restart remain available to leave that timeline.

The scheduler owns the bounded three-frame `VideoFrameQueue`. Decoded frames
are selected by PTS, not by decode completion time. Each `next_due_frame` call
returns the newest queued frame whose PTS is due and whose presentation interval
has not expired. Older superseded or late frames are discarded by the queue.
Frames with negative PTS, nonpositive duration, or an overflowing presentation
end are rejected before entering the queue. Callers present only returned
frames, so an idle clock sample does not redraw.

## Output FPS cap

`VideoFpsLimit` accepts typed presets of 15, 24, 30, 60, and 120 FPS, or a
validated custom value from 1 through 120 FPS. The default cap is 60 FPS. The
scheduler enforces the minimum wall-clock interval using monotonic 100 ns ticks
and ceiling division, so fractional intervals never exceed the requested cap.
Changing the cap does not re-anchor or alter media time; playback speed can be
added independently. If the source is slower than the cap, only decoded frames
are returned and no synthetic frames are created. If the source is faster,
the queue supplies the newest due frame at each available output slot.
Expired frames are pruned on every scheduler sample, and EOS completion is
evaluated independently of the presentation gate.

## Transport behavior

- `play` starts from the stored media position; from `Paused` it re-anchors the
  monotonic clock without advancing media time during the pause.
- `pause` stores the current media position and freezes it.
- `stop` resets media time to zero and clears queued frames and end-of-stream
  state.
- `seek` sets a nonnegative media position and clears old queued frames. The
  caller must seek the Media Foundation source to the same position.
- `restart` clears queued frames and starts playback at zero.
- End of stream becomes `Ended` after queued frames drain and the final
  presented frame's PTS-plus-duration interval elapses. Playing again from
  `Ended` restarts at zero.

The API accepts a clock sample on each operation, so tests use integer fake
clock values and do not sleep. This module does not control the decoder's seek
operation, playback-speed mapping, audio, IPC, or UI; those are later Phase 3
work items.
