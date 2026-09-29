<#
.SYNOPSIS
  Paste a history entry into Notepad through the real window: the home page's
  "Paste into the previous window" button.

.DESCRIPTION
  What it proves (the history paste, apps/desktop/src-tauri/src/paste.rs), on a machine with an
  interactive desktop (CI `windows-native`: GitHub's windows-2025 image has one):
    1. with one entry seeded in the history, the home page's recent table offers the button;
    2. pressing it (UI Automation Invoke) moves Voltip out of the way, Notepad comes back to the
       front and receives the entry's text, CJK included (read back from its edit control). The
       script never activates Notepad itself (a background script may not): Voltip brings back the
       window below its own when Windows leaves nothing in front;
    3. Voltip stays minimised after a paste that landed (it comes back only when it did not).
  The entry goes into the runner user's real history file (Windows resolves the data directory
  through the Known Folder API, which no environment variable redirects); a history file that is
  already there is set aside first and put back at the end. Each step has a timeout and the first
  failure stops the run. <OutDir> receives summary.txt and app.log.
  Windows PowerShell 5.1 reads a BOM-less script as ANSI, so every non-ASCII string below is built
  from code points.

.PARAMETER Binary
  Path to voltip-desktop.exe.
#>
param(
  [Parameter(Mandatory = $true)] [string] $Binary,
  [string] $OutDir = 'smoke-windows-paste',
  [int] $WindowTimeoutSec = 120,
  [int] $StepTimeoutSec = 30
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$Binary = (Resolve-Path -LiteralPath $Binary).Path
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$OutDir = (Resolve-Path -LiteralPath $OutDir).Path
$log = Join-Path $OutDir 'app.log'
$summary = New-Object System.Collections.Generic.List[string]
function Note([string] $line) { $summary.Add($line); Write-Host "smoke-windows-paste: $line" }

Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class VoltipPaste {
  delegate bool EnumProc(IntPtr hwnd, IntPtr lparam);
  [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc proc, IntPtr lparam);
  [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr parent, EnumProc proc, IntPtr lparam);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetWindowTextW(IntPtr hwnd, StringBuilder text, int max);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassNameW(IntPtr hwnd, StringBuilder name, int max);
  [DllImport("user32.dll", CharSet = CharSet.Unicode, EntryPoint = "SendMessageW")] static extern IntPtr SendText(IntPtr hwnd, uint msg, IntPtr w, StringBuilder l);
  [DllImport("user32.dll", EntryPoint = "SendMessageW")] static extern IntPtr Send(IntPtr hwnd, uint msg, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hwnd);
  [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr hwnd);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  const uint WM_GETTEXT = 0x000D, WM_GETTEXTLENGTH = 0x000E;

  static string ClassOf(IntPtr hwnd) { var name = new StringBuilder(256); GetClassNameW(hwnd, name, name.Capacity); return name.ToString(); }

  // The top-level window of `pid` titled exactly `title`; IntPtr.Zero when none.
  public static IntPtr FindByTitle(uint pid, string title) {
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

  // A visible top-level window of class `cls` (Notepad's is "Notepad", whichever process owns it:
  // the notepad.exe started may hand over to another one).
  public static IntPtr FindByClass(string cls) {
    IntPtr found = IntPtr.Zero;
    EnumWindows((hwnd, _) => {
      if (!IsWindowVisible(hwnd) || ClassOf(hwnd) != cls) return true;
      found = hwnd;
      return false;
    }, IntPtr.Zero);
    return found;
  }

  public static uint Owner(IntPtr hwnd) { uint pid; GetWindowThreadProcessId(hwnd, out pid); return pid; }

  // The text of the window's edit control: the classic "Edit", or the RichEdit of the newer Notepad.
  public static string EditText(IntPtr hwnd) {
    IntPtr edit = IntPtr.Zero;
    EnumChildWindows(hwnd, (child, _) => {
      var cls = ClassOf(child);
      if (cls != "Edit" && cls != "RichEditD2DPT") return true;
      edit = child;
      return false;
    }, IntPtr.Zero);
    if (edit == IntPtr.Zero) return null;
    int length = Send(edit, WM_GETTEXTLENGTH, IntPtr.Zero, IntPtr.Zero).ToInt32();
    var text = new StringBuilder(length + 1);
    SendText(edit, WM_GETTEXT, new IntPtr(length + 1), text);
    return text.ToString();
  }
}
'@

function Wait-For([scriptblock] $probe, [int] $seconds, [string] $what) {
  $deadline = (Get-Date).AddSeconds($seconds)
  while ((Get-Date) -lt $deadline) {
    $value = & $probe
    if ($value) { return $value }
    Start-Sleep -Milliseconds 250
  }
  throw "smoke-windows-paste: timed out after ${seconds}s waiting for $what"
}

function Log-Text { if (Test-Path -LiteralPath $log) { (Get-Content -LiteralPath $log -Raw -Encoding UTF8) -replace "$([char]27)\[[0-9;]*m", '' } else { '' } }

$A = [System.Windows.Automation.AutomationElement]
$TS = [System.Windows.Automation.TreeScope]
$CT = [System.Windows.Automation.ControlType]
function Cond($property, $value) { New-Object System.Windows.Automation.PropertyCondition($property, $value) }
# Bring a window to the front through UI Automation (a background script may not call
# SetForegroundWindow itself); a refusal is noted, not fatal: the paste reads what is in front.
function Focus([IntPtr] $window, [string] $what) {
  try { $A::FromHandle($window).SetFocus() } catch { Note "focus ${what}: $($_.Exception.Message)" }
}
function Front-Text {
  $front = [VoltipPaste]::GetForegroundWindow()
  $owner = 'none'
  if ($front -ne [IntPtr]::Zero) { $owner = (Get-Process -Id ([VoltipPaste]::Owner($front)) -ErrorAction SilentlyContinue).ProcessName }
  "0x$($front.ToString('x')) ($owner)"
}

# The text to paste: ASCII and CJK ("paste test" in Chinese), so the clipboard path carries both.
$cjk = -join ([char[]](0x7C98, 0x8D34, 0x6D4B, 0x8BD5))
$text = "Voltip paste smoke $cjk"
# The button's accessible name in both interface languages (English, and the Chinese of zh-CN.ts).
$buttonNames = @('Paste into the previous window', (-join ([char[]](0x7C98, 0x8D34, 0x5230, 0x4E0A, 0x4E00, 0x4E2A, 0x7A97, 0x53E3))))

$dataDir = Join-Path ([Environment]::GetFolderPath('ApplicationData')) 'voltip\Voltip\data'
$history = Join-Path $dataDir 'history.json'
$backup = "$history.smoke-backup"
$env:RUST_LOG = 'voltip=info'
$env:NO_COLOR = '1'
$app = $null
$notepad = $null
try {
  # 1. One entry in the history (UTF-8 without a BOM: the app's JSON reader takes no BOM).
  New-Item -ItemType Directory -Force -Path $dataDir | Out-Null
  if (Test-Path -LiteralPath $history) { Move-Item -LiteralPath $history -Destination $backup -Force; Note 'existing history set aside' }
  $entry = [ordered]@{
    id = '6f1c2a3b-4d5e-4f60-8a7b-9c0d1e2f3a4b'
    at_ms = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
    raw_text = $text
    text = $text
    refined = $false
    asr_model = 'sense-voice-small'
    duration_ms = 1200
    asr_ms = 300
    outcome = @{ kind = 'inserted'; via = 'paste' }
    starred = $false
    mode = 'whole_take'
    kind = 'dictation'
  }
  $json = ConvertTo-Json -InputObject ([ordered]@{ schema = 1; entries = @($entry) }) -Depth 5
  [System.IO.File]::WriteAllText($history, $json, (New-Object System.Text.UTF8Encoding $false))
  Note "history seeded at $history"

  # 2. Notepad first, Voltip on top of it.
  $notepad = Start-Process -FilePath 'notepad.exe' -PassThru
  $script:pad = Wait-For { $w = [VoltipPaste]::FindByClass('Notepad'); if ($w -ne [IntPtr]::Zero) { $w } } $StepTimeoutSec 'the Notepad window'
  $pad = $script:pad
  if ($null -eq [VoltipPaste]::EditText($pad)) { throw 'smoke-windows-paste: Notepad has no edit control this script can read' }
  Focus $pad 'Notepad'
  Note "notepad window 0x$($pad.ToString('x')) (pid $([VoltipPaste]::Owner($pad)))"

  $app = Start-Process -FilePath $Binary -PassThru -RedirectStandardError $log -RedirectStandardOutput (Join-Path $OutDir 'app.out')
  $null = $app.Handle
  $script:hwnd = [IntPtr]::Zero
  Wait-For { $script:hwnd = [VoltipPaste]::FindByTitle([uint32]$app.Id, 'Voltip'); $script:hwnd -ne [IntPtr]::Zero -and [VoltipPaste]::IsWindowVisible($script:hwnd) } $WindowTimeoutSec 'the main window' | Out-Null
  $hwnd = $script:hwnd
  $window = $A::FromHandle($hwnd)
  Focus $hwnd 'Voltip'
  Note "voltip window 0x$($hwnd.ToString('x')) (pid $($app.Id)); in front: $(Front-Text)"

  # 3. The recent table's paste button, found in the webview's accessibility tree.
  $button = Wait-For {
    foreach ($name in $buttonNames) {
      $found = $window.FindFirst($TS::Descendants, (New-Object System.Windows.Automation.AndCondition((Cond $A::NameProperty $name), (Cond $A::ControlTypeProperty $CT::Button))))
      if ($null -ne $found) { return $found }
    }
  } $StepTimeoutSec 'the paste button in the recent table'
  Note "button '$($button.Current.Name)'"
  $button.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke()
  # What is in front while Voltip steps aside, change by change (the diagnosis when the paste does
  # not land: Voltip minimised or not, and which window Windows activated).
  $seen = New-Object System.Collections.Generic.List[string]
  $until = (Get-Date).AddSeconds(3)
  while ((Get-Date) -lt $until) {
    $now = "$(Front-Text) voltip-minimised=$([VoltipPaste]::IsIconic($hwnd))"
    if ($seen.Count -eq 0 -or $seen[$seen.Count - 1] -ne $now) { $seen.Add($now) }
    Start-Sleep -Milliseconds 50
  }
  Note "in front after the click: $($seen -join ' > ')"

  # 4. Notepad receives the text; Voltip stays out of the way.
  try {
    $pasted = Wait-For { $now = [VoltipPaste]::EditText($pad); if ($now -eq $text) { $now } } $StepTimeoutSec 'the text in Notepad'
  } catch {
    Note "in front: $(Front-Text); notepad holds '$([VoltipPaste]::EditText($pad))'"
    Note "log: $(((Log-Text) -split "`n" | Where-Object { $_ -match 'paste|foreground' } | Select-Object -Last 5) -join ' | ')"
    throw
  }
  Note "notepad holds '$pasted'"
  $answer = Wait-For { if ((Log-Text) -match 'paste from the history handled[^\r\n]*') { $Matches[0] } } $StepTimeoutSec 'the paste in the log'
  Note $answer
  if ($answer -notmatch 'Pasted') { throw "smoke-windows-paste: the core did not report a paste: $answer" }
  if (-not [VoltipPaste]::IsIconic($hwnd)) { throw 'smoke-windows-paste: Voltip came back after a paste that landed' }
  Note 'voltip stays minimised'

  # 5. --quit ends the instance.
  $quit = Start-Process -FilePath $Binary -ArgumentList '--quit' -PassThru
  if (-not $app.WaitForExit($StepTimeoutSec * 1000)) { throw 'smoke-windows-paste: --quit did not end the running instance' }
  [void]$quit.WaitForExit($StepTimeoutSec * 1000)
  if ($app.ExitCode -ne 0) { throw "smoke-windows-paste: the instance exited with $($app.ExitCode)" }
  $app = $null
  Note 'OK'
} finally {
  if ($null -ne $app -and -not $app.HasExited) { Stop-Process -Id $app.Id -Force -ErrorAction SilentlyContinue }
  $pads = Get-Process -Name notepad -ErrorAction SilentlyContinue
  if ($null -ne $pads) { $pads | Stop-Process -Force -ErrorAction SilentlyContinue }
  if (Test-Path -LiteralPath $backup) { Move-Item -LiteralPath $backup -Destination $history -Force }
  elseif (Test-Path -LiteralPath $history) { Remove-Item -LiteralPath $history -Force }
  $summary | Set-Content -LiteralPath (Join-Path $OutDir 'summary.txt') -Encoding utf8
}
