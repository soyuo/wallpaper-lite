# Static image presentation

## Data and ownership

`wallpaper-media::DecodedImage` supplies tightly packed, top-to-bottom RGBA8
pixels. `wallpaper-renderer::D3d11ImagePresenter` uploads those pixels into an
immutable D3D11 texture and samples it in a rectangle shader. The renderer
exposes validated physical-pixel dimensions and four placement modes:
`Cover` preserves aspect ratio and crops the source, `Fit` preserves aspect
ratio and clears unused output to black, `Stretch` fills the output and may
distort, and `Center` uses one output pixel per source pixel and lets the
client-area clip oversized images. `new` and the current host startup path
default to `Cover`, preserving aspect ratio unless a caller explicitly selects
`Stretch`. Renderer callers can choose a mode with `new_with_placement` or
`set_placement_mode`, and supply transforms with `new_with_transform` or
`set_transform`. UI selection and persistent settings are later work.

`ImageTransform` adds runtime-only position, scale, rotation, anchor, and crop
parameters. Position is a physical-pixel translation applied after the
placement-mode rectangle is computed; positive X moves right and positive Y
moves down. Scale is relative to that rectangle, rotation is in degrees
clockwise in the output coordinate system, and the normalized anchor selects
the pivot within the rectangle. Crop uses normalized top-left-origin source
coordinates and is composed before placement, so `Cover` and `Fit` use the
cropped region's aspect ratio. The order is crop, placement, scale/rotation
around the anchor, then physical-pixel translation. Defaults are zero position,
unit scale, zero rotation, centered anchor, and full-source crop. Geometry uses
the HWND's physical client dimensions; editor CSS pixels are not accepted by
the renderer API.

The presenter creates a two-buffer `FLIP_SEQUENTIAL` swap chain for the existing
desktop child HWND. The desktop crate creates that child inside the shell-owned,
full-screen `WorkerW` below `SHELLDLL_DefView`, so the rendered image appears
behind the icon list. The presenter receives a non-owning HWND value and never
reparents or destroys the window. `DesktopRecoveryWorker` retains
`AttachedDesktopWindow` ownership and drops the presenter before dropping or
replacing that owner. The worker also retains the decoded CPU image so it can
restore presentation after Explorer recreates the child.

## Color adjustments

`ColorAdjustments` applies runtime-only GPU controls in linear sRGB, in this
order: decode sRGB; multiply red/blue by the normalized temperature factors
`1 + temperature * 0.1` and `1 - temperature * 0.1`, then multiply all channels
by their RGB balance; apply brightness and contrast around linear value `0.18`;
mix saturation around Rec. 709 luminance `(0.2126, 0.7152, 0.0722)`; apply gamma
as `pow(channel, 1 / gamma)`; multiply by opacity; clamp and encode to sRGB.
Brightness, contrast, saturation and each RGB balance accept `0..=4`; gamma
accepts `0.1..=4`; opacity accepts `0..=1`; temperature accepts `-1..=1`.
Non-finite or out-of-range input returns `InvalidColorAdjustments`. Neutral
defaults are brightness/contrast/saturation/gamma/opacity/RGB balance `1`, and
temperature `0`.

The HWND swap chain uses `DXGI_ALPHA_MODE_IGNORE`, so the opacity control fades
RGB toward black; it does not reveal the desktop beneath the child. Source PNG
alpha is ignored consistently with that opaque presentation path. A future
transparent composition path would be needed for true per-pixel blending. See
[Microsoft's DXGI alpha mode reference](https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_2/ne-dxgi1_2-dxgi_alpha_mode).
Changing color controls presents one frame only after a changed value is
successfully prepared; unchanged settings do not redraw, and a failed present
leaves the prior settings active so the caller can retry.

## Invalidation and resize

Construction, image replacement, placement-mode changes, and transform changes
each draw and present one frame. Reapplying the active mode or transform is a
no-op. The host does
not run an animation or continuous render loop. While an image is active,
the recovery worker checks the child client size every 100 ms; unchanged sizes
do not call D3D11 or present a frame. A size change unbinds the old render target,
releases its view, resizes swap-chain buffers, creates a new render-target view,
and presents the retained image once.

Renderer errors are returned as safe typed errors and logged with a static error
code and numeric HRESULT. A failed image presentation leaves the desktop child
and shell process alive. The worker releases swap-chain, texture, shader, and
view references during shutdown and before Explorer recovery replaces a child.

## Startup input

For the current renderer acceptance path, the host accepts one optional image:

```powershell
wallpaper-host.exe --image C:\path\to\wallpaper.png
```

PNG and JPEG files use the WIC decoder described in
[`media-decoding.md`](media-decoding.md). The path is never included in logs and
the source file is opened read-only. With no argument, the host starts the
background-layer child without creating a swap chain; UI selection and
persistent wallpaper configuration are later work.

## Verification boundary

Placement geometry tests cover wide and tall source/output aspect ratios,
4K source dimensions, Center clipping, and rejection of zero dimensions. The
transform tests cover identity across all placement modes, physical-pixel
translation, scale, rotation around a normalized anchor, normalized crop
composition, and typed errors for invalid crop/transform values or
unrepresentable geometry. The transform GPU smoke test checks one redraw per
change and confirms the PNG source bytes remain unchanged after transformation.
Color reference tests verify neutral exactness, each adjustment, opacity fading
to black independent of ignored source alpha, and typed rejection of invalid
values. The GPU color smoke test verifies runtime shader compilation, redraw
after a changed setting only, retry after a failed update, and unchanged PNG
source bytes.
Windows test `placement_changes_present_once_and_reselecting_the_mode_stays_idle`
checks that mode changes present one frame and selecting the current mode does
not redraw. `presents_once_and_stays_idle_until_image_or_size_changes` checks
one initial present and no additional frame while idle. The
`resize_recreates_buffers_and_presents_at_the_new_client_size` test changes the
child size to 800×600 and verifies a resize-triggered present. The host test
`presented_image_resources_release_when_the_recovery_worker_stops` exercises
presentation and orderly worker shutdown. These tests require an interactive
Windows desktop and hardware D3D11. On Windows 11 Education 10.0.26300 x64, the
startup-image smoke test displayed the decoded red/blue fixture behind all three
desktop icons, and a click on Recycle Bin selected it while the image remained
visible. Windows 10 runtime verification remains part of blocked issue #6.
