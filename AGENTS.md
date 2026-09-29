# ZWriter — Project Rules & Knowledge

System-wide, offline grammar fixer for Windows. Global hotkey (Ctrl+Alt+G)
captures the selected text via a clipboard round-trip, fixes it with Harper
(rules-based, no AI), shows a review window, and pastes the fix back into the
app the user started in.

## Stack & vocabulary

- **Tauri 2** (Rust backend + React/TS webviews; WebView2 runtime on Windows).
- **harper-core 2.11** — grammar engine. Terms: a **lint** is one found error
  (char-index span + suggestion); **fix** = apply first suggestion of every
  lint, back-to-front so earlier spans stay valid.
- **Capture** = save clipboard, CLEAR the clipboard, synthesize Ctrl+C into
  the foreground window, poll for ANY text (50ms interval, 2s deadline).
  Clearing first is essential: when the selection equals what the clipboard
  already holds (copy → paste → select → fix again), change-detection never
  fires and the capture silently fails — the user sees "nothing happened"
  and concludes only part of their text gets fixed. The saved clipboard is
  restored on apply AND on every capture-failure path.
- **Apply** = write fixed text to clipboard, refocus the saved target window,
  synthesize Ctrl+V, restore the original clipboard after ~250ms.
- Windows: **main** = review window, **settings** = second window (url
  `index.html#/settings`, React routes on the hash). Both hide on close;
  main is taskbar-visible while shown (no skipTaskbar); the tray icon
  (Show / Settings / Quit) is the only way to quit.
- **Word picker** = click any word in the Original pane → popover with ALL
  engine suggestions for that word's lint (Spelling lints offer several
  alternatives) + an edit box + "Add to dictionary"; committing re-runs the
  engine via the `update_pending` command and the Fixed pane updates live.
  Demo-text edits (self-test box) go through `fix_text` instead so a live
  pending fix is never clobbered.
- **Custom dictionary** = user-whitelisted words (settings.json `customWords`
  → `Settings.custom_words`), never flagged as spelling errors. The engine
  merges them as a `MutableDictionary` onto the curated FST
  (`MergedDictionary`) and rebuilds BOTH the LintGroup and the Document
  annotation dictionary when the list changes. Matching is case-insensitive.
  Managed in Settings ("Custom dictionary" section) and from the review
  window's word popover ("+ Add to dictionary").
- **hotkeys-changed** = event `set_hotkeys` emits to `main` after a rebind;
  Review.tsx listens so its hint text follows without a restart.
- **custom-words-changed** = event `add_custom_word`/`remove_custom_word`
  emit to `main`; Review.tsx re-runs the engine on the text it is showing, so
  a word added in Settings unflags the open review live (and vice versa).

## Non-obvious rules (learned the hard way)

1. **Modifier contamination**: the hotkey handler fires on the G *keydown*
   while the user still holds Ctrl/Alt. Injecting Ctrl+C then produces
   Ctrl+Alt+C, which most apps ignore. `wininput::wait_modifiers_released()`
   (GetAsyncKeyState poll, 800ms cap) MUST run before `send_ctrl_c()`.
2. **Windows path case-insensitivity**: `/d/x/zwriter` and `/d/x/ZWriter` are
   the SAME directory in Git Bash on Windows. `rm -rf` on a case-variant path
   deletes the real project (happened once, cost a re-scaffold).
3. **Git Bash `/usr/bin/link.exe` shadows the MSVC linker** → cargo link
   failures ("extra operand"). Never put Git's usr/bin in PATH for cargo; run
   builds through `scripts/dev.bat` / `scripts/build.bat` (vcvars64 + cargo on
   PATH). With link.exe NOT on PATH, rustc auto-detects MSVC + SDK paths.
4. **PowerShell 5.1 reads UTF-8-no-BOM .ps1 as the ANSI/GBK codepage** — a
   UTF-8 em dash's trailing byte swallows the next ASCII quote and the parser
   reports "string is missing the terminator" at a later line. Keep .ps1
   scripts pure ASCII (checked with `grep -nP '[^\x00-\x7F]'`).
