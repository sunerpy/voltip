#!/usr/bin/env python3
"""Cut the overlay pills out of a capture of the desktop front end's /overlay page.

Input, per capture: `<stem>.png`, a viewport screenshot taken at device pixel ratio 2,
and `<stem>.json`, the pill rectangles read in the same page (see docs/site/README.md):

    {"dpr": 2, "rects": {"listening": {"x": .., "y": .., "w": .., "h": ..}, ...}}

The stem ends in `-<lang>-<theme>`, for example `overlay-en-dark`. Each pill named by
--states (default: the home page's listening, processing and inserted) is cut at its
bounding box, given a rounded alpha mask (radius = height / 2, drawn at 4x and
downsampled for a smooth edge) and centred on a transparent canvas of one common width,
so every frame of the home page's cross-fade has the same size and scale. Without
--width each image is as wide as its pill. --pad adds transparent space above and
below: the output page shows the result card as a figure that fills the text column,
so the card sits at its real size in the middle of a column-wide canvas.

Output: `overlay-<state>-<lang>-<theme>.webp` (quality 85) in --out.

    python3 docs/site/tools/crop-overlay.py --width 440 --out docs/site/public/screens /tmp/overlay-*-*.png
    python3 docs/site/tools/crop-overlay.py --states card --width 688 --pad 20 --out docs/site/public/screens /tmp/overlay-*-*.png
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
    parser.add_argument("--width", type=float, help="canvas width in CSS px, at least the widest pill; omitted, each pill's own")
    parser.add_argument("--states", nargs="+", default=list(STATES), help=f"the pills to cut (default: {' '.join(STATES)})")
    parser.add_argument("--pad", type=float, default=0, help="transparent space above and below each pill, in CSS px")
    parser.add_argument("--out", type=Path, required=True, help="output directory")
    parser.add_argument("captures", nargs="+", help="the .png captures (or their paths without a suffix)")
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)

    stems = dict.fromkeys(str(Path(c).with_suffix("")) if Path(c).suffix in (".png", ".json") else c for c in args.captures)
    for stem in stems:
        meta = json.loads(Path(f"{stem}.json").read_text(encoding="utf-8"))
        dpr = meta["dpr"]
        capture = Image.open(f"{stem}.png")
        lang, theme = Path(stem).name.split("-")[-2:]
        for state in args.states:
            if state not in meta["rects"]:
                raise SystemExit(f"{stem}.json: no rectangle for {state}")
            tile = cut(capture, meta["rects"][state], dpr)
            canvas_width = tile.width if args.width is None else math.ceil(args.width * dpr)
            if tile.width > canvas_width:
                raise SystemExit(f"{stem}: the {state} pill is wider than --width {args.width}")
            pad = round(args.pad * dpr)
            canvas = Image.new("RGBA", (canvas_width, tile.height + 2 * pad), (0, 0, 0, 0))
            canvas.alpha_composite(tile, ((canvas_width - tile.width) // 2, pad))
            target = args.out / f"overlay-{state}-{lang}-{theme}.webp"
            canvas.save(target, "WEBP", quality=85, method=6)
            print(f"{target}  {canvas.width}x{canvas.height}")


if __name__ == "__main__":
    main()
