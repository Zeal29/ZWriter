//! Win32 keyboard injection + foreground-window choreography (Windows only).
//!
//! HWNDs cross thread/state boundaries as `isize` (the windows-rs HWND is a
//! raw pointer, hence !Send); convert at the Win32 call site.

#![cfg(windows)]

use std::mem::size_of;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::HWND;
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
    KEYEVENTF_KEYUP, VK_CONTROL, VK_MENU, VK_SHIFT, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowThreadProcessId, IsWindow, SetForegroundWindow,
};

/// Foreground window as an opaque id, or None when the desktop has focus.
pub fn foreground_window() -> Option<isize> {
    let hwnd = unsafe { GetForegroundWindow() };
    (!hwnd.0.is_null()).then_some(hwnd.0 as isize)
}

/// Ctrl+<key> as four serial events: Ctrl down, key down, key up, Ctrl up.
/// Returns Err when the injection was blocked (0 events inserted).
pub fn send_ctrl_combo(key_vk: u16) -> Result<(), String> {
    let inputs = [
        key_event(VK_CONTROL.0 as u16, false),
        key_event(key_vk, false),
        key_event(key_vk, true),
        key_event(VK_CONTROL.0 as u16, true),
    ];
    let sent = unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) };
    if sent as usize == inputs.len() {
        Ok(())
    } else {
        Err(format!(
            "SendInput inserted {sent}/{} events (blocked by another thread or UIPI)",
            inputs.len()
        ))
    }
}

pub fn send_ctrl_c() -> Result<(), String> {
    send_ctrl_combo(b'C' as u16)
}

pub fn send_ctrl_v() -> Result<(), String> {
    send_ctrl_combo(b'V' as u16)
}

/// Wait until the user has physically released the hotkey modifiers, so our
/// injected Ctrl+C is not contaminated into Ctrl+Alt+C. The hotkey handler
/// fires on the G keydown while Ctrl/Alt are still held.
pub fn wait_modifiers_released(deadline: Duration) -> bool {
    let end = Instant::now() + deadline;
    let held = |vk: VIRTUAL_KEY| (unsafe { GetAsyncKeyState(vk.0 as i32) } as u16) & 0x8000 != 0;
    loop {
        if !held(VK_CONTROL) && !held(VK_MENU) && !held(VK_SHIFT) {
            return true;
        }
        if Instant::now() >= end {
            return false;
        }
        std::thread::sleep(Duration::from_millis(15));
    }
}

fn key_event(vk: u16, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(vk),
                wScan: 0,
                dwFlags: if up {
                    KEYEVENTF_KEYUP
                } else {
                    KEYBD_EVENT_FLAGS(0)
                },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

/// Bring a previously-captured window back to the foreground.
///
/// AttachThreadInput shares input state with the current foreground thread so
/// SetForegroundWindow is not demoted to a taskbar flash. SetForegroundWindow
/// itself normally succeeds anyway right after our hotkey ran (the process
/// "received the last input event").
pub fn focus_window(hwnd_id: isize) -> bool {
    let hwnd = HWND(hwnd_id as *mut _);
    if hwnd.0.is_null() || !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
        return false;
    }

    unsafe {
        let fg = GetForegroundWindow();
        if fg == hwnd {
            return true;
        }
        let fg_tid = if fg.0.is_null() {
            0
        } else {
            GetWindowThreadProcessId(fg, None)
        };
        let my_tid = GetCurrentThreadId();
        if fg_tid != 0 && fg_tid != my_tid {
            let _ = AttachThreadInput(my_tid, fg_tid, true);
            let _ = SetForegroundWindow(hwnd);
            let _ = AttachThreadInput(my_tid, fg_tid, false);
        } else {
            let _ = SetForegroundWindow(hwnd);
        }
    }

    // Wait briefly for activation to land before the caller injects keys.
    let deadline = Instant::now() + Duration::from_millis(300);
    while Instant::now() < deadline {
        if unsafe { GetForegroundWindow() } == hwnd {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}
