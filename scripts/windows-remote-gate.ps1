# One native cargo gate for scripts/windows-remote.sh, run as a scheduled task in the logged-on
# user's session (Windows PowerShell 5.1): the repo clone under -Dir, the MSVC developer
# environment, cargo's output copied byte for byte into <Dir>\<Gate>.log by cmd, and a last line
# EXIT=<cargo's exit code>. Anything that fails before cargo runs is logged and ends in EXIT=1.
param([string]$Dir, [string]$Gate, [string]$CargoArgs)
if (-not $Dir -or -not $Gate -or -not $CargoArgs) { exit 2 }
$log = Join-Path $Dir "$Gate.log"
$started = Get-Date
$code = 1
[IO.File]::WriteAllText($log, "windows-remote-gate $Gate`n")
try {
  $ErrorActionPreference = 'Stop'
  Set-Location -LiteralPath (Join-Path $Dir 'repo')
  $env:CARGO_TARGET_DIR = Join-Path $Dir 'target'
  $env:CARGO_TERM_COLOR = 'never'
  # Built-in engine defaults are compile-time inputs; a gate builds without them, like CI's tests.
  Remove-Item Env:VOLTIP_* -ErrorAction SilentlyContinue
  # <Dir>\<Gate>.env (NAME=value lines, written by the `real` gate): the real-model test inputs.
  $envFile = Join-Path $Dir "$Gate.env"
  if (Test-Path $envFile) {
    foreach ($line in Get-Content $envFile) {
      if ($line -match '^([A-Z_][A-Z0-9_]*)=(.*)$') { Set-Item "Env:$($Matches[1])" $Matches[2] }
    }
  }
  $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
  $vs = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
  if (-not $vs) { throw 'no Visual Studio or Build Tools with the C++ x64 tools (vswhere found none)' }
  & (Join-Path $vs 'Common7\Tools\Launch-VsDevShell.ps1') -Arch amd64 -HostArch amd64 -SkipAutomaticLocation | Out-Null
  # tauri::generate_context!() needs the frontend directories to exist (Makefile frontend-dist-dirs).
  New-Item -ItemType Directory -Force apps\desktop\dist, apps\mobile\dist | Out-Null
  [IO.File]::AppendAllText($log, "commit $(git rev-parse --short HEAD)  cargo $CargoArgs`n")
  $ErrorActionPreference = 'Continue'
  cmd /c "cargo $CargoArgs >> `"$log`" 2>&1"
  $code = $LASTEXITCODE
} catch {
  [IO.File]::AppendAllText($log, "windows-remote-gate: $_`n")
}
[IO.File]::AppendAllText($log, "`nEXIT=$code elapsed=$([int]((Get-Date) - $started).TotalSeconds)s`n")
