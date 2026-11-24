# UI-to-host IPC

`wallpaper-ui` and `wallpaper-host` communicate through the Windows named pipe
`\\.\pipe\wallpaper-lite` for commands and
`\\.\pipe\wallpaper-lite-playback-events` for state subscriptions. Protocol
Version 13 supports a liveness ping, video playback controls, editor settings,
per-wallpaper playback-settings updates, and a bounded performance-metrics
snapshot query. It adds the active wallpaper ID and rational source frame rate
to playback snapshots and identifies video loads by catalog ID. Request and response
decoders reject mismatched versions, so UI and host builds must be deployed
together. Video decoding, scheduling, and presentation stay in the host.

## Endpoint and framing

- Each control client sends one request and receives one response per
  connection. State subscribers use a separate pipe, so a long-lived event
  stream does not hold up playback commands.
- Each message is UTF-8 JSON preceded by a four-byte unsigned little-endian
  payload length. The length must be between 1 and 16384 bytes before a payload
  buffer is allocated.
- Requests carry protocol `version` and a `request_id`; the response echoes
  both. The client retries a busy pipe every 25 ms for at most one second.
- Pipe request reads and request/response waits have a two-second deadline.
  `load_video` acknowledges that a source open has started; completion or
  failure arrives on the state-event pipe. The playback worker runs on its own
  bounded command queue so source opening, decoding, scheduling, and GPU
  presentation do not run on the Tokio IPC loop.
- The command queue holds at most 16 pending requests. A disconnected client
  before load acknowledgement causes its candidate source to be dropped. Once
  acknowledged, the load continues independently and reports its outcome over
  playback state.

## Request and response shapes

Version 13 retains the ping request:

```json
{"version":13,"request_id":1,"command":"ping"}
```

Editor Apply sends the complete validated settings snapshot in one command:

```json
{
  "version": 13,
  "request_id": 42,
  "command": {
    "apply_editor_settings": {
      "settings": { "schemaVersion": 1, "wallpaperId": 7, "render": {}, "color": {} },
      "source_path": "C:/Media/wallpaper.png"
    }
  }
}
```

The UI resolves the selected catalog ID to its available source path. The host
compares that path with the active presenter source and rejects settings for a
different wallpaper before changing renderer or recovery state. For a match, it
applies the snapshot to the active presenter before acknowledging it. The UI
then atomically persists the snapshot. If persistence fails, the UI asks
the host to restore the previously loaded snapshot and reports an error. A host
validation or render failure leaves the settings file untouched. If the host
response is lost or uncertain, the UI also asks the host to restore the prior
snapshot before returning an error. Apply requests are serialized by the UI
backend; Cancel and Escape never send a host or storage command.

Playback commands use the same request envelope. Enum values are snake case;
fields use the names below:

```json
{"version":13,"request_id":2,"command":{"load_video":{"wallpaper_id":7,"path":"C:/Media/clip.mp4"}}}
{"version":13,"request_id":3,"command":"play"}
{"version":13,"request_id":4,"command":"pause"}
{"version":13,"request_id":5,"command":"stop"}
{"version":13,"request_id":6,"command":{"seek":{"position_100ns":5000000}}}
{"version":13,"request_id":7,"command":{"set_speed":{"percent":125}}}
{"version":13,"request_id":8,"command":{"set_fps":{"frames_per_second":30}}}
{"version":13,"request_id":9,"command":"get_playback_state"}
{"version":13,"request_id":11,"command":{"apply_playback_settings":{"settings":{"schemaVersion":1,"wallpaperId":7,"loopMode":"segment","segment":{"startMs":1000,"endMs":5000},"speedPercent":125,"fpsLimit":30,"volumePercent":100,"muted":false},"source_path":"C:/Media/clip.mp4"}}}
{"version":13,"request_id":12,"command":"shutdown"}
```

The playback-events pipe accepts a long-lived subscription request:

```json
{"version":13,"request_id":10,"command":"subscribe_playback_state"}
```

The response result can be `pong`, a settings-applied acknowledgement, a typed
playback error, or a `playback_state` snapshot:

```json
{"version":13,"request_id":3,"result":{"playback_state":{"sequence":2,"wallpaper_id":7,"state":"playing","media_time_100ns":50000,"duration_100ns":120000000,"frame_rate_numerator":30,"frame_rate_denominator":1,"speed_percent":100,"fps_limit":60,"loading_video":false,"last_error":null}}}
```

State snapshots are returned with control-command responses and on explicit
`get_playback_state` requests. Their monotonically wrapping `sequence` lets a
client discard stale snapshots. A subscriber receives the current state
immediately and later coalesced updates on the separate event pipe.
`media_time_100ns` is the current media clock; duration is `null` when no
source is loaded. `loading_video` indicates an acknowledged asynchronous open,
and `last_error` carries a safe typed error from asynchronous work.

## Playback semantics

