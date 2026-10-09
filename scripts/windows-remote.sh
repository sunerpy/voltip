#!/usr/bin/env bash
# shellcheck disable=SC2016  # the PowerShell programs are expanded on Windows (their $Dir / $_ are PowerShell variables)
# Native build, test and headless run on a real Windows machine over OpenSSH, before CI sees the
# change (docs/runbook.md「Windows 真机（SSH）」).
#
#   scripts/windows-remote.sh sync               HEAD → <dir>\repo (git bundle over scp, detached checkout)
#   scripts/windows-remote.sh gate test|clippy   the Rust half of `make test` / `make lint`, natively (MSVC)
#   scripts/windows-remote.sh gate real [filter] the real-model tests on the machine's CPU (VOLTIP_LOCAL_* as for
#                                                crates/voltip-asr-local/tests/real.rs; the files are copied over)
#   scripts/windows-remote.sh gate hooks         the lone-key trigger's input hooks, fed with SendInput (the
#                                                logged-on user's session must be unlocked)
#   scripts/windows-remote.sh gate mdns          two real mDNS daemons on the machine see each other (LAN discovery)
#   scripts/windows-remote.sh gate echo          the mixed take's echo canceller, optimised, on the machine's CPU: the
#                                                synthetic rooms and the per-frame timing (docs/dictation.md §22.6)
#   scripts/windows-remote.sh gate loopback      a tone played through the default output is recorded back as the
#                                                computer's sound (docs/dictation.md §22; audible on the machine); with
#                                                VOLTIP_LOOPBACK_PLAY=<16 kHz mono WAV> that recording plays instead and
#                                                what was recorded comes back as target/windows-remote/loopback-out.wav
#   scripts/windows-remote.sh smoke [dist]       scripts/smoke-native-cli.ps1 on a package (default dist/windows-x64)
#   scripts/windows-remote.sh wait test|clippy|real|smoke   re-attach to a run that is still going
#   scripts/windows-remote.sh ps                 run the PowerShell on stdin there (UTF-8 both ways)
#
# Settings, from the environment or else the git-ignored .env.build (KEY=value lines):
# VOLTIP_WINDOWS_SSH      ssh arguments that reach the machine, ending in the host (required), for
#                         example `-F ~/.ssh/config win-build` or `-p 2222 admin@winbox`
# VOLTIP_WINDOWS_DIR      work directory there (default: C:\voltip-ci)
# VOLTIP_WINDOWS_TIMEOUT  seconds to wait for a gate or the smoke (default 5400)
# VOLTIP_WINDOWS_MODELS   catalogue ids for the smoke (default: sense-voice-small,qwen3-asr-0.6b, as CI)
#
# The machine needs the OpenSSH server with an administrator login, Git, Rust with the MSVC
# toolchain, Visual Studio 2022 Build Tools (C++ workload, CMake) and PowerShell 7. `gate` and
# `smoke` run as scheduled tasks (scripts/windows-remote-{gate,smoke}.ps1), so a dropped SSH
# connection does not kill them, and this script waits for the log's EXIT= line: `gate` in the
# logged-on user's session (their toolchain), `smoke` as SYSTEM, whose data directory is the system
# profile, so no real user's Voltip library is touched. Logs are copied back to target/windows-remote/.
set -euo pipefail
cd "$(dirname "$0")/.."

. scripts/lib/build-env.sh
ssh_spec=$(voltip_build_env_value VOLTIP_WINDOWS_SSH)
if [ -z "$ssh_spec" ]; then
  echo "windows-remote: set VOLTIP_WINDOWS_SSH (ssh arguments ending in the host) in the environment or .env.build" >&2
  exit 2
