#!/usr/bin/env bash
# Issue #235 in a GNOME 50 Wayland session (docs/dictation.md §14.9): how often GNOME asks for
# remote interaction when Voltip pastes, before and after the fix. Each side runs in its own
# container from ubuntu:26.04 with no network: a private session bus, headless GNOME Shell with
# Xwayland (Mutter starts it with `-enable-ei-portal`), xdg-desktop-portal and
# xdg-desktop-portal-gnome, a GTK text field the pastes land in, a keyboard and mouse for the
# session (Mutter's own remote-desktop interface, not the portal), and answer.py, which answers each
# "Remote Desktop" window as a person would ("Allow Remote Interaction" on, Share) and counts it.
#
# The paste path is the desktop shell's (`voltip_inject::system_paste_injector`, harness/main.rs),
# built from two revisions:
#   base  BASE (default: the parent of the fix): every paste opens a new X11 connection, Xwayland
#         asks the portal for each new client and keeps a session for ten minutes after it left.
#   fix   FIX (default: HEAD), with the restore token kept in a file between runs as the shell
#         keeps it in the keychain: the portal and libei directly, one window for the first paste.
# Steps (scripts/repro/gnome-remote-desktop/session.sh): three pastes in one run, two in a second
# run (Voltip restarted), two from a copy of the program elsewhere (an AppImage mounts at a new
# path at every launch), ten and a half minutes of nothing, two more pastes; on the fix side then
# the grant revoked in the permission store, the window cancelled once, and the paste after that;
# last GNOME Shell restarted as at the next login, and two more pastes.
#
# Usage: scripts/repro-gnome-remote-desktop.sh [--base REV] [--fix REV] [--only base|fix] [--keep]
#   --keep  keeps the work directory (the trees, the builds) and the image.
# VOLTIP_REPRO_REUSE_WAIT=<seconds> shortens the ten and a half minutes to try the script out;
# the results then do not show Xwayland's session running out.
# Writes <out>/{base,fix}/ with steps.log, windows.log, portal.log, summary.txt and the session's
# logs, <out>/summary.txt, and <out>/verdict.txt: verdict.sh checks each step against what the issue
# and the fix lead to expect, and ends with PASS or FAIL; out is $VOLTIP_REPRO_OUT or a new
# directory under /tmp. Exits 1 when a check fails or a side does not finish.
# Needs docker, git and rustup with the toolchain of rust-toolchain.toml. Takes about 12 minutes
# once the image is built: both sides run at once, and each waits out Xwayland's ten minutes.
# shellcheck disable=SC2016  # `sh -c '…' _ "$x"` expands in the inner shell
set -euo pipefail
cd "$(dirname "$0")/.."
repo=$PWD
for tool in docker git rustup tar; do
  command -v "$tool" >/dev/null || { echo "repro-gnome-remote-desktop: $tool not installed"; exit 2; }
done

base_rev="" fix_rev=HEAD only="" keep=0
while [ $# -gt 0 ]; do
  case $1 in
    --base) base_rev=$2; shift 2 ;;
    --fix) fix_rev=$2; shift 2 ;;
    --only) only=$2; shift 2 ;;
    --keep) keep=1; shift ;;
    *) echo "repro-gnome-remote-desktop: unknown argument $1"; exit 2 ;;
  esac
done
fix_rev=$(git rev-parse --verify "$fix_rev^{commit}")
if [ -z "$base_rev" ]; then
  # The parent of the commit that brought the portal backend (docs/dictation.md §14.9).
  first=$(git log --format=%H --diff-filter=A -1 "$fix_rev" -- crates/voltip-inject/src/portal/mod.rs)
  [ -n "$first" ] || { echo "repro-gnome-remote-desktop: $fix_rev has no crates/voltip-inject/src/portal/mod.rs; pass --base"; exit 2; }
  base_rev=$first^
fi
base_rev=$(git rev-parse --verify "$base_rev^{commit}")
sides="base fix"
[ -z "$only" ] || sides=$only