5. **Harper spans are CHAR indices** (Unicode scalars), not UTF-16 — the
    frontend converts via `charToUtf16Map` before slicing JS strings.
5b. **Smart quotes break harper's grammar linters**: "She don’t like" (U+2019,
    what smart-quote autocorrect produces) yields ZERO Agreement lints while
    "She don't like" lints fine (harper-core 2.11.0 = latest; verified). The
    engine normalizes ‘’“” → '" 1:1 before linting (`normalize_char` in
    engine.rs) and applies suggestions to the ORIGINAL chars so clean text
    round-trips byte-for-byte and untouched quotes keep their style.
6. **Apply edits back-to-front** (sort by `Reverse(span.start)`) — forward
   application shifts later spans. `remove_overlaps()` first (harper-core) —
   BUT drop suggestion-less lints before it: harper's Readability lint ("this
   sentence is N words long") spans the WHOLE sentence at priority 127 and
   remove_overlaps then deletes every real spelling lint (priority 63)
   inside it — long sentences came out completely unfixed while short ones
   worked. Same retain drops the app's own name ("ZWriter" is not in the
   curated dictionary and was being "corrected" to "Writer").
7. **LintGroup init is expensive** — one instance lives in `AppState`
   (Mutex), never per-call. `lint()` takes `&mut self` (sync).
8. **Window close must be intercepted** (`make_close_hide` in lib.rs): closing
   would destroy the webview, and the hotkey emits `fix-ready` INTO it.
9. **harper-core needs rustc >= ~1.93** — it does not compile on older stable
   (fn-array coercion regression around 1.91). Machine toolchain: updated via
   fresh rustup install (2026-09-29), now 1.98.1. Rust proxies live in
   `~/.cargo/bin` (rustup.exe was missing before; `.rustup` existed alone).
10. **Tauri capabilities gate everything** — `src-tauri/capabilities/*.json`
    must list window labels (`["main", "settings"]`) and every plugin
    permission. No capability = failing IPC with no build error.
11. **`Process.MainWindowHandle` lies for multi-window processes** — it can
    return an auxiliary window ("Tao Thread Event Target"). Test scripts must
    EnumWindows by pid + visible + exact title (`Get-ZWindow` in smoke-test.ps1).
12. **`SetWindowPos` flag 0x40 is SWP_SHOWWINDOW** — z-order helpers that pass
    0x43 (NOSIZE|NOMOVE|SHOWWINDOW) RE-SHOW windows the app just hid; use
    0x13 (add SWP_NOACTIVATE, no SHOWWINDOW). Cost an hour of blaming the
    app for a "window stays visible after Apply" bug that was the test's.
13. **Win11 Notepad autocorrects while typing** — scripted misspellings may
    be fixed before the app ever captures them. Assert exact deterministic
    output and expect Harper's contextual pick (mid-sentence "beleive" →
    "belief", not "believe").
14. **A running instance locks `target/release/zwriter.exe`** — builds fail
    with "Access is denied". `scripts/build.bat` taskkills first.
15. **Settings-window Esc needs webview focus** — clicking the title bar
    focuses the OS window but not the page; keydown handlers don't fire.
    Give buttons `aria-label`s (e.g. "Close settings") so tests can click
    them via UIA instead of relying on keys.
16. **Log-grep invariants must read BOTH redirect files** — the app's
    "no text captured" is `eprintln!` (stderr), captured to a separate
    .err.log; an invariant grepping only stdout passes exactly when a
    capture fails. L3 in smoke-test.ps1 reads both.
17. **Capture races in the smoke suite get ONE retry** — the global hotkey
    fires regardless of focus, but the injected Ctrl+C needs the target
    still foreground; on a machine in active use the capture can lose that
    race (~1 flake per 2 runs). Same for the Settings RECORDING keystroke:
    the ^chord sent right after clicking the hotkey field can go astray
    (C4/D3 retry: re-click the field, re-record). C6/D4 press again if no
    window in 6s.
