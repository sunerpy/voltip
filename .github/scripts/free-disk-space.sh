#!/usr/bin/env bash
# Free disk space on a GitHub-hosted Ubuntu runner only when the job could run short:
# `free-disk-space.sh [min-free-GB]` (default 40). The runners measured on 2026-09-28 had 86 GB free
# on / before any cleanup, so the unconditional `rm -rf` of the preinstalled SDKs (1-2 minutes per
# job) freed space nothing used. Below the threshold it removes the biggest SDKs nothing here uses.
set -euo pipefail
min=${1:-40}
free=$(df --output=avail -BG / | tail -1 | tr -dc '0-9')
echo "free-disk-space: ${free} GB free on /, threshold ${min} GB"
if [ "$free" -ge "$min" ]; then
  exit 0
fi
sudo rm -rf /usr/local/lib/android /usr/share/dotnet /opt/ghc /usr/local/.ghcup /opt/hostedtoolcache/CodeQL
echo "free-disk-space: $(df --output=avail -BG / | tail -1 | tr -dc '0-9') GB free after the cleanup"
