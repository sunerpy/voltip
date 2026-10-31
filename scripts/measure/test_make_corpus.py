"""Tests for scripts/measure/make-corpus.py (stdlib unittest, no network).

Run: python3 -m unittest discover -s scripts/measure -p 'test_*.py'
"""

from __future__ import annotations

import csv
import importlib.util
import io
import sys
import tarfile
import tempfile
import unittest
import wave
from pathlib import Path

HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("make_corpus", HERE / "make-corpus.py")
assert SPEC is not None and SPEC.loader is not None
mc = importlib.util.module_from_spec(SPEC)
# The dataclasses look their module up while the class is made.
sys.modules[SPEC.name] = mc
SPEC.loader.exec_module(mc)


def sentence(sid: str, seconds: float, text: str = "") -> "mc.Sentence":
    return mc.Sentence(sid, text or f"s{sid}", [1000] * round(seconds * mc.RATE))


def wav_bytes(samples: list[int], rate: int = mc.RATE, channels: int = 1) -> bytes:
    buf = io.BytesIO()
    with wave.open(buf, "wb") as w:
        w.setnchannels(channels)
        w.setsampwidth(2)
        w.setframerate(rate)
        w.writeframes(b"".join(int(s).to_bytes(2, "little", signed=True) for s in samples))
    return buf.getvalue()