18. **UIA helpers must never throw** — a stale/zero hwnd inside
    Click-Button/Has-Button/Click-Edit used to abort the whole suite with
    an uncaught COM exception (and the settings-restore step never ran).
    All are try/catch + Zero-guarded now; a dead window FAILs its check.
19. **SendKeys cannot send Ctrl+Space** (`"^ "` silently does nothing) —
    inject it with `keybd_event` (VK_CONTROL/VK_SPACE down/up). Also:
    `SetForegroundWindow` from a background process usually FAILS (foreground
    lock) — get focus with a real `mouse_event` click at the window center,
    as `New-Selection` does. Unicode test text (curly quotes) can't be TYPED
    via SendKeys either — `Set-Clipboard` + `^v` paste instead.
20. **Custom words must go into the DOCUMENT, not just the LintGroup** —
    harper's SpellCheck only skips a word when the TOKEN carries metadata
    (`if let Some(metadata) = word.kind.as_word()`), and
    `Document::new_curated` leaves unknown words as `Word(None)` because it
    annotates from the curated dictionary alone. Passing a MergedDictionary
    to LintGroup but still building the Document with `new_curated` keeps
    flagging whitelisted words ("Did you mean to spell X this way?" with X
    itself as first suggestion = this bug). Use
    `Document::new(text, parser, &merged_dict)` (what harper-ls does) and
    rebuild it together with the LintGroup. Probe-verified 2026-09-29.

## Knowledge sources

- Tauri 2 docs: https://v2.tauri.app (config schema, capabilities, plugins)
- harper-core: https://docs.rs/harper-core/2.11.0 (Linter trait, LintGroup,
  Suggestion::apply(span, &mut Vec<char>), remove_overlaps)
- windows crate: https://microsoft.github.io/windows-docs-rs/ (0.62:
  SendInput takes a slice; IsWindow takes Option<HWND>; GetAsyncKeyState
  takes i32; AttachThreadInput lives in Win32::System::Threading)
- Foreground rules: SetForegroundWindow restrictions on learn.microsoft.com;
  hotkey press grants foreground rights (Raymond Chen).

## Dev loop

