#!/bin/sh
# Inside the container of scripts/repro-gnome-remote-desktop.sh (docs/dictation.md §14.9, issue
# #235), on a private session bus: headless GNOME Shell with Xwayland, the accessibility bus, the
# text field the pastes land in (target.py), the session's keyboard and mouse (seat.py), the
# answerer of GNOME's remote-desktop window (answer.py), a log of the portal calls; then the steps
# of SCENARIO with the paste harness /repro-bin/voltip-paste-repro.
#
#   session.sh base|fix
#
# Writes to /repro-out: steps.log (one line per step: pastes, windows answered, what arrived),
# windows.log (answer.py), portal.log (dbus-monitor), target.txt (what the field holds;
# target.<n>.txt from an earlier GNOME Shell; target.txt.events with the times), gnome-shell.log,
# xwayland.txt (the command line of each Xwayland), versions.txt (the packages that run here),
# summary.txt.
# shellcheck disable=SC2016  # `sh -c '…' _ "$x"` expands in the inner shell
set -eu
scenario=$1
out=/repro-out
repro=/repro
bin=/repro-bin/voltip-paste-repro
state=$(mktemp -d)
export VOLTIP_REPRO_TOKEN_FILE="$state/token"
echo allow > "$state/answer"
: > "$out/windows.log"
: > "$out/xwayland.txt"

# What runs here, by installed package: the versions docs/dictation.md §14.9 quotes.
{
  dpkg-query -W -f '${db:Status-Abbrev}${Package} ${Version}\n' gnome-shell 'libmutter-*' mutter-common \
    xwayland xdg-desktop-portal xdg-desktop-portal-gnome 'liboeffis*' 2>/dev/null | sed -n 's/^ii *//p'
  gnome-shell --version 2>/dev/null || true
} > "$out/versions.txt"

pids=""
cleanup() {
  for pid in $pids; do kill "$pid" 2>/dev/null || true; done
}
trap cleanup EXIT

# Wait until COMMAND succeeds, at most SECONDS; a short poll, never a guessed sleep.
until_ok() {
  limit=$1; shift
  timeout "$limit" sh -c 'until "$@" >/dev/null 2>&1; do sleep 0.2; done' _ "$@"
}

# A system bus of the session's own: GNOME Shell asks it for services that are not here (logind,
# UPower, the parental-controls timer) and copes when they are missing, but not without the bus.
cat > "$state/system-bus.conf" <<CONF
<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>system</type>
  <listen>unix:path=$XDG_RUNTIME_DIR/system_bus_socket</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
CONF
dbus-daemon --config-file="$state/system-bus.conf" --nofork --nopidfile &
pids="$pids $!"
export DBUS_SYSTEM_BUS_ADDRESS="unix:path=$XDG_RUNTIME_DIR/system_bus_socket"
until_ok 30 gdbus call --system --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus --method org.freedesktop.DBus.GetId

# Settings every process of the session reads: GLib's key-file backend (no dconf here). The
# accessibility bus on (answer.py reads the portal's window through it); no screen lock and no idle
# blanking during the ten minutes the scenario waits. No animations: a GTK switch flips at the end
# of its animation, which runs on the window's frames, and Mutter sends the portal's window none
# while it is not drawn, so answer.py's switch would stay off. No hot corner: seat.py moves the
# pointer through the top-left corner, which would open the overview.
export GSETTINGS_BACKEND=keyfile
gsettings set org.gnome.desktop.interface toolkit-accessibility true
gsettings set org.gnome.desktop.interface enable-animations false
gsettings set org.gnome.desktop.interface enable-hot-corners false
gsettings set org.gnome.desktop.screensaver lock-enabled false
gsettings set org.gnome.desktop.session idle-delay 0

launcher=""
for candidate in /usr/libexec/at-spi-bus-launcher /usr/lib/at-spi2-core/at-spi-bus-launcher; do
  if [ -x "$candidate" ]; then launcher=$candidate; break; fi
done
"$launcher" --launch-immediately &
pids="$pids $!"
until_ok 30 gdbus call --session --dest org.a11y.Bus --object-path /org/a11y/bus --method org.a11y.Bus.GetAddress

