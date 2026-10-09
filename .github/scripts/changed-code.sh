#!/usr/bin/env bash
# Decide whether a CI run needs the build, test and platform jobs: writes code=true|false to
# $GITHUB_OUTPUT. false only when every changed file is documentation (docs/**, *.md, LICENSE*),
# or when the rest only bumps a version and the change is release-please's own: the pull request
# release-please opens (author github-actions[bot], branch release-please--branches--main--*,
# label "autorelease: pending") and the commit it lands on main (`chore: release x.y.z`) change
# CHANGELOG.md, the "version" of the package.json files and .release-please-manifest.json. The
# release candidate (release-candidate.yml) builds and checks every package from that exact head
# after proving the same delta, and the head's base already passed CI, so those runs take
# verify-web only. The same diff from anyone else, or without the label, runs everything, as does
# a new branch, an API failure, or 300 or more files in a push.
#
# Environment: EVENT (github.event_name), REPO, PR (pull request number), BEFORE / AFTER (push),
# PR_AUTHOR / PR_HEAD_REPO / PR_HEAD_REF / PR_LABELS (pull request; labels as a JSON array of
# names), GH_TOKEN for the REST API.
set -euo pipefail
out=${GITHUB_OUTPUT:-/dev/stdout}
decide() {
  echo "code=$1" >>"$out"
  echo "changed-code: code=$1 ($2)"
  exit 0
}
# Every changed line of a unified-diff patch sets a version string ("version": "0.0.4" in a
# package.json, ".": "0.0.4" in the release-please manifest).
version_only() {
  local changed
  changed=$(grep -E '^[+-]' <<<"$1" || true)
  [ -n "$changed" ] || return 1
  ! grep -Eqv '^[+-][[:space:]]*"(version|\.)"[[:space:]]*:[[:space:]]*"[0-9A-Za-z.+-]+",?[[:space:]]*$' <<<"$changed"
}
# release-please's pull request, recognised by all four marks (a fork cannot be github-actions[bot]).
release_pr=0
if [ "${EVENT:?}" = pull_request ] && [ "${PR_AUTHOR:-}" = "github-actions[bot]" ] &&
  [ "${PR_HEAD_REPO:-}" = "${REPO:?}" ] && [[ ${PR_HEAD_REF:-} == release-please--branches--main--* ]] &&
  jq -e 'index("autorelease: pending") != null' <<<"${PR_LABELS:-[]}" >/dev/null 2>&1; then
  release_pr=1
fi
case "${EVENT:?}" in
  pull_request)
    entries=$(gh api "repos/${REPO:?}/pulls/${PR:?}/files" --paginate --jq '.[] | {filename, patch}' | jq -sc .) || decide true "could not list the pull request's files"
    ;;
  push)
    [[ ${BEFORE:?} =~ ^0+$ ]] && decide true "a new branch"
    json=$(gh api "repos/${REPO:?}/compare/${BEFORE}...${AFTER:?}") || decide true "could not compare ${BEFORE}...${AFTER}"
    count=$(jq '.files | length' <<<"$json")
    [ "$count" -lt 300 ] || decide true "$count files (the compare API lists at most 300)"
    entries=$(jq -c '[.files[] | {filename, patch}]' <<<"$json")
    ;;
  *) decide true "event ${EVENT}" ;;
esac
[ "$(jq length <<<"$entries")" -gt 0 ] || decide true "no changed files listed"
bumped=0
while IFS= read -r file; do
  case $file in
    docs/* | *.md | LICENSE | LICENSE.*) ;;
    package.json | */package.json | .release-please-manifest.json)
      patch=$(jq -r --arg f "$file" '.[] | select(.filename == $f) | .patch // ""' <<<"$entries")
      if { [ "$EVENT" != push ] && [ "$release_pr" != 1 ]; } || ! version_only "$patch"; then
        decide true "$file changes more than a released version"
      fi
      bumped=1
      ;;
    *) decide true "$file is not documentation" ;;
  esac
done < <(jq -r '.[].filename' <<<"$entries")
[ "$bumped" = 1 ] && decide false "a release version bump and documentation"
decide false "documentation only"
