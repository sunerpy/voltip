#!/usr/bin/env python3
"""Cut one card or overlay pill out of a capture, with its rounded corners transparent.

The home pages show two kinds of cut-out (docs/site/README.md, "Screenshots"):

- the phone panels, `phone-pair-<lang>-<theme>.webp` and `phone-mic-<lang>-<theme>.webp`:
  cards of the desktop front end's 手机 page, captured at device pixel ratio 2, with the
  cards' 24 px corners (48 in the capture);
- the hero's overlay frames, `hero-live-<lang>.webp` and `hero-inserted-<lang>.webp`: the
  overlay in a frame of a screen recording of a real dictation, cut as a capsule.

The rectangle and the radius are in the capture's own pixels: multiply what
getBoundingClientRect() reports by the device pixel ratio. The mask is drawn at 4x and
downsampled for a smooth edge, as crop-overlay.py does for the /overlay pills.

    python3 docs/site/tools/crop-card.py --rect 534,235,1482,584 --radius 48 \
        /tmp/phone-zh-light.png docs/site/public/screens/phone-pair-zh-light.webp
    python3 docs/site/tools/crop-card.py --rect 1478,1995,884,104 --pill \
        /tmp/frame.png docs/site/public/screens/hero-live-en.webp
"""

import argparse
from pathlib import Path

from PIL import Image, ImageDraw

SUPERSAMPLE = 4


def rounded_mask(width: int, height: int, big_radius: int) -> Image.Image:
    """An alpha mask of the rectangle; `big_radius` is at the supersampled scale."""
    big = Image.new("L", (width * SUPERSAMPLE, height * SUPERSAMPLE), 0)
    ImageDraw.Draw(big).rounded_rectangle(
        (0, 0, width * SUPERSAMPLE - 1, height * SUPERSAMPLE - 1),
        radius=big_radius,
        fill=255,
    )
    return big.resize((width, height), Image.LANCZOS)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--rect", required=True, help="x,y,width,height in the capture's pixels")
    shape = parser.add_mutually_exclusive_group(required=True)
    shape.add_argument("--radius", type=int, help="corner radius in the capture's pixels")
    shape.add_argument("--pill", action="store_true", help="round the ends: radius = height / 2")
    parser.add_argument("capture", type=Path, help="the screenshot or video frame (PNG)")
    parser.add_argument("out", type=Path, help="the .webp to write")
    args = parser.parse_args()

    parts = args.rect.split(",")
    if len(parts) != 4:
        parser.error("--rect takes four numbers: x,y,width,height")
    x, y, w, h = (round(float(part)) for part in parts)
    capture = Image.open(args.capture).convert("RGBA")
    if x < 0 or y < 0 or w <= 0 or h <= 0 or x + w > capture.width or y + h > capture.height:
        raise SystemExit(f"{args.capture}: {args.rect} is not inside the {capture.width}x{capture.height} capture")

    tile = capture.crop((x, y, x + w, y + h))
    big_radius = h * SUPERSAMPLE // 2 if args.pill else args.radius * SUPERSAMPLE
    tile.putalpha(rounded_mask(w, h, big_radius))
    args.out.parent.mkdir(parents=True, exist_ok=True)
    tile.save(args.out, "WEBP", quality=90, alpha_quality=100, method=6)
    print(f"{args.out}  {tile.width}x{tile.height}")


if __name__ == "__main__":
    main()
