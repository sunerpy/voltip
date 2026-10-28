#!/usr/bin/env bash
# The React Native phone app on a Device Farm phone (docs/mobile-rn.md §7), driven over adb from
# the test host: cold start, a take on the phone (held, slipped, cancelled, tapped), every tab and the main settings pages, a
# dropdown menu, a full-screen editor, the pairing pages, Android's back through the stack and the
# second back that leaves; then the update check, the camera page, the same tabs and sheets with
# the device turned the other way (docs/mobile-rn.md §5), and the floating bubble (docs/dictation.md
# §20.12): takes with 手势粘贴 on (its accessibility service enabled over adb) and off, tucking
# itself away after 5 s idle, staying out while held for 8 s, and keeping to the screen when it
# turns. Rotation and the accessibility settings are put back on the way out. The app's log must
# show no class, method, field or library that R8 lost. Every step runs whatever the one before it did, and writes what it saw:
# the screenshots, the UI trees, the app's own logcat and summary.txt (key=value lines) go into
# <out>. The verdict is the build host's (scripts/devicefarm-rn.sh reads summary.txt); this side
# only measures. The run is scheduled with the device in zh_CN, so the labels below are Chinese,
# with the English one as the alternative.
#
# Usage: acceptance.sh <out dir>     (Device Farm passes $DEVICEFARM_LOG_DIR)
set -uo pipefail

out=${1:?usage: acceptance.sh <out dir>}
mkdir -p "$out"
PKG=dev.voltip.mobile
summary=$out/summary.txt
: >"$summary"

measure() { printf '%s=%s\n' "$1" "$2" | tee -a "$summary"; }
step() { printf '\n===== %s =====\n' "$1"; }

