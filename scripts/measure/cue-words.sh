#!/usr/bin/env bash
# The ready cue and the first words (docs/dictation.md §30.3, §30.4): every prepared sentence
# (tail-loss.py prepare) is recorded through a private PulseAudio server twice, with the cue and
# without it, the sentence starting 150 ms after the cue's window (crates/voltip-audio/tests/
# ready_cue.rs, `the_cue_leaves_the_first_words_alone`); the same local model recognises both
# (piece_quality) and edge-loss.py compares their first characters and CER. Nothing is heard and
# no other session's sound server is touched; the server ends with the script.
#
#   scripts/measure/cue-words.sh <work-dir> <sentences.tsv> <model-dir> [<baseline-hyp-dir> <prefix>]
#
# <model-dir> holds paraformer-zh's files; the optional baseline (the model's text of the whole
# recordings, as edge-loss.py --baseline-dir takes it) keeps unread glosses from counting as lost.
set -euo pipefail
cd "$(dirname "$0")/../.."

work=${1:?usage: cue-words.sh <work-dir> <sentences.tsv> <model-dir> [<baseline-dir> <prefix>]}
sentences=${2:?usage: cue-words.sh <work-dir> <sentences.tsv> <model-dir>}
model=${3:?usage: cue-words.sh <work-dir> <sentences.tsv> <model-dir>}
baseline=${4:-}
prefix=${5:-}
for tool in pulseaudio pactl paplay parec pacat cargo; do
  command -v "$tool" >/dev/null || { echo "cue-words: $tool not installed" >&2; exit 2; }
done
mkdir -p "$work"
work=$(cd "$work" && pwd)

pulse="$work/pulse"
keepalive_pgid=""
cleanup() {
  [ -n "$keepalive_pgid" ] && kill -- "-$keepalive_pgid" 2>/dev/null || true
  scripts/measure/private-pulse.sh stop "$pulse" || true
}
trap cleanup EXIT INT TERM
eval "$(scripts/measure/private-pulse.sh start "$pulse" | sed -n 's/^PULSE_SERVER=/export PULSE_SERVER=/p')"
# As in tail-loss.sh: an idle null sink renders two seconds ahead, and whatever plays or records
# next starts behind that block. Silence on a 10 ms stream keeps it rendering as a sound card does.
setsid timeout --foreground 7200 pacat --playback --latency-msec=10 --raw --format=s16le --rate=48000 --channels=2 /dev/zero &
keepalive_pgid=$!

# The recorder's input, and the references by name.
/usr/bin/python3 -I - "$sentences" "$work" <<'EOF'
import csv, sys
from pathlib import Path
sentences, work = sys.argv[1], Path(sys.argv[2])
rows = list(csv.DictReader(open(sentences, encoding="utf-8"), delimiter="\t"))
with open(work / "cue-input.csv", "w", newline="", encoding="utf-8") as f:
    w = csv.writer(f)
    w.writerow(["name", "wav"])
    for r in rows:
        w.writerow([r["name"], r["wav"]])
EOF

rm -rf "$work/takes"
VOLTIP_CUE_CORPUS="$work/cue-input.csv" VOLTIP_CUE_OUT="$work/takes" \
  cargo test -p voltip-audio --test ready_cue -- --ignored --exact the_cue_leaves_the_first_words_alone --nocapture

# Both recordings of every sentence, recognised by the same model.
/usr/bin/python3 -I - "$sentences" "$work" <<'EOF'
import csv, sys, wave
from pathlib import Path
sentences, work = sys.argv[1], Path(sys.argv[2])
rows = list(csv.DictReader(open(sentences, encoding="utf-8"), delimiter="\t"))
refs = work / "refs"
refs.mkdir(exist_ok=True)
with open(work / "takes.csv", "w", newline="", encoding="utf-8") as f:
    w = csv.writer(f)
    w.writerow(["lang", "name", "length_s", "speech_s", "wav", "ref"])
    for r in rows:
        ref = refs / f"{r['name']}.txt"
        ref.write_text(r["ref"], encoding="utf-8")
        for kind in ("cue", "plain"):
            wav = work / "takes" / f"{r['name']}-{kind}.wav"
            with wave.open(str(wav), "rb") as take:
                length = take.getnframes() / take.getframerate()
            w.writerow(["cmn_hans_cn", f"{r['name']}-{kind}", f"{length:.3f}", f"{length:.3f}", wav, ref])
EOF
rm -rf "$work/hyp"
cargo run --release -p voltip-asr-local --example piece_quality -- \
  --model paraformer-zh --dir "$model" --manifest "$work/takes.csv" --lengths all --machine cue --hyp-dir "$work/hyp" >"$work/pieces.csv"

# One row per recording for edge-loss.py takes: the group is cue or plain, the name the sentence's.
/usr/bin/python3 -I - "$sentences" "$work" <<'EOF'
import csv, sys
from pathlib import Path
sentences, work = sys.argv[1], Path(sys.argv[2])
rows = list(csv.DictReader(open(sentences, encoding="utf-8"), delimiter="\t"))
with open(work / "takes-cue.tsv", "w", newline="", encoding="utf-8") as f:
    w = csv.writer(f, delimiter="\t")
    w.writerow(["group", "name", "run", "ref", "hyp", "base"])
    for r in rows:
        for kind in ("cue", "plain"):
            hyp = (work / "hyp" / f"cue-paraformer-zh-{r['name']}-{kind}.txt").read_text(encoding="utf-8")
            w.writerow([kind, r["name"], 1, r["ref"], hyp, ""])
EOF
if [ -n "$baseline" ]; then
  /usr/bin/python3 -I scripts/measure/edge-loss.py takes "$work/takes-cue.tsv" --baseline-dir "$baseline" --prefix "$prefix" >"$work/scores-cue.csv"
else
  /usr/bin/python3 -I scripts/measure/edge-loss.py takes "$work/takes-cue.tsv" >"$work/scores-cue.csv"
fi
