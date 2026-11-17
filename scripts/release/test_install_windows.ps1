# Run with Windows PowerShell 5.1 or PowerShell 7:
#   powershell -NoProfile -File scripts/release/test_install_windows.ps1
# Exercises the whole `irm | iex` entry with fake HTTP and process ports; never downloads or
# executes a real installer. User report 2026-11-17: Win11 / v0.3.1 said "has no SHA256SUMS"
# even though the published file existed. A TLS/proxy error must retain its actual cause.
# Review 2026-11-17, with the real cmdlet in PowerShell 7.6.3: a connection that dropped after
# 1000 bytes ended the download at its first attempt (an AggregateException around "The response
# ended prematurely"), and a body that stopped after 1000 bytes held `Invoke-WebRequest
# -TimeoutSec 3` until the probe was killed at 280 s: -TimeoutSec bounds the response headers only.
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Net.Http
$InstallerScript = Get-Content -Raw (Join-Path $PSScriptRoot '../install.ps1')
$TestRoot = Join-Path ([IO.Path]::GetTempPath()) ('voltip-install-tests-' + [guid]::NewGuid())
$SavedEnvironment = @{}
foreach ($Name in @('TEMP', 'LOCALAPPDATA', 'PROCESSOR_ARCHITECTURE', 'PROCESSOR_ARCHITEW6432', 'VOLTIP_VERSION')) {
  $SavedEnvironment[$Name] = [Environment]::GetEnvironmentVariable($Name, 'Process')
}
$Payload = [Text.Encoding]::UTF8.GetBytes('a fake Voltip Windows installer')
$Hasher = [Security.Cryptography.SHA256]::Create()
try { $Digest = [BitConverter]::ToString($Hasher.ComputeHash($Payload)).Replace('-', '').ToLowerInvariant() }
finally { $Hasher.Dispose() }
# PowerShell 7.4 added -OperationTimeoutSeconds, which ends a download whose data stops arriving.
$Edition = $PSVersionTable.PSVersion
$ReadLimit = if ($Edition.Major -gt 7 -or ($Edition.Major -eq 7 -and $Edition.Minor -ge 4)) { 30 } else { $null }

function Assert-Install($Condition, $Message) {
  if (-not $Condition) { throw $Message }
}

function Invoke-FakeRequest($Uri, $OutFile, $TimeoutSec, $OperationTimeoutSeconds) {
  $Target = if ($Uri -like '*/latest') { 'latest' } elseif ($Uri -like '*/SHA256SUMS') { 'sums' } else { 'exe' }
  $script:Requests.Add([pscustomobject]@{ Target = $Target; Uri = $Uri; TimeoutSec = $TimeoutSec; ReadLimit = $OperationTimeoutSeconds })
  if ($Target -eq $script:Case.Target -and $script:Remaining -gt 0) {
    $script:Remaining--
    if ($OutFile) { [IO.File]::WriteAllText($OutFile, 'partial failed download') }
    switch ($script:Case.Failure) {
      'tls' { throw [Net.WebException]::new('Could not create SSL/TLS secure channel', [Net.WebExceptionStatus]::SecureChannelFailure) }
      'tls7' { throw [Net.Http.HttpRequestException]::new('The SSL connection could not be established', [Security.Authentication.AuthenticationException]::new('The remote certificate is invalid')) }
      'timeout' { throw [Net.WebException]::new('The operation has timed out', [Net.WebExceptionStatus]::Timeout) }
      'timeout7' { throw [Threading.Tasks.TaskCanceledException]::new('The request was canceled due to the configured timeout') }
      'connect7' { throw [Net.Http.HttpRequestException]::new('Connection refused') }
      'cut' { throw [AggregateException]::new([IO.IOException]::new('The response ended prematurely. (ResponseEnded)')) }
      default {
        # Both PowerShell editions expose HTTP status through Exception.Response.StatusCode.
        $Failure = [Exception]::new('mock HTTP failure ' + $script:Case.Failure)
        $Failure | Add-Member NoteProperty Response ([pscustomobject]@{ StatusCode = [int]$script:Case.Failure })
        throw $Failure
      }
    }
  }
  if ($Target -eq 'latest') { return [pscustomobject]@{ tag_name = 'v0.3.1' } }
  if ($Target -eq 'sums') {
    $Sum = if ($script:Case.Corrupt) { '0' * 64 } else { $Digest }
    $Asset = if ($script:Case.MissingEntry) { 'other-file.exe' } else { 'Voltip_0.3.1_x64-setup.exe' }
    [IO.File]::WriteAllText($OutFile, "$Sum  $Asset`n")
  } else {
    [IO.File]::WriteAllBytes($OutFile, $Payload)
  }
}