# The screen's UI tree into $out/ui.xml, closing another app's not-responding dialog first (a
# busy device's launcher can raise one over ours; one for Voltip is a finding, left on the screen).
# A screen that never settles (a recording's timer and level) gets no tree: uiautomator waits for an
# idle moment and gives up, and the file it leaves is the previous screen's, so it is removed first.
dump() {
  local i anr
  for i in 1 2 3; do
    adb shell rm -f /sdcard/ui.xml
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
# navigation bar) and, unless it is one of the tabs, above the tab dock that floats over the
# page: a tap there lands on the dock (2026-10-13: 历史记录 under the dock opened the 记录 tab).
# The tabs themselves only need to be on the screen: the dock keeps clear of the navigation bar
# itself, and in landscape it sits lower than the 140 px kept free below a page's own controls
# (2026-10-28, a Pixel 9: the dock's middle at 960 of 1080 px).
# Each field is matched on its own, so `^…$` anchors a whole label.
find_node() {
  python3 - "$out/ui.xml" "$1" "$screen_h" <<'PY'
import re, sys, xml.etree.ElementTree as ET
try:
    nodes = list(ET.parse(sys.argv[1]).getroot().iter("node"))
except Exception:
    sys.exit(1)
pattern, height = re.compile(sys.argv[2]), int(sys.argv[3])
def box(n):
    m = re.fullmatch(r"\[(\d+),(\d+)\]\[(\d+),(\d+)\]", n.get("bounds", ""))
    return tuple(map(int, m.groups())) if m else None
dock = next((box(n) for n in nodes if n.get("resource-id") == "tab-dock"), None)
for n in nodes:
    fields = (n.get("text"), n.get("content-desc"), n.get("resource-id"))
    if not any(f and pattern.search(f) for f in fields):
        continue
    b = box(n)
    if b is None:
        continue
    x1, y1, x2, y2 = b
    x, y = (x1 + x2) // 2, (y1 + y2) // 2
    tab = (n.get("resource-id") or "").startswith("tab-")
    if dock is not None and not tab and y >= dock[1]:
        continue
    if 80 < y < height - (16 if tab else 140) and x2 > x1 and y2 > y1:
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
# The app in front again, on the page it was on, after a step that may have left it (a back past
# the first tab when a page did not open): one failure stays one failure.
front() {
  in_front && return 0
  adb shell am start -n "$PKG/.MainActivity" >/dev/null 2>&1
  local _
  for _ in 1 2 3 4 5; do
    sleep 1 # the activity comes back
    in_front && return 0
  done
  return 1
}
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

step "HOLD, SLIP AND TAP"
# docs/dictation.md §20.11 (2026-10-13): a finger that slips while holding still sends, a release
# more than the button's width (120 dp) from the press cancels, and 点按 under the card turns the
# button into tap to start, tap again to finish. Each take is read from the core's phase lines
# that follow the gesture; the next gesture waits for `phase=idle` (a result stays on the card for
# its dwell, and the button ignores a press until then).
phases() {
  local pid
  pid=$(adb shell pidof "$PKG" | tr -d '\r')
  [ -n "$pid" ] && adb logcat -d --pid="$pid" 2>/dev/null | grep -E "dictation phase|take failed"
}
settle() {
  local deadline=$((SECONDS + 30)) last
  while [ "$SECONDS" -lt "$deadline" ]; do
    last=$(phases | grep "dictation phase" | tail -1)
    if [ -z "$last" ] || printf '%s' "$last" | grep -q "phase=idle"; then return 0; fi
    sleep 1
  done
  return 1
}
# The outcome of the take after the first $2 phase lines: done, failed:<code>, cancelled, or none
# within 60 s; it waits for the core to be idle again as well.
take_outcome() {
  local name=$1 skip=$2 lines outcome=none code deadline=$((SECONDS + 60))
  while [ "$SECONDS" -lt "$deadline" ]; do
    lines=$(phases | tail -n +$((skip + 1)))
    printf '%s\n' "$lines" >"$out/$name-phases.txt"
    if printf '%s\n' "$lines" | grep -q "phase=cancelled"; then
      outcome=cancelled
    elif printf '%s\n' "$lines" | grep -q "phase=done kind=dictation"; then
      outcome="done"
    elif printf '%s\n' "$lines" | grep -q "phase=failed"; then
      code=$(printf '%s\n' "$lines" | sed -n 's/.*take failed code=\([A-Za-z_]*\).*/\1/p' | tail -1)
      outcome=failed:${code:-unknown}
    fi
    if [ "$outcome" != none ] && printf '%s\n' "$lines" | grep -q "phase=idle"; then break; fi
    sleep 2
  done
  echo "$outcome"
}
density=$(adb shell wm density | tr -d '\r' | sed -n 's/.*: *\([0-9][0-9]*\).*/\1/p' | tail -1)
measure density "${density:-unknown}"
density=${density:-420}
dp() { echo $(($1 * density / 160)); }
settle || echo "acceptance: the first take did not go idle"
if dump && xy=$(find_node '^phone-mic-hold$'); then
  read -r x y <<<"$xy"
  # 50 dp left and 30 dp up over four seconds, let go there: about 58 dp from the press, inside
  # the button's reach. Before 2026-10-13 this cancelled.
  n=$(phases | wc -l)
  adb shell input swipe "$x" "$y" $((x - $(dp 50))) $((y - $(dp 30))) 4000
  slip=$(take_outcome 02c-slip "$n")
  measure slip_outcome "$slip"
  case $slip in cancelled | none) measure slip_sends no ;; *) measure slip_sends yes ;; esac
  # 200 dp straight up, let go there: a cancel.
  n=$(phases | wc -l)
  adb shell input swipe "$x" "$y" "$x" $((y - $(dp 200))) 4000
  away=$(take_outcome 02d-away "$n")
  measure away_outcome "$away"
  if [ "$away" = cancelled ]; then measure away_cancels yes; else measure away_cancels no; fi
else
  measure slip_sends missing
  measure away_cancels missing
