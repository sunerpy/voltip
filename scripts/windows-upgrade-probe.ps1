#requires -Version 7
<#
.SYNOPSIS
  Update Voltip the way a user does and read which interface code each window runs afterwards.

.DESCRIPTION
  What it checks (docs/dictation.md §33.3; user report 2026-11-02: in the first run after
  installing 0.3.0 the dictionary page had no 学习建议 tab and the relay stayed at 认证中 until
  Voltip was restarted), on a desktop with WebView2 (CI `windows-native`, on demand), round after
  round:
    1. a clean start: no Voltip, no data directory, no WebView2 profile;
    2. the released Voltip (`-OldSetup`, 0.2.1) installed silently and started twice, its relay
       the build's own voltip-relay on this machine (`-Relay`); on the first start the
       simulations of step 6 run as a control: 0.2.1 has no check, so its window keeps them;
    3. this build's installer (`-NewSetup`) run while it runs, as an update does, in one of three
       ways, in turn: run while Voltip runs (`/S /UPDATE /R`, the installer ends Voltip); the way
       the in-app updater does it (tauri-plugin-updater starts `/S /UPDATE /R /ARGS <arguments>`
       and ends the app at once); the same for a Voltip started with `--start-hidden`, which must
       come back with no window shown (read from Windows: WebView2 reports a hidden window's page
       as visible) and is then shown by a second launch;
    4. once the installer has opened Voltip again, every window read over WebView2's debugging
       port (scripts/webview-probe.mjs): the entry script its document loaded, compared with the
       ones the installed program carries; the main window's dictionary page and its 学习建议 tab;
       the relay card reading connected;
    5. the same installer once more over itself (the same version), the same way, and step 4;
    6. the interface check (docs/dictation.md §33.4) on the running program, simulated over the
       debugging port: a first document naming an entry script the program does not carry, and
       a first document whose bundle declares another build, each go through two documents (the
       stale one, then the program's at a fresh address) and end on the program's interface; a
       window naming that script in every document goes through three and ends cleared
       (about:blank); back at its address the window runs the program's interface. Every window
       read must run the build the program's bundle declares.
  A window whose entry script the installed program does not carry, a dictionary page without the
  tab, a relay card that never reads connected, a main window shown or hidden against the variant,
  a window running another build, or an interface check that ends anywhere but stated in step 6
  fails the run; every observation goes to <OutDir>/summary.txt and <OutDir>/rounds.json.

  Nothing reaches a server: Voltip's outbound connections anywhere but this machine are blocked by
  a firewall rule on the installed exe, the settings written before the first start point the
  relay at this machine and leave automatic updates off. The machine is left as it was found: the
  rule, the WebView2 debugging policy, the relay, the installed Voltip and its data are removed.

.PARAMETER OldSetup
  The released installer to start from (Voltip_0.2.1_x64-setup.exe).
.PARAMETER NewSetup
  The installer under test (this build's Voltip_<version>_x64-setup.exe).
.PARAMETER Relay
  The build's voltip-relay.exe.
#>
param(
  [Parameter(Mandatory = $true)] [string] $OldSetup,
  [Parameter(Mandatory = $true)] [string] $NewSetup,
  [Parameter(Mandatory = $true)] [string] $Relay,
  [string] $OutDir = 'upgrade-probe',
  [int] $Rounds = 9,
  [int] $Port = 9222,
  [int] $RelayPort = 47830,
  [int] $TimeoutSec = 120,
  [int] $RelayConnectedSec = 30
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if (-not $IsWindows) { throw 'windows-upgrade-probe: Windows only' }

$OldSetup = (Resolve-Path -LiteralPath $OldSetup).Path
$NewSetup = (Resolve-Path -LiteralPath $NewSetup).Path
$Relay = (Resolve-Path -LiteralPath $Relay).Path
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$OutDir = (Resolve-Path -LiteralPath $OutDir).Path
$probeScript = Join-Path $PSScriptRoot 'webview-probe.mjs'
$summary = [System.Collections.Generic.List[string]]::new()
$failures = [System.Collections.Generic.List[string]]::new()
$records = [System.Collections.Generic.List[object]]::new()
function Note([string] $line) { $summary.Add($line); Write-Host "windows-upgrade-probe: $line" }

$installDir = Join-Path $env:LOCALAPPDATA 'Voltip'
$exePath = Join-Path $installDir 'voltip-desktop.exe'
# The per-user directories Voltip and its WebView2 profile use (`data_dir`: ProjectDirs
# dev/voltip/Voltip; the WebView2 user data folder: the bundle identifier).
$dataDir = Join-Path $env:APPDATA 'voltip\Voltip\data'
$profileDir = Join-Path $env:LOCALAPPDATA 'dev.voltip.desktop'
# HKLM: WebView2 ignores the per-user policy and the WEBVIEW2_* variables in an elevated host, and
# CI's runner runs everything elevated (learn.microsoft.com, "Develop secure WebView2 apps").
$policyKey = 'HKLM:\SOFTWARE\Policies\Microsoft\Edge\WebView2\AdditionalBrowserArguments'
$ruleName = 'voltip-upgrade-probe'

function Wait-Until([scriptblock] $condition, [int] $seconds, [string] $what) {
  $deadline = (Get-Date).AddSeconds($seconds)
  while ((Get-Date) -lt $deadline) {
    $value = & $condition
    if ($value) { return $value }
    Start-Sleep -Milliseconds 250
  }
  throw "timed out after ${seconds}s waiting for $what"
}

function Voltip-Processes { @(Get-Process -Name 'voltip-desktop' -ErrorAction SilentlyContinue) }

# The WebView2 processes of Voltip's profile: their command lines name its user data folder.
function WebView-Processes {
  @(Get-CimInstance Win32_Process -Filter "Name = 'msedgewebview2.exe'" -ErrorAction SilentlyContinue |
      Where-Object { $_.CommandLine -and $_.CommandLine.Contains('dev.voltip.desktop') })
}

function Describe-WebViews {
  @(WebView-Processes | ForEach-Object {
      $kind = if ($_.CommandLine -match '--type=([a-z-]+)') { $Matches[1] } else { 'browser' }
      [ordered]@{ pid = [int]$_.ProcessId; parent = [int]$_.ParentProcessId; type = $kind; started = $_.CreationDate.ToUniversalTime().ToString('o') }
    })
}

# Every interface entry script the program carries: Tauri embeds its assets under their paths.
# Every build the program's bundle declares (apps/desktop/vite.config.ts names each build; 0.2.1's
# bundle names none).
function Embedded-Builds([string] $exe) {
  $text = [System.Text.Encoding]::Latin1.GetString([System.IO.File]::ReadAllBytes($exe))
  @([regex]::Matches($text, 'voltip-ui-build-[0-9a-f]{32}(?![0-9A-Za-z])') | ForEach-Object { $_.Value } | Sort-Object -Unique)
}

function Embedded-Entries([string] $exe) {
  $text = [System.Text.Encoding]::Latin1.GetString([System.IO.File]::ReadAllBytes($exe))
  @([regex]::Matches($text, '/assets/index-[A-Za-z0-9_-]{6,}\.js') | ForEach-Object { $_.Value } | Sort-Object -Unique)
}

# Voltip's connections, counted, never named: to this machine and elsewhere (blocked).
function Connections([int[]] $ids) {
  $all = @(Get-NetTCPConnection -ErrorAction SilentlyContinue | Where-Object { $ids -contains [int]$_.OwningProcess })
  $local = @($all | Where-Object { $_.RemoteAddress -in @('127.0.0.1', '::1', '0.0.0.0', '::') })
  [ordered]@{ local = $local.Count; elsewhere = $all.Count - $local.Count }
}

function Stop-Voltip {
  foreach ($p in Voltip-Processes) {
    Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
    [void]$p.WaitForExit(30000)
  }
  # The profile's WebView2 processes leave by themselves once their host is gone.
  try { Wait-Until { @(WebView-Processes).Count -eq 0 } 60 'the WebView2 processes to exit' | Out-Null }
  catch { Note "WebView2 processes still running; stopped: $(@(WebView-Processes | ForEach-Object { $_.ProcessId }) -join ', ')"; WebView-Processes | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue } }
}

function Clear-Voltip {
  Stop-Voltip
  $uninstaller = Join-Path $installDir 'uninstall.exe'
  if (Test-Path -LiteralPath $uninstaller) {
    $u = Start-Process -FilePath $uninstaller -ArgumentList '/S' -PassThru
    if (-not $u.WaitForExit($TimeoutSec * 1000)) { $u.Kill($true) }
    # The NSIS uninstaller copies itself away and returns at once: wait for the files to go.
    try { Wait-Until { -not (Test-Path -LiteralPath $exePath) } 60 'the uninstaller' | Out-Null } catch { Note 'the uninstaller left voltip-desktop.exe behind' }
  }
  foreach ($dir in @($installDir, $profileDir, (Split-Path -Parent (Split-Path -Parent $dataDir)))) {
    if (Test-Path -LiteralPath $dir) { Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue }
  }
}

# Settings 0.2.1 and this build both read: the relay on this machine, no automatic updates.
function Write-Settings {
  New-Item -ItemType Directory -Force -Path $dataDir | Out-Null
  $settings = [ordered]@{
    schema = 1; theme = 'glass'; follow_system_theme = $false
    relay_url = "ws://127.0.0.1:$RelayPort/ws"; relay_enabled = $true; auto_update = $false
  }
  $settings | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $dataDir 'settings.json') -Encoding utf8NoBOM
}

function Run-Setup([string] $setup, [string[]] $arguments, [string] $what, [System.Diagnostics.Process] $endAtOnce = $null) {
  $p = Start-Process -FilePath $setup -ArgumentList $arguments -PassThru
  $null = $p.Handle
  # The updater ends the app right after starting the installer (`std::process::exit(0)`).
  if ($null -ne $endAtOnce -and -not $endAtOnce.HasExited) { Stop-Process -Id $endAtOnce.Id -Force -ErrorAction SilentlyContinue }
  if (-not $p.WaitForExit($TimeoutSec * 1000)) { $p.Kill($true); throw "$what did not finish within ${TimeoutSec}s" }
  if ($p.ExitCode -ne 0) { throw "$what exited with $($p.ExitCode)" }
}

# The ways an update reaches a running Voltip (step 3 of the description).
$variants = @('installer', 'updater', 'updater, started hidden')
function Update-Voltip([string] $variant, [System.Diagnostics.Process] $running, [string] $what) {
  switch ($variant) {
    'installer' { Run-Setup $NewSetup @('/S', '/UPDATE', '/R') $what }
    'updater' { Run-Setup $NewSetup @('/S', '/UPDATE', '/R', '/ARGS') $what $running }
    'updater, started hidden' { Run-Setup $NewSetup @('/S', '/UPDATE', '/R', '/ARGS', '--start-hidden') $what $running }
  }
}

# A property path of a JSON object, or $null where any step is missing (strict mode throws on a
# missing property).
function Field($object, [string] $path) {
  foreach ($name in $path.Split('.')) {
    if ($null -eq $object -or -not $object.PSObject.Properties[$name]) { return $null }
    $object = $object.$name
  }
  $object
}

# The process listening on the debugging port, and its parent: the WebView2 browser process and
# the Voltip that started it.
function Port-Owner {
  $listen = Get-NetTCPConnection -LocalPort $Port -State Listen -ErrorAction SilentlyContinue | Select-Object -First 1
  if (-not $listen) { return $null }
  $owner = Get-CimInstance Win32_Process -Filter "ProcessId = $($listen.OwningProcess)" -ErrorAction SilentlyContinue
  [ordered]@{ pid = [int]$listen.OwningProcess; parent = if ($owner) { [int]$owner.ParentProcessId } else { $null } }
}

# The page targets on the debugging port. Invoke-RestMethod passes a JSON array on as one object.
function Pages {
  try { $targets = Invoke-RestMethod -Uri "http://127.0.0.1:$Port/json/list" -TimeoutSec 5 } catch { return @() }
  @($targets | Where-Object { $_.type -eq 'page' })
}

# Waits for one running Voltip that is not in `$before`, then for two windows on the debugging
# port served by a browser process that Voltip started. A port still served by another Voltip's
# browser process after the timeout is recorded, not waited out: that is an observation.
function Wait-Voltip([int[]] $before, [string] $what) {
  $proc = Wait-Until { Voltip-Processes | Where-Object { $before -notcontains $_.Id } | Select-Object -First 1 } $TimeoutSec "Voltip to run ($what)"
  try {
    Wait-Until { $o = Port-Owner; $o -and $o.parent -eq $proc.Id -and @(Pages).Count -ge 2 } $TimeoutSec "the windows of pid $($proc.Id) on the debugging port ($what)" | Out-Null
  } catch {
    $o = Port-Owner
    Note "${what}: the debugging port is served by $(if ($o) { "pid $($o.pid), started by pid $($o.parent)" } else { 'nothing' }), not by a browser process of Voltip pid $($proc.Id); $(@(Pages).Count) page(s)"
    foreach ($b in @(WebView-Processes | Where-Object { $_.CommandLine -notmatch '--type=' })) {
      $flags = @([regex]::Matches($b.CommandLine, '--[a-z0-9-]+') | ForEach-Object { $_.Value } | Sort-Object -Unique) -join ' '
      Note "${what}: WebView2 browser pid $($b.ProcessId) <- $($b.ParentProcessId), flags: $flags"
    }
    Wait-Until { @(Pages).Count -ge 2 } $TimeoutSec "two windows on the debugging port ($what)" | Out-Null
  }
  $proc
}

Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public static class VoltipProbeWindows {
  delegate bool EnumProc(IntPtr hwnd, IntPtr lparam);
  [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc proc, IntPtr lparam);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetWindowTextW(IntPtr hwnd, StringBuilder text, int max);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassNameW(IntPtr hwnd, StringBuilder text, int max);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hwnd);
  static string Text(IntPtr hwnd) { var t = new StringBuilder(256); GetWindowTextW(hwnd, t, t.Capacity); return t.ToString(); }
  static string Class(IntPtr hwnd) { var t = new StringBuilder(256); GetClassNameW(hwnd, t, t.Capacity); return t.ToString(); }
  // The top-level window of `pid` titled exactly `title`, visible or not; IntPtr.Zero when none.
  public static IntPtr Find(uint pid, string title) {
    IntPtr found = IntPtr.Zero;
    EnumWindows((hwnd, _) => {
      uint owner;
      GetWindowThreadProcessId(hwnd, out owner);
      if (owner != pid || Text(hwnd) != title) return true;
      found = hwnd;
      return false;
    }, IntPtr.Zero);
    return found;
  }
  // The visible top-level windows of `pid`, as "class 'title'".
  public static string[] Visible(uint pid) {
    var all = new List<string>();
    EnumWindows((hwnd, _) => {
      uint owner;
      GetWindowThreadProcessId(hwnd, out owner);
      if (owner == pid && IsWindowVisible(hwnd)) all.Add(Class(hwnd) + " '" + Text(hwnd) + "'");
      return true;
    }, IntPtr.Zero);
    return all.ToArray();
  }
}
'@

