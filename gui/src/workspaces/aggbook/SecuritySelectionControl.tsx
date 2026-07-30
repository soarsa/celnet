/**
 * SecuritySelectionControl — the Aggregated Book header "Securities…" picker: the
 * per-user "view only what I want" preference. It lists the FI reference-data
 * universe (grouped by issuer, searchable), lets the trader tick the securities to
 * see, and reports the chosen canonical `instrumentId`s up to the workspace, which
 * persists them as a client-side setting.
 *
 * A CLEARED selection means "show all" — never an accidentally-blank book — so the
 * summary reads "All" and the panel offers an explicit "Clear (show all)".
 *
 * The panel is a portalled, labelled `role="dialog"` (escapes the header's stacking
 * context, stays on-screen, dismisses on Escape / click-away). Search focus moves
 * into it on open.
 */

import { useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";

import {
  filterSecurityGroups,
  groupSecurityOptions,
  type SecurityOption,
} from "../../lib/aggBookSelection";
import { useAnchoredPopover } from "../../lib/useAnchoredPopover";
import styles from "./SecuritySelectionControl.module.css";

export function SecuritySelectionControl({
  options,
  selected,
  onChange,
}: {
  /** The full FI reference-data universe as pickable options. */
  options: readonly SecurityOption[];
  /** The currently-chosen canonical `instrumentId`s (empty ⇒ all). */
  selected: readonly string[];
  /** Report a new selection (empty array ⇒ show all). */
  onChange: (ids: string[]) => void;
}): React.ReactElement | null {
  const { open, setOpen, anchorRef, floatingRef, floatingStyle, reposition } =
    useAnchoredPopover<HTMLButtonElement>("bottom-start");
  const [query, setQuery] = useState("");
  const searchRef = useRef<HTMLInputElement>(null);

  const selectedSet = useMemo(() => new Set(selected), [selected]);
  const groups = useMemo(() => groupSecurityOptions(options), [options]);
  const visible = useMemo(() => filterSecurityGroups(groups, query), [groups, query]);

  // Move focus into the panel's search on open; re-clamp once it has laid out.
  useEffect(() => {
    if (!open) return;
    searchRef.current?.focus();
    reposition();
  }, [open, reposition]);

  // Nothing to pick from (no bond reference data yet): omit the control.
  if (options.length === 0) return null;

  const allIds = options.map((o) => o.instrumentId);
  const summary = selected.length === 0 ? "All" : `${selected.length} selected`;

  const toggle = (id: string): void => {
    const next = new Set(selectedSet);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    onChange([...next]);
  };
  const selectAll = (): void => onChange([...allIds]);
  const clear = (): void => onChange([]);

  return (
    <>
      <button
        ref={anchorRef}
        type="button"
        className={styles.trigger}
        aria-haspopup="dialog"
        aria-expanded={open}
        onClick={() => setOpen(!open)}
      >
        <span aria-hidden="true" className={styles.triggerGlyph}>
          ☰
        </span>
        Securities
        <span className={styles.count}>{summary}</span>
      </button>

      {open &&
        createPortal(
          <div
            ref={floatingRef}
            role="dialog"
            aria-label="Choose which securities to display"
            className={styles.panel}
            style={floatingStyle}
          >
            <div className={styles.panelHead}>
              <label className={styles.searchWrap}>
                <span className={styles.srOnly}>Search securities</span>
                <input
                  ref={searchRef}
                  type="search"
                  className={styles.search}
                  placeholder="Search by name, issuer, ISIN…"
                  value={query}
                  onChange={(e) => setQuery(e.target.value)}
                />
              </label>
              <div className={styles.bulk}>
                <button type="button" className={styles.bulkBtn} onClick={selectAll}>
                  Select all
                </button>
                <button
                  type="button"
                  className={styles.bulkBtn}
                  onClick={clear}
                  disabled={selected.length === 0}
                >
                  Clear (show all)
                </button>
              </div>
            </div>

            <div className={styles.body}>
              {visible.length === 0 ? (
                <p className={styles.noMatch}>No securities match “{query}”.</p>
              ) : (
                visible.map((g) => (
                  <fieldset key={g.group} className={styles.group}>
                    <legend className={styles.groupLegend}>{g.group}</legend>
                    {g.options.map((opt: SecurityOption) => (
                      <label key={opt.instrumentId} className={styles.option}>
                        <input
                          type="checkbox"
                          className={styles.check}
                          checked={selectedSet.has(opt.instrumentId)}
                          onChange={() => toggle(opt.instrumentId)}
                        />
                        <span className={styles.optText}>
                          <span className={styles.optLabel}>{opt.label}</span>
                          <span className={styles.optSub}>
                            {opt.sublabel}
                            {opt.isin ? ` · ${opt.isin}` : ""}
                          </span>
                        </span>
                      </label>
                    ))}
                  </fieldset>
                ))
              )}
            </div>

            <p className={styles.footNote}>
              {selected.length === 0
                ? "Showing all streaming securities."
                : "Showing only the ticked securities. Clear to show all."}
            </p>
          </div>,
          document.body,
        )}
    </>
  );
}
