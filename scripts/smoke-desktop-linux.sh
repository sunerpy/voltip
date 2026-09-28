#!/usr/bin/env bash
# shellcheck disable=SC2016  # the `sh -c '…'` wait loops read their arguments as $1 / $2 inside the child shell
# Headless smoke run of the real Tauri desktop app on Linux: build (debug), start it under Xvfb
# with the debug-only in-memory secret store, wait for the window, screenshot the WebView, exit.
# Evidence that the shell builds, starts the core and renders — on a machine with no display and
# no keychain (CI runner, container). The hotkey take runs to the injector (docs/dictation.md §14):
# a private PulseAudio null sink is the microphone, a local mock stands in for the ASR, and the
# X11 clipboard must hold a sentinel again after the paste. Usage: scripts/smoke-desktop-linux.sh [out.png]
set -euo pipefail
cd "$(dirname "$0")/.."
. scripts/lib/build-env.sh && voltip_load_build_env
. scripts/lib/smoke-dictation.sh
out=${1:-docs/acceptance/screens/tauri/desktop-linux-xvfb.png}
display=:99
screen=1280x800x24
window_timeout=90

for tool in Xvfb xdotool scrot xclip pulseaudio pactl paplay; do
  command -v "$tool" >/dev/null || { echo "smoke-desktop-linux: $tool not installed"; exit 2; }
done
# The crop step needs Pillow. On some CI images `python3` is a pyenv interpreter without the apt
# `python3-pil` package, so prefer whichever interpreter can import PIL.
PY=""
for c in python3 /usr/bin/python3; do
  if command -v "$c" >/dev/null && "$c" -c "import PIL" >/dev/null 2>&1; then PY=$c; break; fi
done
[ -n "$PY" ] || { echo "smoke-desktop-linux: no python3 with Pillow (apt install python3-pil)"; exit 2; }
# The debug binary embeds apps/desktop/dist (custom-protocol); rebuild it first or the window shows
# whatever the last `vite build` produced.
pnpm --filter @voltip/desktop run build >/dev/null
cargo build -q -p voltip-desktop --features custom-protocol
mkdir -p "$(dirname "$out")"
data=$(mktemp -d)
app_pid=""
xvfb_pid=""
cleanup() {
  voltip_smoke_stop "$app_pid" "$xvfb_pid" "${VOLTIP_SMOKE_ASR_PID:-}"
  voltip_smoke_pulse_mic_stop
  # Keep the app log next to the screenshots (CI uploads it; git ignores it).
  [ -f "$data/app.log" ] && sed -E 's/\x1b\[[0-9;]*m//g' "$data/app.log" > "${out%.png}-app.log" 2>/dev/null || true
  rm -rf "$data"
}
trap cleanup EXIT

Xvfb "$display" -screen 0 "$screen" >/dev/null 2>&1 &
xvfb_pid=$!
timeout 20 sh -c "until DISPLAY=$display xdpyinfo >/dev/null 2>&1; do sleep 0.5; done"
# The take must reach the injector: a tone into a private PulseAudio null sink (the default source),
# the mock ASR answering with $spoken, paste injection, no refine. Everything else stays default.
spoken="Voltip 冒烟注入"
sentinel="voltip-smoke-sentinel-$$"
voltip_smoke_tone_wav "$data/tone.wav" 8
voltip_smoke_mock_asr_start "$data" "$spoken"
voltip_smoke_pulse_mic_start "$data" "voltip_smoke_$$"
voltip_smoke_settings "$data/data/voltip/settings.json" "$VOLTIP_SMOKE_ASR_URL" paste true

