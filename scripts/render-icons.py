#!/usr/bin/env python3
"""Draw the app mark 「声波光标」 into the raster icons the desktop app and the React Native app ship.

The mark, on a 1024 canvas (packages/ui/src/components/Logo.tsx): a deep ink rounded square
(corner radius 232, gradient #0B1220 at the top left to #1B2A4A at the bottom right), three white
bars 84 wide with fully round ends (left edges 244, 376, 508; heights 220, 420, 300) and a cursor
72 wide (left edge 708, height 560, gradient #38BDF8 at the top to #22D3EE at the bottom), every
part centred on the canvas's middle.

At 16-48 px a bar is one to four pixels wide. Drawn as designed its edges fall between pixels and
each bar smears into a grey band, which is what `cargo tauri icon` produces when it shrinks one big
image. So every desktop size is drawn from the geometry fitted to that size's pixels (`fit`): the
bar width, the gaps and the heights are whole pixels, every straight edge lies on a pixel boundary,
and each part stays centred. Only the round ends and the tile's corners are anti-aliased (16, 8 or
4 samples per pixel side). At 1024 and 512 the fit is the design itself. The tray icon
(crates/voltip-platform/src/tray.rs) and the logo in the UI (Logo.tsx) fit the mark the same way.

Writes:
- apps/desktop/src-tauri/icons/: 32x32.png, 128x128.png, 128x128@2x.png (256 px), icon.png
  (1024 px); icon.ico with the sizes Windows draws at 100-400 % scaling, 32 px first because Tauri
  takes the first entry as the default window icon; icon.icns, 16 to 1024 px.
- apps/mobile-rn/assets/: icon.png (the whole mark), adaptive-icon.png (the bars and the cursor at
  two thirds, inside the 66 dp safe circle of the 108 dp canvas), adaptive-icon-background.png (the
  ink gradient, full bleed: the launcher masks the shape) and adaptive-icon-monochrome.png (the
  foreground in white), 1024 px each. Expo resizes them for each density and launchers scale the
  layers again, so these use the design's own geometry.

The Tauri phone app's launcher icons come from `cargo tauri icon` (apps/mobile/src-tauri/icon-source/).

Needs Python 3 with numpy and Pillow (`pip install numpy pillow`):

    python3 scripts/render-icons.py            # write every file
    python3 scripts/render-icons.py --fit 16 24 32    # print the fitted geometry
"""

from __future__ import annotations

import argparse
import io
import itertools
import math
import struct
from pathlib import Path

import numpy as np
from PIL import Image

ROOT = Path(__file__).resolve().parents[1]
DESKTOP = ROOT / "apps" / "desktop" / "src-tauri" / "icons"
MOBILE_RN = ROOT / "apps" / "mobile-rn" / "assets"

INK, INK_END = (0x0B, 0x12, 0x20), (0x1B, 0x2A, 0x4A)
WAVE = (0xFF, 0xFF, 0xFF)
CURSOR_TOP, CURSOR_BOTTOM = (0x38, 0xBD, 0xF8), (0x22, 0xD3, 0xEE)
# The mark on the 1024 canvas.
TILE_RADIUS = 232
BAR_WIDTH, BAR_GAP, CURSOR_WIDTH, CURSOR_GAP = 84, 48, 72, 116
BAR_HEIGHTS = (220, 420, 300)
CURSOR_HEIGHT = 560
BAR_LEFTS = (244, 376, 508)
CURSOR_LEFT = 708
# The adaptive icon's foreground: two thirds of the mark keep its corners (radius 259.7 on the
# 1024 canvas) inside the 66 dp safe circle of the 108 dp canvas (radius 312.9).
ADAPTIVE_SCALE = 0.67
# The sizes Windows draws an app icon at, 100-400 % scaling (learn.microsoft.com, "Construct your
# Windows app's icon"). The first entry is Tauri's default window icon.
ICO_SIZES = (32, 16, 20, 24, 30, 36, 40, 48, 60, 64, 72, 80, 96, 256)

Part = tuple[float, float, float, float]  # left, top, width, height, in pixels


def design(size: int, scale: float = 1.0) -> tuple[list[Part], Part]:
    """The bars and the cursor of a size × size image as designed, scaled about the centre."""
    px = size / 1024 * scale
    c = size / 2
    bars = [(c + (x - 512) * px, c - h * px / 2, BAR_WIDTH * px, h * px) for x, h in zip(BAR_LEFTS, BAR_HEIGHTS)]
    cursor = (c + (CURSOR_LEFT - 512) * px, c - CURSOR_HEIGHT * px / 2, CURSOR_WIDTH * px, CURSOR_HEIGHT * px)
    return bars, cursor


def near(target: float) -> list[int]:
    """The whole numbers on either side of `target`, at least 1."""
    return sorted({max(1, math.floor(target)), max(1, math.ceil(target))})


