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
       script never activates Notepad itself (a background script may not): Voltip, still in
       front, activates the window below its own and then minimises;
    3. Voltip stays minimised after a paste that landed (it comes back only when it did not).
  The entry goes into the runner user's real data directory (Windows resolves it through the
  Known Folder API, which no environment variable redirects) as a history.json that the app
  imports into a new history.sqlite3 at start-up (docs/dictation.md section 4.3); a database or a
  history.json that is already there is set aside first and put back at the end, and what the run
  wrote is removed. Each step has a timeout and the first
  failure stops the run. <OutDir> receives summary.txt and app.log, and uia-tree.txt (the main
  window's accessibility tree) when the text does not arrive.
  The button is pressed once, and only when the accessibility tree shows the same button element
  (its UI Automation runtime id), enabled, for SettleMs; the element pressed is the one the last
  lookup returned. Twice in some 190 CI runs (2026-11-08, runs 42458882631 and 42495010182) a
  press on the button of the very first lookup, 162 and 171 ms after the main window's page loaded,
  reached nothing: no paste request in the log, Voltip left in front. Of the 61 runs whose logs
  tell, all 58 presses 220 ms or more after the page loaded pasted. Every element id seen, the
  button's state before and after the press, the windows in front and, when the text does not
  arrive, the tree and the log's tail are noted.
  Windows PowerShell 5.1 reads a BOM-less script as ANSI, so every non-ASCII string below is built
  from code points.

.PARAMETER Binary
  Path to voltip-desktop.exe.
.PARAMETER SettleMs
  How long the button must stay the same element before it is pressed. 0 presses the first button
  found, as runs before 2026-11-09 did (to reproduce the lost press).
.PARAMETER PollMs
  How often the tree is looked up while waiting for the button.
#>
param(
  [Parameter(Mandatory = $true)] [string] $Binary,
  [string] $OutDir = 'smoke-windows-paste',
  [int] $WindowTimeoutSec = 120,
  [int] $StepTimeoutSec = 30,
  [int] $SettleMs = 1000,
  [int] $PollMs = 250
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
  [DllImport("user32.dll")] static extern IntPtr GetAncestor(IntPtr hwnd, uint flags);
  const uint GA_ROOT = 2;
  // The top-level window `hwnd` is part of (itself when it is one).
  public static IntPtr Root(IntPtr hwnd) { var root = GetAncestor(hwnd, GA_ROOT); return root == IntPtr.Zero ? hwnd : root; }
  const uint WM_GETTEXT = 0x000D, WM_GETTEXTLENGTH = 0x000E;

  public static string ClassOf(IntPtr hwnd) { var name = new StringBuilder(256); GetClassNameW(hwnd, name, name.Capacity); return name.ToString(); }

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

function Wait-For([scriptblock] $probe, [int] $seconds, [string] $what, [int] $pollMs = 250) {
  $deadline = (Get-Date).AddSeconds($seconds)
  while ((Get-Date) -lt $deadline) {
    $value = & $probe
    if ($value) { return $value }
    Start-Sleep -Milliseconds $pollMs
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
function Window-Text([IntPtr] $window) {
  $owner = 'none'
  if ($window -ne [IntPtr]::Zero) { $owner = (Get-Process -Id ([VoltipPaste]::Owner($window)) -ErrorAction SilentlyContinue).ProcessName }
  "0x$($window.ToString('x')) ($owner, $([VoltipPaste]::ClassOf($window)))"
}
# A UI Automation element as the notes name it: its runtime id (another id is another element),
# name, whether it is enabled and off screen, and its rectangle; one that is gone says so.
function Runtime-Id($element) { try { ($element.GetRuntimeId()) -join '.' } catch { 'gone' } }
function Element-Text($element) {
  if ($null -eq $element) { return 'none' }
  try {
    $c = $element.Current
    $r = $c.BoundingRectangle
    $rect = if ($r.IsEmpty) { 'empty' } else { '{0:0},{1:0} {2:0}x{3:0}' -f $r.X, $r.Y, $r.Width, $r.Height }
    "'$($c.Name)' id=$(Runtime-Id $element) enabled=$($c.IsEnabled) offscreen=$($c.IsOffscreen) rect=$rect"
  } catch { "gone ($($_.Exception.GetType().Name))" }
}
# The main window's tree in the control view, one element a line, for the run whose text does not
# arrive: the toast a paste ends with is in it when the press reached the page.
function Dump-Tree($root, [string] $path, [int] $max = 1500) {
  $walker = [System.Windows.Automation.TreeWalker]::ControlViewWalker
  $lines = New-Object System.Collections.Generic.List[string]
  $stack = New-Object System.Collections.Generic.Stack[object]
  $stack.Push(@($root, 0))
  $budget = [System.Diagnostics.Stopwatch]::StartNew()
  while ($stack.Count -gt 0 -and $lines.Count -lt $max -and $budget.Elapsed.TotalSeconds -lt 20) {
    $item = $stack.Pop()
    $type = '?'
    try { $type = $item[0].Current.ControlType.ProgrammaticName -replace '^ControlType\.', '' } catch { }
    $lines.Add(('  ' * $item[1]) + "$type $(Element-Text $item[0])")
    $children = New-Object System.Collections.Generic.List[object]
    try {
      $child = $walker.GetFirstChild($item[0])
      while ($null -ne $child) { $children.Add($child); $child = $walker.GetNextSibling($child) }
    } catch { }
    for ($i = $children.Count - 1; $i -ge 0; $i--) { $stack.Push(@($children[$i], ($item[1] + 1))) }
  }
  if ($stack.Count -gt 0) { $lines.Add("(stopped after $($lines.Count) elements, $([int]$budget.Elapsed.TotalSeconds) s)") }
  [System.IO.File]::WriteAllLines($path, $lines, (New-Object System.Text.UTF8Encoding $false))
  $lines
}
# When the main window's page ran the interface check (src-tauri/src/ui_guard.rs), from the log.
function Main-Loaded {
  $m = [regex]::Match((Log-Text), '(?m)^(\S+Z)\s+INFO\s+\S*ui_guard\S*:.*window="main"')
  if (-not $m.Success) { return $null }
  [DateTime]::ParseExact($m.Groups[1].Value, 'yyyy-MM-ddTHH:mm:ss.FFFFFFFK', [Globalization.CultureInfo]::InvariantCulture, [Globalization.DateTimeStyles]::AdjustToUniversal)
}
# The window in front, and the top-level window it is part of when it is a child (a web view's
# render window: CI 2026-10-01).
function Front-Text {
  $front = [VoltipPaste]::GetForegroundWindow()
  $text = Window-Text $front
  $root = [VoltipPaste]::Root($front)
  if ($front -ne [IntPtr]::Zero -and $root -ne $front) { $text += " in $(Window-Text $root)" }
  $text
}

# The text to paste: ASCII and CJK ("paste test" in Chinese), so the clipboard path carries both.
$cjk = -join ([char[]](0x7C98, 0x8D34, 0x6D4B, 0x8BD5))
$text = "Voltip paste smoke $cjk"
# The button's accessible name in both interface languages (English, and the Chinese of zh-CN.ts).
$buttonNames = @('Paste into the previous window', (-join ([char[]](0x7C98, 0x8D34, 0x5230, 0x4E0A, 0x4E00, 0x4E2A, 0x7A97, 0x53E3))))
# The recent table's paste button, looked up afresh in the main window's tree ($window, step 2).
function Find-Button {
  foreach ($name in $buttonNames) {
    $found = $window.FindFirst($TS::Descendants, (New-Object System.Windows.Automation.AndCondition((Cond $A::NameProperty $name), (Cond $A::ControlTypeProperty $CT::Button))))
    if ($null -ne $found) { return $found }
  }
  $null
}

$dataDir = Join-Path ([Environment]::GetFolderPath('ApplicationData')) 'voltip\Voltip\data'
$history = Join-Path $dataDir 'history.json'
$database = Join-Path $dataDir 'history.sqlite3'
# The history's files: the database with its WAL files, and the file of before it.
$historyFiles = @($database, "$database-wal", "$database-shm", $history)
# The import renames the file history.json.imported-<seconds>: only the ones this run adds go.
function Imported { @(Get-ChildItem -LiteralPath $dataDir -Filter 'history.json.imported-*' -ErrorAction SilentlyContinue | ForEach-Object { $_.Name }) }
$importedBefore = @()
$env:RUST_LOG = 'voltip=info'
$env:NO_COLOR = '1'
$app = $null
$notepad = $null
try {
  # 1. One entry in the history (UTF-8 without a BOM: the app's JSON reader takes no BOM), in a
  #    history.json the app imports because no database is there.
  New-Item -ItemType Directory -Force -Path $dataDir | Out-Null
  $importedBefore = Imported
  foreach ($file in $historyFiles) {
    if (Test-Path -LiteralPath $file) { Move-Item -LiteralPath $file -Destination "$file.smoke-backup" -Force; Note "existing $(Split-Path -Leaf $file) set aside" }
  }
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

  $started = [System.Diagnostics.Stopwatch]::StartNew()
  $app = Start-Process -FilePath $Binary -PassThru -RedirectStandardError $log -RedirectStandardOutput (Join-Path $OutDir 'app.out')
  $null = $app.Handle
  $script:hwnd = [IntPtr]::Zero
  Wait-For { $script:hwnd = [VoltipPaste]::FindByTitle([uint32]$app.Id, 'Voltip'); $script:hwnd -ne [IntPtr]::Zero -and [VoltipPaste]::IsWindowVisible($script:hwnd) } $WindowTimeoutSec 'the main window' | Out-Null
  $hwnd = $script:hwnd
  $windowMs = $started.ElapsedMilliseconds
  $window = $A::FromHandle($hwnd)
  Focus $hwnd 'Voltip'
  Note "voltip window 0x$($hwnd.ToString('x')) (pid $($app.Id)) at $windowMs ms; in front: $(Front-Text)"

  # 3. The recent table's paste button, found in the webview's accessibility tree and pressed once
  #    it has been the same element, enabled, for SettleMs (the description says why). Every
  #    element the lookups return is noted with the time since Voltip started.
  $script:ids = New-Object System.Collections.Generic.List[string]
  $script:sameId = ''
  $script:sameSince = 0
  try {
    $button = Wait-For {
      $found = Find-Button
      $id = ''
      $enabled = $false
      if ($null -ne $found) {
        $id = Runtime-Id $found
        try { $enabled = $found.Current.IsEnabled } catch { $id = 'gone' }
      }
      if ($id -ne $script:sameId) {
        $script:sameId = $id
        $script:sameSince = $started.ElapsedMilliseconds
        $script:ids.Add("$(if ($id) { $id } else { 'none' }) at $($script:sameSince) ms")
      }
      if ($id -and $id -ne 'gone' -and $enabled -and ($started.ElapsedMilliseconds - $script:sameSince) -ge $SettleMs) { $found }
    } $StepTimeoutSec "the paste button, the same element for $SettleMs ms" $PollMs
  } catch {
    Note "button ids seen: $($script:ids -join ' > '); now: $(Element-Text (Find-Button))"
    throw
  }
  $pressedMs = $started.ElapsedMilliseconds
  Note "button $(Element-Text $button) pressed at $pressedMs ms; in front: $(Front-Text)"
  $pressedAt = [DateTime]::UtcNow
  $button.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke()
  # What is in front while Voltip steps aside, change by change (the diagnosis when the paste does
  # not land: Voltip minimised or not, and which window Windows activated), and the button half a
  # second after the press: disabled while the paste runs, the same element and enabled when the
  # press did not reach the page.
  $seen = New-Object System.Collections.Generic.List[string]
  $buttonAfter = $null
  $until = (Get-Date).AddSeconds(3)
  while ((Get-Date) -lt $until) {
    $now = "$(Front-Text) voltip-minimised=$([VoltipPaste]::IsIconic($hwnd))"
    if ($seen.Count -eq 0 -or $seen[$seen.Count - 1] -ne $now) { $seen.Add($now) }
    if ($null -eq $buttonAfter -and $started.ElapsedMilliseconds - $pressedMs -ge 500) { $buttonAfter = Element-Text (Find-Button) }
    Start-Sleep -Milliseconds 50
  }
  Note "in front after the click: $($seen -join ' > ')"
  Note "button ids seen: $($script:ids -join ' > '); 500 ms after the press: $buttonAfter"
  $loaded = Main-Loaded
  if ($null -ne $loaded) { Note "pressed $([int]($pressedAt - $loaded).TotalMilliseconds) ms after the main window's page loaded" }

  # 4. Notepad receives the text; Voltip stays out of the way.
  try {
    $pasted = Wait-For { $now = [VoltipPaste]::EditText($pad); if ($now -eq $text) { $now } } $StepTimeoutSec 'the text in Notepad'
  } catch {
    Note "in front: $(Front-Text); notepad holds '$([VoltipPaste]::EditText($pad))'"
    Note "button now: $(Element-Text (Find-Button))"
    $tree = @(Dump-Tree $window (Join-Path $OutDir 'uia-tree.txt'))
    Note "uia-tree.txt: $($tree.Count) lines; those that name a paste: $(@($tree | Where-Object { $_ -match 'paste|copied|clipboard' } | ForEach-Object { $_.Trim() } | Select-Object -First 8) -join ' | ')"
    Note "log: $(((Log-Text) -split "`n" | Where-Object { $_ -match 'paste|foreground' } | Select-Object -Last 5) -join ' | ')"
    Note "log tail: $(((Log-Text) -split "`n" | Where-Object { $_.Trim() } | Select-Object -Last 12) -join ' | ')"
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
  # The instance is stopped and waited for: until it has exited it holds the database open, and a
  # removal that fails here would hide the step that failed (CI 2026-10-01).
  if ($null -ne $app -and -not $app.HasExited) {
    Stop-Process -Id $app.Id -Force -ErrorAction SilentlyContinue
    if (-not $app.WaitForExit($StepTimeoutSec * 1000)) { Note 'cleanup: the instance did not exit' }
  }
  $pads = Get-Process -Name notepad -ErrorAction SilentlyContinue
  if ($null -ne $pads) { $pads | Stop-Process -Force -ErrorAction SilentlyContinue }
  # What the run wrote goes (the database the seed was imported into and the renamed seed), then
  # what was set aside comes back. A step that fails is noted, and the rest still run.
  function Cleanup([scriptblock] $step) { try { & $step } catch { Note "cleanup: $($_.Exception.Message)" } }
  foreach ($file in $historyFiles) { if (Test-Path -LiteralPath $file) { Cleanup { Remove-Item -LiteralPath $file -Force } } }
  foreach ($name in (Imported)) { if ($importedBefore -notcontains $name) { Cleanup { Remove-Item -LiteralPath (Join-Path $dataDir $name) -Force } } }
  foreach ($file in $historyFiles) { if (Test-Path -LiteralPath "$file.smoke-backup") { Cleanup { Move-Item -LiteralPath "$file.smoke-backup" -Destination $file -Force } } }
  $summary | Set-Content -LiteralPath (Join-Path $OutDir 'summary.txt') -Encoding utf8
}
