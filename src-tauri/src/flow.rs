//! The hotkey pipeline: capture selection via clipboard round-trip, fix it
//! with Harper, and paste it back into the window the user started in.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_clipboard_manager::ClipboardExt;

use crate::engine::{Engine, LintJson};
use crate::wininput;

pub const HISTORY_CAP: usize = 50;
pub const DEFAULT_FIX_HOTKEY: &str = "ctrl+shift+space";
pub const DEFAULT_QUICK_HOTKEY: &str = "ctrl+space";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LintPayload {
    pub lints: Vec<LintJson>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FixReady {
    pub original: String,
    pub fixed: String,
    pub lints: Vec<LintJson>,
    pub fix_ms: u128,
    pub no_change: bool,
    /// The user edited the text in the review window. When true, noChange
    /// only means "the engine found nothing left" — the edited text is still
    /// exactly what the user wants to apply, so Apply must stay enabled.
    pub user_edited: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub ts: u64,
    pub original: String,
    pub fixed: String,
    pub fix_count: usize,
    pub applied: bool,
}

/// One user-taught abbreviation: trigger -> expansion.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Abbreviation {
    pub trigger: String,
    pub expansion: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// Fix with review window (default ctrl+shift+space).
    pub fix_hotkey: String,
    /// Fix and paste immediately, no review (default ctrl+space).
    pub quick_hotkey: String,
    pub autostart: bool,
    /// The user's custom dictionary: these words are never flagged as
    /// spelling errors (engine matches them case-insensitively).
    pub custom_words: Vec<String>,
    /// User-taught trigger -> expansion pairs (engine matches triggers
    /// case-/space-insensitively, one-letter-off as a GUESS).
    pub abbreviations: Vec<Abbreviation>,
    /// Exact tokens never flagged, never guessed, never suggested.
    pub ignored_words: Vec<String>,
    /// Quick fix opens the review window for guessed (one-edit-away)
    /// abbreviations instead of pasting them. Ambiguous guesses always ask.
    pub confirm_guesses: bool,
}

pub struct AppState {
    pub engine: Mutex<Engine>,
    pub pending: Mutex<Option<PendingFix>>,
    pub history: Mutex<VecDeque<HistoryEntry>>,
    pub settings: Mutex<Settings>,
    pub busy: AtomicBool,
}

