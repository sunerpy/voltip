#!/usr/bin/env bash
# Fail when a file in the tree names a production host. The built-in endpoints reach the clients
# only at compile time through the build environment (docs/dictation.md §3); source, docs, configs
# and scripts use placeholders such as <asr-host> / <relay-host>.
#
# The words to look for are never written in the repository, this script included. They come from
# VOLTIP_PRODUCTION_HOSTS: space- or comma-separated host names or distinctive labels, read from the
# environment (a CI secret) and from the git-ignored .env.build when it exists. Each word is
# matched case-insensitively as a fixed string; a hit is reported as path:line only, so the word
# itself never reaches a log.
#
# Scope: every tracked file plus untracked files that are not git-ignored (a new file is caught
# before it is committed; .env.build is ignored and therefore never scanned). Binary files are
# skipped.
#
# Usage: .github/scripts/check-no-production-hosts.sh [--require]
#   exit 0 = clean, or skipped because no words are configured (without --require)
#   exit 1 = hits
#   exit 2 = misuse, or --require and no words configured
set -euo pipefail

require=0
case "${1:-}" in
	"") ;;
	--require) require=1 ;;
	*)
		echo "usage: $0 [--require]" >&2
		exit 2
		;;
esac

cd "$(git rev-parse --show-toplevel)"

# VOLTIP_PRODUCTION_HOSTS from .env.build (KEY=VALUE, optional quotes), without sourcing the file.
from_file=""
if [[ -f .env.build ]]; then
	from_file=$(sed -n -E 's/^[[:space:]]*(export[[:space:]]+)?VOLTIP_PRODUCTION_HOSTS=//p' .env.build | tail -n 1)
	from_file=${from_file%\"}
	from_file=${from_file#\"}
	from_file=${from_file%\'}
	from_file=${from_file#\'}
fi

patterns=$(mktemp)
trap 'rm -f "$patterns"' EXIT
count=0
set -f # the words are split on whitespace below, never globbed
for word in ${VOLTIP_PRODUCTION_HOSTS:-} $from_file; do
	word=${word//,/ }
	for part in $word; do
		part=$(printf '%s' "$part" | tr '[:upper:]' '[:lower:]')
		if [[ ${#part} -lt 4 ]]; then
			echo "check-no-production-hosts: every word in VOLTIP_PRODUCTION_HOSTS needs at least 4 characters" >&2
			exit 2
		fi
		printf '%s\n' "$part" >>"$patterns"
		count=$((count + 1))
	done
done
set +f

if [[ "$count" -eq 0 ]]; then
	if [[ "$require" -eq 1 ]]; then
		echo "::error::VOLTIP_PRODUCTION_HOSTS is empty; set the repository secret (see .github/README-secrets.md)" >&2
		exit 2
	fi
	echo "check-no-production-hosts: skipped (no VOLTIP_PRODUCTION_HOSTS in the environment or .env.build)"
	exit 0
fi

hits=$(
	git ls-files -z --cached --others --exclude-standard |
		while IFS= read -r -d '' path; do
			[[ -f "$path" ]] || continue # deleted-but-tracked entries
			printf '%s\0' "$path"
		done |
		xargs -0 -r grep -H -I -n -i -F -f "$patterns" -- 2>/dev/null |
		cut -d: -f1,2 || true
)

if [[ -n "$hits" ]]; then
	echo "::error::a production host appears in the tree; replace it with a placeholder or move the value to the build environment" >&2
	printf '%s\n' "$hits" >&2
	exit 1
fi
echo "check-no-production-hosts: clean ($count configured word(s), values not shown)"