# The portal's calls, the sessions closed and the answers, without their arguments (they carry the
# restore token): each header line, and the response code of each Request.Response. GNU sed,
# unbuffered: mawk reads a pipe in blocks and would sit on the lines until the session ends.
dbus-monitor --session \
  "type='method_call',interface='org.freedesktop.portal.RemoteDesktop'" \
  "type='method_call',interface='org.freedesktop.impl.portal.RemoteDesktop'" \
  "type='method_call',interface='org.freedesktop.portal.Session',member='Close'" \
  "type='signal',interface='org.freedesktop.portal.Request',member='Response'" 2> "$out/dbus-monitor.err" \
  | sed -u -n -e '/^\(method call\|signal\|error\)/{p; /member=Response/{n; s/^ *uint32 \([0-9]*\)$/  response \1/p}}' \
  > "$out/portal.log" &
pids="$pids $!"

export XDG_CURRENT_DESKTOP=GNOME XDG_SESSION_TYPE=wayland GSK_RENDERER=cairo LIBGL_ALWAYS_SOFTWARE=1
dbus-update-activation-environment DBUS_SYSTEM_BUS_ADDRESS GSETTINGS_BACKEND XDG_CURRENT_DESKTOP XDG_SESSION_TYPE GSK_RENDERER LIBGL_ALWAYS_SOFTWARE

# GNOME Shell and its Xwayland, then the text field in front: at the start, and again in
# restart_gnome. Mutter starts Xwayland itself here (no systemd, so X11 is not started on demand).
shell="" xwayland="" target="" seat="" runs=0
start_gnome() {
  gnome-shell --headless --wayland --virtual-monitor 1280x800 >> "$out/gnome-shell.log" 2>&1 &
  shell=$!
  pids="$pids $shell"
  # The Shell's own interface, not only its name: the overview is one of its properties.
  until_ok 60 gdbus call --session --dest org.gnome.Shell --object-path /org/gnome/Shell \
    --method org.freedesktop.DBus.Properties.Get org.gnome.Shell OverviewActive
  export WAYLAND_DISPLAY=wayland-0
  # GNOME shows the overview once it has started (at most 30 s here); the pastes go to the text
  # field, not to the overview's search, so leave it and check that it stays left.
  timeout 30 sh -c 'until "$@" | grep -q true; do sleep 0.2; done' _ gdbus call --session --dest org.gnome.Shell \
    --object-path /org/gnome/Shell --method org.freedesktop.DBus.Properties.Get org.gnome.Shell OverviewActive \
    || echo "session.sh: GNOME did not show the overview" >&2
  until_ok 30 sh -c 'gdbus call --session --dest org.gnome.Shell --object-path /org/gnome/Shell \
    --method org.freedesktop.DBus.Properties.Set org.gnome.Shell OverviewActive "<false>" \
    && gdbus call --session --dest org.gnome.Shell --object-path /org/gnome/Shell \
    --method org.freedesktop.DBus.Properties.Get org.gnome.Shell OverviewActive | grep -q false'
  # This Shell's Xwayland (not the last one, which may still be going): its display and the
  # cookie it lets clients in with, which GNOME hands its apps as XAUTHORITY.
  until_ok 60 sh -c 'pgrep -x Xwayland | grep -vx "$1" | grep -q .' _ "${xwayland:-none}"
  xwayland=$(pgrep -x Xwayland | grep -vx "${xwayland:-none}" | head -n 1)
  args=$(tr '\0' ' ' < "/proc/$xwayland/cmdline")
  DISPLAY=$(printf '%s\n' "$args" | sed -n 's/.* \(:[0-9][0-9]*\) .*/\1/p')
  XAUTHORITY=$(printf '%s\n' "$args" | sed -n 's/.* -auth \([^ ]*\) .*/\1/p')
  export DISPLAY XAUTHORITY
  printf '%s\n' "$args" >> "$out/xwayland.txt"
  dbus-update-activation-environment WAYLAND_DISPLAY DISPLAY XAUTHORITY XDG_CURRENT_DESKTOP XDG_SESSION_TYPE GSK_RENDERER LIBGL_ALWAYS_SOFTWARE

  # The text field. What an earlier one held is kept as target.<n>.txt; target.txt.events has
  # its changes and whether its window had the keyboard, with the time.
  runs=$((runs + 1))
  [ ! -f "$out/target.txt" ] || mv "$out/target.txt" "$out/target.$((runs - 1)).txt"
  python3 -I "$repro/target.py" "$out/target.txt" >> "$out/target.log" 2>&1 &
  target=$!
  pids="$pids $target"
  until_ok 30 test -f "$out/target.txt"

  # The desk's keyboard and mouse (seat.py), and the click into the field. GNOME may show the
  # overview again as it finishes starting: leave it and click again, until the field has the
  # keyboard.
  rm -f "$state/seat.ready"
  python3 -I "$repro/seat.py" 1280 800 "$state/seat.ready" >> "$out/seat.log" 2>&1 &
  seat=$!
  pids="$pids $seat"
  until_ok 30 test -f "$state/seat.ready"
  tries=0
  until grep -E ' (active|inactive)$' "$out/target.txt.events" | tail -n 1 | grep -q ' active$'; do
    tries=$((tries + 1))
    if [ "$tries" -gt 10 ]; then
      echo "session.sh: the text field did not get the keyboard" >&2
      break
    fi
    gdbus call --session --dest org.gnome.Shell --object-path /org/gnome/Shell \
      --method org.freedesktop.DBus.Properties.Set org.gnome.Shell OverviewActive '<false>' >/dev/null || true
    kill -USR1 "$seat" 2>/dev/null || true
    timeout 3 sh -c 'until grep -E " (active|inactive)$" "$1" | tail -n 1 | grep -q " active$"; do sleep 0.2; done' _ "$out/target.txt.events" || true
  done
}

