# Media Foundation video source

`wallpaper-media::VideoSource::open` opens a local H.264 video source through the Media Foundation Source Reader. It selects the first video stream, deselects other streams, and exposes validated frame dimensions, frame-rate numerator/denominator, duration in 100 ns units, and the stream-selection kind.

Opening a source does not decode samples. Timestamped frame decoding, the bounded queue, playback scheduling, and D3D11 frame presentation are separate Phase 3 responsibilities.

## Lifetime and errors

Each source owns one Media Foundation startup reference and a COM apartment guard. The Source Reader is dropped before `MFShutdown`; COM is uninitialized only when this code initialized it. A pre-existing COM apartment is reused without balancing another component's initialization.

The source is opened read-only. Safe application errors contain no source path; diagnostic summaries contain only a static stage/reason and, for operating-system failures, the numeric HRESULT. MP4s without a video stream, non-H.264 streams, invalid metadata, and source-reader failures return typed `MediaError` variants.

## Metadata validation

Width and height must be nonzero, no axis may exceed 16,384 pixels, and the total may not exceed 67,108,864 pixels. Frame-rate numerator and denominator must both be nonzero. Duration must be positive. This bounds later frame allocations before sample decoding is implemented.

Windows-target fixtures under `crates/media/tests/fixtures` include a one-second 64×48 H.264 MP4 at 30 FPS, an audio-only MP4, and a non-H.264 MPEG-4 MP4. They were generated with FFmpeg from deterministic `testsrc2` and `sine` sources.

## References

- [Initializing Media Foundation](https://learn.microsoft.com/en-us/windows/desktop/medfound/initializing-media-foundation)
- [Using the Source Reader to Process Media Data](https://learn.microsoft.com/en-us/windows/win32/medfound/processing-media-data-with-the-source-reader)
- [IMFSourceReader::GetPresentationAttribute](https://learn.microsoft.com/en-us/windows/win32/api/mfreadwrite/nf-mfreadwrite-imfsourcereader-getpresentationattribute)
- [MF_MT_FRAME_SIZE](https://learn.microsoft.com/en-us/windows/win32/medfound/mf-mt-frame-size-attribute)
