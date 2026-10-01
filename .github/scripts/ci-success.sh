#!/usr/bin/env bash
# The rule of `CI Success`, ci.yml's aggregate: the check the main ruleset requires and the only one
# release-candidate.yml's source gate reads. Every job it needs must have succeeded, with two
# skips accepted:
#   - any job but `changes` and `verify-web` when the change is documentation or version numbers
#     only (CODE=false, the fast path .github/scripts/changed-code.sh selects);
#   - the `macos` legs on a pull request, where they never run.
# On a push or a manual run the Macs must pass. v0.0.5 shipped from 06334e6, whose macos-x64 leg
# had failed in main's CI while CI Success was green, because the aggregate did not wait for it.
#
# Environment: NEEDS_JSON (`toJSON(needs)`), CODE (`needs.changes.outputs.code`), EVENT
# (`github.event_name`). Exits 0 when the run counts as green. Tests: scripts/release/test_ci_success.py.
set -euo pipefail
: "${NEEDS_JSON:?NEEDS_JSON is required}" "${EVENT:?EVENT is required}"
code=${CODE:-}

echo "$NEEDS_JSON" | jq .
echo "code=${code} event=${EVENT}"

missing=$(jq -r '["changes", "verify-web"] - keys | .[]' <<<"$NEEDS_JSON")
if [ -n "$missing" ]; then
	echo "::error title=CI Success::CI Success does not need $(tr '\n' ' ' <<<"$missing")"
	exit 1
fi

failing=$(jq -r --arg code "$code" --arg event "$EVENT" '
  def accepted($job; $result):
    $result == "success" or
    ($job != "changes" and $job != "verify-web" and $result == "skipped" and
      ($code == "false" or ($job == "macos" and $event == "pull_request")));
  to_entries[] | select(accepted(.key; .value.result) | not) | "\(.key)=\(.value.result)"
' <<<"$NEEDS_JSON")
if [ -n "$failing" ]; then
	echo "::error title=CI Success::not every required job succeeded: $(tr '\n' ' ' <<<"$failing")"
	exit 1
fi
echo "CI Success: every required job succeeded"
