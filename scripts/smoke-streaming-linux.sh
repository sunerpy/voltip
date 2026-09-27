#!/usr/bin/env bash
# shellcheck disable=SC2016  # the `sh -c '…'` wait loops read their arguments as $1 inside the child shell
# Real audio through the streaming output mode, end to end, in the real desktop app under Xvfb.
#
# What it proves (docs/dictation.md §11–§12): with the streaming Zipformer and a local whole-take
# model installed, `output_mode = streaming_final` shows partial text on the pill while a real
# 16 kHz Chinese sample plays into the microphone, and the finished take is recorded in
# history.json with `mode = streaming_final` and the recogniser's sentence segments.
#
# The microphone is a PulseAudio null sink's monitor (the host has no sound card): the sample is
# played into the sink with paplay while the hotkey is held with xdotool. Nothing here talks to a
# network: the engine is local and the models are copied from already-downloaded directories.
#
# Required environment:
#   VOLTIP_LOCAL_STREAM_DIR   directory with the streaming Zipformer files (catalogue
#                             `zipformer-stream-zh-en`: encoder.int8.onnx decoder.onnx
#                             joiner.int8.onnx tokens.txt bpe.model)
#   VOLTIP_LOCAL_MODEL_DIR    directory with SenseVoice-small (`sense-voice-small`:
#                             model.int8.onnx tokens.txt)
#   VOLTIP_LOCAL_SAMPLE_WAV   16 kHz mono WAV with Chinese speech (default /tmp/voltip-sample.wav)
#
# Output: docs/acceptance/screens/tauri/desktop-linux-xvfb-streaming-live.png (the pill with live
# text, cropped to the overlay window so no engine host name is captured) and
# docs/acceptance/screens/tauri/desktop-linux-xvfb-streaming-summary.txt (mode, segments, text;
# git-ignored: it names this machine's paths and the commit).
# Exit 0 only if the take finished as streaming_final with at least one committed segment.
set -euo pipefail
cd "$(dirname "$0")/.."

stream_dir=${VOLTIP_LOCAL_STREAM_DIR:?set VOLTIP_LOCAL_STREAM_DIR to the streaming Zipformer directory}
model_dir=${VOLTIP_LOCAL_MODEL_DIR:?set VOLTIP_LOCAL_MODEL_DIR to the SenseVoice-small directory}
sample=${VOLTIP_LOCAL_SAMPLE_WAV:-/tmp/voltip-sample.wav}
display=${VOLTIP_SMOKE_DISPLAY:-:98}
out_dir=docs/acceptance/screens/tauri
shot=$out_dir/desktop-linux-xvfb-streaming-live.png
summary=$out_dir/desktop-linux-xvfb-streaming-summary.txt

for tool in Xvfb xdotool scrot pactl paplay sha256sum; do
  command -v "$tool" >/dev/null || { echo "smoke-streaming-linux: missing $tool" >&2; exit 2; }
done
[ -f "$sample" ] || { echo "smoke-streaming-linux: sample WAV not found: $sample" >&2; exit 2; }
py=""
for candidate in python3 /usr/bin/python3; do
  if "$candidate" -c 'import PIL' 2>/dev/null; then py=$candidate; break; fi
done
[ -n "$py" ] || { echo "smoke-streaming-linux: no python3 with PIL (apt install python3-pil)" >&2; exit 2; }

# VOLTIP_SMOKE_NO_BUILD=1 reuses the binary already in target/ (a tree mid-edit must not be rebuilt).
if [ -z "${VOLTIP_SMOKE_NO_BUILD:-}" ]; then cargo build -q -p voltip-desktop --features custom-protocol; fi
[ -x ./target/debug/voltip-desktop ] || { echo "smoke-streaming-linux: target/debug/voltip-desktop missing" >&2; exit 2; }

data=$(mktemp -d)
app_data=$data/data/voltip
mkdir -p "$app_data/models"
sink=voltip_smoke_$$
cleanup() {
  set +e
  [ -n "${app_pid:-}" ] && kill "$app_pid" 2>/dev/null
  [ -n "${xvfb_pid:-}" ] && kill "$xvfb_pid" 2>/dev/null
  [ -n "${sink_module:-}" ] && pactl unload-module "$sink_module" >/dev/null 2>&1
  [ -n "${previous_source:-}" ] && pactl set-default-source "$previous_source" >/dev/null 2>&1
  rm -rf "$data"
}
trap cleanup EXIT

