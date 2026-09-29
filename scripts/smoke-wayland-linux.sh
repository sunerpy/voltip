#!/usr/bin/env bash
# Pure-Wayland smoke of the real desktop app (docs/dictation.md §14): no X server anywhere, the app
# is a native Wayland client, and `voltip-desktop --toggle` stands in for the hotkey (Wayland lets
# no application grab one).
#
# Two headless compositors, because neither shows everything on its own:
#   weston  headless-backend.so, the reference compositor, no XWayland. Checks: the app starts as a
#           Wayland client, `HotkeyStatus.backend` is `global-shortcut · Linux · Wayland`, the hotkey
#           is refused with the `--toggle` hint, the injector names the session. Headless weston has
#           no wl_seat, hence no clipboard and no keyboard: a take there must end in a typed
#           injection failure (recorded, never a hang, never a fake success).
#   sway    wlroots headless: wl_seat, zwp_virtual_keyboard_v1, wlr-data-control. Checks: two
#           `--toggle` calls run a whole take (fake microphone + mock ASR, scripts/lib/
#           smoke-dictation.sh), the paste goes out through `wtype` (a PATH shim logs the call), the
#           take is recorded as inserted via paste, and the clipboard holds the sentinel again.
# Neither is a desktop: KWin (kwtype), Mutter (XWayland XTEST) and the GlobalShortcuts portal stay
# manual checks.
#
# Usage: scripts/smoke-wayland-linux.sh [out-dir]      (default docs/acceptance/screens/tauri)
# Writes <out>/desktop-linux-wayland-summary.txt, <out>/desktop-linux-wayland-sway.png (grim; headless
# weston cannot be captured) and <out>/desktop-linux-wayland-{weston,sway}-app.log.
# VOLTIP_SMOKE_NO_BUILD=1 reuses target/debug/voltip-desktop instead of rebuilding.
# shellcheck disable=SC2016,SC2012  # `sh -c '…' _ "$x"` expands in the inner shell; ls lists our own sockets
set -euo pipefail
cd "$(dirname "$0")/.."
. scripts/lib/build-env.sh && voltip_load_build_env
. scripts/lib/smoke-dictation.sh
. scripts/lib/head-commit.sh

out_dir=${1:-docs/acceptance/screens/tauri}
summary=$out_dir/desktop-linux-wayland-summary.txt
bin=$PWD/target/debug/voltip-desktop
spoken="Voltip Wayland 冒烟"
sentinel="voltip-wayland-sentinel-$$"
for tool in weston sway wtype wl-copy wl-paste python3 dbus-daemon; do
  command -v "$tool" >/dev/null || { echo "smoke-wayland-linux: $tool not installed (sudo apt-get install -y weston sway wtype wl-clipboard)"; exit 2; }
done
if [ -z "${VOLTIP_SMOKE_NO_BUILD:-}" ]; then
  pnpm --filter @voltip/desktop run build >/dev/null
  cargo build -q -p voltip-desktop --features custom-protocol
fi
[ -x "$bin" ] || { echo "smoke-wayland-linux: $bin missing"; exit 2; }
mkdir -p "$out_dir"

work=$(mktemp -d)
compositor_pid="" app_pid="" bus_pid="" failures=0
# Snapshot the binary (hard links, same filesystem as target/): a concurrent `cargo build` / `cargo
# test` relinks target/debug/voltip-desktop, and a run must not switch binaries half-way. The
# sherpa-onnx libraries come along because the binary finds them through `$ORIGIN`.
snap=$PWD/target/smoke-wayland-bin-$$
mkdir -p "$snap"
for f in "$bin" "$(dirname "$bin")"/libsherpa-onnx-c-api.so "$(dirname "$bin")"/libonnxruntime.so; do
  [ -e "$f" ] && { ln "$f" "$snap/" 2>/dev/null || cp "$f" "$snap/"; }
