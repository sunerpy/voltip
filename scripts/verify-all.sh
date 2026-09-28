#!/usr/bin/env bash
# Run every acceptance gate against the current tree and write one log that binds the results to
# the exact commit: `scripts/verify-all.sh [log-path]`. Exit 0 only if every gate passed.
# The log records HEAD, whether the tree was dirty, and the exit code and duration of each gate, so
# a green claim can always be traced back to the SHA it was measured on.
#
# VERIFY_GATES=rust|web runs one half (CI runs the halves in parallel jobs); the default, all, is
# `make verify`. rust: formatting, features, clippy, the cross-checks, the test suite under coverage,
# cargo-deny and the IPC e2e (it builds Rust binaries). web: the web gates, the ledger, the host
# guard and the release helpers.
set -uo pipefail
cd "$(dirname "$0")/.." || exit 1
log=${1:-verify-all.log}
: > "$log"
groups=${VERIFY_GATES:-all}
case $groups in all | rust | web) ;; *) echo "verify-all: VERIFY_GATES must be all, rust or web (got $groups)" >&2; exit 2 ;; esac
head_sha=$(git rev-parse HEAD 2>/dev/null || echo "no-git")
dirty=$(git status --porcelain 2>/dev/null | wc -l | tr -d ' ')
{
  echo "verify-all: $(date -u +%Y-%m-%dT%H:%M:%SZ) host=$(uname -srm)"
  echo "commit=$head_sha dirty_paths=$dirty gates=$groups"
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
# gate <group> <name> <command…>: run it when its group is selected.
gate() {
  local group=$1 name=$2; shift 2
  if [ "$groups" != all ] && [ "$groups" != "$group" ]; then return 0; fi
  echo "== $name: $*" | tee -a "$log"
  local start=$SECONDS
  "$@" >> "$log" 2>&1
  local rc=$?
  echo "== $name exit=$rc secs=$((SECONDS - start))" | tee -a "$log"
  if [ "$rc" -ne 0 ]; then overall=1; fi
}

gate rust rust-fmt        cargo fmt --all -- --check
# Every feature but the GPU backends (they need the Vulkan SDK; docs/dictation.md §10.6).
. scripts/lib/rust-features.sh
gate rust rust-features   ./scripts/check-rust-features.sh
gate rust rust-clippy     cargo clippy --workspace --all-targets --features "$RUST_FEATURES" -- -D warnings
gate rust cross-check-darwin  cargo check -p voltip-platform -p voltip-inject -p voltip-hooks --target aarch64-apple-darwin
gate rust cross-check-windows cargo check -p voltip-platform -p voltip-inject -p voltip-hooks --target x86_64-pc-windows-msvc
# The test suite runs once, instrumented, with the same features as clippy: coverage-gate is the
# test gate (the workspace has no doctests, which it would skip).
gate rust rust-coverage   make coverage-gate
gate rust coverage-parity ./scripts/check-coverage-parity.sh
gate rust cargo-deny      cargo deny check bans licenses sources
gate rust ipc-e2e         make e2e-ipc
gate web web-lint        pnpm -r run lint
gate web web-typecheck   pnpm -r run typecheck
gate web web-format      pnpm -r run format:check
gate web web-coverage    pnpm -r run test:coverage
gate web web-build       pnpm -r run build
gate web web-bundle      ./scripts/check-web-bundle.sh
gate web acceptance      ./scripts/check-acceptance-ledger.sh
gate web no-prod-hosts   .github/scripts/check-no-production-hosts.sh
gate web release-helpers python3 -m unittest discover -s scripts/release -p "test_*.py"

echo "verify-all: overall_exit=$overall commit=$head_sha" | tee -a "$log"
exit "$overall"
