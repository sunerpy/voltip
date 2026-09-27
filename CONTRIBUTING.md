# Contributing

Thanks for looking. Bug reports, fixes and small features are welcome as issues and pull requests.

1. Read [AGENTS.md](AGENTS.md): layout, commands and the rules the code follows.
2. Install the toolchain: Rust (the channel in `rust-toolchain.toml`), Node 22 with pnpm 9,
   `cmake`, and on Linux the Tauri system packages listed in `.github/workflows/ci.yml`.
3. `pnpm install --frozen-lockfile`, then `make verify` before you open a pull request. It runs
   every gate CI runs except the packaging and platform jobs.
4. Title the pull request as a Conventional Commit (`fix: …`, `feat: …`); it becomes the squashed
   commit and decides the release version.

A fix comes with a test that fails without it. Keep a pull request to one change; say in its
description what you verified and on which platforms.

Security problems: see [SECURITY.md](SECURITY.md), not the issue tracker.