fi
read -r -a ssh_args <<<"$ssh_spec"
ssh_args=("${ssh_args[@]/#\~\//$HOME/}") # a ~/ path from .env.build is not expanded by the shell
host=${ssh_args[${#ssh_args[@]} - 1]}
ssh_opts=("${ssh_args[@]:0:${#ssh_args[@]}-1}" -o BatchMode=yes -o ConnectTimeout=15)
dir=$(voltip_build_env_value VOLTIP_WINDOWS_DIR)
dir=${dir:-'C:\voltip-ci'}
scp_dir=${dir//\\//}
local_dir=target/windows-remote
mkdir -p "$local_dir"

# remote_ps [NAME=value ...] <script: PowerShell from stdin with $Dir and every NAME defined first
# as single-quoted strings. The remote shell would expand `$` and answer in the console code page,
# so the script travels as -EncodedCommand (UTF-16LE base64) behind a UTF-8 output preamble. Any
# error stops the script with one plain line on stderr (not CLIXML) and makes ssh exit non-zero.
remote_ps() {
  local enc assign value
  enc=$({
    printf '%s\n' '$ErrorActionPreference = "Stop"; $ProgressPreference = "SilentlyContinue"; [Console]::OutputEncoding = [Text.Encoding]::UTF8'
    printf "\$Dir = '%s'\n" "${dir//\'/\'\'}"
    for assign in "$@"; do
      value=${assign#*=}
      printf "\$%s = '%s'\n" "${assign%%=*}" "${value//\'/\'\'}"
    done
    printf 'try {\n'
    cat
    printf '\n} catch { [Console]::Error.WriteLine("windows-remote: $($_.Exception.Message) (line $($_.InvocationInfo.ScriptLineNumber))"); exit 1 }\n'
  } | iconv -f UTF-8 -t UTF-16LE | base64 -w0)
  # shellcheck disable=SC2029  # built here on purpose: $enc is the whole (encoded) remote script
  ssh "${ssh_opts[@]}" "$host" "powershell -NoProfile -NonInteractive -OutputFormat Text -EncodedCommand $enc"
}

# put <subdir-or-empty> <file>...: copy local files into <dir>\<subdir>.
put() {
  local sub=$1
  shift
  scp -q "${ssh_opts[@]}" "$@" "$host:$scp_dir/$sub"
}

sync() {
  local bundle
  bundle=$(mktemp --suffix=.bundle)
  git bundle create -q "$bundle" HEAD
  remote_ps <<<'New-Item -ItemType Directory -Force $Dir | Out-Null'
  put '' "$bundle"
  remote_ps Bundle="$(basename "$bundle")" <<'PS'
$repo = Join-Path $Dir 'repo'
$bundle = Join-Path $Dir $Bundle
if (-not (Test-Path (Join-Path $repo '.git'))) { git init -q $repo }
Set-Location -LiteralPath $repo
git fetch -q $bundle HEAD
if ($LASTEXITCODE -eq 0) { git checkout -q --detach FETCH_HEAD }
$code = $LASTEXITCODE
Remove-Item -LiteralPath $bundle
if ($code -ne 0) { exit $code }
"windows-remote: $repo at $(git log -1 --format='%h %s')"
PS
  rm -f "$bundle"
}

# Wait for <dir>\<name>.log to end with EXIT=<code>, polling every 20 s up to the timeout. An
# unreachable machine is retried until then; a task that stopped without writing EXIT= fails at
# once (an Interactive task does not start while nobody is logged on).
wait_log() {
  local name=$1 task=$2 deadline=$((SECONDS + ${VOLTIP_WINDOWS_TIMEOUT:-5400})) status
  while :; do
    sleep 20
    status=$(remote_ps Name="$name" Task="$task" <<'PS'
$log = Join-Path $Dir "$Name.log"
$exit = $null
if (Test-Path $log) {
  $fs = [IO.File]::Open($log, 'Open', 'Read', 'ReadWrite, Delete')
  $reader = New-Object IO.StreamReader($fs)
  $text = $reader.ReadToEnd()
  $reader.Close()
  $exit = $text -split "`n" | Where-Object { $_ -match '^EXIT=' } | Select-Object -Last 1
}
if ($exit) { $exit.Trim() } else { "STATE=$((Get-ScheduledTask -TaskName $Task).State)" }
PS
    ) || status=unreachable
    status=${status//$'\r'/}
    case $status in
      EXIT=*) break ;;
      STATE=Running | STATE=Queued | unreachable) ;;
      *) echo "windows-remote: task $task is not running and wrote no EXIT line ($status; is anyone logged on?)" >&2; return 1 ;;
    esac
    if [ "$SECONDS" -ge "$deadline" ]; then
      echo "windows-remote: $name still running after ${VOLTIP_WINDOWS_TIMEOUT:-5400}s" >&2
      return 124
    fi
  done
  scp -q "${ssh_opts[@]}" "$host:$scp_dir/$name.log" "$local_dir/$name.log"
  echo "windows-remote: $status → $local_dir/$name.log"
  case $status in "EXIT=0 "* | EXIT=0) return 0 ;; *) return 1 ;; esac
}

# The real-model tests (crates/voltip-asr-local/tests/real.rs, `#[ignore]`d elsewhere) on the
# machine's CPU: the same VOLTIP_LOCAL_* inputs as locally, copied to <dir>\models\ and handed to the
# gate through <dir>\real.env.
stage_real_models() {
  local var src dest lines=()
  for var in VOLTIP_LOCAL_MODEL_DIR VOLTIP_LOCAL_GGUF VOLTIP_LOCAL_STREAM_DIR VOLTIP_LOCAL_VAD_MODEL VOLTIP_LOCAL_SAMPLE_WAV; do
    src=${!var:-}
    [ -n "$src" ] && [ -e "$src" ] || { echo "windows-remote: set $var to the local file / directory (see crates/voltip-asr-local/tests/real.rs)" >&2; exit 2; }
  done
  remote_ps <<<'Remove-Item -Recurse -Force (Join-Path $Dir "models") -ErrorAction SilentlyContinue; New-Item -ItemType Directory -Force (Join-Path $Dir "models") | Out-Null'
  for var in VOLTIP_LOCAL_MODEL_DIR VOLTIP_LOCAL_GGUF VOLTIP_LOCAL_STREAM_DIR VOLTIP_LOCAL_VAD_MODEL VOLTIP_LOCAL_SAMPLE_WAV; do
    src=${!var}
    dest=models/$(basename "$src")
    scp -q -r "${ssh_opts[@]}" "$src" "$host:$scp_dir/$dest"
    lines+=("$var=$dir\\${dest//\//\\}")
  done
  printf '%s\n' "${lines[@]}" >"$local_dir/real.env"
  put '' "$local_dir/real.env"
}

# The loopback gate's inputs: the recording to play, and where the test writes what it recorded.
stage_loopback() {
  local lines=() play=${VOLTIP_LOOPBACK_PLAY:-}
  remote_ps <<<'$out = Join-Path $Dir "loopback-out.wav"; if (Test-Path $out) { Remove-Item $out }'
  if [ -n "$play" ]; then
    [ -f "$play" ] || { echo "windows-remote: VOLTIP_LOOPBACK_PLAY=$play is not a file" >&2; exit 2; }
    scp -q "${ssh_opts[@]}" "$play" "$host:$scp_dir/loopback-play.wav"
    lines+=("VOLTIP_LOOPBACK_PLAY=$dir\\loopback-play.wav" "VOLTIP_LOOPBACK_OUT=$dir\\loopback-out.wav")
  fi
  printf '%s\n' "${lines[@]}" >"$local_dir/loopback.env"
  put '' "$local_dir/loopback.env"
}

gate() {
  local name=${1:-} args
  case $name in
    test) args='test --workspace --all-targets --no-fail-fast' ;;
    clippy) args='clippy --workspace --all-targets -- -D warnings' ;;
    real) args="test -p voltip-asr-local --test real -- --ignored --nocapture --test-threads=1${2:+ $2}" ;;
    # The lone-key trigger's low-level hooks (docs/dictation.md §13.1), fed with SendInput: they need
    # the interactive session the scheduled task runs in, never an SSH one.
    hooks) args='test -p voltip-hooks --lib -- --ignored --nocapture --test-threads=1' ;;
    # LAN discovery (docs/pairing.md 「局域网发现」): real multicast DNS through Windows' own stack.
    mdns) args='test -p voltip-core --lib discovery -- --ignored --nocapture --test-threads=1' ;;
    # Recording the computer's sound (docs/dictation.md §22): WASAPI loopback of the default output,
    # in the logged-on user's session where the audio engine runs.
    loopback) args='test -p voltip-audio --test loopback -- --ignored --nocapture --test-threads=1' ;;
    # Echo cancellation of the mixed take (docs/dictation.md §22.6): its timing means something only
    # in an optimised build on the machine being judged.
    echo) args='test --release -p voltip-audio --test echo -- --include-ignored --nocapture --test-threads=1' ;;
    *) echo "usage: $0 gate test|clippy|real|hooks|mdns|loopback|echo [test-name filter]" >&2; exit 2 ;;
  esac
  if [ "$name" = real ]; then stage_real_models; fi
  if [ "$name" = loopback ]; then stage_loopback; fi
  put '' scripts/windows-remote-gate.ps1
  remote_ps Gate="$name" CargoArgs="$args" <<'PS'
