#requires -Version 7
<#
.SYNOPSIS
  Drive the shipped Windows binary's real window: close to the tray, reopen, quit.

.DESCRIPTION
  What it proves (docs/dictation.md §15.4), on a machine with an interactive desktop (CI
  `windows-native`: GitHub's windows-2025 image has one):
    1. the GUI starts and maps its main window (a top-level window titled "Voltip");
    2. the tray icon and its menu were installed (the app's log);
    3. WM_CLOSE — what the title bar's × and Alt+F4 send — hides the window and the process keeps
       running (before 2026-09-28 the window was destroyed while the prewarmed pill window kept
       the process alive, with nothing left to show and no way to quit);
    4. a second launch without arguments shows the same window again (single instance);
    5. `--quit` ends the running instance with exit code 0.
  Each step has a timeout; the first failure stops the run with a non-zero exit. A summary is
  written to <OutDir>/summary.txt, the app's log to <OutDir>/app.log.

.PARAMETER Binary
  Path to voltip-desktop.exe.
#>
param(
  [Parameter(Mandatory = $true)] [string] $Binary,
  [string] $OutDir = 'smoke-windows-gui',
  [int] $WindowTimeoutSec = 120,
  [int] $StepTimeoutSec = 30
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if (-not $IsWindows) { throw 'smoke-windows-gui: Windows only' }

$Binary = (Resolve-Path -LiteralPath $Binary).Path
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$OutDir = (Resolve-Path -LiteralPath $OutDir).Path
$log = Join-Path $OutDir 'app.log'
$summary = [System.Collections.Generic.List[string]]::new()
function Note([string] $line) { $summary.Add($line); Write-Host "smoke-windows-gui: $line" }

Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public static class VoltipWindows {
  delegate bool EnumProc(IntPtr hwnd, IntPtr lparam);
  [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc proc, IntPtr lparam);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetWindowTextW(IntPtr hwnd, StringBuilder text, int max);
  [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr hwnd);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hwnd);
  [DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr hwnd, uint msg, IntPtr w, IntPtr l);
  public const uint WM_CLOSE = 0x0010;
  // The top-level window of `pid` titled exactly `title`, visible or not; IntPtr.Zero when none.
  public static IntPtr Find(uint pid, string title) {
    IntPtr found = IntPtr.Zero;
    EnumWindows((hwnd, _) => {
      uint owner;
      GetWindowThreadProcessId(hwnd, out owner);
      if (owner != pid) return true;
      var text = new StringBuilder(256);
      GetWindowTextW(hwnd, text, text.Capacity);
      if (text.ToString() != title) return true;
      found = hwnd;
      return false;
    }, IntPtr.Zero);
    return found;
  }
}
'@

function Wait-Until([scriptblock] $condition, [int] $seconds, [string] $what) {
  $deadline = (Get-Date).AddSeconds($seconds)
  while ((Get-Date) -lt $deadline) {
    if (& $condition) { return }
    Start-Sleep -Milliseconds 250
  }
  throw "smoke-windows-gui: timed out after ${seconds}s waiting for $what"
}

function Log-Text { if (Test-Path -LiteralPath $log) { (Get-Content -LiteralPath $log -Raw) -replace "`e\[[0-9;]*m", '' } else { '' } }

$env:RUST_LOG = 'voltip=info'
$env:NO_COLOR = '1'
$app = $null
try {
  Note "binary $Binary ($((Get-FileHash -LiteralPath $Binary -Algorithm SHA256).Hash.ToLowerInvariant()))"
  $app = Start-Process -FilePath $Binary -PassThru -RedirectStandardError $log -RedirectStandardOutput (Join-Path $OutDir 'app.out')
  # Holding the handle keeps ExitCode readable after the process ends (a Start-Process quirk).
  $null = $app.Handle
  Note "started pid $($app.Id)"

  # 1. The main window (the pill window is titled "Voltip Overlay").
  $script:hwnd = [IntPtr]::Zero
  Wait-Until { $script:hwnd = [VoltipWindows]::Find([uint32]$app.Id, 'Voltip'); $script:hwnd -ne [IntPtr]::Zero -and [VoltipWindows]::IsWindowVisible($script:hwnd) } $WindowTimeoutSec 'the main window'
  $hwnd = $script:hwnd
  Note "main window 0x$($hwnd.ToString('x')) visible"

  # 2. The tray (installed from the setup hook once the core is up).
  Wait-Until { (Log-Text) -match 'tray icon (not )?installed' } $StepTimeoutSec 'the tray install line in the log'
  if ((Log-Text) -match 'tray icon not installed') { throw 'smoke-windows-gui: the tray icon was not installed (see app.log)' }
  Note (((Log-Text) -split "`n" | Where-Object { $_ -match 'tray icon installed' } | Select-Object -First 1).Trim())

  # 3. WM_CLOSE hides the window; the process stays.
  [void][VoltipWindows]::PostMessageW($hwnd, [VoltipWindows]::WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
  Wait-Until { -not [VoltipWindows]::IsWindowVisible($hwnd) } $StepTimeoutSec 'the window to hide after WM_CLOSE'
  # The quit path (no tray) exits right after the close request: 3 s without an exit rules it out.
  if ($app.WaitForExit(3000)) { throw "smoke-windows-gui: WM_CLOSE quit the app (exit $($app.ExitCode)) instead of hiding it" }
  if (-not [VoltipWindows]::IsWindow($hwnd)) { throw 'smoke-windows-gui: WM_CLOSE destroyed the main window' }
  Note 'WM_CLOSE: window hidden, process still running'

  # 4. A second launch shows it again and exits itself.
  $second = Start-Process -FilePath $Binary -PassThru
  $null = $second.Handle
  Wait-Until { [VoltipWindows]::IsWindowVisible($hwnd) } $StepTimeoutSec 'a second launch to show the window'
  if (-not $second.WaitForExit($StepTimeoutSec * 1000)) { throw 'smoke-windows-gui: the second launch did not exit' }
  Note "second launch (exit $($second.ExitCode)): window shown again"

  # 5. --quit ends the running instance.
  $quit = Start-Process -FilePath $Binary -ArgumentList '--quit' -PassThru
  if (-not $app.WaitForExit($StepTimeoutSec * 1000)) { throw 'smoke-windows-gui: --quit did not end the running instance' }
  [void]$quit.WaitForExit($StepTimeoutSec * 1000)
  Note "--quit: instance exited with $($app.ExitCode)"
  if ($app.ExitCode -ne 0) { throw "smoke-windows-gui: the instance exited with $($app.ExitCode)" }
  $app = $null
  Note 'OK'
} finally {
  if ($null -ne $app -and -not $app.HasExited) { Stop-Process -Id $app.Id -Force -ErrorAction SilentlyContinue }
  $summary | Set-Content -LiteralPath (Join-Path $OutDir 'summary.txt') -Encoding utf8
}
