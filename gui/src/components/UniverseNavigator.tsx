/**
 * UniverseNavigator — the desk-scale pair-universe browser (TRADING-UNIVERSE-
 * SCALE §5: navigate a large pair universe). A keyboard-first, searchable,
 * grouped (Majors / Crosses / Emerging) overlay launched from the toolbar
 * (button beside the PairMenu). It RE-TARGETS the single global active pair
 * (`app.setPair`) — it does not introduce a second pair concept. Favourites and
 * recents float to the top so the desk's working set is one keystroke away;
 * real seeded spots are shown at each pair's pip precision; the active pair is
 * marked in brand coral.
 *
 * HONESTY (CLAUDE.md rule 2): this navigates TODAY'S seeded pairs. The full
 * pair-universe registry (hundreds of pairs, P1-10) is NOT built — the footer
 * states the honest count + scope. The structure (virtualisation-ready flat
 * list, grouped sections, fuzzy search) scales unchanged when the registry lands.
 *
 * Keyboard grammar: type to filter; ↑/↓ move the highlight across the FLAT
 * ordered result list (favourites → recents → grouped buckets); Enter activates
 * the highlighted pair (and closes); ⌘D / "f" toggles its favourite; Escape
 * closes. The list auto-scrolls the highlight into view.
 */

import { useCallback, useEffect, useId, useMemo, useRef, useState } from "react";
import { useApp } from "../app/AppContext";
import {
  groupHits,
  samePair,
  searchUniverse,
  type UniverseHit,
  type UniversePair,
} from "../lib/universe";
import styles from "./UniverseNavigator.module.css";

/** A flat, ordered navigable item: a section header or a pair row. */
type NavItem =
  | { kind: "header"; key: string; label: string; count: number }
  | { kind: "pair"; key: string; pair: UniversePair; indices: number[]; section: string };

/** Highlight a label given the fuzzy-matched character indices. */
function Highlighted({ text, indices }: { text: string; indices: number[] }): React.ReactElement {
  if (indices.length === 0) return <>{text}</>;
  const set = new Set(indices);
  return (
    <>
      {Array.from(text).map((ch, i) =>
        set.has(i) ? (
          <mark key={i} className={styles.mark}>
            {ch}
          </mark>
        ) : (
          <span key={i}>{ch}</span>
        ),
      )}
    </>
  );
}

