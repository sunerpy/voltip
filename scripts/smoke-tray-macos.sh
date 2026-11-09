#!/usr/bin/env bash
# Drive the menu bar item of an installed Voltip on a real macOS desktop: the item shows the mark,
# its menu lists the build's entries, and each entry does what it says (docs/dictation.md §15.4).
# install-scripts.yml runs it on the published release, on Apple silicon and on Intel.
#
#   1. the app starts, the tray is installed (the log names the menu's language and whether the
#      build has an update source), the title bar's left end is pictured (window-top.png) with the
#      space measured between the traffic lights and whatever is drawn next (at least 12 pt, a hard
#      check with VOLTIP_CHECK_CHROME=1 as CI sets for the build under test; user feedback
#      2026-09-29: they crowded the app mark), and the main window's close button hides it;
#   2. the status item's pixels are the template mark (tray-icon.png), the sound bars and the
#      cursor: strokes that stand out from the menu bar, and no colour icon;
#   3. its menu holds exactly the build's entries in the UI's language;
#   4. Open shows the main window; Settings shows it with the Settings dialog open (the webview's
#      role=dialog in the accessibility tree); Check for Updates asks the update source and gets
#      an answer; the AI Polish submenu lists its switch and every preset with the ones in use
#      checked, a preset and the switch reach the core, the menu is rebuilt from what it saved,
#      and choosing the preset in use keeps it checked (docs/dictation.md §21); a real click on
#      the item opens the menu once the double-click interval has passed, and a real double
#      click shows the main window and opens no menu (user request 2026-09-30; releases before
#      0.0.16 fail here by design); Quit ends the process with exit code 0.
# The Accessibility API does the clicking, except for the real clicks, which are mouse events
# (scripts/tray-ax.swift, compiled here and granted both permissions in the runner's writable TCC
# database, as ci.yml does for the event tap test).
#
# With VOLTIP_CHECK_UI_GUARD=1 (CI, for the build under test) the log must also show that the
# interface check (docs/dictation.md §33.4) ran in the main window and in the pill window and found
# the program's interface in both, with no window replaced; releases without the check fail it.
# The pill window is prewarmed hidden, and a hidden window may load its page only once it is first
# shown (apps/desktop/src-tauri/src/overlay.rs, `OverlaySlot`): the pill is shown once (step 5)
# before its check is read. `--edit-toggle` starts a voice edit take, which a build without an AI
# polish service (CI's) refuses at once, before the microphone opens; the pill shows the refusal.
# The take needs a recogniser to start at all, so for the run the profile's settings name a custom
# one at a loopback port nothing listens on (it is never asked); the profile's own settings are put
# back when the script ends.
#
# Usage: scripts/smoke-tray-macos.sh [app bundle (default /Applications/Voltip.app)] [out dir]
#        VOLTIP_NO_UPDATER=1 for a build without an update source; VOLTIP_CHECK_CHROME=1 to fail
#        when the traffic lights crowd the title bar (releases before 0.0.6 do).
set -euo pipefail

bundle=${1:-/Applications/Voltip.app}
out=${2:-smoke-tray-macos}
here=$(cd "$(dirname "$0")" && pwd)
mkdir -p "$out"
out=$(cd "$out" && pwd)
summary=$out/summary.txt
: >"$summary"
log=$out/app.log
note() { echo "smoke-tray-macos: $*" | tee -a "$summary"; }
fail() {
  note "FAIL: $*"
  exit 1
}
# wait_for <seconds> <what> <command…>: poll the condition every 250 ms until it holds.
wait_for() {
  local seconds=$1 what=$2 deadline
  shift 2
  deadline=$((SECONDS + seconds))
  until "$@"; do
    [ "$SECONDS" -lt "$deadline" ] || fail "timed out after ${seconds}s waiting for $what"
    sleep 0.25
  done
}
log_text() { sed $'s/\x1b\\[[0-9;]*m//g' "$log" 2>/dev/null || true; }
log_has() { log_text | grep -E -- "$1" >/dev/null; }
log_count() { log_text | grep -E -c -- "$1" || true; }
log_count_above() { [ "$(log_count "$1")" -gt "$2" ]; }
# settles <seconds> <command…>: whether the condition holds within the time (no failure).
settles() {
  local deadline=$((SECONDS + $1))
  shift
  until "$@"; do
    [ "$SECONDS" -lt "$deadline" ] || return 1
    sleep 0.25
  done
}