function Invoke-WebRequest {
  param($Uri, $OutFile, [switch]$UseBasicParsing, $TimeoutSec, $OperationTimeoutSeconds, $Headers)
  Assert-Install $UseBasicParsing 'web requests must keep -UseBasicParsing for PowerShell 5.1'
  Invoke-FakeRequest $Uri $OutFile $TimeoutSec $OperationTimeoutSeconds
}
function Invoke-RestMethod {
  param($Uri, [switch]$UseBasicParsing, $TimeoutSec, $OperationTimeoutSeconds, $Headers)
  Assert-Install $UseBasicParsing 'release requests must keep -UseBasicParsing'
  Invoke-FakeRequest $Uri '' $TimeoutSec $OperationTimeoutSeconds
}
function Start-Sleep { param($Seconds) $script:Waits.Add($Seconds) }
function Get-Process { param($Name, $ErrorAction) return $null }
function Get-ItemProperty { param($Path, $ErrorAction) return $null }
function Start-Process {
  param($FilePath, $ArgumentList, [switch]$Wait, [switch]$PassThru)
  Assert-Install ($ArgumentList -eq '/S' -and $Wait -and $PassThru) 'only a silent fake install is allowed'
  Assert-Install ((Get-FileHash -Algorithm SHA256 $FilePath).Hash.ToLowerInvariant() -eq $Digest) 'unverified installer reached Start-Process'
  $script:Installed++
  return [pscustomobject]@{ ExitCode = 0 }
}

