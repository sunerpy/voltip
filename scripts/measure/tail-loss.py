#!/usr/bin/env python3
"""Lost first and last words through the real desktop app (docs/dictation.md §30.4).

The app (a debug build) runs under Xvfb with a private PulseAudio server whose null sink's
monitor is its microphone, and a private session bus (scripts/measure/tail-loss.sh sets them up);
a sentence of the corpus plays into that sink during a take, which `voltip-desktop --toggle`
starts and stops (`--via toggle`, the default: the remote control of docs/dictation.md §13) or the
global shortcut Ctrl+Alt+Space holds, pressed through XTEST as a person presses it (`--via
hotkey`: its release waits out Hold's 50 ms grace); the take's text is read from the app's history.

  tail-loss.py prepare <manifest.csv> <out-dir>
      every sentence cut to start 50 ms before its speech, and when its speech ends: a 20 ms frame
      is speech 20 dB above the recording's noise floor (the quietest fifth of its frames):
      <out-dir>/sentences.tsv
  tail-loss.py tail --binary B --data D --sentences TSV --out TAKES.tsv [--via toggle|hotkey]
                    [--extras 0,150,300] [--early 0,100,200] [--runs 3]
      the stop `d` ms before the speech ends, with the extra recording at each value: the take
      starts, its capture starts, the sentence plays, the take stops
  tail-loss.py first --binary B --data D --sentences TSV --out TAKES.tsv --label NAME [--via ...]
                     [--runs 3]
      the take starts as the sentence starts playing and stops 800 ms after its speech (the extra
      recording off): what the start of a take loses

Every take is a row of TAKES.tsv (group, name, run, ref, hyp, the take's timing when the build
records one, and `played_at`, the Unix ms the playback started); edge-loss.py takes TAKES.tsv
scores them. One app at a time; DISPLAY, PULSE_SERVER and DBUS_SESSION_BUS_ADDRESS come from the
environment, and VOLTIP_MEASURE_MODEL names paraformer-zh's files (default
/tmp/voltip-measure/models/paraformer-zh). Run with the system Python, isolated.
"""

from __future__ import annotations

import argparse
import csv
import hashlib
import json
import math
import os
import signal
import sqlite3
import struct
import subprocess
import sys
import time
import wave
from pathlib import Path

HOTKEY = "ctrl+alt+space"
LEAD_MS = 50
# A 20 ms frame is speech this far above the noise floor: FLEURS recordings have their floor near
# -82 dBFS and speech from -30 down to -65 dBFS at the ends of words.
SPEECH_ABOVE_FLOOR_DB = 20.0
MODEL = "paraformer-zh"
MODEL_DIR = Path(os.environ.get("VOLTIP_MEASURE_MODEL", "/tmp/voltip-measure/models/paraformer-zh"))


def frames_dbfs(samples: list[int], rate: int) -> list[float]:
    step = rate // 50
    out = []
    for i in range(0, len(samples) - step + 1, step):
        frame = samples[i : i + step]
        rms = (sum(s * s for s in frame) / len(frame)) ** 0.5 / 32768.0
        out.append(-120.0 if rms <= 0 else 20.0 * math.log10(rms))
    return out


def read_wav(path: Path) -> tuple[list[int], int]:
    with wave.open(str(path), "rb") as w:
        assert w.getnchannels() == 1 and w.getsampwidth() == 2, path
        rate = w.getframerate()
        data = w.readframes(w.getnframes())
    return list(struct.unpack(f"<{len(data) // 2}h", data)), rate