# gone <pid>: the process has ended, or is a zombie waiting to be reaped (kill -0 still answers
# for one).
gone() {
  local stat
  stat=$(ps -p "$1" -o stat= 2>/dev/null || true)
  [ -z "$stat" ] || [[ $stat == Z* ]]
}
# stop <pid>: end a process this script started, with SIGKILL when it is still there 10 s later.
stop() {
  kill "$1" 2>/dev/null || return 0
  settles 10 gone "$1" || kill -9 "$1" 2>/dev/null || true
  wait "$1" 2>/dev/null || true
}

helper=$out/tray-ax
swiftc -O -o "$helper" "$here/tray-ax.swift"
"$here/tcc-grant-macos.sh" "$helper" kTCCServiceAccessibility kTCCServicePostEvent
"$here/tcc-grant-macos.sh" /usr/sbin/screencapture kTCCServiceScreenCapture
ax() { "$helper" "$pid" "$@"; }
window_shown() { ax windows | grep -qx Voltip; }
window_hidden() { ! window_shown; }

pkill -x voltip-desktop || true
pid='' second=''
if [ "${VOLTIP_CHECK_UI_GUARD:-0}" = 1 ]; then
  # The settings for step 5 (see the head of this file), on top of the profile's own; a profile
  # without settings gets the fields a settings file must name at the app's defaults.
  profile="$HOME/Library/Application Support/dev.voltip.Voltip"
  kept=$(mktemp -d)
  had_settings=0
  if [ -f "$profile/settings.json" ]; then
    had_settings=1
    cp -p "$profile/settings.json" "$kept/settings.json"
  fi
  # The app saves its settings when they change: it is stopped before the profile's come back.
  # Once only: a signal while it runs does not start it again.
  restore_profile() {
    trap - EXIT INT TERM
    [ -z "$second" ] || stop "$second"
    [ -z "$pid" ] || stop "$pid"
    if [ "$had_settings" = 0 ]; then
      rm -f "$profile/settings.json"
      rm -rf "$kept"
    elif cp -p "$kept/settings.json" "$profile/settings.json"; then
      rm -rf "$kept"
    else
      note "FAIL: could not put back $profile/settings.json; the profile's own copy is in $kept"
    fi
  }
  trap restore_profile EXIT
  trap 'exit 1' INT TERM
  mkdir -p "$profile"
  if [ "$had_settings" = 1 ]; then base=$(cat "$kept/settings.json"); else base='{"schema":1,"theme":"glass","follow_system_theme":false,"relay_enabled":true}'; fi
  jq '.engines.asr_provider = "custom" | .engines.providers.custom.asr_url = "http://127.0.0.1:9" | .engines.providers.custom.asr_model = "voltip-smoke"' \
    <<<"$base" >"$profile/settings.json"
  note "settings for the pill step: a custom recogniser at http://127.0.0.1:9 on top of $([ "$had_settings" = 1 ] && echo "the profile's settings" || echo 'the defaults (the profile had none)')"
fi
RUST_LOG=voltip=info NO_COLOR=1 "$bundle/Contents/MacOS/voltip-desktop" 2>"$log" >"$out/app.out" &
pid=$!
note "started $bundle/Contents/MacOS/voltip-desktop (pid $pid, $(shasum -a 256 "$bundle/Contents/MacOS/voltip-desktop" | cut -d' ' -f1))"

# 1. The tray, the main window, and the window hidden (so Open has something to show).
wait_for 120 'the tray install line' log_has 'tray icon (not )?installed'
installed=$(log_text | grep -E -o 'tray icon (not )?installed[^[:cntrl:]]*' | head -1)
case $installed in *'not installed'*) fail "$installed" ;; esac
note "$installed"
case $installed in *locale=ZhCn*) zh=1 ;; *) zh=0 ;; esac
case $installed in *updater=true*) updater=1 ;; *) updater=0 ;; esac
if [ "${VOLTIP_NO_UPDATER:-0}" = 1 ]; then want_updater=0; else want_updater=1; fi
[ "$updater" = "$want_updater" ] || fail "updater=$updater, expected $want_updater"
if [ "$zh" = 1 ]; then
  labels=("打开 Voltip" "复制上一条结果" "AI 润色" "设置…" "检查更新…" "退出 Voltip")
  title="设置"
  toggle="启用 AI 润色"
  presets=("校对" "提示词优化" "意图整理" "口语聊天" "中英互译" "要点纪要" "只加标点" "书面语")
