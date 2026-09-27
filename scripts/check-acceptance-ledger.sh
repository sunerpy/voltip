#!/usr/bin/env bash
# shellcheck disable=SC2016  # the grep patterns match literal backticks
# Every evidence reference in docs/acceptance.md must point at a real test:
#   `path/to/file@test name`  →  file exists AND contains the name (any characters but a backtick:
#   test names carry punctuation and CJK)
# and every repository path it names must exist (`crates/x/src/{a,b}.rs` and `…/*.json` expand), so
# a renamed or deleted file cannot leave a row pointing at nothing.
# Rows without evidence are allowed only when marked [!] (explicitly unverified).
# Usage: scripts/check-acceptance-ledger.sh [ledger]   (default docs/acceptance.md)
set -euo pipefail
cd "$(dirname "$0")/.."
ledger=${1:-docs/acceptance.md}
fail=0
count=0
while IFS= read -r ref; do
  file=${ref%%@*}
  name=${ref#*@}
  count=$((count + 1))
  if [ ! -f "$file" ]; then echo "missing file: $file (for $name)"; fail=1; continue; fi
  if ! grep -qF -- "$name" "$file"; then echo "missing test: $name in $file"; fail=1; fi
done < <(grep -oE '`[A-Za-z0-9_./-]+\.(rs|ts|tsx|sh|py|ps1)@[^`]+`' "$ledger" | tr -d '`' | sort -u)

# `a/{b,c}/d` → a/b/d, a/c/d (one group at a time, so several groups multiply).
expand() {
  if [[ $1 =~ ^([^{]*)\{([^{}]*)\}(.*)$ ]]; then
    local head=${BASH_REMATCH[1]} tail=${BASH_REMATCH[3]} alternative
    local -a alternatives
    IFS=, read -ra alternatives <<<"${BASH_REMATCH[2]}"
    for alternative in "${alternatives[@]}"; do expand "$head$alternative$tail"; done
  else
    printf '%s\n' "$1"
  fi
}
paths=0
while IFS= read -r token; do
  while IFS= read -r path; do
    paths=$((paths + 1))
    if [[ $path == *"*"* ]]; then
      compgen -G "$path" >/dev/null || { echo "missing path: $path"; fail=1; }
    elif [ ! -e "$path" ]; then
      echo "missing path: $path"
      fail=1
    fi
  done < <(expand "$token")
done < <(grep -oE '`(apps|crates|packages|scripts|docs|cmake|\.github)/[^`[:space:]@<>$]+`' "$ledger" | tr -d '`' | sed -E 's/(:[0-9]+(-[0-9]+)?)+$//; s#/$##' | sort -u)
echo "acceptance-ledger: $count evidence references and $paths repository paths checked"
exit "$fail"
