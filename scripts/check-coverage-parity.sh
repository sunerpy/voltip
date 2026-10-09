#!/usr/bin/env bash
# Fail when the Makefile's COVERAGE_EXCLUDE regex and codecov.yml's ignore list disagree.
set -euo pipefail
cd "$(dirname "$0")/.."
regex=$(grep -E '^COVERAGE_EXCLUDE :=' Makefile | sed -E 's/^COVERAGE_EXCLUDE := //')
floor=$(grep -E '^COVERAGE_MIN :=' Makefile | sed -E 's/^COVERAGE_MIN := //')
target=$(grep -E '^\s+target: [0-9]+%' codecov.yml | head -1 | grep -oE '[0-9]+')
fail=0
token='src/main\.rs'
if ! grep -qF "$token" <<<"$regex"; then echo "Makefile COVERAGE_EXCLUDE lacks $token"; fail=1; fi
# The `ignore:` block only (flags list paths too): from `ignore:` to the next top-level key.
ignore=$(awk '/^ignore:/ { on = 1; next } /^[^[:space:]#]/ { on = 0 } on' codecov.yml)
grep -qF '**/src/main.rs' <<<"$ignore" || { echo "codecov.yml ignore lacks **/src/main.rs"; fail=1; }
# The Tauri shells are measured (apps/*/src-tauri/tests/ipc.rs); neither side may exclude them.
if grep -qF 'apps/' <<<"$regex"; then echo "Makefile COVERAGE_EXCLUDE must not exclude apps/"; fail=1; fi
if grep -qE '^\s*-\s*"?apps/' <<<"$ignore"; then echo "codecov.yml must not ignore apps/"; fail=1; fi
if [ "$floor" != "$target" ]; then echo "floor mismatch: Makefile $floor vs codecov $target"; fail=1; fi
[ "$fail" -eq 0 ] && echo "coverage-parity: OK (floor ${floor}%)"
exit "$fail"
