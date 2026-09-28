# shellcheck shell=bash
# shellcheck disable=SC2034  # the VOLTIP_SMOKE_* variables are this library's output to its callers
# Helpers for the Linux smoke scripts that need a dictation to reach the injector
# (docs/dictation.md §14): a clocked fake microphone, a tone to speak into it, a local stand-in
# for the OpenAI-compatible ASR endpoint, and a settings.json that points the app at it.
#
# Why each piece exists: the core refuses a silent take ("没有听到声音") before any ASR call, and
# Xvfb / headless compositors have no sound card. A PulseAudio null sink is clocked in real time;
# its monitor becomes the default source (ALSA `default` → pulse), and paplay feeds it the tone.
# The mock ASR answers every transcription with a fixed text, so the pipeline runs to the injector
# without a network or a model. Source this file; every function returns non-zero on failure.

# Write a 16 kHz mono 16-bit WAV with a 440 Hz tone at -12 dBFS: voltip_smoke_tone_wav <out> [seconds]
voltip_smoke_tone_wav() {
  python3 - "$1" "${2:-8}" <<'PY'
import math, struct, sys, wave
path, seconds, rate = sys.argv[1], float(sys.argv[2]), 16000
with wave.open(path, "wb") as w:
    w.setnchannels(1)
    w.setsampwidth(2)
    w.setframerate(rate)
    w.writeframes(b"".join(struct.pack("<h", int(0.25 * 32767 * math.sin(2 * math.pi * 440 * i / rate))) for i in range(int(rate * seconds))))
PY
}