- `load_video` acknowledges source opening immediately with
  `loading_video: true`; the current source remains active during inspection.
  A failed open leaves it active and publishes `last_error: video_load_failed`.
  The request includes the catalog wallpaper ID; snapshots publish that active
  ID and the source's rational frame rate so the UI can restore per-wallpaper
  settings and step to adjacent source frames without approximating the FPS.
  Successful replacement resets transport to stopped at zero and releases the
  old source. A request disconnected before acknowledgement drops its candidate;
  an acknowledged load continues independently and reports completion or
  failure through playback state.
- `play`, `pause`, `stop`, and `seek` operate on the host-owned scheduler and
  Media Foundation source reader. Seek positions are signed 100 ns units and
  must be within the loaded source duration.
- Seeking or stopping while paused/stopped presents the newest frame for the
  resulting media position, so the visible desktop does not remain on a stale
  frame.
- `set_speed.percent` accepts the scheduler's supported exact speed presets.
  `set_fps.frames_per_second` accepts values from 1 through 120. The two
  settings are independent.
- `apply_playback_settings` validates the active video path, source duration,
  segment bounds, speed, and FPS before acknowledging an update. Invalid
  settings and source mismatches return typed errors. It updates runtime
  behavior; `PlaybackSettingsStore` owns persistence.
- If a read/decode or scheduler failure occurs between requests, playback stops
  and the next `get_playback_state` returns a typed `decode_failed` or
  `playback_failed` error once; a later snapshot reports the stopped state.
- `GetPlaybackState` returns a single current snapshot. While playing, updates
  are coalesced and capped at four per second; load acceptance/completion/failure,
  transport, seek, settings, end-of-stream, and error transitions publish
  immediately. Closing the event-pipe connection stops delivery.
- `Shutdown` asks the host to stop. The control pipe answers `shutting_down`,
  waits for the client to close, then the host releases its wallpaper windows and
  exits so Windows restores the desktop background. Its response is
  `{"version":13,"request_id":12,"result":"shutting_down"}`. The UI backend
  (`host_process.rs`) sends it and waits for the pipe to disappear. The host also
  holds the per-session mutex `Local\com.soyuo.wallpaper-lite.host`, so a second
  host exits immediately with code 3 instead of attaching another wallpaper
  window; the UI retries the launch while the previous host is still shutting
  down.
- `GetPerformanceMetrics` returns a versioned snapshot of per-display source and
  rendered FPS, frame timing, drops, decode/presentation latency, and available
  process/GPU memory counters. `null` denotes an unavailable or inapplicable
  counter. Metric definitions, one-second sampling, and aggregation bounds are
  documented in [`performance-metrics.md`](performance-metrics.md).
- Decoded frames pass through a bounded three-frame PTS queue. The host sends
  only due frames to the recovery worker; static UI/IPC polling does not redraw.
- The playback scheduler and source-opening coordination live on a dedicated
  playback thread; each Media Foundation source reader stays on its own COM
  initialized worker thread. Bounded frame queues keep decoding from running
  ahead of playback when consumers are slow. Replacing a source
  signals the prior reader to stop, waits up to one second for its thread, and
  clears scheduled frames. The source worker retains at most one pending frame
  and pauses decoding when the bounded frame channel is full. Host shutdown
  explicitly signals the playback thread; each source worker gets a bounded
  stop and join before recovery
  releases its GPU/window resources. A playback worker that exceeds its
  five-second shutdown deadline is detached with a warning, and recovery
  resources stay alive until process termination so an in-flight frame cannot
  use a destroyed window.

## Error codes

`PlaybackCommandErrorCode` values are stable protocol data and contain no path
or OS diagnostic text:

| Code | Meaning |
| --- | --- |
| `playback_unavailable` | The playback worker is stopping or unavailable. |
| `video_load_in_progress` | A source open is still in progress; only one open is allowed at a time. |
| `video_not_loaded` | A transport command needs a loaded source. |
| `video_load_failed` | The requested source could not be opened. |
| `invalid_seek` | The seek position is outside the source duration. |
| `seek_failed` | Media Foundation rejected an otherwise valid seek. |
| `unsupported_speed` | The requested speed is not a supported preset. |
| `unsupported_fps` | The requested output cap is outside 1–120 FPS. |
| `playback_failed` | The scheduler could not apply the transport command. |
| `decode_failed` | Source decoding stopped after a decode error. |
| `playback_settings_invalid` | Playback settings are malformed or outside the active source duration. |
| `playback_settings_apply_failed` | The host could not apply the requested runtime settings. |
| `playback_settings_source_mismatch` | The request path does not match the active video source. |

Transport-level errors include `host_unavailable`, `timeout`, `disconnected`,
`malformed_response`, `message_too_large`, `protocol_version_mismatch`,
`transport_error`, and `unsupported_platform`.

`wallpaper-ui` currently exposes the ping request as the Tauri command
`host_ping`; video UI controls are outside Phase 3. `wallpaper-host` runs the
pipe server independently. See [`apps/ui/README.md`](../../apps/ui/README.md)
for development round-trip steps.
