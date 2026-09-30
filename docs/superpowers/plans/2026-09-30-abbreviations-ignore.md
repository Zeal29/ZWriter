# Abbreviations, Ignored Words & Alphanumeric Rule — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Teach ZWriter abbreviations (`sc2` → `StarCraft 2`), per-token ignores, and stop flagging letter+digit codes as typos — per the approved spec.

**Architecture:** Two new pure-Rust passes in `Engine::fix` (exact + guessed abbreviation matching, ignore suppression) plus an alphanumeric rule in the harper retain step. Quick path consults `confirm_guesses` to decide paste-vs-review for guesses. Settings persist three new keys; two new Settings sections and three popover buttons teach them.

**Tech Stack:** Tauri 2, harper-core 2.11 (`Dictionary::contains_word` verified on docs.rs), React/TS.

**Spec:** `docs/superpowers/specs/2026-09-30-abbreviations-ignore-design.md` (read together with this plan).

## Global Constraints

- Spans are CHAR indices; all matching runs on `normalize_char`-normalized chars (identical indices).
- Precedence per token: exact abbreviation > ignored word > guessed > harper.
- Exact match: case-insensitive, whitespace-skipping (spaces only, never across punctuation/newlines), word-bounded. Guess: same length, digits identical per position, ≤1 letter substitution or 1 adjacent transposition, ≥1 digit in token, letter-stem not a dictionary word.
- Ambiguous guesses are NEVER auto-applied, regardless of `confirm_guesses`.
- `no_change = (fixed == text) && !lints.iter().any(|l| l.guessed)`.
- Settings keys hand-rolled in BOTH `load_settings` and `save_settings`.
- New settings default: `confirm_guesses: true`.
- Build via `scripts/build.bat`; cargo tests NOT from Git Bash default PATH (AGENTS.md rule 3).
- Lint kinds: `"Abbreviation"` (exact), `"AbbreviationGuess"` (guessed); `guessed: true` marks guesses.

---

### Task 1: Engine — fields, setters, helpers (no behavior change yet)

**Files:** Modify `src-tauri/src/engine.rs`
**Interfaces (produces):**
- `pub fn canonical_trigger(t: &str) -> String` (lowercase, whitespace-stripped)
- `pub fn set_abbreviations(&mut self, pairs: &[(String, String)])`, `pub fn set_ignored_words(&mut self, words: &[String])`
- `LintJson.guessed: bool`
- `pub fn fix(&mut self, text: &str, apply_guesses: bool) -> (String, Vec<LintJson>)`

- [ ] Add `use std::collections::HashSet;` and `LintKind` to imports. Add fields `abbrs: Vec<(Vec<char>, String)>` (trigger chars pre-lowercased, sorted longest-first) and `ignored: HashSet<String>` to `Engine`; init empty in `with_custom_words`.
- [ ] `LintJson` gains `pub guessed: bool` (all existing constructors set `false`).
- [ ] `fix` gains the `apply_guesses: bool` parameter (ignore it for now); update the two internal call sites none — tests call `e.fix(text)` → change all test call sites to `e.fix(text, false)`.
- [ ] Add free fns (full code):

