# ZWriter

**System-wide, offline grammar fixer for Windows.** Select text anywhere,
press **Ctrl+Shift+Space**, and ZWriter fixes spelling and grammar mistakes and
pastes the result right back where you were typing.

![Platform](https://img.shields.io/badge/platform-Windows%2010%2F11-blue)
![Offline](https://img.shields.io/badge/privacy-100%25%20offline-green)
![License](https://img.shields.io/badge/license-MIT-yellow)

Fixes are made by [Harper](https://writewithharper.com) — a **rules-based
grammar engine, no AI, no internet**. Nothing you write ever leaves your
machine.

---

## What it does

1. You select (or just placed your cursor in) text in **any app** — Notepad,
   browser, email, anywhere.
2. You press the hotkey.
3. ZWriter grabs the selection, fixes it in milliseconds, and shows a small
   review window: the original with errors highlighted next to the corrected
   text.
4. One click (or `Enter`) pastes the fix back into the app you started in.
   Your clipboard is restored afterwards.

There is also a **quick-fix hotkey** (`Ctrl+Space`) that pastes the fix
immediately with no window at all — and stays silent when your text is
already clean.

## Features

- **Review flow** — original vs. fixed side by side, errors highlighted,
  `Enter` applies, `Esc` skips.
- **Quick fix** — fix + paste in one step, no interruption on clean text.
- **Custom dictionary** — your own words (names, jargon, product terms) are
  never flagged. Add them in Settings, or click a flagged word →
  **"+ Add to dictionary"**. Matching ignores capitalization.
- **Abbreviations** — teach ZWriter that `sc2` means `StarCraft 2` and it
  will suggest the expansion (even from one letter off, like `se2` or
  `sc 2`). Exact matches always expand; near-misses show up as a
  suggestion you confirm. Teach pairs in Settings, or click a word →
  **"+ Add as abbreviation"**.
- **Ignored words** — codes and IDs (`s12`, `mp3`) are left exactly as
  typed: never flagged, never "corrected". Session-only by design — the
  list clears when ZWriter restarts. Settings section, or click a word →
  **"+ Ignore word"**. Codes that mix letters and digits are never flagged
  in the first place.
- **Word picker** — click any word in the review window to see every
  alternative the engine offers, or type your own replacement; the fix
  updates live.
- **Rebindable hotkeys** — click a hotkey field in Settings and press any
  key combination (e.g. `Ctrl+Shift+Space`). Conflicts with other apps are
  detected and rolled back.
- **History** — the last 50 fixes, viewable in the review window.
- **Tray app** — lives in the system tray; Start-with-Windows optional;
  quit from the tray menu.
- **Offline & private** — no accounts, no telemetry, no network access.

## Install

### Option A — download (recommended)

Grab the latest release from the
[Releases page](https://github.com/Zeal29/ZWriter/releases/latest):

| File | What it is |
|------|------------|
| `ZWriter_0.1.0_x64-setup.exe` | Installer (recommended). Start-menu shortcut, easy uninstall. |
| `ZWriter_0.1.0_portable.exe` | Portable single exe. Run it anywhere, no install. |

1. Run the installer (or the portable exe).
2. ZWriter starts minimized to the system tray.
3. Optional: open Settings and enable **Start with Windows**.

> **Windows SmartScreen note:** the binaries are not code-signed (it costs
> money). If SmartScreen pops up, click **More info → Run anyway**. If you
> prefer not to, build from source below — it takes one command.

### Option B — build from source

Requirements: **Windows 10/11**, [Rust](https://rustup.rs) (recent stable —
harper-core needs ≥ 1.93), [Node.js](https://nodejs.org) +
[pnpm](https://pnpm.io), and **MSVC Build Tools 2022** (the C++ linker). The
WebView2 runtime ships with Windows 11.

```cmd
git clone https://github.com/Zeal29/ZWriter.git
cd ZWriter
pnpm install
scripts\build.bat
```

The build produces:

- `src-tauri\target\release\zwriter.exe` — portable exe
- `src-tauri\target\release\bundle\nsis\ZWriter_0.1.0_x64-setup.exe` — installer

For development with hot reload: `scripts\dev.bat`.
(On Windows, build through the provided `.bat` scripts — Git Bash's
`/usr/bin/link.exe` shadows the MSVC linker otherwise; see
[AGENTS.md](AGENTS.md).)

## Usage

| Action | How |
|--------|-----|
| Fix with review | Select text → `Ctrl+Shift+Space` → **Apply** (`Enter`) |
| Fix instantly | Select text → `Ctrl+Space` |
| Skip a suggestion | `Esc` or the **Skip** button |
| Whitelist a word | Click the flagged word → **+ Add to dictionary** |
| Teach an abbreviation | Click a word → type the full term in the box → **+ Add as abbreviation** (or Settings → Abbreviations) |
| Ignore a code/ID | Click the word → **+ Ignore word** (session-only; or Settings → Ignored words) |
| Quick fix & guesses | Exact abbreviations paste instantly. Unsure near-matches follow two Settings checkboxes: *skip unsure* (no review window) and *auto-apply unsure* (use the expansion anyway) |
| Replace a word manually | Click any word → pick a suggestion or type your own |
| Change hotkeys | Settings (gear icon, tray menu, or `Ctrl+,` in the review window) |
| Quit | Tray icon → **Quit** |

## How it works

ZWriter is a [Tauri 2](https://v2.tauri.app) app: a Rust backend with two
small React windows (review + settings), running as a system-tray
application. The fix pipeline:

```
selection ──▶ capture ──▶ Harper engine ──▶ review / paste ──▶ clipboard restored
```

1. **Capture.** The hotkey handler remembers which window you were in and
   what your clipboard held, *clears* the clipboard, synthesizes `Ctrl+C`
   into the foreground window, and polls until the selection arrives
   (clearing first means any text that appears is provably a fresh copy —
   re-fixing text you just pasted works even though the clipboard "didn't
   change").
2. **Fix.** The text is parsed by
   [harper-core](https://docs.rs/harper-core) — a deterministic, rules-based
   grammar engine (no AI, no network). ZWriter runs its `LintGroup`
   (spelling, agreement, capitalization, ~100 rules), normalizes smart
   quotes so curly-apostrophe text still gets grammar fixes, drops
   suggestion-less lints, and applies the best suggestion for every finding
   back-to-front. Your custom-dictionary words are merged into the
   engine's dictionary, so they are never flagged in the first place.
   A typical sentence fixes in under 10 ms.
3. **Review or quick-paste.** The review window highlights the findings and
   offers alternatives per word. The quick hotkey skips straight to pasting.
4. **Apply.** ZWriter refocuses your original window, pastes the fix via
   `Ctrl+V`, then quietly restores the clipboard you had before.

### Project layout

| Path | What lives there |
|------|------------------|
| `src-tauri/src/engine.rs` | Harper wrapper: linting, suggestion application, custom dictionary |
| `src-tauri/src/flow.rs` | The hotkey pipeline: capture → fix → paste, history, settings |
| `src-tauri/src/wininput.rs` | Windows input injection (`SendInput`, foreground-window helpers) |
| `src-tauri/src/lib.rs` | Tauri commands, tray, global hotkeys, settings persistence |
| `src/Review.tsx` | Review window (highlights, word picker, history) |
| `src/Settings.tsx` | Settings window (hotkeys, custom dictionary, autostart) |
| `scripts/` | `dev.bat`, `build.bat`, and the end-to-end smoke test |
| `docs/SELF-TESTING.md` | How the app is tested end-to-end |

## Privacy

- **No network access.** The engine is compiled into the exe; no text is
  ever sent anywhere.
- Settings are a plain JSON file in your user config directory.
- The clipboard round-trip restores your original clipboard content on
  every path, including failures.

## Limitations

- **English only** (Harper's current language).
- Apps running **as administrator** refuse injected input (Windows UIPI) —
  run ZWriter elevated too if you need that.
- The clipboard capture needs an app that supports `Ctrl+C` for its
  selection (Notepad, browsers, Office, most editors — verified; terminals
  each have their own conventions).

## Development & testing

```cmd
scripts\dev.bat               :: run with hot reload
cd src-tauri && cargo test    :: engine unit tests (14)
scripts\smoke-test.ps1        :: 68-check end-to-end UI suite (needs a display)
```

The smoke suite drives the real app with real global hotkeys and Notepad:
capture → review → apply, quick fix, word picker, custom dictionary,
hotkey rebinding, plus log-derived invariants. See
[docs/SELF-TESTING.md](docs/SELF-TESTING.md).

## License

[MIT](LICENSE) — © 2026 Ali Nawaz