class PlanTest(unittest.TestCase):
    def test_shorter_versions_of_a_clip_are_prefixes_and_every_file_has_its_exact_length(self) -> None:
        pool = [sentence(str(i), 9.0) for i in range(40)]
        clips = mc.plan_clips("cmn_hans_cn", pool, [30, 60, 121], 2, 300, "fleurs")
        self.assertEqual([c.name for c in clips[:3]], ["cmn_hans_cn-00-30s", "cmn_hans_cn-00-60s", "cmn_hans_cn-00-121s"])
        first = clips[:3]
        # 9 s sentences with 0.3 s between them: 3 fit in 30 s, 6 in 60 s, 13 in 121 s.
        self.assertEqual([len(c.sentences) for c in first], [3, 6, 13])
        for short, long in zip(first, first[1:]):
            self.assertEqual(long.sentences[: len(short.sentences)], short.sentences)
        for c in clips:
            self.assertEqual(len(c.samples()), round(c.length_s * mc.RATE))
        # The second clip starts where the first one's run ended: no sentence is used twice.
        second_ids = {s.id for s in clips[3].sentences}
        self.assertFalse(second_ids & {s.id for s in clips[2].sentences})
        self.assertEqual(clips[3].sentences[0].id, "13")

    def test_the_gap_is_silence_between_sentences_and_padding_follows_the_last(self) -> None:
        clip = mc.Clip("en_us", "x", 3.0, [sentence("a", 1.0), sentence("b", 1.0)], 500, "fleurs")
        pcm = clip.samples()
        self.assertEqual(len(pcm), 3 * mc.RATE)
        self.assertTrue(all(v == 1000 for v in pcm[: mc.RATE]))
        self.assertTrue(all(v == 0 for v in pcm[mc.RATE : mc.RATE + mc.RATE // 2]))
        self.assertTrue(all(v == 1000 for v in pcm[3 * mc.RATE // 2 : 5 * mc.RATE // 2]))
        self.assertTrue(all(v == 0 for v in pcm[5 * mc.RATE // 2 :]))
        self.assertAlmostEqual(clip.speech_s(), 2.0)

    def test_references_join_chinese_without_spaces_and_english_with_one(self) -> None:
        self.assertEqual(mc.join_texts("cmn_hans_cn", ["你好。", " 世界。"]), "你好。世界。")
        self.assertEqual(mc.join_texts("en_us", ["Hello.", "World. ", ""]), "Hello. World.")

    def test_too_little_speech_is_refused(self) -> None:
        with self.assertRaises(SystemExit):
            mc.plan_clips("en_us", [sentence(str(i), 9.0) for i in range(5)], [121], 1, 300, "fleurs")
        with self.assertRaises(SystemExit):
            mc.plan_clips("en_us", [sentence("long", 40.0)] * 10, [30], 1, 300, "fleurs")

    def test_sentences_shape_keeps_recordings_in_the_range(self) -> None:
        pool = [sentence("a", 2.0), sentence("b", 4.0), sentence("c", 9.0), sentence("d", 7.5)]
        picked = mc.plan_sentences("cmn_hans_cn", pool, 3, 8, 2, "fleurs")
        self.assertEqual([c.sentences[0].id for c in picked], ["b", "d"])
        self.assertEqual([c.name for c in picked], ["cmn_hans_cn-s00", "cmn_hans_cn-s01"])
        with self.assertRaises(SystemExit):
            mc.plan_sentences("cmn_hans_cn", pool, 3, 8, 3, "fleurs")
        self.assertTrue(mc.has_latin("特朗普与 Erdoğan 通话") and not mc.has_latin("特朗普通话"))

    def test_the_fallback_repeats_its_files(self) -> None:
        pool = [sentence("zh", 5.0)]
        self.assertEqual(len(mc.repeated(pool, 12.0)), 3)


class FilesTest(unittest.TestCase):
    def test_wavs_round_trip_and_stereo_is_averaged(self) -> None:
        self.assertEqual(mc.read_wav(wav_bytes([1, -2, 3]), "a"), [1, -2, 3])
        self.assertEqual(mc.read_wav(wav_bytes([100, 300, -10, -30], channels=2), "b"), [200, -20])
        with self.assertRaises(ValueError):
            mc.read_wav(wav_bytes([1], rate=48_000), "c")
        with self.assertRaises(ValueError):
            mc.read_wav(b"not a wav file at all", "d")

    def test_float_wavs_as_fleurs_stores_them_are_read(self) -> None:
        import struct

        floats = [0.0, 0.5, -1.0, 2.0]
        body = struct.pack("<4f", *floats)
        fmt = struct.pack("<HHIIHH", 3, 1, mc.RATE, mc.RATE * 4, 4, 32)
        data = b"RIFF" + struct.pack("<I", 4 + 8 + len(fmt) + 8 + len(body)) + b"WAVE"
        data += b"fmt " + struct.pack("<I", len(fmt)) + fmt + b"data" + struct.pack("<I", len(body)) + body
        self.assertEqual(mc.read_wav(data, "f"), [0, 16384, -32767, 32767])

    def test_the_corpus_and_its_manifest_are_written(self) -> None:
        with tempfile.TemporaryDirectory() as d:
            clips = mc.plan_clips("cmn_hans_cn", [sentence(str(i), 2.0, f"第{i}句。") for i in range(10)], [5], 1, 300, "test_wavs")
            manifest = mc.write_corpus(Path(d), clips)
            with manifest.open(encoding="utf-8") as f:
                rows = list(csv.DictReader(f))
            self.assertEqual(len(rows), 1)
            row = rows[0]
            self.assertEqual((row["lang"], row["name"], row["length_s"], row["source"]), ("cmn_hans_cn", "cmn_hans_cn-00-5s", "5", "test_wavs"))
            self.assertEqual(row["sentences"], "0 1")
            self.assertEqual(Path(row["ref"]).read_text(encoding="utf-8"), "第0句。第1句。\n")
            with wave.open(row["wav"], "rb") as w:
                self.assertEqual((w.getframerate(), w.getnchannels(), w.getsampwidth(), w.getnframes()), (mc.RATE, 1, 2, 5 * mc.RATE))

    def test_fleurs_rows_keep_the_first_recording_of_each_sentence(self) -> None:
        with tempfile.TemporaryDirectory() as d:
            tsv = Path(d) / "test.tsv"
            tsv.write_text("1\ta.wav\t甲。\tx\tx\t1\tMALE\n1\tb.wav\t甲。\tx\tx\t1\tFEMALE\n2\tc.wav\t乙。\tx\tx\t1\tMALE\nbroken\n", encoding="utf-8")
            self.assertEqual(mc.fleurs_rows(tsv), [("1", "a.wav", "甲。"), ("2", "c.wav", "乙。")])

    def test_archive_members_are_read_by_base_name_without_extracting(self) -> None:
        with tempfile.TemporaryDirectory() as d:
            archive = Path(d) / "test.tar.gz"
            with tarfile.open(archive, "w:gz") as tar:
                data = wav_bytes([5] * 10)
                info = tarfile.TarInfo("../../escape/a.wav")
                info.size = len(data)
                tar.addfile(info, io.BytesIO(data))
            self.assertEqual(mc.archive_audio(archive, {"a.wav", "missing.wav"}), {"a.wav": [5] * 10})
            self.assertEqual(mc.archive_audio(archive, {"b.wav"}), {})
            self.assertEqual(sorted(p.name for p in Path(d).iterdir()), ["test.tar.gz"], "nothing extracted")


if __name__ == "__main__":
    unittest.main()
