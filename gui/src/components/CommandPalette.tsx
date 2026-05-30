/**
 * CommandPalette (⌘K) — the universal escape hatch and the keyboard-first spine
 * (GUI-DESIGN principle 6, §2). Fuzzy over pairs, workspaces, and actions. No
 * function codes, no <GO>. Rendered on the `hud` material (thick blur) — the one
 * thing that owns focus when open. Fully keyboard-driven; honours Escape.
 */

import { useEffect, useMemo, useRef, useState } from "react";
import { fuzzyMatch } from "../lib/fuzzy";
import styles from "./CommandPalette.module.css";

export interface Command {
  id: string;
  title: string;
  hint?: string;
  group: string;
  run: () => void;
}

export interface CommandPaletteProps {
  open: boolean;
  commands: Command[];
  onClose: () => void;
}

export function CommandPalette({
  open,
  commands,
  onClose,
}: CommandPaletteProps): React.ReactElement | null {
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (open) {
      setQuery("");
      setActive(0);
      // Focus on next tick so the element exists.
      requestAnimationFrame(() => inputRef.current?.focus());
    }
  }, [open]);

  const results = useMemo(() => {
    const scored = commands
      .map((cmd) => {
        const m = fuzzyMatch(query, `${cmd.title} ${cmd.hint ?? ""}`);
        return m ? { cmd, score: m.score } : null;
      })
      .filter((x): x is { cmd: Command; score: number } => x !== null);
    scored.sort((a, b) => b.score - a.score);
    return scored.slice(0, 9).map((s) => s.cmd);
  }, [commands, query]);

  useEffect(() => {
    if (active >= results.length) setActive(Math.max(0, results.length - 1));
  }, [results.length, active]);

  if (!open) return null;

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Escape") {
      e.preventDefault();
      onClose();
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      setActive((a) => Math.min(results.length - 1, a + 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setActive((a) => Math.max(0, a - 1));
    } else if (e.key === "Enter") {
      e.preventDefault();
      const cmd = results[active];
      if (cmd) {
        cmd.run();
        onClose();
      }
    }
  };

  return (
    <div className={styles.scrim} onMouseDown={onClose}>
      <div
        className={styles.palette}
        role="dialog"
        aria-modal="true"
        aria-label="command palette"
        onMouseDown={(e) => e.stopPropagation()}
        onKeyDown={onKeyDown}
      >
        <div className={styles.inputRow}>
          <span className={styles.prompt}>⌘K</span>
          <input
            ref={inputRef}
            className={styles.input}
            placeholder="Search pairs, workspaces, actions…  (e.g. “EUR/USD” · “mark surface” · “stream … 25Δ RR”)"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            spellCheck={false}
            autoComplete="off"
          />
        </div>
        <ul className={styles.list} role="listbox">
          {results.length === 0 && (
            <li className={styles.empty}>No matches</li>
          )}
          {results.map((cmd, i) => (
            <li
              key={cmd.id}
              role="option"
              aria-selected={i === active}
              className={`${styles.item} ${i === active ? styles.activeItem : ""}`}
              onMouseEnter={() => setActive(i)}
              onMouseDown={(e) => {
                e.preventDefault();
                cmd.run();
                onClose();
              }}
            >
              <span className={styles.group}>{cmd.group}</span>
              <span className={styles.itemTitle}>{cmd.title}</span>
              {cmd.hint && <span className={styles.hint}>{cmd.hint}</span>}
            </li>
          ))}
        </ul>
      </div>
    </div>
  );
}
