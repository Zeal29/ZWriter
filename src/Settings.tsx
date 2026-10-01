import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { chordFromEvent, prettyChord, type Abbreviation, type Settings } from "./types";

/**
 * Click-to-record hotkey button. Modifier-only keydowns update the preview;
 * a non-modifier key finalizes the chord. A modifier is REQUIRED (a bare
 * letter as a global hotkey would swallow typing everywhere).
 */
function HotkeyField({
  value,
  onCommit,
}: {
  value: string;
  onCommit: (chord: string) => void;
}) {
  const [recording, setRecording] = useState(false);
  const [preview, setPreview] = useState<string | null>(null);
  const [hint, setHint] = useState("");

  useEffect(() => {
    if (!recording) return;
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.key === "Escape") {
        setRecording(false);
        setPreview(null);
        return;
      }
      // A modifier keydown sets BOTH e.key and its flag - keep waiting
      // for the real key (see AGENTS.md global rules).
      if (["Control", "Shift", "Alt", "Meta"].includes(e.key)) {
        setPreview(chordFromEvent(e, null));
        return;
      }
      if (!(e.ctrlKey || e.altKey || e.metaKey)) {
        setHint("add Ctrl or Alt (a bare key would block your typing)");
        setRecording(false);
        setPreview(null);
        return;
      }
      const chord = chordFromEvent(e, e.key);
      setRecording(false);
      setPreview(null);
      if (!chord) {
        setHint("that key is not supported - use a letter, digit, F-key or Space");
        return;
      }
      setHint("");
      onCommit(chord);
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [recording, onCommit]);

  return (
    <div className="hotkey-field">
      <button
        className={`hotkey ${recording ? "recording" : ""}`}
        onClick={() => {
          setHint("");
          setRecording(true);
        }}
        title="Click, then press the new key combination"
      >
        {recording ? preview ?? "press keys…" : <kbd>{prettyChord(value)}</kbd>}
      </button>
      {hint && <p className="sub hotkey-hint">{hint}</p>}
    </div>
  );
}

