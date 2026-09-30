# AGENTS.md

How to work in this repository, for coding agents and people alike. `CONTRIBUTING.md` points here.

## Layout

- `crates/` — the Rust workspace. `voltip-core` is the platform-free application core (commands in,
  events out, ports for everything native); the other crates are its adapters: audio, cloud and
  on-device recognition, clean-up, injection, platform tables, protocol, crypto, identity, pairing,
  transport, relay, and the Tauri bridge.
- `apps/desktop`, `apps/mobile` — the Tauri 2 shells (`src-tauri/`) and their React front ends.
- `packages/shared` (IPC contract, i18n, labels, mock backend), `packages/ui` (design system).
- `docs/` — `architecture.md` first. `dictation.md` is the pipeline's contract; the `§N` references
  in code comments point into it.
- `docs/site/` — the pages of voltip.firlab.app, English with Chinese under `zh/`; the site itself
  lives in `sunerpy/firlab` (`voltip/`). `docs/site/README.md` has the writing rules.

## Commands

```bash
pnpm install --frozen-lockfile   # once
make verify                      # every gate; writes verify-all.log bound to HEAD. Run before pushing.
make fmt | make lint | make test # the narrower loops
make desktop-dev                 # the desktop app with hot reload
make smoke-desktop               # the real app under Xvfb (Linux); make smoke-wayland for Wayland
make windows-x64 | make linux-x64 | make android-apk   # packages
make help                        # everything else
```

- The Rust gates build an explicit feature list (`scripts/lib/rust-features.sh`), never
  `--all-features`: the Vulkan backend needs the Vulkan SDK. `scripts/check-rust-features.sh` fails
  when a new feature is not in one of the lists. The Windows and Linux package scripts do build
  Vulkan; `scripts/lib/vulkan-sdk.sh` downloads the pinned SDK and loader for them.
- On Linux the desktop shell leaves through `_exit` (`src/exit.rs`, an NVIDIA driver teardown
  bug): flush anything you write to stdout or stderr yourself, and never rely on an exit handler.
- After changing a type that crosses the IPC bridge, regenerate the fixtures and commit them:
  `UPDATE_IPC_FIXTURES=1 cargo test -p voltip-tauri-bridge --test contract`.
- Coverage floors: Rust 90 % of lines (`make coverage-gate`), each web package 90 %
  (`make web-coverage-gate`). Codecov receives the reports but does not gate.

## Architecture rules

- Native capability lives in Rust: hotkeys, audio, recognition, injection, windows, clipboard,
  secrets. The webview renders `UiState` and sends commands; it never touches the OS.
- Streams (level meters, progress) use `tauri::ipc::Channel`; discrete state changes are UI events
  on `voltip://event`.
- Logic goes into `voltip-core` behind a port and is tested there with the fakes
  (`voltip_core::dictation::fakes`); the shells only wire real implementations in.
- One wire contract: the Rust types and the zod schemas in `packages/shared/src/schema.ts` are
  checked against each other through `packages/shared/src/fixtures/ipc/`.
- Every user-visible string goes through the typed dictionaries in `packages/shared/src/i18n/`
  (`zh-CN.ts` defines the keys, `en.ts` must match). The UI never shows a feature as "coming soon":
  it works, or it is not there.
- A change a user can see updates the matching `docs/site` pages, in both languages, in the same
  pull request; a change to the interface's text or layout retakes the screenshots that show it
  (`docs/site/README.md`).
- The mock backend (`@voltip/shared/mock`) and the dev-only pages load under `import.meta.env.DEV`
  only; `scripts/check-web-bundle.sh` fails a release bundle that contains them.

## Secrets and hosts

- No real host name, token or key goes into the tree, the UI or a log line. Built-in endpoints are
  compile-time values (`option_env!`) from the git-ignored `.env.build` or from CI secrets
  (`.github/README-secrets.md`); docs and tests use placeholders such as `<asr-host>`.
- `.github/scripts/check-no-production-hosts.sh` scans the tree for the words listed in
  `VOLTIP_PRODUCTION_HOSTS`; `make verify` runs it with your `.env.build`.
- A client never carries a provider API key; the packaging scripts scan the binaries for one.
- Release builds never fall back to an insecure secret store (the in-memory store is debug-only).