toolchain=$(sed -n 's/^channel *= *"\(.*\)"/\1/p' rust-toolchain.toml)
image=voltip-repro-gnome50:${VOLTIP_REPRO_IMAGE_TAG:-local}
work=$(mktemp -d /tmp/voltip-repro-gnome.XXXXXX)
out=${VOLTIP_REPRO_OUT:-$work/out}
mkdir -p "$out"
containers=""
cleanup() {
  for name in $containers; do docker rm -f "$name" >/dev/null 2>&1 || true; done
  if [ "$keep" = 0 ]; then
    case $out in "$work"/*) mv "$out" "$work.out" ;; esac
    rm -rf "$work"
    case $out in "$work"/*) echo "repro-gnome-remote-desktop: results in $work.out" ;; esac
  fi
}
trap cleanup EXIT

echo "repro-gnome-remote-desktop: base $base_rev, fix $fix_rev, toolchain $toolchain"
build_args=()
for var in http_proxy https_proxy no_proxy; do
  [ -z "${!var:-}" ] || build_args+=(--build-arg "$var=${!var}")
done
docker build --network host "${build_args[@]}" -t "$image" scripts/repro/gnome-remote-desktop >"$work/docker-build.log" 2>&1 \
  || { tail -20 "$work/docker-build.log"; exit 1; }

# The harness against REV's voltip-inject, with the dependency versions of REV's lock file.
build_harness() {
  local side=$1 rev=$2 features=$3
  local tree=$work/$side/tree harness=$work/$side/harness
  mkdir -p "$tree" "$harness"
  git archive "$rev" | tar -x -C "$tree"
  cp scripts/repro/gnome-remote-desktop/harness/main.rs "$harness/main.rs"
  cp "$tree/Cargo.lock" "$harness/Cargo.lock"
  cat >"$harness/Cargo.toml" <<EOF
[package]
name = "voltip-paste-repro"
version = "0.0.0"
edition = "2024"
publish = false

[[bin]]
name = "voltip-paste-repro"
path = "main.rs"

[features]
fixed = []

[dependencies]
voltip-inject = { path = "$tree/crates/voltip-inject" }

[workspace]
EOF
  RUSTUP_TOOLCHAIN=$toolchain cargo build --quiet --manifest-path "$harness/Cargo.toml" --target-dir "$work/target" ${features:+--features "$features"}
  mkdir -p "$work/$side/bin"
  cp "$work/target/debug/voltip-paste-repro" "$work/$side/bin/"
}

for side in $sides; do
  case $side in
    base) build_harness base "$base_rev" "" ;;
    fix) build_harness fix "$fix_rev" fixed ;;
    *) echo "repro-gnome-remote-desktop: --only base|fix"; exit 2 ;;
  esac
done

pids=()
for side in $sides; do
  mkdir -p "$out/$side"
  name=voltip-repro-$side-$$
  containers="$containers $name"
  # The session's own /tmp and runtime directory; no network, nothing of this machine's desktop.
  timeout 3600 docker run --rm --name "$name" --network none --shm-size 512m \
    -e XDG_RUNTIME_DIR=/run/user/1000 --tmpfs /run/user/1000:mode=0700,uid=1000,gid=1000 \
    ${VOLTIP_REPRO_REUSE_WAIT:+-e "VOLTIP_REPRO_REUSE_WAIT=$VOLTIP_REPRO_REUSE_WAIT"} \
    -v "$repo/scripts/repro/gnome-remote-desktop:/repro:ro" \
    -v "$work/$side/bin:/repro-bin:ro" \
    -v "$out/$side:/repro-out" \
    "$image" dbus-run-session -- sh /repro/session.sh "$side" >"$out/$side/session.log" 2>&1 &
  pids+=($!)
done
status=0
for pid in "${pids[@]}"; do wait "$pid" || status=1; done

{
  echo "base=$base_rev"
  echo "fix=$fix_rev"
  for side in $sides; do
    echo "== $side"
    cat "$out/$side/steps.log" 2>/dev/null || echo "(no steps: see $side/session.log)"
    cat "$out/$side/summary.txt" 2>/dev/null || true
  done
} | tee "$out/summary.txt"
# shellcheck disable=SC2086  # one word per side
sh scripts/repro/gnome-remote-desktop/verdict.sh "$out" $sides | tee "$out/verdict.txt" || status=1
[ "$keep" = 0 ] || echo "repro-gnome-remote-desktop: kept $work and $image"
[ "$keep" = 1 ] || docker image rm "$image" >/dev/null 2>&1 || true
exit "$status"
