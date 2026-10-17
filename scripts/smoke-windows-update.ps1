#requires -Version 7
<#
.SYNOPSIS
  Run the NSIS installer the way the in-app updater does and count the installer's visible frames.

.DESCRIPTION
  What it proves (docs/dictation.md §9; user request 2026-10-17: an in-app update showed the
  installer's window), on a machine with an interactive desktop (CI `windows-native`) where Voltip
  is already installed for the current user:
    1. `/P /UPDATE /R /ARGS`, what Voltip 0.1.6 and earlier start the new release's installer with
       (the plugin's passive mode): the installer's own hook (src-tauri/nsis/installer-hooks.nsh)
       keeps its window invisible from the first frame;
    2. `/S /UPDATE /R /ARGS`, what later releases start it with (the quiet mode, `/S`): no window;
    3. after each run the installer exits with 0 and opens the installed Voltip again (`/R`), which
       `--quit` then ends.
  While an installer runs, its top-level windows are sampled every few milliseconds; a window
  counts as seen when it is visible and not made fully transparent. One seen sample fails the run.
  A summary is written to <OutDir>/summary.txt.

.PARAMETER Setup
  Path to the Voltip_<version>_x64-setup.exe already installed on this machine.
#>
param(
  [Parameter(Mandatory = $true)] [string] $Setup,
  [string] $OutDir = 'smoke-windows-update',
  [int] $InstallTimeoutSec = 300,
  [int] $RelaunchTimeoutSec = 90,
  [int] $SampleMs = 5
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if (-not $IsWindows) { throw 'smoke-windows-update: Windows only' }

$Setup = (Resolve-Path -LiteralPath $Setup).Path
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$OutDir = (Resolve-Path -LiteralPath $OutDir).Path
$summary = [System.Collections.Generic.List[string]]::new()
function Note([string] $line) { $summary.Add($line); Write-Host "smoke-windows-update: $line" }

Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
public sealed class InstallerFrames {
  public int Samples;
  public int SeenSamples;
  public int MostWindows;
  public int HiddenByHook;
  public string FirstSeen = "";
}
public static class VoltipInstallerWindows {
  delegate bool EnumProc(IntPtr hwnd, IntPtr lparam);
  [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc proc, IntPtr lparam);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
  [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr hwnd);
  [DllImport("user32.dll")] static extern int GetWindowLongW(IntPtr hwnd, int index);
  [DllImport("user32.dll")] static extern bool GetLayeredWindowAttributes(IntPtr hwnd, out uint key, out byte alpha, out uint flags);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetWindowTextW(IntPtr hwnd, StringBuilder text, int max);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassNameW(IntPtr hwnd, StringBuilder text, int max);
  const int GWL_EXSTYLE = -20;
  const int WS_EX_LAYERED = 0x80000;
  const uint LWA_ALPHA = 2;

  // Layered with an alpha of 0: on the screen it paints nothing, whatever its visible flag says.
  static bool Transparent(IntPtr hwnd) {
    if ((GetWindowLongW(hwnd, GWL_EXSTYLE) & WS_EX_LAYERED) == 0) return false;
    uint key, flags;
    byte alpha;
    return GetLayeredWindowAttributes(hwnd, out key, out alpha, out flags) && (flags & LWA_ALPHA) != 0 && alpha == 0;
  }

  static string Describe(IntPtr hwnd) {
    var title = new StringBuilder(256);
    var cls = new StringBuilder(256);
    GetWindowTextW(hwnd, title, title.Capacity);
    GetClassNameW(hwnd, cls, cls.Capacity);
    return "0x" + hwnd.ToString("x") + " class=" + cls + " title=" + title;
  }

  // Samples the top-level windows of `installer` every `intervalMs` until it exits or `timeoutMs`
  // passes; returns false on the timeout.
  public static bool Watch(Process installer, int intervalMs, int timeoutMs, InstallerFrames frames) {
    var pid = (uint)installer.Id;
    var clock = Stopwatch.StartNew();
    while (!installer.HasExited) {
      if (clock.ElapsedMilliseconds > timeoutMs) return false;
      var windows = 0;
      var seen = false;
      EnumWindows((hwnd, _) => {
        uint owner;
        GetWindowThreadProcessId(hwnd, out owner);
        if (owner != pid) return true;
        windows++;
        var transparent = Transparent(hwnd);
        if (transparent) frames.HiddenByHook++;
        if (IsWindowVisible(hwnd) && !transparent) {
          if (!seen && frames.FirstSeen.Length == 0) frames.FirstSeen = Describe(hwnd);
          seen = true;
        }
        return true;
      }, IntPtr.Zero);
      frames.Samples++;
      if (seen) frames.SeenSamples++;
      if (windows > frames.MostWindows) frames.MostWindows = windows;
      Thread.Sleep(intervalMs);
    }
    return true;
  }
}
'@

# Where the per-user install put the exe: the uninstall key's InstallLocation; a runner that runs as
# SYSTEM gets the 32-bit NSIS stub's file-system redirection into SysWOW64 (see ci.yml).
function Installed-Exe {
  $key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Voltip'
  $location = ((Get-ItemProperty -Path $key -Name InstallLocation -ErrorAction SilentlyContinue).InstallLocation) -replace '"', ''
  $candidates = @($location, ($location -replace '(?i)\\system32\\', '\SysWOW64\'), (Join-Path $env:LOCALAPPDATA 'Voltip')) | Where-Object { $_ } | Select-Object -Unique
  $exe = $candidates | ForEach-Object { Join-Path $_ 'voltip-desktop.exe' } | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
  if (-not $exe) { throw "smoke-windows-update: Voltip is not installed (looked in: $($candidates -join '; '))" }
  (Resolve-Path -LiteralPath $exe).Path
}

function Running([string] $exe) {
  @(Get-Process -Name 'voltip-desktop' -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $exe })
}

# Ends the installed instance with `--quit`, and by force if it does not go.
function Stop-Installed([string] $exe) {
  $running = Running $exe
  if ($running.Count -eq 0) { return }
  $quit = Start-Process -FilePath $exe -ArgumentList '--quit' -PassThru
  [void]$quit.WaitForExit(30000)
  foreach ($p in $running) {
    if (-not $p.WaitForExit(30000)) {
      Note "pid $($p.Id) did not quit; stopped"
      Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
    }
  }
}

$exe = Installed-Exe
$failures = [System.Collections.Generic.List[string]]::new()
try {
  Note "setup $Setup ($((Get-FileHash -LiteralPath $Setup -Algorithm SHA256).Hash.ToLowerInvariant()))"
  Note "installed $exe"
  # `Window`: whether the installer creates a window at all. The passive run does, and has to keep
  # it hidden (the sampler seeing it hidden shows the check is not passing by missing it); the
  # silent run has none.
  $runs = @(
    @{ Name = 'passive update run (0.1.6 and earlier)'; Arguments = '/P /UPDATE /R /ARGS'; Window = $true },
    @{ Name = 'silent update run (after 0.1.6)'; Arguments = '/S /UPDATE /R /ARGS'; Window = $false }
  )
  foreach ($run in $runs) {
    Stop-Installed $exe
    $installer = Start-Process -FilePath $Setup -ArgumentList $run.Arguments -PassThru
    # Holding the handle keeps ExitCode readable after the process ends (a Start-Process quirk).
    $null = $installer.Handle
    $frames = [InstallerFrames]::new()
    $finished = [VoltipInstallerWindows]::Watch($installer, $SampleMs, $InstallTimeoutSec * 1000, $frames)
    if (-not $finished) {
      Stop-Process -Id $installer.Id -Force -ErrorAction SilentlyContinue
      $failures.Add("$($run.Name): the installer did not finish within ${InstallTimeoutSec}s")
      continue
    }
    $installer.WaitForExit()
    Note ("$($run.Name) [$($run.Arguments)]: exit $($installer.ExitCode); $($frames.Samples) samples, " +
      "$($frames.SeenSamples) with a visible installer window; at most $($frames.MostWindows) window(s); " +
      "$($frames.HiddenByHook) transparent sightings")
    if ($frames.SeenSamples -gt 0) { $failures.Add("$($run.Name): the installer window was on the screen ($($frames.FirstSeen))") }
    if ($installer.ExitCode -ne 0) { $failures.Add("$($run.Name): the installer exited with $($installer.ExitCode)") }
    if ($run.Window -and $frames.HiddenByHook -eq 0) { $failures.Add("$($run.Name): no hidden installer window was sampled; the check saw nothing to judge") }

    # `/R`: the installer opens Voltip again when it is done.
    $deadline = (Get-Date).AddSeconds($RelaunchTimeoutSec)
    while ((Running $exe).Count -eq 0 -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 250 }
    $reopened = Running $exe
    if ($reopened.Count -eq 0) {
      $failures.Add("$($run.Name): Voltip was not opened again within ${RelaunchTimeoutSec}s")
    } else {
      Note "$($run.Name): Voltip opened again (pid $($reopened[0].Id))"
    }
    Stop-Installed $exe
  }
} finally {
  Stop-Installed $exe
  foreach ($failure in $failures) { Note "FAILED: $failure" }
  if ($failures.Count -eq 0) { Note 'OK' }
  $summary | Set-Content -LiteralPath (Join-Path $OutDir 'summary.txt') -Encoding utf8
}
if ($failures.Count -gt 0) { throw "smoke-windows-update: $($failures.Count) check(s) failed (see $OutDir/summary.txt)" }
