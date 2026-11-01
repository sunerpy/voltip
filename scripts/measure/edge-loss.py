#!/usr/bin/env python3
"""First and last characters lost, and the character error rate (docs/dictation.md §30.4).

Each hypothesis is aligned with its reference (Levenshtein over the characters, letters and
digits of the simplified text, lower case, without punctuation); a reference character the
alignment deletes is lost. A take loses its first (last) character when the alignment deletes
the reference's first (last) one, which a substitution there does not count as: that is a
recognition error, not missing audio.

  edge-loss.py pairs <manifest.csv> <hyp-dir> [--prefix P] [--group NAME] [--baseline-dir D]
      one row per sentence of the manifest whose hypothesis is <hyp-dir>/<P><name>.txt
  edge-loss.py takes <takes.tsv> [--baseline-dir D --prefix P]
      one row per take of tail-loss.py (its hypothesis inline; its baseline in `base`, or from
      <D>/<P><name>.txt where that column is empty)

A first or last character counts only where the baseline (the same model on the whole recording,
`--baseline-dir <dir>/<P><name>.txt` or the takes' `base` column) keeps it: FLEURS writes glosses
the speakers do not read (「露娜 (Luna)」), and a character no complete recording yields is not lost
by a take.

Rows go to stdout as CSV (group, name, edits, ref_units, cer, first_lost, last_lost), and the
totals per group to stderr. Chinese is compared as written: the FLEURS references and the local
models' text are simplified characters already.
Run with the system Python, isolated: /usr/bin/python3 -I scripts/measure/edge-loss.py ...
"""

from __future__ import annotations

import argparse
import csv
import sys
import unicodedata
from pathlib import Path


def units(text: str) -> list[str]:
    """The characters, letters and digits of `text`, lower case, without punctuation or tags."""
    out = []
    i = 0
    while i < len(text):
        if text.startswith("<|", i):
            end = text.find("|>", i)
            i = len(text) if end < 0 else end + 2
            continue
        ch = text[i]
        i += 1
        if unicodedata.category(ch)[0] in "LN":
            out.extend(ch.lower())
    return out


def align(ref: list[str], hyp: list[str]) -> tuple[int, bool, bool]:
    """Edits, and whether the reference's first and last units are deleted in an optimal
    alignment (ties prefer keeping them: a deletion counts only when every optimal path deletes
    the unit, approximated by preferring match/substitution moves in the backtrace)."""
    n, m = len(ref), len(hyp)
    d = [[0] * (m + 1) for _ in range(n + 1)]
    for i in range(n + 1):
        d[i][0] = i
    for j in range(m + 1):
        d[0][j] = j
    for i in range(1, n + 1):
        for j in range(1, m + 1):
            d[i][j] = min(d[i - 1][j - 1] + (ref[i - 1] != hyp[j - 1]), d[i - 1][j] + 1, d[i][j - 1] + 1)
    if n == 0:
        return m, False, False
    # Backtrace from the end: which reference units are deleted.
    deleted = [False] * n
    i, j = n, m
    while i > 0 or j > 0:
        if i > 0 and j > 0 and d[i][j] == d[i - 1][j - 1] + (ref[i - 1] != hyp[j - 1]):
            i, j = i - 1, j - 1
        elif j > 0 and d[i][j] == d[i][j - 1] + 1:
            j -= 1
        else:
            deleted[i - 1] = True
            i -= 1
    return d[n][m], deleted[0], deleted[-1]


def score(ref_text: str, hyp_text: str) -> tuple[int, int, bool, bool]:
    ref, hyp = units(ref_text), units(hyp_text)
    edits, first, last = align(ref, hyp)
    return edits, len(ref), first, last


def counted(ref_text: str, hyp_text: str, base_text: str | None) -> tuple[int, int, bool, bool]:
    """[`score`] whose first and last losses count only where `base_text` keeps them."""
    edits, n, first, last = score(ref_text, hyp_text)
    if base_text is not None:
        _, _, base_first, base_last = score(ref_text, base_text)
        first, last = first and not base_first, last and not base_last
    return edits, n, first, last


def emit(rows: list[tuple[str, str, int, int, bool, bool]]) -> None:
    out = csv.writer(sys.stdout)
    out.writerow(["group", "name", "edits", "ref_units", "cer", "first_lost", "last_lost"])
    totals: dict[str, list[int]] = {}
    for group, name, edits, n, first, last in rows:
        out.writerow([group, name, edits, n, f"{(100 * edits / n) if n else 0:.2f}", int(first), int(last)])
        t = totals.setdefault(group, [0, 0, 0, 0, 0])
        t[0] += edits
        t[1] += n
        t[2] += int(first)
        t[3] += int(last)
        t[4] += 1
    for group, (edits, n, first, last, count) in totals.items():
        print(f"{group}: {count} takes, CER {100 * edits / max(n, 1):.2f}% ({edits}/{n}), first lost {first} ({100 * first / count:.1f}%), last lost {last} ({100 * last / count:.1f}%)", file=sys.stderr)


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest="cmd", required=True)
    pairs = sub.add_parser("pairs")
    pairs.add_argument("manifest")
    pairs.add_argument("hyp_dir")
    pairs.add_argument("--prefix", default="")
    pairs.add_argument("--group", default="all")
    pairs.add_argument("--baseline-dir")
    takes = sub.add_parser("takes")
    takes.add_argument("tsv")
    takes.add_argument("--baseline-dir")
    takes.add_argument("--prefix", default="")
    a = p.parse_args()
    rows = []
    if a.cmd == "pairs":
        with open(a.manifest, newline="", encoding="utf-8") as f:
            for r in csv.DictReader(f):
                ref = Path(r["ref"]).read_text(encoding="utf-8")
                hyp_path = Path(a.hyp_dir) / f"{a.prefix}{r['name']}.txt"
                hyp = hyp_path.read_text(encoding="utf-8") if hyp_path.exists() else ""
                base = None
                if a.baseline_dir:
                    base_path = Path(a.baseline_dir) / f"{a.prefix}{r['name']}.txt"
                    base = base_path.read_text(encoding="utf-8") if base_path.exists() else ""
                rows.append((a.group, r["name"], *counted(ref, hyp, base)))
    else:
        with open(a.tsv, newline="", encoding="utf-8") as f:
            for r in csv.DictReader(f, delimiter="\t"):
                # An empty column is no baseline; --baseline-dir fills one from the files.
                base = r.get("base") or None
                if base is None and a.baseline_dir:
                    path = Path(a.baseline_dir) / f"{a.prefix}{r['name']}.txt"
                    base = path.read_text(encoding="utf-8") if path.exists() else None
                rows.append((r["group"], r["name"], *counted(r["ref"], r["hyp"], base)))
    emit(rows)
    return 0


if __name__ == "__main__":
    sys.exit(main())
