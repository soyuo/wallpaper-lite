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

The host's source-worker channel is separately bounded to three frames. When
it is full, the source worker retains at most one pending decoded frame and
does not read another media sample until the channel has room. A seek discards
that pending frame before advancing the generation, so stale frames cannot
cross the seek boundary or let the reader run to end of stream ahead of
presentation.

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

## Playback speed

`VideoPlaybackSpeed` accepts the documented 25%, 50%, 75%, 100%, 125%, 150%,
and 200% rates. Unsupported percentages return `VideoPlaybackSpeedError`.
`set_speed` captures the current media position before changing the rate, so
the timeline remains continuous. The media clock uses exact rational scaling
with a wide intermediate; frame PTS values are not rewritten. FPS limiting
continues to use monotonic wall time and is unchanged by speed updates.

## Loop behavior

`PlaybackSettings` selects `Disabled`, `Full`, or `Segment`. The host accepts a
settings snapshot only for the currently loaded source path and validates it
against that source's duration before acknowledging it. Segment ranges use
milliseconds, must be at least 500 ms, and satisfy
`0 <= start < end <= duration`. The runtime command returns a typed success or
error; per-wallpaper persistence is handled separately by
`PlaybackSettingsStore`.
The runtime currently consumes loop mode, segment, speed, and FPS fields;
volume and mute values remain persisted preferences until audio output is
implemented.

- `Disabled` lets the scheduler reach `Ended`. The recovery worker keeps the
  last presented frame on the desktop after end of stream.
- `Full` waits until the final frame's presentation interval expires, then
  seeks both the Media Foundation reader and scheduler to zero and resumes.
- `Segment` treats the interval as half-open `[start, end)`. Playback wraps
  when media time reaches `end`; Play from outside the configured range starts
  at `start`. A seek may land on an earlier keyframe, so the source worker
  discards decoded frames below the requested start and forwards the first
  eligible frame in the new generation.

Each wrap advances the source generation and clears scheduled/deferred frames
from the prior iteration. This prevents old frames from crossing the loop
boundary. It does not guarantee a visually seamless boundary for every codec;
bounded boundary predecoding and artifact reduction are addressed separately.

This Phase 3 path is video-only. It decodes or outputs no audio and makes no
A/V synchronization claim. If audio is added later, it must remain synchronized
with video or be muted at non-1.0× speed.

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
  `Ended` restarts at zero, or at the configured segment start in Segment mode.

The API accepts a clock sample on each operation, so tests use integer fake
clock values and do not sleep. This module does not control the decoder's seek
operation, audio, IPC, or UI.

## UI control flow

Each video library entry can start playback by catalog ID. The UI resolves that
ID to an available video path, asks the host to load it, waits for the matching
ID in host state, restores that wallpaper's persisted loop/speed/FPS settings,
then sends Play. The host remains the source of truth for playback state; the
UI refreshes snapshots while the panel is open and displays host errors rather
than optimistically changing transport state.

The timeline and timecode field seek in milliseconds at the UI boundary and
convert to the host's 100 ns media clock. Adjacent-frame controls use the
source's rational frame rate from the host snapshot. Segment timecode and
handles are validated against the active duration and 500 ms minimum before
settings are applied. Playback settings are persisted per wallpaper only after
the host accepts them; if persistence fails, the UI requests restoration of the
previous host settings and reports the failure.

The Windows 11 manual verification record is in
[`../development/phase-5-playback-acceptance.md`](../development/phase-5-playback-acceptance.md).
