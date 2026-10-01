# One-off visual check: stage a review window containing all three highlight
# classes (normal error, guessed abbreviation, exact abbreviation) and take
# a screenshot of it. Restores the user's settings + relaunches the app.
$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing

Add-Type @"
using System;
using System.Text;
using System.Runtime.InteropServices;
public class Win2 {
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr l);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern int GetWindowText(IntPtr h, StringBuilder t, int n);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, int x, int y, uint d, UIntPtr e);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
  [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
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

$exe = "D:\Work\AliNawaz\ZWriter\src-tauri\target\release\zwriter.exe"
$cfgDir = "$env:APPDATA\com.alinawaz.zwriter"
$settings = "$cfgDir\settings.json"
$backup = "$cfgDir\settings.json.bak-colorcheck"

function Find-ZW {
    $p = Get-Process zwriter -ErrorAction SilentlyContinue
    if (-not $p) { return [IntPtr]::Zero }
    foreach ($proc in $p) {
        $h = [Win2]::FindExact([uint32]$proc.Id, "ZWriter")
        if ($h -ne [IntPtr]::Zero) { return $h }
    }
    return [IntPtr]::Zero
}

try {
    # 1. test settings: sc2 taught; keep the user's words + hotkeys
    Copy-Item $settings $backup -Force
    $user = Get-Content $backup -Raw | ConvertFrom-Json
    @{ customWords = $user.customWords
       fixHotkey = $user.fixHotkey
       quickHotkey = $user.quickHotkey
       abbreviations = @(@{ trigger = "sc2"; expansion = "StarCraft 2" }) } |
        ConvertTo-Json -Depth 5 | Out-Null
    $json = @{ customWords = $user.customWords
       fixHotkey = $user.fixHotkey
       quickHotkey = $user.quickHotkey
       abbreviations = @(@{ trigger = "sc2"; expansion = "StarCraft 2" }) } | ConvertTo-Json -Depth 5 -Compress
    # PS 5.1 Set-Content -Encoding UTF8 writes a BOM; serde_json rejects
    # BOM'd files - write BOM-less UTF-8.
    [IO.File]::WriteAllText($settings, $json, (New-Object System.Text.UTF8Encoding($false)))

    # 2. restart the app on the test settings, stdout+stderr to files
    Stop-Process -Name zwriter -Force -ErrorAction SilentlyContinue
    Start-Sleep -Seconds 1
    Start-Process -FilePath $exe -RedirectStandardOutput "D:\Work\AliNawaz\ZWriter\zw-app.log" -RedirectStandardError "D:\Work\AliNawaz\ZWriter\zw-app.err.log"
    Start-Sleep -Seconds 6
    Write-Host ("[colorcheck] fixHotkey=" + $user.fixHotkey + " quickHotkey=" + $user.quickHotkey)

    # 3. notepad with a sentence carrying all three classes:
    #    beleive = normal error, sc2 = exact abbr, se2 = guessed abbr
    Start-Process "$env:SystemRoot\System32\notepad.exe" | Out-Null
    Start-Sleep -Seconds 3
    # classic notepad hands off to the Store app; find the real process
    $np = Get-Process notepad -ErrorAction SilentlyContinue |
        Where-Object { $_.MainWindowHandle -ne [IntPtr]::Zero } | Select-Object -First 1
    if (-not $np) { throw "notepad did not start" }
    Set-Clipboard -Value "i beleive sc2 and se2 are games"
    $nph = $np.MainWindowHandle
    $focused = $false
    foreach ($attempt in 1..4) {
        $r = New-Object Win2+RECT
        [Win2]::GetWindowRect($nph, [ref]$r) | Out-Null
        $cx = [int](($r.L + $r.R) / 2); $cy = [int](($r.T + $r.B) / 2)
        [Win2]::SetCursorPos($cx, $cy) | Out-Null
        Start-Sleep -Milliseconds 200
        [Win2]::mouse_event(2, 0, 0, 0, [UIntPtr]::Zero)
        [Win2]::mouse_event(4, 0, 0, 0, [UIntPtr]::Zero)
        Start-Sleep -Milliseconds 400
        if ([Win2]::GetForegroundWindow() -eq $nph) { $focused = $true; break }
        Start-Sleep -Milliseconds 1000
    }
    if (-not $focused) { throw "could not focus notepad" }
    [System.Windows.Forms.SendKeys]::SendWait("^a")
    [System.Windows.Forms.SendKeys]::SendWait("{DEL}")
    Start-Sleep -Milliseconds 200
    [System.Windows.Forms.SendKeys]::SendWait("^v")
    Start-Sleep -Milliseconds 500
    [System.Windows.Forms.SendKeys]::SendWait("^a")
    Start-Sleep -Milliseconds 300

    # 4. the review hotkey (user chord) via keybd_event - SendKeys cannot
    #    send Ctrl+Space
    [Win2]::keybd_event(0x11, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 80
    [Win2]::keybd_event(0x20, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 80
    [Win2]::keybd_event(0x20, 0, 2, [UIntPtr]::Zero)
    [Win2]::keybd_event(0x11, 0, 2, [UIntPtr]::Zero)
    Write-Host ("[colorcheck] notepad fg after click: " + ([Win2]::GetForegroundWindow() -eq $nph))

    # 5. wait for the review window, raise it, screenshot it
    $zw = [IntPtr]::Zero
    foreach ($i in 1..20) {
        Start-Sleep -Milliseconds 500
        $zw = Find-ZW
        if ($zw -ne [IntPtr]::Zero) { break }
    }
    if ($zw -eq [IntPtr]::Zero) {
        Get-Content "D:\Work\AliNawaz\ZWriter\zw-app.log" -Tail 15 -ErrorAction SilentlyContinue
        Get-Content "D:\Work\AliNawaz\ZWriter\zw-app.err.log" -Tail 10 -ErrorAction SilentlyContinue
        throw "review window never appeared"
    }
    [Win2]::SetWindowPos($zw, [IntPtr](-1), 0, 0, 0, 0, 0x13) | Out-Null
    Start-Sleep -Milliseconds 900
    $zr = New-Object Win2+RECT
    [Win2]::GetWindowRect($zw, [ref]$zr) | Out-Null
    $w = $zr.R - $zr.L; $h2 = $zr.B - $zr.T
    $bmp = New-Object System.Drawing.Bitmap($w, $h2)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($zr.L, $zr.T, 0, 0, (New-Object System.Drawing.Size($w, $h2)))
    $bmp.Save("D:\Work\AliNawaz\ZWriter\colors-check.png", [System.Drawing.Imaging.ImageFormat]::Png)
    $g.Dispose(); $bmp.Dispose()
    Write-Host "[colorcheck] saved colors-check.png ($w x $h2)"
    [Win2]::SetWindowPos($zw, [IntPtr](-2), 0, 0, 0, 0, 0x13) | Out-Null
}
finally {
    Stop-Process -Name notepad -Force -ErrorAction SilentlyContinue
    # 6. restore the user's settings and relaunch the app on them
    Stop-Process -Name zwriter -Force -ErrorAction SilentlyContinue
    Start-Sleep -Seconds 1
    Move-Item $backup $settings -Force
    Start-Process -FilePath $exe
    Start-Sleep -Seconds 2
    Write-Host "[colorcheck] settings restored, app relaunched"
}
