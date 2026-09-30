//! Harper grammar engine wrapper.
//!
//! Spans are CHAR indices (Unicode scalar values), not byte offsets — the
//! frontend must convert before slicing JS strings (see Review.tsx).

use harper_core::linting::{LintGroup, LintKind, Linter, Suggestion};
use harper_core::parsers::PlainEnglish;
use harper_core::spell::{Dictionary as _, FstDictionary, MergedDictionary, MutableDictionary};
use harper_core::{remove_overlaps, Dialect, DictWordMetadata, Document};
use serde::Serialize;
use std::cmp::Reverse;
use std::collections::HashSet;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LintJson {
    pub start: usize,
    pub end: usize,
    pub message: String,
    pub kind: String,
    pub replacement: Option<String>,
    /// Every suggestion the engine offers for this lint, deduped, in engine
    /// order. None = remove the span; Some = replacement text (an
    /// InsertAfter suggestion is flattened to span_text + inserted).
    pub suggestions: Vec<Option<String>>,
    pub priority: u8,
    /// True for abbreviation GUESSES (one-edit-away matches): they are
    /// suggestions, never auto-applied without the caller asking for it,
    /// and never auto-applied when ambiguous.
    pub guessed: bool,
}

pub struct Engine {
    group: LintGroup,
    dict: Arc<MergedDictionary>,
    /// User-taught abbreviations: canonical trigger chars (lowercase, no
    /// whitespace) -> expansion, sorted longest-trigger-first so the most
    /// specific wins at a word start.
    abbrs: Vec<(Vec<char>, String)>,
    /// Lowercased tokens the user never wants touched (no flag, no guess).
    ignored: HashSet<String>,
}

impl Engine {
    /// LintGroup init is expensive — build once and reuse.
    pub fn new() -> Self {
        Engine::with_custom_words(&[])
    }

    /// `words` = the user's custom dictionary: whitelisted words are never
    /// flagged. Built on the curated FST via a zero-copy MergedDictionary.
    pub fn with_custom_words(words: &[String]) -> Self {
        let dict = build_dictionary(words);
        Engine {
            group: LintGroup::new_curated(dict.clone(), Dialect::American),
            dict,
            abbrs: Vec::new(),
            ignored: HashSet::new(),
        }
    }

    /// Swap in a new word list. Rebuilds dictionary + LintGroup — the
    /// canonical harper-ls pattern; the curated FST is process-cached, so
    /// this is cheap (no dictionary copying, ~100 small linter structs).
    pub fn set_custom_words(&mut self, words: &[String]) {
        self.dict = build_dictionary(words);
        self.group = LintGroup::new_curated(self.dict.clone(), Dialect::American);
    }

    /// Swap in the user's abbreviations (trigger, expansion). Triggers are
    /// canonicalized defensively; pure-Rust pass, no harper rebuild needed.
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

    /// Swap in the ignored tokens (exact, case-insensitive): never flagged,
    /// never guessed, never suggested.
    pub fn set_ignored_words(&mut self, words: &[String]) {
        self.ignored = words
            .iter()
            .map(|w| w.trim().to_lowercase())
            .filter(|w| !w.is_empty())
            .collect();
    }