# VOLTIP_DEV_OPAQUE_OVERLAY: Xvfb has no compositor, so an RGBA (transparent) window renders as a
# black rectangle; the debug-only knob draws the pill window opaque so the capture shows the pill.
DISPLAY=$display VOLTIP_DEV_SECRET_STORE=memory VOLTIP_DEV_OPAQUE_OVERLAY=1 XDG_DATA_HOME=$data/data XDG_CONFIG_HOME=$data/config \
  RUST_LOG=voltip=info,voltip_inject=debug ./target/debug/voltip-desktop >"$data/app.log" 2>&1 &
app_pid=$!

# The window appears once the WebView is up; the core is already running by then (setup hook).
if ! timeout "$window_timeout" sh -c "until DISPLAY=$display xdotool search --name '^Voltip$' >/dev/null 2>&1; do sleep 1; done"; then
  echo "smoke-desktop-linux: window did not appear within ${window_timeout}s"; cat "$data/app.log"; exit 1
fi
# First paint of the React app after the window maps; the WebView gives no event we can wait on.
sleep 3
win=$(DISPLAY=$display xdotool search --name '^Voltip$' | head -1)
geometry=$(DISPLAY=$display xwininfo -id "$win" | awk '/Absolute upper-left X/ {x=$4} /Absolute upper-left Y/ {y=$4} /Width/ {w=$2} /Height/ {h=$2} END {printf "%dx%d+%d+%d", w, h, x, y}')
shoot() { DISPLAY=$display scrot --overwrite "$1" && "$PY" - "$1" "$geometry" <<'PYCROP'
import sys, re
from PIL import Image
path, geo = sys.argv[1], sys.argv[2]
w, h, x, y = map(int, re.match(r"(\d+)x(\d+)\+(\d+)\+(\d+)", geo).groups())
Image.open(path).crop((x, y, x + w, y + h)).save(path)
PYCROP
}
# A fresh profile opens the first-run guide by itself (apps/desktop/src/app/first-run.ts). Record
# it, then leave it the way a user would (Esc = 稍后设置) after a click on an empty spot of the card
# gives the WebView the keyboard; the home page follows. Same paint wait as above: the WebView
# reports no event for it.
shoot "${out%.png}-first-run.png"
DISPLAY=$display xdotool mousemove --window "$win" 600 450 click 1
DISPLAY=$display xdotool key Escape
sleep 2
shoot "$out"
# Devices page: identity fingerprint and the (empty) trusted list come from the real core over IPC.
# (Sidebar row 6 since Bridge & MCP was removed: 引擎 · 手机麦克风 · 设置.)
DISPLAY=$display xdotool mousemove --window "$win" 85 293 click 1
sleep 2
shoot "${out%.png}-devices.png"
# Global hotkey → prewarmed overlay pill: hold Ctrl+Alt+Space (XTEST goes through the X server, so
# the XGrabKey registration fires), screenshot the whole screen with the pill at the bottom centre,
# then release and confirm the pill window is hidden again. The sentinel goes on the clipboard first
# (xclip keeps serving it until the injector takes the selection over).
printf '%s' "$sentinel" | DISPLAY=$display xclip -selection clipboard -i
DISPLAY=$display xdotool keydown ctrl+alt+space
voltip_smoke_speak "$data/tone.wav"
# Poll for the pill instead of sleeping: on a host without a sound card the take fails within
# milliseconds and the failure pill hides again after its 2.5 s dwell, so a fixed 3 s wait would
# miss a perfectly working hotkey. Screenshot at first sight, keep holding for a realistic take.
overlay_visible=0
held_since=$(date +%s%N)
while [ $(( ($(date +%s%N) - held_since) / 1000000 )) -lt 3000 ]; do
  if [ "$( (DISPLAY=$display xdotool search --onlyvisible --name '^Voltip Overlay$' || true) | wc -l)" != "0" ]; then
    overlay_visible=1
    DISPLAY=$display scrot --overwrite "${out%.png}-hotkey-overlay.png"
    break
  fi
  sleep 0.1
