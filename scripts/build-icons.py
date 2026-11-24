"""Regenerate the app icon files from apps/ui/src-tauri/icons/icon.svg.

Writes:
  apps/ui/src-tauri/icons/icon.ico   multi-size Windows icon (16-256 px)
  apps/ui/src-tauri/icons/icon.png   512 px PNG used by Tauri
  docs/assets/icon.png               256 px PNG shown in the README

Requirements: Python 3 with Pillow, and Microsoft Edge or Google Chrome. The SVG is
rendered once at 1024 px with a transparent background by headless Chromium and then
scaled down for each size, so every size comes from the same vector source.

Usage: python scripts/build-icons.py [--browser PATH] [--output-root DIR]
"""

import argparse
import io
import struct
import subprocess
import sys
import tempfile
from pathlib import Path

from PIL import Image

REPO = Path(__file__).resolve().parent.parent
SVG = REPO / "apps" / "ui" / "src-tauri" / "icons" / "icon.svg"
ICO_SIZES = [16, 20, 24, 32, 40, 48, 64, 96, 128, 256]
RENDER_SIZE = 1024
BROWSER_CANDIDATES = [
    r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
    r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
    r"C:\Program Files\Google\Chrome\Application\chrome.exe",
    r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
]


def find_browser(explicit):
    candidates = [explicit] if explicit else BROWSER_CANDIDATES
    for candidate in candidates:
        if candidate and Path(candidate).is_file():
            return candidate
    sys.exit("No Edge or Chrome found. Pass --browser PATH.")


def render_master(browser):
    """Render the SVG to a RENDER_SIZE square RGBA image with a transparent background."""
    source = SVG.read_text(encoding="utf-8")
    if 'width="256" height="256"' not in source:
        sys.exit("icon.svg must declare width=\"256\" height=\"256\" on its root element.")
    sized = source.replace(
        'width="256" height="256"', f'width="{RENDER_SIZE}" height="{RENDER_SIZE}"', 1
    )
    # Browser helper processes can outlive the screenshot and still hold files in the
    # profile folder, so a failed cleanup must not abort a render that worked.
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as work:
        svg_path = Path(work) / "icon.svg"
        png_path = Path(work) / "master.png"
        svg_path.write_text(sized, encoding="utf-8")
        subprocess.run(
            [
                browser,
                "--headless=new",
                "--disable-gpu",
                "--hide-scrollbars",
                "--default-background-color=00000000",
                f"--window-size={RENDER_SIZE},{RENDER_SIZE}",
                f"--screenshot={png_path}",
                f"--user-data-dir={Path(work) / 'profile'}",
                svg_path.as_uri(),
            ],
            check=True,
            capture_output=True,
            timeout=120,
        )
        if not png_path.is_file():
            sys.exit("The browser did not write a screenshot.")
        image = Image.open(png_path).convert("RGBA")
    if image.size != (RENDER_SIZE, RENDER_SIZE):
        sys.exit(f"Browser produced a {image.size} image, expected {RENDER_SIZE}x{RENDER_SIZE}.")
    corners = [(0, 0), (RENDER_SIZE - 1, 0), (0, RENDER_SIZE - 1), (RENDER_SIZE - 1, RENDER_SIZE - 1)]
    if any(image.getpixel(corner)[3] != 0 for corner in corners):
        sys.exit("A rendered corner is not transparent; the browser ignored the transparent background.")
    # The icon fills its canvas, so the opaque area must reach every edge.
    left, top, right, bottom = image.getchannel("A").getbbox() or (0, 0, 0, 0)
    if (left, top, right, bottom) != (0, 0, RENDER_SIZE, RENDER_SIZE):
        sys.exit(f"The rendered icon does not fill the canvas (bounds {(left, top, right, bottom)}).")
    return image


def png_bytes(image):
    buffer = io.BytesIO()
    image.save(buffer, format="PNG", optimize=True)
    return buffer.getvalue()


def write_ico(path, master):
    """Write an ICO whose entries are PNG images, one per size (Windows Vista and later)."""
    entries = []
    for size in ICO_SIZES:
        scaled = master.resize((size, size), Image.Resampling.LANCZOS)
        entries.append((size, png_bytes(scaled)))
    header = struct.pack("<HHH", 0, 1, len(entries))
    offset = len(header) + 16 * len(entries)
    directory = b""
    for size, data in entries:
        # A stored size of 0 means 256.
        dimension = 0 if size == 256 else size
        directory += struct.pack("<BBBBHHII", dimension, dimension, 0, 0, 1, 32, len(data), offset)
        offset += len(data)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(header + directory + b"".join(data for _, data in entries))


def write_png(path, master, size):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(png_bytes(master.resize((size, size), Image.Resampling.LANCZOS)))


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--browser")
    parser.add_argument("--output-root", type=Path, default=REPO)
    args = parser.parse_args()

    master = render_master(find_browser(args.browser))
    root = args.output_root.resolve()
    icons = root / "apps" / "ui" / "src-tauri" / "icons"
    write_ico(icons / "icon.ico", master)
    write_png(icons / "icon.png", master, 512)
    write_png(root / "docs" / "assets" / "icon.png", master, 256)
    print(f"Wrote icons under {root}")


if __name__ == "__main__":
    main()
