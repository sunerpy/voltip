#!/usr/bin/env bash
# The React Native phone app on a Device Farm phone (docs/mobile-rn.md §7), driven over adb from
# the test host: cold start, a take on the phone, every tab and the main settings pages, a
# dropdown menu, a full-screen editor, the pairing pages, Android's back through the stack and the
# second back that leaves. Every step runs whatever the one before it did, and writes what it saw:
# the screenshots, the UI trees, the app's own logcat and summary.txt (key=value lines) go into
# <out>. The verdict is the build host's (scripts/devicefarm-rn.sh reads summary.txt); this side
# only measures. The run is scheduled with the device in zh_CN, so the labels below are Chinese,
# with the English one as the alternative.
#
# Usage: acceptance.sh <out dir>     (Device Farm passes $DEVICEFARM_LOG_DIR)
set -uo pipefail

out=${1:?usage: acceptance.sh <out dir>}
mkdir -p "$out"
PKG=dev.voltip.mobile.rn
summary=$out/summary.txt
: >"$summary"

measure() { printf '%s=%s\n' "$1" "$2" | tee -a "$summary"; }
step() { printf '\n===== %s =====\n' "$1"; }

# The screen's UI tree into $out/ui.xml, closing another app's not-responding dialog first (a
# busy device's launcher can raise one over ours; one for Voltip is a finding, left on the screen).
dump() {
  local i anr
  for i in 1 2 3; do
    adb shell uiautomator dump /sdcard/ui.xml >/dev/null 2>&1 && adb pull /sdcard/ui.xml "$out/ui.xml" >/dev/null 2>&1 || return 1
    anr=$(python3 - "$out/ui.xml" <<'PY'
import re, sys, xml.etree.ElementTree as ET
try:
    nodes = list(ET.parse(sys.argv[1]).getroot().iter("node"))
except Exception:
    sys.exit(0)
title = next((n.get("text") or "" for n in nodes if n.get("resource-id") == "android:id/alertTitle"), "")
close = next((n for n in nodes if n.get("resource-id") == "android:id/aerr_close"), None)
m = close is not None and re.fullmatch(r"\[(\d+),(\d+)\]\[(\d+),(\d+)\]", close.get("bounds", ""))
if m and "Voltip" not in title:
    x1, y1, x2, y2 = map(int, m.groups())
    print((x1 + x2) // 2, (y1 + y2) // 2)
PY
)
    [ -z "$anr" ] && return 0
    echo "acceptance: closing another app's not-responding dialog"
    # shellcheck disable=SC2086 # "x y"
    adb shell input tap $anr
    sleep 1 # the dialog's close animation
  done
}

screen_h=2400
screen_w=1080

# The centre of the first node whose text, content-desc or resource-id (the app's testID) matches
# the regular expression $1 and that lies on the screen (between the status bar and the
# navigation bar). Each field is matched on its own, so `^…$` anchors a whole label.
find_node() {
  python3 - "$out/ui.xml" "$1" "$screen_h" <<'PY'
import re, sys, xml.etree.ElementTree as ET
try:
    nodes = list(ET.parse(sys.argv[1]).getroot().iter("node"))
except Exception:
    sys.exit(1)
pattern, height = re.compile(sys.argv[2]), int(sys.argv[3])
for n in nodes:
    fields = (n.get("text"), n.get("content-desc"), n.get("resource-id"))
    if not any(f and pattern.search(f) for f in fields):
        continue
    m = re.fullmatch(r"\[(\d+),(\d+)\]\[(\d+),(\d+)\]", n.get("bounds", ""))
    if not m:
        continue
    x1, y1, x2, y2 = map(int, m.groups())
    x, y = (x1 + x2) // 2, (y1 + y2) // 2
    if 80 < y < height - 140 and x2 > x1 and y2 > y1:
        print(x, y)
        sys.exit(0)
sys.exit(1)
PY
}

# Whether the first node whose resource-id (testID) matches $1 is checked (a switch that is on).
checked() {
  python3 - "$out/ui.xml" "$1" <<'PY'
import re, sys, xml.etree.ElementTree as ET
nodes = list(ET.parse(sys.argv[1]).getroot().iter("node"))
pattern = re.compile(sys.argv[2])
node = next((n for n in nodes if pattern.search(n.get("resource-id") or "")), None)
sys.exit(0 if node is not None and node.get("checked") == "true" else 1)
PY
}

# Wait at most $2 seconds for text matching $1 on the screen.
wait_text() {
  local deadline=$((SECONDS + $2))
  while [ "$SECONDS" -lt "$deadline" ]; do
    if dump && find_node "$1" >/dev/null; then return 0; fi
    sleep 2
  done
  return 1
}

# Move the page by half a screen: `down` shows what is below, `up` what is above.
scroll() {
  local from=$((screen_h * 7 / 10)) to=$((screen_h * 3 / 10))
  [ "$1" = up ] && { from=$((screen_h * 3 / 10)); to=$((screen_h * 7 / 10)); }
  adb shell input swipe $((screen_w / 2)) "$from" $((screen_w / 2)) "$to" 400
  sleep 1 # the fling settles
}
scroll_down() { scroll down; }

# Tap the node matching $1: where the page is, else below (three half screens), else above (a
# page that kept its scroll position from an earlier step). An app that is no longer in front has
# nothing to scroll: that fails at once.
tap_text() {
  local xy way
  for way in here down down down up up up up up up up; do
    if [ "$way" != here ]; then
      in_front || return 1
      scroll "$way"
    fi
    if dump && xy=$(find_node "$1"); then
      # shellcheck disable=SC2086
      adb shell input tap $xy
      sleep 1 # the transition to the next page
      return 0
    fi
  done
  return 1
}

# Press and hold the node matching $1 for $2 milliseconds (a swipe that does not move).
hold_text() {
  local xy
  dump && xy=$(find_node "$1") || return 1
  # shellcheck disable=SC2086
  set -- $xy "$2"
  adb shell input swipe "$1" "$2" "$1" "$2" "$3"
}

back() {
  adb shell input keyevent KEYCODE_BACK
  sleep 1 # the transition back
}

# Android's back until the screen shows $1, at most $2 presses; the screen is checked before each
# press, so the page that shows $1 never gets one.
back_until() {
  local i
  for ((i = 0; i < $2; i++)); do
    wait_text "$1" 3 && return 0
    back
  done
  wait_text "$1" 3
}

shot() {
  adb exec-out screencap -p >"$out/$1.png" 2>/dev/null || true
  dump && cp "$out/ui.xml" "$out/$1.xml" 2>/dev/null || true
}

running() { [ -n "$(adb shell pidof "$PKG" 2>/dev/null | tr -d '\r')" ]; }
in_front() { adb shell dumpsys activity activities 2>/dev/null | grep -E "mResumedActivity|topResumedActivity" | grep -q "$PKG"; }

# A page: tap $2, wait for $3 on the new page, screenshot $1; records $1=ok|missing.
page() {
  local name=$1 open=$2 expect=$3
  if tap_text "$open" && wait_text "$expect" 20; then
    measure "$name" ok
  else
    measure "$name" missing
  fi
  shot "$name"
}

step "DEVICE"
measure model "$(adb shell getprop ro.product.model | tr -d '\r')"
measure android "$(adb shell getprop ro.build.version.release | tr -d '\r')/$(adb shell getprop ro.build.version.sdk | tr -d '\r')"
measure locale "$(adb shell getprop persist.sys.locale | tr -d '\r')"
size=$(adb shell wm size | tr -d '\r' | tail -1 | sed 's/.*: //')
measure size "$size"
screen_w=${size%x*}
screen_h=${size#*x}

step "INSTALLED"
if adb shell pm list packages | grep -q "package:$PKG\$"; then
  measure installed yes
else
  measure installed no
  adb shell pm list packages | grep -i voltip
  exit 0
fi
adb shell dumpsys package "$PKG" | grep -E "versionName|targetSdk" | head -3
adb shell pm grant "$PKG" android.permission.RECORD_AUDIO 2>&1 | tail -1
adb shell pm grant "$PKG" android.permission.CAMERA 2>&1 | tail -1
adb shell dumpsys package "$PKG" | grep -E "RECORD_AUDIO|CAMERA" | head -4

step "COLD START"
adb shell am force-stop "$PKG"
# A larger buffer: the UI-tree dumps log a line per invisible node and push the app's lines out.
adb logcat -G 16M >/dev/null 2>&1 || true
adb logcat -c
start=$(adb shell am start -W -n "$PKG/.MainActivity" 2>&1 | tr -d '\r')
echo "$start"
measure cold_start_ms "$(printf '%s\n' "$start" | sed -n 's/^TotalTime: //p')"
t0=$SECONDS
if wait_text '按住说话|Hold to talk' 90; then
  measure first_screen ok
  measure first_screen_s "$((SECONDS - t0))"
else
  measure first_screen missing
fi
shot 01-talk
alive=yes
for _ in 1 2 3 4 5; do
  sleep 2 # the check itself: an app that closes a few seconds after its first screen fails here
  running || alive=no
done
measure stays_up "$alive"

step "TALK ON THE PHONE"
# The microphone hears the device's room; whatever it records, the take must run its course:
# recording, recognition (the built-in service), then a result or a reason.
# The result line stays a few seconds before the card goes idle again, so the outcome is read from
# the core's own log: `phase=done`, or `take failed code=…` (the shell logs the code only).
if hold_text '^按住说话$|^Hold to talk$' 4000; then
  measure talk_pressed yes
  shot 02-talk-result
  pid=$(adb shell pidof "$PKG" | tr -d '\r')
  outcome=none
  deadline=$((SECONDS + 60))
  while [ "$SECONDS" -lt "$deadline" ]; do
    adb logcat -d --pid="$pid" >"$out/talk-logcat.txt" 2>&1
    if grep -q "phase=done kind=dictation" "$out/talk-logcat.txt"; then outcome="done"; break; fi
    code=$(sed -n 's/.*take failed code=\([A-Za-z_]*\).*/\1/p' "$out/talk-logcat.txt" | tail -1)
    if [ -n "$code" ]; then outcome="failed:$code"; break; fi
    sleep 2
  done
  measure talk_outcome "$outcome"
  if [ "$outcome" = none ]; then measure talk_finished timeout; else measure talk_finished yes; fi
  grep -E "recorder (started|finished)|dictation phase|take failed" "$out/talk-logcat.txt" | sed 's/^.*voltip_/voltip_/' | tail -12
  shot 02b-talk-after
else
  measure talk_pressed missing
fi

# The talk page: its header is the only one with the 本机 action.
TALK='^open-this-device$'

step "TABS"
# The tabs by their testIDs: on a tall screen the labels sit close to the navigation bar.
page 03-history '^tab-history$' '^今天$|^Today$'
page 04-settings '^tab-settings$' '语音模型|Speech models'

step "SETTINGS PAGES"
page 05-speech '^语音模型$|^Speech models$' '内置服务|Built-in service'
scroll_down
shot 05b-speech-more
back
page 06-ai '^AI 模型与预设$|^AI models and presets$' 'AI 润色|AI polish|预设|Presets'
back
page 07-appearance '^外观与语言$|^Appearance and language$' '暗黑|Dark'
if tap_text '^跟随系统$|^Follow system$' && tap_text '^暗黑$|^Dark$'; then
  measure theme_dark ok
else
  measure theme_dark missing
fi
shot 08-dark
tap_text '^默认$|^Default$' || true
# Following the system: on Android 12+ the accent becomes the wallpaper's (docs/mobile-rn.md §5);
# the switch's track and the selected tile show it. The switch goes back off afterwards.
if tap_text '^follow-system-theme$' && dump && checked '^follow-system-theme$'; then
  measure follow_system ok
else
  measure follow_system missing
fi
shot 08b-follow-system
tap_text '^follow-system-theme$' || true
back
page 09-recording '^录音$|^Recording$' '最长录音时长|Longest recording'
# The menu's options: the page itself shows the current value only (10 分钟).
if tap_text '最长录音时长|Longest' && wait_text '^30 分钟$|^30 minutes$' 10; then
  measure dropdown ok
else
  measure dropdown missing
fi
shot 10-dropdown
back # closes the menu, not the page
if wait_text '最长录音时长|Longest recording' 5 && ! find_node '^30 分钟$|^30 minutes$' >/dev/null; then
  measure back_closes_menu_first ok
else
  measure back_closes_menu_first no
fi
shot 10b-after-back
back
page 11-dictionary '^个人词典$|^Dictionary$' '新建词条|New entry'
if tap_text '新建词条|New entry' && wait_text '正确写法|Right spelling' 10; then
  measure editor ok
else
  measure editor missing
fi
shot 12-editor
back
back
page 12b-rules '^替换规则$|^Replacement rules$' '导入 TOML|Import TOML|新建规则|New rule'
back
page 12c-scenes '^场景$|^Scenes$' '新建场景|New scene|内置|Built-in'
back
page 12d-history-settings '^历史记录$|^History$' '保存听写历史|Keep dictation history'
back
page 12e-feedback '^反馈$|^Feedback$' '发送反馈|Send feedback'
back
page 12f-about '^关于 Voltip$|^About Voltip$' 'AGPL-3.0'
back

step "PAIRING PAGES"
# From 设置 back to 说话: a tab's back goes to the first tab.
back
if wait_text "$TALK" 10; then measure back_to_talk ok; else measure back_to_talk missing; fi
page 13-device "$TALK" '^指纹$|^Fingerprint$'
page 14-pair '^配对电脑$|^Pair a computer$' '扫码|Scan'
if tap_text '^输入 6 位验证码$|^Enter 6-digit code$' && wait_text '电脑上显示的 6 位验证码|shown on the computer' 10; then
  measure pair_code ok
else
  measure pair_code missing
fi
shot 15-pair-code

step "BACK TO LEAVE"
# Back through the pairing page and 本机 to 说话, one page per press.
if back_until "$TALK" 4; then measure talk_again ok; else measure talk_again missing; fi
# The first back shows its hint for three seconds (TOAST_MS.neutral). On a Pixel 3 the hint was
# gone by the time back()'s pause and a separate dump had read the tree (rn-accept-4), so the tree
# is read in the same shell as the key press, with no pause in between; the screenshot follows.
adb shell 'input keyevent KEYCODE_BACK; uiautomator dump /sdcard/ui.xml >/dev/null 2>&1; screencap -p /sdcard/back-once.png'
adb pull /sdcard/ui.xml "$out/ui.xml" >/dev/null 2>&1 && cp "$out/ui.xml" "$out/16-back-once.xml"
adb pull /sdcard/back-once.png "$out/16-back-once.png" >/dev/null 2>&1 || true
if find_node '再返回一次即可退出|Go back again to leave' >/dev/null && in_front; then measure back_once_stays ok; else measure back_once_stays missing; fi
# The app leaves on a second back within two seconds (EXIT_WINDOW_MS) of the first; the hint
# above took longer than that, so the window is spent and two presses in a row are needed now.
sleep 2 # let the window the first press opened run out (EXIT_WINDOW_MS is 2000)
adb shell 'input keyevent KEYCODE_BACK; input keyevent KEYCODE_BACK'
left=no
for _ in 1 2 3 4 5; do
  sleep 1 # the activity finishes
  in_front || { left=yes; break; }
done
if [ "$left" = yes ]; then measure back_twice_leaves ok; else measure back_twice_leaves no; fi

step "MEMORY"
adb shell am start -W -n "$PKG/.MainActivity" >/dev/null 2>&1
wait_text '按住说话|Hold to talk' 30 || true
adb shell dumpsys meminfo "$PKG" | tee "$out/meminfo.txt" | grep -E "TOTAL PSS|TOTAL:" | head -2
measure total_pss_kb "$(sed -n 's/^ *TOTAL PSS: *\([0-9]*\).*/\1/p; s/^ *TOTAL: *\([0-9]*\).*/\1/p' "$out/meminfo.txt" | head -1)"
adb shell dumpsys gfxinfo "$PKG" | grep -E "Total frames rendered|Janky frames" | tee "$out/gfxinfo.txt"

step "LOGS"
pid=$(adb shell pidof "$PKG" | tr -d '\r')
[ -n "$pid" ] && adb logcat -d --pid="$pid" >"$out/app-logcat.txt" 2>&1
adb logcat -d >"$out/logcat-run.txt" 2>&1
adb logcat -b crash -d >"$out/crash.txt" 2>&1
adb shell dumpsys activity exit-info "$PKG" >"$out/exit-info.txt" 2>&1
if grep -qE "FATAL EXCEPTION|panicked|Fatal signal" "$out/logcat-run.txt" 2>/dev/null && grep -E "FATAL EXCEPTION|panicked|Fatal signal" "$out/logcat-run.txt" | grep -q "$PKG\|voltip"; then
  measure fatal found
elif grep -q "$PKG" "$out/crash.txt"; then
  measure fatal found
else
  measure fatal none
fi
grep -E "voltip|ReactNativeJS" "$out/logcat-run.txt" 2>/dev/null | grep -vE "nativeloader" | tail -60
echo "summary:"
cat "$summary"