done
while [ $(( ($(date +%s%N) - held_since) / 1000000 )) -lt 3000 ]; do sleep 0.1; done
# With a real capture the pill is still up after 3 s and fully painted: prefer that frame.
if [ "$( (DISPLAY=$display xdotool search --onlyvisible --name '^Voltip Overlay$' || true) | wc -l)" != "0" ]; then
  overlay_visible=1
  DISPLAY=$display scrot --overwrite "${out%.png}-hotkey-overlay.png"
fi
[ -f "${out%.png}-hotkey-overlay.png" ] || DISPLAY=$display scrot --overwrite "${out%.png}-hotkey-overlay.png"
DISPLAY=$display xdotool keyup ctrl+alt+space
# Releasing the hotkey runs the real pipeline: the recorder hands the tone take to the core, the
# mock ASR answers with $spoken, and the injector pastes it (docs/dictation.md §14: X11 → enigo
# XTEST first). The pill shows the result and the core returns to idle afterwards.
if ! timeout 20 sh -c 'until sed -E "s/\x1b\[[0-9;]*m//g" "$1" | grep -qE "phase=\"(done|failed)\""; do sleep 0.5; done' _ "$data/app.log"; then
  echo "smoke-desktop-linux: releasing the hotkey produced no dictation outcome"; grep -iE "dictation|record|asr" "$data/app.log" | sed -E 's/\x1b\[[0-9;]*m//g' | tail -8; exit 1
fi
DISPLAY=$display scrot --overwrite "${out%.png}-dictation-outcome.png"
# Injection: the take is recorded as pasted with the ASR text, and after the restore delay the
# clipboard holds the sentinel again (the injector saved it, pasted ours, put it back).
outcome=$(voltip_smoke_last_history "$data/data/voltip/history.json")
echo "smoke-desktop-linux: history → ${outcome:-<none>}; $(voltip_smoke_plain "$data/app.log" | grep -oE 'text injection backend backend=[^=]*' | head -1)"
if [ "$outcome" != "$(printf 'inserted\tpaste\t%s' "$spoken")" ]; then
  echo "smoke-desktop-linux: the take was not pasted (mock ASR requests: $(wc -l <"$data/asr.log" 2>/dev/null || echo 0))"
  voltip_smoke_plain "$data/app.log" | grep -E "inject|delivered|phase=\"failed|capture|asr" | tail -8; exit 1
fi
if ! timeout 5 sh -c 'until [ "$(DISPLAY=$2 xclip -o -selection clipboard 2>/dev/null)" = "$1" ]; do sleep 0.1; done' _ "$sentinel" "$display"; then
  echo "smoke-desktop-linux: clipboard not restored after the paste: $(DISPLAY=$display xclip -o -selection clipboard 2>/dev/null | head -c 80)"; exit 1
fi
echo "smoke-desktop-linux: pasted via $(voltip_smoke_plain "$data/app.log" | grep -oE 'pasted .*tool=[a-z0-9-]+' | grep -oE 'tool=[a-z0-9-]+' | head -1 || echo 'tool=?'), clipboard restored to the sentinel"
# The outcome dwells 2.5 s (a failure that keeps its text 6 s), then the core goes idle and the pill hides.
if ! timeout 15 sh -c 'until [ "$( (DISPLAY=:99 xdotool search --onlyvisible --name "^Voltip Overlay$" || true) | wc -l)" = "0" ]; do sleep 0.5; done'; then
  echo "smoke-desktop-linux: pill did not hide after the outcome dwell"; exit 1
fi
echo "smoke-desktop-linux: overlay visible while held=$overlay_visible, hidden after outcome=yes"
if [ "$overlay_visible" != "1" ]; then
  echo "smoke-desktop-linux: hotkey did not drive the overlay window"
  echo "--- app.log (full) ---"; sed -E 's/\x1b\[[0-9;]*m//g' "$data/app.log"
  echo "--- X windows ---"; DISPLAY=$display xdotool search --name 'Voltip' 2>/dev/null | while read -r w; do DISPLAY=$display xwininfo -id "$w" | grep -E "Window id|Map State|geometry"; done
  echo "--- keymap ---"; DISPLAY=$display setxkbmap -print 2>&1 | head -8; DISPLAY=$display xmodmap -pke 2>/dev/null | grep -E " (space|Control_L|Alt_L)( |$)" | head -3
  exit 1
