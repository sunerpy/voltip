#!/usr/bin/env bash
# The Android app on a device: install the APK, start it, check that it comes up and stays up, the
# first thing every user does, and that Android's back moves through it as on any app. 0.0.18 and
# 0.0.19 closed at once on the user's phone (a Xiaomi HyperOS 3, 2026-10-01) although every package
# check passed; nothing had started the app. CI runs this against an emulator (`android-device` in
# ci.yml); it runs the same against a phone over adb.
#
# Usage: android-device-smoke.sh <apk or directory holding one> <out dir>
# Needs `adb` on PATH with one device online. Writes into <out>: install.txt, start.txt,
# logcat.txt, crash.txt, events.txt, exit-info.txt, ui.xml and screen.png, whatever the outcome, and
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

adb wait-for-device
# A freshly booted emulator is still delivering the broadcasts of its own setup (packages enabled,
# settings synced), which is what keeps its launcher too busy to answer: let them drain first, for
# at most two minutes (Android 13 and later; elsewhere this ends at once).
timeout 120 adb shell am wait-for-broadcast-idle >/dev/null 2>&1 || echo "android-device-smoke: the broadcast queues were still busy; going on" >&2
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

# Android's back (user request 2026-10-02, docs/acceptance/android/manual-checklist.md item 17):
# a page goes up a level, 设置 goes to 说话, and there a first back says so and a second within two
# seconds leaves the app. The WebView's text is in the UI tree the dump writes.
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
# else containing it. With --clear-of-tabs, the centre of the part above the tab bar (the
# WebView's nodes reach under it, and a tap there lands on a tab), or status 3 when too little
# of it shows there: the page has to scroll first.
centre() {
  python3 - "$out/ui.xml" "$@" <<'PY'
import re, sys, xml.etree.ElementTree as ET
path, *args = sys.argv[1:]
clear = bool(args) and args[0] == "--clear-of-tabs"
words = args[1:] if clear else args
TABS = {"说话", "记录", "设置", "Talk", "History", "Settings"}
nodes = []
for node in ET.parse(path).getroot().iter("node"):
    m = re.fullmatch(r"\[(\d+),(\d+)\]\[(\d+),(\d+)\]", node.get("bounds", ""))
    if m:
        x1, y1, x2, y2 = map(int, m.groups())
        if x2 > x1 and y2 > y1:
            nodes.append(((node.get("text") or "").strip(), (node.get("content-desc") or "").strip(), x1, y1, x2, y2))
# The window is the first node; the tab bar is where the tab names are, in its lowest quarter.
bottom = nodes[0][5] if nodes else 0
tab_top = min((n[3] for n in nodes if (n[0] in TABS or n[1] in TABS) and bottom * 3 // 4 <= n[3] < bottom), default=bottom)
for exact in (True, False):
    for text, desc, x1, y1, x2, y2 in nodes:
        if any((w in (text, desc)) if exact else (w in text or w in desc) for w in words):
            if not clear:
                print((x1 + x2) // 2, (y1 + y2) // 2)
                sys.exit(0)
            shown = min(y2, tab_top) - y1
            if shown < 24:
                sys.exit(3)
            print((x1 + x2) // 2, (y1 + min(y2, tab_top)) // 2)
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
# A row of a page that scrolls: while the tab bar hides it (a small screen, a large font), the
# page scrolls up by about a third of the screen first.
tap_row() {
  local xy status w h
  read -r w h <<EOF
$(adb shell wm size | tr -d '\r' | awk -F'[ x]' '/size/ { w = $(NF - 1); h = $NF } END { print w, h }')
EOF
  for _ in 1 2 3 4 5 6; do
    dump || fail "the screen could not be read"
    status=0
    xy=$(centre --clear-of-tabs "$@") || status=$?
    if [ "$status" -eq 0 ]; then
      # shellcheck disable=SC2086 # "x y"
      adb shell input tap $xy
      return 0
    fi
    [ "$status" -eq 3 ] || fail "nothing on the screen reads $*"
    adb shell input swipe $((w / 2)) $((h * 3 / 5)) $((w / 2)) $((h * 3 / 10)) 300
  done
  fail "$* stayed under the tab bar"
}
back() {
  adb shell input keyevent KEYCODE_BACK
}
in_front() {
  adb shell dumpsys window | grep -E 'mCurrentFocus' | grep -q "$package"
}

# Up a level from 设置 › 外观与语言, then to 说话.
up_a_level() {
  tap 设置 Settings
  showing '外观与语言|Appearance and language'
  tap_row 外观与语言 'Appearance and language'
  showing '按操作系统的语言选择|picks by the operating system language'
  back
  showing '外观与语言|Appearance and language'
  if grep -qE '按操作系统的语言选择|picks by the operating system language' "$out/ui.xml"; then
    fail "back on 外观与语言 did not go up a level ($1)"
  fi
  in_front || fail "back on 外观与语言 left the app ($1)"
  back
  showing '按住说话|Hold to talk'
  if grep -qE '外观与语言|Appearance and language' "$out/ui.xml"; then
    fail "back on 设置 did not go to 说话 ($1)"
  fi
  in_front || fail "back on 设置 left the app ($1)"
}
up_a_level "the activity Android started"

# Android recreates the activity when an overlay changes (the navigation mode, the wallpaper's
# colours) and when the font or display size does, and the back handler has to survive that
# (2026-10-03: Tauri's own one stayed with the first activity, so after a recreation every back
# left the app). A font-size change recreates it here, and the size goes back before the second
# walk, which recreates it once more: a larger font moves the rows, and a row half under the tab
# bar takes a tap meant for it as one on the tab. The system's event log says when, and so does
# the app's log, where every MainActivity says when it takes the backs (BackPlugin.kt).
attached() {
  adb logcat -d -s VoltipBack:I | grep -c 'attached to activity' || true
}
relaunched() {
  adb logcat -b events -d | grep -cE "(wm|am)_relaunch(_resume)?_activity.*$package" || true
}
# The new activity's own line comes once it exists, after the system's; a build without the plugin
# has the system's alone.
if [ "$(attached)" -gt 0 ]; then recreations=attached; else recreations=relaunched; fi
# Set the font scale to $1 and wait until Android has recreated the activity.
recreate_with_font_scale() {
  local before deadline
  before=$("$recreations")
  adb shell settings put system font_scale "$1"
  deadline=$((SECONDS + 60))
  until [ "$("$recreations")" -gt "$before" ]; do
    running || fail "the app closed when the font size changed to $1"
    [ "$SECONDS" -lt "$deadline" ] || fail "Android did not recreate the activity within 60 s of a font-size change to $1"
    sleep 1
  done
}
font_scale=$(adb shell settings get system font_scale | tr -d '\r')
restore_font_scale() {
  if [ "$font_scale" = null ] || [ -z "$font_scale" ]; then
    adb shell settings delete system font_scale >/dev/null 2>&1 || true
  else
    adb shell settings put system font_scale "$font_scale" >/dev/null 2>&1 || true
  fi
}
trap restore_font_scale EXIT
case "$font_scale" in
  null | "") scale=1.0 ;;
  *) scale=$font_scale ;;
esac
if [ "$scale" = 1.15 ]; then
  recreate_with_font_scale 1.3
else
  recreate_with_font_scale 1.15
fi
recreate_with_font_scale "$scale"
showing '按住说话|Hold to talk'
up_a_level "the activity Android recreated"

# At 说话 a back asks the plugin to let the backs of the next two seconds through, and shows a hint
# that a second one leaves. The hint is up for 3 s, about as long as one dump takes here, so the
# check reads the request in the app's log (BackPlugin.kt); back.test.tsx checks the hint.
releases() {
  adb logcat -d -s VoltipBack:I | grep -c 'release ' || true
}
first_back_at_talk() {
  local before deadline
  before=$(releases)
  back
  deadline=$((SECONDS + 30))
  until [ "$(releases)" -gt "$before" ]; do
    running || fail "the app closed on a first back at 说话 ($1)"
    [ "$SECONDS" -lt "$deadline" ] || fail "a first back at 说话 did not let the next one through ($1)"
    sleep 0.2
  done
  in_front || fail "a first back at 说话 left the app ($1)"
}
first_back_at_talk "the first"
# The two seconds pass (the wait is the behaviour under test): a back after them asks again
# instead of leaving. Then a back within the two seconds leaves.
sleep 3
first_back_at_talk "after the two seconds"
back
deadline=$((SECONDS + 15))
while in_front; do
  [ "$SECONDS" -lt "$deadline" ] || fail "a second back on 说话 did not leave the app"
  sleep 1
done
collect
echo "android-device-smoke: back goes up a level, before and after Android recreates the activity, and two backs on 说话 leave the app"
echo "android-device-smoke: $package came up and stayed up ($(basename "$apk"))"