# Install the two models the way the store expects them: files + manifest.json whose sha256 values
# match the catalogue (the store re-checks name, sha and size before it loads anything).
install_model() {
  local id=$1 src=$2; shift 2
  local dir=$app_data/models/$id
  mkdir -p "$dir"
  local entries=()
  for f in "$@"; do
    [ -f "$src/$f" ] || { echo "smoke-streaming-linux: $src/$f missing" >&2; exit 2; }
    cp "$src/$f" "$dir/$f"
    entries+=("\"$f\": \"$(sha256sum "$dir/$f" | cut -d' ' -f1)\"")
  done
  local joined; joined=$(IFS=,; echo "${entries[*]}")
  printf '{"id":"%s","version":1,"downloaded_at":%s,"files":{%s}}\n' "$id" "$(date +%s)" "$joined" >"$dir/manifest.json"
}
install_model zipformer-stream-zh-en "$stream_dir" encoder.int8.onnx decoder.onnx joiner.int8.onnx tokens.txt bpe.model
install_model sense-voice-small "$model_dir" model.int8.onnx tokens.txt

# Settings: local SenseVoice for the whole take, live preview on, streaming_final, no refine (no
# network), locale zh-CN so the pill copy is Chinese.
cat >"$app_data/settings.json" <<'EOF'
{
  "schema": 1,
  "theme": "graphite",
  "follow_system_theme": false,
  "relay_enabled": false,
  "hotkey": "Ctrl+Alt+Space",
  "engines": {
    "asr_provider": "local",
    "local_model": "sense-voice-small",
    "live_preview": true,
    "output_mode": "streaming_final",
    "vad_trim": false,
    "language": "zh",
    "refine_enabled": false,
    "inject": "clipboard_only"
  },
  "locale": "zh-cn",
  "auto_update": false,
  "activation": "hold",
  "hold_threshold_ms": 300,
  "extra_recording_ms": 0
}
EOF

# Microphone: a null sink whose monitor becomes the default source.
previous_source=$(pactl get-default-source 2>/dev/null || true)
sink_module=$(pactl load-module module-null-sink sink_name="$sink" "sink_properties=device.description=$sink")
pactl set-default-source "$sink.monitor"

Xvfb "$display" -screen 0 1280x800x24 >/dev/null 2>&1 &
xvfb_pid=$!
timeout 20 sh -c "until DISPLAY=$display xdpyinfo >/dev/null 2>&1; do sleep 0.5; done"

DISPLAY=$display VOLTIP_DEV_SECRET_STORE=memory VOLTIP_DEV_OPAQUE_OVERLAY=1 XDG_DATA_HOME=$data/data XDG_CONFIG_HOME=$data/config \
  RUST_LOG=voltip=info ./target/debug/voltip-desktop >"$data/app.log" 2>&1 &
app_pid=$!

if ! timeout 90 sh -c "until DISPLAY=$display xdotool search --name '^Voltip$' >/dev/null 2>&1; do sleep 1; done"; then
  echo "smoke-streaming-linux: main window never appeared" >&2; tail -30 "$data/app.log" >&2; exit 1
fi
# The engines state must report the streaming model as ready before the take starts.
if ! timeout 60 sh -c 'until sed -E "s/\x1b\[[0-9;]*m//g" "$1" | grep -q "global hotkey registered"; do sleep 0.5; done' _ "$data/app.log"; then
  echo "smoke-streaming-linux: hotkey never registered" >&2; tail -30 "$data/app.log" >&2; exit 1
fi
sleep 2

# Hold the hotkey, play the sample into the sink, screenshot the pill mid-take, release.
DISPLAY=$display xdotool keydown ctrl+alt+space
if ! timeout 20 sh -c 'until sed -E "s/\x1b\[[0-9;]*m//g" "$1" | grep -qE "phase=\"listening\""; do sleep 0.2; done' _ "$data/app.log"; then
  DISPLAY=$display xdotool keyup ctrl+alt+space
  echo "smoke-streaming-linux: listening never started" >&2; tail -30 "$data/app.log" >&2; exit 1
fi
paplay --device="$sink" "$sample" &
play_pid=$!
sleep 9
DISPLAY=$display scrot -o "$data/full.png"
overlay_id=$(DISPLAY=$display xdotool search --onlyvisible --name '^Voltip Overlay$' | head -1 || true)
if [ -n "$overlay_id" ]; then
  eval "$(DISPLAY=$display xdotool getwindowgeometry --shell "$overlay_id")"
  "$py" - "$data/full.png" "$shot" "$X" "$Y" "$WIDTH" "$HEIGHT" <<'EOF'
