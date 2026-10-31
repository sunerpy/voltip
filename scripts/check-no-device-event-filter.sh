#!/usr/bin/env bash
# The lone-key hook's Raw Input registration on Windows replaces tao's (docs/dictation.md §29), and
# tao registers again only when the app sets a device event filter. Such a call would take Raw
# Input from the hook thread, and a hook the system removed would no longer be put back: keep
# every `set_device_event_filter` / `device_event_filter` call out of the Rust sources.
set -euo pipefail
cd "$(dirname "$0")/.."
if git grep --untracked -n -E 'device_event_filter[[:space:]]*\(' -- '*.rs'; then
  echo "check-no-device-event-filter: a device event filter takes Raw Input from the lone-key hook (docs/dictation.md §29)" >&2
  exit 1
fi
echo "check-no-device-event-filter: no device event filter is set"
