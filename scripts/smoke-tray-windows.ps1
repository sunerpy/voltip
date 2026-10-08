<#
.SYNOPSIS
  Drive the tray of an installed Voltip on a real Windows desktop: the icon is the logo, the
  right-click menu lists its entries, and each entry does what it says.

.DESCRIPTION
  What it proves (docs/dictation.md, section 15.4), on a machine with an interactive desktop and a shell
  taskbar (GitHub's windows-2025 image: install-scripts.yml runs it on the published release):
    1. the GUI starts, the tray is installed (the app's log names the menu's language and whether
       the build has an update source), and the main window hides on WM_CLOSE;
    2. the notification area (or its overflow flyout) holds a button named after the tray tooltip,
       and its pixels are the colour mark: the ink square with the white sound bars and the cyan
       cursor (saved as tray-icon.png), not a blank or foreign icon;
    3. a right click opens the native menu with exactly the build's entries in the UI's language
       (tray-menu.png);
    4. Open shows the main window; Settings shows it with the Settings dialog open (found in the
       webview's accessibility tree, settings.png); Check for Updates asks the update source and
       gets an answer (update.png); the AI Polish submenu lists its switch and every preset with
       the ones in use checked, choosing a preset or the switch reaches the core and the menu is
       rebuilt from what it saved, and choosing the preset in use keeps it checked
       (tray-polish.png, docs/dictation.md section 21); a double click on the tray button shows
       the main window (user request 2026-09-30); Quit ends the process with exit code 0.
  Mouse input is real (SetCursorPos + mouse_event). The popup menu is read through Win32
  (MN_GETHMENU, GetMenuStringW, GetMenuItemRect): UI Automation does not list the entries of the
  app's menu, which the screen shows. Each step has a timeout and the first failure stops the run.
  <OutDir> receives summary.txt, app.log and the screenshots.

.PARAMETER Binary
  Path to the installed voltip-desktop.exe.

.PARAMETER NoUpdater
  The build has no update source (a CI build): the menu has no Check for Updates entry.
#>
param(
  [Parameter(Mandatory = $true)] [string] $Binary,
  [string] $OutDir = 'smoke-tray-windows',
  [switch] $NoUpdater,
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
function Note([string] $line) { $summary.Add($line); Write-Host "smoke-tray-windows: $line" }

Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, System.Drawing
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public class VoltipMenuEntry { public string Name; public int X; public int Y; public bool Checked; }
public class VoltipMenu {
  public IntPtr Hwnd;
  public int Left, Top, Right, Bottom;
  public List<VoltipMenuEntry> Entries = new List<VoltipMenuEntry>();
}
public static class VoltipTray {
  [StructLayout(LayoutKind.Sequential)] struct RECT { public int Left, Top, Right, Bottom; }
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassNameW(IntPtr hwnd, StringBuilder name, int max);
  [DllImport("user32.dll")] static extern IntPtr SendMessageW(IntPtr hwnd, uint msg, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] static extern int GetMenuItemCount(IntPtr menu);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetMenuStringW(IntPtr menu, uint item, StringBuilder text, int max, uint flags);
  [DllImport("user32.dll")] static extern bool GetMenuItemRect(IntPtr hwnd, IntPtr menu, uint item, out RECT rect);
  [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr hwnd, out RECT rect);
  [DllImport("user32.dll")] static extern uint GetMenuState(IntPtr menu, uint item, uint flags);
  [DllImport("user32.dll")] static extern void keybd_event(byte key, byte scan, uint flags, UIntPtr extra);
  const uint MN_GETHMENU = 0x01E1, MF_BYPOSITION = 0x0400, MF_CHECKED = 0x0008, KEYUP = 0x0002;
  // The visible popup menu (window class #32768) and its entries with their centres on screen;
  // separators have no text and are left out. null while no such menu is up.
  public static VoltipMenu FindOpenMenu() {
    var all = FindOpenMenus();
    return all.Count == 0 ? null : all[0];
  }
  // Every visible popup menu, topmost first (an open submenu is its own #32768 window).
  public static List<VoltipMenu> FindOpenMenus() {
    var found = new List<VoltipMenu>();
    EnumWindows((hwnd, _) => {
      var cls = new StringBuilder(64);
      GetClassNameW(hwnd, cls, cls.Capacity);
      if (cls.ToString() != "#32768" || !IsWindowVisible(hwnd)) return true;
      IntPtr menu = SendMessageW(hwnd, MN_GETHMENU, IntPtr.Zero, IntPtr.Zero);
      if (menu == IntPtr.Zero) return true;
      RECT frame;
      GetWindowRect(hwnd, out frame);
      var open = new VoltipMenu { Hwnd = hwnd, Left = frame.Left, Top = frame.Top, Right = frame.Right, Bottom = frame.Bottom };
      int count = GetMenuItemCount(menu);
      for (uint i = 0; i < count; i++) {
        var text = new StringBuilder(256);
        if (GetMenuStringW(menu, i, text, text.Capacity, MF_BYPOSITION) <= 0) continue;
        RECT r;
        if (!GetMenuItemRect(IntPtr.Zero, menu, i, out r)) continue;
        bool on = (GetMenuState(menu, i, MF_BYPOSITION) & MF_CHECKED) != 0;
        open.Entries.Add(new VoltipMenuEntry { Name = text.ToString(), X = (r.Left + r.Right) / 2, Y = (r.Top + r.Bottom) / 2, Checked = on });
      }
      if (open.Entries.Count > 0) found.Add(open);
      return true;
    }, IntPtr.Zero);
    return found;
  }
  // Esc twice: closes a submenu and the menu under it.
  public static void Escape() {
    for (int i = 0; i < 2; i++) {
      keybd_event(0x1B, 0, 0, UIntPtr.Zero);
      System.Threading.Thread.Sleep(40);
      keybd_event(0x1B, 0, KEYUP, UIntPtr.Zero);
      System.Threading.Thread.Sleep(120);
    }
  }
  delegate bool EnumProc(IntPtr hwnd, IntPtr lparam);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc proc, IntPtr lparam);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetWindowTextW(IntPtr hwnd, StringBuilder text, int max);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hwnd);
  [DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr hwnd, uint msg, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extra);
  public const uint WM_CLOSE = 0x0010;
  const uint LEFTDOWN = 0x0002, LEFTUP = 0x0004, RIGHTDOWN = 0x0008, RIGHTUP = 0x0010;
  // The top-level window of `pid` titled exactly `title`, visible or not; IntPtr.Zero when none.
  public static IntPtr Find(uint pid, string title) {
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
  public static void Click(int x, int y, bool right) {
    SetCursorPos(x, y);
    System.Threading.Thread.Sleep(150);
    mouse_event(right ? RIGHTDOWN : LEFTDOWN, 0, 0, 0, UIntPtr.Zero);
    System.Threading.Thread.Sleep(60);
    mouse_event(right ? RIGHTUP : LEFTUP, 0, 0, 0, UIntPtr.Zero);
  }
  // Two left presses 100 ms apart, well inside the double-click time.
  public static void DoubleClick(int x, int y) {
    SetCursorPos(x, y);
    System.Threading.Thread.Sleep(150);
    for (int press = 0; press < 2; press++) {
      if (press > 0) System.Threading.Thread.Sleep(60);
      mouse_event(LEFTDOWN, 0, 0, 0, UIntPtr.Zero);
      System.Threading.Thread.Sleep(40);
      mouse_event(LEFTUP, 0, 0, 0, UIntPtr.Zero);
    }
  }
}
'@
# Physical pixels everywhere: UI Automation rectangles, the cursor and the screen copy agree.
[void][VoltipTray]::SetProcessDPIAware()

$A = [System.Windows.Automation.AutomationElement]
$TS = [System.Windows.Automation.TreeScope]
$CT = [System.Windows.Automation.ControlType]
function Cond($property, $value) { New-Object System.Windows.Automation.PropertyCondition($property, $value) }
function And-Cond($a, $b) { New-Object System.Windows.Automation.AndCondition($a, $b) }

function Wait-For([scriptblock] $probe, [int] $seconds, [string] $what) {
  $deadline = (Get-Date).AddSeconds($seconds)
  while ((Get-Date) -lt $deadline) {
    $value = & $probe
    if ($value) { return $value }
    Start-Sleep -Milliseconds 250
  }
  throw "smoke-tray-windows: timed out after ${seconds}s waiting for $what"
}

# A string from code points: U 0x8BBE 0x7F6E.
function U { -join ($args | ForEach-Object { [char][int]$_ }) }

function Log-Text { if (Test-Path -LiteralPath $log) { (Get-Content -LiteralPath $log -Raw) -replace "$([char]27)\[[0-9;]*m", '' } else { '' } }

function Center($element) {
  $r = $element.Current.BoundingRectangle
  return @([int]($r.X + $r.Width / 2), [int]($r.Y + $r.Height / 2))
}

function Save-Rect([int] $x, [int] $y, [int] $w, [int] $h, [string] $name) {
  if ($w -le 0 -or $h -le 0) { throw "smoke-tray-windows: $name has an empty rectangle" }
  $bitmap = New-Object System.Drawing.Bitmap($w, $h)
  $g = [System.Drawing.Graphics]::FromImage($bitmap)
  $g.CopyFromScreen($x, $y, 0, 0, $bitmap.Size)
  $g.Dispose()
  $bitmap.Save((Join-Path $OutDir $name), [System.Drawing.Imaging.ImageFormat]::Png)
  return $bitmap
}

function Save-Shot($element, [string] $name) {
  $r = $element.Current.BoundingRectangle
  return (Save-Rect ([int]$r.X) ([int]$r.Y) ([int][Math]::Ceiling($r.Width)) ([int][Math]::Ceiling($r.Height)) $name)
}

# The tray button: named after the tooltip ("Voltip" while idle), in the taskbar's notification
# area or, for an icon the shell has not promoted, in the overflow flyout behind "Show Hidden Icons".
# A closed flyout can still list its buttons (off screen): then the chevron opens it and the next
# call finds the button on screen.
function Find-TrayButton {
  $button = And-Cond (Cond $A::ControlTypeProperty $CT::Button) (Cond $A::NameProperty 'Voltip')
  $taskbar = $A::RootElement.FindFirst($TS::Children, (Cond $A::ClassNameProperty 'Shell_TrayWnd'))
  # UI Automation now and then misses the taskbar among the desktop's children: ask again.
  if ($null -eq $taskbar) { return $null }
  $found = $taskbar.FindFirst($TS::Descendants, $button)
  if ($null -ne $found -and -not $found.Current.IsOffscreen) { return @($found, 'notification area') }
  $overflow = $A::RootElement.FindFirst($TS::Children, (Cond $A::ClassNameProperty 'TopLevelWindowForOverflowXamlIsland'))
  if ($null -ne $overflow -and -not $overflow.Current.IsOffscreen) {
    $found = $overflow.FindFirst($TS::Descendants, $button)
    if ($null -ne $found -and -not $found.Current.IsOffscreen) { return @($found, 'overflow flyout') }
  }
  $chevron = $taskbar.FindFirst($TS::Descendants, (Cond $A::AutomationIdProperty 'SystemTrayIcon'))
  if ($null -eq $chevron) { $chevron = $taskbar.FindFirst($TS::Descendants, (Cond $A::NameProperty 'Show Hidden Icons')) }
  if ($null -ne $chevron) {
    $xy = Center $chevron
    [VoltipTray]::Click($xy[0], $xy[1], $false)
    Start-Sleep -Milliseconds 700
  }
  return $null
}

# Right-click the tray button and return the menu it opens (VoltipMenu: its frame and entries). A
# click that lands while the flyout closes opens nothing: the button is looked up and clicked again.
function Open-TrayMenu {
  for ($attempt = 1; $attempt -le 4; $attempt++) {
    $tray = Wait-For { Find-TrayButton } $StepTimeoutSec 'the tray button'
    $xy = Center $tray[0]
    [VoltipTray]::Click($xy[0], $xy[1], $true)
    $deadline = (Get-Date).AddSeconds(5)
    while ((Get-Date) -lt $deadline) {
      $menu = [VoltipTray]::FindOpenMenu()
      if ($null -ne $menu) { return $menu }
      Start-Sleep -Milliseconds 250
    }
    Note "right click $attempt on the tray button opened no menu; looking for the button again"
  }
  throw 'smoke-tray-windows: four right clicks on the tray button opened no menu'
}

function Click-MenuItem($menu, [string] $name) {
  foreach ($entry in $menu.Entries) {
    if ($entry.Name -eq $name) {
      [VoltipTray]::Click($entry.X, $entry.Y, $false)
      return
    }
  }
  throw "smoke-tray-windows: no menu entry '$name'"
}

# The AI Polish submenu: its entry clicked in the tray menu, then the popup that lists the switch.
function Open-PolishMenu {
  $opened = Open-TrayMenu
  Click-MenuItem $opened $polishLabel
  return Wait-For {
    foreach ($m in [VoltipTray]::FindOpenMenus()) {
      if (@($m.Entries | Where-Object { $_.Name -eq $toggleLabel }).Count -gt 0) { return $m }
    }
  } $StepTimeoutSec 'the AI Polish submenu'
}

# The checked entries of the submenu once the menu shows `$want` (rebuilt from the core's settings
# after a choice): the submenu is opened, read and closed until it does.
function Wait-PolishChecked([string[]] $want, [string] $what) {
  return Wait-For {
    $sub = Open-PolishMenu
    $checked = @($sub.Entries | Where-Object { $_.Checked } | ForEach-Object { $_.Name })
    [VoltipTray]::Escape()
    if (($checked -join "`n") -eq ($want -join "`n")) { return ,$checked }
  } $StepTimeoutSec $what
}

function Count-Near($bitmap, [int[]] $rgb, [int] $tolerance) {
  $n = 0
  for ($y = 0; $y -lt $bitmap.Height; $y++) {
    for ($x = 0; $x -lt $bitmap.Width; $x++) {
      $p = $bitmap.GetPixel($x, $y)
      $d = [Math]::Abs($p.R - $rgb[0]) + [Math]::Abs($p.G - $rgb[1]) + [Math]::Abs($p.B - $rgb[2])
      if ($d -le $tolerance) { $n++ }
    }
  }
  return $n
}

$env:RUST_LOG = 'voltip=info'
$env:NO_COLOR = '1'
$app = $null
try {
  Get-Process -Name 'voltip-desktop' -ErrorAction SilentlyContinue | Stop-Process -Force
  Note "binary $Binary ($((Get-FileHash -LiteralPath $Binary -Algorithm SHA256).Hash.ToLowerInvariant()))"
  $app = Start-Process -FilePath $Binary -PassThru -RedirectStandardError $log -RedirectStandardOutput (Join-Path $OutDir 'app.out')
  $null = $app.Handle
  Note "started pid $($app.Id)"

  # 1. The main window, the tray, and the window hidden (so Open has something to show).
  $hwnd = Wait-For {
    $h = [VoltipTray]::Find([uint32]$app.Id, 'Voltip')
    if ($h -ne [IntPtr]::Zero -and [VoltipTray]::IsWindowVisible($h)) { $h }
  } $WindowTimeoutSec 'the main window'
  $installed = Wait-For { if ((Log-Text) -match 'tray icon (not )?installed[^\r\n]*') { $Matches[0] } } $StepTimeoutSec 'the tray install line'
  if ($installed -match 'not installed') { throw "smoke-tray-windows: $installed" }
  Note $installed
  $zh = $installed -match 'locale=ZhCn'
  $updater = $installed -match 'updater=true'
  if ($updater -eq [bool]$NoUpdater) { throw "smoke-tray-windows: updater=$updater, expected $(-not $NoUpdater)" }
  # Windows PowerShell 5.1 reads a BOM-less script as ANSI, so the labels' non-ASCII characters
  # are spelled as code points: the ellipsis U+2026 and the Chinese labels.
  $dots = [string][char]0x2026
  $settingsZh = U 0x8BBE 0x7F6E
  $labels = if ($zh) {
    @("$(U 0x6253 0x5F00) Voltip", "AI $(U 0x6DA6 0x8272)", "$settingsZh$dots", "$(U 0x68C0 0x67E5 0x66F4 0x65B0)$dots", "$(U 0x9000 0x51FA) Voltip")
  } else {
    @('Open Voltip', 'AI Polish', "Settings$dots", "Check for Updates$dots", 'Quit Voltip')
  }
  $openLabel, $polishLabel, $settingsLabel, $updateLabel, $quitLabel = $labels
  $expected = @($labels | Where-Object { $updater -or $_ -ne $updateLabel })
  # The AI Polish submenu: the switch, then the built-in presets (a fresh profile has no custom one).
  $toggleLabel = if ($zh) { "$(U 0x542F 0x7528) AI $(U 0x6DA6 0x8272)" } else { 'Enable AI Polish' }
  $presetLabels = if ($zh) {
    @("$(U 0x6821 0x5BF9)", "$(U 0x63D0 0x793A 0x8BCD 0x4F18 0x5316)", "$(U 0x610F 0x56FE 0x6574 0x7406)", "$(U 0x53E3 0x8BED 0x804A 0x5929)", "$(U 0x4E2D 0x82F1 0x4E92 0x8BD1)", "$(U 0x8981 0x70B9 0x7EAA 0x8981)", "$(U 0x53EA 0x52A0 0x6807 0x70B9)", "$(U 0x4E66 0x9762 0x8BED)")
  } else {
    @('Proofread', 'Prompt optimizer', 'Clarify intent', 'Casual chat', "Chinese $(U 0x21C4) English", 'Key points', 'Punctuation only', 'Formal')
  }
  [void][VoltipTray]::PostMessageW($hwnd, [VoltipTray]::WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
  Wait-For { -not [VoltipTray]::IsWindowVisible($hwnd) } $StepTimeoutSec 'the window to hide' | Out-Null
  Note 'main window hidden to the tray'

  # 2. The icon in the notification area is the colour mark.
  $tray = Wait-For { Find-TrayButton } $StepTimeoutSec 'the tray button'
  $shot = Save-Shot $tray[0] 'tray-icon.png'
  # The bars sit on whole pixels, so their insides are pure white: a light taskbar does not count.
  $ink = Count-Near $shot @(0x0B, 0x12, 0x20) 60
  $white = Count-Near $shot @(0xFF, 0xFF, 0xFF) 15
  $cyan = Count-Near $shot @(0x22, 0xD3, 0xEE) 90
  Note "tray button in the $($tray[1]) ($($shot.Width)x$($shot.Height) px): ink $ink, white $white, cyan $cyan pixels"
  if ($ink -lt 30 -or $white -lt 4 -or $cyan -lt 4) { throw 'smoke-tray-windows: the tray icon is not the Voltip mark (see tray-icon.png)' }
  $shot.Dispose()

  # 3. The menu.
  $opened = Open-TrayMenu
  $names = @($opened.Entries | ForEach-Object { $_.Name })
  (Save-Rect $opened.Left $opened.Top ($opened.Right - $opened.Left) ($opened.Bottom - $opened.Top) 'tray-menu.png').Dispose()
  Note "menu: $($names -join ' | ')"
  if (($names -join "`n") -ne ($expected -join "`n")) { throw "smoke-tray-windows: menu is '$($names -join ' | ')', expected '$($expected -join ' | ')'" }

  # 4a. Open.
  Click-MenuItem $opened $openLabel
  Wait-For { (Log-Text) -match 'tray menu action=Open' } $StepTimeoutSec 'the Open entry in the log' | Out-Null
  Wait-For { [VoltipTray]::IsWindowVisible($hwnd) } $StepTimeoutSec 'Open to show the window' | Out-Null
  Note 'Open: main window shown'
  [void][VoltipTray]::PostMessageW($hwnd, [VoltipTray]::WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
  Wait-For { -not [VoltipTray]::IsWindowVisible($hwnd) } $StepTimeoutSec 'the window to hide' | Out-Null

  # 4b. Settings: the window with the Settings dialog (the webview's role=dialog, named by its title).
  $opened = Open-TrayMenu
  Click-MenuItem $opened $settingsLabel
  Wait-For { (Log-Text) -match 'tray menu action=Settings' } $StepTimeoutSec 'the Settings entry in the log' | Out-Null
  Wait-For { [VoltipTray]::IsWindowVisible($hwnd) } $StepTimeoutSec 'Settings to show the window' | Out-Null
  $window = $A::FromHandle($hwnd)
  $title = if ($zh) { $settingsZh } else { 'Settings' }
  $dialog = Wait-For {
    $window.FindFirst($TS::Descendants, (And-Cond (Cond $A::NameProperty $title) (Cond $A::LocalizedControlTypeProperty 'dialog')))
  } $StepTimeoutSec "the '$title' dialog in the webview"
  (Save-Shot $window 'settings.png').Dispose()
  Note "Settings: window shown with the '$($dialog.Current.Name)' dialog"
  [void][VoltipTray]::PostMessageW($hwnd, [VoltipTray]::WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
  Wait-For { -not [VoltipTray]::IsWindowVisible($hwnd) } $StepTimeoutSec 'the window to hide' | Out-Null

  # 4c. Check for Updates: the webview asks the update source and gets an answer.
  if ($updater) {
    $opened = Open-TrayMenu
    Click-MenuItem $opened $updateLabel
    Wait-For { (Log-Text) -match 'tray menu action=CheckUpdate' } $StepTimeoutSec 'the Check for Updates entry in the log' | Out-Null
    $answer = Wait-For { if ((Log-Text) -match '(no update available|update available)[^\r\n]*') { $Matches[0] } } 60 'the update check to answer'
    Wait-For { [VoltipTray]::IsWindowVisible($hwnd) } $StepTimeoutSec 'Check for Updates to show the window' | Out-Null
    (Save-Shot $A::FromHandle($hwnd) 'update.png').Dispose()
    Note "Check for Updates: $answer"
  }

  # 4d. AI Polish (docs/dictation.md section 21): the switch and every preset, the ones in use
  # checked; each choice goes through the core, and the menu is rebuilt from what it saved.
  $sub = Open-PolishMenu
  (Save-Rect $sub.Left $sub.Top ($sub.Right - $sub.Left) ($sub.Bottom - $sub.Top) 'tray-polish.png').Dispose()
  $names = @($sub.Entries | ForEach-Object { $_.Name })
  $want = @($toggleLabel) + $presetLabels
  Note "AI Polish: $($names -join ' | ')"
  if (($names -join "`n") -ne ($want -join "`n")) { throw "smoke-tray-windows: the AI Polish submenu is '$($names -join ' | ')', expected '$($want -join ' | ')'" }
  $checked = @($sub.Entries | Where-Object { $_.Checked } | ForEach-Object { $_.Name })
  if (($checked -join "`n") -ne (@($toggleLabel, $presetLabels[0]) -join "`n")) { throw "smoke-tray-windows: checked '$($checked -join ' | ')', expected the switch and the first preset" }
  Click-MenuItem $sub $presetLabels[1]
  Wait-For { (Log-Text) -match 'tray menu polish action=Preset\("prompt"\)' } $StepTimeoutSec 'the preset entry in the log' | Out-Null
  Wait-PolishChecked @($toggleLabel, $presetLabels[1]) 'the second preset checked' | Out-Null
  Note "AI Polish: chose '$($presetLabels[1])'"
  # The preset in use again: the OS unchecks a clicked check item; the menu must show it checked.
  $sub = Open-PolishMenu
  Click-MenuItem $sub $presetLabels[1]
  Wait-PolishChecked @($toggleLabel, $presetLabels[1]) 'the preset in use still checked' | Out-Null
  $sub = Open-PolishMenu
  Click-MenuItem $sub $toggleLabel
  Wait-For { (Log-Text) -match 'tray menu polish action=Toggle' } $StepTimeoutSec 'the switch in the log' | Out-Null
  Wait-PolishChecked @($presetLabels[1]) 'the switch off' | Out-Null
  Note 'AI Polish: switched off'
  # Leave the profile as it was found: the switch on, the first preset.
  $sub = Open-PolishMenu
  Click-MenuItem $sub $toggleLabel
  Wait-PolishChecked @($toggleLabel, $presetLabels[1]) 'the switch on again' | Out-Null
  $sub = Open-PolishMenu
  Click-MenuItem $sub $presetLabels[0]
  Wait-PolishChecked @($toggleLabel, $presetLabels[0]) 'the defaults back' | Out-Null

  # 4e. A double click on the tray button (user request 2026-09-30) leaves the main window shown.
  # Its first click shows the window already, and where the button sits in the overflow flyout
  # the window coming up can close the flyout before the second click: whether the shell reported
  # a double click is noted, the window is what is checked. As with a right click, a double click
  # that lands while the flyout closes reaches nothing (CI 2026-10-01: no click in the app's log):
  # the button is looked up and double-clicked again.
  if ([VoltipTray]::IsWindowVisible($hwnd)) {
    [void][VoltipTray]::PostMessageW($hwnd, [VoltipTray]::WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
    Wait-For { -not [VoltipTray]::IsWindowVisible($hwnd) } $StepTimeoutSec 'the window to hide' | Out-Null
  }
  $doubles = ([regex]::Matches((Log-Text), 'tray double click: main window')).Count
  $shown = $false
  for ($attempt = 1; $attempt -le 4 -and -not $shown; $attempt++) {
    $clicks = ([regex]::Matches((Log-Text), 'tray click: main window')).Count
    $tray = Wait-For { Find-TrayButton } $StepTimeoutSec 'the tray button'
    $xy = Center $tray[0]
    [VoltipTray]::DoubleClick($xy[0], $xy[1])
    $deadline = (Get-Date).AddSeconds(5)
    while (-not $shown -and (Get-Date) -lt $deadline) {
      $shown = [VoltipTray]::IsWindowVisible($hwnd)
      if (-not $shown) { Start-Sleep -Milliseconds 250 }
    }
    if (-not $shown) {
      $reached = ([regex]::Matches((Log-Text), 'tray click: main window')).Count - $clicks
      Note "double click $attempt on the tray button showed no window ($reached clicks reached the app); looking for the button again"
    }
  }
  if (-not $shown) { throw 'smoke-tray-windows: four double clicks on the tray button showed no window' }
  # The shell's report, when it makes one, follows the second press.
  $reported = $false
  $deadline = (Get-Date).AddSeconds(3)
  while (-not $reported -and (Get-Date) -lt $deadline) {
    $reported = ([regex]::Matches((Log-Text), 'tray double click: main window')).Count -gt $doubles
    if (-not $reported) { Start-Sleep -Milliseconds 250 }
  }
  Note "double click in the $($tray[1]): main window shown (reported as a double click: $reported)"
  [void][VoltipTray]::PostMessageW($hwnd, [VoltipTray]::WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
  Wait-For { -not [VoltipTray]::IsWindowVisible($hwnd) } $StepTimeoutSec 'the window to hide' | Out-Null

  # 4f. Quit.
  $opened = Open-TrayMenu
  Click-MenuItem $opened $quitLabel
  if (-not $app.WaitForExit($StepTimeoutSec * 1000)) { throw 'smoke-tray-windows: Quit did not end the process' }
  if (-not ((Log-Text) -match 'tray menu action=Quit')) { throw 'smoke-tray-windows: the process ended without the Quit entry in the log' }
  Note "Quit: process exited with $($app.ExitCode)"
  if ($app.ExitCode -ne 0) { throw "smoke-tray-windows: exit code $($app.ExitCode)" }
  $app = $null
  Note 'OK'
} catch {
  Note "FAIL: $($_.Exception.Message)"
  try { (Save-Shot $A::RootElement 'failure-screen.png').Dispose() } catch { Note "no failure screenshot: $($_.Exception.Message)" }
  throw
} finally {
  if ($null -ne $app -and -not $app.HasExited) { Stop-Process -Id $app.Id -Force -ErrorAction SilentlyContinue }
  $summary | Set-Content -LiteralPath (Join-Path $OutDir 'summary.txt') -Encoding utf8
}