fi
# 点按: a tap starts, letting go does not stop it, the next tap finishes it. While it records the
# screen never settles, so there is no UI tree: the taps go to the button's place read before the
# take, and the take is read from the core's phase lines (取消录音 is covered by the JS tests).
if tap_text '^phone-talk-mode-tap$' && wait_text '^点按开始说话$|^Tap to talk$' 10; then
  measure tap_mode ok
  shot 02e-tap-mode
  if dump && xy=$(find_node '^phone-mic-hold$'); then
    read -r x y <<<"$xy"
    n=$(phases | wc -l)
    adb shell input tap "$x" "$y"
    started=no
    for _ in 1 2 3 4 5 6 7 8 9 10; do
      if phases | tail -n +$((n + 1)) | grep -q "phase=listening"; then started=yes; break; fi
      sleep 1
    done
    measure tap_started "$started"
    sleep 3 # recording, with the finger off the screen
    adb exec-out screencap -p >"$out/02f-tap-recording.png" 2>/dev/null || true
    adb shell input tap "$x" "$y"
    tapped=$(take_outcome 02g-tap "$n")
  else
    tapped=missing
  fi
  measure tap_outcome "$tapped"
  case $tapped in cancelled | none | missing) measure tap_finished no ;; *) measure tap_finished yes ;; esac
else
  measure tap_mode missing
fi
# Back to holding for the steps below.
if tap_text '^phone-talk-mode-hold$' && wait_text '^按住说话$|^Hold to talk$' 10; then
  measure hold_again ok
else
  measure hold_again missing
fi

# The talk page: its header is the only one with the 本机 action.
TALK='^open-this-device$'

step "TABS"
# The tabs by their testIDs: on a tall screen the labels sit close to the navigation bar.
page 03-history '^tab-history$' '^今天$|^Today$'
page 04-settings '^tab-settings$' '语音模型|Speech models'

step "SETTINGS PAGES"
page 05-speech '^语音模型$|^Speech models$' '内置服务|Built-in service'
# A provider's card opens its settings in a sheet from the bottom (user decision 2026-10-10), and
# back closes the sheet, not the page.
if tap_text '^provider-asr-groq$' && wait_text '^provider-form$' 10; then
  measure provider_sheet ok
else
  measure provider_sheet missing
fi
shot 05a-provider-sheet
back
if wait_text '^语音模型$|^Speech models$' 5 && ! find_node '^provider-form$' >/dev/null; then
  measure back_closes_sheet_first ok
else
  measure back_closes_sheet_first no
fi
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
# The share sheet (2026-10-28, R8): the exported TOML goes out the way a history entry's text does
# (phone_share_text), so the system's chooser comes to the front while the app keeps running.
share_sheet=missing
if tap_text '^rules-export-open$' && wait_text '^rules-export-share$' 10 && tap_text '^rules-export-share$'; then
  for _ in 1 2 3 4 5; do
    sleep 1 # the chooser opens
    if ! in_front && running; then share_sheet=ok; break; fi
  done
  shot 12b2-share-sheet
fi
measure share_sheet "$share_sheet"
# Back past the chooser and the export dialog to the rules page (its own export button), then to 设置.
back_until '^rules-export-open$' 3 || true
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

# ---- the release build's other surfaces (2026-10-28: R8, edge to edge, large screens, the bubble) ----

# Put back on every way out: the rotation and the accessibility services this script changes.
rotation_auto=$(adb shell settings get system accelerometer_rotation | tr -d '\r')
rotation_user=$(adb shell settings get system user_rotation | tr -d '\r')
a11y_services=$(adb shell settings get secure enabled_accessibility_services | tr -d '\r')
a11y_enabled=$(adb shell settings get secure accessibility_enabled | tr -d '\r')
put_back() {
  local namespace=$1 name=$2 value=$3
  if [ "$value" = null ] || [ -z "$value" ]; then
    adb shell settings delete "$namespace" "$name" >/dev/null 2>&1 || true
  else
    adb shell settings put "$namespace" "$name" "$value" >/dev/null 2>&1 || true
  fi
}
size_override=$(adb shell wm size | tr -d '\r' | sed -n 's/^Override size: *//p')
density_override=$(adb shell wm density | tr -d '\r' | sed -n 's/^Override density: *//p')
# The display as it was: its override, or none.
restore_display() {
  if [ -n "$size_override" ]; then adb shell wm size "$size_override" >/dev/null 2>&1; else adb shell wm size reset >/dev/null 2>&1; fi
  if [ -n "$density_override" ]; then adb shell wm density "$density_override" >/dev/null 2>&1; else adb shell wm density reset >/dev/null 2>&1; fi
}
restore_device() {
  put_back system accelerometer_rotation "$rotation_auto"
  put_back system user_rotation "$rotation_user"
  put_back secure enabled_accessibility_services "$a11y_services"
  put_back secure accessibility_enabled "$a11y_enabled"
  restore_display
}
trap restore_device EXIT
phys_w=$screen_w phys_h=$screen_h
# Turn the device: rotation 0 is its natural orientation (portrait on a phone, landscape on most
# tablets), 1 the other one. Taps and the on-screen check follow the turned size.
turn() {
  adb shell settings put system accelerometer_rotation 0
  adb shell settings put system user_rotation "$1"
  if [ "$1" = 1 ] || [ "$1" = 3 ]; then screen_w=$phys_h screen_h=$phys_w; else screen_w=$phys_w screen_h=$phys_h; fi
  sleep 2 # the rotation's animation and the app's new layout
}