function Settings() {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [error, setError] = useState("");
  const [newWord, setNewWord] = useState("");
  const [newTrigger, setNewTrigger] = useState("");
  const [newExpansion, setNewExpansion] = useState("");
  const [newIgnored, setNewIgnored] = useState("");

  useEffect(() => {
    invoke<Settings>("get_settings").then(setSettings).catch((e) => setError(String(e)));
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        invoke("hide_settings").catch(() => {});
      }
    };
    window.addEventListener("keydown", onKey);
    // The window mounts at app start and stays alive hidden — without a
    // refetch on show, words added from the review window's picker would
    // never appear here.
    const refetch = () => invoke<Settings>("get_settings").then(setSettings).catch(() => {});
    window.addEventListener("focus", refetch);
    // Live-follow edits made in the review window while open.
    const unlisten = getCurrentWebviewWindow().listen<{ words: string[] }>(
      "custom-words-changed",
      (e) => {
        setSettings((s) => (s ? { ...s, customWords: e.payload.words } : s));
      },
    );
    const unlistenAbbr = getCurrentWebviewWindow().listen<{ abbreviations: Abbreviation[] }>(
      "abbreviations-changed",
      (e) => {
        setSettings((s) => (s ? { ...s, abbreviations: e.payload.abbreviations } : s));
      },
    );
    const unlistenIgn = getCurrentWebviewWindow().listen<{ words: string[] }>(
      "ignored-words-changed",
      (e) => {
        setSettings((s) => (s ? { ...s, ignoredWords: e.payload.words } : s));
      },
    );
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("focus", refetch);
      unlisten.then((f) => f()).catch(() => {});
      unlistenAbbr.then((f) => f()).catch(() => {});
      unlistenIgn.then((f) => f()).catch(() => {});
    };
  }, []);

  const toggleAutostart = (enabled: boolean) => {
    setSettings((s) => (s ? { ...s, autostart: enabled } : s));
    invoke<boolean>("set_autostart", { enabled })
      .then((ok) => setSettings((s) => (s ? { ...s, autostart: ok } : s)))
      .catch((e) => {
        setError(String(e));
        invoke<Settings>("get_settings").then(setSettings).catch(() => {});
      });
  };

  const saveHotkeys = useCallback(
    async (next: { fixHotkey?: string; quickHotkey?: string }) => {
      if (!settings) return;
      const fix = next.fixHotkey ?? settings.fixHotkey;
      const quick = next.quickHotkey ?? settings.quickHotkey;
      if (fix === quick) {
        setError("the two hotkeys must be different");
        return;
      }
      const prev = settings;
      setSettings({ ...settings, fixHotkey: fix, quickHotkey: quick });
      try {
        await invoke("set_hotkeys", { fixHotkey: fix, quickHotkey: quick });
        setError("");
      } catch (e) {
        setSettings(prev); // rolled back server-side; mirror it in the UI
        setError(String(e));
      }
    },
    [settings],
  );

  const clearHistory = () => invoke("clear_history").catch((e) => setError(String(e)));

  const addWord = async () => {
    if (!newWord.trim()) return;
    try {
      const words = await invoke<string[]>("add_custom_word", { word: newWord });
      setSettings((s) => (s ? { ...s, customWords: words } : s));
      setNewWord("");
      setError("");
    } catch (e) {
      setError(String(e));
    }
  };

  const removeWord = async (w: string) => {
    try {
      const words = await invoke<string[]>("remove_custom_word", { word: w });
      setSettings((s) => (s ? { ...s, customWords: words } : s));
    } catch (e) {
      setError(String(e));
    }
  };

  const addAbbreviation = async () => {
    if (!newTrigger.trim() || !newExpansion.trim()) return;
    try {
      const abbreviations = await invoke<Abbreviation[]>("add_abbreviation", {
        trigger: newTrigger,
        expansion: newExpansion,
      });
      setSettings((s) => (s ? { ...s, abbreviations } : s));
      setNewTrigger("");
      setNewExpansion("");
      setError("");
    } catch (e) {
      setError(String(e));
    }
  };

  const removeAbbreviation = async (t: string) => {
    try {
      const abbreviations = await invoke<Abbreviation[]>("remove_abbreviation", { trigger: t });
      setSettings((s) => (s ? { ...s, abbreviations } : s));
    } catch (e) {
      setError(String(e));
    }
  };

  const addIgnoredWord = async () => {
    if (!newIgnored.trim()) return;
    try {
      const words = await invoke<string[]>("add_ignored_word", { word: newIgnored });
      setSettings((s) => (s ? { ...s, ignoredWords: words } : s));
      setNewIgnored("");
      setError("");
    } catch (e) {
      setError(String(e));
    }
  };

  const removeIgnoredWord = async (w: string) => {
    try {
      const words = await invoke<string[]>("remove_ignored_word", { word: w });
      setSettings((s) => (s ? { ...s, ignoredWords: words } : s));
    } catch (e) {
      setError(String(e));
    }
  };

  const toggleSkipGuessWindow = (enabled: boolean) => {
    setSettings((s) => (s ? { ...s, skipGuessWindow: enabled } : s));
    invoke<boolean>("set_skip_guess_window", { enabled })
      .then((ok) => setSettings((s) => (s ? { ...s, skipGuessWindow: ok } : s)))
      .catch((e) => {
        setError(String(e));
        invoke<Settings>("get_settings").then(setSettings).catch(() => {});
      });
  };

  const toggleAutoApplyGuesses = (enabled: boolean) => {
    setSettings((s) => (s ? { ...s, autoApplyGuesses: enabled } : s));
    invoke<boolean>("set_auto_apply_guesses", { enabled })
      .then((ok) => setSettings((s) => (s ? { ...s, autoApplyGuesses: ok } : s)))
      .catch((e) => {
        setError(String(e));
        invoke<Settings>("get_settings").then(setSettings).catch(() => {});
      });
  };

  if (!settings) {
    return (
      <main className="container">
        <p className="empty">{error || "Loading settings…"}</p>
      </main>
    );
  }

  return (
    <main className="container">
      <header className="header">
        <h1>Settings</h1>
        <button
          className="ghost"
          onClick={() => invoke("hide_settings")}
          title="Close"
          aria-label="Close settings"
        >
          ✕
        </button>
      </header>

      <section className="setting-row">
        <div>
          <strong>Fix hotkey</strong>
          <p className="sub">Select text, press it, review the fix, then Apply</p>
        </div>
        <HotkeyField value={settings.fixHotkey} onCommit={(c) => saveHotkeys({ fixHotkey: c })} />
      </section>

      <section className="setting-row">
        <div>
          <strong>Quick-fix hotkey</strong>
          <p className="sub">Select text, press it, fixed text is pasted immediately</p>
        </div>
        <HotkeyField
          value={settings.quickHotkey}
          onCommit={(c) => saveHotkeys({ quickHotkey: c })}
        />
      </section>

      <section className="setting-row">
        <div>
          <strong>Custom dictionary</strong>
          <p className="sub">Your own words are never flagged as spelling errors</p>
        </div>
      </section>
      <section className="dictionary">
        <div className="add-word">
          <input
            aria-label="New dictionary word"
            placeholder="Type a word, press Enter"
            value={newWord}
            onChange={(e) => setNewWord(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                addWord();
              }
            }}
          />
          <button aria-label="Add word" onClick={addWord}>
            Add
          </button>
        </div>
        {settings.customWords.length === 0 ? (
          <p className="sub">No custom words yet. Misspelled a name? Add it here or from the review window.</p>
        ) : (
          <div className="word-list">
            {[...settings.customWords]
              .sort((a, b) => a.toLowerCase().localeCompare(b.toLowerCase()))
              .map((w) => (
                <span className="word-chip" key={w.toLowerCase()}>
                  {w}
                  <button
                    className="chip-x"
                    aria-label={`Remove word ${w}`}
                    title={`Remove ${w}`}
                    onClick={() => removeWord(w)}
                  >
                    ✕
                  </button>
                </span>
              ))}
          </div>
        )}
      </section>

      <section className="setting-row">
        <div>
          <strong>Abbreviations</strong>
          <p className="sub">A shortcut expands to the full term, even one letter off</p>
        </div>
      </section>
      <section className="dictionary">
        <div className="add-abbrev">
          <input
            aria-label="New abbreviation trigger"
            placeholder="sc2"
            value={newTrigger}
            onChange={(e) => setNewTrigger(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                addAbbreviation();
              }
            }}
          />
          <input
            aria-label="New abbreviation expansion"
            placeholder="StarCraft 2"
            value={newExpansion}
            onChange={(e) => setNewExpansion(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                addAbbreviation();
              }
            }}
          />
          <button aria-label="Add abbreviation" onClick={addAbbreviation}>
            Add
          </button>
        </div>
        {settings.abbreviations.length === 0 ? (
          <p className="sub">No abbreviations yet. Type a shortcut and its full term — or add one from the review window.</p>
        ) : (
          <div className="word-list">
            {settings.abbreviations.map((a) => (
              <span className="word-chip" key={a.trigger.toLowerCase()}>
                {a.trigger} → {a.expansion}
                <button
                  className="chip-x"
                  aria-label={`Remove abbreviation ${a.trigger}`}
                  title={`Remove ${a.trigger}`}
                  onClick={() => removeAbbreviation(a.trigger)}
                >
                  ✕
                </button>
              </span>
            ))}
          </div>
        )}
      </section>

      <section className="setting-row">
        <div>
          <strong>Ignored words</strong>
          <p className="sub">
            Codes and IDs left as typed — for this session only, cleared when
            ZWriter restarts
          </p>
        </div>
      </section>
      <section className="dictionary">
        <div className="add-word">
          <input
            aria-label="New ignored word"
            placeholder="Type a code, press Enter"
            value={newIgnored}
            onChange={(e) => setNewIgnored(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                addIgnoredWord();
              }
            }}
          />
          <button aria-label="Add ignored word" onClick={addIgnoredWord}>
            Add
          </button>
        </div>
        {settings.ignoredWords.length === 0 ? (
          <p className="sub">Nothing ignored this session.</p>
        ) : (
          <div className="word-list">
            {[...settings.ignoredWords]
              .sort((a, b) => a.toLowerCase().localeCompare(b.toLowerCase()))
              .map((w) => (
                <span className="word-chip" key={w.toLowerCase()}>
                  {w}
                  <button
                    className="chip-x"
                    aria-label={`Remove ignored word ${w}`}
                    title={`Remove ${w}`}
                    onClick={() => removeIgnoredWord(w)}
                  >
                    ✕
                  </button>
                </span>
              ))}
          </div>
        )}
      </section>

      <section className="setting-row">
        <div>
          <strong>Quick fix: skip unsure abbreviations</strong>
          <p className="sub">
            Near-matches like se2 are left as typed instead of opening the
            review window (ambiguous words still open it)
          </p>
        </div>
        <input
          type="checkbox"
          aria-label="Skip unsure abbreviations"
          checked={settings.skipGuessWindow}
          onChange={(e) => toggleSkipGuessWindow(e.target.checked)}
        />
      </section>

      <section className="setting-row">
        <div>
          <strong>Quick fix: auto-apply unsure abbreviations</strong>
          <p className="sub">
            Use the guessed expansion without asking; when the review window
            opens, it is already applied
          </p>
        </div>
        <input
          type="checkbox"
          aria-label="Auto-apply unsure abbreviations"
          checked={settings.autoApplyGuesses}
          onChange={(e) => toggleAutoApplyGuesses(e.target.checked)}
        />
      </section>

      <section className="setting-row">
        <div>
          <strong>Start with Windows</strong>
          <p className="sub">Runs minimized to the tray when you log in</p>
        </div>
        <input
          type="checkbox"
          checked={settings.autostart}
          onChange={(e) => toggleAutostart(e.target.checked)}
        />
      </section>

      <section className="setting-row">
        <div>
          <strong>History</strong>
          <p className="sub">Last 50 fixes, viewable in the review window</p>
        </div>
        <button className="danger" onClick={clearHistory}>
          Clear
        </button>
      </section>

      {error && <p className="status error">{error}</p>}

      <footer className="about">
        ZWriter v0.1.0 — offline grammar fixing powered by Harper. English only. Nothing leaves
        your machine.
      </footer>
    </main>
  );
}

export default Settings;