# GNOME Shell restarted, as at the next login: a new Xwayland, which knows no session of the last
# one. The portal's processes belong to the graphical session too (their systemd units are PartOf
# graphical-session.target): they end with it, and D-Bus starts them afresh when they are next
# called, the permission store from its file on disk.
restart_gnome() {
  for pid in "$seat" "$target" "$shell"; do
    kill "$pid" 2>/dev/null || true
    # Gone, or a zombie waiting for this shell: at most 30 s, then without asking.
    until_ok 30 sh -c 'case $(ps -o stat= -p "$1") in Z*|"") exit 0 ;; esac; exit 1' _ "$pid" \
      || kill -KILL "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
  done
  pkill -f '^/usr/libexec/xdg-(desktop-portal|permission-store)' || true
  until_ok 30 sh -c '! pgrep -f "^/usr/libexec/xdg-(desktop-portal|permission-store)"' \
    || echo "session.sh: the portal's processes did not end" >&2
  start_gnome
  printf 'gnome\trestarted: Xwayland %s\n' "$xwayland" | tee -a "$out/steps.log"
}

start_gnome

python3 -I "$repro/answer.py" "$state/answer" "$out/windows.log" > "$out/answer.log" 2>&1 &
pids="$pids $!"

windows() { wc -l < "$out/windows.log" | tr -d ' '; }

# One step: COUNT pastes in one run of the harness (one Voltip run) labelled LABEL, from the binary
# at BIN (default: where it is installed); then how many windows were answered and what reached the
# text field.
step() {
  count=$1 label=$2 note=$3 run=${4:-$bin}
  before=$(windows)
  timeout 300 "$run" "$count" "$label" > "$state/step.out" 2>&1 || true
  # The keys of the last paste reach the field after the harness returned: wait for its text,
  # or 10 s for a paste that did not go out.
  timeout 10 sh -c 'until grep -q "$1 $2" "$3"; do sleep 0.2; done' _ "$label" "$count" "$out/target.txt" || true
  after=$(windows)
  arrived=$(grep -o "$label [0-9]*" "$out/target.txt" | wc -l | tr -d ' ')
  results=$(tr '\n' ';' < "$state/step.out")
  printf '%s\tpastes=%s\twindows=%s\tarrived=%s\t%s\t%s\n' "$label" "$count" "$((after - before))" "$arrived" "$results" "$note" | tee -a "$out/steps.log"
}