step "UPDATE AND CAMERA"
page 30-settings '^tab-settings$' '^phone-settings$'
page 31-about '^关于 Voltip$|^About Voltip$' 'AGPL-3.0'
if tap_text '^phone-update-check$' && wait_text '^已是最新|^有新版本|^更新失败|^此版本未配置更新源|^Up to date|^New version|^Update failed|^This build has no update source' 30; then
  measure update_check ok
else
  measure update_check missing
fi
shot 32-update-check
back
back
if wait_text "$TALK" 10 && tap_text "$TALK" && tap_text '^配对电脑$|^Pair a computer$' && tap_text '^pair-open-camera$' && wait_text '^scanner-close$' 15; then
  sleep 2 # the camera preview starts
  shot 33-scanner
  if running; then measure scanner ok; else measure scanner closed; fi
else
  measure scanner missing
  shot 33-scanner
fi
back_until "$TALK" 4 || true

step "TURNED"
front || true
turn 1
if wait_text '按住说话|Hold to talk' 20; then measure turned_talk ok; else measure turned_talk missing; fi
shot 40-turned-talk
# A take in landscape runs its course as in portrait: a result, or a reason (the room is silent).
# The talk card can sit below the fold of a short landscape page; the page scrolls by a swipe at a
# sixth of its width, clear of the hold-to-talk button in the middle (a swipe over it is a take).
side_scroll() {
  local x=$((screen_w / 6)) from=$((screen_h * 7 / 10)) to=$((screen_h * 3 / 10))
  [ "$1" = up ] && { from=$((screen_h * 3 / 10)); to=$((screen_h * 7 / 10)); }
  adb shell input swipe "$x" "$from" "$x" "$to" 400
  sleep 1 # the fling settles
}
settle || true
held=no scrolled=0
for way in here down down; do
  if [ "$way" = down ]; then side_scroll down; scrolled=$((scrolled + 1)); fi
  if dump && xy=$(find_node '^phone-mic-hold$'); then
    read -r x y <<<"$xy"
    n=$(phases | wc -l)
    adb shell input swipe "$x" "$y" "$x" "$y" 4000
    measure turned_take "$(take_outcome 40b-turned-take "$n")"
    held=yes
    break
  fi
done
[ "$held" = yes ] || measure turned_take missing
shot 40c-turned-take
for ((i = 0; i < scrolled; i++)); do side_scroll up; done
page 40d-turned-device "$TALK" '^指纹$|^Fingerprint$'
page 40e-turned-pair '^配对电脑$|^Pair a computer$' '扫码|Scan'
# The camera's preview in landscape (CameraView follows the window), then back to the pairing page.
if tap_text '^pair-open-camera$' && wait_text '^scanner-close$' 15; then
  sleep 2 # the camera preview starts
  shot 40e2-turned-scanner
  if running; then measure turned_scanner ok; else measure turned_scanner closed; fi
  back
  wait_text '扫码|Scan' 10 || true
else
  measure turned_scanner missing
  shot 40e2-turned-scanner
fi
if tap_text '^输入 6 位验证码$|^Enter 6-digit code$' && wait_text '电脑上显示的 6 位验证码|shown on the computer' 10; then
  measure turned_pair_code ok
else
  measure turned_pair_code missing
