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
  Before each run the script asks the Restart Manager, read only, which processes hold the
  installed exe. The installer shuts every one of them down by force (tauri-cli 2.12.4's
  CheckIfAppIsRunning: RmShutdown with RmForceShutdown), so a holder that is still there once the
  script has stopped Voltip fails the run before the installer starts.
  A summary is written to <OutDir>/summary.txt, or summary-<Run>.txt for one run.

  -Snapshot <name> runs no installer: it writes the machine's state (image, memory, the process
  table, the exe's holders) to <OutDir>/machine-<name>.txt. CI uploads it before the first run, so
  it outlives a runner the platform loses inside a run (2026-11-26 and 11-27, see ci.yml).

.PARAMETER Setup
  Path to the Voltip_<version>_x64-setup.exe already installed on this machine.

.PARAMETER Run
  Which update runs: both (the default), or one of them, as CI runs each in a step of its own.
#>
param(
  [Parameter(Mandatory = $true)] [string] $Setup,
  [string] $OutDir = 'smoke-windows-update',
  [ValidateSet('both', 'passive', 'silent')] [string] $Run = 'both',
  [string] $Snapshot = '',
  # On CI a run's installer takes 5-8 s and Voltip is open again about 30 ms after it ends
  # (November 2026). With these bounds a run ends by itself in under five minutes, inside its CI
  # step's limit, so its summary is always written.
  [int] $InstallTimeoutSec = 120,
  [int] $RelaunchTimeoutSec = 45,
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

// Which processes hold a file, as the Restart Manager sees it: the list the installer's
// RmGetList finds before its RmShutdown. This session only reads; it never shuts anything down.
public static class VoltipExeHolders {
  [StructLayout(LayoutKind.Sequential)]
  struct RM_UNIQUE_PROCESS {
    public int dwProcessId;
    public System.Runtime.InteropServices.ComTypes.FILETIME ProcessStartTime;
  }
  [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
  struct RM_PROCESS_INFO {
    public RM_UNIQUE_PROCESS Process;
    [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 256)] public string strAppName;
    [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 64)] public string strServiceShortName;
    public int ApplicationType;
    public uint AppStatus;
    public uint TSSessionId;
    [MarshalAs(UnmanagedType.Bool)] public bool bRestartable;
  }
  [DllImport("rstrtmgr.dll", CharSet = CharSet.Unicode)]
  static extern int RmStartSession(out uint session, int flags, StringBuilder key);
  [DllImport("rstrtmgr.dll")]
  static extern int RmEndSession(uint session);
  [DllImport("rstrtmgr.dll", CharSet = CharSet.Unicode)]
  static extern int RmRegisterResources(uint session, uint files, string[] names, uint applications, RM_UNIQUE_PROCESS[] processes, uint services, string[] serviceNames);
  [DllImport("rstrtmgr.dll")]
  static extern int RmGetList(uint session, out uint needed, ref uint count, [In, Out] RM_PROCESS_INFO[] infos, out uint rebootReasons);
  const int ERROR_MORE_DATA = 234;
  static readonly string[] Kinds = { "unknown", "main window", "other window", "service", "Explorer", "console" };

  // One "pid name (kind, session n)" line per holder of `path`.
  public static List<string> Of(string path) {
    uint session;
    var key = new StringBuilder(64);
    var rc = RmStartSession(out session, 0, key);
    if (rc != 0) throw new InvalidOperationException("RmStartSession failed with " + rc);
    try {
      rc = RmRegisterResources(session, 1, new[] { path }, 0, null, 0, null);
      if (rc != 0) throw new InvalidOperationException("RmRegisterResources failed with " + rc);
      uint needed, reasons, count = 0;
      RM_PROCESS_INFO[] infos = null;
      rc = RmGetList(session, out needed, ref count, null, out reasons);
      for (var tries = 0; rc == ERROR_MORE_DATA && tries < 5; tries++) {
        count = needed;
        infos = new RM_PROCESS_INFO[count];
        rc = RmGetList(session, out needed, ref count, infos, out reasons);
      }
      if (rc != 0) throw new InvalidOperationException("RmGetList failed with " + rc);
      var holders = new List<string>();
      for (var i = 0; i < count; i++) {
        var p = infos[i];
        var kind = p.ApplicationType >= 0 && p.ApplicationType < Kinds.Length ? Kinds[p.ApplicationType] : "type " + p.ApplicationType;
        if (!string.IsNullOrEmpty(p.strServiceShortName)) kind += " " + p.strServiceShortName;
        holders.Add(p.Process.dwProcessId + " " + p.strAppName + " (" + kind + ", session " + p.TSSessionId + ")");
      }
      return holders;
    } finally {
      RmEndSession(session);
    }
  }
}
'@

