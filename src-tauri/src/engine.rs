//! Harper grammar engine wrapper.
//!
//! Spans are CHAR indices (Unicode scalar values), not byte offsets — the
//! frontend must convert before slicing JS strings (see Review.tsx).

use harper_core::linting::{LintGroup, Linter, Suggestion};
use harper_core::parsers::PlainEnglish;
use harper_core::spell::{FstDictionary, MergedDictionary, MutableDictionary};
use harper_core::{remove_overlaps, Dialect, DictWordMetadata, Document};
use serde::Serialize;
use std::cmp::Reverse;
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
}

pub struct Engine {
    group: LintGroup,
    dict: Arc<MergedDictionary>,
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
        }
    }

    /// Swap in a new word list. Rebuilds dictionary + LintGroup — the
    /// canonical harper-ls pattern; the curated FST is process-cached, so
    /// this is cheap (no dictionary copying, ~100 small linter structs).
    pub fn set_custom_words(&mut self, words: &[String]) {
        self.dict = build_dictionary(words);
        self.group = LintGroup::new_curated(self.dict.clone(), Dialect::American);
    }

    /// Lint `text` and apply every first suggestion.
    /// Returns (fixed_text, lints). Lint spans refer to the ORIGINAL text.
    pub fn fix(&mut self, text: &str) -> (String, Vec<LintJson>) {
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

        // Two lint classes must go BEFORE remove_overlaps, or they swallow
        // real fixes: (1) suggestion-less lints — Readability's "sentence is
        // N words long" spans the WHOLE sentence at priority 127, and
        // remove_overlaps then drops every spelling lint (priority 63)
        // inside it; (2) the app's own name, which the curated dictionary
        // doesn't know ("ZWriter" -> "Writer" is not a correction).
        let norm_chars: Vec<char> = normalized.chars().collect();
        lints.retain(|l| {
            if l.suggestions.is_empty() {
                return false;
            }
            let span_text: String = norm_chars[l.span.start..l.span.end].iter().collect();
            !span_text.eq_ignore_ascii_case("zwriter")
        });
        remove_overlaps(&mut lints);

        let meta: Vec<LintJson> = lints
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
                }
            })
            .collect();

        // Apply edits back-to-front so earlier char spans stay valid.
        lints.sort_by_key(|l| Reverse(l.span.start));
        let mut chars: Vec<char> = text.chars().collect();
        for lint in lints {
            if let Some(s) = lint.suggestions.first() {
                s.apply(lint.span, &mut chars);
            }
        }

        (chars.into_iter().collect(), meta)
    }
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
        assert_eq!(e.fix("").0, "");
        assert_eq!(e.fix("   ").0, "   ");
        let (fixed, lints) = e.fix("The quick brown fox jumps over the lazy dog.");
        assert_eq!(fixed, "The quick brown fox jumps over the lazy dog.");
        assert!(lints.is_empty(), "expected no lints, got {lints:?}");
    }

    #[test]
    fn fixes_misspelling() {
        let mut e = Engine::new();
        let (fixed, lints) = e.fix("This is an exampel of a typo.");
        assert_eq!(fixed, "This is an example of a typo.", "lints: {lints:?}");
    }

    #[test]
    fn lint_spans_point_at_original_chars() {
        let mut e = Engine::new();
        let text = "This is an exampel of a typo.";
        let (_fixed, lints) = e.fix(text);
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
        let (fixed, lints) = e.fix("i beleive this is a mistkae");
        assert!(
            fixed.contains("believe") && fixed.contains("mistake"),
            "got: {fixed:?} lints: {lints:?}"
        );
    }

    #[test]
    fn unicode_input_does_not_corrupt_offsets() {
        let mut e = Engine::new();
        // emoji before the typo shifts char indices, not byte indices
        let (fixed, _lints) = e.fix("héllo — this is an exampel 😀");
        assert!(fixed.contains("example"), "got: {fixed:?}");
    }

    #[test]
    fn misspelling_exposes_multiple_suggestions() {
        let mut e = Engine::new();
        let text = "This is an exampel of a typo.";
        let (_fixed, lints) = e.fix(text);
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
        let (fixed, lints) = e.fix("She don't like apples.");
        assert!(fixed.contains("doesn't"), "got: {fixed:?}");
        assert!(lints.iter().any(|l| l.kind == "Agreement"), "{lints:?}");

        let (fixed, lints) = e.fix("She don\u{2019}t like apples.");
        assert!(fixed.contains("doesn't"), "got: {fixed:?}");
        assert!(lints.iter().any(|l| l.kind == "Agreement"), "{lints:?}");

        let (fixed, _) = e.fix("He didn\u{2019}t went home.");
        assert!(fixed.contains("go home"), "got: {fixed:?}");
    }

    #[test]
    fn curly_quotes_without_errors_come_back_untouched() {
        let mut e = Engine::new();
        // Normalization is for LINTING only: text with no findings must not
        // be silently converted to straight quotes.
        let text = "It\u{2019}s a \u{201C}test\u{201D} and she said \u{2018}hello\u{2019}.";
        let (fixed, lints) = e.fix(text);
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
            let (fixed, lints) = e.fix(text);
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
        let (fixed, lints) = e.fix(text);
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
        let (fixed, _) = plain.fix("the mistkae stays here");
        assert!(fixed.contains("mistake"), "control failed: {fixed:?}");

        // Whitelisted: same word must survive untouched, with no lints.
        let mut e = Engine::with_custom_words(&["mistkae".to_string()]);
        let text = "the mistkae stays here";
        let (fixed, lints) = e.fix(text);
        assert_eq!(fixed, text, "whitelisted word was altered: {lints:?}");
        assert!(lints.is_empty(), "unexpected lints: {lints:?}");
    }

    #[test]
    fn custom_dictionary_word_matches_any_capitalization() {
        // harper's WordId hashes lowercase, so one entry covers every case.
        let mut e = Engine::with_custom_words(&["mistkae".to_string()]);
        let (fixed, lints) = e.fix("The Mistkae stays here.");
        assert_eq!(fixed, "The Mistkae stays here.", "lints: {lints:?}");
        assert!(lints.is_empty(), "unexpected lints: {lints:?}");
    }

    #[test]
    fn removing_custom_word_flags_again() {
        let mut e = Engine::with_custom_words(&["mistkae".to_string()]);
        let (fixed, _) = e.fix("the mistkae stays here");
        assert_eq!(fixed, "the mistkae stays here");
        e.set_custom_words(&[]);
        let (fixed, lints) = e.fix("the mistkae stays here");
        assert!(fixed.contains("mistake"), "removal must re-enable flagging: {fixed:?} {lints:?}");
    }

    #[test]
    fn sentence_latency_is_interactive() {
        let mut e = Engine::new();
        let text = "i beleive thiss is a exampel of bad gramar in a sentense";
        // warm-up (dictionary/linter caches) is separate from steady-state
        let _ = e.fix(text);
        let t0 = std::time::Instant::now();
        let (_fixed, lints) = e.fix(text);
        let ms = t0.elapsed().as_millis();
        println!("steady-state sentence fix: {ms}ms ({} lints)", lints.len());
        // generous: this is a DEBUG-build number; release is far faster
        assert!(ms < 1000, "sentence fix took {ms}ms — not interactive");
    }
}
