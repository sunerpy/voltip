#!/usr/bin/env bash
# Grant TCC services to one client in the system TCC database of a GitHub-hosted Mac (SIP is off
# there and the database is writable with sudo), then print what the database holds for it. Used
# by ci.yml for the test binaries that post events or read the accessibility tree, and by
# scripts/smoke-tray-macos.sh.
#
# tccd keeps the database open and writes to it on its own. sqlite3 waits for no lock by default
# and gives up at once when it finds the database locked (run 42489391064, macos-aarch64: "Error:
# stepping, database is locked (5)" from one of scripts/smoke-tray-macos.sh's grants), so every
# statement here waits up to 30 s for the lock; a lock held longer still fails, with sqlite3's own
# message and exit status.
#
# Usage: scripts/tcc-grant-macos.sh <client path> <service>...
#        e.g. scripts/tcc-grant-macos.sh "$bin" kTCCServiceAccessibility kTCCServicePostEvent
set -euo pipefail

client=${1:?usage: tcc-grant-macos.sh <client path> <service>...}
shift
[ "$#" -gt 0 ] || { echo "usage: tcc-grant-macos.sh <client path> <service>..." >&2; exit 2; }
db="/Library/Application Support/com.apple.TCC/TCC.db"
# A quote in the path stays inside the SQL string literal.
quoted=${client//\'/\'\'}
tcc() { sudo sqlite3 -cmd '.timeout 30000' "$db" "$1"; }
for service in "$@"; do
  tcc "INSERT OR REPLACE INTO access (service, client, client_type, auth_value, auth_reason, auth_version, indirect_object_identifier, flags, last_modified) VALUES ('$service', '$quoted', 1, 2, 4, 1, 'UNUSED', 0, $(date +%s));"
done
tcc "SELECT service, client, auth_value FROM access WHERE client = '$quoted';"