# The same program from another path: Xwayland reuses a session by the client's command name, and
# an AppImage gets a new mount point, so a new command name, at every launch.
elsewhere=$state/elsewhere/voltip-paste-repro
mkdir -p "$state/elsewhere"
cp "$bin" "$elsewhere"

# Xwayland keeps the portal session of an X11 client that disconnected for ten minutes, for a
# client with the same command name (hw/xwayland/xwayland-xtest.c: `TimerSet(…, 10 * 60 * 1000,
# disconnect_timer_cb, …)`). This interval is the scenario's, not a wait for something to finish;
# VOLTIP_REPRO_REUSE_WAIT shortens it to try the script out (the result then says so).
past_xwayland_reuse() {
  wait_s=${VOLTIP_REPRO_REUSE_WAIT:-630}
  printf 'wait\t%s s: past the ten minutes Xwayland keeps a session\n' "$wait_s" | tee -a "$out/steps.log"
  sleep "$wait_s"
}

# What the person does who withdraws the grant: GNOME's record of it goes. (GNOME Settings 50 has
# no page for this; docs/dictation.md §14.9.) The saved token stays where Voltip keeps it.
revoke() {
  if gdbus call --session --dest org.freedesktop.impl.portal.PermissionStore \
    --object-path /org/freedesktop/impl/portal/PermissionStore \
    --method org.freedesktop.impl.portal.PermissionStore.Delete remote-desktop "$(cat "$VOLTIP_REPRO_TOKEN_FILE" 2>/dev/null)" >/dev/null 2>&1; then
    printf 'revoke\tthe grant removed from the permission store\n' | tee -a "$out/steps.log"
  else
    printf 'revoke\tno grant to remove\n' | tee -a "$out/steps.log"
  fi
}

: > "$out/steps.log"
case $scenario in
  base)
    step 3 base-a "first run: three pastes"
    step 2 base-b "Voltip restarted"
    step 2 base-c "Voltip started from another path" "$elsewhere"
    past_xwayland_reuse
    step 2 base-d "ten minutes and a half later"
    restart_gnome
    step 2 base-e "GNOME Shell restarted: a new Xwayland"
    ;;
  fix)
    step 3 fix-a "first run: three pastes"
    step 2 fix-b "Voltip restarted"
    step 2 fix-c "Voltip started from another path" "$elsewhere"
    past_xwayland_reuse
    step 2 fix-d "ten minutes and a half later"
    revoke
    step 2 fix-e "the grant revoked"
    revoke
    echo cancel > "$state/answer"
    step 1 fix-f "the grant revoked, the window cancelled"
    echo allow > "$state/answer"
    step 2 fix-g "the next paste after the refusal"
    restart_gnome
    step 2 fix-h "GNOME Shell restarted: a new Xwayland"
    ;;
  *) echo "session.sh: unknown scenario $scenario" >&2; exit 2 ;;
esac

{
  echo "scenario=$scenario"
  echo "versions=$(tr '\n' ';' < "$out/versions.txt")"
  echo "xwayland=$(sort -u "$out/xwayland.txt" | tr '\n' ';')"
  echo "windows=$(windows)"
  # Voltip asks to be remembered (the window shows "Remember This Selection"); Xwayland does not.
  echo "windows_with_remember=$(grep -c 'remember=ticked\|remember=unticked' "$out/windows.log" || true)"
  echo "windows_without_remember=$(grep -c 'remember=hidden' "$out/windows.log" || true)"
  # Who asked: Xwayland on the base side, the harness on the fix side (dbus-monitor's sender).
  echo "portal_create_session=$(grep -c 'interface=org.freedesktop.portal.RemoteDesktop; member=CreateSession' "$out/portal.log" || true)"
  echo "portal_start=$(grep -c 'interface=org.freedesktop.portal.RemoteDesktop; member=Start' "$out/portal.log" || true)"
  echo "portal_session_close=$(grep -c 'interface=org.freedesktop.portal.Session; member=Close' "$out/portal.log" || true)"
  echo "responses_cancelled=$(grep -c '^  response 1$' "$out/portal.log" || true)"
  echo "pastes=$(awk -F'\t' '{split($2, p, "="); n += p[2]} END {print n}' "$out/steps.log")"
} | tee "$out/summary.txt"
