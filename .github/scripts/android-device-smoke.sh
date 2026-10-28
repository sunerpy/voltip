#!/usr/bin/env bash
# The Android app on a device: install the APK, start it, check that it comes up and stays up, the
# first thing every user does. 0.0.18 and 0.0.19 closed at once on the user's phone (a Xiaomi
# HyperOS 3, 2026-10-01) although every package check passed; nothing had started the app. CI runs
# this against an emulator (`android-device` in ci.yml); it runs the same against a phone over adb.
#
# The app is the React Native one since 0.0.50 (docs/mobile-rn.md §6), under the package the Tauri
# phone app had: installed, first screen up, still up 20 s later, nothing fatal in its log. Its back
# navigation is React Navigation's own, which App.test.tsx and the Device Farm acceptance
# (apps/mobile-rn/devicefarm) walk.
#
# Then, on an emulator only (ro.kernel.qemu is 1) or with VOLTIP_SMOKE_LARGE_SCREEN=1, a large
# screen (Google Play's Android 16 notice, docs/mobile-rn.md §5): the display becomes a 2560 × 1600
# tablet at density 320 (1280 × 800 dp, its smallest width beyond the 600 dp Android 16 counts as
# large), with auto-rotation off in landscape and then in portrait; each time the first screen is
# still there and the app stays up without a fatal line. The display size, density and both
# rotation settings are read first and put back on every way out (an EXIT trap set before the
# first change). On a phone this part does not run at all, so the script changes no phone's display.
#
# Usage: android-device-smoke.sh <apk or directory holding one> <out dir>
# A directory is searched for the app's release name (Voltip_*.apk): the release candidate's
# Android leg holds the APK and the AAB. Needs `adb` on PATH with one device online.
# Writes into <out>: install.txt, start.txt, logcat.txt, crash.txt, events.txt, exit-info.txt,
# ui.xml and screen.png, whatever the outcome, app-logcat.txt (the app's process alone) when it
# came up, and app-logcat-large-screen.txt after the large screen.
set -euo pipefail

package=dev.voltip.mobile name='Voltip_*.apk'
if [ "$#" -ne 2 ] || [ "${1:0:1}" = - ]; then
  echo "usage: $0 <apk or directory> <out dir>" >&2
  exit 2
fi
apk=$1 out=$2
if [ -d "$apk" ]; then
  found=$(find "$apk" -name "$name" | sort)
  [ "$(grep -c . <<<"$found")" -le 1 ] || { echo "android-device-smoke: more than one $name in $1" >&2; exit 2; }
  apk=$found
fi
[ -f "$apk" ] || { echo "android-device-smoke: no APK for $package at $1" >&2; exit 2; }
mkdir -p "$out"

collect() {
  adb logcat -d >"$out/logcat.txt" 2>&1 || true
  adb logcat -b crash -d >"$out/crash.txt" 2>&1 || true
  adb logcat -b events -d >"$out/events.txt" 2>&1 || true
  # Android 11+ keeps why the process last ended (a crash, a native crash, an exit code).
  adb shell dumpsys activity exit-info "$package" >"$out/exit-info.txt" 2>&1 || true
  adb exec-out screencap -p >"$out/screen.png" 2>/dev/null || true
}
fail() {
  collect
  echo "::error title=Android device::$*" >&2
  echo "--- crash buffer" >&2
  head -60 "$out/crash.txt" >&2 || true
  echo "--- the app's fatal lines" >&2
  grep -E "FATAL EXCEPTION|AndroidRuntime|panicked at|Fatal signal|$package" "$out/logcat.txt" | head -60 >&2 || true
  exit 1
}
running() {
  [ -n "$(adb shell pidof "$package" 2>/dev/null | tr -d '\r')" ]
}

