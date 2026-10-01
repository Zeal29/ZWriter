# ZWriter end-to-end smoke test (real global hotkeys).
# Prereq: zwriter.exe running (dev or release), no other instance.
#
# Sections:
#   A. Fix hotkey (Ctrl+Alt+G): capture -> review window -> Apply -> paste
#   B. Quick hotkey (Ctrl+Alt+F): capture -> paste immediately, no window
#   E. Word picker: taskbar visibility, suggestion chip, inline word edit,
#      Apply pastes the EDITED text
#   F. Smart quotes: curly-apostrophe sentence still gets the grammar fix
#   G. Multi-line paragraph: every line fixed, not just the last
#   H. User edit on a clean capture: Apply stays enabled and pastes the edit
#   I. Custom dictionary: picker "Add to dictionary" unflags the word live,
#      Settings add/remove works, removal re-flags it
#   J. Abbreviations + ignored words + guess checkboxes taught in Settings
#   K. Exact abbreviation: review shows the expansion, Apply pastes it
#   L. Quick fix, exact abbreviation: pastes instantly, no window
#   M. Quick fix x guess checkboxes: the full 2x2 matrix + ambiguous carve-out
#   N. Popover teach: "Add as abbreviation" from the word popover,
#      "Ignore word" unflags live
#   C. Rebind: Settings UI record Ctrl+Alt+J -> old chord dead, new works,
#      review-window hotkey hint text follows the rebind
#   D. Rebind back to Ctrl+Alt+G and verify
#
# All keyboard input is REAL (SendInput via SendKeys); UI actions use UIA.
# The machine may be in active use, so every interaction temporarily raises
# its target window topmost, clicks/verifies, then restores normal z-order.

$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Windows.Forms

Add-Type @"
using System;
using System.Text;
using System.Runtime.InteropServices;
public class Win {
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr l);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern int GetWindowText(IntPtr h, StringBuilder t, int n);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, int x, int y, uint d, UIntPtr e);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
  [DllImport("user32.dll")] public static extern int GetWindowLong(IntPtr h, int idx);
  [DllImport("user32.dll")] public static extern IntPtr FindWindow(string cls, string title);
  public struct RECT { public int L, T, R, B; }

  public static IntPtr FindExact(uint pid, string title) {
    IntPtr found = IntPtr.Zero;
    EnumWindows((h, l) => {
      if (!IsWindowVisible(h)) return true;
      uint wpid; GetWindowThreadProcessId(h, out wpid);
      if (wpid != pid) return true;
      var sb = new StringBuilder(256);
      GetWindowText(h, sb, 256);
      if (sb.ToString() == title) { found = h; return false; }
      return true;
    }, IntPtr.Zero);
    return found;
  }
}
"@

Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes

$results = @()
function Check($name, $ok) {
    Write-Host ("[smoke] {0}: {1}" -f ($(if ($ok) {"PASS"} else {"FAIL"})), $name)
    $script:results += @($name, [bool]$ok)
}
function Step($msg) { Write-Host "[smoke] $msg" }

function Show-Above($h) {
    if ($h -eq [IntPtr]::Zero) { return }
    # 0x13 = SWP_NOSIZE | SWP_NOMOVE | SWP_NOACTIVATE. Do NOT use
    # SWP_SHOWWINDOW (0x40) - it re-shows windows the app just hid.
    [Win]::SetWindowPos($h, [IntPtr](-1), 0, 0, 0, 0, 0x13) | Out-Null  # TOPMOST
    Start-Sleep -Milliseconds 200
}
function Un-Top($h) {
    if ($h -eq [IntPtr]::Zero) { return }
    [Win]::SetWindowPos($h, [IntPtr](-2), 0, 0, 0, 0, 0x13) | Out-Null  # NOTOPMOST
}

