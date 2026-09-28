#!/usr/bin/env bash
# Decide whether a CI run needs the build, test and platform jobs: writes code=true|false to
# $GITHUB_OUTPUT. false only when every changed file is documentation (docs/**, *.md, LICENSE*),
# or, on a push, when the rest only bumps a version: the commit release-please's pull request
# lands on main (`chore: release x.y.z`) changes CHANGELOG.md, the "version" of the package.json
# files and .release-please-manifest.json. That pull request already ran the full CI on the same
# tree, and the release run builds and checks every package from it, so the push runs verify-web
# only. Anything else (a merge queue, a new branch, an API failure, 300 or more files in a push)
# counts as code, so the full CI runs.
#
# Environment: EVENT (github.event_name), REPO, PR (pull request number), BEFORE / AFTER (push),
# GH_TOKEN for the REST API.
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
      if [ "$EVENT" != push ] || ! version_only "$patch"; then
        decide true "$file changes more than a pushed version"
      fi
      bumped=1
      ;;
    *) decide true "$file is not documentation" ;;
  esac
done < <(jq -r '.[].filename' <<<"$entries")
[ "$bumped" = 1 ] && decide false "a release version bump and documentation"
decide false "documentation only"