# The screen's UI tree into $out/ui.xml. A not-responding dialog over the app hides it from the
# dump: on a busy, freshly booted emulator the launcher can stop answering, and while it stays
# stuck its dialog comes back every few seconds and takes the taps meant for the app (CI runs
# 37098351522 and 37109103623). Such a dialog for another app closes that app (the system starts
# the launcher again when it needs it); one for Voltip is a failure.
dump() {
  local title xy
  for _ in 1 2 3 4 5; do
    adb shell uiautomator dump /sdcard/ui.xml >/dev/null 2>&1 && adb pull /sdcard/ui.xml "$out/ui.xml" >/dev/null 2>&1 || return 1
    { read -r title && read -r xy; } < <(not_responding) || return 0
    case "$title" in
      *Voltip*) fail "the app is not responding: $title" ;;
    esac
    echo "android-device-smoke: $title; closing that app" >&2
    # shellcheck disable=SC2086 # "x y"
    adb shell input tap $xy
    # The dialog's own close animation, not a wait for anything in the app.
    sleep 1
  done
}
# The title of a not-responding dialog on the screen and the centre of its "Close app" button
# (Android's own resource ids, the same in every language), or status 1 when there is none.
not_responding() {
  python3 - "$out/ui.xml" <<'PY'
import re, sys, xml.etree.ElementTree as ET
nodes = list(ET.parse(sys.argv[1]).getroot().iter("node"))
title = next((n.get("text") or "" for n in nodes if n.get("resource-id") == "android:id/alertTitle"), "")
close = next((n for n in nodes if n.get("resource-id") == "android:id/aerr_close"), None)
m = close is not None and re.fullmatch(r"\[(\d+),(\d+)\]\[(\d+),(\d+)\]", close.get("bounds", ""))
if not m:
    sys.exit(1)
x1, y1, x2, y2 = map(int, m.groups())
print(title.replace("\n", " ") or "an app is not responding")
print((x1 + x2) // 2, (y1 + y2) // 2)
PY
}

# The device answering as a device, for at most two minutes. A freshly booted emulator's adb link
# can still drop to `offline` and come back (main CI 2026-10-07: `adb: device offline` the moment the
# smoke began, after the boot had completed), and `adb wait-for-device` gives up on that state.
online() {
  local deadline=$((SECONDS + 120))
  until [ "$(adb get-state 2>/dev/null | tr -d '\r')" = device ]; do
    [ "$SECONDS" -lt "$deadline" ] || fail "the device stayed offline for 120 s"
    sleep 1
  done
}

online
# A freshly booted emulator is still delivering the broadcasts of its own setup (packages enabled,
# settings synced), which is what keeps its launcher too busy to answer: let them drain first, for
# at most two minutes (Android 13 and later; elsewhere this ends at once).
timeout 120 adb shell am wait-for-broadcast-idle >/dev/null 2>&1 || echo "android-device-smoke: the broadcast queues were still busy; going on" >&2
online
# A build signed with another key cannot replace the installed one.
adb uninstall "$package" >/dev/null 2>&1 || true
adb install -r -g "$apk" >"$out/install.txt" 2>&1 || fail "the APK did not install: $(tail -3 "$out/install.txt")"
adb logcat -c
adb shell am start -W -n "$package/.MainActivity" >"$out/start.txt" 2>&1 || fail "the activity did not start: $(tail -5 "$out/start.txt")"

# Up: the first screen's hold-to-talk button (in Chinese or English) is in the window's UI tree.
# At most 120 s: an emulator running arm64 code through its ARM translation is slow.
deadline=$((SECONDS + 120))
until dump && grep -qE '按住说话|Hold to talk' "$out/ui.xml"; do
  running || fail "the app closed on start"
  [ "$SECONDS" -lt "$deadline" ] || fail "the first screen did not come up within 120 s"
  sleep 3
done

# And it stays up. The 20 s are the check itself (an app that closes a few seconds after its first
# screen fails here), not a wait for something to finish.
for _ in $(seq 1 10); do
  sleep 2
  running || fail "the app closed after its first screen came up"
done
collect
# The app's own lines only: other processes on the device may log their own failures.
pid=$(adb shell pidof "$package" | tr -d '\r')
adb logcat -d --pid="$pid" >"$out/app-logcat.txt" 2>&1 || true
if grep -qE "FATAL EXCEPTION|panicked at|Fatal signal" "$out/app-logcat.txt" || grep -q "$package" "$out/crash.txt"; then
  fail "the app logged a fatal error although it is still running"
fi

# ---- large screen (emulator only) ----

# `wm size` / `wm density` print the physical value and, when one is set, the override: what to put
# back is the override, or a reset.
override() {
  sed -n 's/^Override [a-z]*: *//p' <<<"$1" | tr -d '\r' | head -1
}
# `settings get` prints `null` for a setting nobody wrote: put back means deleting it again.
restore_setting() {
  if [ "$2" = null ] || [ -z "$2" ]; then
    adb shell settings delete system "$1" >/dev/null 2>&1 || true
  else
    adb shell settings put system "$1" "$2" >/dev/null 2>&1 || true
  fi
}
restore_display() {
  if [ -n "$size_before" ]; then adb shell wm size "$size_before" >/dev/null 2>&1 || true; else adb shell wm size reset >/dev/null 2>&1 || true; fi
  if [ -n "$density_before" ]; then adb shell wm density "$density_before" >/dev/null 2>&1 || true; else adb shell wm density reset >/dev/null 2>&1 || true; fi
  restore_setting accelerometer_rotation "$auto_rotate_before"
  restore_setting user_rotation "$rotation_before"
  echo "android-device-smoke: display size, density and rotation put back" >&2
}
# The first screen up again on the changed display, and the app still up 10 s later.
still_up() {
  local deadline=$((SECONDS + 60))
  until dump && grep -qE '按住说话|Hold to talk' "$out/ui.xml"; do
    running || fail "the app closed on $1"
    [ "$SECONDS" -lt "$deadline" ] || fail "the first screen was not there on $1 within 60 s"
    sleep 3
  done
  for _ in $(seq 1 5); do
    sleep 2
    running || fail "the app closed on $1"
  done
}
qemu=$(adb shell getprop ro.kernel.qemu 2>/dev/null | tr -d '\r')
if [ "$qemu" = 1 ] || [ "${VOLTIP_SMOKE_LARGE_SCREEN:-0}" = 1 ]; then
  size_before=$(override "$(adb shell wm size)")
  density_before=$(override "$(adb shell wm density)")
  auto_rotate_before=$(adb shell settings get system accelerometer_rotation | tr -d '\r')
  rotation_before=$(adb shell settings get system user_rotation | tr -d '\r')
  trap restore_display EXIT
  adb logcat -c
  adb shell settings put system accelerometer_rotation 0
  adb shell wm size 2560x1600
  adb shell wm density 320
  adb shell settings put system user_rotation 0
  still_up "a tablet in landscape (2560 × 1600, density 320)"
  adb shell settings put system user_rotation 1
  still_up "a tablet in portrait"
  collect
  pid=$(adb shell pidof "$package" | tr -d '\r')
  adb logcat -d --pid="$pid" >"$out/app-logcat-large-screen.txt" 2>&1 || true
  if grep -qE "FATAL EXCEPTION|panicked at|Fatal signal" "$out/app-logcat-large-screen.txt" || grep -q "$package" "$out/crash.txt"; then
    fail "the app logged a fatal error on a large screen"
  fi
  echo "android-device-smoke: $package stayed up on a large screen, in landscape and in portrait"
else
  echo "android-device-smoke: not an emulator (ro.kernel.qemu '$qemu'): the large-screen part changes no device's display"
fi
echo "android-device-smoke: $package came up and stayed up ($(basename "$apk"))"
