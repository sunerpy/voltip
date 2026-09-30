#!/usr/bin/env python3
"""Cut the overlay pills out of a capture of the desktop front end's /overlay page.

Input, per capture: `<stem>.png`, a viewport screenshot taken at device pixel ratio 2,
and `<stem>.json`, the pill rectangles read in the same page (see docs/site/README.md):

    {"dpr": 2, "rects": {"listening": {"x": .., "y": .., "w": .., "h": ..}, ...}}

The stem ends in `-<lang>-<theme>`, for example `overlay-en-dark`. Each pill is cut at
its bounding box, given a rounded alpha mask (radius = height / 2, drawn at 4x and
downsampled for a smooth edge) and centred on a transparent canvas of one common width,
so every frame of the home page's cross-fade has the same size and scale.

Output: `overlay-<state>-<lang>-<theme>.webp` (quality 85) in --out.

    python3 docs/site/tools/crop-overlay.py --width 440 --out docs/site/public/screens /tmp/overlay-*-*.png
"""

import argparse
import json
import math
from pathlib import Path

from PIL import Image, ImageDraw

STATES = ("listening", "processing", "inserted")
SUPERSAMPLE = 4


def pill_mask(width: int, height: int) -> Image.Image:
    big = Image.new("L", (width * SUPERSAMPLE, height * SUPERSAMPLE), 0)
    ImageDraw.Draw(big).rounded_rectangle(
        (0, 0, width * SUPERSAMPLE - 1, height * SUPERSAMPLE - 1),
        radius=height * SUPERSAMPLE // 2,
        fill=255,
    )
    return big.resize((width, height), Image.LANCZOS)


def cut(capture: Image.Image, rect: dict, dpr: float) -> Image.Image:
    x0, y0 = round(rect["x"] * dpr), round(rect["y"] * dpr)
    x1, y1 = round((rect["x"] + rect["w"]) * dpr), round((rect["y"] + rect["h"]) * dpr)
    tile = capture.crop((x0, y0, x1, y1)).convert("RGBA")
    tile.putalpha(pill_mask(x1 - x0, y1 - y0))
    return tile


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--width", type=float, required=True, help="canvas width in CSS px; at least the widest pill")
    parser.add_argument("--out", type=Path, required=True, help="output directory")
    parser.add_argument("captures", nargs="+", help="the .png captures (or their paths without a suffix)")
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)

    stems = dict.fromkeys(str(Path(c).with_suffix("")) if Path(c).suffix in (".png", ".json") else c for c in args.captures)
    for stem in stems:
        meta = json.loads(Path(f"{stem}.json").read_text(encoding="utf-8"))
        dpr = meta["dpr"]
        capture = Image.open(f"{stem}.png")
        canvas_width = math.ceil(args.width * dpr)
        lang, theme = Path(stem).name.split("-")[-2:]
        for state in STATES:
            tile = cut(capture, meta["rects"][state], dpr)
            if tile.width > canvas_width:
                raise SystemExit(f"{stem}: the {state} pill is wider than --width {args.width}")
            canvas = Image.new("RGBA", (canvas_width, tile.height), (0, 0, 0, 0))
            canvas.alpha_composite(tile, ((canvas_width - tile.width) // 2, 0))
            target = args.out / f"overlay-{state}-{lang}-{theme}.webp"
            canvas.save(target, "WEBP", quality=85, method=6)
            print(f"{target}  {canvas.width}x{canvas.height}")


if __name__ == "__main__":
    main()
