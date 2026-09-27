#!/usr/bin/env bash
# Every Cargo feature a workspace crate declares is either built by the verify gates
# (RUST_FEATURES) or a GPU backend checked on a GPU host (RUST_GPU_FEATURES); a feature in
# neither list would never be compiled by CI.
set -euo pipefail
cd "$(dirname "$0")/.."
. scripts/lib/rust-features.sh
cargo metadata --no-deps --format-version 1 | python3 -c '
import json, sys
known = set(sys.argv[1].split()) | set(sys.argv[2].split())
meta = json.load(sys.stdin)
declared = {f"{p['"'"'name'"'"']}/{f}" for p in meta["packages"] for f in p["features"] if f != "default"}
missing = sorted(declared - known)
stale = sorted(known - declared)
for m in missing: print(f"check-rust-features: {m} is in neither RUST_FEATURES nor RUST_GPU_FEATURES", file=sys.stderr)
for s in stale: print(f"check-rust-features: {s} is listed but no crate declares it", file=sys.stderr)
if missing or stale: sys.exit(1)
print(f"check-rust-features: {len(declared)} features accounted for")
' "$RUST_FEATURES" "$RUST_GPU_FEATURES"