```rust
/// Trigger canonical form: lowercase, all whitespace removed.
pub fn canonical_trigger(t: &str) -> String {
    t.chars().filter(|c| !c.is_whitespace()).flat_map(|c| c.to_lowercase()).collect()
}

/// Extend a span to the enclosing alphanumeric run ("sc2" may arrive as
/// harper tokens "sc"+"2"; the rule must see the whole code).
fn enclosing_alnum_run(chars: &[char], start: usize, end: usize) -> (usize, usize) {
    let a = |c: char| c.is_alphanumeric();
    let mut s = start; while s > 0 && a(chars[s - 1]) { s -= 1; }
    let mut e = end;   while e < chars.len() && a(chars[e]) { e += 1; }
    (s, e)
}

fn is_ignored(chars: &[char], start: usize, end: usize, ignored: &HashSet<String>) -> bool {
    let t: String = chars[start..end].iter().collect::<String>()
        .trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase();
    !t.is_empty() && ignored.contains(&t)
}

/// Whitespace-skipping, case-insensitive trigger match at a word start.
/// Returns the exclusive end index, or None. Never matches unless the match
/// ends on a word boundary; skips only ' ' and '\t' (never newlines/punct).
fn match_trigger(chars: &[char], start: usize, trig: &[char]) -> Option<usize> {
    let mut ci = start;
    for &t in trig {
        while ci < chars.len() && matches!(chars[ci], ' ' | '\t') { ci += 1; }
        if ci >= chars.len() || !chars[ci].eq_ignore_ascii_case(&t) { return None; }
        ci += 1;
    }
    if ci < chars.len() && chars[ci].is_alphanumeric() { return None; }
    Some(ci)
}

/// One edit: same length, digits identical per position, ≤1 letter
/// substitution or one adjacent transposition.
fn one_edit_away(token: &[char], trig: &[char]) -> bool {
    if token.len() != trig.len() { return false; }
    let mut diff = 0; let mut first = 0; let mut second = 0;
    for i in 0..token.len() {
        if token[i].eq_ignore_ascii_case(&trig[i]) { continue; }
        if token[i].is_numeric() || trig[i].is_numeric() { return false; }
        if diff == 0 { first = i; } else if diff == 1 { second = i; }
        diff += 1;
        if diff > 2 { return false; }
    }
    match diff {
        1 => true,
        2 => second == first + 1 && token[first] == trig[second] && token[second] == trig[first],
        _ => false,
    }
}
```

- [ ] Setters:

```rust
/// Swap in the user's abbreviations. Longest trigger first so the most
/// specific wins at a word start; triggers canonicalized defensively.
pub fn set_abbreviations(&mut self, pairs: &[(String, String)]) {
    let mut abbrs: Vec<(Vec<char>, String)> = pairs
        .iter()
        .map(|(t, e)| (canonical_trigger(t), e.trim().to_string()))
        .filter(|(t, e)| !t.is_empty() && t.chars().any(|c| c.is_alphanumeric()) && !e.is_empty())
        .map(|(t, e)| (t.chars().collect(), e))
        .collect();
    abbrs.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(&b.0)));
    self.abbrs = abbrs;
}

pub fn set_ignored_words(&mut self, words: &[String]) {
    self.ignored = words.iter().map(|w| w.trim().to_lowercase()).filter(|w| !w.is_empty()).collect();
}
```

- [ ] `cargo test` must still pass unchanged (16/16) — `fix` callers updated, zero behavior change.
- [ ] Commit: `engine: fields, setters and match helpers for abbreviations`

### Task 2: Engine — the three passes + alphanumeric rule

**Files:** Modify `src-tauri/src/engine.rs`
**Interfaces (produces):** full pipeline per spec §Engine design; lint kinds `Abbreviation` / `AbbreviationGuess` with `guessed` flag.

- [ ] In the `lints.retain(...)` closure add, after the zwriter check:
  - ignore check: `if !self.ignored.is_empty() && is_ignored(&norm_chars, l.span.start, l.span.end, &self.ignored) { return false; }`
  - alphanumeric rule: `if matches!(l.lint_kind, LintKind::Spelling) { let (s, e) = enclosing_alnum_run(&norm_chars, l.span.start, l.span.end); let run = &norm_chars[s..e]; if run.iter().any(|c| c.is_alphabetic()) && run.iter().any(|c| c.is_numeric()) { return false; } }`
- [ ] After `remove_overlaps(&mut lints)`, run the taught passes and merge:

```rust
let (mut taught, guesses) = self.abbr_lints(&norm_chars);
taught.extend(guesses);
// Taught lints own their span: harper findings inside them are noise.
lints.retain(|l| !taught.iter().any(|t| l.span.start < t.end && t.start < l.end));
```

