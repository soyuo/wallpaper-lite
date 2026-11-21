# Phase 6 Span Acceptance (#95)

## Result

The Span geometry and host wiring are implemented and covered by synthetic
geometry tests plus a Windows D3D11 readback test. The required physical
two-display visual check is unavailable on this machine, which has one active
display. Issue #95 therefore remains open and blocked; this report does not
claim full multi-display acceptance.

## Behavior covered

- A connected Span profile selects one wallpaper source for every active target;
  each target retains its own FPS override.
- The host computes the physical virtual-desktop union, gives each target its
  own positioned child window, and passes shared `SpanBounds` to its presenter.
- Placement, crop, transform, and source UVs are computed over the union. Each
  target maps the same global placement into its local viewport and clips the
  off-target portion. Explorer recovery reuses the same bounds.
- Synthetic tests cover negative virtual origins, horizontal and vertical
  layouts, mixed target sizes, gaps, crop, scale, translation, and rotation.
- A Windows D3D11 readback test applies left and right target viewports to the
  same child surface and verifies the left viewport samples the red side and
  the right viewport samples the blue side of `pixels.png`.
- The source fixture remains unchanged; SHA-256:
  `D1834273D59897DA619B76E1C0ADE794398AE09EFBF6E57896545FD38A4F60A8`.

## Environment and limits

- Windows 11 Education, 26H2, build `10.0.26300.9974`, x64.
- Intel Arc 140T, 8 GB; driver `32.0.101.6104`.
- One active physical display: 2560×1600 at 140% scale and approximately 144 Hz.
- No second physical display was available. Wallpaper continuity across real
  display edges, negative-origin physical layouts, mixed-DPI scanout, and
  real-display gaps remain unverified. Synthetic viewport readback is not a
  substitute for that visual check.
- Windows 10 runtime verification was unavailable.

## Reproduction

Run the renderer geometry tests and the `span_bounds_present_continuous_source_slices_for_each_target`
Windows test under an interactive Windows session with D3D11 hardware. For full
acceptance, configure at least two active displays with different physical
bounds, assign one Span wallpaper, and visually inspect the shared image across
the seam and any desktop gap. Confirm each target's FPS override independently,
then restart Explorer and stop the host to check recovery and cleanup.
