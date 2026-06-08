/**
 * ScopeSwitcher — the scope drill LEAF view (GW1-S3). The re-homed
 * `UniverseNavigator`: instead of a parallel "browse pairs" overlay, this is the
 * single drill affordance the ONE breadcrumb-scope control opens. It is the
 * terminal leaf of the scope grammar — and the four redundant pair affordances it
 * absorbs (PairMenu, the "Pairs" button, the ⌘K pair list, PairStrip) are deleted.
 *
 * What it shows depends on the child level being drilled into (from the scope
 * tail, via `lib/scope.childLevel`):
 *   • PAIR (terminal — book→pair drill, or re-selecting the active underlier):
 *     the live pair universe — favourites + recents + Majors/Crosses/Emerging
 *     buckets, fuzzy-searchable, real seeded spots. Selecting a pair re-targets
 *     the global underlier AND, if the scope is at-or-above book, drills the path
 *     to a `pair` crumb (FX terminal == active pair).
 *   • DESK / BOOK (non-terminal org drill): the entitlement-ready org scaffold
 *     (`ORG_SCAFFOLD`) — the desk/book child nodes — filtered by the same fuzzy
 *     search. Selecting one drills the scope DOWN one level with that label.
 *
 * Keyboard grammar (unchanged from the absorbed navigator): type to filter; ↑/↓
 * move the highlight; Enter activates; ⌘D toggles favourite (pair mode only); Esc
 * closes. The active option is announced via `aria-activedescendant` (combobox /
 * listbox roving pattern — the rows are `role="option"` with no nested focusable
 * controls, so no `nested-interactive` a11y violation).
 *
 * HONESTY (CLAUDE.md rule 2): pair mode navigates TODAY'S seeded pairs (the full
 * P1-10 registry is not built); the org scaffold is the grant-all entitlement seam
 * (a real org/entitlement feed replaces it with zero rework). Nothing is fabricated.
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
import { childLevel, currentLevel } from "../lib/scope";
import { ORG_SCAFFOLD } from "../data/seed";
import { fuzzyMatch } from "../lib/fuzzy";
import styles from "./ScopeSwitcher.module.css";

/** A flat, ordered navigable item: a section header or a selectable row. */
type NavItem =
  | { kind: "header"; key: string; label: string; count: number }
  | { kind: "pair"; key: string; pair: UniversePair; indices: number[] }
  | { kind: "org"; key: string; label: string; indices: number[] };

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