# A 4xx answer other than 408 or 429 is final; every other failure is tried up to three times.
$Cases = @(
  @{ Name = 'regression: a TLS failure keeps its cause and URL, and is tried again'; Target = 'sums'; Failure = 'tls'; Failures = 3; Attempts = 3; Error = 'SSL/TLS secure channel' },
  @{ Name = 'regression: a nested PowerShell 7 failure keeps every inner message'; Target = 'sums'; Failure = 'tls7'; Failures = 3; Attempts = 3; Error = 'remote certificate is invalid' },
  @{ Name = 'regression: a missing checksum file (404) is not tried again'; Target = 'sums'; Failure = '404'; Failures = 3; Attempts = 1; Error = '404' },
  @{ Name = 'regression: a proxy refusal (407) is not called a missing checksum'; Target = 'sums'; Failure = '407'; Failures = 3; Attempts = 1; Error = '407' },
  @{ Name = 'regression: a rate limit (429) is tried again'; Target = 'sums'; Failure = '429'; Failures = 1; Attempts = 2; Install = 1 },
  @{ Name = 'regression: a server error (503) is tried again'; Target = 'sums'; Failure = '503'; Failures = 1; Attempts = 2; Install = 1 },
  @{ Name = 'regression: a checksum timeout is tried again'; Target = 'sums'; Failure = 'timeout'; Failures = 1; Attempts = 2; Install = 1 },
  @{ Name = 'regression: an installer download cut off midway is tried again'; Target = 'exe'; Failure = 'cut'; Failures = 1; Attempts = 2; Install = 1 },
  @{ Name = 'regression: the latest lookup stops after three attempts'; Target = 'latest'; Failure = 'timeout7'; Failures = 3; Attempts = 3; Latest = $true; Error = 'configured timeout' },
  @{ Name = 'regression: checksum retry exhaustion never downloads the installer'; Target = 'sums'; Failure = 'connect7'; Failures = 3; Attempts = 3; Error = 'Connection refused' },
  @{ Name = 'regression: installer retry exhaustion installs nothing'; Target = 'exe'; Failure = 'cut'; Failures = 3; Attempts = 3; Error = 'ended prematurely' },
  @{ Name = 'regression: missing checksum entry installs nothing'; Target = 'none'; MissingEntry = $true; Error = 'has no checksum for Voltip_0.3.1_x64-setup.exe' },
  # A Windows Server 2025 machine, 2026-11-17: Windows PowerShell 5.1.26100 ended a download whose
  # connection closed after 1000 of 100000 bytes as if it were complete; only the checksum caught it.
  @{ Name = 'regression: a mismatched checksum names the size downloaded and installs nothing'; Target = 'none'; Corrupt = $true; Error = 'checksum mismatch for Voltip_0.3.1_x64-setup.exe (31 bytes downloaded)' },
  @{ Name = 'regression: latest release installs only after verification'; Target = 'none'; Latest = $true; Install = 1 }
)
$Failures = 0
try {
  New-Item -ItemType Directory -Path $TestRoot | Out-Null
  foreach ($script:Case in $Cases) {
    $CaseDir = Join-Path $TestRoot ([guid]::NewGuid().ToString() + ' with spaces')
    New-Item -ItemType Directory -Path $CaseDir | Out-Null
    $env:TEMP = $CaseDir
    $env:LOCALAPPDATA = $CaseDir
    $env:PROCESSOR_ARCHITECTURE = 'AMD64'
    $env:PROCESSOR_ARCHITEW6432 = ''
    $env:VOLTIP_VERSION = if ($Case.Latest) { '' } else { '0.3.1' }
    $script:Remaining = [int]$Case.Failures
    $script:Installed = 0
    $script:Requests = [Collections.Generic.List[object]]::new()
    $script:Waits = [Collections.Generic.List[int]]::new()
    $script:InstallFailure = $null
    # Match the README entry, isolate its variables, and capture its host output as well as the
    # final exception so a double error line cannot hide in a passing assertion.
    $Messages = & {
      try { Invoke-Expression $InstallerScript } catch { $script:InstallFailure = $_ }
    } 6>&1
    $Log = ($Messages | Out-String)
    try {
      if ($Case.Error) {
        Assert-Install ($null -ne $InstallFailure) 'installation unexpectedly succeeded'
        $ErrorText = $InstallFailure.Exception.Message
        Assert-Install ($ErrorText.Contains($Case.Error)) "lost original cause: $ErrorText"
        Assert-Install (-not $Log.Contains($ErrorText)) 'the final error was printed twice'
        Assert-Install (-not $ErrorText.Contains('has no SHA256SUMS')) "download failure misreported: $ErrorText"
        if ($Case.Target -ne 'none') {
          $FailedRequest = @($Requests | Where-Object { $_.Target -eq $Case.Target })[-1]
          Assert-Install ($ErrorText.Contains($FailedRequest.Uri)) "missing download URL: $ErrorText"
        }
      } else {
        Assert-Install ($null -eq $InstallFailure) "installation failed: $InstallFailure"
        Assert-Install ($Log.Contains('SHA-256 matches SHA256SUMS')) 'verification did not finish'
      }
      Assert-Install ($Installed -eq [int]$Case.Install) "installer calls: $Installed"
      if ($Case.Attempts) {
        $Attempts = @($Requests | Where-Object { $_.Target -eq $Case.Target }).Count
        Assert-Install ($Attempts -eq $Case.Attempts) "expected $($Case.Attempts) attempts, got $Attempts"
        Assert-Install ($Waits.Count -eq ($Case.Attempts - 1)) 'unexpected retry waits'
      }
      # Every request waits 30 s for its response headers; where the cmdlet can (7.4 and later),
      # a download also ends when its data stops arriving for 30 s. A slow one is never cut off.
      foreach ($Request in $Requests) {
        Assert-Install ($Request.TimeoutSec -eq 30) "unexpected header timeout for $($Request.Target): $($Request.TimeoutSec)"
        Assert-Install ($Request.ReadLimit -eq $ReadLimit) "unexpected read timeout for $($Request.Target): $($Request.ReadLimit)"
      }
      if ($Case.Error -and $Case.Target -ne 'exe' -and -not $Case.Corrupt) {
        Assert-Install (@($Requests | Where-Object { $_.Target -eq 'exe' }).Count -eq 0) 'installer downloaded after checksum failure'
      }
      Assert-Install (@(Get-ChildItem -LiteralPath $CaseDir -Force).Count -eq 0) 'temporary download files were not cleaned'
      Write-Output "PASS: $($Case.Name)"
    } catch {
      $Failures++
      Write-Output "FAIL: $($Case.Name): $($_.Exception.Message)"
    }
  }
} finally {
  foreach ($Name in $SavedEnvironment.Keys) {
    [Environment]::SetEnvironmentVariable($Name, $SavedEnvironment[$Name], 'Process')
  }
  Remove-Item -LiteralPath $TestRoot -Recurse -Force -ErrorAction SilentlyContinue
}
if ($Failures) { throw "$Failures installer regression test(s) failed" }
Write-Output "$($Cases.Count) installer regression tests passed (PowerShell $($PSVersionTable.PSVersion))"