# Where the per-user install put the exe: the uninstall key's InstallLocation; a runner that runs as
# SYSTEM gets the 32-bit NSIS stub's file-system redirection into SysWOW64 (see ci.yml).
function Installed-Exe {
  $key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Voltip'
  $entry = Get-ItemProperty -Path $key -Name InstallLocation -ErrorAction SilentlyContinue
  $location = if ($entry) { $entry.InstallLocation -replace '"', '' } else { '' }
  $candidates = @($location, ($location -replace '(?i)\\system32\\', '\SysWOW64\'), (Join-Path $env:LOCALAPPDATA 'Voltip')) | Where-Object { $_ } | Select-Object -Unique
  $exe = $candidates | ForEach-Object { Join-Path $_ 'voltip-desktop.exe' } | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
  if (-not $exe) { throw "smoke-windows-update: Voltip is not installed (looked in: $($candidates -join '; '))" }
  (Resolve-Path -LiteralPath $exe).Path
}

# The running instances of the installed exe, the `--quit` helper included. Callers wrap the call
# in @(): a function's empty array reaches the caller as $null, which has no Count under strict
# mode (CI 2026-10-17).
function Running([string] $exe) {
  @(Get-Process -Name 'voltip-desktop' -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $exe })
}

# The holders of the exe, waiting up to `$seconds` for them to go; empty when none is left.
function Holders-After([string] $exe, [int] $seconds) {
  $deadline = (Get-Date).AddSeconds($seconds)
  while ($true) {
    $holders = @([VoltipExeHolders]::Of($exe))
    if ($holders.Count -eq 0 -or (Get-Date) -ge $deadline) { return $holders }
    Start-Sleep -Milliseconds 500
  }
}

# Ends the installed instances with `--quit`, and by force whatever is still there 20 s later: an
# instance that did not quit, or the `--quit` helper itself.
function Stop-Installed([string] $exe) {
  if (@(Running $exe).Count -eq 0) { return }
  $quit = Start-Process -FilePath $exe -ArgumentList '--quit' -PassThru
  $deadline = (Get-Date).AddSeconds(20)
  while (@(Running $exe).Count -gt 0 -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 250 }
  foreach ($p in @(Running $exe)) {
    $what = if ($p.Id -eq $quit.Id) { 'the --quit helper' } else { 'Voltip' }
    Note "$what (pid $($p.Id)) did not end within 20 s; stopped"
    Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
    if (-not $p.WaitForExit(5000)) { Note "pid $($p.Id) was still there 5 s after Stop-Process" }
  }
}

# The machine as it is now, for a run that may not get to write anything afterwards.
function Write-Machine([string] $name) {
  $lines = [System.Collections.Generic.List[string]]::new()
  function Section([string] $title, [scriptblock] $body) {
    try { foreach ($line in @(& $body)) { $lines.Add("$title $line") } } catch { $lines.Add("$title failed: $_") }
  }
  Section 'utc' { (Get-Date).ToUniversalTime().ToString('o') }
  Section 'image' { "$env:ImageOS $env:ImageVersion, runner $env:RUNNER_NAME" }
  Section 'os' {
    $os = Get-CimInstance Win32_OperatingSystem
    "$($os.Caption) $($os.Version) build $($os.BuildNumber), booted $($os.LastBootUpTime.ToUniversalTime().ToString('o'))"
    # Both pairs are in KiB.
    'memory free {0:N2} of {1:N2} GiB; commit free {2:N2} of {3:N2} GiB' -f ($os.FreePhysicalMemory / 1MB), ($os.TotalVisibleMemorySize / 1MB), ($os.FreeVirtualMemory / 1MB), ($os.TotalVirtualMemorySize / 1MB)
  }
  Section 'cpu' { Get-CimInstance Win32_Processor | ForEach-Object { "$($_.Name), $($_.NumberOfLogicalProcessors) logical, load $($_.LoadPercentage) %" } }
  Section 'user' { "$(whoami); $(@(whoami /groups) -match 'Mandatory Label' -replace '\s+', ' ')" }
  Section 'setup' { "$Setup ($((Get-FileHash -LiteralPath $Setup -Algorithm SHA256).Hash.ToLowerInvariant()))" }
  Section 'installed' {
    $exe = Installed-Exe
    $file = Get-Item -LiteralPath $exe
    "$exe, $($file.Length) bytes, written $($file.LastWriteTimeUtc.ToString('o'))"
    $holders = @([VoltipExeHolders]::Of($exe))
    "holders: $(if ($holders.Count) { $holders -join '; ' } else { 'none' })"
  }
  # Every process with its parent, session, start, memory and handles; the command line only for
  # Voltip and its installers.
  Section 'process' {
    $all = @(Get-CimInstance Win32_Process)
    "count $($all.Count), handles $(($all | Measure-Object -Property HandleCount -Sum).Sum)"
    foreach ($p in ($all | Sort-Object ProcessId)) {
      $started = if ($p.CreationDate) { $p.CreationDate.ToUniversalTime().ToString('HH:mm:ss') } else { '-' }
      $line = '{0} parent {1} session {2} started {3} ws {4:N0} MB handles {5} {6}' -f $p.ProcessId, $p.ParentProcessId, $p.SessionId, $started, ($p.WorkingSetSize / 1MB), $p.HandleCount, $p.Name
      if ($p.Name -match '^(voltip-desktop|Voltip_.*setup)\.exe$' -and $p.CommandLine) { $line += " :: $($p.CommandLine)" }
      $line
    }
  }
  $path = Join-Path $OutDir "machine-$name.txt"
  $lines | Set-Content -LiteralPath $path -Encoding utf8
  Write-Host "smoke-windows-update: the machine's state is in $path ($($lines.Count) lines)"
}

if ($Snapshot) {
  Write-Machine $Snapshot
  return
}

$exe = Installed-Exe
$failures = [System.Collections.Generic.List[string]]::new()
$summaryFile = if ($Run -eq 'both') { 'summary.txt' } else { "summary-$Run.txt" }
try {
  Note "setup $Setup ($((Get-FileHash -LiteralPath $Setup -Algorithm SHA256).Hash.ToLowerInvariant()))"
  Note "installed $exe"
  # `Window`: whether the installer creates a window at all. The passive run does, and has to keep
  # it hidden (the sampler seeing it hidden shows the check is not passing by missing it); the
  # silent run has none.
  $runs = @(
    @{ Key = 'passive'; Name = 'passive update run (0.1.6 and earlier)'; Arguments = '/P /UPDATE /R /ARGS'; Window = $true },
    @{ Key = 'silent'; Name = 'silent update run (after 0.1.6)'; Arguments = '/S /UPDATE /R /ARGS'; Window = $false }
  ) | Where-Object { $Run -eq 'both' -or $_.Key -eq $Run }
  # Not `$run`: names are case-insensitive, so that would be the -Run parameter, whose ValidateSet
  # refuses a hashtable (PR #274's first CI run, 46077300426).
  foreach ($update in $runs) {
    Stop-Installed $exe
    # The installer skips its own check when the Restart Manager does not answer, so a question
    # that fails here is noted, not judged.
    $holders = @()
    try { $holders = @(Holders-After $exe 20) } catch { Note "$($update.Name): the Restart Manager could not be asked who holds the exe: $_" }
    if ($holders.Count -gt 0) {
      $failures.Add("$($update.Name): with Voltip stopped, these still held $exe, and the installer would have shut them down by force: $($holders -join '; ')")
      continue
    }
    $installer = Start-Process -FilePath $Setup -ArgumentList $update.Arguments -PassThru
    # Holding the handle keeps ExitCode readable after the process ends (a Start-Process quirk).
    $null = $installer.Handle
    $frames = [InstallerFrames]::new()
    $finished = [VoltipInstallerWindows]::Watch($installer, $SampleMs, $InstallTimeoutSec * 1000, $frames)
    if (-not $finished) {
      Stop-Process -Id $installer.Id -Force -ErrorAction SilentlyContinue
      $failures.Add("$($update.Name): the installer did not finish within ${InstallTimeoutSec}s")
      continue
    }
    $installer.WaitForExit()
    Note ("$($update.Name) [$($update.Arguments)]: exit $($installer.ExitCode); $($frames.Samples) samples, " +
      "$($frames.SeenSamples) with a visible installer window; at most $($frames.MostWindows) window(s); " +
      "$($frames.HiddenByHook) transparent sightings")
    if ($frames.SeenSamples -gt 0) { $failures.Add("$($update.Name): the installer window was on the screen ($($frames.FirstSeen))") }
    if ($installer.ExitCode -ne 0) { $failures.Add("$($update.Name): the installer exited with $($installer.ExitCode)") }
    if ($update.Window -and $frames.HiddenByHook -eq 0) { $failures.Add("$($update.Name): no hidden installer window was sampled; the check saw nothing to judge") }

    # `/R`: the installer opens Voltip again when it is done.
    $deadline = (Get-Date).AddSeconds($RelaunchTimeoutSec)
    while (@(Running $exe).Count -eq 0 -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 250 }
    $reopened = @(Running $exe)
    if ($reopened.Count -eq 0) {
      $failures.Add("$($update.Name): Voltip was not opened again within ${RelaunchTimeoutSec}s")
    } else {
      Note "$($update.Name): Voltip opened again (pid $($reopened[0].Id))"
    }
    Stop-Installed $exe
  }
} catch {
  $failures.Add("the check stopped: $_")
} finally {
  try { Stop-Installed $exe } catch { $failures.Add("Voltip could not be stopped afterwards: $_") }
  foreach ($failure in $failures) { Note "FAILED: $failure" }
  if ($failures.Count -eq 0) { Note 'OK' }
  $summary | Set-Content -LiteralPath (Join-Path $OutDir $summaryFile) -Encoding utf8
}
if ($failures.Count -gt 0) { throw "smoke-windows-update: $($failures.Count) check(s) failed (see $OutDir/$summaryFile)" }
