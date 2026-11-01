"""Tests for scripts/measure/edge-loss.py (stdlib unittest).

Run: python3 -m unittest discover -s scripts/measure -p 'test_*.py'
"""

from __future__ import annotations

import contextlib
import importlib.util
import io
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("edge_loss", HERE / "edge-loss.py")
assert SPEC is not None and SPEC.loader is not None
el = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = el
SPEC.loader.exec_module(el)


class EdgeLossTest(unittest.TestCase):
    def test_units_drop_punctuation_tags_and_case(self) -> None:
        self.assertEqual(el.units("<|zh|>你好，World 3！"), list("你好world3"))

    def test_a_deleted_first_or_last_character_is_lost(self) -> None:
        self.assertEqual(el.score("今天天气很好", "今天天气很好"), (0, 6, False, False))
        self.assertEqual(el.score("今天天气很好", "天天气很好"), (1, 6, True, False))
        self.assertEqual(el.score("今天天气很好", "今天天气很"), (1, 6, False, True))
        self.assertEqual(el.score("今天天气很好", "天天气很"), (2, 6, True, True))

    def test_a_substitution_at_an_end_is_not_lost_audio(self) -> None:
        # 好 heard as 号: a recognition error, the audio was there.
        self.assertEqual(el.score("今天天气很好", "今天天气很号"), (1, 6, False, False))
        # An extra word after the end is an insertion, not a loss.
        self.assertEqual(el.score("你好", "你好啊"), (1, 2, False, False))

    def test_nothing_recognised_loses_both_ends(self) -> None:
        self.assertEqual(el.score("你好", ""), (2, 2, True, True))
        self.assertEqual(el.score("", "多余"), (2, 0, False, False))

    def test_a_character_the_whole_recording_does_not_yield_is_not_lost(self) -> None:
        # FLEURS glosses 「(Luna)」, never spoken: the baseline loses it as well.
        ref = "其他选手也悼念露娜 (Luna) 。"
        self.assertEqual(el.counted(ref, "其他选手也悼念露娜", "其他选手也悼念露娜。"), (4, 13, False, False))
        self.assertEqual(el.counted("你好世界", "你好世", "你好世界"), (1, 4, False, True))
        self.assertEqual(el.counted("你好世界", "你好世", None), (1, 4, False, True))

    def test_pairs_reads_the_manifest_and_counts_per_group(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "a.txt").write_text("你好世界", encoding="utf-8")
            (root / "hyp").mkdir()
            (root / "hyp" / "x-s00.txt").write_text("你好世", encoding="utf-8")
            manifest = root / "manifest.csv"
            manifest.write_text(f"lang,name,wav,ref\ncmn,s00,{root / 'a.wav'},{root / 'a.txt'}\n", encoding="utf-8")
            out, err = io.StringIO(), io.StringIO()
            with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
                sys.argv = ["edge-loss.py", "pairs", str(manifest), str(root / "hyp"), "--prefix", "x-", "--group", "off"]
                self.assertEqual(el.main(), 0)
            self.assertIn("off,s00,1,4,25.00,0,1", out.getvalue())
            self.assertIn("off: 1 takes, CER 25.00% (1/4), first lost 0 (0.0%), last lost 1 (100.0%)", err.getvalue())


    def test_takes_read_an_empty_baseline_as_none_and_fill_it_from_files(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "base").mkdir()
            (root / "base" / "p-s01.txt").write_text("你好世界", encoding="utf-8")
            tsv = root / "takes.tsv"
            tsv.write_text("group\tname\trun\tref\thyp\tbase\n" "g\ts00\t1\t你好世界\t你好世\t\n" "g\ts01\t1\t你好世界\t你好世\t\n", encoding="utf-8")
            out, err = io.StringIO(), io.StringIO()
            with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
                sys.argv = ["edge-loss.py", "takes", str(tsv), "--baseline-dir", str(root / "base"), "--prefix", "p-"]
                self.assertEqual(el.main(), 0)
            # Both lose the last character; the baseline keeps it for s01 and is missing for s00.
            self.assertIn("g,s00,1,4,25.00,0,1", out.getvalue())
            self.assertIn("g,s01,1,4,25.00,0,1", out.getvalue())


if __name__ == "__main__":
    unittest.main()
