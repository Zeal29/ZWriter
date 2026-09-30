import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import {
  charToUtf16Map,
  prettyChord,
  type FixReady,
  type HistoryEntry,
  type Lint,
  type Settings,
} from "./types";

const DEMO_TEXT = "i beleive this is a exampel of text with some mistkaes in it";

/** Trim edge punctuation so the picker token "mistkae," adds "mistkae". */
function stripWord(w: string): string {
  return w.replace(/^[^\p{L}\p{N}']+|[^\p{L}\p{N}']+$/gu, "");
}

function HotkeyKeys({ chord }: { chord: string }) {
  const parts = prettyChord(chord).split(" + ");
  return (
    <span className="keys">
      {parts.map((k, i) => (
        <span key={i}>
          <kbd>{k}</kbd>
          {i < parts.length - 1 && "+"}
        </span>
      ))}
    </span>
  );
}

/** A span (UTF-16 offsets) the picker can replace, plus what the engine knows. */
interface WordPick {
  spanStart: number;
  spanEnd: number;
  word: string;
  suggestions: (string | null)[];
  message: string | null;
  left: number; // px inside the original pane
  top: number;
}

/**
 * Original text as clickable word tokens. Tokens covered by a lint are
 * highlighted and carry the lint's suggestions; clean words open a plain
 * replace box (so even engine-invisible misspellings can be corrected here).
 */
function OriginalText({
  text,
  lints,
  onPick,
}: {
  text: string;
  lints: Lint[];
  onPick: (p: Omit<WordPick, "left" | "top">, el: HTMLElement) => void;
}) {
  const map = charToUtf16Map(text);
  const lintSpans = lints
    .map((l) => ({
      ...l,
      u16start: map[Math.min(l.start, map.length - 1)],
      u16end: map[Math.min(l.end, map.length - 1)],
    }))
    .filter((l) => l.u16end > l.u16start);

  const nodes: React.ReactNode[] = [];
  const re = /\S+/g;
  let m: RegExpExecArray | null;
  let cursor = 0;
  let key = 0;
  while ((m = re.exec(text)) !== null) {
    const s = m.index;
    const e = s + m[0].length;
    if (s > cursor) nodes.push(<span key={key++}>{text.slice(cursor, s)}</span>);
    // Lints never overlap each other (remove_overlaps in the engine); attach
    // the token to the lint it intersects most.
    let best: (typeof lintSpans)[number] | null = null;
    let bestOv = 0;
    for (const l of lintSpans) {
      const ov = Math.min(e, l.u16end) - Math.max(s, l.u16start);
      if (ov > bestOv) {
        bestOv = ov;
        best = l;
      }
    }
    const word = text.slice(s, e);
    nodes.push(
      <button
        key={key++}
        type="button"
        className={`word${best ? " err" : ""}`}
        title={best ? `${best.kind}: ${best.message}` : "Click to fix or edit this word"}
        aria-label={`word: ${word}`}
        onClick={(ev) =>
          onPick(
            {
              spanStart: best ? best.u16start : s,
              spanEnd: best ? best.u16end : e,
              word,
              suggestions: best?.suggestions ?? [],
              message: best ? `${best.kind}: ${best.message}` : null,
            },
            ev.currentTarget,
          )
        }
      >
        {word}
      </button>,
    );
    cursor = e;
  }
  if (cursor < text.length) nodes.push(<span key={key++}>{text.slice(cursor)}</span>);
  return <>{nodes}</>;
}

