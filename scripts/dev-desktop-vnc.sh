#!/usr/bin/env bash
# Interactive desktop for manual testing on a headless Linux host: an xfce session on a TigerVNC
# display, reachable from a browser through noVNC, with `make desktop-dev` (Vite + `cargo tauri
# dev`, hot reload) running in a terminal window inside it and a private PulseAudio whose null
# sink is the microphone (play a sample into it while holding the hotkey).
#
#   scripts/dev-desktop-vnc.sh start [--no-app]   start everything (idempotent) and print the URL
#   scripts/dev-desktop-vnc.sh speak <wav>        play a WAV into the fake microphone
#   scripts/dev-desktop-vnc.sh status | stop
#
# Security: Xvnc listens on localhost only. The browser endpoint (websockify + noVNC) listens on
# VOLTIP_VNC_BIND (default: this host's first non-loopback IPv4) and every connection needs the
# VNC password, generated once into ~/.config/voltip-dev/ (mode 600). The session is a full desktop
# with a terminal: anyone with the password gets a shell as this user. Keep the host on a private
# network, or set VOLTIP_VNC_BIND=127.0.0.1 and reach it with `ssh -L 6080:127.0.0.1:6080 <host>`.
#
# Needs: tigervnc-standalone-server tigervnc-tools novnc websockify xfce4-session xfwm4 xfce4-panel
# xfdesktop4 xfce4-terminal dbus-x11 pulseaudio pulseaudio-utils (see docs/runbook.md 本地开发).
set -euo pipefail
cd "$(dirname "$0")/.."
repo=$(pwd)