else
  labels=("Open Voltip" "Copy Last Result" "AI Polish" "Settings…" "Check for Updates…" "Quit Voltip")
  title="Settings"
  toggle="Enable AI Polish"
  presets=("Proofread" "Prompt optimizer" "Clarify intent" "Casual chat" "Chinese ⇄ English" "Key points" "Punctuation only" "Formal")
fi
# Copy Last Result (docs/dictation.md §28) is listed, greyed until there is a result.
open_label=${labels[0]} copy_last_label=${labels[1]} polish_label=${labels[2]} settings_label=${labels[3]} update_label=${labels[4]} quit_label=${labels[5]}
expected=("$open_label" "$copy_last_label" "$polish_label" "$settings_label")
[ "$updater" = 1 ] && expected+=("$update_label")
expected+=("$quit_label")
wait_for 60 'the main window' window_shown
# The window is mapped before its webview draws: wait for the brand's text, then for pixels.
ax wait-text Voltip Voltip >/dev/null
ui_guard_ok='the window runs the interface the program carries window='
if [ "${VOLTIP_CHECK_UI_GUARD:-0}" = 1 ]; then
  wait_for 30 "the main window's interface check" log_has "${ui_guard_ok}\"main\""
  note "interface check: $(log_text | grep -E -o "${ui_guard_ok}\"main\"[^[:cntrl:]]*" | head -1)"
fi
read -r wx wy ww wh zoom_right <<<"$(ax chrome Voltip)"
strip_w=$((ww < 480 ? ww : 480))
measure_strip() {
  # From 0.1.1 the lights sit in the floating sidebar's brand row (trafficLightPosition y 42, 12 pt
  # tall): picture the 32 pt band around them, which the sidebar's glass fills on both sides.
  screencapture -x -R "$wx,$((wy + 32)),$strip_w,32" "$out/window-top.png"
  scale=$(sips -g pixelWidth "$out/window-top.png" | awk -v w="$strip_w" '/pixelWidth/ { printf "%d", $2 / w }')
  gap=$("$helper" gap "$out/window-top.png" $(((zoom_right - wx) * scale)) 2>/dev/null | sed -n 's/^gap=//p')
  [ -n "$gap" ]
}
wait_for 20 'the title bar to draw past the traffic lights' measure_strip
gap_pt=$((gap / scale))
note "window at $wx,$wy ${ww}x${wh} pt: the first thing after the traffic lights starts ${gap_pt} pt after them (window-top.png)"
if [ "${VOLTIP_CHECK_CHROME:-0}" = 1 ]; then
  [ "$gap_pt" -ge 12 ] || fail "the traffic lights crowd the brand: ${gap_pt} pt between them (see window-top.png)"
fi
ax close Voltip
wait_for 30 'the window to hide' window_hidden
note 'main window hidden by its close button; the process keeps running'

# 2. The status item is the template mark.
read -r x y w h <<<"$(ax frame)"
screencapture -x -R "$x,$y,$w,$h" "$out/tray-icon.png"
stats=$("$helper" ink "$out/tray-icon.png")
note "status item at $x,$y ${w}x${h} pt: $stats"
ink=$(sed -n 's/.*ink=\([0-9.]*\).*/\1/p' <<<"$stats")
blue=$(sed -n 's/.*blue=\([0-9.]*\).*/\1/p' <<<"$stats")
awk -v ink="$ink" -v blue="$blue" 'BEGIN { exit !(ink >= 0.03 && blue < 0.02) }' ||
  fail "the status item is not the Voltip mark (see tray-icon.png)"

# 3. The menu.
menu=$(ax menu)
note "menu: $(tr '\n' '|' <<<"$menu" | sed 's/|$//; s/|/ | /g')"
[ "$menu" = "$(printf '%s\n' "${expected[@]}")" ] || fail "menu differs from: ${expected[*]}"