if ($Gate -notin @('real', 'loopback')) { Remove-Item (Join-Path $Dir "$Gate.env") -ErrorAction SilentlyContinue }
$task = "voltip-gate-$Gate"
$runner = Join-Path $Dir 'windows-remote-gate.ps1'
$argv = "-NoProfile -NonInteractive -ExecutionPolicy Bypass -File `"$runner`" -Dir `"$Dir`" -Gate $Gate -CargoArgs `"$CargoArgs`""
$action = New-ScheduledTaskAction -Execute 'powershell.exe' -Argument $argv
$user = [Security.Principal.WindowsIdentity]::GetCurrent().Name
$principal = New-ScheduledTaskPrincipal -UserId $user -LogonType Interactive -RunLevel Highest
Register-ScheduledTask -TaskName $task -Action $action -Principal $principal -Force | Out-Null
Remove-Item (Join-Path $Dir "$Gate.log") -ErrorAction SilentlyContinue
Start-ScheduledTask -TaskName $task
"windows-remote: started $task in the session of $user"
PS
  local rc=0
  wait_log "$name" "voltip-gate-$name" || rc=$?
  if [ "$name" = loopback ] && scp -q "${ssh_opts[@]}" "$host:$scp_dir/loopback-out.wav" "$local_dir/loopback-out.wav" 2>/dev/null; then
    echo "windows-remote: the recording is at $local_dir/loopback-out.wav"
  fi
  return "$rc"
}

