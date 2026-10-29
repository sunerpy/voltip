#!/usr/bin/env python3
"""Speech corpus for measuring the local recognisers (docs/dictation.md §29).

Builds 16 kHz mono 16-bit WAV files with their reference texts from the FLEURS test sets
(google/fleurs on Hugging Face, CC BY 4.0), pinned to one dataset revision whose archives are
checked against the SHA-256 the Hub reports. Nothing it writes goes into the repository: the
audio and the cache live under /tmp/voltip-measure by default.

Two shapes:

  clips      --lengths 30,60,90,119,121 --count 10
             `count` clips per language. Clip i is a run of distinct sentences joined with
             `--gap-ms` of silence; its version of length L holds the longest prefix of that run
             that fits in L seconds, padded with silence to exactly L. The shorter versions of a
             clip are therefore prefixes of the longer ones: a recogniser's error rate at 30 s and
             at 119 s is measured on the same speech.
  sentences  --sentence-range 3,8 --count 20
             `count` single recordings between 3 and 8 seconds per language, as they are.

Either shape writes <out>/<lang>/<name>.wav, <name>.txt (the reference) and one
<out>/manifest.csv for all of them (lang, name, length_s, speech_s, sentences, wav, ref, source).
`--require-latin` keeps only sentences with Latin letters in them (Chinese with English words,
the code-switched speech FLEURS has).

When the download fails and `--fallback-dir` names directories of WAV files (the `test_wavs` of
the sherpa-onnx model archives), the clips are made by repeating those files instead, and the
manifest's `source` column says `test_wavs` so every result can say so. Their reference is the
`<file>.txt` beside each WAV when there is one, otherwise empty (no error rate).

Run with the system Python, isolated, from anywhere:

  /usr/bin/python3 -I scripts/measure/make-corpus.py --lengths 30,60,90,119,121 --count 10
  /usr/bin/python3 -I scripts/measure/make-corpus.py --sentence-range 3,8 --count 20 --langs cmn_hans_cn
  /usr/bin/python3 -I scripts/measure/make-corpus.py --lengths 60 --count 1 --langs cmn_hans_cn

Downloads go through the proxy in https_proxy / http_proxy when set (urllib reads them);
`--base-url https://hf-mirror.com` (or HF_ENDPOINT) uses the mirror.
"""

from __future__ import annotations

import argparse
import csv
import hashlib
import json
import os
import random
import struct
import sys
import tarfile
import urllib.error
import urllib.request
import wave
from dataclasses import dataclass, field
from pathlib import Path

RATE = 16_000
FLEURS_REPO = "google/fleurs"
# The dataset revision every corpus is built from (2026-05-15 on the Hub).
FLEURS_REVISION = "70bb2e84b976b7e960aa89f1c648e09c59f894dd"
DEFAULT_LANGS = "cmn_hans_cn,en_us"
DEFAULT_OUT = "/tmp/voltip-measure/corpus"
DEFAULT_CACHE = "/tmp/voltip-measure/fleurs"
CHUNK = 1 << 20


@dataclass
class Sentence:
    """One recording: its id in the dataset, the text read, and the 16 kHz samples."""

    id: str
    text: str
    samples: list[int] = field(default_factory=list)

    @property
    def seconds(self) -> float:
        return len(self.samples) / RATE