## Tests

- Every fixed bug gets a regression test that fails without the fix (`regression_…` in Rust,
  `it("regression: …")` in TypeScript).
- Never weaken, skip or delete a test to get a green run.
- Tests that need a display, a real model or a GPU are `#[ignore]`d; the head of each such file says
  how to run it.
- Code that depends on the host OS must pass on Linux, Windows and macOS: CI runs all three.
- `docs/acceptance.md` maps each feature to its implementation and tests; `make acceptance` checks
  that every reference still exists.

### Tests on CI's machines

Most red runs on `main` came from tests that raced on a slower machine than the author's: the
coverage run on `main` and the Intel Mac. Locally and in PR CI the same tests passed. Three such
runs landed in a row on 2026-09-30, and one of them stalled a release for an hour. So:

- Wait for the state an assertion reads. When two threads or tasks produce events, do not wait for
  one event on the assumption that another has arrived before it. Check the current state before
  waiting for a status: an event may already have been folded by a helper. Under
  `#[tokio::test(start_paused = true)]` the clock stands still while a blocking thread runs (a live
  decode thread, a held fake), so a virtual timeout never ends such a wait.
- To show that something returns before a timeout, give it a long timeout (seconds) and assert
  under half of it. A bound equal to the timeout fails as soon as a machine is slow.
- A test stops what it started before it ends: it unmounts a React root it created itself, clears
  its intervals, and releases what a fake holds. Blocking fakes give up after `HOLD_LIMIT`
  (`voltip_core::dictation::fakes`). Dropping a tokio runtime waits for every blocking thread, so a
  failed assertion behind a held call would otherwise hang the job for its hour instead of failing.
- Before pushing a test that depends on timing or threads, run it a few hundred times with every
  core busy. To confirm a suspected race, delay the step in the fake and watch the test fail.
- The `macos` jobs do not run on pull requests. After a merge, check that `main`'s run is green on
  both Macs; a red `macos` job is fixed like any other.
- A CI job that runs twice its usual time is a hung test. The Rust gates step stops at 25 minutes
  by itself; cancel the run to learn sooner. Either way the gate logs still upload, and the hung
  test is the one "running for over 60 seconds".
- `scripts/windows-remote.sh gate` runs whatever was synced last. Run `sync` first, and check that
  the commit at the head of the gate's log is yours.

## Commits and releases

- Conventional Commits (`feat:`, `fix:`, `docs:`, `test:`, `refactor:`, `chore:`, `ci:`). The type
  decides the release-please bump; `feat!:` or a `BREAKING CHANGE:` footer marks a breaking change.
- release-please owns the version (root `package.json`, read by `tauri.conf.json`; the Cargo crates
  stay at `0.0.0`), `CHANGELOG.md` and the tags. Do not bump versions by hand.
- Every change reaches `main` through a pull request, never a direct push: work on a branch, push
  it, open the pull request, and merge it with squash (`gh pr merge --squash`), release PRs
  included. One commit per pull request keeps the history of `main` short; the branch's
  intermediate commits (fixes for CI, formatting) stay out of it. The exception is a very large
  change or a refactor whose step-by-step history is worth keeping. The repository allows squash
  merges only, so for that merge the owner turns on rebase merging (Settings → General → Pull
  Requests) and merges with `--rebase`. (User decision 2026-09-29: sixteen commits pushed straight
  to `main` sat between 0.0.5 and 0.0.6.)
- Merging needs an up-to-date branch and the `Release candidate` status on its head (the
  `release-candidate` ruleset): `ci.yml`'s `candidate-status` writes it for an ordinary pull request,
  `release-candidate.yml` for release-please's once the packages are built. `CI Success` is CI's
  verdict; wait for it too. The ruleset lets the owner bypass it, for an emergency push or a fork's
  pull request that cannot write the status, not as the way changes land. The `macos` jobs of
  `ci.yml` (Apple silicon and Intel) run on pushes to `main` and on demand (`gh workflow run ci.yml
  --ref <branch>` before merging a macOS change), not on pull requests.
- Releases build once: `release-candidate.yml` builds, signs and seals every package for the
  release PR head, and `release.yml` promotes those exact bytes after the squash merge, without
  compiling. `docs/runbook.md` (发布) has the recovery steps.
