//! ZWriter — system-wide offline grammar fixer (Windows tray app).

mod engine;
mod flow;
mod wininput;

use std::collections::VecDeque;
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;

use serde::Serialize;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager,
};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt as _};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use flow::{AppState, FixReady, HistoryEntry, LintPayload, Settings};

// ---------------------------------------------------------------------------
// Settings persistence (app_config_dir/settings.json)
// ---------------------------------------------------------------------------

fn settings_path(app: &AppHandle) -> Option<std::path::PathBuf> {
    app.path().app_config_dir().ok().map(|d| d.join("settings.json"))
}

fn load_settings(app: &AppHandle) {
    let Some(path) = settings_path(app) else { return };
    let Ok(raw) = std::fs::read_to_string(&path) else { return };
    let Ok(saved) = serde_json::from_str::<serde_json::Value>(&raw) else { return };
    let state = app.state::<AppState>();
    let mut s = state.settings.lock().unwrap();
    // hotkeys; "hotkey" is the pre-rebinding key name
    if let Some(v) = saved
        .get("fixHotkey")
        .or_else(|| saved.get("hotkey"))
        .and_then(|v| v.as_str())
    {
        s.fix_hotkey = v.to_string();
    }
    if let Some(v) = saved.get("quickHotkey").and_then(|v| v.as_str()) {
        s.quick_hotkey = v.to_string();
    }
    // autostart is read live from the plugin
    // NB: a stale "autoApply" key in old settings.json files is ignored.
    if let Some(v) = saved.get("customWords").and_then(|v| v.as_array()) {
        s.custom_words = v
            .iter()
            .filter_map(|x| x.as_str())
            .map(|x| x.trim().to_string())
            .filter(|x| !x.is_empty())
            .collect();
        dedup_words(&mut s.custom_words);
    }
}

/// Case-insensitive dedup, keeping the first (display) spelling.
fn dedup_words(words: &mut Vec<String>) {
    let mut seen = std::collections::HashSet::new();
    words.retain(|w| seen.insert(w.to_lowercase()));
}

fn save_settings(app: &AppHandle) {
    let Some(path) = settings_path(app) else { return };
    let state = app.state::<AppState>();
    let s = state.settings.lock().unwrap().clone();
    let json = serde_json::json!({
        "fixHotkey": s.fix_hotkey,
        "quickHotkey": s.quick_hotkey,
        "customWords": s.custom_words,
    });
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(&path, json.to_string()) {
        eprintln!("[zwriter] saving settings failed: {e}");
    }
}

/// Rebuild the engine's dictionary from the current settings. Cheap: the
/// curated FST is process-cached, only the user words are re-hashed.
fn apply_custom_words(app: &AppHandle) {
    let words = {
        let state = app.state::<AppState>();
        let words = state.settings.lock().unwrap().custom_words.clone();
        words
    };
    let state = app.state::<AppState>();
    state.engine.lock().unwrap().set_custom_words(&words);
}

