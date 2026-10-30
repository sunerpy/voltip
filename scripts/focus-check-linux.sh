#!/usr/bin/env bash
# The insert check's AT-SPI reader against a real GTK 3 window (docs/dictation.md §28,
# crates/voltip-focus/tests/focus_linux.rs): a private D-Bus session with the accessibility bus,
# accessibility switched on in it, and an X server (Xvfb). Needs xvfb-run, xdotool,
# dbus-run-session, gdbus, at-spi2-core, python3-gi and gir1.2-gtk-3.0. Builds first, so a hang in
# the real run is told apart from a slow build.
#
#   scripts/focus-check-linux.sh                    the real tests (CI's smoke-desktop)
#   scripts/focus-check-linux.sh --probe RUNS CSV   the measurement (examples/insert_confirm_probe.rs)
#                                                   in the window's editable entry, RUNS pastes
set -euo pipefail
cd "$(dirname "$0")/.."
mode=test runs=0 csv=""
if [ "${1:-}" = "--probe" ]; then
  mode=probe runs=${2:?--probe RUNS CSV} csv=$(realpath -m "${3:?--probe RUNS CSV}")
fi
launcher=""
for candidate in /usr/libexec/at-spi-bus-launcher /usr/lib/at-spi2-core/at-spi-bus-launcher; do
  if [ -x "$candidate" ]; then launcher=$candidate; break; fi
done
[ -n "$launcher" ] || { echo "focus-check-linux: at-spi-bus-launcher not found (apt install at-spi2-core)"; exit 2; }
for tool in xvfb-run xdotool dbus-run-session gdbus; do
  command -v "$tool" >/dev/null || { echo "focus-check-linux: $tool not installed"; exit 2; }
done
/usr/bin/python3 -c 'import gi; gi.require_version("Gtk", "3.0")' || { echo "focus-check-linux: python3-gi with GTK 3 missing"; exit 2; }
if [ "$mode" = test ]; then
  cargo test -p voltip-focus --test focus_linux --no-run
else
  cargo build -p voltip-focus --example insert_confirm_probe
fi
# shellcheck disable=SC2016 # expanded by the inner shell
exec xvfb-run -a dbus-run-session -- sh -euc '
  "$1" --launch-immediately &
  launcher=$!
  trap "kill $launcher 2>/dev/null || true" EXIT
  # The accessibility bus is up once org.a11y.Bus answers; then accessibility is switched on in
  # this session only, as an assistive technology would.
  i=0
  until gdbus call --session --dest org.a11y.Bus --object-path /org/a11y/bus --method org.a11y.Bus.GetAddress >/dev/null 2>&1; do
    i=$((i + 1)); [ "$i" -le 100 ] || { echo "focus-check-linux: the accessibility bus did not come up"; exit 1; }
    sleep 0.1
  done
  gdbus call --session --dest org.a11y.Bus --object-path /org/a11y/bus --method org.freedesktop.DBus.Properties.Set org.a11y.Status IsEnabled "<true>" >/dev/null
  if [ "$2" = test ]; then
    exec cargo test -p voltip-focus --test focus_linux -- --ignored --test-threads=1 --nocapture
  fi
  # The probe: the helper window, its editable entry focused, then the pastes.
  dir=$(mktemp -d)
  mkfifo "$dir/in"
  /usr/bin/python3 crates/voltip-focus/tests/support/gtk_entries.py <"$dir/in" >"$dir/out" &
  helper=$!
  exec 3>"$dir/in"
  trap "echo quit >&3 2>/dev/null || true; kill $helper $launcher 2>/dev/null || true; rm -rf $dir" EXIT
  i=0
  until grep -qx ready "$dir/out"; do
    i=$((i + 1)); [ "$i" -le 200 ] || { echo "focus-check-linux: the helper window did not start"; exit 1; }
    sleep 0.1
  done
  xdotool search --sync --name voltip-focus-check windowfocus --sync
  echo editable >&3
  i=0
  until grep -qx "focused editable" "$dir/out"; do
    i=$((i + 1)); [ "$i" -le 200 ] || { echo "focus-check-linux: the entry did not take the focus"; exit 1; }
    sleep 0.1
  done
  cargo run -q -p voltip-focus --example insert_confirm_probe -- --app gtk3-entry --runs "$3" --reset select-all --out "$4"
' _ "$launcher" "$mode" "$runs" "$csv"