smoke() {
  local dist=${1:-dist/windows-x64} models=${VOLTIP_WINDOWS_MODELS:-sense-voice-small,qwen3-asr-0.6b} f rc=0
  local files=(voltip-desktop.exe sherpa-onnx-c-api.dll onnxruntime.dll onnxruntime_providers_shared.dll vulkan-1.dll)
  for f in "${files[@]}"; do
    [ -f "$dist/$f" ] || { echo "windows-remote: $dist/$f missing (make windows-x64)" >&2; exit 2; }
  done
  remote_ps <<<'New-Item -ItemType Directory -Force (Join-Path $Dir "dist") | Out-Null'
  put dist/ "${files[@]/#/$dist/}"
  put '' scripts/smoke-native-cli.ps1 scripts/windows-remote-smoke.ps1
  remote_ps Models="$models" <<'PS'
$pwsh = 'C:\Program Files\PowerShell\7\pwsh.exe'
if (-not (Test-Path $pwsh)) {
  $pkg = Get-AppxPackage -AllUsers -Name Microsoft.PowerShell | Where-Object { $_.Version -like '7.*' } | Sort-Object { [version]$_.Version } | Select-Object -Last 1
  if ($pkg) { $pwsh = Join-Path $pkg.InstallLocation 'pwsh.exe' }
}
if (-not (Test-Path $pwsh)) { throw 'PowerShell 7 not found (smoke-native-cli.ps1 needs it)' }
$runner = Join-Path $Dir 'windows-remote-smoke.ps1'
$argv = "-NoProfile -NonInteractive -ExecutionPolicy Bypass -File `"$runner`" -Dir `"$Dir`" -Models $Models"
$action = New-ScheduledTaskAction -Execute $pwsh -Argument $argv
$principal = New-ScheduledTaskPrincipal -UserId 'SYSTEM' -LogonType ServiceAccount -RunLevel Highest
Register-ScheduledTask -TaskName 'voltip-smoke-native' -Action $action -Principal $principal -Force | Out-Null
Remove-Item (Join-Path $Dir 'smoke.log') -ErrorAction SilentlyContinue
Start-ScheduledTask -TaskName 'voltip-smoke-native'
"windows-remote: started voltip-smoke-native as SYSTEM ($pwsh)"
PS
  wait_log smoke voltip-smoke-native || rc=$?
  if scp -q "${ssh_opts[@]}" "$host:$scp_dir/smoke/summary.txt" "$local_dir/smoke-summary.txt" 2>/dev/null; then
    cat "$local_dir/smoke-summary.txt"
  fi
  return "$rc"
}

case "${1:-}" in
  sync) sync ;;
  gate) shift; gate "$@" ;;
  smoke) shift; smoke "$@" ;;
  # Re-attach to a run that is still going (after a timeout or a lost terminal).
  wait)
    case "${2:-}" in
      test | clippy | real) wait_log "$2" "voltip-gate-$2" ;;
      smoke) wait_log smoke voltip-smoke-native ;;
      hooks | mdns | loopback | echo) wait_log "$2" "voltip-gate-$2" ;;
      *) echo "usage: $0 wait test|clippy|real|hooks|mdns|loopback|echo|smoke" >&2; exit 2 ;;
    esac
    ;;
  ps) remote_ps ;;
  *) echo "usage: $0 sync | gate test|clippy|real | smoke [dist-dir] | wait test|clippy|real|smoke | ps" >&2; exit 2 ;;
esac