done
bin=$snap/$(basename "$bin")
cleanup() {
  set +e
  voltip_smoke_stop "$app_pid" "$compositor_pid" "${VOLTIP_SMOKE_ASR_PID:-}"
  voltip_smoke_pulse_mic_stop
  voltip_smoke_stop "$bus_pid"
  rm -rf "$work" "$snap"
}
trap cleanup EXIT
# A private session bus: `--toggle` reaches the running instance through the single-instance
# plugin's D-Bus name, which must not meet a Voltip the user is running. Forked with its stdio on
# /dev/null, so the portal services GTK activates on it stay quiet.
bus=$(timeout 10 dbus-daemon --session --fork --print-address=1 --print-pid=1) || { echo "smoke-wayland-linux: private dbus-daemon did not start"; exit 2; }
DBUS_SESSION_BUS_ADDRESS=$(printf '%s\n' "$bus" | sed -n 1p)
bus_pid=$(printf '%s\n' "$bus" | sed -n 2p)
export DBUS_SESSION_BUS_ADDRESS

note() { echo "$*" | tee -a "$summary"; }
check() { # <description> <command…>: record PASS / FAIL, keep going
  local what=$1; shift
  if "$@"; then note "  PASS  $what"; else note "  FAIL  $what"; failures=$((failures + 1)); fi
}
# grep reads to the end: `grep -q` under pipefail let sed die of SIGPIPE on an early match and
# the check fail ("sed: couldn't flush stdout: Broken pipe").
log_has() { voltip_smoke_plain "$1" | grep -E -- "$2" >/dev/null; }
# Condition wait on the app log with a deadline; fails early when the app died.
wait_log() { # <log> <regex> <seconds>
  local deadline=$(($(date +%s) + $3))
  until log_has "$1" "$2"; do
    kill -0 "$app_pid" 2>/dev/null || return 2
    [ "$(date +%s)" -lt "$deadline" ] || return 1
    sleep 0.3
  done
}
# A shim directory first on the app's PATH: every wtype call is logged, then the real one runs.
shim=$work/shim
mkdir -p "$shim"
printf '#!/bin/sh\nprintf "%%s\\n" "$*" >> "%s/wtype-calls.log"\nexec %s "$@"\n' "$work" "$(command -v wtype)" >"$shim/wtype"
chmod +x "$shim/wtype"

start_app() { # <phase> [extra env…]
  local phase=$1; shift
  local dir=$work/$phase
  mkdir -p "$dir"
  voltip_smoke_settings "$dir/data/voltip/settings.json" "$VOLTIP_SMOKE_ASR_URL" paste
  env -u DISPLAY "$@" PATH="$shim:$PATH" XDG_SESSION_TYPE=wayland GDK_BACKEND=wayland VOLTIP_DEV_SECRET_STORE=memory \
    XDG_DATA_HOME="$dir/data" XDG_CONFIG_HOME="$dir/config" RUST_LOG=voltip=info,voltip_inject=debug \
    "$bin" >"$dir/app.log" 2>&1 &
  app_pid=$!
}
toggle() { # <phase> [extra env…]: the second instance forwards `--toggle` over D-Bus and exits
  local phase=$1; shift
  local dir=$work/$phase
  env -u DISPLAY "$@" PATH="$shim:$PATH" XDG_SESSION_TYPE=wayland GDK_BACKEND=wayland VOLTIP_DEV_SECRET_STORE=memory \
    XDG_DATA_HOME="$dir/data" XDG_CONFIG_HOME="$dir/config" RUST_LOG=voltip=info \
    timeout 60 "$bin" --toggle >>"$dir/toggle.log" 2>&1
}
# One take: toggle on, speak for 3 s (the length of the recording, not a wait), toggle off.
take() { # <phase> [extra env…]
  local phase=$1; shift
  local log=$work/$phase/app.log
  toggle "$phase" "$@" || { note "  FAIL  first --toggle exited non-zero"; return 1; }
  wait_log "$log" 'phase="listening"' 20 || { note "  FAIL  --toggle did not start listening"; return 1; }
  voltip_smoke_speak "$work/tone.wav"
  sleep 3
  toggle "$phase" "$@" || { note "  FAIL  second --toggle exited non-zero"; return 1; }
  wait_log "$log" 'phase="(done|failed|cancelled)"' 60 || { note "  FAIL  the take never finished"; return 1; }
}
finish_phase() { # <phase>: stop the app and the compositor, keep the app log
  local phase=$1
  # `wait` reports the SIGTERM (143) it just delivered; that is the expected end, not a failure.
  if [ -n "$app_pid" ]; then kill "$app_pid" 2>/dev/null; wait "$app_pid" 2>/dev/null || true; fi
  app_pid=""
  if [ -n "$compositor_pid" ]; then kill "$compositor_pid" 2>/dev/null; wait "$compositor_pid" 2>/dev/null || true; fi
  compositor_pid=""
  voltip_smoke_plain "$work/$phase/app.log" >"$out_dir/desktop-linux-wayland-$phase-app.log" 2>/dev/null || true
}

