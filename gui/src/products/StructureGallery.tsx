/**
 * StructureGallery (GW2) — the grouped, searchable, keyboard-first structure
 * picker that REPLACES the flat 19-item `<select>` in the ticket head. The
 * catalogue is driven entirely from the {@link PRODUCT_REGISTRY} via
 * {@link registryByGroup}, so adding a product family adds a card here with no
 * edit — the same registry-as-seam discipline the rest of GW2 follows.
 *
 * ASSET-CLASS AWARE (multi-asset wave): given the active underlier's `assetClass`,
 * each family is partitioned by {@link galleryCardStates} into AVAILABLE (builds a
 * priceable instrument on this class — selectable), DIMMED (no spec of this arm
 * prices on the class — shown dimmed with the honest capability reason so the
 * matrix is visible, never a dead end discovered at request time) and HIDDEN (a
 * sibling spec of the same arm is the builder on this class — no duplicate card).
 * On FX/metal every family is available; on equity/commodity/crypto only the
 * cost-of-carry leaves + the agnostic arms are, and the FX/metal-only exotics dim.
 *
 * Accessibility: a single `role="listbox"` owns keyboard focus and a roving
 * `aria-activedescendant`; each AVAILABLE family card is a `role="option"`, dimmed
 * cards are `aria-disabled` and skipped by the roving walk; arrow keys move across
 * the available cards only, Enter/Space select, and the search field filters by
 * label / summary / keywords (case-insensitive substring) — type-to-search.
 *
 * Honest empties (GW0): when a search matches nothing the list renders an explicit
 * "No structures match" message rather than a blank panel.
 */
import { useEffect, useId, useMemo, useRef, useState } from "react";

import { PRODUCT_REGISTRY, registryByGroup } from "./index";
import { galleryCardStates, type CardState } from "./capability";
import type { AnyProductSpec, AssetClass, ProductGroup } from "./types";
import styles from "./StructureGallery.module.css";

export interface StructureGalleryProps {
  /** The currently-selected structure id. */
  value: string;
  /** Called with the chosen structure id when a card is activated. */
  onSelect: (id: string) => void;
  /** The catalogue to render (defaults to the full registry; injectable for tests). */
  specs?: readonly AnyProductSpec[];
  /** The active underlier's asset class — partitions cards available/dimmed/hidden. */
  assetClass?: AssetClass;
}

/** A filtered group section + its (already-filtered) specs, in catalogue order. */
interface FilteredGroup {
  group: ProductGroup;
  specs: AnyProductSpec[];
}

/** Does this spec match the (lower-cased, trimmed) query across label/summary/keywords? */
function matches(spec: AnyProductSpec, q: string): boolean {
  if (q === "") return true;
  const hay = `${spec.label} ${spec.summary} ${spec.keywords.join(" ")}`.toLowerCase();
  return hay.includes(q);
}

const isDimmed = (st: CardState | undefined): st is { dimmed: true; reason: string } =>
  typeof st === "object" && st !== null;

