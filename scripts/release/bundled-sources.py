#!/usr/bin/env python3
"""Print the text of every source Metro bundled into the Android app, from the packager source map
(scripts/build-android-rn.sh scans it for provider keys: the bundle itself is Hermes bytecode, whose
packed string table reads like keys). The map is a plain one with `sourcesContent`, or an index map
whose sections embed such maps (Metro in React Native 0.88, Expo SDK 58), nested to any depth.

Usage: bundled-sources.py <index.android.bundle.packager.map>   (the text goes to stdout)
Fails when the map carries no source text at all, or a section names another file instead of
embedding its map, so that the scan never passes over sources it did not read.
"""

from __future__ import annotations

import json
import sys


def contents(source_map: dict) -> list[str]:
    if "sections" in source_map:
        texts: list[str] = []
        for section in source_map["sections"]:
            if not isinstance(section.get("map"), dict):
                sys.exit("a section of the packager source map has no embedded map")
            texts.extend(contents(section["map"]))
        return texts
    return [text for text in source_map.get("sourcesContent") or [] if text]


def main() -> None:
    if len(sys.argv) != 2:
        sys.exit("usage: bundled-sources.py <packager source map>")
    with open(sys.argv[1], encoding="utf-8") as file:
        texts = contents(json.load(file))
    if not texts:
        sys.exit("no sourcesContent in the packager source map")
    for text in texts:
        print(text)


if __name__ == "__main__":
    main()