: >"$summary"
note "smoke-wayland-linux @ $(voltip_head_commit) (+ uncommitted changes, if any) on $(uname -srm)"
note "tools: $(weston --version 2>&1 | head -1); sway $(sway --version | awk '{print $3}'); wtype $(dpkg-query -W -f='${Version}' wtype 2>/dev/null || echo '?')"
# A build without `custom-protocol` loads the dev server instead of the bundled pages; the checks
# below do not depend on the page, the screenshot does.
if strings -n 8 "$bin" | grep -E '/assets/index-[A-Za-z0-9_-]+\.js' >/dev/null; then frontend=bundled; else frontend="dev server (no custom-protocol: the screenshot shows an error page)"; fi
note "binary: $(stat -c %s "$bin") bytes, frontend $frontend"
voltip_smoke_tone_wav "$work/tone.wav" 8
voltip_smoke_mock_asr_start "$work" "$spoken"
voltip_smoke_pulse_mic_start "$work" "voltip_wayland_$$"
unset DISPLAY

# ---- weston ------------------------------------------------------------------------------------
note ""
note "[weston headless, no XWayland]"
export XDG_RUNTIME_DIR=$work/run-weston
mkdir -m 700 "$XDG_RUNTIME_DIR"
weston --backend=headless-backend.so --socket=voltip-smoke --idle-time=0 --width=1280 --height=800 >"$work/weston.log" 2>&1 &
compositor_pid=$!
timeout 20 sh -c 'until [ -S "$1" ]; do sleep 0.2; done' _ "$XDG_RUNTIME_DIR/voltip-smoke" || { note "  FAIL  weston did not start: $(tail -3 "$work/weston.log")"; exit 1; }
export WAYLAND_DISPLAY=voltip-smoke
start_app weston
log=$work/weston/app.log
# Both chords are refused (dictation, then voice edit); wait for the second line.
if wait_log "$log" 'no global hotkey on this session.* purpose="edit"' 90; then
  check "session judged in the log (linux session session=Wayland · …)" log_has "$log" 'linux session session=Wayland · '
  check "HotkeyStatus.backend = global-shortcut · Linux · Wayland" log_has "$log" 'no global hotkey on this session.* backend=global-shortcut · Linux · Wayland'
  check "HotkeyStatus.error names the --toggle binding" log_has "$log" 'purpose="dictation" .*error=.*纯 Wayland.*--toggle'
  check "HotkeyStatus.edit_error names the --edit-toggle binding" log_has "$log" 'purpose="edit" .*error=.*纯 Wayland.*--edit-toggle'
  check "injector judged the session (text injection backend backend=Wayland · …)" log_has "$log" 'text injection backend backend=Wayland · '
  note "  info  $(voltip_smoke_plain "$log" | grep -oE 'text injection backend backend=[^=]*options' | sed 's/ options$//' | head -1)"
  # No screenshot here: weston-screenshooter 13 aborts on the headless output
  # (`screenshot_create_shm_buffer: Assertion width > 0`); the sway phase captures with grim.
  if take weston; then
    outcome=$(voltip_smoke_last_history "$work/weston/data/voltip/history.sqlite3")
    note "  info  take on seatless weston → history: ${outcome:-<none>}"
    check "a take on a seatless compositor ends typed, not as a fake paste" test "$(printf '%s' "$outcome" | cut -f1-2)" != "$(printf 'inserted\tpaste')"
  else
    failures=$((failures + 1))
  fi
