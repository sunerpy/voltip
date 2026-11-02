#requires -Version 7
<#
.SYNOPSIS
  Update Voltip the way a user does and read which interface code each window runs afterwards.

.DESCRIPTION
  What it checks (docs/dictation.md §9.4; user report 2026-11-02: in the first run after
  installing 0.3.0 the dictionary page had no 学习建议 tab and the relay stayed at 认证中 until
  Voltip was restarted), on a desktop with WebView2 (CI `windows-native`, on demand), round after
  round:
    1. a clean start: no Voltip, no data directory, no WebView2 profile;
    2. the released Voltip (`-OldSetup`, 0.2.1) installed silently and started twice, its relay
       the build's own voltip-relay on this machine (`-Relay`);
    3. this build's installer (`-NewSetup`) run while it runs, as an update does: `/S /UPDATE /R`;
    4. once the installer has opened Voltip again, every window read over WebView2's debugging
       port (scripts/webview-probe.mjs): the entry script its document loaded, compared with the
       ones the installed program carries; the main window's dictionary page and its 学习建议 tab;
       the relay card reading connected;
    5. the same installer once more over itself (the same version) and step 4 again.
  A window whose entry script the installed program does not carry, a dictionary page without the
  tab, or a relay card that never reads connected fails the run; every observation goes to
  <OutDir>/summary.txt and <OutDir>/rounds.json.

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
  [int] $Rounds = 3,
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
$policyKey = 'HKCU:\Software\Policies\Microsoft\Edge\WebView2\AdditionalBrowserArguments'
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
    relay_url = "ws://127.0.0.1:$RelayPort"; relay_enabled = $true; auto_update = $false
  }
  $settings | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $dataDir 'settings.json') -Encoding utf8NoBOM
}

function Run-Setup([string] $setup, [string[]] $arguments, [string] $what) {
  $p = Start-Process -FilePath $setup -ArgumentList $arguments -PassThru
  $null = $p.Handle
  if (-not $p.WaitForExit($TimeoutSec * 1000)) { $p.Kill($true); throw "$what did not finish within ${TimeoutSec}s" }
  if ($p.ExitCode -ne 0) { throw "$what exited with $($p.ExitCode)" }
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
    Wait-Until { @(Pages).Count -ge 2 } $TimeoutSec "two windows on the debugging port ($what)" | Out-Null
  }
  $proc
}

function Probe([string] $stage, [int] $round, [System.Diagnostics.Process] $proc, [bool] $judge) {
  $entries = Embedded-Entries $exePath
  $out = & node $probeScript --port $Port --wait-ms ($TimeoutSec * 1000) --relay-connected-ms ($RelayConnectedSec * 1000)
  if ($LASTEXITCODE -ne 0) { throw "webview-probe failed ($stage)" }
  $seen = $out | ConvertFrom-Json -Depth 20
  $started = [DateTimeOffset]::new($proc.StartTime.ToUniversalTime()).ToUnixTimeMilliseconds()
  $owner = Port-Owner
  $record = [ordered]@{
    round = $round; stage = $stage; pid = $proc.Id; startedMs = $started
    exe = (Get-FileHash -LiteralPath $exePath -Algorithm SHA256).Hash.ToLowerInvariant(); embedded = $entries
    portOwner = $owner; webviews = Describe-WebViews; connections = Connections (@($proc.Id)); windows = $seen.windows
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
      "navigation $(Field $w 'navigation.type')/$(Field $w 'navigation.deliveryType') $(Field $w 'navigation.transferSize') B; rendered $(Field $w 'rendered')"
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
    if ($main) {
      if (-not (Field $w 'dictionary.suggestionsTab')) { $failures.Add("round $round ${stage}: the dictionary page has no 学习建议 tab") }
      if ($null -eq (Field $w 'relayCard.connectedMs')) { $failures.Add("round $round ${stage}: the relay card reads '$(Field $w 'relayCard.text')' after ${RelayConnectedSec}s (the core says $(Field $w 'core.relay'))") }
    }
  }
  if ($judge -and -not $mainSeen) { $failures.Add("round $round ${stage}: no main window was read") }
  $count = Connections (@($proc.Id))
  Note ("round $round ${stage}: pid $($proc.Id); debugging port $(if ($owner) { "pid $($owner.pid) <- $($owner.parent)" } else { 'none' }); webviews " +
    ((Describe-WebViews | ForEach-Object { "$($_.type)#$($_.pid)<-$($_.parent)" }) -join ', ') + "; connections $($count.local) local, $($count.elsewhere) elsewhere")
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
    try {
      Clear-Voltip
      Write-Settings
      Run-Setup $OldSetup @('/S') "round ${round}: the 0.2.1 installer"
      if (-not (Test-Path -LiteralPath $exePath)) { throw "round ${round}: 0.2.1 is not at $exePath" }

      # Started twice, as a copy in use has been: the second start reads what the first stored.
      foreach ($start in 1, 2) {
        $log = Join-Path $OutDir "round$round-old-start$start"
        Start-Process -FilePath $exePath -RedirectStandardError "$log.err" -RedirectStandardOutput "$log.out" | Out-Null
        $old = Wait-Voltip @() "0.2.1 start $start"
        Probe "0.2.1 start $start" $round $old $false
        if ($start -eq 1) {
          $quit = Start-Process -FilePath $exePath -ArgumentList '--quit' -PassThru
          [void]$quit.WaitForExit(30000)
          if (-not $old.WaitForExit(30000)) { Stop-Process -Id $old.Id -Force }
          Wait-Until { @(WebView-Processes).Count -eq 0 } 60 'the WebView2 processes after --quit' | Out-Null
        }
      }

      # The update, while 0.2.1 runs: the installer ends it and opens the new one (`/R`).
      $before = @(Voltip-Processes | ForEach-Object { $_.Id })
      Run-Setup $NewSetup @('/S', '/UPDATE', '/R') "round ${round}: the update"
      $new = Wait-Voltip $before 'after the update'
      Probe 'after the update' $round $new $true

      # The same version over itself.
      $before = @(Voltip-Processes | ForEach-Object { $_.Id })
      Run-Setup $NewSetup @('/S', '/UPDATE', '/R') "round ${round}: the same version again"
      $again = Wait-Voltip $before 'after the same version again'
      Probe 'after the same version again' $round $again $true
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
