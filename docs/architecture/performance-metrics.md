# Playback and rendering metrics

The host publishes a versioned performance snapshot through the named-pipe IPC
`GetPerformanceMetrics` command. The current payload uses schema version `1` and
protocol version `11`. Metrics are available to clients; a dashboard is outside
this feature's scope.

## Snapshot contract

The snapshot includes process working-set and private bytes, DXGI local and
non-local GPU usage and budgets, and a bounded list of per-display targets.
Every memory value and each FPS, frame-interval, or latency value is optional.
JSON `null` means the counter is unavailable, unsupported, not yet sampled, or
not applicable; clients must not interpret it as zero. Image targets report
their media kind and leave continuous playback/rendering counters unavailable.

For video targets, source FPS counts decoded frames accepted by the playback
actor during the sample window; rendered FPS counts frames whose presentation
completed successfully. Frame interval is derived from increasing source PTS
values and reported in milliseconds. Decode latency measures the source-reader
read operation; presentation latency measures the synchronous host-to-presenter
operation. Dropped-frame count is cumulative for the actor lifetime and includes
late, capacity-rejected, evicted, and superseded frames. Intentional queue clears
for seeks do not count as drops.

## Sampling and bounds

Playback counters publish on one-second windows. Process and GPU memory queries
run on a separate stoppable worker at one-second intervals so those Windows API
calls do not run on playback ticks. Sampling errors produce `null` for the
unavailable memory counters. Snapshot reads use a mutex around a bounded
16-target map; display IDs are limited to 256 UTF-8 bytes. IPC frames are capped
at 16 KiB, leaving room for the complete bounded snapshot.

Each display has its own recorder and playback actor. A target's FPS, timing,
or drops therefore cannot be combined with another display's counters. Registry
tests cover the cadence, target isolation, and capacity bounds; frame-queue tests
cover the cumulative drop counter. Runtime metrics are observable through the
typed IPC response and can be queried without subscribing to playback events.
