#!/usr/bin/env bash
# A private PulseAudio server for the audio measurements and the real-device tests
# (docs/dictation.md §30.4): its own runtime directory and socket, a null sink as the default output
# and its monitor as the default input. Nothing is heard, and no other session's sound server is
# touched: clients reach this one only through the PULSE_SERVER it prints.
#
#   scripts/measure/private-pulse.sh start <dir>   # prints PULSE_SERVER=unix:<dir>/native
#   scripts/measure/private-pulse.sh stop <dir>    # ends the server and everything it started
#
# The server runs under `timeout` (two hours at most) in a session of its own, so `stop` ends it
# by its process group, and a forgotten one ends by itself.
set -euo pipefail

cmd=${1:?usage: private-pulse.sh start|stop <dir>}
dir=${2:?usage: private-pulse.sh start|stop <dir>}

case "$cmd" in
start)
  if [ -f "$dir/pgid" ] && kill -0 -- "-$(cat "$dir/pgid")" 2>/dev/null; then
    echo "PULSE_SERVER=unix:$dir/native"
    exit 0
  fi
  mkdir -p "$dir/runtime" "$dir/state"
  chmod 700 "$dir/runtime"
  cat >"$dir/default.pa" <<EOF
load-module module-native-protocol-unix socket=$dir/native auth-anonymous=1
load-module module-null-sink sink_name=voltip_null rate=48000 channels=2 sink_properties=device.description=VoltipNull
set-default-sink voltip_null
set-default-source voltip_null.monitor
EOF
  rm -f "$dir/native"
  # HOME and the runtime paths point into <dir>: the cookie and the state stay there too.
  HOME="$dir" XDG_RUNTIME_DIR="$dir/runtime" PULSE_RUNTIME_PATH="$dir/runtime" PULSE_STATE_PATH="$dir/state" \
    setsid timeout --foreground 7200 pulseaudio -n -F "$dir/default.pa" --daemonize=no --exit-idle-time=-1 \
    --use-pid-file=no --disable-shm=yes --log-target="file:$dir/pulse.log" </dev/null >/dev/null 2>&1 &
  echo $! >"$dir/pgid"
  if ! timeout 20 sh -c 'until [ -S "$1/native" ]; do sleep 0.2; done' _ "$dir"; then
    echo "the private PulseAudio server did not start; see $dir/pulse.log" >&2
    kill -- "-$(cat "$dir/pgid")" 2>/dev/null || true
    exit 1
  fi
  echo "PULSE_SERVER=unix:$dir/native"
  ;;
stop)
  if [ -f "$dir/pgid" ]; then
    pgid=$(cat "$dir/pgid")
    kill -- "-$pgid" 2>/dev/null || true
    timeout 10 sh -c 'while kill -0 -- "-$1" 2>/dev/null; do sleep 0.2; done' _ "$pgid" || kill -9 -- "-$pgid" 2>/dev/null || true
    rm -f "$dir/pgid" "$dir/native"
  fi
  ;;
*)
  echo "usage: private-pulse.sh start|stop <dir>" >&2
  exit 2
  ;;
esac
