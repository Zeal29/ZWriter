# ZWriter v0.2 — Abbreviations, Ignored Words & the Alphanumeric Rule

Design spec — 2026-09-30, revised 2026-10-01 after first user feedback.
Status: implemented; see §Revision 1 for the changes that supersede the
original quick-path and ignore-list design.

## Problem

ZWriter flags codes it does not know as spelling errors and offers nonsense
suggestions. Typed: `is there a tool for sc2 so i can set the hot keys…`
ZWriter suggested *sci / sch / sac* for `sc2`. Worse, there is no way to say
"leave `s12` alone" — the only teach tools today are the custom dictionary
("this is a word", wrong for a code) and a self-abbreviation hack (also wrong).

Three user-approved decisions came out of the discussion:

1. **Option A + toggle**: quick fix pastes exact abbreviations instantly but
   asks (review window) before applying a *guessed* one; a Settings checkbox
   (default ON) can turn asking off — except ambiguous guesses always ask.
2. **Both teaching surfaces in v1**: a Settings section AND an
   "Add as abbreviation" button in the review window's word popover.
3. **A third teach option, Ignore**: a per-token "never flag, never suggest"
   list, distinct from the dictionary.

Explicitly **out of scope (phase 2, not built now)**: any AI/model signal
(e.g. Julia-1) for ranking suggestions or judging confidence.

## Terminology (for AGENTS.md §Vocabulary)

- **Abbreviation** = a user-taught pair: **trigger** (`sc2`) → **expansion**
  (`StarCraft 2`). Canonical trigger form: lowercased, all whitespace removed.
- **Exact match** = the text contains the canonical trigger verbatim
  (case-insensitive, whitespace-insensitive in the text: `SC2`, `sc 2`,
  `s c 2` all match trigger `sc2`).
- **Guessed match** (fuzzy) = the text token is *not* a trigger but is one
  letter-edit away from exactly one. A guess is a suggestion, never a fact.
- **Ambiguous guess** = the token is one letter-edit from **two or more**
  triggers. Ambiguous guesses always open the review window, regardless of
  the checkbox.
- **Ignored word** = an exact token the user never wants touched: no spelling
  flag, no abbreviation guess, no suggestion.
- **Alphanumeric rule** (shape rule) = a token that mixes letters and digits
  (`sc2`, `s12`, `mp3`, `x86`) is never flagged as a spelling error.
- **Confirm guesses** = the Settings checkbox governing the quick path.

## Engine design (`src-tauri/src/engine.rs`)

`Engine` gains three cheap, pure-Rust fields — no harper structures, no
LintGroup rebuilds: `abbrs: Vec<(String /*canonical*/, String /*expansion*/)>`,
`ignored: HashSet<String>` (lowercased), set via `set_abbreviations()` /
`set_ignored_words()`.

`fix(&mut self, text, apply_guesses: bool) -> (String, Vec<LintJson>)` — the
pipeline becomes:

1. Smart-quote normalize (unchanged), build Document, `group.lint` (unchanged).
2. Retain pass (unchanged rules, plus one): also drop harper **Spelling** and
   **Capitalization** lints whose span, extended to the enclosing
   alphanumeric run, mixes letters and digits. Extend-to-run handles harper
   splitting `sc2` into `sc` + `2`; Capitalization is included because harper
   nags `mp3` → `MP3` ("canonical spelling is all-caps").
3. `remove_overlaps` (unchanged).
4. **Abbreviation pass** (new). Scan word starts (a char preceded by a
   non-alphanumeric). At each start, try each trigger longest-first, comparing
   case-insensitively and **skipping whitespace on the text side only**
   (comparison stops at any non-alphanumeric, non-space char — it never jumps
   punctuation). A match must end at a word boundary (next char is not
   alphanumeric). Each exact match emits a lint:
   `kind: "Abbreviation"`, `guessed: false`, `priority: 0`,
   `suggestions: [expansion]`, span = the consumed original chars.