fi
shot 40f-turned-pair-code
back_until "$TALK" 4 || true
front || true
page 41-turned-history '^tab-history$' '^今天$|^Today$'
page 42-turned-settings '^tab-settings$' '^phone-settings$'
page 43-turned-speech '^语音模型$|^Speech models$' '内置服务|Built-in service'
if tap_text '^provider-asr-groq$' && wait_text '^provider-form$' 10; then
  measure turned_sheet ok
else
  measure turned_sheet missing
fi
shot 44-turned-sheet
back
back
page 45-turned-recording '^录音$|^Recording$' '最长录音时长|Longest recording'
# A landscape sheet shows its first options and scrolls the rest (30 分钟 is below them).
if tap_text '最长录音时长|Longest' && wait_text '^recording-max-minutes-option-[0-9]+$' 10; then
  measure turned_dropdown ok
else
  measure turned_dropdown missing
fi
shot 46-turned-dropdown
back
back
page 46b-turned-dictionary '^个人词典$|^Dictionary$' '新建词条|New entry'
if tap_text '新建词条|New entry' && wait_text '正确写法|Right spelling' 10; then
  measure turned_editor ok
else
  measure turned_editor missing
fi
shot 46c-turned-editor
back
back
if tap_text '^tab-talk$' && wait_text "$TALK" 10; then measure turned_back_to_talk ok; else measure turned_back_to_talk missing; fi
if running; then measure turned_stays_up yes; else measure turned_stays_up no; fi
turn 0

step "UNFOLDED"
front || true
# Device Farm has no foldable phone (2026-10-28): on a phone the display takes a foldable's inner
# size and density, 1840 × 2208 at 420 dpi (701 dp smallest width, a large screen for Android 16),
# while the app runs, as unfolding does, and then the phone's own again (folding). A tablet is large
# already and skips this. The restore is the EXIT trap's as well.
smallest_dp=$(( (phys_w < phys_h ? phys_w : phys_h) * 160 / density ))
measure smallest_width_dp "$smallest_dp"
if [ "$smallest_dp" -lt 600 ]; then
  adb shell wm size 1840x2208
  adb shell wm density 420
  screen_w=1840 screen_h=2208 density_before=$density density=420
  sleep 3 # the new configuration reaches the app
  if wait_text '按住说话|Hold to talk' 20; then measure unfolded_talk ok; else measure unfolded_talk missing; fi
  shot 47-unfolded-talk
  page 48-unfolded-settings '^tab-settings$' '^phone-settings$'
  if tap_text '^tab-talk$' && wait_text "$TALK" 10; then measure unfolded_back_to_talk ok; else measure unfolded_back_to_talk missing; fi
  restore_display
  screen_w=$phys_w screen_h=$phys_h density=$density_before
  sleep 3
  if wait_text '按住说话|Hold to talk' 20 && running; then measure folded_again ok; else measure folded_again missing; fi
  shot 49-folded-again
else
  measure unfolded_talk tablet
  measure unfolded_back_to_talk tablet
  measure folded_again tablet
fi

step "FLOATING BUBBLE"
front || true
# The bubble's own window (BubbleOverlay titles it VoltipBubble): "x1 y1 x2 y2", or nothing.
bubble_frame() {
  adb shell dumpsys window windows 2>/dev/null | tr -d '\r' | python3 -c '
import re, sys
for block in re.split(r"\n(?=  Window #)", sys.stdin.read()):
    if "VoltipBubble" not in block.split("\n", 1)[0]:
        continue
    m = re.search(r"\b(?:mFrame|frame)=\[(-?\d+),(-?\d+)\]\[(-?\d+),(-?\d+)\]", block)
    if m:
        print(*m.groups())
        break
'
}
bubble_width() {
  local frame
  frame=$(bubble_frame)
  [ -n "$frame" ] || return 1
  # shellcheck disable=SC2086
  set -- $frame
  echo $(($3 - $1))
}
handle_px=$(dp 32)
# The bubble's take from its centre: $1 = how far toward the middle of the screen the finger moves
# (0 = stays put; the bubble starts docked at the right), $2 = for how many milliseconds.
bubble_take() {
  local frame n
  frame=$(bubble_frame) || return 1
  [ -n "$frame" ] || return 1
  # shellcheck disable=SC2086
  set -- $frame "$1" "$2"
  local x=$((($1 + $3) / 2)) y=$((($2 + $4) / 2))
  n=$(phases | wc -l)
  adb shell input swipe "$x" "$y" $((x - $(dp "$5"))) "$y" "$6"
  take_outcome "bubble-$5" "$n"
}
adb shell pm grant "$PKG" android.permission.POST_NOTIFICATIONS >/dev/null 2>&1 || true
adb shell appops set "$PKG" SYSTEM_ALERT_WINDOW allow
page 50-bubble-page '^tab-settings$' '^phone-settings$'
if tap_text '^settings-bubble$' && wait_text '^bubble-toggle$' 10; then
  tap_text '^bubble-toggle$'
  if wait_text '^bubble-mode$' 10 && dump && checked '^bubble-toggle$'; then measure bubble_on ok; else measure bubble_on missing; fi
