# Media Foundation video source

`wallpaper-media::VideoSource::open` opens a local H.264 video source through the Media Foundation Source Reader. It selects the first video stream, deselects other streams, and exposes validated frame dimensions, frame-rate numerator/denominator, duration in 100 ns units, and the stream-selection kind. `read_next` yields decoded RGBA8 frames, stream ticks, and an explicit end-of-stream event. The bounded frame queue, playback scheduling, and D3D11 presentation remain separate Phase 3 responsibilities.

## Lifetime and errors

Each source owns one Media Foundation startup reference and a COM apartment guard. The Source Reader is dropped before `MFShutdown`; COM is uninitialized only when this code initialized it. A pre-existing COM apartment is reused without balancing another component's initialization. The source file is opened read-only and its bytes are unchanged.

The source is opened read-only. Safe application errors contain no source path; diagnostic summaries contain only a static stage/reason and, for operating-system failures, the numeric HRESULT. MP4s without a video stream, non-H.264 streams, invalid metadata, and source-reader failures return typed `MediaError` variants.

## Metadata validation

Width and height must be nonzero, no axis may exceed 16,384 pixels, and the total may not exceed 67,108,864 pixels. Frame-rate numerator and denominator must both be nonzero. Duration must be positive. These limits also bound decoded frame allocations.

## Decoded frame contract

`read_next` returns tightly packed, top-to-bottom RGBA8 frames with dimensions, stride, presentation timestamp, and sample duration. Timestamps use Media Foundation's 100 ns units and must be nonnegative and strictly increasing. A positive sample duration is used when present; otherwise it is calculated from the validated frame-rate rational. Source Reader stream ticks are returned with their timestamps, and end-of-stream remains explicit on subsequent reads. A current media-type change is accepted only if it remains RGB32 with the original dimensions; otherwise `VideoFormatChanged` is returned. Invalid sample layouts, pixel formats, timestamps, and decoder failures have distinct typed errors with path-free diagnostics. The independent decoded-frame queue is documented in [`video-frame-queue.md`](video-frame-queue.md).

## Hardware preference and fallback

`VideoSource::open` configures Media Foundation's software video-processing path. `VideoSource::open_with_d3d11_device` reuses the host's D3D11 device, supplies an `IMFDXGIDeviceManager`, enables hardware transforms, leaves DXVA enabled, and requests the advanced video-processing path. It does not require hardware-only transforms. If hardware reader setup or creation fails, opening falls back to the software path. DXGI-backed samples are copied through a bounded D3D11 staging texture into the same RGBA8 CPU contract; supported output textures include BGRA8, BGRX8, and RGBA8. The shared D3D11 immediate context is created with video support and multithread protection so renderer and decoder work can safely share it.

The Windows integration fixture decodes all 30 timestamped frames through both software and D3D11-device-backed paths. Hardware selection remains a Media Foundation policy decision; the API reports `HardwarePreferred` when configured and never claims hardware-only decoding.

Windows-target fixtures under `crates/media/tests/fixtures` include a one-second 64×48 H.264 MP4 at 30 FPS, a red-to-blue 64×48 H.264 clip for renderer update checks, an 80×64 H.264 clip for texture-dimension changes, an audio-only MP4, and a non-H.264 MPEG-4 MP4. They were generated with FFmpeg from deterministic color, `testsrc2`, and `sine` sources.

## References

- [Initializing Media Foundation](https://learn.microsoft.com/en-us/windows/desktop/medfound/initializing-media-foundation)
- [Using the Source Reader to Process Media Data](https://learn.microsoft.com/en-us/windows/win32/medfound/processing-media-data-with-the-source-reader)
- [IMFSourceReader::GetPresentationAttribute](https://learn.microsoft.com/en-us/windows/win32/api/mfreadwrite/nf-mfreadwrite-imfsourcereader-getpresentationattribute)
- [MF_MT_FRAME_SIZE](https://learn.microsoft.com/en-us/windows/win32/medfound/mf-mt-frame-size-attribute)
