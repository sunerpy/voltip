"""Tests for scripts/measure/tail-loss.py's sentence preparation (stdlib unittest).

Run: python3 -m unittest discover -s scripts/measure -p 'test_*.py'
"""

from __future__ import annotations

import csv
import importlib.util
import math
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("tail_loss", HERE / "tail-loss.py")
assert SPEC is not None and SPEC.loader is not None
tl = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = tl
SPEC.loader.exec_module(tl)

RATE = 16_000


def tone(seconds: float, dbfs: float) -> list[int]:
    amplitude = 32767 * 10 ** (dbfs / 20) * math.sqrt(2)
    return [int(amplitude * math.sin(2 * math.pi * 440 * i / RATE)) for i in range(int(seconds * RATE))]


def noise(seconds: float, dbfs: float) -> list[int]:
    # A deterministic hiss: alternating signs at the RMS asked for.
    level = int(32767 * 10 ** (dbfs / 20))
    return [level if i % 2 else -level for i in range(int(seconds * RATE))]


class PrepareTest(unittest.TestCase):
    def prepared(self, samples: list[int]) -> dict:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            wav, ref = root / "s00.wav", root / "s00.txt"
            tl.write_wav(wav, samples, RATE)
            ref.write_text("你好", encoding="utf-8")
            manifest = root / "manifest.csv"
            manifest.write_text(f"lang,name,wav,ref\ncmn,s00,{wav},{ref}\n", encoding="utf-8")
            tl.prepare(manifest, root / "out")
            with open(root / "out" / "sentences.tsv", newline="", encoding="utf-8") as f:
                rows = list(csv.DictReader(f, delimiter="\t"))
            self.assertEqual(len(rows), 1)
            row = rows[0]
            row["cut"] = tl.read_wav(Path(row["wav"]))[0]
            return row

    def test_the_sentence_starts_50_ms_before_its_speech_and_ends_where_the_speech_does(self) -> None:
        # 1 s of floor, 1.5 s of speech, 1 s of floor.
        row = self.prepared(noise(1.0, -82) + tone(1.5, -30) + noise(1.0, -82))
        self.assertEqual(row["ref"], "你好")
        self.assertEqual(int(row["speech_end_ms"]), 1550)
        self.assertEqual(int(row["length_ms"]), 2550)
        self.assertEqual(len(row["cut"]), int(2.55 * RATE))

    def test_quiet_word_ends_count_against_the_recording_s_own_floor(self) -> None:
        # The last word fades to -65 dBFS, still 20 dB above this recording's floor of -90 dBFS:
        # an absolute -40 dBFS threshold would end the speech 0.4 s early.
        row = self.prepared(noise(1.0, -90) + tone(1.0, -30) + tone(0.4, -65) + noise(1.0, -90))
        self.assertEqual(int(row["speech_end_ms"]), 1450)


if __name__ == "__main__":
    unittest.main()
