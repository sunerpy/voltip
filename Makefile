# Voltip repository tasks. Every target is a thin wrapper so a shell and CI run the same thing.
# Variables use `:=` on purpose: an environment variable must not be able to lower a floor.

SHELL := /usr/bin/env bash
.DEFAULT_GOAL := help

CARGO := cargo
PNPM  := pnpm

# Filtered line-coverage floor (percent) for Rust. Measured 94.9% when introduced (2026-09-24).
COVERAGE_MIN := 90
# Paths excluded from the Rust gate — mirror codecov.yml `ignore:`; `make coverage-parity` checks.
#   src/main.rs — binary entry points (arg parsing + process exit)
# The Tauri shells (apps/*/src-tauri) are measured: their command layer runs on tauri's mock
# runtime in apps/*/src-tauri/tests/ipc.rs; only the thin `run()` needs a real webview.
COVERAGE_EXCLUDE := (src/main\.rs)

.PHONY: help
help: ## List targets
	@grep -E '^[a-zA-Z0-9_.-]+:.*## ' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*## "}; {printf "  %-22s %s\n", $$1, $$2}'

.PHONY: fmt
fmt: ## Format Rust and the web packages
	$(CARGO) fmt --all
	$(PNPM) -r run format

.PHONY: fmt-check
fmt-check: ## Verify formatting (CI gate)
	$(CARGO) fmt --all -- --check
	$(PNPM) -r run format:check

.PHONY: lint
lint: frontend-dist-dirs ## clippy -D warnings + eslint/tsc
	$(CARGO) clippy --workspace --all-targets -- -D warnings
	$(PNPM) -r run lint

.PHONY: test
test: frontend-dist-dirs ## Rust + web unit/integration tests
	$(CARGO) test --workspace --all-targets
	$(PNPM) -r run test

.PHONY: check
check: fmt-check lint test coverage-gate web-coverage-gate ## Everything CI runs

# tauri::generate_context!() needs apps/*/dist to exist at compile time (fresh clone: it does not).
.PHONY: frontend-dist-dirs
frontend-dist-dirs:
	@mkdir -p apps/desktop/dist apps/mobile/dist

.PHONY: coverage-gate
coverage-gate: frontend-dist-dirs ## Enforce the Rust filtered line-coverage floor (COVERAGE_MIN)
	@command -v cargo-llvm-cov >/dev/null 2>&1 || { echo "cargo-llvm-cov not found — cargo install cargo-llvm-cov"; exit 2; }
	@$(CARGO) llvm-cov --workspace --all-targets --summary-only \
	    --ignore-filename-regex '$(COVERAGE_EXCLUDE)' \
	  | tee -a /dev/stderr \
	  | awk -v min='$(COVERAGE_MIN)' '\
	      $$1 == "TOTAL" { seen = 1; pct = $$10; gsub(/%/, "", pct); \
	        printf "coverage-gate: filtered line coverage %.2f%%, floor %s%%\n", pct, min; \
	        if (pct + 0 < min + 0) { printf "coverage-gate: FAILED\n"; exit 1 } printf "coverage-gate: PASSED\n" } \
	      END { if (!seen) { print "coverage-gate: UNPROVEN — no TOTAL row"; exit 2 } }'

.PHONY: coverage-lcov
coverage-lcov: ## Write lcov.info for Codecov from the last coverage-gate run (no second test run)
	$(CARGO) llvm-cov report --lcov --output-path lcov.info --ignore-filename-regex '$(COVERAGE_EXCLUDE)'

.PHONY: coverage-parity
coverage-parity: ## Fail if codecov.yml's ignore set drifts from COVERAGE_EXCLUDE
	@./scripts/check-coverage-parity.sh

.PHONY: e2e-ipc
e2e-ipc: ## TypeScript drives two real Rust cores through the bridge harness over a local relay
	$(PNPM) --filter @voltip/shared run test:e2e

.PHONY: web-coverage-gate
web-coverage-gate: ## Vitest coverage thresholds (configured per package at 90%)
	$(PNPM) -r run test:coverage

.PHONY: relay
relay: ## Run a local relay on 127.0.0.1:47830
	$(CARGO) run -p voltip-relay --

.PHONY: desktop-dev
desktop-dev: ## Tauri desktop dev server (built-in engine defaults from .env.build, like the packaged builds)
	. scripts/lib/build-env.sh && voltip_load_build_env && cd apps/desktop && $(PNPM) tauri dev

.PHONY: acceptance
acceptance: ## Check the feature → implementation → test acceptance ledger
	@./scripts/check-acceptance-ledger.sh

.PHONY: verify
verify: ## Run every gate and write verify-all.log bound to the current commit
	./scripts/verify-all.sh verify-all.log

.PHONY: smoke-desktop
smoke-desktop: ## Build and run the real Tauri desktop app under Xvfb, screenshot the WebView (Linux)
	./scripts/smoke-desktop-linux.sh docs/acceptance/screens/tauri/desktop-linux-xvfb.png

.PHONY: android-apk
android-apk: ## Build the arm64 debug APK of the mobile shell (needs ANDROID_HOME, NDK_HOME, JAVA_HOME)
	./scripts/build-android-debug.sh dist/android/build-info.txt

.PHONY: windows-x64
windows-x64: ## Cross-build the Windows x64 portable exe + NSIS installer from Linux (cargo-xwin)
	./scripts/build-windows-x64.sh dist/windows-x64

.PHONY: smoke-wayland
smoke-wayland: ## Real desktop app on headless weston + sway: pure Wayland, --toggle, wtype paste, clipboard restore
	./scripts/smoke-wayland-linux.sh docs/acceptance/screens/tauri

.PHONY: linux-x64
linux-x64: ## Build the Linux x64 deb / rpm / AppImage with their build record (dist/linux-x64)
	./scripts/build-linux-x64.sh dist/linux-x64

.PHONY: smoke-native
smoke-native: ## Headless native run of a built binary (BIN=path): version, list, download a model, transcribe a public sample
	pwsh -NoProfile -File scripts/smoke-native-cli.ps1 -Binary "$(or $(BIN),target/debug/voltip-desktop)" -OutDir smoke-native-cli

.PHONY: windows-remote
windows-remote: ## Real Windows over SSH (VOLTIP_WINDOWS_SSH): sync HEAD, native cargo test + clippy (MSVC), headless run of dist/windows-x64
	./scripts/windows-remote.sh sync
	./scripts/windows-remote.sh gate test
	./scripts/windows-remote.sh gate clippy
	./scripts/windows-remote.sh smoke dist/windows-x64

.PHONY: desktop-vnc desktop-vnc-stop
desktop-vnc: ## Remote desktop for manual testing: xfce on TigerVNC + noVNC (browser), `make desktop-dev` inside, fake mic
	./scripts/dev-desktop-vnc.sh start

desktop-vnc-stop: ## Stop the VNC desktop, its fake microphone and the app inside it
	./scripts/dev-desktop-vnc.sh stop
