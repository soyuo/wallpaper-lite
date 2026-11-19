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
`set_placement_mode`. UI selection and persistent settings are later work.

The presenter creates a two-buffer `FLIP_SEQUENTIAL` swap chain for the existing
desktop child HWND. The desktop crate creates that child inside the shell-owned,
full-screen `WorkerW` below `SHELLDLL_DefView`, so the rendered image appears
behind the icon list. The presenter receives a non-owning HWND value and never
reparents or destroys the window. `DesktopRecoveryWorker` retains
`AttachedDesktopWindow` ownership and drops the presenter before dropping or
replacing that owner. The worker also retains the decoded CPU image so it can
restore presentation after Explorer recreates the child.

## Invalidation and resize

Construction, image replacement, and placement-mode changes each draw and
present one frame. Selecting the already-active mode is a no-op. The host does
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
