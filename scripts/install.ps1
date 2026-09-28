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
  Write-Host "voltip-install: $Message" -ForegroundColor Red
  throw "voltip-install: $Message"
}

function Say($Message) {
  Write-Host "voltip-install: $Message"
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
  $Release = Invoke-RestMethod -UseBasicParsing `
    -Uri "https://api.github.com/repos/$Repo/releases/latest" `
    -Headers @{ "User-Agent" = "voltip-install" }
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
  Say "downloading $Asset (Voltip $Version)"
  try {
    Invoke-WebRequest -UseBasicParsing -Uri "$BaseUrl/$ChecksumFile" -OutFile $Checksums
  } catch {
    Stop-Install "release v$Version has no $ChecksumFile (is $Version a Voltip release?)"
  }
  $Escaped = [Regex]::Escape($Asset)
  $Line = Get-Content $Checksums | Where-Object { $_ -match "^[0-9a-fA-F]{64}\s+\*?$Escaped$" } | Select-Object -First 1
  if (-not $Line) { Stop-Install "release v$Version has no $Asset" }
  Invoke-WebRequest -UseBasicParsing -Uri "$BaseUrl/$Asset" -OutFile $Installer

  $Expected = ($Line -split '\s+')[0].ToLowerInvariant()
  $Actual = (Get-FileHash -Algorithm SHA256 -Path $Installer).Hash.ToLowerInvariant()
  if ($Actual -ne $Expected) { Stop-Install "checksum mismatch for ${Asset}: nothing was installed" }
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