    /// Lint `text` and apply every first suggestion. Returns (fixed_text,
    /// lints). Lint spans refer to the ORIGINAL text. `apply_guesses` also
    /// applies unambiguous abbreviation guesses (quick path option B);
    /// ambiguous guesses are never applied by the engine.
    pub fn fix(&mut self, text: &str, apply_guesses: bool) -> (String, Vec<LintJson>) {
        if text.trim().is_empty() {
            return (text.to_string(), Vec::new());
        }

        // Lint a smart-quote-normalized copy (see normalize_char) but apply
        // suggestions to the ORIGINAL chars: untouched text keeps its curly
        // quotes byte-for-byte, and clean text comes back exactly as it went
        // in. Spans are identical because normalization is 1:1 per char.
        let normalized: String = text.chars().map(normalize_char).collect();
        // Annotate with the MERGED dictionary, not new_curated: SpellCheck
        // only skips a word when the token carries metadata (Word(Some(..))),
        // and new_curated leaves words it doesn't know as Word(None) — a
        // whitelisted word would still be linted (probe-verified).
        let document = Document::new(&normalized, &PlainEnglish, &self.dict);
        let mut lints = self.group.lint(&document);

        // Lints that must go or be corrected BEFORE remove_overlaps, or they
        // swallow real fixes / annoy on codes: (1) suggestion-less lints —
        // Readability's "sentence is N words long" spans the WHOLE sentence
        // at priority 127, and remove_overlaps then drops every spelling
        // lint (priority 63) inside it; (2) the app's own name, which the
        // curated dictionary doesn't know ("ZWriter" -> "Writer" is not a
        // correction); (3) ignored tokens — per-token "leave it alone";
        // (4) the alphanumeric rule — letter+digit runs (sc2, s12, mp3,
        // x86) are codes, not typos. Extend to the enclosing run because
        // harper may split "sc2" into tokens "sc" + "2".
        let norm_chars: Vec<char> = normalized.chars().collect();
        lints.retain(|l| {
            if l.suggestions.is_empty() {
                return false;
            }
            let span_text: String = norm_chars[l.span.start..l.span.end].iter().collect();
            if span_text.eq_ignore_ascii_case("zwriter") {
                return false;
            }
            if !self.ignored.is_empty() && is_ignored(&norm_chars, l.span.start, l.span.end, &self.ignored) {
                return false;
            }
            if matches!(l.lint_kind, LintKind::Spelling | LintKind::Capitalization) {
                let (s, e) = enclosing_alnum_run(&norm_chars, l.span.start, l.span.end);
                let run = &norm_chars[s..e];
                if run.iter().any(|c| c.is_alphabetic()) && run.iter().any(|c| c.is_numeric()) {
                    return false;
                }
            }
            true
        });
        remove_overlaps(&mut lints);

        // User-taught passes (exact abbreviations, then guesses) own their
        // spans: harper findings inside them are noise.
        let (mut taught, guesses) = abbr_lints(self, &norm_chars);
        taught.extend(guesses);
        lints.retain(|l| !taught.iter().any(|t| l.span.start < t.end && t.start < l.span.end));

        let mut meta: Vec<LintJson> = lints
            .iter()
            .map(|l| {
                let span_text: String = norm_chars[l.span.start..l.span.end].iter().collect();
                let mut suggestions: Vec<Option<String>> = Vec::new();
                for s in &l.suggestions {
                    let rep = suggestion_replacement(s, &span_text);
                    if !suggestions.contains(&rep) {
                        suggestions.push(rep);
                    }
                }
                LintJson {
                    start: l.span.start,
                    end: l.span.end,
                    message: l.message.clone(),
                    kind: format!("{:?}", l.lint_kind),
                    replacement: l.suggestions.first().map(suggestion_text),
                    suggestions,
                    priority: l.priority,
                    guessed: false,
                }
            })
            .collect();
        meta.extend(taught);

        // Apply edits back-to-front so earlier char spans stay valid.
        // A guess is applied only when asked for, and never when ambiguous
        // (several candidates = no safe auto-answer; the review window asks).
        meta.sort_by_key(|l| Reverse(l.start));
        let mut chars: Vec<char> = text.chars().collect();
        for lint in &meta {
            if lint.guessed && (!apply_guesses || lint.suggestions.len() != 1) {
                continue;
            }
            let mid: Vec<char> = match lint.suggestions.first() {
                Some(Some(rep)) => rep.chars().collect(),
                Some(None) => Vec::new(), // None = remove the span
                None => continue,
            };
            let mut next: Vec<char> = chars[..lint.start].to_vec();
            next.extend(mid);
            next.extend(chars[lint.end..].iter().copied());
            chars = next;
        }

        (chars.into_iter().collect(), meta)
    }
}

