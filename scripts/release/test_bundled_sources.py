"""Tests for scripts/release/bundled-sources.py: the text of every source Metro bundled into the
Android app, read from the packager source map, for the provider key scan of
scripts/build-android-rn.sh (the bundle itself is Hermes bytecode, whose packed string table reads
like keys).

Run: python3 -m unittest discover -s scripts/release -p 'test_*.py'
"""

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent / "bundled-sources.py"


def run(source_map: object) -> subprocess.CompletedProcess[str]:
    with tempfile.TemporaryDirectory() as tmp:
        path = Path(tmp) / "index.android.bundle.packager.map"
        path.write_text(json.dumps(source_map), encoding="utf-8")
        return subprocess.run([sys.executable, "-I", str(SCRIPT), str(path)], capture_output=True, text=True, check=False)


class BundledSources(unittest.TestCase):
    def test_a_plain_map_gives_the_text_of_each_source(self) -> None:
        result = run({"version": 3, "sources": ["a.js", "b.js", "c.js"], "sourcesContent": ["const a = 1;", None, "const c = 3;"], "mappings": ""})
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "const a = 1;\nconst c = 3;\n")

    def test_regression_an_index_map_gives_the_text_of_every_section(self) -> None:
        # 2026-11-07, Expo SDK 58 (React Native 0.88): Metro writes the packager map as an index map,
        # one section per module and no top-level sourcesContent, and the scan found nothing to read.
        index = {
            "version": 3,
            "sections": [
                {"offset": {"line": 0, "column": 0}, "map": {"version": 3, "sources": ["a.js"], "sourcesContent": ["const a = 1;"], "mappings": ""}},
                {"offset": {"line": 1, "column": 0}, "map": {"version": 3, "sources": ["b.js"], "mappings": ""}},
                {
                    "offset": {"line": 2, "column": 0},
                    "map": {
                        "version": 3,
                        "sections": [{"offset": {"line": 0, "column": 0}, "map": {"version": 3, "sources": ["c.js"], "sourcesContent": ["const c = 3;"], "mappings": ""}}],
                    },
                },
            ],
        }
        result = run(index)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "const a = 1;\nconst c = 3;\n")

    def test_a_map_without_the_sources_text_stops_the_build(self) -> None:
        for source_map in (
            {"version": 3, "sources": ["a.js"], "mappings": ""},
            {"version": 3, "sources": ["a.js"], "sourcesContent": [None, ""], "mappings": ""},
            {"version": 3, "sections": [{"offset": {"line": 0, "column": 0}, "map": {"version": 3, "sources": ["a.js"], "mappings": ""}}]},
            {"version": 3, "sections": []},
        ):
            result = run(source_map)
            self.assertNotEqual(result.returncode, 0, source_map)
            self.assertIn("no sourcesContent in the packager source map", result.stderr)
            self.assertEqual(result.stdout, "")

    def test_a_section_that_points_at_another_file_is_refused(self) -> None:
        # Metro embeds each section's map; one that names a URL would hide its sources from the scan.
        result = run({"version": 3, "sections": [{"offset": {"line": 0, "column": 0}, "url": "a.js.map"}]})
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("a section of the packager source map has no embedded map", result.stderr)
        self.assertEqual(result.stdout, "")


if __name__ == "__main__":
    unittest.main()
