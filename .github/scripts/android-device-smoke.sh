#!/usr/bin/env bash
# The Android app on a device: install the APK, start it, check that it comes up and stays up, the
# first thing every user does, and that Android's back moves through it as on any app. 0.0.18 and
# 0.0.19 closed at once on the user's phone (a Xiaomi HyperOS 3, 2026-10-01) although every package
# check passed; nothing had started the app. CI runs this against an emulator (`android-device` in
# ci.yml); it runs the same against a phone over adb.
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

# Android's back (user request 2026-10-02, docs/acceptance/android/manual-checklist.md item 17):
# a page goes up a level, 设置 goes to 说话, and there a first back says so and a second within two
# seconds leaves the app. The WebView's text is in the UI tree the dump writes.
dump() {
  adb shell uiautomator dump /sdcard/ui.xml >/dev/null 2>&1 && adb pull /sdcard/ui.xml "$out/ui.xml" >/dev/null 2>&1
}
# Wait (at most 60 s) until the screen shows text matching the extended regex $1.
showing() {
  local deadline=$((SECONDS + 60))
  until dump && grep -qE "$1" "$out/ui.xml"; do
    running || fail "the app closed while waiting for: $1"
    [ "$SECONDS" -lt "$deadline" ] || fail "the screen did not show $1 within 60 s"
    sleep 1
  done
}
# The centre of the node labelled with one of the words: the text or description equal to it,
# else containing it.
centre() {
  python3 - "$out/ui.xml" "$@" <<'PY'
import re, sys, xml.etree.ElementTree as ET
path, *words = sys.argv[1:]
nodes = []
for node in ET.parse(path).getroot().iter("node"):
    m = re.fullmatch(r"\[(\d+),(\d+)\]\[(\d+),(\d+)\]", node.get("bounds", ""))
    if m:
        x1, y1, x2, y2 = map(int, m.groups())
        if x2 > x1 and y2 > y1:
            nodes.append(((node.get("text") or "").strip(), (node.get("content-desc") or "").strip(), (x1 + x2) // 2, (y1 + y2) // 2))
for exact in (True, False):
    for text, desc, x, y in nodes:
        if any((w in (text, desc)) if exact else (w in text or w in desc) for w in words):
            print(x, y)
            sys.exit(0)
sys.exit(1)
PY
}
tap() {
  local xy
  dump || fail "the screen could not be read"
  xy=$(centre "$@") || fail "nothing on the screen reads $*"
  # shellcheck disable=SC2086 # "x y"
  adb shell input tap $xy
}
back() {
  adb shell input keyevent KEYCODE_BACK
}
in_front() {
  adb shell dumpsys window | grep -E 'mCurrentFocus' | grep -q "$package"
}

tap 设置 Settings
showing '外观与语言|Appearance and language'
tap 外观与语言 'Appearance and language'
showing '按操作系统的语言选择|picks by the operating system language'
back
showing '外观与语言|Appearance and language'
if grep -qE '按操作系统的语言选择|picks by the operating system language' "$out/ui.xml"; then
  fail "back on 外观与语言 did not go up a level"
fi
in_front || fail "back on 外观与语言 left the app"
back
showing '按住说话|Hold to talk'
if grep -qE '外观与语言|Appearance and language' "$out/ui.xml"; then
  fail "back on 设置 did not go to 说话"
fi
in_front || fail "back on 设置 left the app"
back
showing '再返回一次即可退出|Go back again to leave'
in_front || fail "the first back on 说话 left the app"
# The two seconds pass: a back after them asks again instead of leaving. Then two backs in a row
# leave (the pause between them is the pace of a person's two backs, not a wait for anything).
sleep 3
back
sleep 0.3
back
deadline=$((SECONDS + 15))
while in_front; do
  [ "$SECONDS" -lt "$deadline" ] || fail "two backs on 说话 did not leave the app"
  sleep 1
done
collect
echo "android-device-smoke: back goes up a level, and two backs on 说话 leave the app"
echo "android-device-smoke: $package came up and stayed up ($(basename "$apk"))"