else
  measure bubble_on missing
fi
shot 51-bubble-settings
# 手势粘贴's service, enabled as the system's Accessibility settings would; the page reads it again
# when it comes back into view.
adb shell settings put secure enabled_accessibility_services "$PKG/dev.voltip.rn.BubblePasteService"
adb shell settings put secure accessibility_enabled 1
sleep 3 # the system binds the service
back
tap_text '^settings-bubble$' || true
if tap_text '^bubble-paste-enabled$' && wait_text '^shell-confirm$' 10; then
  shot 52-paste-disclosure
  tap_text '^shell-confirm$'
fi
sleep 2 # saved, the page read again
if dump && checked '^bubble-paste-enabled$' && ! find_node '^bubble-paste-needs-service$' >/dev/null; then
  measure paste_service connected
else
  measure paste_service missing
fi
shot 53-paste-on
adb shell input keyevent KEYCODE_HOME
sleep 2 # the launcher, the bubble over it
shot 54-bubble-home
if [ -n "$(bubble_frame)" ]; then measure bubble_shown ok; else measure bubble_shown missing; fi
# With 手势粘贴 on: a swipe toward the middle while held. No text field has focus on the launcher,
# so the result stays on the clipboard; the take must still run its course.
outcome=$(bubble_take 90 4000 || echo missing)
measure bubble_take_paste_on "$outcome"
grep -hE "paste requested|not pasted" <(adb logcat -d -s VoltipBubble 2>/dev/null) | tail -2
shot 55-bubble-paste-on
# 手势粘贴 off again, the same take held in place.
adb shell am start -n "$PKG/.MainActivity" >/dev/null 2>&1
wait_text '^bubble-paste-enabled$' 15 || tap_text '^settings-bubble$' || true
tap_text '^bubble-paste-enabled$' || true
sleep 2 # saved
if dump && ! checked '^bubble-paste-enabled$'; then measure paste_off ok; else measure paste_off missing; fi
adb shell input keyevent KEYCODE_HOME
sleep 2
outcome=$(bubble_take 0 4000 || echo missing)
measure bubble_take_paste_off "$outcome"
shot 56-bubble-paste-off
# 空闲时自动收起 at 5 s, chosen only now: the takes above ran with it off (a tucked-in bubble is a
# handle, and a swipe from a handle pulls the bubble out instead of recording).
adb shell am start -n "$PKG/.MainActivity" >/dev/null 2>&1
wait_text '^bubble-paste-enabled$' 15 || tap_text '^settings-bubble$' || true
tap_text '^bubble-auto-stow-5$' || true
if dump && checked '^bubble-auto-stow-5$'; then measure auto_stow_option ok; else measure auto_stow_option missing; fi
adb shell input keyevent KEYCODE_HOME
# Within 25 s (5 s idle, and a busy device's slack) the bubble's window is the handle's width.
until_handle() {
  local deadline=$((SECONDS + 25)) width=0
  while [ "$SECONDS" -lt "$deadline" ]; do
    width=$(bubble_width || echo 0)
    if [ "$width" -ge $((handle_px - 2)) ] && [ "$width" -le $((handle_px + 2)) ]; then return 0; fi
    sleep 1
  done
  return 1
}
# A tap in the middle of the bubble's window: on the handle it brings the bubble out.
tap_bubble() {
  local frame
  frame=$(bubble_frame)
  [ -n "$frame" ] || return 1
  # shellcheck disable=SC2086
  set -- $frame
  adb shell input tap $((($1 + $3) / 2)) $((($2 + $4) / 2))
  sleep 1 # the bubble slides out
}
stowed_idle=no stowed_again=no
until_handle && stowed_idle=yes
shot 57-bubble-stowed
# Out again by a tap on the handle, then idle once more: in again.
tap_bubble || true
out=$(bubble_width || echo 0)
until_handle && stowed_again=yes
measure bubble_width_out_px "$out"
measure bubble_width_handle_px "$(bubble_width || echo 0)"
if [ "$stowed_idle" = yes ] && [ "$out" -gt $((handle_px + 2)) ] && [ "$stowed_again" = yes ]; then measure auto_stow ok; else measure auto_stow "no:$stowed_idle/$out/$stowed_again"; fi
# The handle brings the bubble back; held for 8 s it stays out the whole time.
tap_bubble || true
if frame=$(bubble_frame) && [ -n "$frame" ]; then
  # shellcheck disable=SC2086
  set -- $frame
  n=$(phases | wc -l)
  adb shell input swipe $((($1 + $3) / 2)) $((($2 + $4) / 2)) $((($1 + $3) / 2)) $((($2 + $4) / 2)) 8000 &
  holder=$!
  sleep 6
  during=$(bubble_width || echo 0)
  wait "$holder"
  measure bubble_width_held_px "$during"
  if [ "$during" -gt $((handle_px + 2)) ]; then measure hold_keeps_bubble ok; else measure hold_keeps_bubble no; fi
  measure bubble_take_held "$(take_outcome bubble-held "$n")"