export function ScopeSwitcher(): React.ReactElement | null {
  const app = useApp();
  const open = app.scopeSwitcherOpen;
  const [query, setQuery] = useState("");
  const [cursor, setCursor] = useState(0);
  const inputRef = useRef<HTMLInputElement | null>(null);
  const listRef = useRef<HTMLDivElement | null>(null);
  const rowRefs = useRef<Map<string, HTMLDivElement | null>>(new Map());
  const titleId = useId();
  const listId = useId();
  const optId = useCallback(
    (key: string): string => `${listId}-${key.replace(/[^\w-]/g, "_")}`,
    [listId],
  );

  // The level being drilled into = the child of the current scope tail. When the
  // tail is already a pair (terminal), re-selecting stays at `pair` (the underlier
  // switch). Otherwise the child level decides org-drill vs pair-drill.
  const tailLevel = currentLevel(app.scope);
  const targetLevel = tailLevel === "pair" ? "pair" : (childLevel(tailLevel) ?? "pair");
  const mode: "pair" | "org" = targetLevel === "pair" ? "pair" : "org";

  // Reset transient state each time the switcher opens; focus the search box.
  useEffect(() => {
    if (!open) return;
    setQuery("");
    setCursor(0);
    const t = requestAnimationFrame(() => inputRef.current?.focus());
    return () => cancelAnimationFrame(t);
  }, [open]);

  const close = useCallback(() => app.setScopeSwitcherOpen(false), [app]);

  // Build the ordered, flat navigable item list for the current mode.
  const { items, selectable } = useMemo(() => {
    const out: NavItem[] = [];

    if (mode === "pair") {
      const hits = searchUniverse(app.universe, query);
      const hitById = new Map<string, UniverseHit>(hits.map((h) => [h.pair.id, h]));

      const pushSection = (label: string, key: string, sectionHits: UniverseHit[]): void => {
        if (sectionHits.length === 0) return;
        out.push({ kind: "header", key: `h-${key}`, label, count: sectionHits.length });
        for (const h of sectionHits) {
          out.push({ kind: "pair", key: `${key}-${h.pair.id}`, pair: h.pair, indices: h.indices });
        }
      };

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
      for (const g of groupHits(hits)) pushSection(g.label, g.bucket, g.hits);
    } else if (targetLevel === "desk" || targetLevel === "book") {
      // Org drill: the child nodes for the level being entered. At `desk` we list
      // every desk; at `book` we list the books under the desk the path is in.
      const labels = orgChildLabels(targetLevel, app.scope.path);
      const q = query.trim();
      const matched = q.length === 0 ? labels.map((l) => ({ label: l, indices: [] as number[] })) : labels
        .map((l) => {
          const m = fuzzyMatch(q, l);
          return m ? { label: l, indices: m.indices } : null;
        })
        .filter((x): x is { label: string; indices: number[] } => x !== null);
      if (matched.length > 0) {
        const headLabel = targetLevel === "desk" ? "Desks" : "Books";
        out.push({ kind: "header", key: "h-org", label: headLabel, count: matched.length });
        for (const m of matched) {
          out.push({ kind: "org", key: `org-${m.label}`, label: m.label, indices: m.indices });
        }
      }
    }

    const selectableOnly = out.filter(
      (i): i is Exclude<NavItem, { kind: "header" }> => i.kind !== "header",
    );
    return { items: out, selectable: selectableOnly };
  }, [mode, targetLevel, app.universe, app.favourites, app.recents, app.scope.path, query]);

  useEffect(() => {
    setCursor((c) => Math.max(0, Math.min(c, Math.max(0, selectable.length - 1))));
  }, [selectable.length]);

  useEffect(() => {
    const item = selectable[cursor];
    if (!item) return;
    rowRefs.current.get(item.key)?.scrollIntoView({ block: "nearest" });
  }, [cursor, selectable]);

  const activate = useCallback(
    (item: Exclude<NavItem, { kind: "header" }>): void => {
      if (item.kind === "pair") {
        app.setPair(item.pair.pair);
        // FX terminal == active pair: if standing above the pair level, the
        // underlier selection IS the leaf drill — append the pair crumb. If already
        // at a pair crumb, `setPair` re-targeted it in lock-step (no extra drill).
        if (currentLevel(app.scope) !== "pair") app.drillScopeDown(item.pair.label);
      } else {
        app.drillScopeDown(item.label);
      }
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
        setCursor((c) => Math.min(selectable.length - 1, c + 1));
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        setCursor((c) => Math.max(0, c - 1));
      } else if (e.key === "Enter") {
        e.preventDefault();
        const item = selectable[cursor];
        if (item) activate(item);
      } else if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "d" && mode === "pair") {
        // ⌘D toggles the highlighted pair's favourite without closing (pair mode).
        e.preventDefault();
        const item = selectable[cursor];
        if (item && item.kind === "pair") app.toggleFavourite(item.pair.pair);
      }
    },
    [selectable, cursor, activate, app, close, mode],
  );

  if (!open) return null;

  const activePair = app.pairCtx.pair;
  const title = mode === "pair" ? "Pairs" : targetLevel === "desk" ? "Desks" : "Books";
  const placeholder =
    mode === "pair"
      ? "Search pairs — EUR, usdjpy, jpy…"
      : `Search ${targetLevel === "desk" ? "desks" : "books"}…`;

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
            {title}
          </h2>
          <input
            ref={inputRef}
            className={`num ${styles.search}`}
            type="text"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder={placeholder}
            aria-label={`search ${title.toLowerCase()}`}
            role="combobox"
            aria-expanded="true"
            aria-controls={listId}
            aria-activedescendant={selectable[cursor] ? optId(selectable[cursor]!.key) : undefined}
            spellCheck={false}
            autoComplete="off"
          />
        </header>

        <div
          className={styles.list}
          ref={listRef}
          id={listId}
          role="listbox"
          aria-label={mode === "pair" ? "currency pairs" : "scope nodes"}
        >
          {selectable.length === 0 ? (
            <p className={styles.empty}>No match for “{query.trim()}”.</p>
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
              const flatIndex = selectable.indexOf(item);
              const isCursor = flatIndex === cursor;
              if (item.kind === "org") {
                return (
                  <div
                    key={item.key}
                    id={optId(item.key)}
                    ref={(el) => {
                      rowRefs.current.set(item.key, el);
                    }}
                    role="option"
                    aria-selected={isCursor}
                    aria-label={`${item.label} — drill scope`}
                    className={[styles.row, isCursor ? styles.rowCursor : ""].join(" ")}
                    onClick={() => activate(item)}
                    onMouseMove={() => setCursor(flatIndex)}
                  >
                    <span className={styles.rowPair}>
                      <Highlighted text={item.label} indices={item.indices} />
                    </span>
                  </div>
                );
              }
              const u = item.pair;
              const isActive = samePair(u.pair, activePair);
              const isFav = app.favourites.has(u.id);
              return (
                <div
                  key={item.key}
                  id={optId(item.key)}
                  ref={(el) => {
                    rowRefs.current.set(item.key, el);
                  }}
                  role="option"
                  aria-selected={isCursor}
                  aria-label={`${u.label}${isFav ? " (favourite)" : ""} — set active pair, ⌘D to ${
                    isFav ? "unfavourite" : "favourite"
                  }`}
                  className={[
                    styles.row,
                    isActive ? styles.rowActive : "",
                    isCursor ? styles.rowCursor : "",
                  ].join(" ")}
                  onClick={() => activate(item)}
                  onMouseMove={() => setCursor(flatIndex)}
                >
                  <span
                    className={`${styles.star} ${isFav ? styles.starOn : ""}`}
                    aria-hidden="true"
                    title={isFav ? `Unfavourite ${u.label} (⌘D)` : `Favourite ${u.label} (⌘D)`}
                    onClick={(e) => {
                      e.stopPropagation();
                      app.toggleFavourite(u.pair);
                    }}
                  >
                    {isFav ? "★" : "☆"}
                  </span>
                  <span className={styles.rowPair}>
                    <Highlighted text={u.label} indices={item.indices} />
                  </span>
                  <span className={`num ${styles.rowSpot}`}>
                    {u.market.spot.toFixed(u.pipDecimals)}
                  </span>
                  {isActive && <span className={styles.activeDot} aria-hidden="true" />}
                </div>
              );
            })
          )}
        </div>

        <footer className={styles.foot}>
          <span className={styles.hint}>
            <kbd className={styles.kbd}>↑↓</kbd> move <kbd className={styles.kbd}>↵</kbd> select{" "}
            {mode === "pair" && (
              <>
                <kbd className={styles.kbd}>⌘D</kbd> favourite{" "}
              </>
            )}
            <kbd className={styles.kbd}>esc</kbd> close
          </span>
          <span className={styles.scope}>
            {mode === "pair"
              ? `${app.universe.all.length} pairs · seeded set (full registry: Phase 1)`
              : "grant-all org scaffold (entitlement feed: Phase 1)"}
          </span>
        </footer>
      </div>
    </div>
  );
}

/**
 * The org child labels for the level being drilled into. At `desk`, every desk;
 * at `book`, the books under the desk the current path sits in (or every book if
 * no desk crumb yet — grant-all shows the union). Pure read of `ORG_SCAFFOLD`.
 */
function orgChildLabels(level: "desk" | "book", path: { level: string; label: string }[]): string[] {
  if (level === "desk") return ORG_SCAFFOLD.map((d) => d.desk);
  const deskCrumb = path.find((n) => n.level === "desk");
  if (deskCrumb) {
    const desk = ORG_SCAFFOLD.find((d) => d.desk === deskCrumb.label);
    if (desk) return desk.books;
  }
  return ORG_SCAFFOLD.flatMap((d) => d.books);
}
