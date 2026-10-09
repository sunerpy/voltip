# shellcheck shell=bash disable=SC2034  # sourced: the callers read both lists
# The Cargo features the verify gates build with: every feature of the workspace but the GPU
# backends, which need the Vulkan SDK at build time and are checked on a GPU host
# (docs/dictation.md §10.6). `scripts/check-rust-features.sh` fails when a new feature is in neither list.
RUST_FEATURES="voltip-audio/test-support voltip-inject/test-support voltip-core/keyring voltip-identity/keyring voltip-identity/android-keystore voltip-relay/server voltip-desktop/custom-protocol voltip-desktop/keychain-harness voltip-mobile/custom-protocol"
RUST_GPU_FEATURES="voltip-asr-local/vulkan voltip-desktop/gpu-vulkan voltip-server/gpu-vulkan"
