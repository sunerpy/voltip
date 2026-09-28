#!/usr/bin/env bash
# Decide whether a CI run needs the build, test and platform jobs: writes code=true|false to
# $GITHUB_OUTPUT. false only when every changed file is documentation (docs/**, *.md, LICENSE*);
# anything else (a merge queue, a new branch, an API failure, 300 or more files in a push)
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
case "${EVENT:?}" in
  pull_request)
    files=$(gh api "repos/${REPO:?}/pulls/${PR:?}/files" --paginate --jq '.[].filename') || decide true "could not list the pull request's files"
    ;;
  push)
    [[ ${BEFORE:?} =~ ^0+$ ]] && decide true "a new branch"
    json=$(gh api "repos/${REPO:?}/compare/${BEFORE}...${AFTER:?}") || decide true "could not compare ${BEFORE}...${AFTER}"
    count=$(jq '.files | length' <<<"$json")
    [ "$count" -lt 300 ] || decide true "$count files (the compare API lists at most 300)"
    files=$(jq -r '.files[].filename' <<<"$json")
    ;;
  *) decide true "event ${EVENT}" ;;
esac
[ -n "$files" ] || decide true "no changed files listed"
while IFS= read -r file; do
  case $file in
    docs/* | *.md | LICENSE | LICENSE.*) ;;
    *) decide true "$file is not documentation" ;;
  esac
done <<<"$files"
decide false "documentation only"