- [ ] Build `meta` for harper lints as today (set `guessed: false`), then `meta.extend(taught)`.
- [ ] Replace the apply loop with LintJson-driven application:

```rust
meta.sort_by_key(|l| Reverse(l.start));
let mut chars: Vec<char> = text.chars().collect();
for lint in &meta {
    // A guess is applied only when asked, and never when ambiguous
    // (several candidates = no safe auto-answer).
    if lint.guessed && (!apply_guesses || lint.suggestions.len() != 1) { continue; }
    match lint.suggestions.first() {
        Some(Some(rep)) => { let mut rep: Vec<char> = rep.chars().collect(); rep.extend(chars[lint.end..].iter().copied()); }
        Some(None) => {}
        None => continue,
    }
    let tail: Vec<char> = chars[lint.end..].to_vec();
    let head: Vec<char> = chars[..lint.start].to_vec();
    let mid: Vec<char> = match lint.suggestions.first() {
        Some(Some(rep)) => rep.chars().collect(),
        _ => Vec::new(), // None = remove the span
    };
    chars = head; chars.extend(mid); chars.extend(tail);
}
```

(Simplify while writing — the sketch above computes `rep` twice; final code keeps one clean match.) `return (chars.into_iter().collect(), meta);`

- [ ] Add `abbr_lints`:

```rust
/// Exact-trigger pass over word starts, then a guessed pass over the
/// remaining alphanumeric runs. Spans index the ORIGINAL text.
fn abbr_lints(&self, chars: &[char]) -> (Vec<LintJson>, Vec<LintJson>) {
    let mut exact: Vec<LintJson> = Vec::new();
    let mut guesses: Vec<LintJson> = Vec::new();
    if self.abbrs.is_empty() { return (exact, guesses); }
    let alnum = |c: char| c.is_alphanumeric();
    let mut i = 0;
    while i < chars.len() {
        if !alnum(chars[i]) || (i > 0 && alnum(chars[i - 1])) { i += 1; continue; }
        let mut matched = false;
        for (trig, expansion) in &self.abbrs {
            if let Some(end) = match_trigger(chars, i, trig) {
                let span_text: String = chars[i..end].iter().collect();
                exact.push(LintJson {
                    start: i, end,
                    message: format!("Abbreviation: {span_text} → {expansion}"),
                    kind: "Abbreviation".to_string(),
                    replacement: Some(expansion.clone()),
                    suggestions: vec![Some(expansion.clone())],
                    priority: 0,
                    guessed: false,
                });
                i = end; matched = true; break;
            }
        }
        if !matched { i += 1; }
    }
    let mut j = 0;
    while j < chars.len() {
        if !alnum(chars[j]) { j += 1; continue; }
        let mut k = j; while k < chars.len() && alnum(chars[k]) { k += 1; }
        let overlaps_exact = exact.iter().any(|l| l.start < k && j < l.end);
        if !overlaps_exact {
            if let Some(g) = self.guess_lint(&chars[j..k], j) { guesses.push(g); }
        }
        j = k;
    }
    (exact, guesses)
}

/// One candidate trigger one-edit-away → a single-suggestion guess;
/// two or more → one ambiguous lint listing every expansion. Real-word
/// stems ("set2" → "set") and ignored tokens are never guessed.
fn guess_lint(&self, run: &[char], start: usize) -> Option<LintJson> {
    if !run.iter().any(|c| c.is_numeric()) { return None; }
    let letters: Vec<char> = run.iter().filter(|c| !c.is_numeric()).copied().collect();
    if letters.is_empty() || self.dict.contains_word(&letters) { return None; }
    let token_lc = run.iter().collect::<String>().to_lowercase();
    if self.ignored.contains(&token_lc) { return None; }
    let mut candidates: Vec<String> = Vec::new();
    for (_, expansion) in &self.abbrs {
        // self.abbrs stores pre-collected trigger chars alongside
        if candidates.len() < 20 { /* hard cap: triggers are user's own */ }
    }
    let mut trig_matches: Vec<&String> = Vec::new();
    for (trig, expansion) in &self.abbrs {
        if one_edit_away(run, trig) && !trig_matches.contains(&expansion) { trig_matches.push(expansion); }
    }
    if trig_matches.is_empty() { return None; }
    let span_text: String = run.iter().collect();
    let suggestions: Vec<Option<String>> = trig_matches.into_iter().map(Some).collect();
    let message = if suggestions.len() == 1 {
        format!("Guessed abbreviation: {span_text} → {}?", suggestions[0].clone().unwrap_or_default())
    } else {
        format!("{span_text}: close to {} abbreviations — pick one", suggestions.len())
    };
    Some(LintJson {
        start, end: start + run.len(),
        message,
        kind: "AbbreviationGuess".to_string(),
        replacement: suggestions.first().cloned().flatten(),
        suggestions,
        priority: 0,
        guessed: true,
    })
}
```