@dataclass
class Clip:
    """What one output file holds."""

    lang: str
    name: str
    length_s: float
    sentences: list[Sentence]
    gap_ms: int
    source: str

    def samples(self) -> list[int]:
        gap = [0] * (RATE * self.gap_ms // 1000)
        out: list[int] = []
        for i, s in enumerate(self.sentences):
            if i:
                out.extend(gap)
            out.extend(s.samples)
        total = round(self.length_s * RATE)
        if len(out) < total:
            out.extend([0] * (total - len(out)))
        return out[:total]

    def reference(self) -> str:
        return join_texts(self.lang, [s.text for s in self.sentences])

    def speech_s(self) -> float:
        return sum(s.seconds for s in self.sentences)


def join_texts(lang: str, texts: list[str]) -> str:
    """Sentences as one reference: no space between Chinese sentences, one between others."""
    sep = "" if lang.startswith(("cmn", "yue", "ja")) else " "
    return sep.join(t.strip() for t in texts if t.strip())


def fits(sentences: list[Sentence], gap_ms: int, length_s: float) -> int:
    """How many of `sentences`, from the first, fit in `length_s` with `gap_ms` between them."""
    used = 0.0
    n = 0
    for s in sentences:
        need = s.seconds + (gap_ms / 1000 if n else 0.0)
        if used + need > length_s:
            break
        used += need
        n += 1
    return n


def plan_clips(lang: str, pool: list[Sentence], lengths: list[float], count: int, gap_ms: int, source: str) -> list[Clip]:
    """`count` runs of distinct sentences from `pool`, each long enough for the longest length,
    and every run's version at every length (the shorter ones are prefixes)."""
    longest = max(lengths)
    clips: list[Clip] = []
    at = 0
    for i in range(count):
        run: list[Sentence] = []
        # Take sentences until the run fills the longest length (one more is checked by `fits`).
        while at < len(pool) and fits(run, gap_ms, longest) == len(run):
            run.append(pool[at])
            at += 1
        if fits(run, gap_ms, longest) == len(run):
            raise SystemExit(f"make-corpus: {lang}: not enough speech for {count} clips of {longest:g} s ({i} made)")
        # The sentence that did not fit goes back to the pool for the next clip.
        at -= 1
        run.pop()
        for length in lengths:
            n = fits(run, gap_ms, length)
            if n == 0:
                raise SystemExit(f"make-corpus: {lang}: no sentence fits in {length:g} s")
            clips.append(Clip(lang, f"{lang}-{i:02d}-{length:g}s", length, run[:n], gap_ms, source))
    return clips


def plan_sentences(lang: str, pool: list[Sentence], low: float, high: float, count: int, source: str) -> list[Clip]:
    """`count` single recordings of `low`..`high` seconds."""
    picked = [s for s in pool if low <= s.seconds <= high][:count]
    if len(picked) < count:
        raise SystemExit(f"make-corpus: {lang}: only {len(picked)} recordings of {low:g}–{high:g} s, {count} wanted")
    return [Clip(lang, f"{lang}-s{i:02d}", round(s.seconds, 3), [s], 0, source) for i, s in enumerate(picked)]


def has_latin(text: str) -> bool:
    return any("a" <= c.lower() <= "z" for c in text)


def write_wav(path: Path, samples: list[int]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with wave.open(str(path), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(RATE)
        w.writeframes(struct.pack(f"<{len(samples)}h", *samples))


WAVE_PCM = 1
WAVE_FLOAT = 3
WAVE_EXTENSIBLE = 0xFFFE


def read_wav(data: bytes, name: str) -> list[int]:
    """16-bit samples of a 16 kHz WAV: 16-bit PCM, or 32-bit float as FLEURS stores it (the
    `wave` module reads PCM only); several channels are averaged."""
    if data[:4] != b"RIFF" or data[8:12] != b"WAVE":
        raise ValueError(f"{name}: not a WAV file")
    fmt = None
    raw = b""
    at = 12
    while at + 8 <= len(data):
        tag, size = data[at : at + 4], struct.unpack("<I", data[at + 4 : at + 8])[0]
        body = data[at + 8 : at + 8 + size]
        if tag == b"fmt ":
            fmt = struct.unpack("<HHIIHH", body[:16])
            if fmt[0] == WAVE_EXTENSIBLE and len(body) >= 26:
                # The sub-format GUID starts with the real format tag.
                fmt = (struct.unpack("<H", body[24:26])[0],) + fmt[1:]
        elif tag == b"data":
            raw = body
        at += 8 + size + (size & 1)
    if fmt is None:
        raise ValueError(f"{name}: no fmt chunk")
    format_tag, channels, rate, _, _, bits = fmt
    if rate != RATE:
        raise ValueError(f"{name}: {rate} Hz; the corpus needs 16 kHz")
    if format_tag == WAVE_PCM and bits == 16:
        values = list(struct.unpack(f"<{len(raw) // 2}h", raw[: len(raw) // 2 * 2]))
    elif format_tag == WAVE_FLOAT and bits == 32:
        floats = struct.unpack(f"<{len(raw) // 4}f", raw[: len(raw) // 4 * 4])
        values = [max(-32768, min(32767, round(v * 32767))) for v in floats]
    else:
        raise ValueError(f"{name}: format {format_tag}, {bits}-bit; the corpus reads 16-bit PCM and 32-bit float")
    if channels <= 1:
        return values
    return [sum(values[i : i + channels]) // channels for i in range(0, len(values) - channels + 1, channels)]


def write_corpus(out: Path, clips: list[Clip]) -> Path:
    rows = []
    for clip in clips:
        wav_path = out / clip.lang / f"{clip.name}.wav"
        ref_path = out / clip.lang / f"{clip.name}.txt"
        write_wav(wav_path, clip.samples())
        ref_path.write_text(clip.reference() + "\n", encoding="utf-8")
        rows.append(
            {
                "lang": clip.lang,
                "name": clip.name,
                "length_s": f"{clip.length_s:g}",
                "speech_s": f"{clip.speech_s():.2f}",
                "sentences": " ".join(s.id for s in clip.sentences),
                "wav": str(wav_path),
                "ref": str(ref_path),
                "source": clip.source,
            }
        )
    manifest = out / "manifest.csv"
    with manifest.open("w", newline="", encoding="utf-8") as f:
        writer = csv.DictWriter(f, fieldnames=list(rows[0]) if rows else ["lang"])
        writer.writeheader()
        writer.writerows(rows)
    return manifest


# --- FLEURS ------------------------------------------------------------------------------------


def hub_url(base: str, path: str) -> str:
    return f"{base.rstrip('/')}/datasets/{FLEURS_REPO}/resolve/{FLEURS_REVISION}/{path}"


def expected_sha256(base: str, lang: str, split: str) -> tuple[str, int]:
    """The archive's SHA-256 and size as the Hub lists them at the pinned revision."""
    url = f"{base.rstrip('/')}/api/datasets/{FLEURS_REPO}/tree/{FLEURS_REVISION}/data/{lang}/audio"
    with urllib.request.urlopen(url, timeout=60) as r:
        entries = json.load(r)
    for e in entries:
        if e.get("path") == f"data/{lang}/audio/{split}.tar.gz" and "lfs" in e:
            return e["lfs"]["oid"], int(e["lfs"]["size"])
    raise RuntimeError(f"{url}: no {split}.tar.gz with an LFS digest")


def download(url: str, dest: Path, sha256: str | None = None, size: int | None = None) -> Path:
    """`url` to `dest` (resuming a `.part` left by an earlier run); checked against `sha256`."""
    if dest.exists() and (sha256 is None or file_sha256(dest) == sha256):
        return dest
    dest.parent.mkdir(parents=True, exist_ok=True)
    part = dest.with_name(dest.name + ".part")
    have = part.stat().st_size if part.exists() else 0
    request = urllib.request.Request(url, headers={"Range": f"bytes={have}-"} if have else {})
    try:
        response = urllib.request.urlopen(request, timeout=60)
    except urllib.error.HTTPError as e:
        if e.code == 416 and size is not None and have == size:
            response = None
        else:
            raise
    if response is not None:
        with response, part.open("ab" if response.status == 206 else "wb") as f:
            done = have if response.status == 206 else 0
            shown = -1
            while chunk := response.read(CHUNK):
                f.write(chunk)
                done += len(chunk)
                if size and done * 10 // size != shown:
                    shown = done * 10 // size
                    print(f"make-corpus: {dest.parent.name}/{dest.name} {shown * 10}%", file=sys.stderr, flush=True)
    if sha256 is not None and (got := file_sha256(part)) != sha256:
        part.unlink()
        raise RuntimeError(f"{url}: SHA-256 {got}, expected {sha256}")
    part.rename(dest)
    return dest


def file_sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        while chunk := f.read(CHUNK):
            h.update(chunk)
    return h.hexdigest()


def fleurs_rows(tsv: Path) -> list[tuple[str, str, str]]:
    """(id, file name, raw transcription) for the first recording of every sentence, in file order."""
    seen: set[str] = set()
    rows = []
    with tsv.open(encoding="utf-8") as f:
        for line in f:
            cols = line.rstrip("\n").split("\t")
            if len(cols) < 3 or cols[0] in seen:
                continue
            seen.add(cols[0])
            rows.append((cols[0], cols[1], cols[2]))
    return rows


def fleurs_pool(base: str, cache: Path, lang: str, split: str, seed: int) -> list[Sentence]:
    """Every sentence of the split once, shuffled with `seed`, with its audio."""
    tsv = download(hub_url(base, f"data/{lang}/{split}.tsv"), cache / lang / f"{split}.tsv")
    sha256, size = expected_sha256(base, lang, split)
    archive = download(hub_url(base, f"data/{lang}/audio/{split}.tar.gz"), cache / lang / f"{split}.tar.gz", sha256, size)
    rows = fleurs_rows(tsv)
    random.Random(seed).shuffle(rows)
    audio = archive_audio(archive, {name for _, name, _ in rows})
    return [Sentence(sid, text, audio[name]) for sid, name, text in rows if name in audio]


def archive_audio(archive: Path, wanted: set[str]) -> dict[str, list[int]]:
    """The samples of the archive's WAV members whose base name is in `wanted`. Members are read
    into memory and never extracted: no path from the archive reaches the file system."""
    audio: dict[str, list[int]] = {}
    with tarfile.open(archive, "r:gz") as tar:
        for member in tar:
            name = os.path.basename(member.name)
            if not member.isfile() or name not in wanted:
                continue
            data = tar.extractfile(member)
            if data is not None:
                audio[name] = read_wav(data.read(), name)
    return audio


# --- the sherpa-onnx test_wavs fallback ----------------------------------------------------------


def fallback_pool(dirs: list[Path], lang: str) -> list[Sentence]:
    """The WAV files in `dirs` (sorted), repeated by the planner as often as the clips need. A
    file named after the language's two-letter code (`zh.wav`, `en.wav`) is preferred."""
    files = sorted(p for d in dirs for p in d.glob("*.wav"))
    short = {"cmn_hans_cn": "zh", "en_us": "en"}.get(lang, lang[:2])
    preferred = [p for p in files if p.stem.startswith(short)]
    files = preferred or files
    if not files:
        raise SystemExit(f"make-corpus: no WAV files in {', '.join(map(str, dirs))}")
    pool = []
    for p in files:
        ref = p.with_suffix(".txt")
        text = ref.read_text(encoding="utf-8").strip() if ref.exists() else ""
        pool.append(Sentence(p.stem, text, read_wav(p.read_bytes(), str(p))))
    return pool


def repeated(pool: list[Sentence], seconds: float) -> list[Sentence]:
    """`pool` over and over until it holds `seconds` of audio."""
    out: list[Sentence] = []
    total = 0.0
    while total < seconds:
        for s in pool:
            out.append(s)
            total += s.seconds
    return out


# --- command line ------------------------------------------------------------------------------


def parse_args(argv: list[str]) -> argparse.Namespace:
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0], formatter_class=argparse.RawDescriptionHelpFormatter)
    shape = p.add_mutually_exclusive_group(required=True)
    shape.add_argument("--lengths", help="clip lengths in seconds, comma-separated (clips shape)")
    shape.add_argument("--sentence-range", help="MIN,MAX seconds of single recordings (sentences shape)")
    p.add_argument("--count", type=int, default=10, help="clips (or sentences) per language")
    p.add_argument("--langs", default=DEFAULT_LANGS, help=f"FLEURS language codes (default {DEFAULT_LANGS})")
    p.add_argument("--gap-ms", type=int, default=300, help="silence between sentences in a clip")
    p.add_argument("--split", default="test", help="FLEURS split (default test)")
    p.add_argument("--seed", type=int, default=29, help="shuffle seed: which sentences go first")
    p.add_argument("--require-latin", action="store_true", help="only sentences with Latin letters (code-switched speech)")
    p.add_argument("--out", default=DEFAULT_OUT, help=f"output directory (default {DEFAULT_OUT})")
    p.add_argument("--cache", default=DEFAULT_CACHE, help=f"download cache (default {DEFAULT_CACHE})")
    p.add_argument("--base-url", default=os.environ.get("HF_ENDPOINT", "https://huggingface.co"), help="Hugging Face or a mirror")
    p.add_argument("--fallback-dir", action="append", default=[], help="WAV directory used when FLEURS cannot be downloaded")
    return p.parse_args(argv)


def main(argv: list[str]) -> int:
    args = parse_args(argv)
    out, cache = Path(args.out), Path(args.cache)
    clips: list[Clip] = []
    for lang in [l.strip() for l in args.langs.split(",") if l.strip()]:
        source = "fleurs"
        try:
            pool = fleurs_pool(args.base_url, cache, lang, args.split, args.seed)
        except (OSError, RuntimeError, tarfile.TarError, ValueError) as e:
            if not args.fallback_dir:
                raise SystemExit(f"make-corpus: FLEURS {lang} could not be fetched ({e}); pass --fallback-dir to use test_wavs")
            print(f"make-corpus: FLEURS {lang} could not be fetched ({e}); using the test_wavs in {args.fallback_dir}", file=sys.stderr)
            pool, source = fallback_pool([Path(d) for d in args.fallback_dir], lang), "test_wavs"
        if args.require_latin:
            pool = [s for s in pool if has_latin(s.text)]
        if args.lengths:
            lengths = [float(x) for x in args.lengths.split(",")]
            if source == "test_wavs":
                pool = repeated(pool, (max(lengths) + 30) * args.count)
            clips += plan_clips(lang, pool, lengths, args.count, args.gap_ms, source)
        else:
            low, high = (float(x) for x in args.sentence_range.split(","))
            clips += plan_sentences(lang, pool, low, high, args.count, source)
    manifest = write_corpus(out, clips)
    print(f"make-corpus: {len(clips)} files, manifest {manifest}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
