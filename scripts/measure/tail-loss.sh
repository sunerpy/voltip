#!/usr/bin/env bash
# shellcheck disable=SC2016  # the `sh -c '…'` wait loops read their arguments as $1 inside the child shell
# Lost first and last words through the real desktop app (docs/dictation.md §30.4): Xvfb on a free
# display, a private PulseAudio server (scripts/measure/private-pulse.sh) whose null sink is the
# microphone, a private session bus (`--toggle` reaches the app over D-Bus), and tail-loss.py
# driving the takes with `voltip-desktop --toggle` or the global shortcut through XTEST. Nothing is
# heard and no other session's display, sound server or bus is touched; everything ends with the
# script.
#
#   scripts/measure/tail-loss.sh <work-dir> tail  --binary <voltip-desktop> [tail-loss.py options]
#   scripts/measure/tail-loss.sh <work-dir> first --binary <voltip-desktop> --label NAME [options]
#
# <work-dir> holds sentences.tsv (tail-loss.py prepare) and gets a data-<mode>.* folder per run (the
# app's settings, history and log), and takes-<mode>.tsv. The binary is a debug build (`cargo build -p voltip-desktop --features custom-protocol`) with
# its sherpa-onnx libraries beside it.
set -euo pipefail
cd "$(dirname "$0")/../.."

work=${1:?usage: tail-loss.sh <work-dir> tail|first --binary B [...]}
mode=${2:?usage: tail-loss.sh <work-dir> tail|first --binary B [...]}
shift 2
for tool in Xvfb xdotool xdpyinfo pulseaudio pactl paplay pacat dbus-run-session; do
  command -v "$tool" >/dev/null || { echo "tail-loss: $tool not installed" >&2; exit 2; }
done

# A display nobody uses: no lock file and no Xvfb on it.
display=""
for n in $(seq 120 160); do
  if [ ! -e "/tmp/.X$n-lock" ] && [ ! -e "/tmp/.X11-unix/X$n" ]; then display=":$n"; break; fi
done
[ -n "$display" ] || { echo "tail-loss: no free display between :120 and :160" >&2; exit 2; }

pulse="$work/pulse-$mode"
xvfb_pgid=""
keepalive_pgid=""
cleanup() {
  [ -n "$keepalive_pgid" ] && kill -- "-$keepalive_pgid" 2>/dev/null || true
  [ -n "$xvfb_pgid" ] && kill -- "-$xvfb_pgid" 2>/dev/null || true
  scripts/measure/private-pulse.sh stop "$pulse" || true
}
trap cleanup EXIT INT TERM

setsid timeout --foreground 14400 Xvfb "$display" -screen 0 1280x800x24 >/dev/null 2>&1 &
xvfb_pgid=$!
timeout 20 sh -c 'until DISPLAY=$1 xdpyinfo >/dev/null 2>&1; do sleep 0.2; done' _ "$display"
eval "$(scripts/measure/private-pulse.sh start "$pulse" | sed -n 's/^PULSE_SERVER=/export PULSE_SERVER=/p')"
export DISPLAY=$display
# Silence on a 10 ms stream for the whole run. An idle null sink renders two seconds ahead, and
# whatever plays or records next starts behind that block: the app's first samples came 1.9 s
# after its device opened and a release dropped the sentence's last 1.9 s. A sound card plays and
# records as the samples go; with this stream the sink does too (first samples within 30 ms).
setsid timeout --foreground 14400 pacat --playback --latency-msec=10 --raw --format=s16le --rate=48000 --channels=2 /dev/zero &
keepalive_pgid=$!

# Each run keeps its own data folder, the app's log among it.
data=$(mktemp -d "$work/data-$mode.XXXXXX")
# The session bus the environment names may be another session's, or gone: the app gets its own,
# which ends with tail-loss.py.
dbus-run-session -- /usr/bin/python3 -I scripts/measure/tail-loss.py "$mode" --data "$data" --sentences "$work/sentences.tsv" --out "$work/takes-$mode.tsv" "$@"