5. **Guess pass** (new). At each word start not already exact-matched, match
   trigger-shaped with one tolerated in-word letter edit — substitution or
   adjacent transposition, digits identical at every position, never across a
   skipped space, and the trigger must carry ≥1 digit (so plain words can
   never match, and digit-less triggers only ever expand exactly). The token
   must not be ignored, and its letter-stem must **not be a dictionary word
   at 3+ letters** (probe-verified: harper's curated FST contains two-letter
   entries — `se`, `cs`, `sd` are "words" — so guarding those would kill
   exactly the two-letter+digit codes this feature serves; `set2` → `set`
   stays protected).
   - Exactly one trigger matches → one lint `kind: "AbbreviationGuess"`,
     `guessed: true`, `suggestions: [expansion]`.
   - Two or more → one lint `guessed: true` whose `suggestions` lists every
     candidate expansion (review window shows a chip per candidate).
   - Guesses are **applied into the fixed text only when `apply_guesses` is
     true**, and even then never when ambiguous (see §Quick path).
6. **Ignore pass** (new). Drop any remaining harper lint whose span text,
   edge-punctuation-stripped and lowercased, is in `ignored`. (Exact
   abbreviations already won above; ignore beats guesses and harper.)
7. Merge abbreviation + guess lints with the surviving harper lints, dropping
   harper lints that overlap an exact-abbreviation span. Sort by
   `Reverse(span.start)` and apply back-to-front (exact + harper always;
   guesses per `apply_guesses`), unchanged mechanics.

Precedence, per token: **exact abbreviation > ignored word > guessed
abbreviation > harper**.

`no_change` semantics: `fixed == text` **and** no unapplied guess lints —
"a guess is pending" is something for the user to do, so a pure-guess capture
must reach the review window, not the silent "already clean" return.

`LintJson` gains `pub guessed: bool` (serialized; TS `Lint.guessed: boolean`).

### Worked examples (all covered by unit tests)

| Text (trigger `sc2`→StarCraft 2 unless noted) | Result |
|---|---|
| `sc2` / `SC2` / `sc 2` / `s c 2` | exact → `StarCraft 2` |
| `sc2.` / `(sc2)` | exact; punctuation untouched |
| `sc2x`, `massc2` | no match (word-boundary rule) |
| `sc3` | nothing (digits must match) |
| `se2`, `cs 2` | guess → StarCraft 2 chip |
| + trigger `cs2`→Counter-Strike 2, text `cs 2` | exact → Counter-Strike 2 |
| + triggers `sc2`,`sl2`, text `sd2` | ambiguous guess, 2 chips |
| `s12`, `mp3`, `x86` (no abbreviation defined) | alphanumeric rule: no spelling lint |
| ignored `se2`, text `se2` | untouched, no guess offered |
| ignored `se2`, trigger `se2` defined | exact expansion wins |

## Quick path decision table (`src-tauri/src/flow.rs`)

`capture_and_fix(app, quick)` — quick branch becomes:

| Situation (engine run with `apply_guesses=false`) | Behavior |
|---|---|
| No lints at all | silent return (unchanged: "already clean") |
| Only exact/harper lints | paste `fixed` (unchanged) |
| Guessed lints, `confirm_guesses = true` (default) | open review window (same emit/show path as the review hotkey); chips offer the expansion(s) |
| Guessed lints, unchecked, all unambiguous | re-run `fix(text, apply_guesses=true)`, paste |
| Ambiguous guess present, unchecked | open review window — no safe auto-answer |

The review path (fix hotkey) is unchanged: always `apply_guesses=false`, so
guesses always appear as chips, never pre-applied. `fix_text` (demo box) and
`update_pending` (picker edits) likewise run with `apply_guesses=false`.

The redefined `no_change` keeps the review window's existing Apply gate
(`disabled = noChange && !userEdited`) correct with **no frontend change**:
a pending guess makes `noChange` false, so Apply is enabled exactly when
there are chips to commit; committing re-runs the engine via
`update_pending` as today.

## Data & persistence (`src-tauri/src/lib.rs`)

`Settings` gains:
- `abbreviations: Vec<Abbreviation>` — `struct Abbreviation { trigger: String, expansion: String }` (camelCase serde: `{trigger, expansion}`),
- `ignored_words: Vec<String>`,
- `confirm_guesses: bool` (default **true**).

