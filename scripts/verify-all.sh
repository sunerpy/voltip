#!/usr/bin/env bash
# Run every acceptance gate against the current tree and write one log that binds the results to
# the exact commit: `scripts/verify-all.sh [log-path]`. Exit 0 only if every gate passed.
# The log records HEAD, whether the tree was dirty, and the exit code of each gate, so a green
# claim can always be traced back to the SHA it was measured on.
set -uo pipefail
cd "$(dirname "$0")/.." || exit 1
log=${1:-verify-all.log}
: > "$log"
head_sha=$(git rev-parse HEAD 2>/dev/null || echo "no-git")
dirty=$(git status --porcelain 2>/dev/null | wc -l | tr -d ' ')
{
  echo "verify-all: $(date -u +%Y-%m-%dT%H:%M:%SZ) host=$(uname -srm)"
  echo "commit=$head_sha dirty_paths=$dirty"
  echo "rust=$(rustc --version 2>/dev/null) node=$(node --version 2>/dev/null) pnpm=$(pnpm --version 2>/dev/null)"
} | tee -a "$log"

# `tauri::generate_context!()` refuses to compile when `frontendDist` (apps/*/dist) does not exist,
# which is the state of a fresh clone before any web build; vite empties the directory on build, so a
# tracked placeholder would churn. Create the directories instead.
mkdir -p apps/desktop/dist apps/mobile/dist
# Hermetic gates: the core tests assert the "no built-in engine" state, so compile-time defaults
# from a sourced .env.build or the CI environment must not leak into this run.
unset VOLTIP_ASR_URL VOLTIP_ASR_TOKEN VOLTIP_ASR_MODEL VOLTIP_REFINE_URL VOLTIP_REFINE_API_KEY VOLTIP_REFINE_MODEL VOLTIP_RELAY_URL VOLTIP_UPDATE_URL VOLTIP_UPDATE_PUBKEY VOLTIP_FEEDBACK_URL VOLTIP_FEEDBACK_TOKEN

overall=0
gate() {
  local name=$1; shift
  echo "== $name: $*" | tee -a "$log"
  "$@" >> "$log" 2>&1
  local rc=$?
  echo "== $name exit=$rc" | tee -a "$log"
  if [ "$rc" -ne 0 ]; then overall=1; fi
}

gate rust-fmt        cargo fmt --all -- --check
# Every feature but the GPU backends (they need the Vulkan SDK; docs/dictation.md §10.6).
. scripts/lib/rust-features.sh
gate rust-features   ./scripts/check-rust-features.sh
gate rust-clippy     cargo clippy --workspace --all-targets --features "$RUST_FEATURES" -- -D warnings
gate cross-check-darwin  cargo check -p voltip-platform -p voltip-inject --target aarch64-apple-darwin
gate cross-check-windows cargo check -p voltip-platform -p voltip-inject --target x86_64-pc-windows-msvc
gate rust-test       cargo test --workspace --features "$RUST_FEATURES"
gate rust-coverage   make coverage-gate
gate coverage-parity ./scripts/check-coverage-parity.sh
gate cargo-deny      cargo deny check bans licenses sources
gate web-lint        pnpm -r run lint
gate web-typecheck   pnpm -r run typecheck
gate web-format      pnpm -r run format:check
gate web-coverage    pnpm -r run test:coverage
gate web-build       pnpm -r run build
gate web-bundle      ./scripts/check-web-bundle.sh
gate ipc-e2e         make e2e-ipc
gate acceptance      ./scripts/check-acceptance-ledger.sh
gate no-prod-hosts   .github/scripts/check-no-production-hosts.sh
gate release-helpers python3 -m unittest discover -s scripts/release -p "test_*.py"

echo "verify-all: overall_exit=$overall commit=$head_sha" | tee -a "$log"
exit "$overall"
