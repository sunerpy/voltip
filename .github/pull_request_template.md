## What and why

<!-- What changed and why. Title the PR as a Conventional Commit (feat / fix / docs …): it decides the release version. -->

## Verification

- [ ] `make verify` passes locally (or say which gates you did not run, and why)
- [ ] For shell changes: `make smoke-desktop` / `make windows-x64`, or the platform you tested on
- [ ] No real host name, token or key in the change (`.github/scripts/check-no-production-hosts.sh`)

## Risk and rollback

<!-- Changes to data, the wire protocol or a security boundary, and how to roll back. "None" if none. -->
