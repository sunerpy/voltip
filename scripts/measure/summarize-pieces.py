#!/usr/bin/env python3
"""Summary of the piece_quality measurement (docs/dictation.md §29).

Reads the rows `crates/voltip-asr-local/examples/piece_quality.rs` prints (lines starting with
`piece,` and `peak,`; anything else is skipped) from every file given, and writes:

  <out>/summary.csv    one row per machine, model, language and length: clips, errors, the
                       error rate over all clips (100 x edits / reference units: CER for Chinese,
                       WER for English), the mean and the slowest real-time factor, the slowest
                       inference and the peak memory.
  <out>/decisions.csv  each model's max_piece and the transcriber's LOCK_WAIT, by the rules below.

The rules (docs/dictation.md §29):

  max_piece  the largest L of 30, 60, 90 and 119 s at which, on every machine and in every
             language, no clip failed and the error rate is at most the 30 s rate + 1 point;
             30 s when none is.
  LOCK_WAIT  twice the slowest inference of a 121 s clip on any machine, rounded up to 5 s,
             at least 30 s.

  /usr/bin/python3 -I scripts/measure/summarize-pieces.py --out /tmp/voltip-measure/results rows...
"""

from __future__ import annotations

import argparse
import csv
import math
import sys
from collections import defaultdict
from dataclasses import dataclass
from pathlib import Path

CANDIDATES = (30, 60, 90, 119)
BASE = 30
TIMING = 121
TOLERANCE = 1.0
MIN_LOCK_WAIT_S = 30
PIECE_FIELDS = (
    "machine model backend lang name length_s speech_s load_ms infer_ms rtf metric edits ref_units rate status"
).split()


@dataclass(frozen=True)
class Piece:
    machine: str
    model: str
    backend: str
    lang: str
    name: str
    length_s: int
    infer_ms: int
    rtf: float
    metric: str
    edits: int
    ref_units: int
    status: str


def parse(lines) -> tuple[list[Piece], dict[tuple[str, str, int], float]]:
    """The clips and the peak memory (MB) by machine, model and length."""
    pieces: list[Piece] = []
    peaks: dict[tuple[str, str, int], float] = {}
    for line in lines:
        cells = line.rstrip("\r\n").split(",")
        if cells[0] == "piece" and len(cells) == len(PIECE_FIELDS) + 1 and cells[1] != "machine":
            row = dict(zip(PIECE_FIELDS, cells[1:]))
            pieces.append(
                Piece(
                    row["machine"],
                    row["model"],
                    row["backend"],
                    row["lang"],
                    row["name"],
                    int(row["length_s"]),
                    int(row["infer_ms"]),
                    float(row["rtf"]),
                    row["metric"],
                    int(row["edits"]),
                    int(row["ref_units"]),
                    row["status"],
                )
            )
        elif cells[0] == "peak" and len(cells) == 5 and cells[1] != "machine" and cells[4]:
            peaks[(cells[1], cells[2], int(cells[3]))] = float(cells[4])
    return pieces, peaks


@dataclass(frozen=True)
class Group:
    clips: int
    errors: int
    metric: str
    rate: float
    mean_rtf: float
    max_rtf: float
    max_infer_ms: int


def groups(pieces: list[Piece]) -> dict[tuple[str, str, str, int], Group]:
    """The pieces by machine, model, language and length."""
    by: dict[tuple[str, str, str, int], list[Piece]] = defaultdict(list)
    for p in pieces:
        by[(p.machine, p.model, p.lang, p.length_s)].append(p)
    out = {}
    for key, ps in sorted(by.items()):
        ok = [p for p in ps if p.status == "ok"]
        units = sum(p.ref_units for p in ok)
        out[key] = Group(
            clips=len(ps),
            errors=len(ps) - len(ok),
            metric=ps[0].metric,
            rate=100.0 * sum(p.edits for p in ok) / units if units else 0.0,
            mean_rtf=sum(p.rtf for p in ps) / len(ps),
            max_rtf=max(p.rtf for p in ps),
            max_infer_ms=max(p.infer_ms for p in ps),
        )
    return out


def max_piece(model: str, by: dict[tuple[str, str, str, int], Group]) -> tuple[int, list[str]]:
    """`model`'s max_piece and the reasons each other candidate was not taken."""
    keys = [k for k in by if k[1] == model]
    combos = sorted({(k[0], k[2]) for k in keys})
    notes = []
    best = BASE
    for length in CANDIDATES:
        failed = []
        for machine, lang in combos:
            group = by.get((machine, model, lang, length))
            base = by.get((machine, model, lang, BASE))
            if group is None or base is None:
                failed.append(f"{machine}/{lang}: not measured")
            elif group.errors:
                failed.append(f"{machine}/{lang}: {group.errors} failed")
            elif base.errors:
                failed.append(f"{machine}/{lang}: the 30 s clips failed")
            elif group.rate > base.rate + TOLERANCE:
                failed.append(f"{machine}/{lang}: {group.metric} {group.rate:.2f} > {base.rate:.2f} + {TOLERANCE:g}")
        if failed:
            notes.append(f"{length} s: " + "; ".join(failed))
        else:
            best = length
    return best, notes


def lock_wait(by: dict[tuple[str, str, str, int], Group]) -> tuple[int, int, str]:
    """LOCK_WAIT in seconds, the slowest 121 s inference (ms) and where it was."""
    slowest = max(((g.max_infer_ms, f"{k[0]}/{k[1]}/{k[2]}") for k, g in by.items() if k[3] == TIMING), default=(0, "none"))
    seconds = max(MIN_LOCK_WAIT_S, 5 * math.ceil(2 * slowest[0] / 1000 / 5))
    return seconds, slowest[0], slowest[1]


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("rows", type=Path, nargs="+")
    args = parser.parse_args(argv)
    lines = []
    for path in args.rows:
        lines.extend(path.read_text(encoding="utf-8").splitlines())
    pieces, peaks = parse(lines)
    if not pieces:
        print("summarize-pieces: no piece rows", file=sys.stderr)
        return 1
    by = groups(pieces)
    args.out.mkdir(parents=True, exist_ok=True)
    with (args.out / "summary.csv").open("w", newline="", encoding="utf-8") as f:
        w = csv.writer(f)
        w.writerow(["machine", "model", "lang", "length_s", "clips", "errors", "metric", "rate", "mean_rtf", "max_rtf", "max_infer_ms", "peak_rss_mb"])
        for (machine, model, lang, length), g in by.items():
            peak = peaks.get((machine, model, length))
            w.writerow([machine, model, lang, length, g.clips, g.errors, g.metric, f"{g.rate:.2f}", f"{g.mean_rtf:.4f}", f"{g.max_rtf:.4f}", g.max_infer_ms, "" if peak is None else f"{peak:.0f}"])
    with (args.out / "decisions.csv").open("w", newline="", encoding="utf-8") as f:
        w = csv.writer(f)
        w.writerow(["decision", "subject", "value_s", "notes"])
        for model in sorted({p.model for p in pieces}):
            value, notes = max_piece(model, by)
            w.writerow(["max_piece", model, value, " | ".join(notes)])
        seconds, slowest_ms, where = lock_wait(by)
        w.writerow(["lock_wait", "all", seconds, f"slowest {TIMING} s inference {slowest_ms} ms ({where})"])
    print((args.out / "decisions.csv").read_text(encoding="utf-8"), end="")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