`settings.json` keys: `abbreviations: [{"trigger","expansion"}]`,
`ignoredWords: [..]`, `confirmGuesses: bool`. NOTE: `load_settings`/`save_settings`
hand-roll each key (`saved.get(...)`, the `json!` macro) — all three keys must
be added to BOTH functions or they silently won't persist. On load,
canonicalize triggers (lowercase, strip whitespace) and dedup by trigger
(last wins); ignored words dedup case-insensitively like `customWords`.

Engine refresh after changes is a plain `set_*` call — no LintGroup rebuild.

## Commands & events

Mirror the custom-word pattern (validate → mutate settings → persist → emit):

- `add_abbreviation(trigger, expansion) -> Vec<Abbreviation>` — canonicalize
  trigger; require non-empty expansion; same trigger replaces (update); emit
  `abbreviations-changed {abbreviations}`.
- `remove_abbreviation(trigger) -> Vec<Abbreviation>` — idempotent; same emit.
- `add_ignored_word(word) -> Vec<String>` / `remove_ignored_word(word)` —
  bulk-split on whitespace, edge-punctuation strip (same as `add_custom_word`);
  emit `ignored-words-changed {words}`.
- `set_confirm_guesses(enabled: bool) -> bool` — persist only (flow reads it
  live at quick-fix time); no event needed.

Both new events are broadcast with `app.emit` (all windows), like
`custom-words-changed` since the Settings-staleness fix.

## UI

### Settings window (`src/Settings.tsx`)

- **Abbreviations** section (below Custom dictionary): two inputs —
  `aria-label="New abbreviation trigger"` (placeholder "sc2") and
  `aria-label="New abbreviation expansion"` (placeholder "StarCraft 2") —
  plus Add. Chips render `sc2 → StarCraft 2` with an ✕
  (`aria-label="Remove abbreviation sc2"`).
- **Ignored words** section: one input + chips, same pattern as custom words
  (`aria-label="New ignored word"` / `Remove ignored word …`).
- **Confirm guessed abbreviations** checkbox row (`aria-label` same), sub:
  "Quick fix asks before applying a near-match like se2 → StarCraft 2".
  Binds to `set_confirm_guesses`.
- Listens to `abbreviations-changed` / `ignored-words-changed` to live-sync
  (the focus-refetch already present covers the rest).

### Review window popover (`src/Review.tsx`)

Three buttons under the suggestion chips, for any picked word with a
non-empty `stripWord`:

- **+ Add as abbreviation** (`aria-label="Add as abbreviation"`): trigger =
  the stripped picked word, expansion = the popover's edit input
  (`editVal`). Disabled (with tooltip "type the full term in the box above")
  while `editVal.trim()` is empty or equals the word. No re-edit confirm —
  committing closes the popover; the engine re-run comes from the
  `abbreviations-changed` listener.
- **+ Ignore word** (`aria-label="Ignore word"`): calls `add_ignored_word`.
- **+ Add to dictionary** — unchanged.

Review listens to both new events and re-runs the engine on the displayed
text (same handler shape as the `custom-words-changed` listener). Guessed
chips render like normal suggestion chips; an ambiguous guess simply shows
several chips.

`src/types.ts`: `Settings.abbreviations: {trigger: string; expansion: string}[]`,
`Settings.ignoredWords: string[]`, `Settings.confirmGuesses: boolean`,
`Lint.guessed: boolean`.

## Error handling

- Bad `add_abbreviation` input (empty trigger/expansion, trigger without any
  alphanumeric) → `Err("…")`, surfaced in Settings' existing error line.
- Duplicate trigger → replaces the expansion (not an error).
- Zero abbreviations / ignored words → passes are no-ops; perf unchanged
  (linear scans over a short user list, noise next to the 2 ms/sentence harper
  run).
- Hand-edited settings.json with malformed new keys → per-key overlay parse
  (`.get().and_then(...)` style) skips them; next save rewrites valid JSON.

## Testing

**Unit (engine.rs)** — the worked-examples table above, one assert each, plus:
mixed sentence ("i beleive sc2 is an exampel") fixes the typo AND expands,
back-to-front application intact; guess not applied when `apply_guesses=false`
but applied when true; ambiguous never applied; alphanumeric rule does not
touch real-word typos (`beleive` still flagged). Existing 16 tests must stay
green (the shape rule changes harper lint output only for letter+digit tokens).

