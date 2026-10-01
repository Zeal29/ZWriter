# ZWriter — Self-Testing Playbook

Every way to test this app without asking the user anything.

## 0. Environment requirements

- Rust (rustup, stable) at `%USERPROFILE%\.cargo\bin` — proxies must exist.
  If `cargo --version` fails from cmd, re-run the rustup installer:
  `rustup-init.exe -y --no-modify-path --default-toolchain stable`.
- MSVC 2022 (present) — builds MUST go through `scripts/dev.bat` /
  `scripts/build.bat` (Git Bash's /usr/bin/link.exe poisons cargo otherwise).
- pnpm + Node (in `C:\Program Files\nodejs`).
- harper-core needs a recent stable (>= ~1.93). 1.91 fails to compile it.

## 1. Engine unit tests (no UI)

```
scripts/dev.bat-shell alternative: open cmd, run vcvars+PATH as in dev.bat, then:
cd src-tauri
cargo test
```

Expected: `test result: ok. 11 passed` (engine::tests — typo fixing, span
correctness, unicode offsets, clean-text passthrough, per-lint suggestions
list, smart-quote grammar lints + clean-text round-trip, multi-line
paragraph, long-sentence lints surviving overlap removal (user's exact
paragraph), steady-state latency). From Git Bash, strip the MSYS dirs from PATH
first (`/usr/bin`, `/bin`, `/mingw64/bin`, `/usr/local/bin`) — note `/bin` is
a SYMLINK of `/usr/bin` in Git Bash, so filtering only `/usr/bin` still
exposes Git's link.exe to rustc. Filter all four via a bash `case` loop.

## 2. Frontend build

```
pnpm build        # tsc + vite; must exit 0
```

## 3. App boots (tray + hotkey registered)

1. `scripts/dev.bat` — wait for `Running target\debug\zwriter.exe`.
2. Log must NOT contain `[zwriter] could not register ctrl+alt+g`.
3. Tray icon "ZWriter" exists; left-click opens the review window; tray menu
   has Show / Settings / Quit.
4. `zwriter.exe` must be alive after 60s (no panic loop).

## 4. End-to-end flow (the real thing)

`scripts/smoke-test.ps1` against a RUNNING app (dev or release):

- Prereq: pure-ASCII script (PowerShell 5.1 + GBK codepage; check with
  `grep -nP '[^\x00-\x7F]' scripts/smoke-test.ps1`).
- It: sets a sentinel clipboard, opens Notepad, types error-laden text,
  selects all, sends the REAL Ctrl+Alt+G, waits for the review window
  (EnumWindows by pid+title, NOT Process.MainWindowHandle — that returns an
  auxiliary window), clicks the Apply button via UIA InvokePattern
  (WebView2 exposes the tree; allow retries for it to materialize), then
  reads Notepad's text via UIA TextPattern.
- PASS = window shown + notepad contains Harper-fixed text ("believe thiss
  is a example"; Harper fixes beleive/exampel, keeps thiss) + clipboard
  sentinel restored + dev/app log has `pasted fix (refocus=true)`.
- Known false negative: `notepadPasted=False` when the document also holds
  older text (Win11 Notepad restores session tabs) — read the actual text
  line in the output before judging.
- If `no text captured within deadline` in the log: the modifier wait
  (wait_modifiers_released) is missing/short-circuited — see AGENTS.md rule 1.
- Sections J-N (v0.2 abbreviations/ignored words) cover: Settings teaching
  (trigger + expansion inputs, ignore input, the two guess checkboxes),
  exact-abbreviation review + quick paths, the 2x2 guess matrix
  (skip-window x auto-apply; ambiguous always opens the window) plus M22-M23
  (auto-apply pre-applies in the NORMAL review window too), and both popover
  teach buttons ("+ Add as abbreviation" opens the teach sub-form: trigger
  shown, type the expansion, Add/Cancel; "+ Ignore word") with live engine
  re-runs.
- Checkbox toggles in the suite MUST be verified against UIA ToggleState
  (`Set-Checkbox` helper): an unverified geometry click can be eaten by
  focus/scroll and silently test the wrong branch (first run's M6-M8).
- Expected outputs for abbreviation captures are PROBED engine behavior,
  not grammar intuition: "is there a tool for sc2 here" gets sentence
  capitalization, "the sc2 game" does NOT (harper quirk; AGENTS.md rule 22)
  — hence K7 `-ceq "Is ..."` vs L2 `-ceq "the ..."`.

- `scripts/color-check.ps1` (one-off visual check): stages a sentence with
  all three highlight classes (error / guessed abbr / exact abbr) in a
  review window and screenshots it to `colors-check.png`; backs up and
  restores the user's settings itself.

## 5. Manual checks (things the script can't judge)

- Paste lands at the caret in: a browser (Chromium), Electron app (ZCode),
  Office — not just Notepad.
- Elevated (admin) target windows: expected to FAIL silently (UIPI) — this is
  a documented Windows limit, confirm it degrades gracefully.
- Review window: hover a highlighted word shows the rule message; Enter
  applies, Esc skips; history section lists past fixes (cap 50).
- Word picker (review window): click ANY word in Original — linted words are
  highlighted; a popover lists ALL engine suggestions + an edit box; picking
  or editing re-runs the engine and the Fixed pane updates live; Apply pastes
  the edited text — INCLUDING when the engine finds no lints after the edit
  (user_edited flag; smoke section H). Smoke section E covers the picker.
- Smart quotes: grammar lints must fire on curly-apostrophe text ("She
  don’t like apples." → "doesn't"). It cannot be TYPED via SendKeys — smoke
  section F pastes it via `Set-Clipboard` + `^v`, captures with the real
  hotkey, and asserts the applied fix. SendKeys also cannot send Ctrl+Space;
  a suite running the USER's chords needs `keybd_event` for that.
- Multi-line + paste-then-fix: selecting a multi-line paragraph fixes every
  line (section G), and fixing text that is ALREADY on the clipboard (paste →
  select → fix, no sentinel) must still capture — the app clears the
  clipboard before its synthetic Ctrl+C precisely for this.
- Tray + taskbar: review window shows in the TASKBAR while visible and leaves
  it on close (hide). Tray icon always present (may sit in the Win11 overflow
  flyout); tray menu Show / Settings / Quit; Quit is the only way to exit.
- Settings: hotkey rebinds persist across app restart
  (%APPDATA%/com.alinawaz.zwriter/settings.json); Start-with-Windows toggle
  round-trips the registry entry. The old "Auto-apply fixes" setting was
  removed (redundant once both hotkeys became rebindable) — old settings
  files may still contain a stale "autoApply" key; the app ignores it. After a rebind, the review window's header
  hint text shows the new chords without a restart (smoke check C7).

## 6. Release verification

- `scripts/build.bat` → installer at
  `src-tauri/target/release/bundle/nsis/ZWriter_0.1.0_x64-setup.exe`,
  portable exe at `src-tauri/target/release/zwriter.exe`.
- Run the release exe, repeat test 4, and record: fix latency from the log
  line (`fix ready in Xms` — target: <100ms for a sentence) and RAM of
  zwriter.exe in Task Manager (target: well under 200MB; user priority).