import sys
from PIL import Image
src, dst, x, y, w, h = sys.argv[1], sys.argv[2], *map(int, sys.argv[3:7])
img = Image.open(src)
pad = 12
box = (max(0, x - pad), max(0, y - pad), min(img.width, x + w + pad), min(img.height, y + h + pad))
img.crop(box).save(dst)
EOF
fi
wait "$play_pid" || true
sleep 1
DISPLAY=$display xdotool keyup ctrl+alt+space

if ! timeout 60 sh -c 'until sed -E "s/\x1b\[[0-9;]*m//g" "$1" | grep -qE "phase=\"(done|failed|cancelled)\""; do sleep 0.5; done' _ "$data/app.log"; then
  echo "smoke-streaming-linux: the take never finished" >&2; tail -30 "$data/app.log" >&2; exit 1
fi
sleep 1

# Evidence: the history entry (mode, segments, text) and the engine's mode decision lines.
history=$app_data/history.json
[ -f "$history" ] || { echo "smoke-streaming-linux: no history.json written" >&2; tail -30 "$data/app.log" >&2; exit 1; }
status=0
"$py" - "$history" "$data/app.log" "$summary" "$(git rev-parse --short HEAD)" <<'EOF' || status=$?
import json, re, sys
history, log, out, commit = sys.argv[1:5]
data = json.load(open(history))
entries = data["entries"] if isinstance(data, dict) else data
entry = entries[-1] if entries else None
ansi = re.compile(r"\x1b\[[0-9;]*m")
lines = [ansi.sub("", l.rstrip()) for l in open(log, encoding="utf-8", errors="replace")]
keep = [l for l in lines if any(k in l for k in ("output mode", "dictation phase", "live preview", "capture stopped", "streaming", "segments", "local model loaded"))]
# docs/dictation.md §10.7: the whole-take model is warmed at start-up, before the first key press.
def first(pred):
    return next((i for i, l in enumerate(lines) if pred(l)), None)
loaded = first(lambda l: "local model loaded" in l and "sense-voice-small" in l)
listening = first(lambda l: 'phase="listening"' in l)
warm = loaded is not None and listening is not None and loaded < listening
with open(out, "w", encoding="utf-8") as f:
    f.write(f"smoke-streaming-linux @ {commit}\n")
    f.write("engine: local sense-voice-small (whole take) + zipformer-stream-zh-en (streaming), output_mode=streaming_final, refine off\n")
    f.write("microphone: PulseAudio null-sink monitor fed by paplay with the 16 kHz Chinese sample\n\n")
    if entry is None:
        f.write("history: EMPTY\n")
    else:
        f.write(f"history.mode: {entry.get('mode')}\n")
        f.write(f"history.text: {entry.get('text')}\n")
        segs = entry.get("segments") or []
        f.write(f"history.segments: {len(segs)}\n")
        for s in segs:
            f.write(f"  [{s['start_ms']:>6} ms – {s['end_ms']:>6} ms] {s['text']}\n")
        if entry.get("live_error"):
            f.write(f"history.live_error: {entry['live_error']}\n")
        f.write(f"history.outcome: {json.dumps(entry.get('outcome'), ensure_ascii=False)}\n")
        f.write(f"history.duration_ms: {entry.get('duration_ms')} asr_ms: {entry.get('asr_ms')}\n")
    f.write(f"warm-up: {'sense-voice-small was in memory before the key press' if warm else 'sense-voice-small was NOT loaded before the key press'}\n")
    f.write("\napp.log (filtered, consecutive repeats collapsed):\n")
    previous, repeats = None, 0
    def flush():
        if previous is not None:
            f.write("  " + previous + (f"  (x{repeats})" if repeats > 1 else "") + "\n")
    for l in keep:
        line = re.sub(r"^\S+\s+", "", l)
        if line == previous:
            repeats += 1
            continue
        flush()
        previous, repeats = line, 1
    flush()
ok = warm and entry is not None and entry.get("mode") == "streaming_final" and len(entry.get("segments") or []) >= 1 and not entry.get("live_error")
print(open(out, encoding="utf-8").read())
sys.exit(0 if ok else 1)
EOF
[ -f "$shot" ] && echo "smoke-streaming-linux: pill screenshot → $shot"
if [ "$status" -ne 0 ]; then echo "smoke-streaming-linux: FAILED (see $summary)" >&2; exit 1; fi
echo "smoke-streaming-linux: OK → $summary"