/// (Re)register both hotkeys from the current settings. unregister_all is
/// safe — these are the only global shortcuts the app owns.
fn apply_hotkeys(app: &AppHandle) -> Result<(), String> {
    app.global_shortcut()
        .unregister_all()
        .map_err(|e| e.to_string())?;
    let (fix, quick) = {
        let state = app.state::<AppState>();
        let g = state.settings.lock().unwrap();
        (g.fix_hotkey.clone(), g.quick_hotkey.clone())
    };
    app.global_shortcut()
        .register(fix.as_str())
        .map_err(|e| format!("'{fix}' could not be registered (in use by another app?): {e}"))?;
    app.global_shortcut()
        .register(quick.as_str())
        .map_err(|e| format!("'{quick}' could not be registered (in use by another app?): {e}"))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Commands (called from the React windows)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingPayload {
    pub original: String,
    pub fixed: String,
    pub lints: Vec<engine::LintJson>,
    pub fix_count: usize,
    pub user_edited: bool,
}

#[tauri::command]
fn fix_text(text: String, app: AppHandle) -> Result<FixReady, String> {
    let state = app.state::<AppState>();
    let started = std::time::Instant::now();
    let (fixed, lints) = state.engine.lock().unwrap().fix(&text);
    Ok(FixReady {
        no_change: fixed == text,
        fix_ms: started.elapsed().as_millis(),
        original: text,
        fixed,
        lints,
        user_edited: false,
    })
}

/// The user edited the text inside the review window (word picker). Re-run
/// the engine and update the pending fix in place so Apply/Copy use the
/// edited text; the target window and clipboard snapshot are kept. With no
/// pending fix (self-test demo text) it behaves exactly like fix_text.
///
/// Every call marks the fix user_edited: even when the engine finds no lints
/// in the edited text, the edit itself is what Apply must paste.
#[tauri::command]
fn update_pending(app: AppHandle, text: String) -> Result<FixReady, String> {
    let state = app.state::<AppState>();
    let started = std::time::Instant::now();
    let (fixed, lints) = state.engine.lock().unwrap().fix(&text);
    let has_pending = state.pending.lock().unwrap().is_some();
    if has_pending {
        if let Some(p) = state.pending.lock().unwrap().as_mut() {
            p.original = text.clone();
            p.fixed = fixed.clone();
            p.lints = lints.clone();
            p.fix_count = lints.len();
            p.user_edited = true;
        }
        // Keep the newest unapplied history entry in sync with the edits.
        let mut h = state.history.lock().unwrap();
        if let Some(front) = h.front_mut() {
            if !front.applied {
                front.original = text.clone();
                front.fixed = fixed.clone();
                front.fix_count = lints.len();
            }
        }
    }
    Ok(FixReady {
        no_change: fixed == text,
        fix_ms: started.elapsed().as_millis(),
        original: text,
        fixed,
        lints,
        user_edited: true,
    })
}

/// Dev/self-test helper: run the real capture pipeline from the UI.
#[tauri::command]
fn simulate_hotkey(app: AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || flow::capture_and_fix(app, false));
}

/// Rebind both hotkeys. Registers them before committing; on failure rolls
/// the settings (and registration) back to the previous chords.
#[tauri::command]
fn set_hotkeys(app: AppHandle, fix_hotkey: String, quick_hotkey: String) -> Result<(), String> {
    let norm = |s: String| s.trim().to_lowercase();
    let (fix, quick) = (norm(fix_hotkey), norm(quick_hotkey));
    if fix == quick {
        return Err("the two hotkeys must be different".into());
    }
    fix.parse::<Shortcut>()
        .map_err(|_| format!("cannot parse hotkey '{fix}'"))?;
    quick
        .parse::<Shortcut>()
        .map_err(|_| format!("cannot parse hotkey '{quick}'"))?;

    let old = {
        let state = app.state::<AppState>();
        let mut g = state.settings.lock().unwrap();
        let old = (g.fix_hotkey.clone(), g.quick_hotkey.clone());
        g.fix_hotkey = fix.clone();
        g.quick_hotkey = quick.clone();
        old
    };

    if let Err(e) = apply_hotkeys(&app) {
        {
            let state = app.state::<AppState>();
            let mut g = state.settings.lock().unwrap();
            g.fix_hotkey = old.0;
            g.quick_hotkey = old.1;
        }
        if let Err(e2) = apply_hotkeys(&app) {
            eprintln!("[zwriter] hotkey rollback failed: {e2}");
        }
        return Err(e);
    }
    save_settings(&app);
    // The review window lives for the whole session (close = hide) and only
    // reads settings at mount — push the new chords so its hint text follows.
    let _ = app.emit_to(
        "main",
        "hotkeys-changed",
        serde_json::json!({ "fixHotkey": fix, "quickHotkey": quick }),
    );
    Ok(())
}