(Write the final `guess_lint` cleanly — the `candidates` scratch loop in the sketch is dead and must not appear.)

- [ ] New engine unit tests (full code, appended to `mod tests`; helpers first):

```rust
fn engine_with_abbrs(pairs: &[(&str, &str)]) -> Engine {
    let mut e = Engine::new();
    e.set_abbreviations(&pairs.iter().map(|(t, x)| (t.to_string(), x.to_string())).collect::<Vec<_>>());
    e
}
fn engine_with_ignored(words: &[&str]) -> Engine {
    let mut e = Engine::new();
    e.set_ignored_words(&words.iter().map(|w| w.to_string()).collect::<Vec<_>>());
    e
}
fn span_texts(lints: &[LintJson], text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    lints.iter().map(|l| chars[l.start..l.end].iter().collect()).collect()
}
```

Tests:
1. `exact_abbreviation_expands` — `engine_with_abbrs(&[("sc2","StarCraft 2")])`; fix("a tool for sc2 here", false) → contains "StarCraft 2", some lint kind=="Abbreviation", `guessed==false`.
2. `exact_matches_any_case_and_spacing` — "SC2", "sc 2", "s c 2" each expand; "(SC2)." keeps punctuation; span for "sc 2" covers all 4 chars.
3. `word_boundary_required` — "massc2" and "sc2x" unchanged, no Abbreviation lints.
4. `digits_must_match` — "sc3" unchanged, no lints of kind Abbreviation/AbbreviationGuess.
5. `guessed_match_suggested_not_applied` — "se2" with (false) → fixed == original, exactly one guessed lint with suggestions==["StarCraft 2"]; same text with (true) → applied.
6. `transposition_counts_as_one_edit` — only sc2 defined; "cs2" and "cs 2" → guessed StarCraft 2.
7. `second_trigger_wins_exactly` — abbrs sc2 + ("cs2","Counter-Strike 2"); "cs 2" → exact Counter-Strike 2 (guessed==false).
8. `ambiguous_guess_lists_all_and_never_applies` — abbrs sc2 + ("sl2","System Link 2"); "sd2" → one guessed lint, 2 suggestions; fixed == original EVEN with apply_guesses=true.
9. `alphanumeric_tokens_not_spelling_flagged` — default engine; "sc2 s12 mp3 x86 ok" → zero Spelling lints; control "beleive" still flagged (same test).
10. `dictionary_stem_not_guessed` — "set2 ok" (stem "set") → no guesses.
11. `ignored_word_untouched` — engine_with_ignored(&["se2"]); "the se2 id stays" → unchanged, zero lints.
12. `ignore_loses_to_exact_trigger` — ignored se2 AND abbr ("se2","S Extra 2"); "se2" → expands.
13. `mixed_sentence_back_to_front` — "i beleive sc2 is an exampel" → both "believe" and "StarCraft 2" present.
14. `clean_codes_round_trip` — "build x86 s12 mp3 done." unchanged byte-for-byte, no lints.

