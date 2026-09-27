#!/usr/bin/env bash
# A release webview bundle carries no in-memory mock backend and no sample-value views: both
# `main.tsx` import `@voltip/shared/mock`, and the desktop `App` imports the overlay spec sheet, only
# under `import.meta.env.DEV`, a build-time constant. Run after `pnpm -r run build`. The sentinels
# are strings minification keeps: `MOCK_HOTKEY_BACKEND` and a dictionary key only the sheet reads.
set -euo pipefail
cd "$(dirname "$0")/.."

sentinels=('mock · browser preview' 'overlay.anatomyBody')
status=0
for dist in apps/desktop/dist apps/mobile/dist; do
  if [ ! -d "$dist/assets" ]; then
    echo "check-web-bundle: $dist/assets is missing; run pnpm -r run build first" >&2
    status=1
    continue
  fi
  for sentinel in "${sentinels[@]}"; do
    if grep -rqF "$sentinel" "$dist"; then
      echo "check-web-bundle: $dist carries dev-only code ($sentinel)" >&2
      status=1
    fi
  done
done
if [ "$status" -eq 0 ]; then
  echo "check-web-bundle: no dev-only code in the release bundles"
fi
exit "$status"
