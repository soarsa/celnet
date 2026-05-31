/**
 * PairMenu — the toolbar pair switcher as a REAL anchored dropdown (closes the
 * product owner's "the dropdown doesn't work" gap: a ▾ caret must open a menu, not
 * a full-screen command modal). Clicking the trigger opens a compact popover of the
 * watched pairs (label + live spot, active marked in brand coral); selecting one
 * re-targets the global pair. A footer row escalates to the full command palette
 * (⌘K) for search across the entire universe — the right tool when the watchlist
 * is too long to eyeball. Closes on select, Escape, or an outside click.
 *
 * This is a quick, universally-correct affordance; the desk-scale pair navigation
 * (searchable, grouped majors/EM, favourites) is designed in the experience
 * architecture and supersedes this when built — but a click-to-open dropdown is
 * the table-stakes behaviour the caret promises.
 */

import { useCallback, useEffect, useId, useRef, useState } from "react";
import { useApp } from "../app/AppContext";
import type { CcyPair } from "../data/contract";
import styles from "./PairMenu.module.css";

function samePair(a: CcyPair, b: CcyPair): boolean {
  return a.base === b.base && a.quote === b.quote;
}

export function PairMenu(): React.ReactElement {
  const app = useApp();
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement | null>(null);
  const menuId = useId();
  const active = app.pairCtx.pair;

  // Close on outside click / Escape while open.
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent): void => {
      if (rootRef.current && !rootRef.current.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  const choose = useCallback(
    (pair: CcyPair): void => {
      app.setPair(pair);
      setOpen(false);
    },
    [app],
  );

  const searchAll = useCallback((): void => {
    setOpen(false);
    app.setPaletteOpen(true);
  }, [app]);

  return (
    <div className={styles.root} ref={rootRef}>
      <button
        type="button"
        className={styles.trigger}
        onClick={() => setOpen((o) => !o)}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls={open ? menuId : undefined}
        title="Switch pair"
      >
        <span className={`num ${styles.pair}`}>
          {active.base}/{active.quote}
        </span>
        <span className={`${styles.caret} ${open ? styles.caretOpen : ""}`} aria-hidden>
          ▾
        </span>
      </button>

      {open && (
        <div className={styles.menu} id={menuId} role="menu" aria-label="currency pairs">
          {app.pairs.map((ctx) => {
            const isActive = samePair(ctx.pair, active);
            return (
              <button
                key={`${ctx.pair.base}${ctx.pair.quote}`}
                type="button"
                role="menuitemradio"
                aria-checked={isActive}
                className={`${styles.item} ${isActive ? styles.itemActive : ""}`}
                onClick={() => choose(ctx.pair)}
              >
                <span className={styles.itemPair}>
                  {ctx.pair.base}/{ctx.pair.quote}
                </span>
                <span className={`num ${styles.itemSpot}`}>
                  {ctx.market.spot.toFixed(ctx.pipDecimals)}
                </span>
              </button>
            );
          })}
          <button type="button" role="menuitem" className={styles.searchAll} onClick={searchAll}>
            <span>Search all pairs &amp; commands</span>
            <kbd className={styles.kbd}>⌘K</kbd>
          </button>
        </div>
      )}
    </div>
  );
}
