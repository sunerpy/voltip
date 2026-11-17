# One-line install of Voltip on Windows (x64) from a GitHub release (Linux and macOS:
# scripts/install.sh):
#
#   irm https://raw.githubusercontent.com/sunerpy/voltip/main/scripts/install.ps1 | iex
#
# It downloads the per-user installer and SHA256SUMS from the same release, runs nothing unless
# the installer's SHA-256 matches its line there, installs silently for the current user (no
# administrator prompt) and starts Voltip.
#
#   $env:VOLTIP_VERSION = "0.0.4"    a given release instead of the latest
#
# Works in Windows PowerShell 5.1 (what `irm | iex` runs in by default) and PowerShell 7.
$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

$Repo = "sunerpy/voltip"
$ChecksumFile = "SHA256SUMS"

function Stop-Install($Message) {
  throw "voltip-install: $Message"
}

function Say($Message) {
  Write-Host "voltip-install: $Message"
}

# One GET with the parameters Windows PowerShell 5.1 and PowerShell 7 share, up to three times. An
# HTTP 4xx other than 408 and 429 is final; anything else (a 5xx, a timeout, a refused, dropped or
# failed TLS connection) is tried again. A failed request does not mean that the release lacks the
# file, so the last error names the URL and keeps every inner message. -TimeoutSec bounds only the
# wait for the response headers. A download is never cut off for being slow, only when its data
# stops arriving: for 30 s where the cmdlet has -OperationTimeoutSeconds (PowerShell 7.4 and
# later); Windows PowerShell 5.1 gives up on a read after 300 s by itself.
function Invoke-InstallRequest([string]$Uri, [string]$OutFile) {
  $Request = @{
    Uri = $Uri
    UseBasicParsing = $true
    TimeoutSec = 30
    Headers = @{ "User-Agent" = "voltip-install" }
  }
  if ((Get-Command Microsoft.PowerShell.Utility\Invoke-WebRequest).Parameters.ContainsKey('OperationTimeoutSeconds')) {
    $Request.OperationTimeoutSeconds = 30
  }
  for ($Attempt = 1; $Attempt -le 3; $Attempt++) {
    try {
      if ($OutFile) { Invoke-WebRequest @Request -OutFile $OutFile } else { Invoke-RestMethod @Request }
      return
    } catch {
      $Status = $null
      $Response = $_.Exception.PSObject.Properties['Response']
      if ($Response -and $Response.Value) { $Status = [int]$Response.Value.StatusCode }
      $Detail = $_.Exception.Message
      for ($Inner = $_.Exception.InnerException; $Inner; $Inner = $Inner.InnerException) { $Detail += " --> " + $Inner.Message }
      if ($Status) { $Detail = "HTTP ${Status}: $Detail" }
      $Final = $Status -ge 400 -and $Status -lt 500 -and $Status -notin @(408, 429)
      if ($Final -or $Attempt -eq 3) { Stop-Install "could not download ${Uri}: $Detail" }
      Say "could not download ${Uri} (attempt ${Attempt}/3): $Detail; retrying in 1s"
      Start-Sleep -Seconds 1
    }
  }
}

# Windows PowerShell 5.1 on an older Windows 10 may still offer TLS 1.0 only; GitHub needs 1.2.
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

# Voltip ships an x64 build; an ARM64 Windows runs it under emulation.
$Machine = if ($env:PROCESSOR_ARCHITEW6432) { $env:PROCESSOR_ARCHITEW6432 } else { $env:PROCESSOR_ARCHITECTURE }
if ($Machine -ne "AMD64" -and $Machine -ne "ARM64") {
  Stop-Install "Voltip ships for 64-bit Windows only (this is $Machine)"
}

if ($env:VOLTIP_VERSION) {
  $Version = $env:VOLTIP_VERSION -replace '^v', ''
} else {
  Say "finding the latest release"
  $Release = Invoke-InstallRequest -Uri "https://api.github.com/repos/$Repo/releases/latest"
  $Version = $Release.tag_name -replace '^v', ''
}
# The version goes into URLs and file names: digits and dots, and an optional pre-release tail.
if ($Version -notmatch '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$') {
  Stop-Install "not a release version: '$Version'"
}

$Asset = "Voltip_${Version}_x64-setup.exe"
$BaseUrl = "https://github.com/$Repo/releases/download/v$Version"
$TempDir = New-Item -ItemType Directory -Path (Join-Path $env:TEMP ("voltip-" + [System.Guid]::NewGuid()))

try {
  $Installer = Join-Path $TempDir $Asset
  $Checksums = Join-Path $TempDir $ChecksumFile
  Say "downloading $ChecksumFile (Voltip $Version)"
  Invoke-InstallRequest -Uri "$BaseUrl/$ChecksumFile" -OutFile $Checksums
  $Escaped = [Regex]::Escape($Asset)
  $Line = Get-Content $Checksums | Where-Object { $_ -match "^[0-9a-fA-F]{64}\s+\*?$Escaped$" } | Select-Object -First 1
  if (-not $Line) { Stop-Install "$ChecksumFile has no checksum for $Asset in release v$Version" }
  Say "downloading $Asset (Voltip $Version)"
  Invoke-InstallRequest -Uri "$BaseUrl/$Asset" -OutFile $Installer

  $Expected = ($Line -split '\s+')[0].ToLowerInvariant()
  $Actual = (Get-FileHash -Algorithm SHA256 -Path $Installer).Hash.ToLowerInvariant()
  # Windows PowerShell 5.1 can end a download whose connection closes early as if it were complete;
  # the size downloaded tells that apart from a changed file.
  if ($Actual -ne $Expected) {
    Stop-Install "checksum mismatch for $Asset ($((Get-Item -LiteralPath $Installer).Length) bytes downloaded): nothing was installed"
  }
  Say "SHA-256 matches $ChecksumFile"

  # A running Voltip holds its files; the installer would stop at it.
  $Running = Get-Process -Name "voltip-desktop" -ErrorAction SilentlyContinue
  if ($Running) {
    Say "closing the running Voltip"
    $Running | Stop-Process -Force
    $Running | Wait-Process -Timeout 15 -ErrorAction SilentlyContinue
  }

  Say "installing for the current user"
  $Setup = Start-Process -FilePath $Installer -ArgumentList "/S" -Wait -PassThru
  if ($Setup.ExitCode -ne 0) { Stop-Install "the installer exited with code $($Setup.ExitCode)" }

  # The per-user installer's directory, as the NSIS template names it; the uninstall entry says
  # where it went when that differs.
  $Candidates = @((Join-Path $env:LOCALAPPDATA "Voltip\voltip-desktop.exe"))
  $Uninstall = Get-ItemProperty "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*" -ErrorAction SilentlyContinue |
    Where-Object { $_.DisplayName -eq "Voltip" } | Select-Object -First 1
  if ($Uninstall -and $Uninstall.InstallLocation) {
    $Candidates = @((Join-Path ($Uninstall.InstallLocation.Trim('"')) "voltip-desktop.exe")) + $Candidates
  }
  $Exe = $Candidates | Where-Object { Test-Path $_ } | Select-Object -First 1
  if ($Exe) {
    Say "installed Voltip $Version; starting it"
    Start-Process -FilePath $Exe
  } else {
    Say "installed Voltip $Version; start it from the Start menu"
  }
} finally {
  Remove-Item -Recurse -Force $TempDir -ErrorAction SilentlyContinue
}