# 4a. Open.
ax press "$open_label"
wait_for 30 'the Open entry in the log' log_has 'tray menu action=Open'
wait_for 30 'Open to show the window' window_shown
note 'Open: main window shown'
ax close Voltip
wait_for 30 'the window to hide' window_hidden

# 4b. Settings: the window with the Settings dialog.
ax press "$settings_label"
wait_for 30 'the Settings entry in the log' log_has 'tray menu action=Settings'
wait_for 30 'Settings to show the window' window_shown
note "Settings: window shown with $(ax dialog "$title")"
screencapture -x "$out/settings.png"
ax close Voltip
wait_for 30 'the window to hide' window_hidden

# 4c. Check for Updates: the webview asks the update source and gets an answer.
if [ "$updater" = 1 ]; then
  ax press "$update_label"
  wait_for 30 'the Check for Updates entry in the log' log_has 'tray menu action=CheckUpdate'
  wait_for 60 'the update check to answer' log_has '(no update available|update available)'
  wait_for 30 'Check for Updates to show the window' window_shown
  screencapture -x "$out/update.png"
  note "Check for Updates: $(log_text | grep -E -o '(no update available|update available)[^[:cntrl:]]*' | head -1)"
fi

# 4d. AI Polish (docs/dictation.md §21): the switch and every preset (a fresh profile has no
# custom one), the ones in use checked; each choice goes through the core and the menu is rebuilt
# from what it saved.
polish_menu() { # <entries that must carry the ✓, in order>
  local want=("$@") entry line checked lines=()
  for entry in "$toggle" "${presets[@]}"; do
    line=$entry
    for checked in "${want[@]}"; do [ "$checked" = "$entry" ] && line="$entry"$'\t'"✓"; done
    lines+=("$line")
  done
  printf '%s\n' "${lines[@]}"
}
polish_shows() { [ "$(ax submenu "$polish_label")" = "$(polish_menu "$@")" ]; }
# press_polish <entry> <the log line the app writes for it>: choose the entry and wait for that line
# to appear once more. An accessibility press on a menu entry can report success and still never
# reach the app (CI 2026-09-29: the switch, pressed right after a read of the submenu, left no
# `tray menu polish` line, while the next press did); a press the app did not log within 10 s is
# made once more, and the summary says so.
press_polish() {
  local entry=$1 line=$2 before
  before=$(log_count "$line")
  ax press-sub "$polish_label" "$entry"
  if ! settles 10 log_count_above "$line" "$before"; then
    note "AI Polish: the press on $entry did not reach the app; pressed again"
    ax press-sub "$polish_label" "$entry"
  fi
  wait_for 30 "$entry in the log" log_count_above "$line" "$before"
}
note "AI Polish: $(ax submenu "$polish_label" | tr '\t\n' ' |' | sed 's/|$//; s/|/ | /g')"
polish_shows "$toggle" "${presets[0]}" || fail "the AI Polish submenu differs from: $(polish_menu "$toggle" "${presets[0]}" | tr '\t\n' ' |')"
press_polish "${presets[1]}" 'tray menu polish action=Preset\("prompt"\)'
wait_for 30 'the second preset checked' polish_shows "$toggle" "${presets[1]}"
note "AI Polish: chose ${presets[1]}"
# The preset in use again: the menu must still show it checked.
press_polish "${presets[1]}" 'tray menu polish action=Preset\("prompt"\)'
wait_for 30 'the preset in use still checked' polish_shows "$toggle" "${presets[1]}"
press_polish "$toggle" 'tray menu polish action=Toggle'
wait_for 30 'the switch off' polish_shows "${presets[1]}"
note 'AI Polish: switched off'
# Leave the profile as it was found: the switch on, the first preset.
press_polish "$toggle" 'tray menu polish action=Toggle'
wait_for 30 'the switch on again' polish_shows "$toggle" "${presets[1]}"
press_polish "${presets[0]}" 'tray menu polish action=Preset\("proofread"\)'
wait_for 30 'the defaults back' polish_shows "$toggle" "${presets[0]}"

