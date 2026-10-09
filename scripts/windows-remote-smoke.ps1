#requires -Version 7
# The headless native run for scripts/windows-remote.sh, as a SYSTEM scheduled task. The app takes
# its data directory from the Known Folder API, which no environment variable redirects, so under
# SYSTEM the models land in the system profile and never in a real user's Voltip library. Runs
# smoke-native-cli.ps1 (copied next to this file) on <Dir>\dist\voltip-desktop.exe; log in
# <Dir>\smoke.log, the run's summary.txt in <Dir>\smoke\, last log line EXIT=<0|1>.
param([string]$Dir, [string]$Models)
if (-not $Dir -or -not $Models) { exit 2 }
$log = Join-Path $Dir 'smoke.log'
$started = Get-Date
"whoami: $(whoami)  data root: $([Environment]::GetFolderPath('ApplicationData'))  pwsh $($PSVersionTable.PSVersion)" | Set-Content $log -Encoding utf8
$code = 1
try {
  & (Join-Path $Dir 'smoke-native-cli.ps1') -Binary (Join-Path $Dir 'dist\voltip-desktop.exe') -Models $Models -OutDir (Join-Path $Dir 'smoke') *>> $log
  $code = 0
} catch {
  "FAILED: $_" | Add-Content $log -Encoding utf8
}
"`nEXIT=$code elapsed=$([int]((Get-Date) - $started).TotalSeconds)s" | Add-Content $log -Encoding utf8
