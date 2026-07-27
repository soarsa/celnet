/**
 * HelpPanel — the rich, in-app renderer for one {@link HelpEntry}: purpose, how it
 * works, the WORKED numeric example (formatted), how to configure, when to use, and
 * the risks. A portal-mounted, accessible modal dialog (role="dialog",
 * aria-modal, focus moves in on mount, Escape + backdrop close), so it escapes the
 * editor's stacking context and never traps the trader. When the entry references a
 * guided tour and an `onStartTour` handler is supplied, a "Walk me through it"
 * button launches it.
 *
 * Presentational + controlled: it is visible exactly while mounted; the caller
 * (a {@link HelpButton} or the Help center) owns the open/closed state.
 */

import { useEffect, useRef } from "react";
import { createPortal } from "react-dom";

import type { HelpEntry } from "../lib/help";
import type { TourId } from "../lib/tours";
import styles from "./HelpPanel.module.css";

export interface HelpPanelProps {
  /** The topic to render. */
  entry: HelpEntry;
  /** Close the panel (Escape, backdrop, or the close button). */
  onClose: () => void;
  /** Launch the entry's guided tour — omit to hide the "Walk me through it" button. */
  onStartTour?: (id: TourId) => void;
}

const CATEGORY_LABEL: Record<HelpEntry["category"], string> = {
  feature: "Pricing feature",
  strategy: "Tiering strategy",
  concept: "Concept",
};

export function HelpPanel({ entry, onClose, onStartTour }: HelpPanelProps): React.ReactElement {
  const closeRef = useRef<HTMLButtonElement | null>(null);
  const titleId = `help-${entry.id}-title`;

  // Move focus into the dialog on open so Escape/Tab are scoped here.
  useEffect(() => {
    const t = requestAnimationFrame(() => closeRef.current?.focus());
    return () => cancelAnimationFrame(t);
  }, []);

  const onKeyDown = (e: React.KeyboardEvent): void => {
    if (e.key === "Escape") {
      e.preventDefault();
      onClose();
    }
  };

  const canTour = entry.tourId !== undefined && onStartTour !== undefined;

  return createPortal(
    <div className={styles.scrim} onMouseDown={onClose} role="presentation">
      <div
        className={styles.panel}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        data-testid="help-panel"
        onMouseDown={(e) => e.stopPropagation()}
        onKeyDown={onKeyDown}
      >
        <header className={styles.head}>
          <div className={styles.headMain}>
            <span className={styles.badge} data-category={entry.category}>
              {CATEGORY_LABEL[entry.category]}
            </span>
            <h2 id={titleId} className={styles.title}>
              {entry.title}
            </h2>
            <p className={styles.purpose}>{entry.purpose}</p>
          </div>
          <button
            ref={closeRef}
            type="button"
            className={styles.close}
            onClick={onClose}
            aria-label={`Close help for ${entry.title}`}
          >
            <span aria-hidden="true">✕</span>
          </button>
        </header>

        <div className={styles.scroll}>
          <section className={styles.section} aria-labelledby={`${entry.id}-how`}>
            <h3 id={`${entry.id}-how`} className={styles.sectionTitle}>
              How it works
            </h3>
            <p className={styles.prose}>{entry.howItWorks}</p>
          </section>

          <section className={styles.section} aria-label="Worked example">
            <h3 className={styles.sectionTitle}>Worked example</h3>
            <p className={styles.scenario}>{entry.example.scenario}</p>
            <dl className={styles.example}>
              {entry.example.rows.map((row, i) => (
                <div key={i} className={styles.exampleRow}>
                  <dt className={styles.exampleLabel}>{row.label}</dt>
                  <dd className={styles.exampleValue}>{row.value}</dd>
                </div>
              ))}
            </dl>
            {entry.example.takeaway && (
              <p className={styles.takeaway}>{entry.example.takeaway}</p>
            )}
          </section>

          <section className={styles.section} aria-label="How to configure">
            <h3 className={styles.sectionTitle}>How to configure</h3>
            <ol className={styles.steps}>
              {entry.howToConfigure.map((step, i) => (
                <li key={i} className={styles.step}>
                  {step}
                </li>
              ))}
            </ol>
          </section>

          <div className={styles.twoCol}>
            <section className={styles.section} aria-label="When to use">
              <h3 className={styles.sectionTitle}>When to use</h3>
              <p className={styles.prose}>{entry.whenToUse}</p>
            </section>
            <section className={styles.section} aria-label="Risks">
              <h3 className={`${styles.sectionTitle} ${styles.riskTitle}`}>Risks</h3>
              <p className={styles.prose}>{entry.risks}</p>
            </section>
          </div>
        </div>

        <footer className={styles.foot}>
          {canTour ? (
            <button
              type="button"
              className={styles.tourBtn}
              onClick={() => {
                onClose();
                onStartTour?.(entry.tourId as TourId);
              }}
            >
              <span aria-hidden="true">▸</span> Walk me through it
            </button>
          ) : (
            <span className={styles.footHint}>
              Press <kbd className={styles.kbd}>Esc</kbd> to close
            </span>
          )}
          <button type="button" className={styles.doneBtn} onClick={onClose}>
            Done
          </button>
        </footer>
      </div>
    </div>,
    document.body,
  );
}
