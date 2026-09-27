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

## Commits and releases

- Conventional Commits (`feat:`, `fix:`, `docs:`, `test:`, `refactor:`, `chore:`, `ci:`). The type
  decides the release-please bump; `feat!:` or a `BREAKING CHANGE:` footer marks a breaking change.
- release-please owns the version (root `package.json`, read by `tauri.conf.json`; the Cargo crates
  stay at `0.0.0`), `CHANGELOG.md` and the tags. Do not bump versions by hand.
- The required check is `CI Success`. `macos.yml` runs on every push to `main`.