def write_wav(path: Path, samples: list[int], rate: int) -> None:
    with wave.open(str(path), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(rate)
        w.writeframes(struct.pack(f"<{len(samples)}h", *samples))


def prepare(manifest: Path, out: Path) -> None:
    out.mkdir(parents=True, exist_ok=True)
    rows = []
    with open(manifest, newline="", encoding="utf-8") as f:
        for r in csv.DictReader(f):
            samples, rate = read_wav(Path(r["wav"]))
            levels = frames_dbfs(samples, rate)
            quiet = sorted(levels)[: max(1, len(levels) // 5)]
            threshold = sum(quiet) / len(quiet) + SPEECH_ABOVE_FLOOR_DB
            loud = [i for i, db in enumerate(levels) if db >= threshold]
            if not loud:
                print(f"tail-loss: {r['name']}: no speech found, skipped", file=sys.stderr)
                continue
            frame = rate // 50
            start = max(0, loud[0] * frame - LEAD_MS * rate // 1000)
            end = min(len(samples), (loud[-1] + 1) * frame)
            cut = samples[start:]
            wav = out / f"{r['name']}.wav"
            write_wav(wav, cut, rate)
            rows.append({
                "name": r["name"],
                "wav": str(wav),
                "ref": Path(r["ref"]).read_text(encoding="utf-8").strip(),
                "length_ms": len(cut) * 1000 // rate,
                "speech_end_ms": (end - start) * 1000 // rate,
            })
    with open(out / "sentences.tsv", "w", newline="", encoding="utf-8") as f:
        w = csv.DictWriter(f, fieldnames=list(rows[0].keys()), delimiter="\t")
        w.writeheader()
        w.writerows(rows)
    print(f"tail-loss: {len(rows)} sentences in {out / 'sentences.tsv'}", file=sys.stderr)


def stage_model(models: Path) -> None:
    """The local model in the app's library, as the store installs it (files and manifest)."""
    dir = models / MODEL
    dir.mkdir(parents=True, exist_ok=True)
    files = {}
    for name in ("model.int8.onnx", "tokens.txt"):
        src, dst = MODEL_DIR / name, dir / name
        if not dst.exists():
            os.link(src, dst)
        files[name] = hashlib.sha256(src.read_bytes()).hexdigest()
    (dir / "manifest.json").write_text(json.dumps({"id": MODEL, "version": 1, "downloaded_at": 1, "files": files}), encoding="utf-8")


def settings(extra_ms: int) -> dict:
    return {
        "schema": 1,
        "theme": "glass",
        "follow_system_theme": False,
        "relay_enabled": False,
        "auto_update": False,
        "activation": "hold",
        "extra_recording_ms": extra_ms,
        "history": {"enabled": True, "keep": 2000},
        "audio_cache": {"enabled": False, "keep": 20, "max_mb": 200},
        "engines": {
            "asr_provider": "local",
            "local_model": MODEL,
            "language": "zh",
            "refine_enabled": False,
            "inject": "clipboard_only",
            "vad_trim": False,
            "output_mode": "whole_take",
        },
    }


class App:
    """The desktop app under test, in a process group of its own."""

    def __init__(self, binary: Path, data: Path, extra_ms: int) -> None:
        self.data, self.binary = data, binary
        voltip = data / "data" / "voltip"
        voltip.mkdir(parents=True, exist_ok=True)
        (data / "config").mkdir(parents=True, exist_ok=True)
        stage_model(voltip / "models")
        (voltip / "settings.json").write_text(json.dumps(settings(extra_ms)), encoding="utf-8")
        self.history = voltip / "history.sqlite3"
        self.log = data / f"app-extra{extra_ms}.log"
        env = dict(os.environ)
        env.update({
            "VOLTIP_DEV_SECRET_STORE": "memory",
            "XDG_DATA_HOME": str(data / "data"),
            "XDG_CONFIG_HOME": str(data / "config"),
            "RUST_LOG": "info",
            "LD_LIBRARY_PATH": f"{binary.parent}:{env.get('LD_LIBRARY_PATH', '')}",
        })
        self.env = env
        self.log_file = open(self.log, "wb")
        self.proc = subprocess.Popen([str(binary)], env=env, stdout=self.log_file, stderr=subprocess.STDOUT, start_new_session=True)
        self.wait_log("global hotkey registered", 120)

    def toggle(self) -> None:
        """`voltip-desktop --toggle`: the second process hands the edge to this one and exits."""
        subprocess.run([str(self.binary), "--toggle"], env=self.env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True, timeout=30)

    def log_text(self) -> str:
        return self.log.read_bytes().decode("utf-8", "replace")

    def wait_log(self, marker: str, limit: float, after: int = 0) -> None:
        deadline = time.monotonic() + limit
        while marker not in self.log_text()[after:]:
            if self.proc.poll() is not None:
                raise RuntimeError(f"the app ended: see {self.log}")
            if time.monotonic() > deadline:
                raise RuntimeError(f"no '{marker}' in {limit} s: see {self.log}")
            time.sleep(0.02)

    def entries(self) -> list[dict]:
        if not self.history.exists():
            return []
        conn = sqlite3.connect(f"{self.history.resolve().as_uri()}?mode=ro", uri=True)
        try:
            return [json.loads(row[0]) for row in conn.execute("SELECT json FROM entries ORDER BY at_ms DESC, rowid DESC")]
        except sqlite3.Error:
            return []
        finally:
            conn.close()

    def stop(self) -> None:
        with_timeout = time.monotonic() + 10
        try:
            os.killpg(self.proc.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        while self.proc.poll() is None and time.monotonic() < with_timeout:
            time.sleep(0.05)
        if self.proc.poll() is None:
            os.killpg(self.proc.pid, signal.SIGKILL)
            self.proc.wait(5)
        self.log_file.close()


def key(action: str) -> None:
    subprocess.run(["xdotool", action, HOTKEY], check=True)


def edge(app: App, via: str, down: bool) -> None:
    """Start (`down`) or stop the take."""
    if via == "toggle":
        app.toggle()
    else:
        key("keydown" if down else "keyup")


def play(wav: str) -> subprocess.Popen:
    # A short buffer: the sentence starts as the take's press plans it.
    return subprocess.Popen(["paplay", "--latency-msec=20", wav], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def take(app: App, sentence: dict, *, via: str, early_ms: int | None) -> dict:
    """One take, the entry it wrote and when the playback started. With `early_ms` (the tail): the
    take starts, its capture starts, the sentence plays, the take stops `early_ms` before the
    speech ends. Without (the first words): the sentence starts playing and the take starts at
    once, and stops 800 ms after the speech."""
    before = {e.get("id") for e in app.entries()}
    offset = len(app.log_text())
    if early_ms is not None:
        edge(app, via, True)
        app.wait_log("dictation capture started", 10, offset)
    player = play(sentence["wav"])
    started = time.monotonic()
    played_at = time.time_ns() // 1_000_000
    if early_ms is None:
        edge(app, via, True)
    speech_end = int(sentence["speech_end_ms"]) / 1000
    up_at = started + (speech_end - early_ms / 1000 if early_ms is not None else speech_end + 0.8)
    time.sleep(max(0.0, up_at - time.monotonic()))
    edge(app, via, False)
    deadline = time.monotonic() + 60
    entry = None
    while entry is None and time.monotonic() < deadline:
        fresh = [e for e in app.entries() if e.get("id") not in before]
        if fresh:
            entry = fresh[0]
        else:
            time.sleep(0.05)
    try:
        player.wait(timeout=max(1.0, int(sentence["length_ms"]) / 1000 + 2))
    except subprocess.TimeoutExpired:
        player.kill()
        player.wait()
    if entry is None:
        return {"hyp": "", "timing": "", "outcome": "no-entry", "played_at": played_at}
    # The overlay dwells after a take; the next start interrupts it.
    return {
        "hyp": entry.get("raw_text", ""),
        "timing": json.dumps(entry.get("timing") or {}),
        "outcome": (entry.get("outcome") or {}).get("kind", ""),
        "played_at": played_at,
    }


def warm_up(app: App, sentence: dict, via: str) -> None:
    """Takes that are not counted. The core comes up after the shortcut is registered and drops a
    remote edge that arrives before it: a take is started until one starts. Then one take of
    `sentence`, whose recognition is the model's first."""
    for _ in range(20):
        offset = len(app.log_text())
        edge(app, via, True)
        try:
            app.wait_log("dictation capture started", 3, offset)
            break
        except RuntimeError:
            if via == "hotkey":
                key("keyup")
    else:
        raise RuntimeError(f"no take started: see {app.log}")
    offset = len(app.log_text())
    edge(app, via, False)
    app.wait_log('phase="idle"', 30, offset)
    take(app, sentence, via=via, early_ms=None)


def run(args: argparse.Namespace) -> None:
    with open(args.sentences, newline="", encoding="utf-8") as f:
        sentences = list(csv.DictReader(f, delimiter="\t"))
    out = Path(args.out)
    new = not out.exists()
    fields = ["group", "name", "run", "ref", "hyp", "base", "timing", "outcome", "played_at"]
    with open(out, "a", newline="", encoding="utf-8") as f:
        w = csv.DictWriter(f, fieldnames=fields, delimiter="\t")
        if new:
            w.writeheader()
        plans = []
        if args.cmd == "tail":
            for extra in [int(x) for x in args.extras.split(",")]:
                for early in [int(x) for x in args.early.split(",")]:
                    plans.append((extra, early, f"{args.via}-extra{extra}-early{early}"))
        else:
            plans.append((0, None, f"{args.via}-first-{args.label}"))
        app = None
        current_extra = None
        try:
            for extra, early, group in plans:
                if extra != current_extra:
                    if app is not None:
                        app.stop()
                    app = App(Path(args.binary), Path(args.data), extra)
                    current_extra = extra
                    warm_up(app, sentences[0], args.via)
                for run_no in range(1, args.runs + 1):
                    for s in sentences:
                        result = take(app, s, via=args.via, early_ms=early)
                        w.writerow({"group": group, "name": s["name"], "run": run_no, "ref": s["ref"], "base": "", **result})
                        f.flush()
                    print(f"tail-loss: {group} run {run_no} done", file=sys.stderr, flush=True)
        finally:
            if app is not None:
                app.stop()


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest="cmd", required=True)
    prep = sub.add_parser("prepare")
    prep.add_argument("manifest")
    prep.add_argument("out")
    for name in ("tail", "first"):
        s = sub.add_parser(name)
        s.add_argument("--binary", required=True)
        s.add_argument("--data", required=True)
        s.add_argument("--sentences", required=True)
        s.add_argument("--out", required=True)
        s.add_argument("--runs", type=int, default=3)
        s.add_argument("--via", choices=["toggle", "hotkey"], default="toggle")
        if name == "tail":
            s.add_argument("--extras", default="0,150,300")
            s.add_argument("--early", default="0,100,200")
        else:
            s.add_argument("--label", required=True)
    a = p.parse_args()
    if a.cmd == "prepare":
        prepare(Path(a.manifest), Path(a.out))
    else:
        run(a)
    return 0


if __name__ == "__main__":
    sys.exit(main())
