#!/usr/bin/env bash
# Recognition quality and speed by input length for local models (docs/dictation.md §29): one
# `piece_quality` run per model over the corpus of make-corpus.py, under `/usr/bin/time -v`, its
# rows kept in <out>/<machine>-<model>.rows, then summarize-pieces.py over every rows file in <out>
# (rows another machine produced, such as scripts/windows-remote.sh gate pieces, included).
#
#   scripts/measure/local-pieces.sh [--out DIR] [--machine NAME] <model-id>=<model-dir>...
#
#   VOLTIP_MEASURE_MANIFEST  the corpus (default /tmp/voltip-measure/corpus/manifest.csv)
#   VOLTIP_MEASURE_LENGTHS   the clip lengths in seconds (default 30,60,90,119,121)
#   VOLTIP_MEASURE_ESTIMATES space-separated MACHINE:MODEL:REFERENCE estimates for a model not
#                            measured on a machine (summarize-pieces.py --estimate)
#
# Results go to /tmp/voltip-measure/results by default; neither the audio nor the results are
# committed. Run it alone on the machine: the timings are what LOCK_WAIT is made of.
set -euo pipefail
cd "$(dirname "$0")/../.."

out=/tmp/voltip-measure/results
machine=$(uname -s | tr '[:upper:]' '[:lower:]')
while [ $# -gt 0 ]; do
  case $1 in
    --out) out=$2; shift 2 ;;
    --machine) machine=$2; shift 2 ;;
    -*) echo "local-pieces: unknown option $1" >&2; exit 2 ;;
    *) break ;;
  esac
done
if [ $# -eq 0 ]; then
  echo "usage: $0 [--out DIR] [--machine NAME] <model-id>=<model-dir>..." >&2
  exit 2
fi
manifest=${VOLTIP_MEASURE_MANIFEST:-/tmp/voltip-measure/corpus/manifest.csv}
lengths=${VOLTIP_MEASURE_LENGTHS:-30,60,90,119,121}
[ -f "$manifest" ] || { echo "local-pieces: no corpus at $manifest (scripts/measure/make-corpus.py)" >&2; exit 2; }

mkdir -p "$out/hyp"
cargo build --release -p voltip-asr-local --example piece_quality
bin=target/release/examples/piece_quality
for spec in "$@"; do
  model=${spec%%=*}
  dir=${spec#*=}
  [ -d "$dir" ] || { echo "local-pieces: $model: $dir is not a directory" >&2; exit 2; }
  echo "local-pieces: $model ($dir) on $machine, lengths $lengths"
  /usr/bin/time -v "$bin" --model "$model" --dir "$dir" --manifest "$manifest" --lengths "$lengths" --machine "$machine" --hyp-dir "$out/hyp" \
    >"$out/$machine-$model.rows" 2>"$out/$machine-$model.log"
  grep -E 'Maximum resident set size|Elapsed \(wall clock\)' "$out/$machine-$model.log" | sed 's/^[[:space:]]*/local-pieces:   /'
done
estimates=()
for spec in ${VOLTIP_MEASURE_ESTIMATES:-}; do estimates+=(--estimate "$spec"); done
/usr/bin/python3 -I scripts/measure/summarize-pieces.py --out "$out" "${estimates[@]}" "$out"/*.rows
