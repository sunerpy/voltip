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
#   2. the status item's pixels are the template V (tray-icon.png): strokes that stand out from
#      the menu bar, and no colour icon;
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

helper=$out/tray-ax
swiftc -O -o "$helper" "$here/tray-ax.swift"
db="/Library/Application Support/com.apple.TCC/TCC.db"
grant() { # <service> <client path>
  sudo sqlite3 "$db" "INSERT OR REPLACE INTO access (service, client, client_type, auth_value, auth_reason, auth_version, indirect_object_identifier, flags, last_modified) VALUES ('$1', '$2', 1, 2, 4, 1, 'UNUSED', 0, $(date +%s));"
}
grant kTCCServiceAccessibility "$helper"
grant kTCCServicePostEvent "$helper"
grant kTCCServiceScreenCapture /usr/sbin/screencapture
ax() { "$helper" "$pid" "$@"; }
window_shown() { ax windows | grep -qx Voltip; }
window_hidden() { ! window_shown; }

pkill -x voltip-desktop || true
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
  labels=("打开 Voltip" "AI 润色" "设置…" "检查更新…" "退出 Voltip")
  title="设置"
  toggle="启用 AI 润色"
  presets=("校对" "提示词优化" "意图整理" "口语聊天" "中英互译" "要点纪要" "只加标点" "书面语")
else
  labels=("Open Voltip" "AI Polish" "Settings…" "Check for Updates…" "Quit Voltip")
  title="Settings"
  toggle="Enable AI Polish"
  presets=("Proofread" "Prompt optimizer" "Clarify intent" "Casual chat" "Chinese ⇄ English" "Key points" "Punctuation only" "Formal")
fi
open_label=${labels[0]} polish_label=${labels[1]} settings_label=${labels[2]} update_label=${labels[3]} quit_label=${labels[4]}
expected=("$open_label" "$polish_label" "$settings_label")
[ "$updater" = 1 ] && expected+=("$update_label")
expected+=("$quit_label")
wait_for 60 'the main window' window_shown
# The window is mapped before its webview draws: wait for the brand's text, then for pixels.
ax wait-text Voltip Voltip >/dev/null
read -r wx wy ww wh zoom_right <<<"$(ax chrome Voltip)"
strip_w=$((ww < 480 ? ww : 480))
measure_strip() {
  screencapture -x -R "$wx,$wy,$strip_w,40" "$out/window-top.png"
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

# 2. The status item is the template V.
read -r x y w h <<<"$(ax frame)"
screencapture -x -R "$x,$y,$w,$h" "$out/tray-icon.png"
stats=$("$helper" ink "$out/tray-icon.png")
note "status item at $x,$y ${w}x${h} pt: $stats"
ink=$(sed -n 's/.*ink=\([0-9.]*\).*/\1/p' <<<"$stats")
blue=$(sed -n 's/.*blue=\([0-9.]*\).*/\1/p' <<<"$stats")
awk -v ink="$ink" -v blue="$blue" 'BEGIN { exit !(ink >= 0.03 && blue < 0.02) }' ||
  fail "the status item is not the Voltip V (see tray-icon.png)"

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
ax doubleclick
wait_for 30 'the double click in the log' log_count_above 'tray double click: main window' "$before"
wait_for 30 'the double click to show the window' window_shown
# The first click of the pair must open nothing when its interval ends.
! settles $((interval / 1000 + 2)) ax menu-open || fail 'a double click opened the menu as well as the window'
note 'double click: main window shown, no menu'
ax close Voltip
wait_for 30 'the window to hide' window_hidden

# 4f. Quit.
ax press "$quit_label"
# Gone, or a zombie waiting to be reaped (kill -0 still answers for one).
process_gone() {
  local stat
  stat=$(ps -p "$pid" -o stat= 2>/dev/null || true)
  [ -z "$stat" ] || [[ $stat == Z* ]]
}
wait_for 30 'Quit to end the process' process_gone
status=0
wait "$pid" || status=$?
log_has 'tray menu action=Quit' || fail 'the process ended without the Quit entry in the log'
note "Quit: process exited with $status"
[ "$status" = 0 ] || fail "exit code $status"
note OK