/// Add word(s) to the custom dictionary. Input is split on whitespace, so a
/// paste of several words bulk-adds them; edge punctuation is stripped.
/// Matching is case-insensitive (harper hashes lowercase). Returns the
/// updated list.
#[tauri::command]
fn add_custom_word(app: AppHandle, word: String) -> Result<Vec<String>, String> {
    let new_words: Vec<String> = word
        .split_whitespace()
        .map(|w| w.trim_matches(|c: char| !(c.is_alphanumeric() || c == '\'')))
        .filter(|w| !w.is_empty())
        .map(|w| w.to_string())
        .collect();
    if new_words.is_empty() {
        return Err("type a word first".into());
    }
    let words = {
        let state = app.state::<AppState>();
        let mut s = state.settings.lock().unwrap();
        for w in new_words {
            if !s
                .custom_words
                .iter()
                .any(|x| x.to_lowercase() == w.to_lowercase())
            {
                s.custom_words.push(w);
            }
        }
        s.custom_words.clone()
    };
    apply_custom_words(&app);
    save_settings(&app);
    // The review window may hold a live fix whose lints just changed.
    let _ = app.emit("custom-words-changed", serde_json::json!({ "words": words }));
    Ok(words)
}

/// Remove a word from the custom dictionary (case-insensitive). Idempotent.
#[tauri::command]
fn remove_custom_word(app: AppHandle, word: String) -> Result<Vec<String>, String> {
    let target = word.trim().to_lowercase();
    let words = {
        let state = app.state::<AppState>();
        let mut s = state.settings.lock().unwrap();
        s.custom_words.retain(|x| x.to_lowercase() != target);
        s.custom_words.clone()
    };
    apply_custom_words(&app);
    save_settings(&app);
    let _ = app.emit("custom-words-changed", serde_json::json!({ "words": words }));
    Ok(words)
}

#[tauri::command]
fn get_pending(app: AppHandle) -> Option<PendingPayload> {
    let state = app.state::<AppState>();
    let guard = state.pending.lock().unwrap();
    let p = guard.as_ref()?;
    Some(PendingPayload {
        original: p.original.clone(),
        fixed: p.fixed.clone(),
        lints: p.lints.clone(),
        fix_count: p.fix_count,
        user_edited: p.user_edited,
    })
}

#[tauri::command]
fn apply_paste(app: AppHandle) {
    let worker = app.clone();
    std::thread::spawn(move || flow::paste_pending(&worker));
    if let Some(win) = app.get_webview_window("main") {
        match win.hide() {
            Ok(()) => println!("[zwriter] main hidden after apply"),
            Err(e) => eprintln!("[zwriter] main hide FAILED after apply: {e}"),
        }
    } else {
        eprintln!("[zwriter] main window handle missing at apply");
    }
}

#[tauri::command]
fn dismiss_fix(app: AppHandle) {
    let state = app.state::<AppState>();
    *state.pending.lock().unwrap() = None;
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.hide();
    }
}

/// Copy the fixed text to the clipboard without pasting (review window).
#[tauri::command]
fn copy_fixed(app: AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let text = state
        .pending
        .lock()
        .unwrap()
        .as_ref()
        .map(|p| p.fixed.clone());
    match text {
        Some(t) => app.clipboard().write_text(t).map_err(|e| e.to_string()),
        None => Err("no pending fix".into()),
    }
}

#[tauri::command]
fn get_history(app: AppHandle) -> Vec<HistoryEntry> {
    app.state::<AppState>().history.lock().unwrap().iter().cloned().collect()
}

#[tauri::command]
fn clear_history(app: AppHandle) {
    app.state::<AppState>().history.lock().unwrap().clear();
}

#[tauri::command]
fn get_settings(app: AppHandle) -> Settings {
    let state = app.state::<AppState>();
    let autostart = app.autolaunch().is_enabled().unwrap_or(false);
    let mut s = state.settings.lock().unwrap().clone();
    s.autostart = autostart;
    s
}

#[tauri::command]
fn set_autostart(app: AppHandle, enabled: bool) -> Result<bool, String> {
    let manager = app.autolaunch();
    let result = if enabled {
        manager.enable()
    } else {
        manager.disable()
    };
    result.map_err(|e| e.to_string())?;
    let state = app.state::<AppState>();
    let mut s = state.settings.lock().unwrap();
    s.autostart = enabled;
    Ok(enabled)
}