else
  note "  FAIL  the app never reached the hotkey decision on weston (needs a real compositor?)"
  voltip_smoke_plain "$log" | grep -E " (WARN|ERROR) |panicked|Gdk-|WebKit" | tail -8 | tee -a "$summary"
  failures=$((failures + 1))
fi
finish_phase weston

# ---- sway --------------------------------------------------------------------------------------
note ""
note "[sway headless (wlroots), no XWayland]"
export XDG_RUNTIME_DIR=$work/run-sway
mkdir -m 700 "$XDG_RUNTIME_DIR"
unset WAYLAND_DISPLAY
printf 'output HEADLESS-1 resolution 1280x800\nxwayland disable\n' >"$work/sway.conf"
WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 sway -c "$work/sway.conf" >"$work/sway.log" 2>&1 &
compositor_pid=$!
timeout 20 sh -c 'until ls "$1"/wayland-? >/dev/null 2>&1; do sleep 0.2; done' _ "$XDG_RUNTIME_DIR" || { note "  FAIL  sway did not start: $(tail -3 "$work/sway.log")"; exit 1; }
WAYLAND_DISPLAY=$(basename "$(ls "$XDG_RUNTIME_DIR"/wayland-? | head -1)")
export WAYLAND_DISPLAY
printf '%s' "$sentinel" | wl-copy
start_app sway XDG_CURRENT_DESKTOP=sway
log=$work/sway/app.log
if wait_log "$log" 'no global hotkey on this session' 90; then
  check "session judged as Wayland · wlroots" log_has "$log" 'linux session session=Wayland · wlroots'
  check "HotkeyStatus.backend = global-shortcut · Linux · Wayland" log_has "$log" 'no global hotkey on this session.* backend=global-shortcut · Linux · Wayland'
  check "injector picks wtype first (text injection backend … → wtype)" log_has "$log" 'text injection backend backend=Wayland · wlroots → wtype'
  if take sway XDG_CURRENT_DESKTOP=sway; then
    outcome=$(voltip_smoke_last_history "$work/sway/data/voltip/history.sqlite3")
    note "  info  history: ${outcome:-<none>}"
    check "the take is recorded as inserted via paste with the ASR text" test "$outcome" = "$(printf 'inserted\tpaste\t%s' "$spoken")"
    check "the paste went through wtype (tool=wtype in the injector log)" log_has "$log" 'pasted .*tool=wtype'
    check "wtype was run with the Ctrl+V chord" grep -qx -- '-M ctrl -k v -m ctrl' "$work/wtype-calls.log"
    restored=1
    timeout 5 sh -c 'until [ "$(wl-paste -n 2>/dev/null)" = "$1" ]; do sleep 0.1; done' _ "$sentinel" || restored=0
    note "  info  clipboard after the take: $(wl-paste -n 2>/dev/null | head -c 80)"
    check "clipboard restored to the sentinel after the paste" test "$restored" = 1
  else
    failures=$((failures + 1))
  fi
  if command -v grim >/dev/null && grim "$out_dir/desktop-linux-wayland-sway.png" 2>/dev/null; then
    note "  info  screenshot → $out_dir/desktop-linux-wayland-sway.png"
  fi
else
  note "  FAIL  the app never reached the hotkey decision on sway"
  voltip_smoke_plain "$log" | grep -E " (WARN|ERROR) |panicked|Gdk-|WebKit" | tail -8 | tee -a "$summary"
  failures=$((failures + 1))
fi
finish_phase sway

note ""
note "mock ASR requests: $(wc -l <"$work/asr.log" 2>/dev/null || echo 0); wtype calls: $(wc -l <"$work/wtype-calls.log" 2>/dev/null || echo 0)"
if [ "$failures" -gt 0 ]; then
  note "smoke-wayland-linux: FAILED ($failures) → $summary"
  exit 1
fi
note "smoke-wayland-linux: OK → $summary"