# Whether Voltip's main window (titled Voltip; the pill is Voltip Overlay) is shown. WebView2 reports
# its page visible in a hidden window too, so this is read from Windows.
function Main-Shown([System.Diagnostics.Process] $proc) {
  $hwnd = [VoltipProbeWindows]::Find([uint32]$proc.Id, 'Voltip')
  $hwnd -ne [IntPtr]::Zero -and [VoltipProbeWindows]::IsWindowVisible($hwnd)
}

# A second launch hands over to the running Voltip and shows its main window (`on_second_instance`).
function Show-Main([System.Diagnostics.Process] $running, [string] $what) {
  $second = Start-Process -FilePath $exePath -PassThru
  if (-not $second.WaitForExit($TimeoutSec * 1000)) { $second.Kill($true); throw "${what}: the second launch did not hand over within ${TimeoutSec}s" }
  if ($running.HasExited) { throw "${what}: Voltip exited on the second launch" }
  Wait-Until { Main-Shown $running } $TimeoutSec "the main window to show ($what)" | Out-Null
}

# `$guard`: run the interface check (step 6); judged only for this build, a control for 0.2.1.
# `$window`: 'shown' or 'hidden', what Voltip's main window must be; '' leaves it unjudged.
function Probe([string] $stage, [int] $round, [System.Diagnostics.Process] $proc, [bool] $judge, [bool] $guard = $false, [string] $window = '') {
  $entries = Embedded-Entries $exePath
  $builds = Embedded-Builds $exePath
  $shown = Main-Shown $proc
  $out = & node $probeScript --port $Port --wait-ms ($TimeoutSec * 1000) --relay-connected-ms ($RelayConnectedSec * 1000) `
    --guard-check $(if ($guard) { '1' } else { '0' })
  if ($LASTEXITCODE -ne 0) { throw "webview-probe failed ($stage)" }
  $seen = $out | ConvertFrom-Json -Depth 20
  $started = [DateTimeOffset]::new($proc.StartTime.ToUniversalTime()).ToUnixTimeMilliseconds()
  $owner = Port-Owner
  $record = [ordered]@{
    round = $round; stage = $stage; pid = $proc.Id; startedMs = $started
    exe = (Get-FileHash -LiteralPath $exePath -Algorithm SHA256).Hash.ToLowerInvariant(); embedded = $entries; embeddedBuilds = $builds
    portOwner = $owner; webviews = Describe-WebViews; connections = Connections (@($proc.Id)); windows = $seen.windows
    guard = Field $seen 'guard'; mainWindowShown = $shown; visibleWindows = @([VoltipProbeWindows]::Visible([uint32]$proc.Id))
  }
  $records.Add($record)
  $mainSeen = $false
  foreach ($w in $seen.windows) {
    $label = Field $w 'label'
    if (-not $label) { $label = Field $w 'targetUrl' }
    $problem = Field $w 'error'
    if ($problem) {
      Note "round $round $stage ${label}: not read ($problem)"
      if ($judge) { $failures.Add("round $round ${stage}: the $label window could not be read ($problem)") }
      continue
    }
    $entry = @(Field $w 'entry')
    $carried = $entry.Count -gt 0 -and @($entry | Where-Object { $entries -contains $_ }).Count -eq $entry.Count
    $origin = Field $w 'timeOrigin'
    $older = $null -ne $origin -and [long]$origin -lt $started
    $line = "round $round $stage ${label}: entry $($entry -join ' ') " +
      "($(if ($carried) { 'carried by the program' } else { 'NOT carried by the program, which carries ' + ($entries -join ' ') })); " +
      "document $(if ($older) { 'OLDER than' } else { 'newer than' }) the process ($([long]$origin - $started) ms); " +
      "navigation $(Field $w 'navigation.type')/$(Field $w 'navigation.deliveryType') $(Field $w 'navigation.transferSize') B; rendered $(Field $w 'rendered'); $(Field $w 'visibility'); " +
      "build $(if (Field $w 'build') { Field $w 'build' } else { 'none' })$(if ($builds.Count -gt 0) { if ($builds -contains (Field $w 'build')) { ' (the program''s)' } else { ' (NOT the program''s)' } })"
    $main = $null -ne (Field $w 'dictionary')
    if ($main) {
      $mainSeen = $true
      $line += "; app $(Field $w 'core.appVersion') (core relay $(Field $w 'core.relay')$(if (Field $w 'core.error') { ', core_state failed: ' + (Field $w 'core.error') })); " +
        "relay card '$(Field $w 'relayCard.text')', connected after $(Field $w 'relayCard.connectedMs') ms; " +
        "dictionary page $(Field $w 'dictionary.page'), tabs [$(@(Field $w 'dictionary.tabs') -join ', ')], 学习建议 tab $(Field $w 'dictionary.suggestionsTab')"
    }
    Note $line
    if (-not $judge) { continue }
    if (-not $carried) { $failures.Add("round $round ${stage}: the $label window runs $($entry -join ' '), which the installed program does not carry") }
    if ($older) { $failures.Add("round $round ${stage}: the $label window's document is older than its process") }
    $build = Field $w 'build'
    if ($builds.Count -gt 0 -and $builds -notcontains $build) { $failures.Add("round $round ${stage}: the $label window runs build '$build', the program carries $($builds -join ' ')") }
    if (Field $w 'reloadMarker') { $failures.Add("round $round ${stage}: the $label window's address still carries the interface check's marker") }
    if ($main) {
      if (-not (Field $w 'dictionary.suggestionsTab')) { $failures.Add("round $round ${stage}: the dictionary page has no 学习建议 tab") }
      if ($null -eq (Field $w 'relayCard.connectedMs')) { $failures.Add("round $round ${stage}: the relay card reads '$(Field $w 'relayCard.text')' after ${RelayConnectedSec}s (the core says $(Field $w 'core.relay'))") }
    }
  }
  if ($judge -and -not $mainSeen) { $failures.Add("round $round ${stage}: no main window was read") }
  # This build's bundle always declares its build; none found means the windows' builds went unchecked.
  if ($judge -and $builds.Count -eq 0) { $failures.Add("round $round ${stage}: the installed program carries no interface build (apps/desktop/vite.config.ts)") }
  if ($judge -and $window -eq 'shown' -and -not $shown) { $failures.Add("round $round ${stage}: Voltip shows no window") }
  if ($judge -and $window -eq 'hidden' -and $shown) { $failures.Add("round $round ${stage}: Voltip shows a window after a start with --start-hidden") }
  if ($guard) { Judge-Guard (Field $seen 'guard') $stage $round $judge $entries $builds }
  $count = Connections (@($proc.Id))
  Note ("round $round ${stage}: pid $($proc.Id); main window $(if ($shown) { 'shown' } else { 'hidden' }) (visible: $(@([VoltipProbeWindows]::Visible([uint32]$proc.Id)) -join ', ')); debugging port $(if ($owner) { "pid $($owner.pid) <- $($owner.parent)" } else { 'none' }); webviews " +
    ((Describe-WebViews | ForEach-Object { "$($_.type)#$($_.pid)<-$($_.parent)" }) -join ', ') + "; connections $($count.local) local, $($count.elsewhere) elsewhere")
}

# The interface check on the running program (step 6): the documents each simulation took the main
# window through, and where it ended.
function Judge-Guard($g, [string] $stage, [int] $round, [bool] $judge, [string[]] $entries, [string[]] $builds) {
  $problem = Field $g 'error'
  if ($problem) {
    Note "round $round ${stage}: interface check not run ($problem)"
    if ($judge) { $failures.Add("round $round ${stage}: the interface check could not be run ($problem)") }
    return
  }
  $parts = foreach ($mode in 'stale-once', 'build-once', 'stale-always', 'home') {
    "$mode -> $(Field $g "$mode.documents") document(s), at $(Field $g "$mode.href"), entry $(@(Field $g "$mode.entry") -join ' '), build $(Field $g "$mode.build"), console [$(@(Field $g "$mode.warnings") -join ' | ')]"
  }
  Note "round $round ${stage}: interface check: $($parts -join '; ')"
  if (-not $judge) { return }
  $own = {
    param($mode)
    $entry = @(Field $g "$mode.entry")
    $entry.Count -gt 0 -and @($entry | Where-Object { $entries -notcontains $_ }).Count -eq 0 -and
      ($builds.Count -eq 0 -or $builds -contains (Field $g "$mode.build")) -and "$(Field $g "$mode.href")" -notmatch 'voltip-ui-reload'
  }
  # Replaced once by the program's document, which the window then keeps.
  foreach ($mode in 'stale-once', 'build-once') {
    $docs = Field $g "$mode.documents"
    if ($docs -ne 2) { $failures.Add("round $round ${stage}: interface check ${mode}: $docs document(s), not 2 (the stale one, then the program's)") }
    if (-not (& $own $mode)) { $failures.Add("round $round ${stage}: interface check ${mode}: the window ended on $(Field $g "$mode.href") running $(@(Field $g "$mode.entry") -join ' ') build $(Field $g "$mode.build")") }
  }
  # Still other code after the replacement: cleared, not kept.
  $docs = Field $g 'stale-always.documents'
  if ($docs -ne 3) { $failures.Add("round $round ${stage}: interface check stale-always: $docs document(s), not 3 (stale, stale again, cleared)") }
  if ((Field $g 'stale-always.href') -ne 'about:blank') { $failures.Add("round $round ${stage}: interface check stale-always: the window ended on $(Field $g 'stale-always.href'), not cleared") }
  $docs = Field $g 'home.documents'
  if ($docs -ne 1 -or -not (& $own 'home')) { $failures.Add("round $round ${stage}: interface check: back home the window went through $docs document(s) and runs $(@(Field $g 'home.entry') -join ' ')") }
}

$relayProc = $null
$firewall = $null
$env:RUST_LOG = 'voltip=info'
$env:NO_COLOR = '1'
try {
  Note "old setup $OldSetup ($((Get-FileHash -LiteralPath $OldSetup -Algorithm SHA256).Hash.ToLowerInvariant()))"
  Note "new setup $NewSetup ($((Get-FileHash -LiteralPath $NewSetup -Algorithm SHA256).Hash.ToLowerInvariant()))"
  $wv2 = 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}'
  Note "WebView2 runtime $((Get-ItemProperty -Path $wv2 -Name pv -ErrorAction SilentlyContinue).pv)"

  # WebView2 appends this to every Voltip start's browser arguments, the installer's restart too.
  New-Item -Path $policyKey -Force | Out-Null
  New-ItemProperty -Path $policyKey -Name 'voltip-desktop.exe' -Value "--remote-debugging-port=$Port --remote-allow-origins=*" -PropertyType String -Force | Out-Null

  # Off unless this machine runs a firewall; the rule blocks everything but this machine.
  $firewall = @(Get-NetFirewallProfile | Select-Object Name, Enabled)
  Note "firewall profiles: $(($firewall | ForEach-Object { "$($_.Name)=$($_.Enabled)" }) -join ', ')"
  Set-NetFirewallProfile -Profile Domain, Private, Public -Enabled True -DefaultOutboundAction Allow
  Remove-NetFirewallRule -DisplayName $ruleName -ErrorAction SilentlyContinue
  New-NetFirewallRule -DisplayName $ruleName -Direction Outbound -Action Block -Program $exePath -Profile Any `
    -RemoteAddress @('0.0.0.0-126.255.255.255', '128.0.0.0-255.255.255.255', '::2-ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff') | Out-Null

  $relayProc = Start-Process -FilePath $Relay -ArgumentList '--bind', "127.0.0.1:$RelayPort" -PassThru -RedirectStandardError (Join-Path $OutDir 'relay.log') -RedirectStandardOutput (Join-Path $OutDir 'relay.out')
  Wait-Until { (Test-NetConnection -ComputerName 127.0.0.1 -Port $RelayPort -WarningAction SilentlyContinue).TcpTestSucceeded } 60 'the relay' | Out-Null
  Note "relay pid $($relayProc.Id) on 127.0.0.1:$RelayPort"

  for ($round = 1; $round -le $Rounds; $round++) {
    $variant = $variants[($round - 1) % $variants.Count]
    $hidden = $variant -like '*hidden*'
    try {
      Note "round ${round}: the update the $variant way"
      Clear-Voltip
      Write-Settings
      Run-Setup $OldSetup @('/S') "round ${round}: the 0.2.1 installer"
      if (-not (Test-Path -LiteralPath $exePath)) { throw "round ${round}: 0.2.1 is not at $exePath" }

      # Started twice, as a copy in use has been: the second start reads what the first stored, and
      # is the one a hidden variant starts hidden (as at login).
      foreach ($start in 1, 2) {
        $log = Join-Path $OutDir "round$round-old-start$start"
        $arguments = @(if ($start -eq 2 -and $hidden) { '--start-hidden' })
        if ($arguments.Count -gt 0) {
          Start-Process -FilePath $exePath -ArgumentList $arguments -RedirectStandardError "$log.err" -RedirectStandardOutput "$log.out" | Out-Null
        } else {
          Start-Process -FilePath $exePath -RedirectStandardError "$log.err" -RedirectStandardOutput "$log.out" | Out-Null
        }
        $old = Wait-Voltip @() "0.2.1 start $start"
        if ($arguments.Count -eq 0) { Wait-Until { Main-Shown $old } $TimeoutSec "the 0.2.1 window to show (start $start)" | Out-Null }
        Probe "0.2.1 start $start" $round $old $false ($start -eq 1)
        if ($start -eq 1) {
          $quit = Start-Process -FilePath $exePath -ArgumentList '--quit' -PassThru
          [void]$quit.WaitForExit(30000)
          if (-not $old.WaitForExit(30000)) { Stop-Process -Id $old.Id -Force }
          Wait-Until { @(WebView-Processes).Count -eq 0 } 60 'the WebView2 processes after --quit' | Out-Null
        }
      }

      # The update, while 0.2.1 runs; the installer opens the new one (`/R`).
      $before = @(Voltip-Processes | ForEach-Object { $_.Id })
      Update-Voltip $variant $old "round ${round}: the update"
      $new = Wait-Voltip $before 'after the update'
      if ($hidden) {
        Probe 'after the update' $round $new $true $false 'hidden'
        Show-Main $new 'after the update'
        Probe 'after the update, shown' $round $new $true $false 'shown'
      } else {
        Wait-Until { Main-Shown $new } $TimeoutSec 'the window to show after the update' | Out-Null
        Probe 'after the update' $round $new $true $false 'shown'
      }

      # The same version over itself, the same way, then the interface check.
      $before = @(Voltip-Processes | ForEach-Object { $_.Id })
      Update-Voltip $variant $new "round ${round}: the same version again"
      $again = Wait-Voltip $before 'after the same version again'
      if ($hidden) {
        Probe 'after the same version again' $round $again $true $false 'hidden'
        Show-Main $again 'after the same version again'
        Probe 'after the same version again, shown' $round $again $true $true 'shown'
      } else {
        Wait-Until { Main-Shown $again } $TimeoutSec 'the window to show after the same version again' | Out-Null
        Probe 'after the same version again' $round $again $true $true 'shown'
      }
    } catch {
      $failures.Add("round ${round}: $_")
      Note "round ${round} stopped: $_"
    }
  }
} catch {
  $failures.Add("the probe stopped: $_")
} finally {
  try { Clear-Voltip } catch { $failures.Add("Voltip could not be removed afterwards: $_") }
  if ($null -ne $relayProc -and -not $relayProc.HasExited) { Stop-Process -Id $relayProc.Id -Force -ErrorAction SilentlyContinue }
  Remove-NetFirewallRule -DisplayName $ruleName -ErrorAction SilentlyContinue
  if ($null -ne $firewall) { foreach ($p in $firewall) { Set-NetFirewallProfile -Profile $p.Name -Enabled $p.Enabled } }
  Remove-ItemProperty -Path $policyKey -Name 'voltip-desktop.exe' -ErrorAction SilentlyContinue
  foreach ($failure in $failures) { Note "FAILED: $failure" }
  if ($failures.Count -eq 0) { Note 'OK' }
  $records | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath (Join-Path $OutDir 'rounds.json') -Encoding utf8
  $summary | Set-Content -LiteralPath (Join-Path $OutDir 'summary.txt') -Encoding utf8
}
if ($failures.Count -gt 0) { throw "windows-upgrade-probe: $($failures.Count) check(s) failed (see $OutDir/summary.txt)" }