**Unit (flow.rs)** — none new (quick branch needs an AppHandle; covered by
smoke suite).

**Smoke suite (`scripts/smoke-test.ps1`)** — new sections against the release
exe, settings backed up/restored as today:
- J: Settings UI — add `sc2 → StarCraft 2` and ignored word `s12` via UIA;
  chips visible; checkbox toggles and persists across app restart.
- K: review path — capture "is there a tool for sc2 here", review shows the
  `StarCraft 2` chip, Apply pastes the expansion into Notepad.
- L: quick path, exact — select `sc2`, quick hotkey, `StarCraft 2` pasted, no
  window.
- M: quick path, guess — select `se2`, quick hotkey with confirm ON → review
  window opens; uncheck → pastes `StarCraft 2`; ambiguous (`sd2` with a second
  trigger) → window even when unchecked.
- N: popover — "+ Add as abbreviation" on a picked word, "+ Ignore word"
  unflags a code live.
- Log invariants extended (fix-ready / paste counts per section).

## Docs to update with the release

- `AGENTS.md`: §Vocabulary (the terms above), a non-obvious rule for the
  hand-rolled settings keys (add to load AND save or they silently don't
  persist), and the precedence chain.
- `README.md`: the three teach options (abbreviation / dictionary / ignore)
  and the quick-fix confirm checkbox.

## Revision 1 (2026-10-01, first user feedback — supersedes parts of this spec)

1. **Ignored words are SESSION-ONLY.** Never persisted (no `ignoredWords`
   key in settings.json — removed from load AND save), the list starts
   empty every launch. Rationale: ignore means "leave this one alone for
   now"; permanent vocabulary belongs in the custom dictionary.
2. **Popover teach flow fixed.** The edit box moved ABOVE the teach
   buttons and "+ Add as abbreviation" is never disabled: clicking it
   without a full term focuses the box and shows a hint instead of
   silently doing nothing (the original disabled state read as "broken").
3. **The single `confirmGuesses` checkbox is replaced by TWO** (both
   default OFF = open the window with the guess as a chip):
   - `skipGuessWindow` — quick fix never opens the review window for an
     unsure (guessed) abbreviation;
   - `autoApplyGuesses` — quick fix uses the guessed expansion anyway
     (pasted directly, or pre-applied when the window opens).
   Matrix: (off,off) window+chip · (on,off) word left as typed, silently ·
   (on,on) apply+paste, no window · (off,on) window with guess pre-applied.
   The old "ambiguous always asks" carve-out stands: a word one edit from
   TWO triggers always opens the window as chips under every combination.
   Old settings.json files may keep `confirmGuesses`; it is ignored and
   dropped on the next save.
4. **Guessed words look different from errors**: dashed violet underline
   (`.word.guess`) instead of the error highlight, so an unsure match is
   visibly a question, not a correction.
5. **Clipboard fix found while testing**: every silent quick-path
   early-return ("already clean", and the new skip-guess row) must restore
   the saved clipboard — the capture cleared it, so returning without
   pasting would otherwise eat the user's clipboard content.



## Risks / open points

- ~~`contains_word` on `MergedDictionary`~~ RESOLVED: `Dictionary::contains_word(&[char]) -> bool` exists (docs.rs, in scope via `use ... spell::Dictionary as _`).
- The alphanumeric rule is a behavior change: a *misspelled* letter+digit
  token (rare — `belive2`) is no longer flagged. Accepted: the class of false
  positives it kills (sc2/s12/mp3) is 100× more common.
- Smart-quote normalization already maps ‘’“”→'"; triggers containing
  apostrophes are out of scope (canonical triggers are alnum-only after
  whitespace strip — validation enforces this).
- Probe-verified harper quirk: sentence-start Capitalization fires on "is
  there a tool for sc2 here" but NOT on "the sc2 game" — expected outputs in
  tests are asserted against probed engine behavior, not assumed grammar.
- The popover's ignore/dictionary buttons operate on the STRIPPED word
  (`stripWord`); a token still present in the custom dictionary arrives
  pre-whitelisted regardless of the ignore list — dictionary beats ignore
  for spelling (whitelisted words are never linted at all).