- `scripts/dev.bat` — vcvars + `pnpm tauri dev` (hot reload both sides).
- `scripts/build.bat` — vcvars + `pnpm tauri build -b nsis`.
- `cargo test` (in src-tauri, NOT from Git Bash default PATH — use dev.bat
  environment, or filter ALL MSYS dirs from PATH: `/usr/bin`, `/bin`,
  `/mingw64/bin`, `/usr/local/bin`; `/bin` is a SYMLINK of `/usr/bin` in Git
  Bash so filtering `/usr/bin` alone still exposes Git's link.exe) — engine
  unit tests.
- `scripts/smoke-test.ps1` — end-to-end UI test against a running app; see
  docs/SELF-TESTING.md.

## Verified / Questions / Assumptions

**Verified (2026-09-29, eighth session — custom dictionary):**
- Custom dictionary shipped end-to-end: `Settings.custom_words` (settings.json
  `customWords`), commands `add_custom_word` / `remove_custom_word` (both
  persist + rebuild the engine + emit `custom-words-changed`), Settings UI
  section (add input + removable chips), and a "+ Add to dictionary" button
  in the review window's word popover. Matching is case-insensitive; input
  is split on whitespace (bulk-add) and edge punctuation is stripped.
- Engine: `MergedDictionary` = curated FST + `MutableDictionary` of user
  words; LintGroup AND Document rebuilt on every change (cheap — curated FST
  is process-cached). THE GOTCHA (rule 20): the Document must be annotated
  with the merged dict (`Document::new`), not `new_curated` — SpellCheck only
  skips tokens that carry metadata, and new_curated leaves unknown words
  `Word(None)`, so whitelisted words stayed flagged ("Did you mean to spell X
  this way?" with X itself as first suggestion = this bug). Found by probe
  test printing per-token kind metadata.
- Settings-window staleness: it mounts at app start and used to fetch once —
  words added from the review picker never appeared. Fixed with a focus
  refetch + `custom-words-changed` listener (event now broadcast via
  `app.emit`, not `emit_to("main")`).
- Smoke-section writing lesson: Win11 Notepad autocorrects TYPED text
  ("mistkae"→"mistake", "beleive"→"believe", "exampel"→"example") but leaves
  PASTED text alone — paste the test sentence (F/G pattern), don't type it.
  First I-section run: 61/67, all 6 failures from typed-text autocorrect.
- `cargo test` 14/14 (new: custom word not flagged, any capitalization,
  removal re-flags); `pnpm build` green; release + NSIS rebuilt; smoke suite
  **68/68** (new I1–I14: picker add unflags live, Settings add/remove,
  removal re-flags, settings window shows picker-added words); user settings
  restored by the suite.

**Verified (2026-09-29, seventh session — auto-apply setting removed):**
- The "Auto-apply fixes" setting was removed end-to-end (UI row, types,
  `set_auto_apply` command, `Settings.auto_apply`, persistence, and the
  `quick || auto_apply` branch — the quick hotkey alone routes to
  paste-immediately now). Redundant since both hotkeys are user-rebindable.
- Old settings.json files may keep a stale "autoApply" key — load_settings
  ignores it; the next save drops it.
- `cargo test` 11/11; smoke suite **54/54**. First post-removal run failed
  D3/D4/L1: the Settings RECORDING keystroke raced (chord went astray, the
  rebind-back never happened, so D4's global chord legitimately didn't exist)
  — passed twice before with identical app code, i.e. environmental. The
  suite now retries the C4/D3 recording steps like the capture races.

**Verified (2026-09-29, sixth session — long-sentence fixes swallowed):**
- User's exact test paragraph ("...but youu still use the hotkey... chracter
  ×3...") came out with ONLY the short last sentences fixed. Root cause
  (repro'd on the exact text): harper's Readability lint spans the whole
  61-word first sentence at priority 127 with NO suggestion, and
  `remove_overlaps` dropped all priority-63 spelling lints inside its span —
  6 raw lints became 1 (the useless Readability one). This was also the true
  mechanism behind "only the last line gets fixed": long sentences died,
  short ones survived. Found by running the user's exact text through the
  engine — the third distinct cause hiding behind that one symptom (clipboard
  collision was fixed separately the same day).
- Fix in `Engine::fix`: retain only suggestion-bearing lints (and not the
  product name "ZWriter", which the curated dictionary doesn't know — it was
  being "corrected" to "Writer") BEFORE remove_overlaps. After: chracter →
  character ×3, youu → you, all survive.
- `cargo test` 11/11 (new: long_sentence_lints_survive_readability_overlap,
  asserting the user's exact paragraph); smoke suite **54/54**; release
  rebuilt; user settings (ctrl+space / ctrl+shift+space) restored.

**Verified (2026-09-29, fifth session — user-edit apply + clipboard collision):**
- User reports: (a) after a manual word-picker edit that leaves no engine
  lints, the window said "nothing to fix" and Apply was disabled even though
  the user's edit should paste; (b) fixing a full paragraph only fixed the
  last line.
- (a) root cause: `noChange` (engine found nothing) gated the Apply button
  and the "already clean" placeholder. Fix: `user_edited` flag on
  PendingFix/FixReady, set by `update_pending`; the UI disables Apply only
  when `noChange && !userEdited` (plus a frontend ref for demo-text edits).
- (b) root cause: NOT the engine or multi-line handling (unit test: LF+CRLF
  4-line paragraphs lint all lines). It was the capture: the poll waited for
  the clipboard to CHANGE, and when the selection equals the clipboard
  content (copy → paste → select → fix again — e.g. re-fixing a paragraph
  you just pasted, or the clipboard holding the same paragraph from an
  earlier copy), Ctrl+C produces identical text, the poll times out, and the
  capture silently fails. Full-paragraph attempts died this way while a
  last-line-only selection (≠ clipboard) worked — hence "only fixes the last
  line". Fix: CLEAR the clipboard before Ctrl+C so any appearing text is
  provably a fresh copy; restore the original on failure paths too.
- `cargo test` 10/10 (new: multiline_paragraph_fixes_every_line); smoke
  suite **54/54** with new sections: **G1–G4** paste-select-fix collision →
  all 3 lines of a multi-line paragraph captured, linted, applied; **H1–H8**
  clean capture → Apply provably disabled (UIA IsEnabled) → user edit →
  Apply enabled → "Hello planet this is clean text." pasted exactly.
- SendKeys quirk confirmed live: "^ " (Ctrl+Space) sends NOTHING; the user's
  real chord needs keybd_event (see rule 19).

**Verified (2026-09-29, fourth session — smart-quote grammar fix):**
- User report: grammar issues ("She don't like apples.", "He didn't went
  home.") were not flagged — only spelling was. Root cause: the captured text
  carried CURLY apostrophes (U+2019 from smart-quote autocorrect), and
  harper's phrase-level grammar linters match nothing there (0 lints), while
  the spelling linter still fires. Straight-apostrophe versions litted fine
  on the SAME engine — proven by an examples/ repro, and harper-core 2.11.0
  is the latest (no upgrade path).
- Fix: `normalize_char` in engine.rs maps ‘’“” → '" 1:1 before linting;
  suggestions apply to the ORIGINAL chars (clean text round-trips
  byte-for-byte). Regression tests: curly "She don’t" → "doesn't",
  curly "didn’t went" → "go home", curly-quote clean text untouched.
- `cargo test` 9/9; `pnpm build` green; release rebuilt.
- Smoke suite: **42/42** including new **F1–F6**: a pasted curly-apostrophe
  sentence captured via the real hotkey, review shows the Agreement lint +
  suggestion chip, Apply pasted "She doesn't like apples." exactly, sentinel
  restored. All prior sections (A/B/E/C/D/L) still green; user settings
  (ctrl+space / ctrl+shift+space) backed up and restored; new exe left
  running.

**Verified (2026-09-29, third session — word picker + taskbar + hotkey text):**
- `cargo test` 7/7 (new: misspelling exposes multiple suggestions —
  "exampel" → example/enamel/expel; no Remove option on Spelling lints);
  `pnpm build` green; release rebuilt.
- Smoke suite: **35/35 checks** against the release exe, including new:
  - **E1–E13 word picker**: taskbar visibility while shown (no
    WS_EX_TOOLWINDOW), clicking linted word "thiss" offers a suggestion
    chip, applying it re-renders Original, editing clean word "bad" →
    "terrible" via the popover input, Apply pasted the EDITED text to
    Notepad exactly (`I believe this is an example of terrible text`) and
    restored the clipboard sentinel.
  - **C7 hotkey-hint follows rebind** (the bug-1 fix: `hotkeys-changed`
    event; previously the review window kept showing the old chord until
    restart).
  - Full rebind cycle + back (C/D), quick-fix path (B), log invariants
    (5 fix-ready, 3 pastes, 0 failed captures).
- User settings (ctrl+space / ctrl+shift+space) backed up and restored by
  the suite; the new release exe was left running.
- Tray icon: registered at startup (setup would abort otherwise) but Win11
  parks it in the overflow flyout — not in the visible UIA taskbar tree
  (T1 INFO). Visual confirmation is a manual step.

**Earlier verified:** engine 2ms/sentence, release fix <100ms/sentence, RAM
128MB, exe 13MB / installer 4MB, End-to-end paste+refocus+clipboard restore;
second session 21/21 (quick hotkey, rebind cycle).

**Questions:** none open.

**Assumptions / known limits:**
- Elevated (admin) target apps refuse injected input (UIPI) — by design.
- Cross-app paste verified in Notepad only (Chromium/Office expected fine).
- Quit-from-tray is not UI-automatable (Win11 overflow flyout needs a human
  first open) — code path is `app.exit(0)`; confirm manually once.
- Word-picker multi-lint spans: clicking a word covered by a multi-word lint
  offers the whole span's suggestions and replaces the whole span.
