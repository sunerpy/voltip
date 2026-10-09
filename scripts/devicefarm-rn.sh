#!/usr/bin/env bash
# Run the React Native phone app's device acceptance on AWS Device Farm (docs/mobile-rn.md §7):
# upload the APK, the test package (apps/mobile-rn/devicefarm as a zip) and the test spec,
# schedule the run on a device pool, wait for it, and download every job's artefacts (summary.txt,
# screenshots, UI trees, logcat) into <out>/<device>/.
#
# Usage: devicefarm-rn.sh <apk> <device pool arn> <out dir> [run name]
#        devicefarm-rn.sh --fetch <run arn> <out dir>     (the artefacts of a run again)
# Needs the AWS CLI with a profile for the account (AWS_PROFILE_DF, default `us`); Device Farm is
# in us-west-2 only. Exit 0 when every job passed its summary checks, 1 otherwise, 2 on misuse.
set -euo pipefail
cd "$(dirname "$0")/.."

# Environment variables outrank --profile/--region (a stray AWS_REGION sends the call elsewhere).
unset AWS_REGION AWS_DEFAULT_REGION AWS_PROFILE AWS_ACCESS_KEY_ID AWS_SECRET_ACCESS_KEY AWS_SESSION_TOKEN
df() { aws devicefarm --profile "${AWS_PROFILE_DF:-us}" --region us-west-2 "$@"; }

# Every job's FILE artefacts of run $1 into $2/<device>/ (zips unpacked into files/), then the
# summary checks; the exit status is the verdict. Names hold spaces: the fields are tab-separated.
fetch() {
  local run_arn=$1 out=$2 verdict=0 job_arn device dir aname ext url summary want
  mkdir -p "$out"
  while IFS=$'\t' read -r job_arn device; do
    dir=$out/$(printf '%s' "$device" | tr -c 'A-Za-z0-9._-' '_')
    mkdir -p "$dir"
    while IFS=$'\t' read -r aname ext url; do
      file=$dir/$(printf '%s' "$aname" | tr -c 'A-Za-z0-9._-' '_').$ext
      curl -sS --fail -o "$file" "$url" || { echo "devicefarm-rn: $device: $aname did not download" >&2; continue; }
      case "$ext" in zip) unzip -q -o "$file" -d "$dir/files" ;; esac
    done < <(df list-artifacts --arn "$job_arn" --type FILE --query 'artifacts[].[name,extension,url]' --output text)
    summary=$(find "$dir/files" -name summary.txt 2>/dev/null | head -1)
    if [ -z "$summary" ]; then
      echo "devicefarm-rn: $device: no summary.txt" >&2
      verdict=1
      continue
    fi
    echo "== $device"
    cat "$summary"
    # What apps/mobile-rn/devicefarm/acceptance.sh must have seen for the device to pass.
    for want in installed=yes first_screen=ok stays_up=yes talk_finished=yes 03-history=ok 04-settings=ok \
      05-speech=ok 06-ai=ok 07-appearance=ok theme_dark=ok 09-recording=ok dropdown=ok back_closes_menu_first=ok \
      11-dictionary=ok editor=ok 12b-rules=ok 12c-scenes=ok 12d-history-settings=ok \
      12e-feedback=ok 12f-about=ok follow_system=ok back_to_talk=ok 13-device=ok 14-pair=ok pair_code=ok talk_again=ok back_once_stays=ok \
      back_twice_leaves=ok fatal=none; do
      grep -qx "$want" "$summary" || { echo "devicefarm-rn: $device: expected $want" >&2; verdict=1; }
    done
  done < <(df list-jobs --arn "$run_arn" --query 'jobs[].[arn,device.name]' --output text)
  return "$verdict"
}

if [ "${1:-}" = --fetch ]; then
  [ "$#" -eq 3 ] || { echo "usage: $0 --fetch <run arn> <out dir>" >&2; exit 2; }
  fetch "$2" "$3"
  exit $?
fi
if [ "$#" -lt 3 ]; then
  echo "usage: $0 <apk> <device pool arn> <out dir> [run name]" >&2
  exit 2
fi
apk=$1 pool=$2 out=$3 name=${4:-voltip-rn-$(date -u +%Y%m%dT%H%M%SZ)}
[ -f "$apk" ] || { echo "devicefarm-rn: no APK at $apk" >&2; exit 2; }
project=${pool%/*}
project=${project/:devicepool:/:project:}
mkdir -p "$out"

scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT
package=$scratch/voltip-rn-tests.zip
(cd apps/mobile-rn/devicefarm && zip -q -r "$package" acceptance.sh)

# Upload <file> as <type>; prints the upload's ARN once Device Farm has processed it.
upload() {
  local file=$1 type=$2 arn url status
  # A command substitution does not inherit `set -e`: each step checks its own outcome.
  read -r arn url < <(df create-upload --project-arn "$project" --name "$(basename "$file")" --type "$type" --query 'upload.[arn,url]' --output text) || true
  [ -n "${arn:-}" ] && [ -n "${url:-}" ] || { echo "devicefarm-rn: Device Farm created no $type upload" >&2; exit 1; }
  curl -sS --fail -T "$file" "$url" >/dev/null || { echo "devicefarm-rn: the $type upload did not reach Device Farm" >&2; exit 1; }
  # Processing takes seconds; the wait ends at the first terminal status or after five minutes.
  status=$(timeout 300 bash -c 'while :; do s=$(aws devicefarm --profile "$0" --region us-west-2 get-upload --arn "$1" --query upload.status --output text); case $s in SUCCEEDED|FAILED) echo "$s"; exit 0 ;; esac; sleep 3; done' "${AWS_PROFILE_DF:-us}" "$arn") || status=TIMEOUT
  if [ "$status" != SUCCEEDED ]; then
    echo "devicefarm-rn: $type upload $status: $(df get-upload --arn "$arn" --query upload.metadata --output text)" >&2
    exit 1
  fi
  echo "$arn"
}

app_arn=$(upload "$apk" ANDROID_APP)
pkg_arn=$(upload "$package" APPIUM_NODE_TEST_PACKAGE)
spec_arn=$(upload apps/mobile-rn/devicefarm/testspec.yml APPIUM_NODE_TEST_SPEC)
run_arn=$(df schedule-run --project-arn "$project" --app-arn "$app_arn" --device-pool-arn "$pool" --name "$name" \
  --test "{\"type\":\"APPIUM_NODE\",\"testPackageArn\":\"$pkg_arn\",\"testSpecArn\":\"$spec_arn\"}" \
  --configuration '{"locale":"zh_CN","billingMethod":"METERED"}' \
  --execution-configuration '{"jobTimeoutMinutes":20}' --query run.arn --output text)
echo "devicefarm-rn: run $run_arn"

# A run takes minutes (device setup, install, the script); the wait ends when it completes, or
# fails after an hour.
if ! timeout 3600 bash -c 'while [ "$(aws devicefarm --profile "$0" --region us-west-2 get-run --arn "$1" --query run.status --output text)" != COMPLETED ]; do sleep 20; done' "${AWS_PROFILE_DF:-us}" "$run_arn"; then
  echo "devicefarm-rn: the run did not complete within an hour: $(df get-run --arn "$run_arn" --query 'run.[status,result]' --output text)" >&2
  exit 1
fi
df get-run --arn "$run_arn" --query 'run.{result:result,minutes:deviceMinutes.total,counters:counters}' --output json | tee "$out/run.json"

fetch "$run_arn" "$out"
