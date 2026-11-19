# Still-image decoding

Phase 2 introduces `crates/media` as the boundary between a source file and the
renderer. The current `decode_image` implementation uses Windows Imaging
Component (WIC) for PNG and JPEG files and returns owned CPU pixels. It does not
upload textures, create windows, render frames, or modify source files.

## Decoded image contract

- The output format is tightly packed, top-to-bottom RGBA8.
- `stride` is `width * 4`; the owned pixel buffer contains `stride * height`
  bytes.
- The first frame is decoded. Animated PNG behavior is outside this still-image
  API.
- WIC's container format is checked, so a different supported WIC format is
  returned as `UnsupportedImageFormat` even if WIC can decode it.
- Each axis is limited to 16,384 pixels and the total image to 67,108,864 pixels
  (64 × 1024² pixels; 256 MiB of RGBA8 data). Dimensions and checked buffer sizes are validated
  before allocating the owned output buffer.
- The source is opened with read access only. Safe application errors and
  numeric diagnostics do not include source paths.
- COM initialization is balanced on the calling thread. If the thread already
  uses a different COM apartment model, decoding uses that existing apartment
  without uninitializing it.

`decode_image` is synchronous. Callers doing interactive work should run it on
a worker thread. Swap-chain integration follows in the next Phase 2 issue.

## Verification

Run the WIC tests on Windows:

```powershell
cargo +stable test -p wallpaper-media --target x86_64-pc-windows-msvc -- --test-threads=1
```

The current fixtures cover exact PNG RGBA/alpha output, JPEG channel values,
malformed input, source preservation, dimension limits, and a 3840×2160 PNG.
