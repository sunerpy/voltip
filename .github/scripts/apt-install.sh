#!/usr/bin/env bash
# `apt-get install` on a GitHub-hosted Ubuntu runner that does not hang the job on a slow mirror.
# On 2026-10-01 azure.archive.ubuntu.com served the desktop smoke's packages at a few kB/s, twice
# in one day: the step ran for 48 and 35 minutes, the job never reached its tests, and each time
# the release candidate waiting for CI Success stalled until the run was cancelled and rerun.
#
# The downloads (`apt-get update`, then `install --download-only`) get a deadline per attempt;
# after a failed or late attempt the next one fetches from archive.ubuntu.com instead of the Azure
# mirror. The install itself, from the downloaded files, has no deadline: a dpkg run stopped
# halfway would leave the system broken.
#
# Usage: .github/scripts/apt-install.sh <package>...
#   APT_ATTEMPT_SECONDS  the download deadline of one attempt (default 600)
set -euo pipefail

[ "$#" -gt 0 ] || { echo "usage: $0 <package>..." >&2; exit 2; }
deadline=${APT_ATTEMPT_SECONDS:-600}
opts=(-o Acquire::Retries=3 -o Acquire::http::Timeout=30 -o Dpkg::Use-Pty=0)

# The runner images name the Azure mirror in a mirror list or in the sources themselves.
use_main_archive() {
	local file
	for file in /etc/apt/apt-mirrors.txt /etc/apt/sources.list /etc/apt/sources.list.d/*.list /etc/apt/sources.list.d/*.sources; do
		[ -f "$file" ] || continue
		sudo sed -i 's#azure\.archive\.ubuntu\.com#archive.ubuntu.com#g' "$file"
	done
}

for attempt in 1 2 3; do
	if sudo timeout "$deadline" apt-get "${opts[@]}" update &&
		sudo timeout "$deadline" apt-get "${opts[@]}" install --yes --no-install-recommends --download-only "$@"; then
		sudo apt-get "${opts[@]}" install --yes --no-install-recommends "$@"
		exit 0
	fi
	[ "$attempt" -lt 3 ] || break
	echo "::warning::apt downloads failed or took longer than ${deadline}s (attempt $attempt of 3); fetching from archive.ubuntu.com"
	use_main_archive
done
echo "::error::apt-get could not download the packages in three attempts" >&2
exit 1