- [ ] Run: `cargo test` (dev.bat env or filtered PATH per AGENTS.md) → 16 old + 14 new = 30 pass.
- [ ] Commit: `engine: abbreviation/guess/ignore passes + alphanumeric rule`

### Task 3: Settings & quick path (flow.rs)

**Files:** Modify `src-tauri/src/flow.rs`
**Interfaces:** `Settings { abbreviations: Vec<Abbreviation>, ignored_words: Vec<String>, confirm_guesses: bool }`, `pub struct Abbreviation { pub trigger: String, pub expansion: String }`.

- [ ] Add `Abbreviation` struct (Debug, Clone, Serialize, PartialEq, `#[serde(rename_all = "camelCase")]`). Extend `Settings` + `default_settings()` (`confirm_guesses: true`).
- [ ] Add `fn no_change(fixed: &str, original: &str, lints: &[LintJson]) -> bool` per Global Constraints.
- [ ] Rewrite `capture_and_fix` step 3–4: run `fix(&text, false)`; build payload with the new `no_change` fn; in the `quick` block:

```rust
if quick {
    let has_guess = payload.lints.iter().any(|l| l.guessed);
    if !has_guess {
        if payload.no_change { println!("[zwriter] text already clean, nothing to paste ({fix_ms}ms)"); return; }
        store_pending(&state, &payload, target_hwnd, orig_clipboard);
        paste_pending(&app);
        let mut e = entry.clone(); e.applied = true;
        push_history(&state, e);
        return;
    }
    if !confirm_guesses {
        // Option B: auto-apply unambiguous guesses; ambiguous still asks.
        let (fixed2, lints2) = state.engine.lock().unwrap().fix(&payload.original, true);
        let ambiguous = lints2.iter().any(|l| l.guessed && l.suggestions.len() > 1);
        if !ambiguous {
            let payload2 = FixReady {
                no_change: no_change(&fixed2, &payload.original, &lints2),
                fix_ms, original: payload.original.clone(), fixed: fixed2.clone(),
                lints: lints2.clone(), user_edited: false,
            };
            store_pending(&state, &payload2, target_hwnd, orig_clipboard);
            paste_pending(&app);
            let mut e = entry.clone(); e.fixed = fixed2; e.fix_count = lints2.len(); e.applied = true;
            push_history(&state, e);
            return;
        }
        // ambiguous → fall through to the review window
    }
    // guesses + confirm on (or ambiguous): fall through to review window
}
```

Extract `store_pending(&AppState, &FixReady, Option<isize>, Option<String>)` from the two existing inline `PendingFix` constructions (review path + quick path reuse it). Review path below stays as-is but uses `store_pending` and its `PendingFix.fix_count` comes from `payload.lints.len()` as today.
- [ ] Update `flow.rs` tests: add `confirm_guesses_defaults_true` (`assert!(default_settings().confirm_guesses)`).
- [ ] `cargo test` → all pass.
- [ ] Commit: `flow: settings for abbreviations/ignored/confirm_guesses + quick-path guess routing`

### Task 4: Persistence & commands (lib.rs)

**Files:** Modify `src-tauri/src/lib.rs`
**Interfaces:** commands `add_abbreviation(trigger, expansion) -> Vec<Abbreviation>`, `remove_abbreviation(trigger) -> Vec<Abbreviation>`, `add_ignored_word(word) -> Vec<String>`, `remove_ignored_word(word) -> Vec<String>`, `set_confirm_guesses(enabled) -> bool`; events `abbreviations-changed {abbreviations}`, `ignored-words-changed {words}`.

