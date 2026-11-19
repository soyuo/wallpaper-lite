# Phase 2 Image Renderer Acceptance

**Result:** Passed on the available Windows 11 system. Windows 10 remains unverified and is tracked separately by blocked issue [#6](https://github.com/soyuo/wallpaper-lite/issues/6).

## Environment

- Windows 11 Education, version 10.0.26300, x64 (build 26300)
- Intel Arc 140T GPU (8 GB), driver 32.0.101.6104
- Interactive Explorer desktop; D3D11 hardware device
- Verification date: 2026-11-20

## Acceptance results

| Criterion | Result | Evidence |
| --- | --- | --- |
| PNG, JPEG, and 4K images render behind desktop icons | Pass | WIC tests verify PNG pixels, JPEG decode, and 3840×2160 image dimensions. The runtime captures below show all three images behind the visible desktop icons. |
| Placement, transforms/crop, and color match documented behavior | Pass | GPU readback compares color controls against the CPU reference and checks Fit bars, Stretch fill, Cover/crop, translation, scale, anchor, and rotation. Geometry tests cover Center and wide/tall/4K dimensions. |
| Static redraw and renderer cleanup are verified | Pass | The idle test waits 250 ms and confirms the static frame count remains 1. After presenter drop, a GPU test confirms render targets, image views, constant buffers, shaders, samplers, and rasterizer state are unbound, then confirms the desktop child HWND is destroyed. |
| Reproducible steps, system details, and per-criterion evidence are recorded | Pass | Environment is recorded above; automated commands and runtime captures are listed below. |
| Windows 11 scope is explicit; Windows 10 is not represented as complete | Pass | Windows 10 runtime verification remains open under blocked issue #6. No Windows 10 result is claimed here. |

## Reproduction

Run from the repository root in an interactive Windows session with Explorer and a D3D11-capable GPU:

```powershell
cargo +stable test -p wallpaper-media -p wallpaper-desktop -p wallpaper-renderer -p wallpaper-host --target x86_64-pc-windows-msvc -- --test-threads=1
cargo +stable build -p wallpaper-host --target x86_64-pc-windows-msvc
cargo +stable clippy -p wallpaper-renderer -p wallpaper-host --target x86_64-pc-windows-msvc -- -D warnings
cargo +stable fmt --all -- --check
git diff --check
```

To manually display a fixture, launch the host from the repository root:

```powershell
target\x86_64-pc-windows-msvc\debug\wallpaper-host.exe --image crates\media\tests\fixtures\pixels.png
```

The WIC integration tests also use `solid.jpg` and `4k.png`. The GPU tests read back the D3D11 render target before presentation, so their pixel assertions inspect the current draw rather than the previously flipped swapchain buffer.

## Runtime captures

Each capture is a 1707×1067 desktop screenshot. The icons remain visible above the image.

- PNG fixture: [png-rendered.png](evidence/phase-2/png-rendered.png)
- JPEG fixture: [jpeg-rendered.png](evidence/phase-2/jpeg-rendered.png)
- 4K PNG fixture: [4k-rendered.png](evidence/phase-2/4k-rendered.png)

## Detailed behavior evidence

- `gpu_color_output_matches_the_linear_srgb_reference` checks neutral output and brightness, contrast, saturation, gamma, opacity, temperature, RGB balance, and combined adjustments against the CPU reference within 3 RGBA8 channel values.
- `gpu_placement_and_crop_output_matches_documented_geometry` reads GPU pixels for Fit letterboxing, Stretch fill, Cover with a left-half crop, physical-pixel translation, centered scale, top-left anchor, and 90° rotation.
- `presents_once_and_stays_idle_until_image_or_size_changes` confirms no additional present during a 250 ms idle interval; resizing and changing image or parameters are covered by separate redraw tests.
- WIC tests check PNG pixel values, JPEG decode, 3840×2160 dimensions, typed failures for malformed/unsupported input, and unchanged source bytes. Renderer color tests also preserve the in-memory source pixels.

## Limits

This report covers still images and the available Windows 11 runtime only. Windows 10, video playback, multi-monitor behavior, and editor UI are outside this verification. Issue #6 remains blocked; its Windows 10 acceptance is not inferred from these results.
