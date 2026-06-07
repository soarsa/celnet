/**
 * ShortcutsOverlay (?) — the discoverable keyboard cheatsheet for the keyboard-
 * first GUI (GUI-DESIGN principle 6). Bound to `?` in the Shell and reachable as
 * a command-palette action, it enumerates the SAME binding grammar the Shell and
 * the overlays actually honour, read from the single `src/lib/shortcuts.ts`
 * source-of-truth — so the advertised bindings can never drift from the real ones.
 *
 * It is a modal dialog: focus moves into it on open, Escape closes it, the scrim
 * click closes it, and it carries an accessible name + grouped definition lists.
 */

import { useEffect, useRef } from "react";
import { groupedShortcuts } from "../lib/shortcuts";
import styles from "./ShortcutsOverlay.module.css";

export interface ShortcutsOverlayProps {
  open: boolean;
  onClose: () => void;
}

export function ShortcutsOverlay({
  open,
  onClose,
}: ShortcutsOverlayProps): React.ReactElement | null {
  const closeRef = useRef<HTMLButtonElement | null>(null);

  // Move focus into the dialog on open so the keyboard user lands inside the
  // modal (and Escape/Tab are scoped here), per the dialog a11y pattern.
  useEffect(() => {
    if (!open) return;
    const t = requestAnimationFrame(() => closeRef.current?.focus());
    return () => cancelAnimationFrame(t);
  }, [open]);

  if (!open) return null;

  const onKeyDown = (e: React.KeyboardEvent): void => {
    if (e.key === "Escape") {
      e.preventDefault();
      onClose();
    }
  };

  const sections = groupedShortcuts();

  return (
    <div className={styles.scrim} onMouseDown={onClose} role="presentation">
      <div
        className={styles.panel}
        role="dialog"
        aria-modal="true"
        aria-labelledby="shortcuts-title"
        onMouseDown={(e) => e.stopPropagation()}
        onKeyDown={onKeyDown}
      >
        <header className={styles.head}>
          <h2 id="shortcuts-title" className={styles.title}>
            Keyboard shortcuts
          </h2>
          <button
            ref={closeRef}
            type="button"
            className={styles.close}
            onClick={onClose}
            aria-label="close keyboard shortcuts"
          >
            <span aria-hidden="true">✕</span>
          </button>
        </header>

        <div className={styles.grid}>
          {sections.map((section) => (
            <section key={section.group} className={styles.section}>
              <h3 className={styles.sectionTitle}>{section.group}</h3>
              <dl className={styles.list}>
                {section.items.map((s) => (
                  <div key={s.id} className={styles.row}>
                    <dt className={styles.action}>{s.label}</dt>
                    <dd className={styles.chord}>
                      {s.keys.map((k, i) => (
                        <kbd key={i} className={styles.kbd}>
                          {k}
                        </kbd>
                      ))}
                    </dd>
                  </div>
                ))}
              </dl>
            </section>
          ))}
        </div>

        <footer className={styles.foot}>
          <span>
            Press <kbd className={styles.kbd}>?</kbd> any time · <kbd className={styles.kbd}>Esc</kbd>{" "}
            to close
          </span>
        </footer>
      </div>
    </div>
  );
}
