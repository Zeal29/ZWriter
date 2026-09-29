export interface Lint {
  start: number; // char index into the ORIGINAL text (Unicode chars, not UTF-16)
  end: number;
  message: string;
  kind: string;
  replacement: string | null;
  /** Every engine suggestion for this lint; null = remove the word. */
  suggestions: (string | null)[];
  priority: number;
}

export interface FixReady {
  original: string;
  fixed: string;
  lints: Lint[];
  fixMs: number;
  noChange: boolean;
  /** The user edited the text in the review window: noChange then only
   *  means "engine found nothing left" — Apply must stay enabled. */
  userEdited: boolean;
}

export interface HistoryEntry {
  ts: number;
  original: string;
  fixed: string;
  fixCount: number;
  applied: boolean;
}

export interface Settings {
  fixHotkey: string; // fix with review window, e.g. "ctrl+alt+g"
  quickHotkey: string; // fix + paste immediately, e.g. "ctrl+alt+f"
  autostart: boolean;
  customWords: string[]; // never flagged as spelling errors (case-insensitive)
}

/** "ctrl+alt+g" -> "Ctrl + Alt + G" for display. */
export function prettyChord(chord: string): string {
  return chord
    .split("+")
    .map((p) => p.charAt(0).toUpperCase() + p.slice(1))
    .join(" + ");
}

/**
 * Build a chord string the Rust shortcut parser accepts from a key event.
 * `key` is null while only modifiers are held (live preview).
 * Returns null for keys we do not support as hotkeys.
 */
export function chordFromEvent(e: KeyboardEvent, key: string | null): string | null {
  const parts: string[] = [];
  if (e.ctrlKey) parts.push("ctrl");
  if (e.altKey) parts.push("alt");
  if (e.shiftKey) parts.push("shift");
  if (e.metaKey) parts.push("super");
  if (key !== null) {
    const k = normalizeKey(key);
    if (!k) return null;
    parts.push(k);
  }
  return parts.length ? parts.join("+") : null;
}

function normalizeKey(key: string): string | null {
  if (/^[a-z]$/i.test(key)) return key.toLowerCase();
  if (/^[0-9]$/.test(key)) return key;
  if (/^F([1-9]|1[0-2])$/i.test(key)) return key.toLowerCase();
  if (key === " ") return "space";
  return null; // punctuation/arrows/media keys: unsupported
}

/**
 * Harper lints use char indices; JS strings are UTF-16. Build a map from
 * char index to UTF-16 offset so highlights land on the right characters
 * even with emoji/accented text.
 */
export function charToUtf16Map(text: string): number[] {
  const map: number[] = [];
  let u = 0;
  for (const ch of text) {
    map.push(u);
    u += ch.length;
  }
  map.push(u);
  return map;
}
