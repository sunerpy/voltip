#requires -Version 7
<#
.SYNOPSIS
  Run the shipped desktop binary's headless entry points on a real machine and record the result.

.DESCRIPTION
  What it proves (docs/dictation.md §13): the packaged binary starts on this OS (its DLLs / dylibs /
  shared objects resolve, the static CRT and the bundled sherpa-onnx runtime load), prepares a
  catalogue model with the app's own downloader (build mirror, huggingface.co, hf-mirror.com;
  sha256-verified) and transcribes a public Chinese sample with it. No window, microphone or
  network service of ours is involved beyond the model download.

  Steps, each with a timeout; the first failure stops the run with a non-zero exit:
    1. --version
       --list-compute             (the CPU threads, then every GPU the build's backend sees; a
                                   Vulkan build with no driver lists none and runs on the CPU)
    2. --list-models              (the catalogue rows for -Models are present)
    3. --download-model <id>       for each model in -Models
    4. fetch the sample WAV        (pinned Hugging Face revision + sha256; hf-mirror.com as fallback)
    5. --transcribe-file --json    for each model; the text must contain -Expect (the summary
                                   names the backend it ran on: CPU, Vulkan0, Metal, …)
    6. --list-models              (the models now installed)
  A summary (OS, CPU, binary sha256, every command line, exit code, stdout and the tail of stderr)
  is written to <OutDir>/summary.txt; stdout of every step is also kept in <OutDir>/*.out.

.PARAMETER Binary
  Path to voltip-desktop(.exe).
.PARAMETER Models
  Catalogue ids to download and run, as an array or one comma-separated string (default:
  sense-voice-small, the smallest offline model; CI adds qwen3-asr-0.6b, the default one).
.PARAMETER ModelCache
  A directory holding the model files of an earlier run, one subdirectory per catalogue id (CI keeps
  it in the Actions cache). Before the downloads they are copied into the library without their
  manifest, so the app's downloader checks each file's size and sha256 instead of fetching it; after
  a good run the verified files are copied back. A model the cache lacks, or a file that fails the
  check, is downloaded as before. huggingface.co and hf-mirror.com both unreachable for a minute
  then no longer fails the run (2026-10-06, main CI 37401697547).
.NOTES
  Data directory: on Linux and macOS the run is isolated (XDG_* / HOME point into <OutDir>). On
  Windows the app resolves its data directory through the Known Folder API (roaming AppData), which
  no environment variable redirects, so the models land in the real per-user library — fresh on a
  CI runner; on a workstation the "nothing installed yet" check is skipped when it is not.
#>
param(
  [Parameter(Mandatory = $true)] [string] $Binary,
  [string] $OutDir = 'smoke-native-cli',
  [string[]] $Models = @('sense-voice-small'),
  [string] $ModelCache = '',
  # Tried in order, like the app's own model sources (huggingface.co, then its mirror for networks
  # that cannot reach it); the sha256 pin makes the source irrelevant.
  [string[]] $SampleUrl = @(
    'https://huggingface.co/csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17/resolve/2365baeacb507f821a0c8120fcee3d484dba7a07/test_wavs/zh.wav',
    'https://hf-mirror.com/csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17/resolve/2365baeacb507f821a0c8120fcee3d484dba7a07/test_wavs/zh.wav'
  ),
  [string] $SampleSha256 = 'b77f1794fe374a0ba1ee1dc458bfaf9349496cbbfc32780c50ba3c5a7ad8e373',
  [string] $Expect = '早上',
  [int] $DownloadTimeoutSec = 1500,
  [int] $StepTimeoutSec = 300
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

# `-Models a,b` arrives as one string under `pwsh -File`; accept both forms (same for -SampleUrl).
$Models = @($Models | ForEach-Object { $_ -split ',' } | ForEach-Object { $_.Trim() } | Where-Object { $_ })
$SampleUrl = @($SampleUrl | ForEach-Object { $_ -split ',' } | ForEach-Object { $_.Trim() } | Where-Object { $_ })

$Binary = (Resolve-Path -LiteralPath $Binary).Path
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$OutDir = (Resolve-Path -LiteralPath $OutDir).Path
if ($ModelCache) {
  New-Item -ItemType Directory -Force -Path $ModelCache | Out-Null
  $ModelCache = (Resolve-Path -LiteralPath $ModelCache).Path
}
$summary = Join-Path $OutDir 'summary.txt'
$appData = Join-Path $OutDir 'appdata'
if (Test-Path $appData) { Remove-Item -Recurse -Force $appData }
New-Item -ItemType Directory -Force -Path $appData | Out-Null

# Every per-user directory the app could resolve points into $appData: Windows (APPDATA /
# LOCALAPPDATA), Linux (XDG_*), macOS (HOME). A fresh library, never the runner user's.
$env:APPDATA = $appData
$env:LOCALAPPDATA = $appData
$env:XDG_DATA_HOME = $appData
$env:XDG_CONFIG_HOME = $appData
if (-not $IsWindows) { $env:HOME = $appData }
$env:RUST_LOG = 'info'
$utf8 = [System.Text.UTF8Encoding]::new($false)
[Console]::OutputEncoding = $utf8

function Write-Summary([string] $text) {
  Add-Content -LiteralPath $summary -Value $text -Encoding utf8
  Write-Host $text
}

function Get-OsDescription {
  if ($IsWindows) {
    try {
      $os = Get-CimInstance Win32_OperatingSystem
      return "$($os.Caption) $($os.Version) build $($os.BuildNumber) ($([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture))"
    } catch { }
  }
  return "$([System.Runtime.InteropServices.RuntimeInformation]::OSDescription) ($([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture))"
}

function Get-CpuDescription {
  if ($IsWindows) {
    try { return (Get-CimInstance Win32_Processor | Select-Object -First 1).Name.Trim() } catch { }
  }
  if (Test-Path '/proc/cpuinfo') {
    $line = Select-String -Path '/proc/cpuinfo' -Pattern '^model name\s*:\s*(.+)$' | Select-Object -First 1
    if ($line) { return $line.Matches[0].Groups[1].Value.Trim() }
  }
  if ($IsMacOS) { return (& sysctl -n machdep.cpu.brand_string) }
  return 'unknown'
}

# Run the binary with $arguments; returns exit code, stdout, stderr. A timeout kills the process
# and fails the run (the deadlock this guards against hung forever).
function Invoke-Voltip([string] $step, [string[]] $arguments, [int] $timeoutSec) {
  $psi = [System.Diagnostics.ProcessStartInfo]::new($Binary)
  foreach ($a in $arguments) { $psi.ArgumentList.Add($a) }
  $psi.RedirectStandardOutput = $true
  $psi.RedirectStandardError = $true
  $psi.UseShellExecute = $false
  $psi.StandardOutputEncoding = $utf8
  $psi.StandardErrorEncoding = $utf8
  $started = [System.Diagnostics.Stopwatch]::StartNew()
  $process = [System.Diagnostics.Process]::Start($psi)
  $stdoutTask = $process.StandardOutput.ReadToEndAsync()
  $stderrTask = $process.StandardError.ReadToEndAsync()
  if (-not $process.WaitForExit($timeoutSec * 1000)) {
    try { $process.Kill($true) } catch { }
    Write-Summary "[$step] TIMEOUT after $timeoutSec s: voltip-desktop $($arguments -join ' ')"
    throw "step '$step' timed out"
  }
  $process.WaitForExit()
  $stdout = $stdoutTask.GetAwaiter().GetResult()
  $stderr = $stderrTask.GetAwaiter().GetResult()
  $elapsed = [math]::Round($started.Elapsed.TotalSeconds, 2)
  Set-Content -LiteralPath (Join-Path $OutDir "$step.out") -Value $stdout -Encoding utf8 -NoNewline
  Set-Content -LiteralPath (Join-Path $OutDir "$step.err") -Value $stderr -Encoding utf8 -NoNewline
  $tail = ($stderr -split "`n" | Where-Object { $_.Trim() } | Select-Object -Last 6) -join "`n      "
  Write-Summary "[$step] exit=$($process.ExitCode) ${elapsed}s: voltip-desktop $($arguments -join ' ')"
  Write-Summary "    stdout: $($stdout.TrimEnd() -replace "`n", "`n            ")"
  if ($tail) { Write-Summary "    stderr (tail): $tail" }
  return [pscustomobject]@{ Code = $process.ExitCode; Stdout = $stdout; Stderr = $stderr; Seconds = $elapsed }
}

# Where the app keeps its models: `ProjectDirs::from("dev", "voltip", "Voltip")`'s data directory
# (lib.rs `data_dir`) plus `models` (`CoreConfig::models_root`), under the per-user directories set
# above. Windows resolves roaming AppData through the Known Folder API, as the app does.
function Get-ModelsRoot {
  if ($IsWindows) { return Join-Path ([Environment]::GetFolderPath('ApplicationData')) 'voltip\Voltip\data\models' }
  if ($IsMacOS) { return Join-Path $env:HOME 'Library/Application Support/dev.voltip.Voltip/models' }
  return Join-Path $env:XDG_DATA_HOME 'voltip/models'
}

# Copy a model directory's files without its manifest (and without a `.part` left mid-download).
function Copy-ModelFiles([string] $from, [string] $to) {
  New-Item -ItemType Directory -Force -Path $to | Out-Null
  Copy-Item -Path (Join-Path $from '*') -Destination $to -Recurse -Force
  Get-ChildItem -LiteralPath $to -Recurse -File | Where-Object { $_.Name -eq 'manifest.json' -or $_.Name -like '*.part' } | Remove-Item -Force
}

function Assert-That([bool] $condition, [string] $message) {
  if (-not $condition) {
    Write-Summary "FAILED: $message"
    throw $message
  }
}

Set-Content -LiteralPath $summary -Value '' -Encoding utf8
$binarySha = (Get-FileHash -Algorithm SHA256 -LiteralPath $Binary).Hash.ToLowerInvariant()
Write-Summary "smoke-native-cli $(Get-Date -AsUTC -Format 'yyyy-MM-ddTHH:mm:ssZ')"
Write-Summary "os: $(Get-OsDescription)"
Write-Summary "cpu: $(Get-CpuDescription)"
Write-Summary "binary: $Binary"
Write-Summary "binary sha256: $binarySha"
Write-Summary ''

# 1. The binary starts at all (on Windows a missing DLL is exit 0xC0000135 before main).
$version = Invoke-Voltip 'version' @('--version') $StepTimeoutSec
Assert-That ($version.Code -eq 0 -and $version.Stdout -match '^voltip \d+\.\d+\.\d+') "--version did not print a version"
$compute = Invoke-Voltip 'list-compute' @('--list-compute') $StepTimeoutSec
Assert-That ($compute.Code -eq 0 -and $compute.Stdout -match '^cpu\t\d+') "--list-compute did not report the CPU"

# 2. A fresh library lists the catalogue with nothing installed.
$listed = Invoke-Voltip 'list-models-before' @('--list-models') $StepTimeoutSec
Assert-That ($listed.Code -eq 0) "--list-models failed"
$rows = @($listed.Stdout -split "`n" | Where-Object { $_.Trim() })
Assert-That ($rows.Count -ge 1) "--list-models printed nothing"
foreach ($m in $Models) {
  Assert-That (@($rows | Where-Object { ($_ -split "`t")[0] -eq $m }).Count -eq 1) "catalogue has no row for $m"
}

# 3. Models through the app's own downloader (resumes, verifies sha256, writes the manifest). Files
#    from -ModelCache go in first; the downloader checks them and fetches only what is missing.
$modelsRoot = Get-ModelsRoot
if ($ModelCache) {
  $seeded = @()
  foreach ($m in $Models) {
    $cached = Join-Path $ModelCache $m
    $target = Join-Path $modelsRoot $m
    if ((Test-Path -LiteralPath $cached) -and -not (Test-Path -LiteralPath (Join-Path $target 'manifest.json'))) {
      Copy-ModelFiles $cached $target
      $seeded += $m
    }
  }
  Write-Summary "model cache: $(if ($seeded) { "$($seeded -join ', ') from $ModelCache" } else { "nothing usable in $ModelCache" })"
}
$installed = @{}
foreach ($m in $Models) {
  $dl = Invoke-Voltip "download-$m" @('--download-model', $m) $DownloadTimeoutSec
  Assert-That ($dl.Code -eq 0) "--download-model $m failed"
  $fields = @($dl.Stdout.TrimEnd() -split "`t")
  Assert-That ($fields.Count -eq 3 -and $fields[0] -eq $m -and $fields[1] -eq 'installed') "--download-model $m printed '$($dl.Stdout.TrimEnd())'"
  $installed[$m] = $fields[2]
  if ($ModelCache -and [System.IO.Path]::GetFullPath($fields[2]).TrimEnd('/', '\') -ne [System.IO.Path]::GetFullPath((Join-Path $modelsRoot $m)).TrimEnd('/', '\')) {
    Write-Summary "model cache: the app installed $m in $($fields[2]), not under $modelsRoot; the cache cannot seed it"
  }
}

# 4. The public sample, pinned by revision and sha256: each source in turn, three attempts each.
$sample = Join-Path $OutDir 'zh.wav'
$sampleSource = 'already present'
if (-not (Test-Path -LiteralPath $sample) -or (Get-FileHash -Algorithm SHA256 -LiteralPath $sample).Hash.ToLowerInvariant() -ne $SampleSha256) {
  $sampleSource = $null
  foreach ($url in $SampleUrl) {
    for ($attempt = 1; $attempt -le 3 -and -not $sampleSource; $attempt++) {
      try { Invoke-WebRequest -Uri $url -OutFile $sample -TimeoutSec 60; $sampleSource = $url }
      catch {
        Write-Summary "    sample download from $url failed ($($_.Exception.Message)); attempt $attempt of 3"
        if ($attempt -lt 3) { Start-Sleep -Seconds (5 * $attempt) }
      }
    }
    if ($sampleSource) { break }
  }
  Assert-That ([bool]$sampleSource) "the sample could not be downloaded from $($SampleUrl -join ' or ')"
}
$sampleSha = (Get-FileHash -Algorithm SHA256 -LiteralPath $sample).Hash.ToLowerInvariant()
Assert-That ($sampleSha -eq $SampleSha256) "sample sha256 $sampleSha is not $SampleSha256"
Write-Summary "sample: $sampleSource (sha256 $sampleSha)"

# 5. Recognition through the shipped binary, one JSON line on stdout.
$results = @()
foreach ($m in $Models) {
  $run = Invoke-Voltip "transcribe-$m" @('--transcribe-file', $sample, '--model', $m, '--json') $StepTimeoutSec
  Assert-That ($run.Code -eq 0) "--transcribe-file with $m failed"
  $lines = @($run.Stdout -split "`n" | Where-Object { $_.Trim() })
  Assert-That ($lines.Count -eq 1) "--transcribe-file with $m printed $($lines.Count) lines on stdout"
  $parsed = $lines[0] | ConvertFrom-Json
  Assert-That ($parsed.model -eq $m) "--transcribe-file reported model $($parsed.model)"
  Assert-That ($parsed.text -like "*$Expect*") "text from $m does not contain '$Expect': $($parsed.text)"
  $results += "$m`t$($parsed.latency_ms) ms`t$($parsed.backend)`t$($parsed.text)"
}

# 6. The library now reports the models installed.
$after = Invoke-Voltip 'list-models-after' @('--list-models') $StepTimeoutSec
foreach ($m in $Models) {
  Assert-That (@($after.Stdout -split "`n" | Where-Object { $_ -match "^$([regex]::Escape($m))`tinstalled`t" }).Count -eq 1) "$m not listed as installed afterwards"
}

# The verified files go back to -ModelCache for the next run.
if ($ModelCache) {
  foreach ($m in $Models) {
    $dest = Join-Path $ModelCache $m
    if (Test-Path -LiteralPath $dest) { Remove-Item -Recurse -Force -LiteralPath $dest }
    Copy-ModelFiles $installed[$m] $dest
  }
  Write-Summary "model cache: $($Models -join ', ') kept in $ModelCache"
}

Write-Summary ''
Write-Summary 'RESULT: OK'
foreach ($r in $results) { Write-Summary "  $r" }