display=${VOLTIP_VNC_DISPLAY:-:7}
geometry=${VOLTIP_VNC_GEOMETRY:-1600x1000}
web_port=${VOLTIP_VNC_WEB_PORT:-6080}
bind=${VOLTIP_VNC_BIND:-$(ip -4 -o addr show scope global 2>/dev/null | awk '{sub(/\/.*/, "", $4); print $4; exit}')}
bind=${bind:-127.0.0.1}
state=${XDG_RUNTIME_DIR:-/tmp}/voltip-dev-vnc
conf=$HOME/.config/voltip-dev
vnc_port=$((5900 + ${display#:}))
mkdir -p "$state" "$conf"
chmod 700 "$state" "$conf"

running() { [ -f "$state/$1.pid" ] && kill -0 "$(cat "$state/$1.pid")" 2>/dev/null; }

need() {
  local missing=()
  for tool in Xvnc vncpasswd websockify startxfce4 dbus-launch xfce4-terminal pulseaudio pactl paplay; do
    command -v "$tool" >/dev/null || missing+=("$tool")
  done
  [ -d /usr/share/novnc ] || missing+=(novnc)
  if [ ${#missing[@]} -gt 0 ]; then
    echo "dev-desktop-vnc: missing ${missing[*]}" >&2
    echo "  sudo apt-get install -y --no-install-recommends tigervnc-standalone-server tigervnc-tools novnc websockify xfce4-session xfwm4 xfce4-panel xfdesktop4 xfce4-terminal dbus-x11 pulseaudio pulseaudio-utils" >&2
    exit 2
  fi
}

password() {
  if [ ! -s "$conf/vncpasswd" ]; then
    # 8 characters: the VNC password scheme uses at most 8.
    head -c 64 /dev/urandom | tr -dc 'A-Za-z0-9' | head -c 8 >"$conf/password"
    vncpasswd -f <"$conf/password" >"$conf/vncpasswd"
    chmod 600 "$conf/password" "$conf/vncpasswd"
  fi
  cat "$conf/password"
}

start() {
  local with_app=1
  [ "${1:-}" = "--no-app" ] && with_app=0
  need
  local pass
  pass=$(password)

  if ! running xvnc; then
    Xvnc "$display" -geometry "$geometry" -depth 24 -localhost yes -rfbport "$vnc_port" \
      -SecurityTypes VncAuth -PasswordFile "$conf/vncpasswd" -AlwaysShared -desktop "Voltip dev" \
      >"$state/xvnc.log" 2>&1 &
    echo $! >"$state/xvnc.pid"
    timeout 20 sh -c "until DISPLAY=$display xdpyinfo >/dev/null 2>&1; do sleep 0.3; done" || { tail -5 "$state/xvnc.log" >&2; exit 1; }
  fi

  # Private PulseAudio: a null sink whose monitor is the default source (the "microphone").
  export PULSE_RUNTIME_PATH=$state/pulse PULSE_SERVER=unix:$state/pulse/native
  if ! running pulse; then
    mkdir -p "$PULSE_RUNTIME_PATH" && chmod 700 "$PULSE_RUNTIME_PATH"
    pulseaudio --daemonize=no --exit-idle-time=-1 -n --log-target="file:$state/pulseaudio.log" \
      --load=module-native-protocol-unix \
      --load="module-null-sink sink_name=voltip_dev_mic sink_properties=device.description=Voltip-dev-mic" >/dev/null 2>&1 &
    echo $! >"$state/pulse.pid"
    timeout 15 sh -c 'until pactl info >/dev/null 2>&1; do sleep 0.2; done' || { tail -3 "$state/pulseaudio.log" >&2; exit 1; }
    pactl set-default-source voltip_dev_mic.monitor
  fi

  if ! running session; then
    DISPLAY=$display PULSE_SERVER=$PULSE_SERVER PULSE_RUNTIME_PATH=$PULSE_RUNTIME_PATH \
      dbus-launch --exit-with-session startxfce4 >"$state/session.log" 2>&1 &
    echo $! >"$state/session.pid"
  fi

  if ! running web; then
    websockify --web /usr/share/novnc "$bind:$web_port" "127.0.0.1:$vnc_port" >"$state/websockify.log" 2>&1 &
    echo $! >"$state/web.pid"
    timeout 15 sh -c "until curl -sf -o /dev/null http://$bind:$web_port/vnc.html; do sleep 0.3; done" || { tail -5 "$state/websockify.log" >&2; exit 1; }
  fi

  if [ "$with_app" = 1 ] && ! pgrep -f "voltip-dev-app" >/dev/null; then
    # A terminal in the session runs the app so its logs stay visible; closing it stops the app.
    # VOLTIP_DEV_SECRET_STORE=memory: no Secret Service in this session (debug builds only).
    DISPLAY=$display PULSE_SERVER=$PULSE_SERVER PULSE_RUNTIME_PATH=$PULSE_RUNTIME_PATH \
      xfce4-terminal --title "voltip-dev-app" --working-directory "$repo" \
      -x bash -lc 'export VOLTIP_DEV_SECRET_STORE=memory; make desktop-dev; echo; echo "desktop-dev exited ($?) — press Enter to close"; read -r _' \
      >"$state/app.log" 2>&1 &
  fi

  echo "dev-desktop-vnc: open http://$bind:$web_port/vnc.html?autoconnect=1&resize=remote"
  echo "dev-desktop-vnc: password $pass   (also in $conf/password)"
  echo "dev-desktop-vnc: microphone = null sink 'voltip_dev_mic'; feed it with: $0 speak <wav>"
}

speak() {
  local wav=${1:?usage: $0 speak <wav>}
  export PULSE_RUNTIME_PATH=$state/pulse PULSE_SERVER=unix:$state/pulse/native
  running pulse || { echo "dev-desktop-vnc: not started" >&2; exit 1; }
  paplay --device=voltip_dev_mic "$wav"
}

status() {
  for p in xvnc pulse session web; do
    if running "$p"; then echo "$p: running (pid $(cat "$state/$p.pid"))"; else echo "$p: stopped"; fi
  done
  pgrep -fa 'voltip-desktop|tauri dev' | cut -c1-120 || true
}

stop() {
  for p in web session pulse xvnc; do
    if running "$p"; then kill "$(cat "$state/$p.pid")" 2>/dev/null || true; fi
    rm -f "$state/$p.pid"
  done
  echo "dev-desktop-vnc: stopped (the app terminal closes with the session)"
}

case "${1:-start}" in
  start) shift || true; start "$@" ;;
  speak) shift; speak "$@" ;;
  status) status ;;
  stop) stop ;;
  *) echo "usage: $0 start [--no-app] | speak <wav> | status | stop" >&2; exit 2 ;;
esac