export function StructureGallery({
  value,
  onSelect,
  specs = PRODUCT_REGISTRY,
  assetClass = "FX",
}: StructureGalleryProps): React.ReactElement {
  const [query, setQuery] = useState("");
  const listRef = useRef<HTMLDivElement>(null);
  const baseId = useId();

  // The per-card state for the active asset class (available / dimmed / hidden),
  // computed over the full injected catalogue so the same-arm dedup is correct.
  const cardStates = useMemo(() => galleryCardStates(specs, assetClass), [specs, assetClass]);

  // The grouped catalogue, in canonical group + catalogue order, narrowed by the
  // search query and the injected `specs`, with HIDDEN cards dropped (a duplicate
  // of an available sibling arm). Dimmed cards stay — the matrix must be visible.
  const allowed = useMemo(() => new Set(specs.map((s) => s.id)), [specs]);
  const groups: FilteredGroup[] = useMemo(() => {
    const q = query.trim().toLowerCase();
    return registryByGroup()
      .map((g) => ({
        group: g.group,
        specs: g.specs.filter(
          (s) => allowed.has(s.id) && cardStates.get(s.id) !== "hidden" && matches(s, q),
        ),
      }))
      .filter((g) => g.specs.length > 0);
  }, [allowed, cardStates, query]);

  // The flattened keyboard-navigable list is the AVAILABLE cards only (dimmed cards
  // are shown but not selectable / not in the roving walk). The active descendant
  // is the selected available card, falling back to the first available one.
  const flat = useMemo(
    () => groups.flatMap((g) => g.specs).filter((s) => cardStates.get(s.id) === "available"),
    [groups, cardStates],
  );
  const activeIndex = useMemo(() => {
    const sel = flat.findIndex((s) => s.id === value);
    return sel >= 0 ? sel : flat.length > 0 ? 0 : -1;
  }, [flat, value]);

  // When the active class makes the current selection unpriceable (e.g. switching
  // the underlier from EURUSD to AAPL while a barrier is selected), re-select the
  // first available family so the ticket never sits on an unbuildable structure.
  useEffect(() => {
    if (cardStates.get(value) !== "available" && flat.length > 0) {
      const first = flat[0];
      if (first && first.id !== value) onSelect(first.id);
    }
  }, [cardStates, value, flat, onSelect]);

  const optionId = (id: string) => `${baseId}-opt-${id}`;
  const activeSpec = activeIndex >= 0 ? flat[activeIndex] : undefined;
  const activeId = activeSpec ? optionId(activeSpec.id) : undefined;

  function move(delta: number) {
    if (flat.length === 0) return;
    const next = Math.min(flat.length - 1, Math.max(0, activeIndex + delta));
    const spec = flat[next];
    if (!spec) return;
    // Reveal the newly-active card and reflect it as the selection so the roving
    // descendant tracks selection (single-select listbox).
    onSelect(spec.id);
    requestAnimationFrame(() => {
      document.getElementById(optionId(spec.id))?.scrollIntoView({ block: "nearest" });
    });
  }

  function selectAt(index: number) {
    const spec = flat[index];
    if (spec) onSelect(spec.id);
  }

  function onKeyDown(ev: React.KeyboardEvent) {
    switch (ev.key) {
      case "ArrowDown":
      case "ArrowRight":
        ev.preventDefault();
        move(+1);
        break;
      case "ArrowUp":
      case "ArrowLeft":
        ev.preventDefault();
        move(-1);
        break;
      case "Home":
        ev.preventDefault();
        selectAt(0);
        break;
      case "End":
        ev.preventDefault();
        selectAt(flat.length - 1);
        break;
      case "Enter":
      case " ":
        ev.preventDefault();
        selectAt(activeIndex);
        break;
      default:
        break;
    }
  }

  const listLabelId = `${baseId}-label`;
  const total = flat.length;

  return (
    <div className={styles.gallery}>
      <div className={styles.searchRow}>
        <span className={styles.searchGlyph} aria-hidden="true">
          ⌕
        </span>
        <input
          className={styles.search}
          type="search"
          value={query}
          placeholder="Search structures — name, payoff, method…"
          aria-label="search structures"
          aria-controls={`${baseId}-list`}
          spellCheck={false}
          autoComplete="off"
          onChange={(ev) => setQuery(ev.target.value)}
        />
        <span className={styles.count} aria-live="polite" title={`${total} priceable on ${assetClass}`}>
          {total}
        </span>
      </div>

      <span id={listLabelId} hidden>
        Structure catalogue
      </span>

      {total === 0 && groups.length === 0 ? (
        <div className={styles.empty} role="status">
          No structures match
        </div>
      ) : (
        <div
          id={`${baseId}-list`}
          ref={listRef}
          className={styles.list}
          role="listbox"
          aria-labelledby={listLabelId}
          aria-activedescendant={activeId}
          tabIndex={0}
          onKeyDown={onKeyDown}
        >
          {groups.map((g) => {
            const groupId = `${baseId}-grp-${g.group.replace(/\s+/g, "-")}`;
            return (
              <section
                key={g.group}
                className={styles.group}
                role="group"
                aria-labelledby={groupId}
              >
                {/* ARIA: a listbox may only own group/option children, so the
                  * visual group label is PRESENTATIONAL (no heading role inside
                  * the listbox — the WAI-ARIA APG grouped-listbox pattern); the
                  * group still takes its accessible name from it via
                  * aria-labelledby. */}
                <h3 id={groupId} role="presentation" className={styles.groupLabel}>
                  {g.group}
                </h3>
                <div className={styles.cards}>
                  {g.specs.map((spec) => {
                    const st = cardStates.get(spec.id);
                    const dimmed = isDimmed(st);
                    const selected = !dimmed && spec.id === value;
                    const active = !dimmed && activeSpec?.id === spec.id;
                    return (
                      <div
                        key={spec.id}
                        id={dimmed ? undefined : optionId(spec.id)}
                        role={dimmed ? "presentation" : "option"}
                        aria-selected={dimmed ? undefined : selected}
                        aria-disabled={dimmed ? true : undefined}
                        title={dimmed ? st.reason : undefined}
                        className={[
                          styles.card,
                          dimmed ? styles.dimmed : "",
                          selected ? styles.selected : "",
                          active ? styles.active : "",
                        ]
                          .filter(Boolean)
                          .join(" ")}
                        onClick={dimmed ? undefined : () => onSelect(spec.id)}
                      >
                        <div className={styles.cardHead}>
                          <span className={styles.cardLabel}>{spec.label}</span>
                          <span className={styles.chip}>
                            {spec.family === "rates" ? "FI" : spec.assetClass}
                          </span>
                        </div>
                        <span className={styles.cardSummary}>
                          {dimmed ? st.reason : spec.summary}
                        </span>
                      </div>
                    );
                  })}
                </div>
              </section>
            );
          })}
        </div>
      )}
    </div>
  );
}