#[tauri::command]
fn open_settings(app: AppHandle) {
    if let Some(win) = app.get_webview_window("settings") {
        let _ = win.unminimize();
        let _ = win.show();
        let _ = win.set_focus();
    }
}

#[tauri::command]
fn hide_settings(app: AppHandle) {
    if let Some(win) = app.get_webview_window("settings") {
        let _ = win.hide();
    }
}

// ---------------------------------------------------------------------------
// App entry
// ---------------------------------------------------------------------------

fn show_main(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.unminimize();
        let _ = win.show();
        let _ = win.set_focus();
    }
}

/// Closing a window hides it instead of destroying it — the webviews must
/// stay alive for the hotkey to emit into them. Real exit is tray → Quit.
fn make_close_hide(win: &tauri::WebviewWindow) {
    let w = win.clone();
    win.on_window_event(move |event| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = w.hide();
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, None))
        .manage(AppState {
            engine: Mutex::new(engine::Engine::new()),
            pending: Mutex::new(None),
            history: Mutex::new(VecDeque::new()),
            settings: Mutex::new(flow::default_settings()),
            busy: AtomicBool::new(false),
        })
        .invoke_handler(tauri::generate_handler![
            fix_text,
            update_pending,
            simulate_hotkey,
            get_pending,
            apply_paste,
            dismiss_fix,
            copy_fixed,
            get_history,
            clear_history,
            get_settings,
            set_autostart,
            set_hotkeys,
            add_custom_word,
            remove_custom_word,
            open_settings,
            hide_settings
        ])
        .setup(|app| {
            load_settings(app.handle());
            apply_custom_words(app.handle());

            // --- System tray -------------------------------------------------
            let show_i = MenuItem::with_id(app, "show", "Show ZWriter", true, None::<&str>)?;
            let settings_i = MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
            let quit_i = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show_i, &settings_i, &quit_i])?;

            TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("ZWriter - select text, then use your fix hotkey")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => show_main(app),
                    "settings" => open_settings(app.clone()),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        show_main(tray.app_handle());
                    }
                })
                .build(app)?;

            // Windows must hide on close, not die (hotkey emits into them).
            if let Some(win) = app.get_webview_window("main") {
                make_close_hide(&win);
            }
            if let Some(win) = app.get_webview_window("settings") {
                make_close_hide(&win);
            }

            // --- Global hotkeys (from settings, dispatch by chord) ----------
            app.handle().plugin(
                tauri_plugin_global_shortcut::Builder::new()
                    .with_handler(|app, shortcut, event| {
                        if event.state() != ShortcutState::Pressed {
                            return;
                        }
                        // Resolve the current chords at fire time so rebinding
                        // needs no re-registration of the handler.
                        let (fix, quick) = {
                            let state = app.state::<AppState>();
                            let g = state.settings.lock().unwrap();
                            (g.fix_hotkey.clone(), g.quick_hotkey.clone())
                        };
                        let is_fix = fix
                            .parse::<Shortcut>()
                            .map(|s| s == *shortcut)
                            .unwrap_or(false);
                        let is_quick = !is_fix
                            && quick
                                .parse::<Shortcut>()
                                .map(|s| s == *shortcut)
                                .unwrap_or(false);
                        println!(
                            "[zwriter] hotkey fired: {} (fix={fix} quick={quick}) -> is_fix={is_fix} is_quick={is_quick}",
                            shortcut
                        );
                        if is_fix || is_quick {
                            let app = app.clone();
                            std::thread::spawn(move || flow::capture_and_fix(app, is_quick));
                        }
                    })
                    .build(),
            )?;
            if let Err(e) = apply_hotkeys(app.handle()) {
                eprintln!("[zwriter] hotkey registration failed: {e}");
            }

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

// Silence unused-import warning for the payload type re-exports used by commands.
#[allow(unused)]
fn _type_witness(_: FixReady, _: LintPayload, _: VecDeque<HistoryEntry>, _: Settings) {}