- [ ] `use flow::Abbreviation;` and `use engine::canonical_trigger;`.
- [ ] `load_settings`: parse `abbreviations` (array of objects; per-item `.get("trigger")?/.get("expansion")?` as_str), `ignoredWords` (as `customWords`), `confirmGuesses` (as_bool). After collecting abbreviations, dedup by canonical trigger (last wins).
- [ ] `save_settings`: add `"abbreviations": s.abbreviations`, `"ignoredWords": s.ignored_words`, `"confirmGuesses": s.confirm_guesses` to the `json!` block. **Both sides, per Global Constraints.**
- [ ] `fn apply_teachings(app: &AppHandle)` mirroring `apply_custom_words` (clone settings, call `set_abbreviations` with `(trigger, expansion)` pairs + `set_ignored_words`).
- [ ] Commands (validate → mutate → `apply_teachings` + `save_settings` → emit → return list):
  - `add_abbreviation`: canonical trigger; reject empty/alnum-less trigger ("type an abbreviation first") and empty expansion ("type the full term"); replace-on-same-canonical-trigger, else push.
  - `remove_abbreviation`: retain where `canonical_trigger(t) != target`; idempotent.
  - `add_ignored_word` / `remove_ignored_word`: clone `add_custom_word`/`remove_custom_word` shape (split_whitespace, edge-punct strip, case-insensitive dedup/retain); emit `ignored-words-changed {words}`.
  - `set_confirm_guesses(enabled)`: persist, return enabled.
  - abbreviation events: `app.emit("abbreviations-changed", json!({ "abbreviations": list }))`.
- [ ] `setup`: call `apply_teachings(app.handle())` next to `apply_custom_words`. Register all five commands in `generate_handler!`.
- [ ] `cargo test` → pass (add none; persistence is smoke-covered).
- [ ] Commit: `lib: persist + teach abbreviations, ignored words, confirm-guesses`

### Task 5: Frontend — types, Settings sections, popover buttons

**Files:** Modify `src/types.ts`, `src/Settings.tsx`, `src/Review.tsx`, `src/App.css`
**Interfaces:** `Abbreviation {trigger: string; expansion: string}`, `Settings.abbreviations/ignoredWords/confirmGuesses`, `Lint.guessed`.

- [ ] `types.ts`: `export interface Abbreviation { trigger: string; expansion: string }`; `Settings` gains `abbreviations: Abbreviation[]`, `ignoredWords: string[]`, `confirmGuesses: boolean`; `Lint` gains `guessed: boolean`.
- [ ] `Settings.tsx`: state `newTrigger`, `newExpansion`, `newIgnored`; handlers `addAbbreviation`/`removeAbbreviation`, `addIgnoredWord`/`removeIgnoredWord`, `toggleConfirmGuesses` (optimistic set + `set_confirm_guesses`, rollback + error line on failure, mirroring `toggleAutostart`); listeners for `abbreviations-changed {abbreviations}` and `ignored-words-changed {words}` merging into local state.
- [ ] Sections JSX (between Custom dictionary and Start with Windows), with exact aria-labels:
  - Abbreviations: `aria-label="New abbreviation trigger"` (placeholder `sc2`), `aria-label="New abbreviation expansion"` (placeholder `StarCraft 2`), Add button `aria-label="Add abbreviation"`; chips `sc2 → StarCraft 2` with `aria-label="Remove abbreviation {trigger}"`. Empty state: "No abbreviations yet. Type a shortcut and its full term — or add one from the review window."
  - Ignored words: input `aria-label="New ignored word"`, Add `aria-label="Add ignored word"`, chips with `aria-label="Remove ignored word {w}"`. Sub: "Codes and IDs (s12, mp3) are never flagged and never suggested."
  - Checkbox row "Confirm guessed abbreviations" (`aria-label="Confirm guessed abbreviations"`, bound `settings.confirmGuesses`), sub "Quick fix asks before applying a near-match like se2 → StarCraft 2".
- [ ] `Review.tsx`: shared `rerunEngine` for `custom-words-changed` + `abbreviations-changed` + `ignored-words-changed` listeners; handlers `addAsAbbreviation` (trigger = `stripWord(picked.word)`, expansion = `editVal.trim()`) and `ignoreWord` (`add_ignored_word`). Popover JSX after the dictionary button:

