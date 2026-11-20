# Wallpaper editor preview

The editor preview is rendered inside the existing Tauri application window. It
does not create another desktop or top-level window and does not modify saved
settings when the editor window changes size.

`editor_monitors` returns monitor physical dimensions from the application window.
The user selects a display profile; `editor_preview` loads that wallpaper's
validated settings, decodes an image or the first available video frame, and
passes the RGBA8 source to `wallpaper_renderer::render_editor_preview`.

The preview keeps the selected display aspect ratio and caps its longest edge at
1280 pixels. Position and explicit size are scaled from monitor physical pixels
to preview pixels. The renderer and preview share placement geometry, crop,
transform, and color adjustment code. The preview is encoded to BMP for display
inside the editor canvas. Resizing the app window only scales this already
rendered image in the UI; it does not reload or save editor settings.

Windows GPU readback compares the `pixels.png` fixture rendered by D3D11 and by
the editor preview at 1280×720 using a combined non-default crop, physical
position and size, anisotropic scale, rotation, off-center anchor, and color
adjustments. For pixels at least 2 physical pixels inside the transformed quad,
maximum per-channel RGB difference is 5 values on the 8-bit scale. The two-pixel
edge band is excluded because D3D's triangle fill convention and the CPU preview
use different boundary coverage rules; alpha is ignored because the desktop
swap chain uses an ignore-alpha mode. A unit check also verifies the preview
output aspect ratio and 1280-pixel edge cap.

The current video preview uses the first decoded frame. The canvas supports
all five placement modes, drag-to-position, corner resizing, physical-pixel
position/size fields, zoom, rotation, normalized crop selection/move/resize,
aspect presets, and crop reset. Selecting a standard placement mode resets
custom geometry while preserving the crop; using a transform control enters
Custom mode. Shift locks the image resize aspect ratio; Alt resizes around its
center. Edits remain a validated draft until **Save and preview** stores them through
`EditorSettingsStore` and renders the saved result. Pointer deltas are converted
from CSS canvas units to monitor physical pixels. Playback-synchronized preview,
edit history, transactional Apply/Cancel, and applying settings to the live
desktop remain later work.

The editor exposes brightness, contrast, saturation, gamma, opacity, color
temperature, and per-channel RGB balance with the shared settings ranges and
defaults; **Reset color** restores those defaults. Both the CPU preview and
`D3d11ImagePresenter::set_editor_color_settings` convert the same
`EditorColorSettings` through `ColorAdjustments::from_editor_settings`, then
use the color ordering documented in [`image-presentation.md`](image-presentation.md).
The parity test compares this shared settings path against D3D11 readback.

## Edit-session history

The UI keeps complete validated-settings snapshots in memory for the active
editor session. It retains at most 100 undo steps; later edits discard the
oldest step. Reset is recorded as an ordinary edit, and any new edit after an
undo clears redo history. Continuous canvas drags/resizes and range-slider
gestures are coalesced into one step. Closing the preview drops the session
history. `Ctrl/Cmd+Z` undoes, `Ctrl/Cmd+Y` redoes, and `Delete` resets all
editor settings; these shortcuts are ignored while focus is in a text or
selection control. History changes remain drafts until **Save and preview**.
