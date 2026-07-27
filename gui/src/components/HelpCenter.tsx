/**
 * HelpCenter — the global, searchable Help surface (opened from the "?" Help
 * affordance in the shell header). A portal modal with a search box: typing
 * "bid offer tiering" (or any feature / strategy / concept) surfaces the matching
 * {@link HelpEntry} rows, each with its purpose, an "Open" button (the rich
 * {@link HelpPanel}) and a "Walk me through it" button that launches the topic's
 * guided tour. With an empty query it shows the full topic INDEX grouped by
 * category, plus a list of every guided tutorial.
 *
 * Accessible: role="dialog", aria-modal, focus into the search box on open,
 * Escape + backdrop close, and an aria-live result count.
 */

import { useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";

import {
  HELP_INDEX,
  helpByCategory,
  searchHelp,
  type HelpCategory,
  type HelpEntry,
} from "../lib/help";
import { TOUR_INDEX } from "../lib/tours";
import { useTour } from "../app/TourProvider";
import { HelpPanel } from "./HelpPanel";
import styles from "./HelpCenter.module.css";

export interface HelpCenterProps {
  open: boolean;
  onClose: () => void;
}

const CATEGORY_ORDER: readonly { id: HelpCategory; label: string }[] = [
  { id: "concept", label: "Concepts" },
  { id: "feature", label: "Pricing features" },
  { id: "strategy", label: "Tiering strategies" },
];

export function HelpCenter({ open, onClose }: HelpCenterProps): React.ReactElement | null {
  const [query, setQuery] = useState("");
  const [openEntry, setOpenEntry] = useState<HelpEntry | null>(null);
  const searchRef = useRef<HTMLInputElement | null>(null);
  const { startTour } = useTour();

  useEffect(() => {
    if (!open) return;
    const t = requestAnimationFrame(() => searchRef.current?.focus());
    return () => cancelAnimationFrame(t);
  }, [open]);

  // Reset the transient state when the center is closed.
  useEffect(() => {
    if (!open) {
      setQuery("");
      setOpenEntry(null);
    }
  }, [open]);

  const trimmed = query.trim();
  const results = useMemo(() => (trimmed ? searchHelp(trimmed) : HELP_INDEX), [trimmed]);

  if (!open) return null;

  const onKeyDown = (e: React.KeyboardEvent): void => {
    if (e.key === "Escape") {
      e.preventDefault();
      onClose();
    }
  };

  const launch = (tourId: HelpEntry["tourId"]): void => {
    if (!tourId) return;
    onClose();
    startTour(tourId);
  };

  const renderRow = (entry: HelpEntry): React.ReactElement => (
    <li key={entry.id} className={styles.row}>
      <div className={styles.rowMain}>
        <div className={styles.rowHead}>
          <span className={styles.rowBadge} data-category={entry.category}>
            {entry.category}
          </span>
          <span className={styles.rowTitle}>{entry.title}</span>
        </div>
        <p className={styles.rowPurpose}>{entry.purpose}</p>
      </div>
      <div className={styles.rowActions}>
        <button type="button" className={styles.openBtn} onClick={() => setOpenEntry(entry)}>
          Open
        </button>
        {entry.tourId && (
          <button
            type="button"
            className={styles.walkBtn}
            onClick={() => launch(entry.tourId)}
          >
            <span aria-hidden="true">▸</span> Walk me through it
          </button>
        )}
      </div>
    </li>
  );

  return createPortal(
    <div className={styles.scrim} onMouseDown={onClose} role="presentation">
      <div
        className={styles.panel}
        role="dialog"
        aria-modal="true"
        aria-labelledby="help-center-title"
        data-testid="help-center"
        onMouseDown={(e) => e.stopPropagation()}
        onKeyDown={onKeyDown}
      >
        <header className={styles.head}>
          <div>
            <h2 id="help-center-title" className={styles.title}>
              Help &amp; tutorials
            </h2>
            <p className={styles.subtitle}>
              Search every pricing feature, tiering strategy, and concept — with worked
              examples and live walkthroughs.
            </p>
          </div>
          <button type="button" className={styles.close} onClick={onClose} aria-label="close help center">
            <span aria-hidden="true">✕</span>
          </button>
        </header>

        <div className={styles.searchRow}>
          <span className={styles.searchGlyph} aria-hidden="true">
            ⌕
          </span>
          <input
            ref={searchRef}
            type="search"
            className={styles.search}
            placeholder="Search help — e.g. “bid offer tiering”, “inventory skew”, “ESP vs RFS”…"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            aria-label="search help topics"
          />
        </div>

        <div className={styles.scroll}>
          {trimmed ? (
            <>
              <p className={styles.resultCount} role="status" aria-live="polite">
                {results.length} {results.length === 1 ? "topic" : "topics"} for “{trimmed}”
              </p>
              {results.length === 0 ? (
                <p className={styles.empty}>
                  No topics match. Try a feature name (TIERING, AXE), a strategy (Flat markup),
                  or a concept (pricing groups, provenance).
                </p>
              ) : (
                <ul className={styles.list}>{results.map(renderRow)}</ul>
              )}
            </>
          ) : (
            <>
              {CATEGORY_ORDER.map((cat) => (
                <section key={cat.id} className={styles.group} aria-label={cat.label}>
                  <h3 className={styles.groupTitle}>{cat.label}</h3>
                  <ul className={styles.list}>{helpByCategory(cat.id).map(renderRow)}</ul>
                </section>
              ))}

              <section className={styles.group} aria-label="Guided tutorials">
                <h3 className={styles.groupTitle}>Guided tutorials</h3>
                <ul className={styles.tourList}>
                  {TOUR_INDEX.map((t) => (
                    <li key={t.id} className={styles.tourRow}>
                      <div className={styles.rowMain}>
                        <span className={styles.rowTitle}>{t.title}</span>
                        <p className={styles.rowPurpose}>{t.summary}</p>
                      </div>
                      <button
                        type="button"
                        className={styles.walkBtn}
                        onClick={() => {
                          onClose();
                          startTour(t.id);
                        }}
                      >
                        <span aria-hidden="true">▸</span> Start
                      </button>
                    </li>
                  ))}
                </ul>
              </section>
            </>
          )}
        </div>
      </div>

      {openEntry && (
        <HelpPanel entry={openEntry} onClose={() => setOpenEntry(null)} onStartTour={launch} />
      )}
    </div>,
    document.body,
  );
}