```tsx
{stripWord(picked.word) && (
  <button type="button" className="dict-add" aria-label="Add as abbreviation"
    title="Type the full term in the box below first"
    disabled={!editVal.trim() || editVal.trim() === picked.word.trim()}
    onClick={addAsAbbreviation}>
    + Add as abbreviation
  </button>
)}
{stripWord(picked.word) && (
  <button type="button" className="dict-add" aria-label="Ignore word"
    title="Never flag or suggest this token"
    onClick={ignoreWord}>
    + Ignore word
  </button>
)}
```

- [ ] `App.css`: `.add-abbrev` (flex row, two inputs like `.add-word`); popover buttons reuse `.dict-add` spacing. Check `.wordpop` stacking (buttons are block-level already).
- [ ] `pnpm build` green (tsc + vite).
- [ ] Commit: `ui: teach abbreviations and ignored words from Settings and the word popover`

### Task 6: Release build + unit suite green

- [ ] `scripts/build.bat` (taskkills first) → clean release + NSIS.
- [ ] Full `cargo test` once more in release env (30 + 1 flow test).
- [ ] Commit any fixups; tag nothing yet.

### Task 7: Smoke suite sections J–N

**Files:** Modify `scripts/smoke-test.ps1` (reuse existing helpers: `Get-ZWindow` enum-window finder, `New-Selection`, UIA `Click-Button`/`Has-Button` by aria-label, keybd_event chords, settings backup/restore).

- [ ] **J — Settings teaching:** open Settings; add abbreviation `sc2`/`StarCraft 2` via the two new inputs + Add (aria-labels); chip visible; add ignored word `s12`; toggle "Confirm guessed abbreviations" off and on (checkbox state observable); add abbreviation `sl2`/`System Link 2` (for M's ambiguity); close Settings.
- [ ] **K — review path:** paste "is there a tool for sc2 here" in Notepad, select, press fix hotkey (review chord); assert review window up, Original pane has word with `aria-label` title Abbreviation (or the suggestion chip "StarCraft 2" exists); Apply; assert Notepad text contains "StarCraft 2".
- [ ] **L — quick exact:** fresh line "tool sc2 x", select, quick hotkey; assert no review window appeared within 3s and text became "tool StarCraft 2 x"; clipboard sentinel restored.
- [ ] **M — quick guess ×3 states:** (a) confirm ON: "se2" → quick hotkey → review window OPENS with chip "StarCraft 2", Skip; (b) toggle confirm OFF in Settings: "se2" → quick → pastes "StarCraft 2", no window; (c) "sd2" (one edit from sc2 AND sl2) → quick → window opens even with confirm OFF; restore checkbox ON.
- [ ] **N — popover:** capture "the se2 thing", open popover on `se2`, type `Sea Extra 2` in edit box, click `Add as abbreviation` → Fixed pane live-updates to expansion; next line: ignore flow on `zz9` → not flagged after re-capture. Cleanup: remove test abbreviations (`sl2`, `se2` if added) via Settings chips; suite's existing settings restore runs.
- [ ] Each section: one retry on capture races (existing pattern); log invariants read BOTH stdout/stderr files.
- [ ] Run `scripts/smoke-test.ps1` against the release exe → ALL sections pass (prior A–L + new J–N).
- [ ] Commit: `smoke: sections J-N cover abbreviations, guesses, ignores`

### Task 8: Docs & capture

- [ ] `AGENTS.md`: §Vocabulary terms (Abbreviation/trigger/expansion, guessed, ambiguous guess, ignored word, alphanumeric rule, confirm guesses); new rule 21: settings keys must be added to load AND save (hand-rolled); precedence chain; smoke sections listed in Dev loop.
- [ ] `README.md`: teach-options paragraph + checkbox mention.
- [ ] Commit: `docs: v0.2 abbreviations & ignored words`