else
  measure hold_keeps_bubble missing
fi
# Turned with Voltip in front (the app turns; a launcher may stay upright): the bubble's window
# stays on the screen as it is now, read from the display's current size.
display_now() {
  adb shell dumpsys window displays 2>/dev/null | tr -d '\r' | sed -n 's/.* cur=\([0-9]*\)x\([0-9]*\).*/\1 \2/p' | head -1
}
adb shell am start -n "$PKG/.MainActivity" >/dev/null 2>&1
turn 1
frame=$(bubble_frame)
now=$(display_now)
measure bubble_frame_turned "${frame:-none}"
measure display_turned "${now:-unknown}"
if [ -n "$frame" ] && [ -n "$now" ]; then
  # shellcheck disable=SC2086
  set -- $frame $now
  if [ "$1" -ge 0 ] && [ "$2" -ge 0 ] && [ "$3" -le "$5" ] && [ "$4" -le "$6" ] && [ "$5" -gt "$6" ]; then
    measure bubble_turns ok
  elif [ "$5" -le "$6" ]; then
    measure bubble_turns not_turned
  else
    measure bubble_turns off_screen
  fi
else
  measure bubble_turns missing
fi
shot 58-bubble-turned
turn 0
if running; then measure bubble_stays_up yes; else measure bubble_stays_up no; fi
restore_device

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
# What R8 could have removed or renamed that something still looks up by name: the app's own lines
# (its uid's), over the whole run.
uid=$(adb shell cmd package list packages -U "$PKG" 2>/dev/null | tr -d '\r' | sed -n "s/^package:$PKG uid://p" | head -1)
if [ -n "$uid" ] && adb logcat -d --uid="$uid" >"$out/app-uid-logcat.txt" 2>/dev/null; then :; else cp "$out/logcat-run.txt" "$out/app-uid-logcat.txt"; fi
lost='ClassNotFoundException|NoClassDefFoundError|NoSuchMethodError|NoSuchFieldError|UnsatisfiedLinkError'
if grep -qE "$lost" "$out/app-uid-logcat.txt"; then
  measure r8_errors found
  grep -E "$lost" "$out/app-uid-logcat.txt" | head -20
else
  measure r8_errors none
fi
grep -E "voltip|ReactNativeJS" "$out/logcat-run.txt" 2>/dev/null | grep -vE "nativeloader" | tail -60
echo "summary:"
cat "$summary"