/// Everything needed to paste the fix back where the user was.
pub struct PendingFix {
    pub original: String,
    pub fixed: String,
    pub lints: Vec<LintJson>,
    pub fix_count: usize,
    /// Set once the user edits the text in the review window (word picker).
    pub user_edited: bool,
    pub target_hwnd: Option<isize>,
    /// Clipboard text before we pressed Ctrl+C (None = non-text/empty).
    pub orig_clipboard: Option<String>,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn default_settings() -> Settings {
    Settings {
        fix_hotkey: DEFAULT_FIX_HOTKEY.to_string(),
        quick_hotkey: DEFAULT_QUICK_HOTKEY.to_string(),
        autostart: false,
        custom_words: Vec::new(),
        abbreviations: Vec::new(),
        ignored_words: Vec::new(),
        confirm_guesses: true,
    }
}

/// "Nothing to do" for the UI's Apply gate and the quick path's silent
/// return: the engine changed nothing AND no abbreviation guess is pending
/// (a pending guess is exactly what the review window is for).
pub fn no_change(fixed: &str, original: &str, lints: &[LintJson]) -> bool {
    fixed == original && !lints.iter().any(|l| l.guessed)
}

/// Push to the history ring buffer (newest first).
pub fn push_history(state: &AppState, entry: HistoryEntry) {
    let mut h = state.history.lock().unwrap();
    h.push_front(entry);
    h.truncate(HISTORY_CAP);
}

/// Full capture pipeline. Runs on its own thread (never the main thread:
/// clipboard reads must not run there, and we poll with sleeps).
/// `quick` = paste the fix immediately, without the review window.
pub fn capture_and_fix(app: AppHandle, quick: bool) {
    let state = app.state::<AppState>();
    if state.busy.swap(true, Ordering::SeqCst) {
        return; // hotkey re-entry while a fix is already in flight
    }
    let _guard = BusyGuard(&state.busy);

    let started = Instant::now();

    // 1. Remember where the user was and what the clipboard held.
    let target_hwnd = wininput::foreground_window();
    let orig_clipboard = app.clipboard().read_text().ok();

    // 2. Synthesize Ctrl+C, then poll until the clipboard changes.
    //    First wait out the user's still-held Ctrl/Alt (hotkey fired on the
    //    G keydown) or the injection becomes Ctrl+Alt+C and copies nothing.
    if !wininput::wait_modifiers_released(Duration::from_millis(800)) {
        eprintln!("[zwriter] modifiers still held after 800ms; injecting anyway");
    }
    // Clear the clipboard BEFORE Ctrl+C: if the selection equals what the
    // clipboard already holds (copy -> paste -> select -> fix again), a
    // change-detecting poll would never fire and the capture would silently
    // fail. After a successful clear, ANY text that appears is a fresh copy.
    // The original clipboard is restored on every path below that gives up.
    let cleared = app.clipboard().clear().is_ok();
    if !cleared {
        eprintln!("[zwriter] could not clear clipboard; falling back to change detection");
    }
    if let Err(e) = wininput::send_ctrl_c() {
        eprintln!("[zwriter] ctrl+c failed: {e}");
        if cleared {
            restore_clipboard(&app, &orig_clipboard);
        }
        return;
    }
    let poll_before = if cleared { None } else { orig_clipboard.as_deref() };
    let captured = poll_clipboard(&app, poll_before);
    let Some(text) = captured else {
        eprintln!("[zwriter] no text captured within deadline (nothing selected?)");
        if cleared {
            restore_clipboard(&app, &orig_clipboard);
        }
        return;
    };
    if text.trim().is_empty() {
        if cleared {
            restore_clipboard(&app, &orig_clipboard);
        }
        return;
    }

    // 3. Fix it. Review semantics: guesses are reported as lints, never
    // applied here (apply_guesses=false); the quick path may re-run below.
    let fix_started = Instant::now();
    let (fixed, lints) = state.engine.lock().unwrap().fix(&text, false);
    let fix_ms = fix_started.elapsed().as_millis();

    let entry = HistoryEntry {
        ts: now_ms(),
        original: text.clone(),
        fixed: fixed.clone(),
        fix_count: lints.len(),
        applied: false,
    };

    let payload = FixReady {
        no_change: no_change(&fixed, &text, &lints),
        fix_ms,
        original: text,
        fixed: fixed.clone(),
        lints,
        user_edited: false,
    };

    // 4. Quick path: paste straight back, no window. Clean text stays
    // silent — no interruption when there is nothing to fix. A GUESSED
    // abbreviation is different: unless the user opted out (and the guess
    // is unambiguous), the review window asks first (spec: option A).
    if quick {
        let has_guess = payload.lints.iter().any(|l| l.guessed);
        if !has_guess {
            if payload.no_change {
                println!("[zwriter] text already clean, nothing to paste ({fix_ms}ms)");
                return;
            }
            store_pending(&state, &payload, target_hwnd, orig_clipboard);
            paste_pending(&app);
            let mut e = entry.clone();
            e.applied = true;
            push_history(&state, e);
            return;
        }
        if !confirm_guesses(&app) {
            // Opted out: auto-apply unambiguous guesses. An ambiguous guess
            // (one edit from TWO triggers) has no safe auto-answer — window.
            let (fixed2, lints2) = state.engine.lock().unwrap().fix(&payload.original, true);
            let ambiguous = lints2.iter().any(|l| l.guessed && l.suggestions.len() > 1);
            if !ambiguous {
                let payload2 = FixReady {
                    no_change: no_change(&fixed2, &payload.original, &lints2),
                    fix_ms,
                    original: payload.original.clone(),
                    fixed: fixed2.clone(),
                    lints: lints2,
                    user_edited: false,
                };
                store_pending(&state, &payload2, target_hwnd, orig_clipboard);
                paste_pending(&app);
                let mut e = entry.clone();
                e.fixed = fixed2;
                e.applied = true;
                push_history(&state, e);
                return;
            }
            // ambiguous -> fall through to the review window with payload
        }
        // confirm on (or ambiguous): fall through to the review window
    }

    push_history(&state, entry);
    if let Some(win) = app.get_webview_window("main") {
        if app.emit_to("main", "fix-ready", &payload).is_err() {
            eprintln!("[zwriter] emit fix-ready failed");
        }
        let _ = win.show();
        let _ = win.set_focus();
    }
    println!(
        "[zwriter] fix ready in {fix_ms}ms ({} lints) — total {}ms; captured: {:?}; kinds: {:?}",
        payload.lints.len(),
        started.elapsed().as_millis(),
        payload.original.chars().take(60).collect::<String>(),
        payload.lints.iter().map(|l| l.kind.as_str()).collect::<Vec<_>>()
    );

    store_pending(&state, &payload, target_hwnd, orig_clipboard);
}

fn confirm_guesses(app: &AppHandle) -> bool {
    app.state::<AppState>()
        .settings
        .lock()
        .unwrap()
        .confirm_guesses
}

/// Park the fix so Apply/paste can reach it later (quick path pastes at
/// once; the review path waits for the user).
fn store_pending(
    state: &tauri::State<'_, AppState>,
    payload: &FixReady,
    target_hwnd: Option<isize>,
    orig_clipboard: Option<String>,
) {
    *state.pending.lock().unwrap() = Some(PendingFix {
        original: payload.original.clone(),
        fixed: payload.fixed.clone(),
        lints: payload.lints.clone(),
        fix_count: payload.lints.len(),
        user_edited: false,
        target_hwnd,
        orig_clipboard,
    });
}

/// Poll the clipboard for text differing from `before`, 50ms interval, 2s cap.
/// `before = None` means the clipboard was cleared before Ctrl+C, so ANY
/// non-empty read is a fresh copy.
fn poll_clipboard(app: &AppHandle, before: Option<&str>) -> Option<String> {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Ok(text) = app.clipboard().read_text() {
            let fresh = match before {
                Some(b) => Some(text.as_str()) != Some(b),
                None => true,
            };
            if fresh && !text.is_empty() {
                return Some(text);
            }
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Put the user's original clipboard back (no paste happened, so no need to
/// wait for a target app to read it — but give an in-flight Ctrl+C copy a
/// moment to land first so we don't fight it).
fn restore_clipboard(app: &AppHandle, orig: &Option<String>) {
    std::thread::sleep(Duration::from_millis(300));
    let result = match orig {
        Some(t) => app.clipboard().write_text(t.clone()).map(|_| ()),
        None => app.clipboard().clear().map(|_| ()),
    };
    if let Err(e) = result {
        eprintln!("[zwriter] clipboard restore failed: {e}");
    }
}

/// Focus the saved target window, paste the pending fix, restore the
/// original clipboard once the target app has read it. Sleeps — run on a
/// worker thread.
pub fn paste_pending(app: &AppHandle) {
    let state = app.state::<AppState>();
    let Some(pending) = state.pending.lock().unwrap().take() else {
        return;
    };

    if let Err(e) = app.clipboard().write_text(pending.fixed.clone()) {
        eprintln!("[zwriter] clipboard write failed: {e}");
        return;
    }

    let refocused = pending
        .target_hwnd
        .map(wininput::focus_window)
        .unwrap_or(false);
    if !refocused {
        eprintln!("[zwriter] could not refocus target window; pasting into foreground");
    }
    std::thread::sleep(Duration::from_millis(60));
    match wininput::send_ctrl_v() {
        Ok(()) => println!("[zwriter] pasted fix (refocus={refocused})"),
        Err(e) => eprintln!("[zwriter] ctrl+v failed: {e}"),
    }

    // Let the target app read the clipboard before we put the old content back.
    let app = app.clone();
    let orig = pending.orig_clipboard;
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(250));
        let restore = match orig {
            Some(t) => app.clipboard().write_text(t).map(|_| Some(())),
            None => app.clipboard().clear().map(|_| None),
        };
        if let Err(e) = restore {
            eprintln!("[zwriter] clipboard restore failed: {e}");
        }
    });
}

struct BusyGuard<'a>(&'a AtomicBool);

impl Drop for BusyGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;
    use tauri_plugin_global_shortcut::Shortcut;

    #[test]
    fn default_hotkeys_are_space_chords() {
        let s = default_settings();
        assert_eq!(s.fix_hotkey, "ctrl+shift+space");
        assert_eq!(s.quick_hotkey, "ctrl+space");
        assert_ne!(s.fix_hotkey, s.quick_hotkey);
    }

    /// The defaults must be strings the global-shortcut plugin can actually
    /// parse and register — a typo here would fail only at app startup.
    #[test]
    fn default_hotkeys_parse_as_registerable_shortcuts() {
        Shortcut::from_str(DEFAULT_FIX_HOTKEY).unwrap();
        Shortcut::from_str(DEFAULT_QUICK_HOTKEY).unwrap();
    }

    /// Option A is the default: quick fix asks before applying guesses.
    #[test]
    fn confirm_guesses_defaults_true() {
        assert!(default_settings().confirm_guesses);
    }
}
