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

The current video preview uses the first decoded frame. Playback-synchronized
preview, interactive crop/resize controls, edit history, and applying settings
to the live desktop remain separate editor work.