# Start the mock ASR on 127.0.0.1 (random port): voltip_smoke_mock_asr_start <dir> <text>
# Sets VOLTIP_SMOKE_ASR_URL and VOLTIP_SMOKE_ASR_PID; every request is appended to <dir>/asr.log.
voltip_smoke_mock_asr_start() {
  local dir=$1 text=$2
  python3 - "$dir" "$text" >"$dir/asr.out" 2>&1 <<'PY' &
import http.server, json, pathlib, sys
root, text = pathlib.Path(sys.argv[1]), sys.argv[2]
class Handler(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        length = int(self.headers.get("Content-Length") or 0)
        self.rfile.read(length)
        with open(root / "asr.log", "a", encoding="utf-8") as log:
            log.write(f"POST {self.path} {length} bytes\n")
        if not self.path.endswith("/audio/transcriptions"):
            self.send_response(404)
            self.end_headers()
            return
        body = json.dumps({"text": text}, ensure_ascii=False).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    def log_message(self, *args):
        pass
server = http.server.HTTPServer(("127.0.0.1", 0), Handler)
(root / "asr.port").write_text(str(server.server_address[1]))
server.serve_forever()
PY
  VOLTIP_SMOKE_ASR_PID=$!
  # shellcheck disable=SC2016  # $1 is expanded by the inner sh
  timeout 10 sh -c 'until [ -s "$1/asr.port" ]; do sleep 0.1; done' _ "$dir" || { echo "mock ASR did not start: $(cat "$dir/asr.out" 2>/dev/null)" >&2; return 1; }
  VOLTIP_SMOKE_ASR_URL="http://127.0.0.1:$(cat "$dir/asr.port")"
}

# Settings for a take that ends in the injector: the custom provider's recognition at <asr_url>
# (the mock; the custom endpoint needs no key), no refine; every
# other value is the app default (light theme, system locale, hold Ctrl+Alt+Space), so screenshots
# look like a first start. voltip_smoke_settings <settings.json> <asr_url> [paste|clipboard_only] [relay: true|false]
voltip_smoke_settings() {
  mkdir -p "$(dirname "$1")"
  cat >"$1" <<JSON
{
  "schema": 1,
  "theme": "light",
  "follow_system_theme": false,
  "relay_enabled": ${4:-false},
  "engines": {
    "asr_provider": "custom",
    "providers": { "custom": { "asr_url": "$2", "asr_model": "voltip-smoke-mock" } },
    "language": "zh",
    "refine_enabled": false,
    "inject": "${3:-paste}"
  }
}
JSON
}

# A private PulseAudio server (no hardware, no default.pa) with one null sink whose monitor is the
# default source: voltip_smoke_pulse_mic_start <dir> <sink-name>. Exports PULSE_RUNTIME_PATH and
# PULSE_SERVER, so
# pactl / paplay and every app started afterwards (ALSA `default` → pulse plugin) talk to it and
# never to the desktop session's own server. Sets VOLTIP_SMOKE_SINK and VOLTIP_SMOKE_PULSE_PID.
voltip_smoke_pulse_mic_start() {
  local dir=$1 sink=$2
  local tool
  for tool in pulseaudio pactl paplay; do
    command -v "$tool" >/dev/null || { echo "$tool missing (apt install pulseaudio pulseaudio-utils libasound2-plugins)" >&2; return 2; }
  done
  export PULSE_RUNTIME_PATH=$dir/pulse PULSE_SERVER=unix:$dir/pulse/native
  mkdir -p "$PULSE_RUNTIME_PATH" && chmod 700 "$PULSE_RUNTIME_PATH"
  pulseaudio --daemonize=no --exit-idle-time=-1 -n --log-target="file:$dir/pulseaudio.log" \
    --load=module-native-protocol-unix \
    --load="module-null-sink sink_name=$sink sink_properties=device.description=$sink" >/dev/null 2>&1 &
  VOLTIP_SMOKE_PULSE_PID=$!
  timeout 15 sh -c 'until pactl info >/dev/null 2>&1; do sleep 0.2; done' || { echo "private PulseAudio did not start: $(tail -3 "$dir/pulseaudio.log" 2>/dev/null)" >&2; return 2; }
  pactl set-default-source "$sink.monitor" || return 2
  VOLTIP_SMOKE_SINK=$sink
}

# Stop processes and wait until they are gone (10 s each, then SIGKILL), so the directory they
# write into can be removed after: voltip_smoke_stop <pid>... Empty arguments are skipped; never
# fails. A bare `kill` followed by `rm -rf` raced the app's last writes ("Directory not empty").
voltip_smoke_stop() {
  local pid
  for pid in "$@"; do
    [ -n "$pid" ] || continue
    kill "$pid" 2>/dev/null || continue
    if ! timeout 10 tail --pid="$pid" -f /dev/null 2>/dev/null; then
      kill -9 "$pid" 2>/dev/null
      timeout 5 tail --pid="$pid" -f /dev/null 2>/dev/null
    fi
  done
  return 0
}

# Stop the private PulseAudio (safe to call twice, never fails).
voltip_smoke_pulse_mic_stop() {
  voltip_smoke_stop "${VOLTIP_SMOKE_PULSE_PID:-}"
  VOLTIP_SMOKE_PULSE_PID=""
  return 0
}

# Play the tone into the fake microphone in the background: voltip_smoke_speak <wav>
# Sets VOLTIP_SMOKE_PLAY_PID.
voltip_smoke_speak() {
  paplay --device="$VOLTIP_SMOKE_SINK" "$1" >/dev/null 2>&1 &
  VOLTIP_SMOKE_PLAY_PID=$!
}

# The last history entry as `outcome<TAB>via<TAB>text` (empty when none): voltip_smoke_last_history <history.json>
voltip_smoke_last_history() {
  python3 - "$1" <<'PY'
import json, sys
try:
    data = json.load(open(sys.argv[1], encoding="utf-8"))
except (OSError, ValueError):
    sys.exit(0)
entries = data["entries"] if isinstance(data, dict) else data
if entries:
    e = entries[-1]
    o = e.get("outcome") or {}
    print(f"{o.get('kind', '')}\t{o.get('via', '')}\t{e.get('text', '')}")
PY
}

# Strip ANSI colour from a log for grepping: voltip_smoke_plain <log>
voltip_smoke_plain() {
  sed -E 's/\x1b\[[0-9;]*m//g' "$1"
}
