#!/usr/bin/env bash
# Drive the menu bar item of an installed Voltip on a real macOS desktop: the item shows the mark,
# its menu lists the build's entries, and each entry does what it says (docs/dictation.md §15.4).
# install-scripts.yml runs it on the published release, on Apple silicon and on Intel.
#
#   1. the app starts, the tray is installed (the log names the menu's language and whether the
#      build has an update source), and the main window's close button hides it;
#   2. the status item's pixels are the template V (tray-icon.png): strokes that stand out from
#      the menu bar, and no colour icon;
#   3. its menu holds exactly the build's entries in the UI's language;
#   4. Open shows the main window; Settings shows it with the Settings dialog open (the webview's
#      role=dialog in the accessibility tree); Check for Updates asks the update source and gets
#      an answer; Quit ends the process with exit code 0.
# The Accessibility API does the clicking (scripts/tray-ax.swift, compiled here and granted the
# permission in the runner's writable TCC database, as ci.yml does for the event tap test).
#
# Usage: scripts/smoke-tray-macos.sh [app bundle (default /Applications/Voltip.app)] [out dir]
#        VOLTIP_NO_UPDATER=1 for a build without an update source.
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

helper=$out/tray-ax
swiftc -O -o "$helper" "$here/tray-ax.swift"
db="/Library/Application Support/com.apple.TCC/TCC.db"
grant() { # <service> <client path>
  sudo sqlite3 "$db" "INSERT OR REPLACE INTO access (service, client, client_type, auth_value, auth_reason, auth_version, indirect_object_identifier, flags, last_modified) VALUES ('$1', '$2', 1, 2, 4, 1, 'UNUSED', 0, $(date +%s));"
}
grant kTCCServiceAccessibility "$helper"
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
  labels=("打开 Voltip" "设置…" "检查更新…" "退出 Voltip")
  title="设置"
else
  labels=("Open Voltip" "Settings…" "Check for Updates…" "Quit Voltip")
  title="Settings"
fi
expected=("${labels[0]}" "${labels[1]}")
[ "$updater" = 1 ] && expected+=("${labels[2]}")
expected+=("${labels[3]}")
wait_for 60 'the main window' window_shown
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
ax press "${expected[0]}"
wait_for 30 'the Open entry in the log' log_has 'tray menu action=Open'
wait_for 30 'Open to show the window' window_shown
note 'Open: main window shown'
ax close Voltip
wait_for 30 'the window to hide' window_hidden

# 4b. Settings: the window with the Settings dialog.
ax press "${expected[1]}"
wait_for 30 'the Settings entry in the log' log_has 'tray menu action=Settings'
wait_for 30 'Settings to show the window' window_shown
note "Settings: window shown with $(ax dialog "$title")"
screencapture -x "$out/settings.png"
ax close Voltip
wait_for 30 'the window to hide' window_hidden

# 4c. Check for Updates: the webview asks the update source and gets an answer.
if [ "$updater" = 1 ]; then
  ax press "${expected[2]}"
  wait_for 30 'the Check for Updates entry in the log' log_has 'tray menu action=CheckUpdate'
  wait_for 60 'the update check to answer' log_has '(no update available|update available)'
  wait_for 30 'Check for Updates to show the window' window_shown
  screencapture -x "$out/update.png"
  note "Check for Updates: $(log_text | grep -E -o '(no update available|update available)[^[:cntrl:]]*' | head -1)"
fi

# 4d. Quit.
ax press "${expected[${#expected[@]} - 1]}"
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