/// Trigger canonical form: lowercase, all whitespace removed ("sc 2" and
/// "SC2" are the same trigger).
pub fn canonical_trigger(t: &str) -> String {
    t.chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// Exact + guessed abbreviation lints. Spans index the ORIGINAL text
/// (chars here are from the normalized copy, which is 1:1 with it).
fn abbr_lints(engine: &Engine, chars: &[char]) -> (Vec<LintJson>, Vec<LintJson>) {
    let mut exact: Vec<LintJson> = Vec::new();
    let mut guesses: Vec<LintJson> = Vec::new();
    if engine.abbrs.is_empty() {
        return (exact, guesses);
    }
    let alnum = |c: char| c.is_alphanumeric();
    let mut i = 0;
    while i < chars.len() {
        // Word starts only: an alphanumeric preceded by a non-alphanumeric.
        if !alnum(chars[i]) || (i > 0 && alnum(chars[i - 1])) {
            i += 1;
            continue;
        }
        let mut matched = false;
        for (trig, expansion) in &engine.abbrs {
            if let Some(end) = match_trigger(chars, i, trig) {
                let span_text: String = chars[i..end].iter().collect();
                exact.push(LintJson {
                    start: i,
                    end,
                    message: format!("Abbreviation: {span_text} → {expansion}"),
                    kind: "Abbreviation".to_string(),
                    replacement: Some(expansion.clone()),
                    suggestions: vec![Some(expansion.clone())],
                    priority: 0,
                    guessed: false,
                });
                i = end;
                matched = true;
                break;
            }
        }
        if !matched {
            i += 1;
        }
    }
    // Guessed pass over the SAME word starts as the exact pass, tolerating
    // one in-word letter edit against digit-bearing triggers. Trigger-driven
    // (never text-run scanning: squashing spaces fuses whole sentences into
    // one "word"). Ambiguity = several triggers match the same start.
    let mut m = 0;
    while m < chars.len() {
        if !alnum(chars[m]) || (m > 0 && alnum(chars[m - 1])) {
            m += 1;
            continue;
        }
        if exact.iter().any(|l| l.start <= m && m < l.end) {
            m += 1;
            continue;
        }
        let mut expansions: Vec<&String> = Vec::new();
        let mut end = m;
        for (trig, expansion) in &engine.abbrs {
            if let Some(e) = fuzzy_match_trigger(chars, m, trig) {
                if expansions.is_empty() {
                    end = e;
                }
                if !expansions.contains(&expansion) {
                    expansions.push(expansion);
                }
            }
        }
        if expansions.is_empty() {
            m += 1;
            continue;
        }
        // Ignored tokens and real-word stems (3+ letters — harper's curated
        // dictionary knows two-letter entries like "se"/"cs"/"sd") are not
        // guessed.
        if engine
            .ignored
            .contains(&chars[m..end].iter().collect::<String>().to_lowercase())
        {
            m += 1;
            continue;
        }
        let letters: Vec<char> = chars[m..end].iter().filter(|c| !c.is_numeric()).copied().collect();
        if letters.len() >= 3 && engine.dict.contains_word(&letters) {
            m += 1;
            continue;
        }
        let span_text: String = chars[m..end].iter().collect();
        let owned: Vec<String> = expansions.into_iter().cloned().collect();
        let suggestions: Vec<Option<String>> = owned.iter().map(|e| Some(e.clone())).collect();
        let message = if owned.len() == 1 {
            format!("Guessed abbreviation: {span_text} → {}?", owned[0])
        } else {
            format!("{span_text}: close to {} abbreviations — pick one", owned.len())
        };
        guesses.push(LintJson {
            start: m,
            end,
            message,
            kind: "AbbreviationGuess".to_string(),
            replacement: suggestions.first().cloned().flatten(),
            suggestions,
            priority: 0,
            guessed: true,
        });
        m = end;
    }
    (exact, guesses)
}

/// Trigger-shaped match tolerating ONE in-word letter edit (substitution or
/// adjacent transposition). Digits must match exactly at every position, so
/// digit-less triggers never guess (exact matches still expand them).
/// Spaces in the text are skipped — but an edit is never applied across a
/// skipped space, so edits stay inside one word. Returns the exclusive end
/// index (right after the last consumed char).
fn fuzzy_match_trigger(chars: &[char], start: usize, trig: &[char]) -> Option<usize> {
    if !trig.iter().any(|c| c.is_numeric()) {
        return None;
    }
    let mut ci = start;
    let mut last = start;
    let mut edit_used = false;
    let mut ti = 0;
    while ti < trig.len() {
        let before = ci;
        while ci < chars.len() && matches!(chars[ci], ' ' | '\t') {
            ci += 1;
        }
        let skipped = ci > before;
        if ci >= chars.len() {
            return None;
        }
        if chars[ci].eq_ignore_ascii_case(&trig[ti]) {
            ci += 1;
            last = ci;
            ti += 1;
            continue;
        }
        // An edit consumes one letter (substitution) or swaps a letter pair
        // (transposition); never on digits, never across a skipped space.
        if edit_used || skipped || chars[ci].is_numeric() || trig[ti].is_numeric() {
            return None;
        }
        if ti + 1 < trig.len()
            && ci + 1 < chars.len()
            && trig[ti].eq_ignore_ascii_case(&chars[ci + 1])
            && trig[ti + 1].eq_ignore_ascii_case(&chars[ci])
        {
            ci += 2;
            ti += 2;
        } else {
            ci += 1;
            ti += 1;
        }
        last = ci;
        edit_used = true;
    }
    if last < chars.len() && chars[last].is_alphanumeric() {
        return None;
    }
    Some(last)
}

/// Extend a span to the enclosing alphanumeric run (the alphanumeric rule
/// must see the whole code even when harper split it into tokens).
fn enclosing_alnum_run(chars: &[char], start: usize, end: usize) -> (usize, usize) {
    let a = |c: char| c.is_alphanumeric();
    let mut s = start;
    while s > 0 && a(chars[s - 1]) {
        s -= 1;
    }
    let mut e = end;
    while e < chars.len() && a(chars[e]) {
        e += 1;
    }
    (s, e)
}

/// Edge-punctuation-stripped, lowercased span text is an ignored token.
fn is_ignored(chars: &[char], start: usize, end: usize, ignored: &HashSet<String>) -> bool {
    let t: String = chars[start..end].iter().collect::<String>()
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase();
    !t.is_empty() && ignored.contains(&t)
}

/// Whitespace-skipping, case-insensitive trigger match starting at `start`
/// (a word start). Skips only spaces/tabs — never punctuation or newlines —
/// and requires the match to end on a word boundary. Returns the exclusive
/// end index.
fn match_trigger(chars: &[char], start: usize, trig: &[char]) -> Option<usize> {
    let mut ci = start;
    for &t in trig {
        while ci < chars.len() && matches!(chars[ci], ' ' | '\t') {
            ci += 1;
        }
        if ci >= chars.len() || !chars[ci].eq_ignore_ascii_case(&t) {
            return None;
        }
        ci += 1;
    }
    if ci < chars.len() && chars[ci].is_alphanumeric() {
        return None;
    }
    Some(ci)
}

/// Curated dictionary + the user's custom words, merged without copying.
/// Same shape as harper-ls's user-dictionary setup (backend.rs): the SpellCheck
/// linter AND ~20 grammar linters receive this dictionary, so whitelisted
/// words are exempt everywhere.
fn build_dictionary(words: &[String]) -> Arc<MergedDictionary> {
    let mut user = MutableDictionary::new();
    user.extend_words(
        words
            .iter()
            .map(|w| (w.chars().collect::<Vec<char>>(), DictWordMetadata::default())),
    );
    let mut merged = MergedDictionary::new();
    merged.add_dictionary(FstDictionary::curated());
    merged.add_dictionary(Arc::new(user));
    Arc::new(merged)
}

/// Smart quotes break harper's phrase-level grammar rules: "She don’t like"
/// (U+2019) produces ZERO Agreement lints while "She don't like" lints fine
/// (verified on harper-core 2.11.0, the latest release). Each code point here
/// maps 1:1 to its ASCII twin, so spans computed on normalized text index the
/// original identically.
fn normalize_char(c: char) -> char {
    match c {
        '\u{2018}' | '\u{2019}' => '\'',
        '\u{201C}' | '\u{201D}' => '"',
        _ => c,
    }
}

fn suggestion_text(s: &Suggestion) -> String {
    match s {
        Suggestion::ReplaceWith(chars) | Suggestion::InsertAfter(chars) => {
            chars.iter().collect()
        }
        Suggestion::Remove => String::new(),
    }
}

/// The picker UI replaces the whole span, so every suggestion kind is
/// expressed as "new span text" (or None = delete the span).
fn suggestion_replacement(s: &Suggestion, span_text: &str) -> Option<String> {
    match s {
        Suggestion::ReplaceWith(chars) => Some(chars.iter().collect()),
        Suggestion::InsertAfter(chars) => {
            Some(format!("{span_text}{}", chars.iter().collect::<String>()))
        }
        Suggestion::Remove => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_and_clean_text_unchanged() {
        let mut e = Engine::new();
        assert_eq!(e.fix("", false).0, "");
        assert_eq!(e.fix("   ", false).0, "   ");
        let (fixed, lints) = e.fix("The quick brown fox jumps over the lazy dog.", false);
        assert_eq!(fixed, "The quick brown fox jumps over the lazy dog.");
        assert!(lints.is_empty(), "expected no lints, got {lints:?}");
    }

    #[test]
    fn fixes_misspelling() {
        let mut e = Engine::new();
        let (fixed, lints) = e.fix("This is an exampel of a typo.", false);
        assert_eq!(fixed, "This is an example of a typo.", "lints: {lints:?}");
    }

    #[test]
    fn lint_spans_point_at_original_chars() {
        let mut e = Engine::new();
        let text = "This is an exampel of a typo.";
        let (_fixed, lints) = e.fix(text, false);
        assert!(!lints.is_empty());
        let chars: Vec<char> = text.chars().collect();
        let target: &LintJson = lints
            .iter()
            .find(|l| l.start <= 11 && l.end >= 15)
            .expect("expected a lint over 'exampel'");
        assert_eq!(&chars[target.start..target.end].iter().collect::<String>(), "exampel");
    }

    #[test]
    fn multiple_errors_all_fixed() {
        let mut e = Engine::new();
        let (fixed, lints) = e.fix("i beleive this is a mistkae", false);
        assert!(
            fixed.contains("believe") && fixed.contains("mistake"),
            "got: {fixed:?} lints: {lints:?}"
        );
    }

    #[test]
    fn unicode_input_does_not_corrupt_offsets() {
        let mut e = Engine::new();
        // emoji before the typo shifts char indices, not byte indices
        let (fixed, _lints) = e.fix("héllo — this is an exampel 😀", false);
        assert!(fixed.contains("example"), "got: {fixed:?}");
    }

    #[test]
    fn misspelling_exposes_multiple_suggestions() {
        let mut e = Engine::new();
        let text = "This is an exampel of a typo.";
        let (_fixed, lints) = e.fix(text, false);
        let chars: Vec<char> = text.chars().collect();
        let lint = lints
            .iter()
            .find(|l| chars[l.start..l.end].iter().collect::<String>() == "exampel")
            .expect("expected a lint over 'exampel'");
        assert!(
            lint.suggestions.iter().any(|s| s.as_deref() == Some("example")),
            "expected 'example' among suggestions, got {lint:?}"
        );
        assert!(
            lint.suggestions.len() >= 2,
            "expected multiple alternatives, got {lint:?}"
        );
        assert!(
            lint.suggestions.iter().all(|s| s.as_deref() != Some("")),
            "empty-string replacements should not appear, got {lint:?}"
        );
    }

    #[test]
    fn smart_quotes_do_not_block_grammar_lints() {
        let mut e = Engine::new();
        // Straight apostrophes worked even before; curly ones (U+2019, what
        // smart-quote autocorrect produces) produced zero lints and are the
        // user-reported bug.
        let (fixed, lints) = e.fix("She don't like apples.", false);
        assert!(fixed.contains("doesn't"), "got: {fixed:?}");
        assert!(lints.iter().any(|l| l.kind == "Agreement"), "{lints:?}");

        let (fixed, lints) = e.fix("She don\u{2019}t like apples.", false);
        assert!(fixed.contains("doesn't"), "got: {fixed:?}");
        assert!(lints.iter().any(|l| l.kind == "Agreement"), "{lints:?}");

        let (fixed, _) = e.fix("He didn\u{2019}t went home.", false);
        assert!(fixed.contains("go home"), "got: {fixed:?}");
    }

    #[test]
    fn curly_quotes_without_errors_come_back_untouched() {
        let mut e = Engine::new();
        // Normalization is for LINTING only: text with no findings must not
        // be silently converted to straight quotes.
        let text = "It\u{2019}s a \u{201C}test\u{201D} and she said \u{2018}hello\u{2019}.";
        let (fixed, lints) = e.fix(text, false);
        assert_eq!(fixed, text, "clean text must round-trip byte-for-byte");
        assert!(lints.is_empty(), "unexpected lints: {lints:?}");
    }

    #[test]
    fn multiline_paragraph_fixes_every_line() {
        let mut e = Engine::new();
        // User-reported: a full multi-line selection used to be suspected of
        // fixing only the last line. Both newline styles must lint all lines.
        for (name, text) in [
            (
                "LF",
                "She don't like apples.\nHe didn't went home.\ni beleive thiss is a exampel of bad gramar.",
            ),
            (
                "CRLF",
                "She don't like apples.\r\nHe didn't went home.\r\ni beleive thiss is a exampel of bad gramar.",
            ),
        ] {
            let (fixed, lints) = e.fix(text, false);
            assert!(
                fixed.contains("doesn't")
                    && fixed.contains("go home")
                    && fixed.contains("example")
                    && fixed.contains("grammar"),
                "{name}: not every line fixed: {fixed:?} (lints: {lints:?})"
            );
            assert!(lints.len() >= 6, "{name}: too few lints: {lints:?}");
        }
    }

    #[test]
    fn long_sentence_lints_survive_readability_overlap() {
        let mut e = Engine::new();
        // User-reported paragraph: the first sentence is 61 words, so
        // harper's Readability lint spans all of it; before the fix,
        // remove_overlaps let it swallow every spelling lint inside
        // ("chracter" x3, "youu") and only short sentences got fixed.
        let text = "There is 1 more bug which is if you fix the last issue or for example there is only one chracter issue or there is no issue but youu still use the hotkey open the ZWriter window and in the original text change a chracter to a different chracter it will see say there is nothing to change even though there is.";
        let (fixed, lints) = e.fix(text, false);
        assert!(
            fixed.contains("but you still use the hotkey"),
            "'youu' not fixed: {fixed:?}"
        );
        assert!(
            !fixed.contains("chracter") && fixed.matches("character").count() == 3,
            "'chracter' not fixed everywhere: {fixed:?}"
        );
        // The app's own name is not a typo.
        assert!(fixed.contains("ZWriter"), "product name must survive: {fixed:?}");
        assert!(
            lints.iter().all(|l| l.kind != "Readability"),
            "unactionable Readability lints should be dropped: {lints:?}"
        );
    }

    #[test]
    fn custom_dictionary_word_is_not_flagged() {
        // Control: the default engine flags "mistkae" and fixes it.
        let mut plain = Engine::new();
        let (fixed, _) = plain.fix("the mistkae stays here", false);
        assert!(fixed.contains("mistake"), "control failed: {fixed:?}");

        // Whitelisted: same word must survive untouched, with no lints.
        let mut e = Engine::with_custom_words(&["mistkae".to_string()]);
        let text = "the mistkae stays here";
        let (fixed, lints) = e.fix(text, false);
        assert_eq!(fixed, text, "whitelisted word was altered: {lints:?}");
        assert!(lints.is_empty(), "unexpected lints: {lints:?}");
    }

    #[test]
    fn custom_dictionary_word_matches_any_capitalization() {
        // harper's WordId hashes lowercase, so one entry covers every case.
        let mut e = Engine::with_custom_words(&["mistkae".to_string()]);
        let (fixed, lints) = e.fix("The Mistkae stays here.", false);
        assert_eq!(fixed, "The Mistkae stays here.", "lints: {lints:?}");
        assert!(lints.is_empty(), "unexpected lints: {lints:?}");
    }

    #[test]
    fn removing_custom_word_flags_again() {
        let mut e = Engine::with_custom_words(&["mistkae".to_string()]);
        let (fixed, _) = e.fix("the mistkae stays here", false);
        assert_eq!(fixed, "the mistkae stays here");
        e.set_custom_words(&[]);
        let (fixed, lints) = e.fix("the mistkae stays here", false);
        assert!(fixed.contains("mistake"), "removal must re-enable flagging: {fixed:?} {lints:?}");
    }

    #[test]
    fn sentence_latency_is_interactive() {
        let mut e = Engine::new();
        let text = "i beleive thiss is a exampel of bad gramar in a sentense";
        // warm-up (dictionary/linter caches) is separate from steady-state
        let _ = e.fix(text, false);
        let t0 = std::time::Instant::now();
        let (_fixed, lints) = e.fix(text, false);
        let ms = t0.elapsed().as_millis();
        println!("steady-state sentence fix: {ms}ms ({} lints)", lints.len());
        // generous: this is a DEBUG-build number; release is far faster
        assert!(ms < 1000, "sentence fix took {ms}ms — not interactive");
    }

    // --- Abbreviations / ignored words / alphanumeric rule -------------------

    fn engine_with_abbrs(pairs: &[(&str, &str)]) -> Engine {
        let mut e = Engine::new();
        e.set_abbreviations(
            &pairs
                .iter()
                .map(|(t, x)| (t.to_string(), x.to_string()))
                .collect::<Vec<_>>(),
        );
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

    #[test]
    fn exact_abbreviation_expands() {
        let mut e = engine_with_abbrs(&[("sc2", "StarCraft 2")]);
        let text = "is there a tool for sc2 so i can set the hot keys";
        let (fixed, lints) = e.fix(text, false);
        assert!(fixed.contains("StarCraft 2"), "got: {fixed:?}");
        let ab = lints.iter().find(|l| l.kind == "Abbreviation").expect("no Abbreviation lint");
        assert!(!ab.guessed);
        assert_eq!(ab.suggestions, vec![Some("StarCraft 2".to_string())]);
    }

    #[test]
    fn exact_matches_any_case_and_spacing() {
        let mut e = engine_with_abbrs(&[("sc2", "StarCraft 2")]);
        for t in ["use SC2 now", "use sc 2 now", "use s c 2 now"] {
            let (fixed, lints) = e.fix(t, false);
            assert!(fixed.contains("StarCraft 2"), "{t}: {fixed:?}");
            assert!(lints.iter().any(|l| l.kind == "Abbreviation"), "{t}: {lints:?}");
        }
        // punctuation around the match is untouched; span covers the gap
        let (fixed, lints) = e.fix("(SC2).", false);
        assert_eq!(fixed, "(StarCraft 2).");
        let ab = lints.iter().find(|l| l.kind == "Abbreviation").unwrap();
        assert_eq!(span_texts(std::slice::from_ref(ab), "(SC2)."), vec!["SC2"]);
        let (_, lints) = e.fix("use sc 2 ok", false);
        let ab = lints.iter().find(|l| l.kind == "Abbreviation").unwrap();
        assert_eq!(span_texts(std::slice::from_ref(ab), "use sc 2 ok"), vec!["sc 2"]);
    }

    #[test]
    fn word_boundary_required() {
        let mut e = engine_with_abbrs(&[("sc2", "StarCraft 2")]);
        for t in ["massc2 stays", "sc2x stays"] {
            let (fixed, lints) = e.fix(t, false);
            assert_eq!(fixed, t, "lints: {lints:?}");
            assert!(lints.iter().all(|l| l.kind != "Abbreviation"), "{lints:?}");
        }
    }

    #[test]
    fn digits_must_match() {
        let mut e = engine_with_abbrs(&[("sc2", "StarCraft 2")]);
        let (fixed, lints) = e.fix("but sc3 is different", false);
        assert_eq!(fixed, "but sc3 is different", "lints: {lints:?}");
        assert!(lints.iter().all(|l| l.kind != "Abbreviation" && l.kind != "AbbreviationGuess"));
    }

    #[test]
    fn guessed_match_suggested_not_applied() {
        let mut e = engine_with_abbrs(&[("sc2", "StarCraft 2")]);
        let text = "what is se2 anyway";
        let (fixed, lints) = e.fix(text, false);
        assert_eq!(fixed, text, "a guess must not be applied in review mode: {fixed:?}");
        let g = lints.iter().find(|l| l.guessed).expect("no guessed lint");
        assert_eq!(g.suggestions, vec![Some("StarCraft 2".to_string())]);
        let (fixed, _) = e.fix(text, true);
        assert!(fixed.contains("StarCraft 2"), "apply_guesses=true must apply: {fixed:?}");
    }

    #[test]
    fn transposition_counts_as_one_edit() {
        let mut e = engine_with_abbrs(&[("sc2", "StarCraft 2")]);
        for t in ["cs2 unclear", "cs 2 unclear"] {
            let (fixed, lints) = e.fix(t, false);
            assert!(
                lints.iter().any(|l| l.guessed && l.suggestions.contains(&Some("StarCraft 2".to_string()))),
                "{t}: no transposition guess: {lints:?}"
            );
            assert_eq!(fixed, t, "guess applied without apply_guesses");
        }
    }

    #[test]
    fn second_trigger_wins_exactly() {
        let mut e = engine_with_abbrs(&[("sc2", "StarCraft 2"), ("cs2", "Counter-Strike 2")]);
        let (fixed, lints) = e.fix("i play cs 2 daily", false);
        assert!(fixed.contains("Counter-Strike 2"), "got: {fixed:?}");
        let ab = lints.iter().find(|l| l.kind == "Abbreviation").expect("exact expected");
        assert!(!ab.guessed);
        assert!(lints.iter().all(|l| l.kind != "AbbreviationGuess"), "{lints:?}");
    }

    #[test]
    fn ambiguous_guess_lists_all_and_never_applies() {
        let mut e = engine_with_abbrs(&[("sc2", "StarCraft 2"), ("sl2", "System Link 2")]);
        let text = "set sd2 up";
        let (fixed, lints) = e.fix(text, false);
        let g = lints.iter().find(|l| l.guessed).expect("no guessed lint");
        assert_eq!(g.suggestions.len(), 2, "both candidates expected: {lints:?}");
        assert_eq!(fixed, text);
        // even with apply_guesses, ambiguous is never auto-applied
        let (fixed, _) = e.fix(text, true);
        assert_eq!(fixed, text, "ambiguous guess was auto-applied");
    }

    #[test]
    fn alphanumeric_tokens_not_spelling_flagged() {
        let mut e = Engine::new();
        let text = "The codes sc2 s12 mp3 x86 are fine";
        let (fixed, lints) = e.fix(text, false);
        assert_eq!(fixed, text, "lints: {lints:?}");
        assert!(lints.is_empty(), "unexpected lints: {lints:?}");
        // control: real-word typos still flagged
        let (fixed, _) = e.fix("but beleive is a typo", false);
        assert!(fixed.contains("believe"), "control failed: {fixed:?}");
    }

    #[test]
    fn dictionary_stem_not_guessed() {
        let mut e = engine_with_abbrs(&[("sc2", "StarCraft 2")]);
        let (fixed, lints) = e.fix("the set2 value", false);
        assert_eq!(fixed, "the set2 value", "lints: {lints:?}");
        assert!(lints.iter().all(|l| !l.guessed), "{lints:?}");
    }

    #[test]
    fn ignored_word_untouched() {
        let mut e = engine_with_ignored(&["se2"]);
        let text = "the se2 id stays";
        let (fixed, lints) = e.fix(text, false);
        assert_eq!(fixed, text, "lints: {lints:?}");
        assert!(lints.is_empty(), "unexpected lints: {lints:?}");
    }

    #[test]
    fn ignore_loses_to_exact_trigger() {
        let mut e = engine_with_ignored(&["se2"]);
        e.set_abbreviations(&[("se2".to_string(), "Sea Extra 2".to_string())]);
        let (fixed, _) = e.fix("the se2 id", false);
        assert!(fixed.contains("Sea Extra 2"), "exact trigger must beat ignore: {fixed:?}");
    }

    #[test]
    fn mixed_sentence_back_to_front() {
        let mut e = engine_with_abbrs(&[("sc2", "StarCraft 2")]);
        let (fixed, _) = e.fix("i beleive sc2 is an exampel", false);
        assert!(fixed.contains("believe"), "{fixed:?}");
        assert!(fixed.contains("StarCraft 2"), "{fixed:?}");
        assert!(!fixed.contains("sc2"), "{fixed:?}");
    }

    #[test]
    fn clean_codes_round_trip() {
        let mut e = Engine::new();
        let text = "Build x86 s12 mp3 done.";
        let (fixed, lints) = e.fix(text, false);
        assert_eq!(fixed, text, "lints: {lints:?}");
        assert!(lints.is_empty(), "unexpected lints: {lints:?}");
    }
}
