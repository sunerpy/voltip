#!/usr/bin/env bash
# The Android app on a device: install the APK, start it, and check that it comes up and stays up,
# the first thing every user does. 0.0.18 and 0.0.19 closed at once on the user's phone (a Xiaomi
# HyperOS 3, 2026-10-01) although every package check passed; nothing had started the app. CI runs
# this against an emulator (`android-device` in ci.yml); it runs the same against a phone over adb.
#
# Usage: android-device-smoke.sh <apk or directory holding one> <out dir>
# Needs `adb` on PATH with one device online. Writes into <out>: install.txt, start.txt,
# logcat.txt, crash.txt, exit-info.txt, ui.xml and screen.png, whatever the outcome, and
# app-logcat.txt (the app's process alone) when it came up.
set -euo pipefail

if [ "$#" -ne 2 ]; then
  echo "usage: $0 <apk or directory> <out dir>" >&2
  exit 2
fi
apk=$1 out=$2
if [ -d "$apk" ]; then
  apk=$(find "$apk" -name '*.apk' | sort | head -1)
fi
[ -f "$apk" ] || { echo "android-device-smoke: no APK at $1" >&2; exit 2; }
package=dev.voltip.mobile
mkdir -p "$out"

collect() {
  adb logcat -d >"$out/logcat.txt" 2>&1 || true
  adb logcat -b crash -d >"$out/crash.txt" 2>&1 || true
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

adb wait-for-device
# A build signed with another key cannot replace the installed one.
adb uninstall "$package" >/dev/null 2>&1 || true
adb install -r -g "$apk" >"$out/install.txt" 2>&1 || fail "the APK did not install: $(tail -3 "$out/install.txt")"
adb logcat -c
adb shell am start -W -n "$package/.MainActivity" >"$out/start.txt" 2>&1 || fail "the activity did not start: $(tail -5 "$out/start.txt")"

# Up: the first screen's hold-to-talk button (in Chinese or English) is in the window's UI tree.
# At most 120 s: an emulator running arm64 code through its ARM translation is slow.
deadline=$((SECONDS + 120))
until adb shell uiautomator dump /sdcard/ui.xml >/dev/null 2>&1 && adb pull /sdcard/ui.xml "$out/ui.xml" >/dev/null 2>&1 &&
  grep -qE '按住说话|Hold to talk' "$out/ui.xml"; do
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
echo "android-device-smoke: $package came up and stayed up ($(basename "$apk"))"