function Review() {
  const [fix, setFix] = useState<FixReady | null>(null);
  const [history, setHistory] = useState<HistoryEntry[]>([]);
  const [showHistory, setShowHistory] = useState(false);
  const [demoText, setDemoText] = useState(DEMO_TEXT);
  const [status, setStatus] = useState<string>("");
  const [fixChord, setFixChord] = useState("ctrl+shift+space");
  const [quickChord, setQuickChord] = useState("ctrl+space");
  const [picked, setPicked] = useState<WordPick | null>(null);
  const [editVal, setEditVal] = useState("");
  const pendingRef = useRef<FixReady | null>(null);
  pendingRef.current = fix;
  const pickedRef = useRef<WordPick | null>(null);
  pickedRef.current = picked;
  // fix_text (no pending in Rust) for the self-test demo, update_pending for
  // real captures — editing demo text must not clobber a live pending fix.
  const fromCaptureRef = useRef(false);
  // Demo-path edits have no Rust pending to carry userEdited — track them here.
  const editedRef = useRef(false);
  const paneRef = useRef<HTMLDivElement>(null);

  const refreshHistory = useCallback(() => {
    invoke<HistoryEntry[]>("get_history")
      .then(setHistory)
      .catch(() => {});
  }, []);

  useEffect(() => {
    const unlistenP = getCurrentWebviewWindow().listen<FixReady>("fix-ready", (e) => {
      fromCaptureRef.current = true;
      editedRef.current = false;
      setPicked(null);
      setFix(e.payload);
      refreshHistory();
    });
    const unlistenHk = getCurrentWebviewWindow().listen<{
      fixHotkey: string;
      quickHotkey: string;
    }>("hotkeys-changed", (e) => {
      setFixChord(e.payload.fixHotkey);
      setQuickChord(e.payload.quickHotkey);
    });
    // A word added/removed here or in Settings changes what the engine
    // finds — re-check the text currently shown so its lints follow.
    const rerun = () => {
      const f = pendingRef.current;
      if (!f) return;
      const cmd = fromCaptureRef.current
        ? invoke<FixReady>("update_pending", { text: f.original })
        : invoke<FixReady>("fix_text", { text: f.original });
      cmd.then(setFix).catch(() => {});
    };
    const unlistenDict = getCurrentWebviewWindow().listen("custom-words-changed", rerun);
    const unlistenAbbr = getCurrentWebviewWindow().listen("abbreviations-changed", rerun);
    const unlistenIgn = getCurrentWebviewWindow().listen("ignored-words-changed", rerun);
    invoke<FixReady | null>("get_pending")
      .then((p) => {
        if (p) {
          fromCaptureRef.current = true;
          setFix(p);
        }
      })
      .catch(() => {});
    invoke<Settings>("get_settings")
      .then((s) => {
        setFixChord(s.fixHotkey);
        setQuickChord(s.quickHotkey);
      })
      .catch(() => {});
    refreshHistory();
    return () => {
      unlistenP.then((f) => f()).catch(() => {});
      unlistenHk.then((f) => f()).catch(() => {});
      unlistenDict.then((f) => f()).catch(() => {});
      unlistenAbbr.then((f) => f()).catch(() => {});
      unlistenIgn.then((f) => f()).catch(() => {});
    };
  }, [refreshHistory]);

  const apply = useCallback(() => {
    invoke("apply_paste").catch(() => {});
    setStatus("Pasted into your app.");
    setPicked(null);
    setFix(null);
  }, []);

  const dismiss = useCallback(() => {
    invoke("dismiss_fix").catch(() => {});
    setPicked(null);
    setFix(null);
  }, []);

  const copy = useCallback(() => {
    invoke("copy_fixed")
      .then(() => setStatus("Copied."))
      .catch(() => setStatus("Nothing to copy."));
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.target as HTMLElement | null)?.tagName === "INPUT" || (e.target as HTMLElement | null)?.tagName === "TEXTAREA") {
        return; // typing in the word editor: let the input handle the keys
      }
      if (e.ctrlKey && e.key === ",") {
        e.preventDefault();
        invoke("open_settings").catch(() => {});
        return;
      }
      if (e.key === "Escape" && pickedRef.current) {
        e.preventDefault();
        setPicked(null);
        return;
      }
      if (!pendingRef.current) return;
      if (e.key === "Enter") {
        e.preventDefault();
        apply();
      } else if (e.key === "Escape") {
        e.preventDefault();
        dismiss();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [apply, dismiss]);

  const pickWord = useCallback((p: Omit<WordPick, "left" | "top">, el: HTMLElement) => {
    const pane = paneRef.current;
    if (!pane) return;
    const pr = pane.getBoundingClientRect();
    const tr = el.getBoundingClientRect();
    const width = Math.min(300, pr.width - 16);
    const left = Math.max(0, Math.min(tr.left - pr.left, pr.width - width - 8));
    const top = tr.bottom - pr.top + 6;
    setPicked({ ...p, left, top });
    setEditVal(p.word);
  }, []);

  /** Replace the picked span (null = remove it) and re-run the engine live. */
  const commitWord = useCallback(
    (replacement: string | null) => {
      const f = pendingRef.current;
      const p = pickedRef.current;
      if (!f || !p) return;
      setPicked(null);
      const next = f.original.slice(0, p.spanStart) + (replacement ?? "") + f.original.slice(p.spanEnd);
      if (next === f.original) return;
      editedRef.current = true;
      const cmd =
        fromCaptureRef.current
          ? invoke<FixReady>("update_pending", { text: next })
          : invoke<FixReady>("fix_text", { text: next });
      cmd
        .then((r) => {
          setFix(r);
          setStatus("Text updated — engine re-checked it.");
        })
        .catch((e) => setStatus(`Update failed: ${e}`));
    },
    [],
  );

  const commitEdit = useCallback(() => {
    const p = pickedRef.current;
    if (!p) return;
    const v = editVal.trim();
    if (!v) return;
    if (v === p.word.trim()) {
      setPicked(null);
      return;
    }
    commitWord(v);
  }, [editVal, commitWord]);

  /** Whitelist this word so the engine stops flagging it. The lint refresh
   *  comes from the custom-words-changed event the command emits. */
  const addToDictionary = useCallback(() => {
    const p = pickedRef.current;
    if (!p) return;
    const w = stripWord(p.word);
    if (!w) return;
    setPicked(null);
    invoke("add_custom_word", { word: w })
      .then(() => setStatus(`"${w}" added to your dictionary.`))
      .catch((e) => setStatus(`Could not add word: ${e}`));
  }, []);

  /** Teach an abbreviation: the picked word is the trigger, the popover's
   *  edit box holds the full term. The engine re-run comes from the
   *  abbreviations-changed event the command emits. */
  const addAsAbbreviation = useCallback(() => {
    const p = pickedRef.current;
    if (!p) return;
    const trigger = stripWord(p.word);
    const expansion = editVal.trim();
    if (!trigger || !expansion) return;
    setPicked(null);
    invoke("add_abbreviation", { trigger, expansion })
      .then(() => setStatus(`"${trigger}" now expands to "${expansion}".`))
      .catch((e) => setStatus(`Could not add abbreviation: ${e}`));
  }, [editVal]);

  /** Ignore this exact token: never flagged, never guessed, never suggested. */
  const ignoreWord = useCallback(() => {
    const p = pickedRef.current;
    if (!p) return;
    const w = stripWord(p.word);
    if (!w) return;
    setPicked(null);
    invoke("add_ignored_word", { word: w })
      .then(() => setStatus(`"${w}" will be left as-is.`))
      .catch((e) => setStatus(`Could not ignore word: ${e}`));
  }, []);

  const simulate = () => {
    setStatus("Capturing selection… (select text first)");
    invoke("simulate_hotkey").catch(() => {});
  };

  const testEngine = () => {
    invoke<FixReady>("fix_text", { text: demoText })
      .then((r) => {
        fromCaptureRef.current = false;
        editedRef.current = false;
        setPicked(null);
        setFix(r);
        setStatus(`Engine: ${r.lints.length} lints in ${r.fixMs}ms`);
      })
      .catch((e) => setStatus(`Engine error: ${e}`));
  };

  // "Nothing to apply" is only true when the engine found nothing AND the
  // user has not edited the text themselves (backend flag + demo-path edits).
  const userEdited = (fix?.userEdited ?? false) || editedRef.current;

  return (
    <main className="container">
      <header className="header">
        <h1>ZWriter</h1>
        <span className="hint">
          Select text anywhere → <HotkeyKeys chord={fixChord} /> review, <HotkeyKeys chord={quickChord} /> quick fix
        </span>
        <button className="ghost" onClick={() => invoke("open_settings")} title="Settings">
          ⚙
        </button>
      </header>

      {!fix && (
        <p className="empty">
          Nothing to review. Select text in any app and press <HotkeyKeys chord={fixChord} />.
        </p>
      )}

      {fix && (
        <section className="fix-view">
          <div className="pane" ref={paneRef}>
            <h2>
              Original{" "}
              <span className="badge">{fix.lints.length} issue{fix.lints.length === 1 ? "" : "s"}</span>
            </h2>
            <p className="sub pane-hint">Click a word to see fixes or edit it — the Fixed pane follows along.</p>
            <p className="text original">
              <OriginalText text={fix.original} lints={fix.lints} onPick={pickWord} />
            </p>
            {picked && (
              <>
                <div className="pop-backdrop" onMouseDown={() => setPicked(null)} />
                <div
                  className="wordpop"
                  role="dialog"
                  aria-label="Word suggestions"
                  style={{ left: picked.left, top: picked.top, width: Math.min(300, (paneRef.current?.clientWidth ?? 300) - 16) }}
                >
                  <p className="wordpop-msg">
                    {picked.message ?? "No engine issue in this word — you can still replace it."}
                  </p>
                  {picked.suggestions.length > 0 && (
                    <div className="chips">
                      {picked.suggestions.map((s, i) => (
                        <button
                          key={i}
                          type="button"
                          aria-label={s === null ? "Remove word" : `suggestion: ${s}`}
                          onClick={() => commitWord(s)}
                        >
                          {s === null ? "(remove)" : s}
                        </button>
                      ))}
                    </div>
                  )}
                  {picked.message && stripWord(picked.word) && (
                    <button
                      type="button"
                      className="dict-add"
                      aria-label="Add to dictionary"
                      title="Never flag this word again"
                      onClick={addToDictionary}
                    >
                      + Add to dictionary
                    </button>
                  )}
                  {stripWord(picked.word) && (
                    <button
                      type="button"
                      className="dict-add"
                      aria-label="Add as abbreviation"
                      title="Type the full term in the box below first"
                      disabled={!editVal.trim() || editVal.trim() === picked.word.trim()}
                      onClick={addAsAbbreviation}
                    >
                      + Add as abbreviation
                    </button>
                  )}
                  {stripWord(picked.word) && (
                    <button
                      type="button"
                      className="dict-add"
                      aria-label="Ignore word"
                      title="Never flag or suggest this token"
                      onClick={ignoreWord}
                    >
                      + Ignore word
                    </button>
                  )}
                  <div className="wordpop-row">
                    <input
                      aria-label="Edit word"
                      value={editVal}
                      onChange={(e) => setEditVal(e.target.value)}
                      onKeyDown={(e) => {
                        if (e.key === "Enter") {
                          e.preventDefault();
                          e.stopPropagation();
                          commitEdit();
                        }
                      }}
                    />
                    <button type="button" aria-label="Replace word" onClick={commitEdit}>
                      Replace
                    </button>
                  </div>
                </div>
              </>
            )}
          </div>
          <div className="pane">
            <h2>
              Fixed <span className="badge green">{fix.fixMs}ms</span>
            </h2>
            <p className={`text fixed ${fix.noChange && !userEdited ? "nochange" : ""}`}>
              {fix.noChange && !userEdited
                ? "No errors found — text is already clean."
                : fix.fixed}
            </p>
          </div>
          <div className="actions">
            <button className="primary" onClick={apply} disabled={fix.noChange && !userEdited}>
              Apply <span className="kbd">Enter</span>
            </button>
            <button onClick={copy}>Copy</button>
            <button className="ghost" onClick={dismiss}>
              Skip <span className="kbd">Esc</span>
            </button>
          </div>
        </section>
      )}

      {status && <p className="status">{status}</p>}

      <section className="devbox">
        <details>
          <summary>Self-test tools</summary>
          <div className="devgrid">
            <label>
              Test text (engine only)
              <textarea value={demoText} onChange={(e) => setDemoText(e.target.value)} rows={2} />
            </label>
            <div className="devbtns">
              <button onClick={testEngine}>Run engine on text</button>
              <button onClick={simulate}>Simulate hotkey (select text above first)</button>
            </div>
          </div>
        </details>
      </section>

      <section className="history">
        <button className="linklike" onClick={() => { setShowHistory((v) => !v); refreshHistory(); }}>
          {showHistory ? "▾" : "▸"} History ({history.length})
        </button>
        {showHistory && (
          <ul>
            {history.length === 0 && <li className="empty">No fixes yet.</li>}
            {history.map((h) => (
              <li key={h.ts}>
                <span className="time">{new Date(h.ts).toLocaleTimeString()}</span>
                <span className="horig">{h.original.length > 60 ? h.original.slice(0, 60) + "…" : h.original}</span>
                <span className="arrow">→</span>
                <span className="hfixed">{h.fixed.length > 60 ? h.fixed.slice(0, 60) + "…" : h.fixed}</span>
                <span className={`badge ${h.applied ? "green" : ""}`}>{h.fixCount}</span>
              </li>
            ))}
          </ul>
        )}
      </section>
    </main>
  );
}

export default Review;