function Click-At($x, $y) {
    [Win]::SetCursorPos($x, $y) | Out-Null
    Start-Sleep -Milliseconds 150
    [Win]::mouse_event(2, 0, 0, 0, [UIntPtr]::Zero)
    [Win]::mouse_event(4, 0, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 400
}

function Click-Center($h) {
    $r = New-Object Win+RECT
    if (-not [Win]::GetWindowRect($h, [ref]$r)) { return }
    Click-At ([int](($r.L + $r.R) / 2)) ([int](($r.T + $r.B) / 2))
}

function Click-Title($h) {
    if ($h -eq [IntPtr]::Zero) { return }
    $r = New-Object Win+RECT
    [Win]::GetWindowRect($h, [ref]$r) | Out-Null
    if ($r.R -eq 0 -and $r.B -eq 0) { return }
    Click-At ([int](($r.L + $r.R) / 2)) ($r.T + 18)
}

# Click an inert spot inside the settings webview (footer about-text) so the
# page itself has keyboard focus, then Esc reaches the app's Esc handler.
function Close-Settings($st) {
    $r = New-Object Win+RECT
    [Win]::GetWindowRect($st, [ref]$r) | Out-Null
    Click-At ([int](($r.L + $r.R) / 2)) ($r.B - 26)
    Start-Sleep -Milliseconds 200
    [System.Windows.Forms.SendKeys]::SendWait("{ESC}")
    Start-Sleep -Milliseconds 800
}

# UIA: first button whose Name matches -like $pattern; geometry-click it.
# WebView2's tree takes seconds to materialize after a window appears.
# Every COM call is guarded: a stale/zero hwnd must FAIL the check, not
# abort the whole suite with an uncaught exception.
# The settings page now scrolls (4 teach sections) - ScrollIntoView before
# reading geometry, or the click lands on an element that is off-screen.
function Scroll-Into-View($e) {
    try {
        $si = $e.GetCurrentPattern([System.Windows.Automation.ScrollItemPattern]::Pattern)
        $si.ScrollIntoView()
        Start-Sleep -Milliseconds 350
    } catch { }
}

function Click-Button($hwnd, $pattern) {
    if ($hwnd -eq [IntPtr]::Zero) { return $false }
    foreach ($try in 1..8) {
        try {
            $el = [System.Windows.Automation.AutomationElement]::FromHandle($hwnd)
            $all = $el.FindAll([System.Windows.Automation.TreeScope]::Descendants,
                [System.Windows.Automation.Condition]::TrueCondition)
            foreach ($e in $all) {
                if ($e.Current.ControlType.ProgrammaticName -eq "ControlType.Button" -and
                    $e.Current.Name -like $pattern) {
                    Scroll-Into-View $e
                    $r = $e.Current.BoundingRectangle
                    Click-At ([int]($r.X + $r.Width / 2)) ([int]($r.Y + $r.Height / 2))
                    return $true
                }
            }
        } catch { Start-Sleep -Milliseconds 600 }
        Start-Sleep -Milliseconds 600
    }
    return $false
}

function Has-Button($hwnd, $pattern) {
    if ($hwnd -eq [IntPtr]::Zero) { return $false }
    foreach ($try in 1..6) {
        try {
            $el = [System.Windows.Automation.AutomationElement]::FromHandle($hwnd)
            $all = $el.FindAll([System.Windows.Automation.TreeScope]::Descendants,
                [System.Windows.Automation.Condition]::TrueCondition)
            foreach ($e in $all) {
                if ($e.Current.ControlType.ProgrammaticName -eq "ControlType.Button" -and
                    $e.Current.Name -like $pattern) { return $true }
            }
        } catch { }
        Start-Sleep -Milliseconds 600
    }
    return $false
}

# Click an editable (ControlType.Edit) element by name - the word-picker input.
function Click-Edit($hwnd, $pattern) {
    if ($hwnd -eq [IntPtr]::Zero) { return $false }
    foreach ($try in 1..8) {
        try {
            $el = [System.Windows.Automation.AutomationElement]::FromHandle($hwnd)
            $all = $el.FindAll([System.Windows.Automation.TreeScope]::Descendants,
                [System.Windows.Automation.Condition]::TrueCondition)
            foreach ($e in $all) {
                if ($e.Current.ControlType.ProgrammaticName -eq "ControlType.Edit" -and
                    $e.Current.Name -like $pattern) {
                    Scroll-Into-View $e
                    $r = $e.Current.BoundingRectangle
                    Click-At ([int]($r.X + $r.Width / 2)) ([int]($r.Y + $r.Height / 2))
                    return $true
                }
            }
        } catch { Start-Sleep -Milliseconds 600 }
        Start-Sleep -Milliseconds 600
    }
    return $false
}

# Toggle a checkbox (ControlType.CheckBox) until its UIA ToggleState matches
# $want ("On"/"Off"). A single geometry click can be eaten by focus or a
# stale post-scroll rect, and an unverified click silently tests the WRONG
# branch (M6-M8 failed exactly that way in the first run).
function Set-Checkbox($hwnd, $pattern, $want) {
    if ($hwnd -eq [IntPtr]::Zero) { return $false }
    foreach ($try in 1..6) {
        try {
            $el = [System.Windows.Automation.AutomationElement]::FromHandle($hwnd)
            $all = $el.FindAll([System.Windows.Automation.TreeScope]::Descendants,
                [System.Windows.Automation.Condition]::TrueCondition)
            foreach ($e in $all) {
                if ($e.Current.ControlType.ProgrammaticName -eq "ControlType.CheckBox" -and
                    $e.Current.Name -like $pattern) {
                    $tp = $e.GetCurrentPattern([System.Windows.Automation.TogglePattern]::Pattern)
                    if ($tp.Current.ToggleState.ToString() -eq $want) { return $true }
                    Scroll-Into-View $e
                    $r = $e.Current.BoundingRectangle
                    Click-At ([int]($r.X + $r.Width / 2)) ([int]($r.Y + $r.Height / 2))
                    Start-Sleep -Milliseconds 500
                }
            }
        } catch { Start-Sleep -Milliseconds 600 }
        Start-Sleep -Milliseconds 600
    }
    return $false
}

# Concatenated text of every TextPattern element in a window.
function Read-Window-Text($h) {
    if ($h -eq [IntPtr]::Zero) { return "" }
    try {
        $el = [System.Windows.Automation.AutomationElement]::FromHandle($h)
        $condT = New-Object System.Windows.Automation.PropertyCondition(
            [System.Windows.Automation.AutomationElement]::IsTextPatternAvailableProperty, $true)
        $textEls = $el.FindAll([System.Windows.Automation.TreeScope]::Descendants, $condT)
        $sb = ""
        foreach ($te in $textEls) {
            $t = $te.GetCurrentPattern([System.Windows.Automation.TextPattern]::Pattern).DocumentRange.GetText(20000)
            $sb += " " + $t
        }
        return $sb
    } catch { return "" }
}

function Has-Text($hwnd, $pattern) {
    foreach ($try in 1..6) {
        if ((Read-Window-Text $hwnd) -like $pattern) { return $true }
        Start-Sleep -Milliseconds 600
    }
    return $false
}

# WS_EX_TOOLWINDOW (0x80) = skipped in the taskbar. A taskbar-visible window
# must not carry it while shown.
function Test-Taskbar($h) {
    if ($h -eq [IntPtr]::Zero) { return $false }
    return (([Win]::GetWindowLong($h, -20) -band 0x80) -eq 0)
}

# Is the FIRST button matching $pattern enabled (not disabled)?
function Test-Button-Enabled($hwnd, $pattern) {
    if ($hwnd -eq [IntPtr]::Zero) { return $false }
    foreach ($try in 1..6) {
        try {
            $el = [System.Windows.Automation.AutomationElement]::FromHandle($hwnd)
            $all = $el.FindAll([System.Windows.Automation.TreeScope]::Descendants,
                [System.Windows.Automation.Condition]::TrueCondition)
            foreach ($e in $all) {
                if ($e.Current.ControlType.ProgrammaticName -eq "ControlType.Button" -and
                    $e.Current.Name -like $pattern) { return $e.Current.IsEnabled }
            }
        } catch { }
        Start-Sleep -Milliseconds 600
    }
    return $false
}

function Read-Notepad($np) {
    try {
        $npEl = [System.Windows.Automation.AutomationElement]::FromHandle($np.MainWindowHandle)
        $condT = New-Object System.Windows.Automation.PropertyCondition(
            [System.Windows.Automation.AutomationElement]::IsTextPatternAvailableProperty, $true)
        $textEls = $npEl.FindAll([System.Windows.Automation.TreeScope]::Descendants, $condT)
        foreach ($te in $textEls) {
            $t = $te.GetCurrentPattern([System.Windows.Automation.TextPattern]::Pattern).DocumentRange.GetText(20000).Trim()
            if ($t) { return $t }
        }
    } catch { return "(uia failed)" }
    return ""
}

function Get-Z($title) {
    $p = Get-Process zwriter -ErrorAction SilentlyContinue | Select-Object -First 1
    if (-not $p) { return [IntPtr]::Zero }
    return [Win]::FindExact([uint32]$p.Id, $title)
}

function Wait-Window($title, $seconds) {
    foreach ($i in 1..([int]($seconds * 2.5))) {
        $h = Get-Z $title
        if ($h -ne [IntPtr]::Zero) { return $h }
        Start-Sleep -Milliseconds 400
    }
    return [IntPtr]::Zero
}

# Diagnostic: list every visible top-level window of the app process.
function Dump-Windows($when) {
    $p = Get-Process zwriter -ErrorAction SilentlyContinue | Select-Object -First 1
    if (-not $p) { return }
    $cb = [Win+EnumProc]{ param($h, $l)
        $wpid = 0
        [Win]::GetWindowThreadProcessId($h, [ref]$wpid) | Out-Null
        if ($wpid -eq [uint32]$p.Id -and [Win]::IsWindowVisible($h)) {
            $sb = New-Object System.Text.StringBuilder 256
            [Win]::GetWindowText($h, $sb, 256) | Out-Null
            Write-Host ("  [win] hwnd={0} title='{1}'" -f $h, $sb.ToString())
        }
        return $true
    }
    Write-Host "[smoke] visible app windows $when :"
    [Win]::EnumWindows($cb, [IntPtr]::Zero) | Out-Null
}

# Type a CLEAN document: clear everything, one fresh sentence, select it.
function New-Selection($text) {
    $np = Get-Process notepad -ErrorAction SilentlyContinue |
        Where-Object { $_.MainWindowHandle -ne [IntPtr]::Zero } | Select-Object -First 1
    if (-not $np) { throw "notepad is gone" }
    foreach ($attempt in 1..3) {
        Show-Above $np.MainWindowHandle
        Click-Center $np.MainWindowHandle
        if ([Win]::GetForegroundWindow() -eq $np.MainWindowHandle) {
            Start-Sleep -Milliseconds 200
            [System.Windows.Forms.SendKeys]::SendWait("^a")   # select all
            [System.Windows.Forms.SendKeys]::SendWait("{DEL}")  # clear document
            Start-Sleep -Milliseconds 200
            [System.Windows.Forms.SendKeys]::SendWait($text)
            Start-Sleep -Milliseconds 300
            [System.Windows.Forms.SendKeys]::SendWait("^a")
            Start-Sleep -Milliseconds 300
            return $np
        }
        Start-Sleep -Milliseconds 1000
    }
    Un-Top $np.MainWindowHandle | Out-Null
    throw "could not focus notepad (is someone using the machine?)"
}

function Dismiss-Review() {
    $zw = Get-Z "ZWriter"
    if ($zw -ne [IntPtr]::Zero) {
        Show-Above $zw
        Click-Button $zw "Skip*" | Out-Null
        Un-Top $zw
        Start-Sleep -Milliseconds 500
    }
}

# Capture a clean anchor sentence and open Settings from its review window
# (sets $script:st to the settings hwnd). Settings has no hotkey of its own;
# the review window's Ctrl+, is the only scriptable way in.
function Anchor-Settings($text) {
    $np = New-Selection $text
    [System.Windows.Forms.SendKeys]::SendWait("^%g")
    Un-Top $np.MainWindowHandle
    $h = Wait-Window "ZWriter" 6
    if ($h -eq [IntPtr]::Zero) {
        Step "anchor capture race - retrying once"
        [System.Windows.Forms.SendKeys]::SendWait("^%g")
        $h = Wait-Window "ZWriter" 10
    }
    Show-Above $h
    Click-Title $h
    [System.Windows.Forms.SendKeys]::SendWait("^,")
    $script:st = Wait-Window "ZWriter Settings" 10
    Show-Above $script:st
    Start-Sleep -Milliseconds 800
}

# --------------------------------------------------------------- run

# Force a known state: restart the app with default chords, keeping the
# user's real settings.json backed up for restoration at the end.
$settingsFile = "$env:APPDATA\com.alinawaz.zwriter\settings.json"
$settingsBackup = "$env:TEMP\zwriter-settings-backup.json"
$exePath = (Get-Process zwriter -ErrorAction SilentlyContinue | Select-Object -First 1).Path
if (-not $exePath) { $exePath = "D:\Work\AliNawaz\ZWriter\src-tauri\target\release\zwriter.exe" }
Get-Process zwriter -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 1200
if (Test-Path $settingsFile) { Copy-Item $settingsFile $settingsBackup -Force }
'{ "fixHotkey": "ctrl+alt+g", "quickHotkey": "ctrl+alt+f" }' |
    Out-File -FilePath $settingsFile -Encoding ascii -NoNewline
$appLog = "$env:TEMP\zwriter-smoke-app.log"
Start-Process $exePath -RedirectStandardOutput $appLog -RedirectStandardError "$env:TEMP\zwriter-smoke-app.err.log" | Out-Null
Start-Sleep -Seconds 3

$app = Get-Process zwriter -ErrorAction SilentlyContinue
if (-not $app) { throw "zwriter.exe did not start" }
Step "app restarted with known settings (pid $($app.Id))"

# T1 (informational, not a pass/fail check): tray icon visible in the taskbar
# UIA tree. Win11 parks new icons in the overflow flyout, which is only
# enumerable after a human opens it once - absence here is not a failure.
try {
    $shell = [Win]::FindWindow("Shell_TrayWnd", $null)
    if ($shell -ne [IntPtr]::Zero) {
        $tb = [System.Windows.Automation.AutomationElement]::FromHandle($shell)
        $trayHit = $tb.FindAll([System.Windows.Automation.TreeScope]::Descendants,
            [System.Windows.Automation.Condition]::TrueCondition) |
            Where-Object { $_.Current.Name -like "*ZWriter*" }
        if ($trayHit) { Step "T1 INFO: tray icon found in visible taskbar area" }
        else { Step "T1 INFO: tray icon not in visible taskbar tree (likely Win11 overflow - confirm manually)" }
    } else {
        Step "T1 INFO: Shell_TrayWnd not found"
    }
} catch { Step "T1 INFO: tray check skipped: $_" }

Get-Process notepad -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Start-Sleep -Milliseconds 500
Start-Process "$env:SystemRoot\System32\notepad.exe" | Out-Null
Start-Sleep -Milliseconds 2500

# ---- A. fix hotkey: capture -> review -> Apply -> paste ----
Set-Clipboard -Value "SENTINEL-A"
$np = New-Selection "i beleive thiss is a exampel of bad text"
[System.Windows.Forms.SendKeys]::SendWait("^%g")
Un-Top $np.MainWindowHandle
$zw = Wait-Window "ZWriter" 10
Check "A1 review window appeared (Ctrl+Alt+G)" ($zw -ne [IntPtr]::Zero)
Show-Above $zw
Start-Sleep -Milliseconds 400
Check "A2 Apply button clicked" (Click-Button $zw "Apply*")
Un-Top $zw
Start-Sleep -Seconds 3
$clip = Get-Clipboard -Raw
Check "A3 clipboard sentinel restored" ($clip -eq "SENTINEL-A")
$txt = Read-Notepad $np
Step ("doc after A: " + $txt)
Check "A4 notepad has fixed text" ($txt -eq "I believe this is an example of bad text")

# ---- B. quick hotkey: capture -> paste, no window ----
Set-Clipboard -Value "SENTINEL-B"
$np = New-Selection "second beleive exampel for quick mode"
[System.Windows.Forms.SendKeys]::SendWait("^%f")
Un-Top $np.MainWindowHandle
Start-Sleep -Seconds 4
$clip = Get-Clipboard -Raw
Check "B1 clipboard sentinel restored after quick fix" ($clip -eq "SENTINEL-B")
$txt = Read-Notepad $np
Step ("doc after B: " + $txt)
# Harper's fix for "beleive" mid-sentence is "belief" (noun reading), so the
# exact expected output is known and deterministic.
Check "B2 quick fix pasted into notepad" ($txt -eq "Second belief example for quick mode")
$zwB = Get-Z "ZWriter"
if ($zwB -ne [IntPtr]::Zero) { Dump-Windows "at B3" }
Check "B3 no review window opened for quick fix" ($zwB -eq [IntPtr]::Zero)

# ---- E. word picker: taskbar visibility, suggestion chip, inline edit ----
$np = New-Selection "i beleive thiss is a exampel of bad text"
[System.Windows.Forms.SendKeys]::SendWait("^%g")
Un-Top $np.MainWindowHandle
$zw = Wait-Window "ZWriter" 10
Check "E1 review window for picker" ($zw -ne [IntPtr]::Zero)
Show-Above $zw
Start-Sleep -Milliseconds 600
Check "E2 review window is taskbar-visible (no WS_EX_TOOLWINDOW)" (Test-Taskbar $zw)
Check "E3 error word 'thiss' clicked" (Click-Button $zw "word: thiss")
Check "E4 suggestion chip offered" (Has-Button $zw "suggestion: this")
Check "E5 suggestion chip clicked" (Click-Button $zw "suggestion: this")
Start-Sleep -Milliseconds 800
$txt = Read-Window-Text $zw
Check "E6 original re-rendered without 'thiss'" ($txt -and $txt -notmatch "thiss")
Check "E7 clean word 'bad' clicked" (Click-Button $zw "word: bad")
Start-Sleep -Milliseconds 400
Check "E8 word editor focused" (Click-Edit $zw "Edit word")
[System.Windows.Forms.SendKeys]::SendWait("^a")
Start-Sleep -Milliseconds 200
[System.Windows.Forms.SendKeys]::SendWait("terrible")
Start-Sleep -Milliseconds 300
Check "E9 Replace clicked" (Click-Button $zw "Replace word")
Start-Sleep -Milliseconds 800
$txt = Read-Window-Text $zw
Check "E10 edited word visible in window" ($txt -match "terrible")
Check "E11 Apply clicked" (Click-Button $zw "Apply*")
Un-Top $zw
Start-Sleep -Seconds 3
$clip = Get-Clipboard -Raw
Check "E12 clipboard sentinel restored after picker Apply" ($clip -eq "SENTINEL-B")
$txt = Read-Notepad $np
Step ("doc after E: " + $txt)
Check "E13 notepad has the EDITED fixed text" ($txt -eq "I believe this is an example of terrible text")

# ---- F. smart quotes: curly apostrophes must not block grammar lints ----
# Smart-quote autocorrect (U+2019) used to make the Agreement linter miss
# "She don't like apples." entirely. Paste a curly-apostrophe sentence (it
# cannot be TYPED via SendKeys), capture, and require the grammar fix.
$np = New-Selection ""
Set-Clipboard -Value ("She don" + [char]0x2019 + "t like apples.")
[System.Windows.Forms.SendKeys]::SendWait("^v")
Start-Sleep -Milliseconds 600
[System.Windows.Forms.SendKeys]::SendWait("^a")
Set-Clipboard -Value "SENTINEL-F"
[System.Windows.Forms.SendKeys]::SendWait("^%g")
Un-Top $np.MainWindowHandle
$zw = Wait-Window "ZWriter" 10
Check "F1 review window for curly-apostrophe capture" ($zw -ne [IntPtr]::Zero)
Show-Above $zw
Start-Sleep -Milliseconds 500
# the word token carries the CURLY apostrophe in its aria-label
Check "F2a flagged word clicked" (Click-Button $zw "word: don*")
Check "F2 Agreement suggestion chip offered" (Has-Button $zw "suggestion: doesn't")
[System.Windows.Forms.SendKeys]::SendWait("{ESC}")   # close popover; its backdrop would eat the Apply click
Start-Sleep -Milliseconds 300
$txt = Read-Window-Text $zw
Check "F3 fixed pane contains doesn't" ($txt -match "doesn.t")
Check "F4 Apply clicked" (Click-Button $zw "Apply*")
Un-Top $zw
Start-Sleep -Seconds 3
$clip = Get-Clipboard -Raw
Check "F5 clipboard sentinel restored" ($clip -eq "SENTINEL-F")
$txt = Read-Notepad $np
Step ("doc after F: " + $txt)
Check "F6 notepad has the grammar fix" ($txt -eq "She doesn't like apples.")

# ---- G. multi-line paragraph: EVERY line must be fixed, not just the last ----
$np = New-Selection ""
Set-Clipboard -Value "She don't like apples.`r`nHe didn't went home.`r`ni beleive thiss is a exampel of bad gramar."
[System.Windows.Forms.SendKeys]::SendWait("^v")
Start-Sleep -Milliseconds 600
[System.Windows.Forms.SendKeys]::SendWait("^a")
[System.Windows.Forms.SendKeys]::SendWait("^%g")
Un-Top $np.MainWindowHandle
$zw = Wait-Window "ZWriter" 6
if ($zw -eq [IntPtr]::Zero) {
    Step "G first press did not open review (capture race) - retrying once"
    [System.Windows.Forms.SendKeys]::SendWait("^%g")
    $zw = Wait-Window "ZWriter" 10
}
Check "G1 review window for multi-line capture" ($zw -ne [IntPtr]::Zero)
Show-Above $zw
Start-Sleep -Milliseconds 500
$txt = Read-Window-Text $zw
Check "G2 all three lines linted" (($txt -match "doesn.t") -and ($txt -match "go home") -and ($txt -match "example") -and ($txt -match "grammar"))
Check "G3 Apply clicked" (Click-Button $zw "Apply*")
Un-Top $zw
Start-Sleep -Seconds 3
$txt = (Read-Notepad $np) -replace "`r`n", "`n"
$txt = $txt -replace "`r", "`n"
$txt = $txt -replace "`n+", "`n"
Step ("doc after G: " + ($txt -replace "`n", " | "))
Check "G4 notepad has ALL lines fixed" ($txt.Trim() -eq "She doesn't like apples.`nHe didn't go home.`nI believe this is an example of bad grammar.")

# ---- H. user edit on a CLEAN capture: Apply must stay usable ----
$np = New-Selection "Hello world this is clean text."
[System.Windows.Forms.SendKeys]::SendWait("^%g")
Un-Top $np.MainWindowHandle
$zw = Wait-Window "ZWriter" 6
if ($zw -eq [IntPtr]::Zero) {
    Step "H first press did not open review (capture race) - retrying once"
    [System.Windows.Forms.SendKeys]::SendWait("^%g")
    $zw = Wait-Window "ZWriter" 10
}
Check "H1 review window for clean capture" ($zw -ne [IntPtr]::Zero)
Show-Above $zw
Start-Sleep -Milliseconds 500
Check "H2 Apply disabled before any edit" (-not (Test-Button-Enabled $zw "Apply*"))
Check "H3 clean word 'world' clicked" (Click-Button $zw "word: world")
Start-Sleep -Milliseconds 400
Check "H4 word editor focused" (Click-Edit $zw "Edit word")
[System.Windows.Forms.SendKeys]::SendWait("^a")
Start-Sleep -Milliseconds 200
[System.Windows.Forms.SendKeys]::SendWait("planet")
Start-Sleep -Milliseconds 300
Check "H5 Replace clicked" (Click-Button $zw "Replace word")
Start-Sleep -Milliseconds 800
Check "H6 Apply ENABLED after user edit" (Test-Button-Enabled $zw "Apply*")
Check "H7 Apply clicked" (Click-Button $zw "Apply*")
Un-Top $zw
Start-Sleep -Seconds 3
$txt = Read-Notepad $np
Step ("doc after H: " + $txt)
Check "H8 notepad has the user's edit" ($txt -eq "Hello planet this is clean text.")

# ---- I. custom dictionary: flagged word -> whitelist -> unflagged ----
# PASTE the text (F/G pattern): Win11 Notepad autocorrects TYPED text before
# the app can capture it - it rewrote "mistkae" -> "mistake" and "beleive" ->
# "believe" mid-typing, while pasted text is untouched. "thiss" + "mistkae"
# both start flagged (2 issues).
$np = New-Selection ""
Set-Clipboard -Value "the thiss mistkae stays here."
[System.Windows.Forms.SendKeys]::SendWait("^v")
Start-Sleep -Milliseconds 600
[System.Windows.Forms.SendKeys]::SendWait("^a")
[System.Windows.Forms.SendKeys]::SendWait("^%g")
Un-Top $np.MainWindowHandle
$zw = Wait-Window "ZWriter" 6
if ($zw -eq [IntPtr]::Zero) {
    Step "I first press did not open review (capture race) - retrying once"
    [System.Windows.Forms.SendKeys]::SendWait("^%g")
    $zw = Wait-Window "ZWriter" 10
}
Check "I1 review window for custom-dict capture" ($zw -ne [IntPtr]::Zero)
Check "I1b both words flagged (2 issues)" (Has-Text $zw "*2 issues*")
Show-Above $zw
Start-Sleep -Milliseconds 500
Check "I2 flagged word 'mistkae' clicked" (Click-Button $zw "word: mistkae")
Check "I3 Add-to-dictionary button offered" (Has-Button $zw "Add to dictionary")
Check "I4 Add-to-dictionary clicked" (Click-Button $zw "Add to dictionary")
Start-Sleep -Milliseconds 1500   # command -> engine rebuild -> re-lint event
Check "I5 mistkae unflagged (2 -> 1 issue)" (Has-Text $zw "*1 issue*")
Show-Above $zw
Click-Title $zw
[System.Windows.Forms.SendKeys]::SendWait("^,")
$st = Wait-Window "ZWriter Settings" 10
Check "I6 settings opened for dictionary management" ($st -ne [IntPtr]::Zero)
Show-Above $st
Start-Sleep -Milliseconds 800
Check "I7 picker-added word listed in settings" (Has-Button $st "Remove word mistkae")
Check "I8 dictionary word input clicked" (Click-Edit $st "New dictionary word")
[System.Windows.Forms.SendKeys]::SendWait("thiss")
Start-Sleep -Milliseconds 300
Check "I9 Add word clicked" (Click-Button $st "Add word")
Start-Sleep -Milliseconds 1500
Check "I10 second word listed" (Has-Button $st "Remove word thiss")
Click-Button $st "Close settings*" | Out-Null
Un-Top $st
Start-Sleep -Milliseconds 600
Check "I11 thiss unflagged too (1 -> 0 issues)" (Has-Text $zw "*0 issue*")
# Removing the word must re-enable flagging, live in the open review window.
Show-Above $zw
Click-Title $zw
[System.Windows.Forms.SendKeys]::SendWait("^,")
$st = Wait-Window "ZWriter Settings" 10
Show-Above $st
Start-Sleep -Milliseconds 800
Check "I12 remove word mistkae clicked" (Click-Button $st "Remove word mistkae")
Start-Sleep -Milliseconds 1500
Click-Button $st "Close settings*" | Out-Null
Un-Top $st
Start-Sleep -Milliseconds 600
Check "I13 mistkae flagged again (0 -> 1 issue)" (Has-Text $zw "*1 issue*")
Dismiss-Review

# ---- J. teach abbreviations, ignored words + confirm checkbox in Settings ----
$np = New-Selection "anchor clean text for settings j"
[System.Windows.Forms.SendKeys]::SendWait("^%g")
Un-Top $np.MainWindowHandle
$zw = Wait-Window "ZWriter" 6
if ($zw -eq [IntPtr]::Zero) {
    Step "J first press did not open review (capture race) - retrying once"
    [System.Windows.Forms.SendKeys]::SendWait("^%g")
    $zw = Wait-Window "ZWriter" 10
}
Show-Above $zw
Click-Title $zw
[System.Windows.Forms.SendKeys]::SendWait("^,")
$st = Wait-Window "ZWriter Settings" 10
Check "J1 settings opened for teaching" ($st -ne [IntPtr]::Zero)
Show-Above $st
Start-Sleep -Milliseconds 800
Check "J2 trigger input clicked" (Click-Edit $st "New abbreviation trigger")
[System.Windows.Forms.SendKeys]::SendWait("sc2")
Start-Sleep -Milliseconds 200
Check "J3 expansion input clicked" (Click-Edit $st "New abbreviation expansion")
[System.Windows.Forms.SendKeys]::SendWait("StarCraft 2")
Start-Sleep -Milliseconds 200
Check "J4 Add abbreviation clicked" (Click-Button $st "Add abbreviation")
Start-Sleep -Milliseconds 800
Check "J5 sc2 chip listed" (Has-Button $st "Remove abbreviation sc2")
Check "J6 trigger input clicked again" (Click-Edit $st "New abbreviation trigger")
[System.Windows.Forms.SendKeys]::SendWait("cs2")
Start-Sleep -Milliseconds 200
Check "J7 expansion input clicked again" (Click-Edit $st "New abbreviation expansion")
[System.Windows.Forms.SendKeys]::SendWait("Counter-Strike 2")
Start-Sleep -Milliseconds 200
Click-Button $st "Add abbreviation" | Out-Null
Start-Sleep -Milliseconds 800
Check "J8 cs2 chip listed" (Has-Button $st "Remove abbreviation cs2")
Check "J9 ignored word input clicked" (Click-Edit $st "New ignored word")
[System.Windows.Forms.SendKeys]::SendWait("skool")
Start-Sleep -Milliseconds 200
Check "J10 Add ignored word clicked" (Click-Button $st "Add ignored word")
Start-Sleep -Milliseconds 800
Check "J11 skool ignore chip listed" (Has-Button $st "Remove ignored word skool")
Check "J12 skip-unsure checkbox to ON" (Set-Checkbox $st "Skip unsure abbreviations" "On")
Start-Sleep -Milliseconds 400
Check "J13 skip-unsure checkbox back OFF" (Set-Checkbox $st "Skip unsure abbreviations" "Off")
Start-Sleep -Milliseconds 400
Check "J13b auto-apply checkbox to ON" (Set-Checkbox $st "Auto-apply unsure abbreviations" "On")
Start-Sleep -Milliseconds 400
Check "J13c auto-apply checkbox back OFF" (Set-Checkbox $st "Auto-apply unsure abbreviations" "Off")
Start-Sleep -Milliseconds 500
Click-Button $st "Close settings*" | Out-Null
Un-Top $st
Start-Sleep -Milliseconds 600
Dismiss-Review

# ---- K. exact abbreviation through the review path ----
$np = New-Selection ""
Set-Clipboard -Value "is there a tool for sc2 here"
[System.Windows.Forms.SendKeys]::SendWait("^v")
Start-Sleep -Milliseconds 600
[System.Windows.Forms.SendKeys]::SendWait("^a")
Set-Clipboard -Value "SENTINEL-K"
[System.Windows.Forms.SendKeys]::SendWait("^%g")
Un-Top $np.MainWindowHandle
$zw = Wait-Window "ZWriter" 6
if ($zw -eq [IntPtr]::Zero) {
    Step "K first press did not open review (capture race) - retrying once"
    [System.Windows.Forms.SendKeys]::SendWait("^%g")
    $zw = Wait-Window "ZWriter" 10
}
Check "K1 review window for sc2 capture" ($zw -ne [IntPtr]::Zero)
Show-Above $zw
Start-Sleep -Milliseconds 500
Check "K2 fixed pane shows the expansion" (Has-Text $zw "*StarCraft 2*")
Check "K3 flagged word 'sc2' clicked" (Click-Button $zw "word: sc2")
Check "K4 expansion chip offered" (Has-Button $zw "suggestion: StarCraft 2")
[System.Windows.Forms.SendKeys]::SendWait("{ESC}")
Start-Sleep -Milliseconds 300
Check "K5 Apply clicked" (Click-Button $zw "Apply*")
Un-Top $zw
Start-Sleep -Seconds 3
$clip = Get-Clipboard -Raw
Check "K6 clipboard sentinel restored" ($clip -eq "SENTINEL-K")
$txt = Read-Notepad $np
Step ("doc after K: " + $txt)
Check "K7 notepad has the expansion" ($txt -ceq "Is there a tool for StarCraft 2 here")

# ---- L. quick fix with an EXACT abbreviation: instant paste, no window ----
Set-Clipboard -Value "SENTINEL-L"
$np = New-Selection "the sc2 game"
[System.Windows.Forms.SendKeys]::SendWait("^%f")
Un-Top $np.MainWindowHandle
Start-Sleep -Seconds 4
$clip = Get-Clipboard -Raw
Check "L1 clipboard sentinel restored" ($clip -eq "SENTINEL-L")
$txt = Read-Notepad $np
Step ("doc after L: " + $txt)
Check "L2 quick pasted the expansion" ($txt -ceq "the StarCraft 2 game")
Check "L3 no review window for exact quick fix" ((Get-Z "ZWriter") -eq [IntPtr]::Zero)

# ---- M. quick fix x unsure-abbreviation checkboxes: the 2x2 matrix ----
# checkbox1 "Skip unsure abbreviations" (no window), checkbox2 "Auto-apply
# unsure abbreviations". Every toggle is VERIFIED against UIA ToggleState.
# rows (C1,C2): (off,off)=window+chip [default]; (on,off)=word left as typed;
# (on,on)=apply+paste, no window; (off,on)=window with the guess pre-applied.

Anchor-Settings "anchor m row two settings"
Check "M0 settings opened for row-2 toggles" ($st -ne [IntPtr]::Zero)
Check "M1 checkbox1 to ON" (Set-Checkbox $st "Skip unsure abbreviations" "On")
Check "M2 checkbox2 stays OFF" (Set-Checkbox $st "Auto-apply unsure abbreviations" "Off")
Click-Button $st "Close settings*" | Out-Null
Un-Top $st
Start-Sleep -Milliseconds 400
Dismiss-Review
# row 2 (on,off): unsure guess -> word untouched, no window, clipboard kept
Set-Clipboard -Value "SENTINEL-M2"
$np = New-Selection "the se2 id"
[System.Windows.Forms.SendKeys]::SendWait("^%f")
Un-Top $np.MainWindowHandle
Start-Sleep -Seconds 4
$clip = Get-Clipboard -Raw
Check "M3 row2: clipboard sentinel restored" ($clip -eq "SENTINEL-M2")
$txt = Read-Notepad $np
Step ("doc after M row2: " + $txt)
Check "M4 row2: unsure word left as typed" ($txt -ceq "the se2 id")
Check "M5 row2: no review window" ((Get-Z "ZWriter") -eq [IntPtr]::Zero)
# ambiguous beats checkbox1: cc2 is one edit from sc2 AND cs2 -> window
$np = New-Selection "the cc2 id"
[System.Windows.Forms.SendKeys]::SendWait("^%f")
Un-Top $np.MainWindowHandle
$zw = Wait-Window "ZWriter" 6
if ($zw -eq [IntPtr]::Zero) {
    Step "Mc capture race - retrying once"
    [System.Windows.Forms.SendKeys]::SendWait("^%f")
    $zw = Wait-Window "ZWriter" 10
}
Check "M6 ambiguous opens window even with checkbox1" ($zw -ne [IntPtr]::Zero)
Show-Above $zw
Start-Sleep -Milliseconds 500
Check "M7 ambiguous word clicked" (Click-Button $zw "word: cc2")
Check "M8 first candidate offered" (Has-Button $zw "suggestion: StarCraft 2")
Check "M9 second candidate offered" (Has-Button $zw "suggestion: Counter-Strike 2")
[System.Windows.Forms.SendKeys]::SendWait("{ESC}")
Start-Sleep -Milliseconds 300
Dismiss-Review
# row 1 (on,on): unsure guess -> applied + pasted, no window
Anchor-Settings "anchor m row one settings"
Check "M10 checkbox2 to ON" (Set-Checkbox $st "Auto-apply unsure abbreviations" "On")
Click-Button $st "Close settings*" | Out-Null
Un-Top $st
Start-Sleep -Milliseconds 400
Dismiss-Review
Set-Clipboard -Value "SENTINEL-M1"
$np = New-Selection "the se2 id"
[System.Windows.Forms.SendKeys]::SendWait("^%f")
Un-Top $np.MainWindowHandle
Start-Sleep -Seconds 4
$clip = Get-Clipboard -Raw
Check "M11 row1: clipboard sentinel restored" ($clip -eq "SENTINEL-M1")
$txt = Read-Notepad $np
Step ("doc after M row1: " + $txt)
Check "M12 row1: guess applied + pasted" ($txt -ceq "the StarCraft 2 id")
Check "M13 row1: no review window" ((Get-Z "ZWriter") -eq [IntPtr]::Zero)
# row 3 (off,on): window opens, guess pre-applied
Anchor-Settings "anchor m row three settings"
Check "M14 checkbox1 to OFF" (Set-Checkbox $st "Skip unsure abbreviations" "Off")
Click-Button $st "Close settings*" | Out-Null
Un-Top $st
Start-Sleep -Milliseconds 400
Dismiss-Review
$np = New-Selection "the se2 id"
[System.Windows.Forms.SendKeys]::SendWait("^%f")
Un-Top $np.MainWindowHandle
$zw = Wait-Window "ZWriter" 6
if ($zw -eq [IntPtr]::Zero) {
    Step "Me capture race - retrying once"
    [System.Windows.Forms.SendKeys]::SendWait("^%f")
    $zw = Wait-Window "ZWriter" 10
}
Check "M15 row3: window opens" ($zw -ne [IntPtr]::Zero)
Show-Above $zw
Start-Sleep -Milliseconds 500
Check "M16 row3: guess pre-applied in fixed pane" (Has-Text $zw "*StarCraft 2*")
Dismiss-Review
# row 4 (off,off): defaults - window, chip, NOT applied
Anchor-Settings "anchor m row four settings"
Check "M17 checkbox2 to OFF (defaults)" (Set-Checkbox $st "Auto-apply unsure abbreviations" "Off")
Click-Button $st "Close settings*" | Out-Null
Un-Top $st
Start-Sleep -Milliseconds 400
Dismiss-Review
$np = New-Selection "the se2 id"
[System.Windows.Forms.SendKeys]::SendWait("^%f")
Un-Top $np.MainWindowHandle
$zw = Wait-Window "ZWriter" 6
if ($zw -eq [IntPtr]::Zero) {
    Step "Mf capture race - retrying once"
    [System.Windows.Forms.SendKeys]::SendWait("^%f")
    $zw = Wait-Window "ZWriter" 10
}
Check "M18 row4: window opens (default)" ($zw -ne [IntPtr]::Zero)
Show-Above $zw
Start-Sleep -Milliseconds 500
$wtxt = Read-Window-Text $zw
Check "M19 row4: guess NOT applied" ($wtxt -and $wtxt -notmatch "StarCraft")
Check "M20 row4: guessed word clicked" (Click-Button $zw "word: se2")
Check "M21 row4: chip offered" (Has-Button $zw "suggestion: StarCraft 2")
[System.Windows.Forms.SendKeys]::SendWait("{ESC}")
Start-Sleep -Milliseconds 300
Dismiss-Review

# ---- N. teach from the word popover: Add-as-abbreviation + Ignore word ----
$np = New-Selection ""
Set-Clipboard -Value "the se2 thing"
[System.Windows.Forms.SendKeys]::SendWait("^v")
Start-Sleep -Milliseconds 600
[System.Windows.Forms.SendKeys]::SendWait("^a")
[System.Windows.Forms.SendKeys]::SendWait("^%g")
Un-Top $np.MainWindowHandle
$zw = Wait-Window "ZWriter" 6
if ($zw -eq [IntPtr]::Zero) {
    Step "N first press did not open review (capture race) - retrying once"
    [System.Windows.Forms.SendKeys]::SendWait("^%g")
    $zw = Wait-Window "ZWriter" 10
}
Check "N1 review window for popover teach" ($zw -ne [IntPtr]::Zero)
Show-Above $zw
Start-Sleep -Milliseconds 500
Check "N2 guessed word se2 clicked" (Click-Button $zw "word: se2")
Check "N3 Add-as-abbreviation button offered" (Has-Button $zw "Add as abbreviation")
Check "N3b Add-as-abbreviation stays enabled before typing" (Test-Button-Enabled $zw "Add as abbreviation")
Check "N4 word editor focused" (Click-Edit $zw "Edit word")
[System.Windows.Forms.SendKeys]::SendWait("^a")
Start-Sleep -Milliseconds 200
[System.Windows.Forms.SendKeys]::SendWait("Sea Extra 2")
Start-Sleep -Milliseconds 300
Check "N5 Add-as-abbreviation clicked" (Click-Button $zw "Add as abbreviation")
Start-Sleep -Milliseconds 1500
Check "N6 fixed pane shows the taught expansion" (Has-Text $zw "*Sea Extra 2*")
Dismiss-Review
# Ignore-word button: a flagged word, ignored live from the popover.
# "mistkae" on purpose: "thiss" is still in the custom dictionary from
# section I, so it arrives pre-whitelisted (0 lints) and proves nothing.
$np = New-Selection ""
Set-Clipboard -Value "the mistkae thing"
[System.Windows.Forms.SendKeys]::SendWait("^v")
Start-Sleep -Milliseconds 600
[System.Windows.Forms.SendKeys]::SendWait("^a")
[System.Windows.Forms.SendKeys]::SendWait("^%g")
Un-Top $np.MainWindowHandle
$zw = Wait-Window "ZWriter" 6
if ($zw -eq [IntPtr]::Zero) {
    Step "N-ignore first press did not open review (capture race) - retrying once"
    [System.Windows.Forms.SendKeys]::SendWait("^%g")
    $zw = Wait-Window "ZWriter" 10
}
Check "N7 review window for ignore flow" ($zw -ne [IntPtr]::Zero)
Check "N8 word flagged before ignore (1 issue)" (Has-Text $zw "*1 issue*")
Show-Above $zw
Start-Sleep -Milliseconds 300
Check "N9 flagged word clicked" (Click-Button $zw "word: mistkae")
Check "N10 Ignore-word button offered" (Has-Button $zw "Ignore word")
Check "N11 Ignore-word clicked" (Click-Button $zw "Ignore word")
Start-Sleep -Milliseconds 1500
Check "N12 mistkae unflagged live (1 -> 0 issues)" (Has-Text $zw "*0 issue*")
Dismiss-Review

# ---- C. rebind fix hotkey Ctrl+Alt+G -> Ctrl+Alt+J via Settings ----
$np = New-Selection "third beleive exampel rebind test"
[System.Windows.Forms.SendKeys]::SendWait("^%g")
Un-Top $np.MainWindowHandle
$zw = Wait-Window "ZWriter" 10
Check "C1 review window before rebind" ($zw -ne [IntPtr]::Zero)
Show-Above $zw
Click-Title $zw
[System.Windows.Forms.SendKeys]::SendWait("^,")
$st = Wait-Window "ZWriter Settings" 10
Check "C2 settings window opened (Ctrl+,)" ($st -ne [IntPtr]::Zero)
Show-Above $st
Start-Sleep -Milliseconds 800
Check "C3 fix-hotkey field clicked" (Click-Button $st "*Alt + G*")
Start-Sleep -Milliseconds 400
[System.Windows.Forms.SendKeys]::SendWait("^%j")
Start-Sleep -Milliseconds 1200
if (-not (Has-Button $st "*Alt + J*")) {
    # The recorded keystroke can go astray on a machine in active use.
    Step "C4 recording did not take (keystroke race) - retrying once"
    Click-Button $st "*Alt + G*" | Out-Null
    Start-Sleep -Milliseconds 400
    [System.Windows.Forms.SendKeys]::SendWait("^%j")
    Start-Sleep -Milliseconds 1200
}
Check "C4 settings shows new chord Ctrl+Alt+J" (Has-Button $st "*Alt + J*")
Click-Button $st "Close settings*" | Out-Null
Un-Top $st
Start-Sleep -Milliseconds 400
Dismiss-Review    # close the C1 review window still open behind settings
$np = New-Selection "fourth beleive exampel old chord"
Set-Clipboard -Value "SENTINEL-C5"
[System.Windows.Forms.SendKeys]::SendWait("^%g")
Un-Top $np.MainWindowHandle
Start-Sleep -Seconds 3
$clipC5 = Get-Clipboard -Raw
if ((Get-Z "ZWriter") -ne [IntPtr]::Zero -or $clipC5 -ne "SENTINEL-C5") {
    Dump-Windows "at C5"
    Step ("C5 clipboard was: '" + $clipC5 + "'")
}
Check "C5 old chord Ctrl+Alt+G is dead" ((Get-Z "ZWriter") -eq [IntPtr]::Zero -and $clipC5 -eq "SENTINEL-C5")
$np = New-Selection "fifth beleive exampel new chord"
[System.Windows.Forms.SendKeys]::SendWait("^%j")
Un-Top $np.MainWindowHandle
$zw = Wait-Window "ZWriter" 6
if ($zw -eq [IntPtr]::Zero) {
    # The global hotkey fires regardless of focus, but the injected Ctrl+C
    # needs the target still foreground; on a machine in active use the
    # capture can lose that race. The chord itself is verified in the log.
    Step "C6 first press did not open review (capture race) - retrying once"
    [System.Windows.Forms.SendKeys]::SendWait("^%j")
    $zw = Wait-Window "ZWriter" 10
}
Check "C6 new chord Ctrl+Alt+J opens review" ($zw -ne [IntPtr]::Zero)
Show-Above $zw
Start-Sleep -Milliseconds 500
# The review window lives across rebinds; its hotkey hint text must follow.
Check "C7 review hint text shows the NEW chord" (Has-Text $zw "*Ctrl + Alt + J*")
# NOTE: no Dismiss-Review here - section D opens Settings from THIS window.

# ---- D. rebind back to Ctrl+Alt+G and verify ----
Show-Above $zw
Click-Title $zw
[System.Windows.Forms.SendKeys]::SendWait("^,")
$st = Wait-Window "ZWriter Settings" 10
Check "D1 settings reopened" ($st -ne [IntPtr]::Zero)
Show-Above $st
Start-Sleep -Milliseconds 800
Check "D2 fix-hotkey field clicked" (Click-Button $st "*Alt + J*")
Start-Sleep -Milliseconds 400
[System.Windows.Forms.SendKeys]::SendWait("^%g")
Start-Sleep -Milliseconds 1200
if (-not (Has-Button $st "*Alt + G*")) {
    Step "D3 recording did not take (keystroke race) - retrying once"
    Click-Button $st "*Alt + J*" | Out-Null
    Start-Sleep -Milliseconds 400
    [System.Windows.Forms.SendKeys]::SendWait("^%g")
    Start-Sleep -Milliseconds 1200
}
Check "D3 settings shows Ctrl+Alt+G again" (Has-Button $st "*Alt + G*")
Click-Button $st "Close settings*" | Out-Null
Un-Top $st
Start-Sleep -Milliseconds 400
Dismiss-Review
$np = New-Selection "sixth beleive exampel restore check"
[System.Windows.Forms.SendKeys]::SendWait("^%g")
Un-Top $np.MainWindowHandle
$zw = Wait-Window "ZWriter" 6
if ($zw -eq [IntPtr]::Zero) {
    Step "D4 first press did not open review (capture race) - retrying once"
    [System.Windows.Forms.SendKeys]::SendWait("^%g")
    $zw = Wait-Window "ZWriter" 10
}
Check "D4 Ctrl+Alt+G works again" ($zw -ne [IntPtr]::Zero)
Show-Above $zw
Dismiss-Review

# ---- summary ----
Step "app log:"
Get-Content $appLog -ErrorAction SilentlyContinue | ForEach-Object { Write-Host "  $_" }

# Log-derived invariants (independent of desktop focus races):
# 20 review-path fixes (A, E, F, G, H, I, J-anchor, K, 4x M-anchors, Mc, Me,
# Mf, N, N-ignore, C1, C6, D4); 9 pastes (A, B, E, F, G, H, K, L, M-row1);
# 0 failed captures; 0 clean-text shortcuts.
$logText = Get-Content $appLog -Raw -ErrorAction SilentlyContinue
# "no text captured" is eprintln (stderr) - check both redirects, or the
# invariant silently passes exactly when a capture fails.
$errText = Get-Content "$env:TEMP\zwriter-smoke-app.err.log" -Raw -ErrorAction SilentlyContinue
$fixReady = ([regex]::Matches($logText, "fix ready")).Count
$pasted = ([regex]::Matches($logText, "pasted fix")).Count
$noCapture = ([regex]::Matches($logText + " " + $errText, "no text captured")).Count
$clean = ([regex]::Matches($logText, "already clean")).Count
Check "L1 fix-ready count = 20" ($fixReady -eq 20)
Check "L2 pasted count = 9" ($pasted -eq 9)
Check "L3 no failed captures" ($noCapture -eq 0)
Check "L4 no unexpected clean-skips" ($clean -eq 0)

$pass = 0; $fail = 0
for ($i = 0; $i -lt $results.Count; $i += 2) {
    if ($results[$i + 1]) { $pass++ } else { $fail++ }
}
Write-Host ("SMOKE-SUMMARY: {0}/{1} checks passed" -f $pass, ($pass + $fail))

# Restore the user's own settings and restart the app on them.
Get-Process zwriter -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 1200
if (Test-Path $settingsBackup) {
    Move-Item $settingsBackup $settingsFile -Force
    Write-Host "[smoke] restored user settings"
}
Start-Process $exePath | Out-Null
Write-Host "[smoke] app relaunched with user settings"
if ($fail -gt 0) { exit 1 }
