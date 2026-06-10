/**
 * StructureGallery (GW2) — the grouped, searchable, keyboard-first structure
 * picker that REPLACES the flat 19-item `<select>` in the ticket head. The
 * catalogue is driven entirely from the {@link PRODUCT_REGISTRY} via
 * {@link registryByGroup}, so adding a product family adds a card here with no
 * edit — the same registry-as-seam discipline the rest of GW2 follows.
 *
 * Accessibility: a single `role="listbox"` owns keyboard focus and a roving
 * `aria-activedescendant`; each family card is a `role="option"` with
 * `aria-selected`, grouped under `role="group"` sections labelled by their
 * {@link ProductGroup}. Arrow keys move the active descendant across the visible
 * (filtered) cards, Enter/Space select, and the search field above filters by
 * label / summary / keywords (case-insensitive substring) — type-to-search.
 *
 * Honest empties (GW0): when a search matches nothing the list renders an
 * explicit "No structures match" message rather than a blank panel.
 */
import { useId, useMemo, useRef, useState } from "react";

import { PRODUCT_REGISTRY, registryByGroup } from "./index";
import type { AnyProductSpec, ProductGroup } from "./types";
import styles from "./StructureGallery.module.css";

export interface StructureGalleryProps {
  /** The currently-selected structure id. */
  value: string;
  /** Called with the chosen structure id when a card is activated. */
  onSelect: (id: string) => void;
  /** The catalogue to render (defaults to the full registry; injectable for tests). */
  specs?: readonly AnyProductSpec[];
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

export function StructureGallery({
  value,
  onSelect,
  specs = PRODUCT_REGISTRY,
}: StructureGalleryProps): React.ReactElement {
  const [query, setQuery] = useState("");
  const listRef = useRef<HTMLDivElement>(null);
  const baseId = useId();

  // The grouped catalogue, in canonical group + catalogue order, narrowed by the
  // search query. Restrict to the injected `specs` so a test (or a future
  // asset-class filter) can scope the gallery.
  const allowed = useMemo(() => new Set(specs.map((s) => s.id)), [specs]);
  const groups: FilteredGroup[] = useMemo(() => {
    const q = query.trim().toLowerCase();
    return registryByGroup()
      .map((g) => ({
        group: g.group,
        specs: g.specs.filter((s) => allowed.has(s.id) && matches(s, q)),
      }))
      .filter((g) => g.specs.length > 0);
  }, [allowed, query]);

  // The flattened, visible-order list the keyboard walks. The active descendant
  // is whichever visible card is selected, falling back to the first visible one.
  const flat = useMemo(() => groups.flatMap((g) => g.specs), [groups]);
  const activeIndex = useMemo(() => {
    const sel = flat.findIndex((s) => s.id === value);
    return sel >= 0 ? sel : flat.length > 0 ? 0 : -1;
  }, [flat, value]);

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
        <span className={styles.count} aria-live="polite">
          {total}
        </span>
      </div>

      <span id={listLabelId} hidden>
        Structure catalogue
      </span>

      {total === 0 ? (
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
                    const selected = spec.id === value;
                    const active = activeSpec?.id === spec.id;
                    return (
                      <div
                        key={spec.id}
                        id={optionId(spec.id)}
                        role="option"
                        aria-selected={selected}
                        className={[
                          styles.card,
                          selected ? styles.selected : "",
                          active ? styles.active : "",
                        ]
                          .filter(Boolean)
                          .join(" ")}
                        onClick={() => onSelect(spec.id)}
                      >
                        <div className={styles.cardHead}>
                          <span className={styles.cardLabel}>{spec.label}</span>
                          <span className={styles.chip}>{spec.assetClass}</span>
                        </div>
                        <span className={styles.cardSummary}>{spec.summary}</span>
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