# 4e. Real clicks on the item (user request 2026-09-30), mouse events at its centre rather than
# accessibility presses: a click opens the menu once the double-click interval has passed (the app
# logs the interval it waited), a double click shows the main window and opens no menu.
if window_shown; then
  ax close Voltip
  wait_for 30 'the window to hide' window_hidden
fi
before=$(log_count 'tray click: menu')
delay=$(ax click)
wait_for 10 'the click in the log' log_count_above 'tray click: menu' "$before"
interval=$(log_text | sed -n 's/.*tray click: menu after_ms=\([0-9]*\).*/\1/p' | tail -1)
[ -n "$interval" ] || fail 'the click line in the log names no interval'
[ "$delay" -ge "$interval" ] || fail "the menu opened ${delay} ms after the click, inside the double-click interval (${interval} ms)"
window_hidden || fail 'a click showed the main window'
note "click: the menu opened ${delay} ms after it (double-click interval ${interval} ms)"
before=$(log_count 'tray double click: main window')
# Watched past the first click's interval: that click must open nothing when its wait ends.
seen=$(ax doubleclick $((interval + 1500)))
wait_for 30 'the double click in the log' log_count_above 'tray double click: main window' "$before"
wait_for 30 'the double click to show the window' window_shown
[ "$seen" = "no menu" ] || fail 'a double click opened the menu as well as the window'
note 'double click: main window shown, no menu'
ax close Voltip
wait_for 30 'the window to hide' window_hidden

# 5. The pill window, shown once before its interface check is read (see the head of this file):
# a second invocation hands `--edit-toggle` to the running app and exits; the app refuses the edit
# take for want of an AI polish service and shows the pill with the refusal for 2.5 s.
if [ "${VOLTIP_CHECK_UI_GUARD:-0}" = 1 ]; then
  pill_guard="${ui_guard_ok}\"overlay\""
  if log_has "$pill_guard"; then reported_hidden=1; else reported_hidden=0; fi
  before=$(log_count 'edit take refused: no LLM is configured')
  "$bundle/Contents/MacOS/voltip-desktop" --edit-toggle >"$out/edit-toggle.log" 2>&1 &
  second=$!
  wait_for 30 'the second invocation to hand --edit-toggle over and exit' gone "$second"
  handed=0
  wait "$second" || handed=$?
  second=
  [ "$handed" = 0 ] || fail "the --edit-toggle invocation exited with $handed: $(tail -3 "$out/edit-toggle.log")"
  wait_for 30 'the running app to take --edit-toggle' log_has 'remote control (applied|not accepted).*remote=EditToggle'
  ! log_has 'remote control not accepted' || fail "$(log_text | grep -F 'remote control not accepted' | head -1)"
  wait_for 30 'the edit take to be refused (a build without an AI polish service)' \
    log_count_above 'edit take refused: no LLM is configured' "$before"
  pill_shown() { ax windows | grep -qx 'Voltip Overlay'; }
  if settles 2 pill_shown; then seen='on screen'; else seen='not listed by the accessibility API'; fi
  screencapture -x "$out/pill.png"
  wait_for 30 "the pill window's interface check, after the pill was shown" log_has "$pill_guard"
  if [ "$reported_hidden" = 1 ]; then
    note "pill: shown for the refused edit take ($seen, pill.png); its interface check had reported while it was hidden"
  else
    note "pill: shown for the refused edit take ($seen, pill.png); its interface check reported only after that"
  fi
fi

# 6. Quit.
ax press "$quit_label"
wait_for 30 'Quit to end the process' gone "$pid"
status=0
wait "$pid" || status=$?
pid=
log_has 'tray menu action=Quit' || fail 'the process ended without the Quit entry in the log'
note "Quit: process exited with $status"
[ "$status" = 0 ] || fail "exit code $status"
if [ "${VOLTIP_CHECK_UI_GUARD:-0}" = 1 ]; then
  log_has "${ui_guard_ok}\"overlay\"" || fail "the pill window's interface check reported no program interface"
  ! log_has 'ui_guard: ' || [ "$(log_count 'ui_guard: ')" = "$(log_count "ui_guard: ${ui_guard_ok}")" ] ||
    fail "the interface check: $(log_text | grep -F 'ui_guard: ' | grep -vF "$ui_guard_ok" | head -3)"
  note "interface check: both windows run the program's interface, none replaced"
fi
note OK
