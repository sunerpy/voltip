"""Tests for scripts/measure/summarize-pieces.py (stdlib unittest).

Run: python3 -m unittest discover -s scripts/measure -p 'test_*.py'
"""

from __future__ import annotations

import contextlib
import csv
import importlib.util
import io
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("summarize_pieces", HERE / "summarize-pieces.py")
assert SPEC is not None and SPEC.loader is not None
sp = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = sp
SPEC.loader.exec_module(sp)


def row(length: int, edits: int, *, lang: str = "cmn_hans_cn", machine: str = "linux", model: str = "m", infer_ms: int = 1000, status: str = "ok", units: int = 100) -> str:
    metric = "cer" if lang.startswith("cmn") else "wer"
    rate = 100 * edits / units
    return f"piece,{machine},{model},CPU,{lang},{lang}-00-{length}s,{length},{length - 5}.00,0,{infer_ms},{infer_ms / length / 1000:.4f},{metric},{edits},{units},{rate:.2f},{status}"


def lengths(rates: dict[int, int], **kw) -> list[str]:
    return [row(length, edits, **kw) for length, edits in rates.items()]


class MaxPiece(unittest.TestCase):
    def decide(self, lines: list[str], model: str = "m") -> tuple[int, list[str]]:
        pieces, _ = sp.parse(lines)
        return sp.max_piece(model, sp.groups(pieces))

    def test_the_longest_length_within_a_point_of_30_s(self) -> None:
        value, notes = self.decide(lengths({30: 5, 60: 5, 90: 6, 119: 7, 121: 9}))
        self.assertEqual(value, 90)
        self.assertEqual(len(notes), 1)
        self.assertIn("119 s", notes[0])
        self.assertIn("7.00 > 5.00 + 1", notes[0])

    def test_every_language_and_machine_has_to_pass(self) -> None:
        zh = lengths({30: 5, 60: 5, 90: 5, 119: 5})
        en = lengths({30: 5, 60: 5, 90: 5, 119: 9}, lang="en_us")
        self.assertEqual(self.decide(zh + en)[0], 90)
        windows = lengths({30: 5, 60: 8, 90: 5, 119: 5}, machine="windows")
        value, notes = self.decide(zh + windows)
        self.assertEqual(value, 119, "the largest that passes, even when a shorter one did not")
        self.assertTrue(any(n.startswith("60 s") and "windows/cmn_hans_cn" in n for n in notes))

    def test_a_failed_clip_rules_the_length_out_and_none_passing_means_30(self) -> None:
        lines = lengths({30: 5, 60: 5, 90: 5}) + [row(119, 0, status="error:input_too_long")]
        value, notes = self.decide(lines)
        self.assertEqual(value, 90)
        self.assertIn("1 failed", notes[-1])
        self.assertEqual(self.decide(lengths({30: 5, 60: 9, 90: 9, 119: 9}))[0], 30)
        self.assertEqual(self.decide([row(30, 0, status="error:other")] + lengths({60: 0}))[0], 30)

    def test_rates_are_over_all_clips_of_a_length(self) -> None:
        lines = [row(30, 2, units=100), row(30, 10, units=300), row(60, 0, units=50)]
        by = sp.groups(sp.parse(lines)[0])
        self.assertAlmostEqual(by[("linux", "m", "cmn_hans_cn", 30)].rate, 3.0)
        self.assertEqual(by[("linux", "m", "cmn_hans_cn", 30)].clips, 2)


class LockWait(unittest.TestCase):
    def test_twice_the_slowest_121_s_inference_rounded_up_to_5_s(self) -> None:
        lines = [row(121, 0, infer_ms=12_300), row(121, 0, infer_ms=20_100, machine="windows"), row(119, 0, infer_ms=90_000)]
        self.assertEqual(sp.lock_wait(sp.groups(sp.parse(lines)[0])), (45, 20_100, "windows/m/cmn_hans_cn"))

    def test_never_below_30_s(self) -> None:
        self.assertEqual(sp.lock_wait(sp.groups(sp.parse([row(121, 0, infer_ms=2_000)])[0]))[0], 30)
        self.assertEqual(sp.lock_wait({})[0], 30)


class Files(unittest.TestCase):
    def test_writes_the_summary_and_the_decisions(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            rows = Path(tmp) / "linux-m.rows"
            text = [
                "piece,machine,model,backend,lang,name,length_s,speech_s,load_ms,infer_ms,rtf,metric,edits,ref_units,rate,status",
                "peak,machine,model,length_s,peak_rss_mb",
                "   Compiling voltip-asr-local",
                *lengths({30: 5, 60: 5, 90: 5, 119: 5, 121: 5}),
                "peak,linux,m,121,812.5",
            ]
            rows.write_text("\n".join(text) + "\n", encoding="utf-8")
            out = Path(tmp) / "out"
            printed = io.StringIO()
            with contextlib.redirect_stdout(printed):
                self.assertEqual(sp.main(["--out", str(out), str(rows)]), 0)
            self.assertIn("max_piece,m,119", printed.getvalue())
            with (out / "summary.csv").open(encoding="utf-8") as f:
                summary = list(csv.DictReader(f))
            self.assertEqual(len(summary), 5)
            self.assertEqual(summary[-1]["peak_rss_mb"], "812")
            with (out / "decisions.csv").open(encoding="utf-8") as f:
                decisions = {r["decision"]: r for r in csv.DictReader(f)}
            self.assertEqual(decisions["max_piece"]["value_s"], "119")
            self.assertEqual(decisions["lock_wait"]["value_s"], "30")
            empty = Path(tmp) / "empty.rows"
            empty.write_text("nothing\n", encoding="utf-8")
            with contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(sp.main(["--out", str(out), str(empty)]), 1)


if __name__ == "__main__":
    unittest.main()