def fit(size: int, scale: float = 1.0) -> tuple[list[Part], Part]:
    """The bars and the cursor of a size × size image on whole pixels, the mark scaled by `scale`
    about the centre. Keep in step with `Mark::fitted` (tray.rs) and `fitMark` (Logo.tsx)."""
    px = size / 1024 * scale
    w_t, g_t, c_t, g2_t = BAR_WIDTH * px, BAR_GAP * px, CURSOR_WIDTH * px, CURSOR_GAP * px
    span = (3 * BAR_WIDTH + 2 * BAR_GAP + CURSOR_GAP + CURSOR_WIDTH) * px
    best = None
    for w, g, wc in itertools.product(near(w_t), near(g_t), near(c_t)):
        if wc > w:
            continue
        low = max(g + 1, math.floor(g2_t) - 1)
        for g2 in range(low, max(low + 1, math.ceil(g2_t) + 1) + 1):
            total = 3 * w + 2 * g + g2 + wc
            if (size - total) % 2:
                continue  # it could not sit centred on whole pixels
            # The bar width against its target, the gap and the cursor's width as shares of the
            # bar width (what the eye compares), the cursor's distance, and the whole span.
            cost = (
                3 * ((w - w_t) / w_t) ** 2
                + 2 * ((g / w - BAR_GAP / BAR_WIDTH) / (BAR_GAP / BAR_WIDTH)) ** 2
                + ((wc / w - CURSOR_WIDTH / BAR_WIDTH) / (CURSOR_WIDTH / BAR_WIDTH)) ** 2
                + ((g2 - g2_t) / g2_t) ** 2
                + 8 * ((total - span) / span) ** 2
            )
            if best is None or cost < best[0]:
                best = (cost, w, g, wc, g2, total)
    assert best is not None
    _, w, g, wc, g2, total = best
    left = (size - total) // 2
    # Heights take the image's parity, so each part is centred on whole pixels, and keep their
    # order: short bar < middle bar < tall bar < cursor.
    order = sorted(BAR_HEIGHTS) + [CURSOR_HEIGHT]
    targets = [h * px for h in order]
    floors = [w, w, w, wc]
    options = [[v for v in range(max(lo, math.floor(t) - 4), math.floor(t) + 6) if (v - size) % 2 == 0] for t, lo in zip(targets, floors)]
    best_heights = None
    for heights in itertools.product(*options):
        if not all(a < b for a, b in zip(heights, heights[1:])):
            continue
        cost = sum(((h - t) / t) ** 2 for h, t in zip(heights, targets))
        if best_heights is None or cost < best_heights[0]:
            best_heights = (cost, heights)
    assert best_heights is not None
    height = dict(zip(order, best_heights[1]))
    bars = [(left + i * (w + g), (size - height[h]) // 2, w, height[h]) for i, h in enumerate(BAR_HEIGHTS)]
    hc = height[CURSOR_HEIGHT]
    cursor = (left + 3 * w + 2 * g + g2, (size - hc) // 2, wc, hc)
    return bars, cursor


def render(size: int, parts: tuple[list[Part], Part], *, tile: str | None = "rounded", white: bool = False) -> Image.Image:
    """Draw the mark: `tile` "rounded" (the icon), "full" (the adaptive background, no parts) or
    None (the adaptive foreground); `white` draws the parts in white (the monochrome layer)."""
    bars, cursor = parts
    ss = 16 if size <= 64 else 8 if size <= 256 else 4
    radius = TILE_RADIUS / 1024 * size
    xs = (np.arange(size * ss) + 0.5) / ss
    out = np.zeros((size, size, 4))
    rows = max(1, 2_000_000 // (size * ss * ss))  # output rows per strip: about 2 M samples
    for y0 in range(0, size, rows):
        y1 = min(size, y0 + rows)
        ys = (np.arange(y0 * ss, y1 * ss) + 0.5) / ss
        x, y = np.meshgrid(xs, ys)
        colour = np.zeros(x.shape + (3,))
        alpha = np.zeros(x.shape)
        if tile is not None:
            if tile == "rounded":
                dx = np.maximum(np.maximum(radius - x, x - (size - radius)), 0)
                dy = np.maximum(np.maximum(radius - y, y - (size - radius)), 0)
                inside = dx * dx + dy * dy <= radius * radius
            else:
                inside = np.ones(x.shape, dtype=bool)
            # The gradient runs along the diagonal, as SVG's x1=0 y1=0 x2=1 y2=1 does.
            t = np.clip((x + y) / (2 * size), 0, 1)[..., None]
            colour = np.where(inside[..., None], np.array(INK) * (1 - t) + np.array(INK_END) * t, colour)
            alpha = np.where(inside, 1.0, alpha)
        if tile != "full":
            for part, paint in [(b, None) for b in bars] + [(cursor, "cursor")]:
                left, top, width, height = part
                r = width / 2
                cy = np.clip(y, top + r, top + height - r)
                inside = (x - (left + r)) ** 2 + (y - cy) ** 2 <= r * r
                if white or paint is None:
                    fill = np.broadcast_to(np.array(WAVE, dtype=float), colour.shape)
                else:
                    t = np.clip((y - top) / height, 0, 1)[..., None]
                    fill = np.array(CURSOR_TOP) * (1 - t) + np.array(CURSOR_BOTTOM) * t
                colour = np.where(inside[..., None], fill, colour)
                alpha = np.where(inside, 1.0, alpha)
        # Each pixel is the average of its samples: premultiplied colour, then straight again.
        n = y1 - y0
        premultiplied = (colour * alpha[..., None]).reshape(n, ss, size, ss, 3).mean(axis=(1, 3))
        coverage = alpha.reshape(n, ss, size, ss).mean(axis=(1, 3))
        out[y0:y1, :, :3] = np.where(coverage[..., None] > 0, premultiplied / np.maximum(coverage[..., None], 1e-12), 0)
        out[y0:y1, :, 3] = coverage * 255
    return Image.fromarray(np.clip(np.rint(out), 0, 255).astype(np.uint8), "RGBA")


def icon(size: int) -> Image.Image:
    """The whole mark at `size` px, fitted to its pixels."""
    return render(size, fit(size))


def png(image: Image.Image) -> bytes:
    buffer = io.BytesIO()
    image.save(buffer, "PNG", optimize=True)
    return buffer.getvalue()


def ico(frames: list[Image.Image]) -> bytes:
    """An .ico of PNG frames, in the order given."""
    data = [png(frame) for frame in frames]
    out = bytearray(struct.pack("<HHH", 0, 1, len(frames)))
    offset = 6 + 16 * len(frames)
    for frame, blob in zip(frames, data):
        side = frame.width if frame.width < 256 else 0
        out += struct.pack("<BBBBHHII", side, side, 0, 0, 1, 32, len(blob), offset)
        offset += len(blob)
    for blob in data:
        out += blob
    return bytes(out)


def icns_rle(channel: bytes) -> bytes:
    """Apple's run-length coding for the 16 and 32 px icns entries, one colour channel."""
    out = bytearray()
    i = 0
    while i < len(channel):
        run = 1
        while i + run < len(channel) and run < 130 and channel[i + run] == channel[i]:
            run += 1
        if run >= 3:
            out += bytes([0x80 + run - 3, channel[i]])
            i += run
            continue
        start = i
        while i < len(channel) and i - start < 128:
            if i + 2 < len(channel) and channel[i] == channel[i + 1] == channel[i + 2]:
                break
            i += 1
        out += bytes([i - start - 1]) + channel[start:i]
    return bytes(out)


def icns(frames: dict[int, Image.Image]) -> bytes:
    """An .icns: the 16 and 32 px sizes as RLE RGB plus an 8-bit mask (what every macOS version
    reads), the rest as PNG, @2x included."""
    entries: list[tuple[bytes, bytes]] = []
    for size, (rgb_type, mask_type) in ((16, (b"is32", b"s8mk")), (32, (b"il32", b"l8mk"))):
        r, g, b, a = frames[size].split()
        entries.append((rgb_type, b"".join(icns_rle(channel.tobytes()) for channel in (r, g, b))))
        entries.append((mask_type, a.tobytes()))
    for kind, size in ((b"ic11", 32), (b"ic12", 64), (b"ic07", 128), (b"ic13", 256), (b"ic08", 256), (b"ic14", 512), (b"ic09", 512), (b"ic10", 1024)):
        entries.append((kind, png(frames[size])))
    body = b"".join(kind + struct.pack(">I", len(data) + 8) + data for kind, data in entries)
    return b"icns" + struct.pack(">I", len(body) + 8) + body


def write(path: Path, data: bytes) -> None:
    path.write_bytes(data)
    print(f"{path.relative_to(ROOT)}  {len(data):,} bytes")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--fit", type=int, nargs="+", metavar="SIZE", help="print the fitted geometry and exit")
    args = parser.parse_args()
    if args.fit:
        for size in args.fit:
            print(size, fit(size))
        return

    frames = {size: icon(size) for size in sorted({16, 32, 64, 128, 256, 512, 1024, *ICO_SIZES})}
    write(DESKTOP / "32x32.png", png(frames[32]))
    write(DESKTOP / "128x128.png", png(frames[128]))
    write(DESKTOP / "128x128@2x.png", png(frames[256]))
    write(DESKTOP / "icon.png", png(frames[1024]))
    write(DESKTOP / "icon.ico", ico([frames[size] for size in ICO_SIZES]))
    write(DESKTOP / "icon.icns", icns(frames))

    foreground = design(1024, ADAPTIVE_SCALE)
    write(MOBILE_RN / "icon.png", png(frames[1024]))
    write(MOBILE_RN / "adaptive-icon.png", png(render(1024, foreground, tile=None)))
    write(MOBILE_RN / "adaptive-icon-background.png", png(render(1024, foreground, tile="full")))
    write(MOBILE_RN / "adaptive-icon-monochrome.png", png(render(1024, foreground, tile=None, white=True)))


if __name__ == "__main__":
    main()