fi
# Esc cancels a running take (docs/dictation.md §5): hold the hotkey until the take listens and
# the cancel key is registered, press Escape, release. The core reports `cancelled`, the recording
# never reaches the ASR, and Esc goes back to the other applications once the take is over.
asr_before=$(wc -l <"$data/asr.log" 2>/dev/null || echo 0)
cancels_before=$(voltip_smoke_plain "$data/app.log" | grep -c 'phase="cancelled"' || true)
releases_before=$(voltip_smoke_plain "$data/app.log" | grep -c 'cancel key released' || true)
registers_before=$(voltip_smoke_plain "$data/app.log" | grep -c 'cancel key registered for the take' || true)
DISPLAY=$display xdotool keydown ctrl+alt+space
if ! timeout 10 sh -c 'until [ "$(sed -E "s/\x1b\[[0-9;]*m//g" "$1" | grep -c "cancel key registered for the take")" -gt "$2" ]; do sleep 0.1; done' _ "$data/app.log" "$registers_before"; then
  DISPLAY=$display xdotool keyup ctrl+alt+space
  echo "smoke-desktop-linux: the second take never registered the cancel key"; voltip_smoke_plain "$data/app.log" | grep -iE "cancel key|dictation phase" | tail -6; exit 1
fi
DISPLAY=$display xdotool key Escape
if ! timeout 10 sh -c 'until [ "$(sed -E "s/\x1b\[[0-9;]*m//g" "$1" | grep -c "phase=\"cancelled\"")" -gt "$2" ]; do sleep 0.1; done' _ "$data/app.log" "$cancels_before"; then
  DISPLAY=$display xdotool keyup ctrl+alt+space
  echo "smoke-desktop-linux: Escape did not cancel the take"; voltip_smoke_plain "$data/app.log" | grep -iE "cancel|dictation phase" | tail -6; exit 1
fi
DISPLAY=$display xdotool keyup ctrl+alt+space
if ! timeout 10 sh -c 'until [ "$(sed -E "s/\x1b\[[0-9;]*m//g" "$1" | grep -c "cancel key released")" -gt "$2" ]; do sleep 0.1; done' _ "$data/app.log" "$releases_before"; then
  echo "smoke-desktop-linux: the cancel key stayed registered after the take"; exit 1
fi
asr_after=$(wc -l <"$data/asr.log" 2>/dev/null || echo 0)
if [ "$asr_after" != "$asr_before" ]; then
  echo "smoke-desktop-linux: a cancelled take still reached the ASR ($asr_before → $asr_after requests)"; exit 1
fi
echo "smoke-desktop-linux: Escape cancelled the take, nothing sent, cancel key released"
grep -iE "dictation" "$data/app.log" | sed -E 's/\x1b\[[0-9;]*m//g' | tail -4
if ! kill -0 "$app_pid" 2>/dev/null; then
  echo "smoke-desktop-linux: app exited early"; cat "$data/app.log"; exit 1
fi
echo "smoke-desktop-linux: window $geometry"
grep -E "identity|error" "$data/app.log" | sed -E 's/\x1b\[[0-9;]*m//g' | head -3 || true
grep -iE "global hotkey registered|overlay window prewarmed" "$data/app.log" | sed -E 's/\x1b\[[0-9;]*m//g' | head -2 || true
echo "smoke-desktop-linux: OK → ${out%.png}-first-run.png, $out, ${out%.png}-devices.png, ${out%.png}-hotkey-overlay.png, ${out%.png}-dictation-outcome.png"