export function UniverseNavigator(): React.ReactElement | null {
  const app = useApp();
  const open = app.navigatorOpen;
  const [query, setQuery] = useState("");
  const [cursor, setCursor] = useState(0);
  const inputRef = useRef<HTMLInputElement | null>(null);
  const listRef = useRef<HTMLDivElement | null>(null);
  const rowRefs = useRef<Map<string, HTMLButtonElement | null>>(new Map());
  const titleId = useId();

  // Reset transient state each time the overlay opens; focus the search box.
  useEffect(() => {
    if (!open) return;
    setQuery("");
    setCursor(0);
    const t = requestAnimationFrame(() => inputRef.current?.focus());
    return () => cancelAnimationFrame(t);
  }, [open]);

  // Close on outside click / Escape handled inline below (overlay scrim click).
  const close = useCallback(() => app.setNavigatorOpen(false), [app]);

  // Build the ordered, flat navigable item list: Favourites, Recents, then the
  // canonical buckets — each filtered by the current fuzzy query.
  const { items, pairItems } = useMemo(() => {
    const hits = searchUniverse(app.universe, query);
    const hitById = new Map<string, UniverseHit>(hits.map((h) => [h.pair.id, h]));
    const out: NavItem[] = [];

    const pushSection = (label: string, key: string, sectionHits: UniverseHit[]): void => {
      if (sectionHits.length === 0) return;
      out.push({ kind: "header", key: `h-${key}`, label, count: sectionHits.length });
      for (const h of sectionHits) {
        out.push({
          kind: "pair",
          key: `${key}-${h.pair.id}`,
          pair: h.pair,
          indices: h.indices,
          section: key,
        });
      }
    };

    // Favourites + recents only appear when not actively searching (an idle
    // navigator surfaces the working set; a query collapses to relevance).
    if (query.trim().length === 0) {
      const favHits = [...app.favourites]
        .map((id) => hitById.get(id))
        .filter((h): h is UniverseHit => Boolean(h));
      pushSection("Favourites", "fav", favHits);

      const recentHits = app.recents
        .map((id) => hitById.get(id))
        .filter((h): h is UniverseHit => Boolean(h))
        .filter((h) => !app.favourites.has(h.pair.id));
      pushSection("Recent", "recent", recentHits);
    }

    for (const g of groupHits(hits)) {
      pushSection(g.label, g.bucket, g.hits);
    }

    const pairItemsOnly = out.filter((i): i is Extract<NavItem, { kind: "pair" }> => i.kind === "pair");
    return { items: out, pairItems: pairItemsOnly };
  }, [app.universe, app.favourites, app.recents, query]);

  // Clamp the cursor to the available pair rows whenever the result set changes.
  useEffect(() => {
    setCursor((c) => Math.max(0, Math.min(c, Math.max(0, pairItems.length - 1))));
  }, [pairItems.length]);

  // Scroll the highlighted row into view as the cursor moves.
  useEffect(() => {
    const item = pairItems[cursor];
    if (!item) return;
    rowRefs.current.get(item.key)?.scrollIntoView({ block: "nearest" });
  }, [cursor, pairItems]);

  const activate = useCallback(
    (pair: UniversePair): void => {
      app.setPair(pair.pair);
      close();
    },
    [app, close],
  );

  const onKeyDown = useCallback(
    (e: React.KeyboardEvent): void => {
      if (e.key === "Escape") {
        e.preventDefault();
        close();
        return;
      }
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setCursor((c) => Math.min(pairItems.length - 1, c + 1));
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        setCursor((c) => Math.max(0, c - 1));
      } else if (e.key === "Enter") {
        e.preventDefault();
        const item = pairItems[cursor];
        if (item) activate(item.pair);
      } else if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "d") {
        // ⌘D toggles the highlighted pair's favourite without closing.
        e.preventDefault();
        const item = pairItems[cursor];
        if (item) app.toggleFavourite(item.pair.pair);
      }
    },
    [pairItems, cursor, activate, app, close],
  );

  if (!open) return null;

  const activePair = app.pairCtx.pair;

  return (
    <div className={styles.scrim} onMouseDown={close} role="presentation">
      <div
        className={styles.panel}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        onMouseDown={(e) => e.stopPropagation()}
        onKeyDown={onKeyDown}
      >
        <header className={styles.head}>
          <h2 id={titleId} className={styles.title}>
            Pairs
          </h2>
          <input
            ref={inputRef}
            className={`num ${styles.search}`}
            type="text"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search pairs — EUR, usdjpy, jpy…"
            aria-label="search currency pairs"
            spellCheck={false}
            autoComplete="off"
          />
        </header>

        <div className={styles.list} ref={listRef} role="listbox" aria-label="currency pairs">
          {pairItems.length === 0 ? (
            <p className={styles.empty}>No pair matches “{query.trim()}”.</p>
          ) : (
            items.map((item) => {
              if (item.kind === "header") {
                return (
                  <div key={item.key} className={styles.sectionHead} aria-hidden="true">
                    <span className={styles.sectionLabel}>{item.label}</span>
                    <span className={`num ${styles.sectionCount}`}>{item.count}</span>
                  </div>
                );
              }
              const u = item.pair;
              const isActive = samePair(u.pair, activePair);
              const isFav = app.favourites.has(u.id);
              const flatIndex = pairItems.indexOf(item);
              const isCursor = flatIndex === cursor;
              return (
                <button
                  key={item.key}
                  ref={(el) => {
                    rowRefs.current.set(item.key, el);
                  }}
                  type="button"
                  role="option"
                  aria-selected={isCursor}
                  className={[
                    styles.row,
                    isActive ? styles.rowActive : "",
                    isCursor ? styles.rowCursor : "",
                  ].join(" ")}
                  onClick={() => activate(u)}
                  onMouseMove={() => setCursor(flatIndex)}
                  title={`${u.label} — set active pair`}
                >
                  <button
                    type="button"
                    className={`${styles.star} ${isFav ? styles.starOn : ""}`}
                    aria-label={isFav ? `unfavourite ${u.label}` : `favourite ${u.label}`}
                    aria-pressed={isFav}
                    onClick={(e) => {
                      e.stopPropagation();
                      app.toggleFavourite(u.pair);
                    }}
                  >
                    {isFav ? "★" : "☆"}
                  </button>
                  <span className={styles.rowPair}>
                    <Highlighted text={u.label} indices={item.indices} />
                  </span>
                  <span className={`num ${styles.rowSpot}`}>
                    {u.market.spot.toFixed(u.pipDecimals)}
                  </span>
                  {isActive && <span className={styles.activeDot} aria-hidden="true" />}
                </button>
              );
            })
          )}
        </div>

        <footer className={styles.foot}>
          <span className={styles.hint}>
            <kbd className={styles.kbd}>↑↓</kbd> move <kbd className={styles.kbd}>↵</kbd> select{" "}
            <kbd className={styles.kbd}>⌘D</kbd> favourite <kbd className={styles.kbd}>esc</kbd> close
          </span>
          <span className={styles.scope}>
            {app.universe.all.length} pairs · seeded set (full registry: Phase 1)
          </span>
        </footer>
      </div>
    </div>
  );
}
