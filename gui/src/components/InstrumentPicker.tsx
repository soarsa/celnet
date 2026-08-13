/**
 * InstrumentPicker — a searchable, grouped combobox for choosing a real instrument from
 * reference data, in place of a free-text contract-code field.
 *
 * A trader typing `ZFU26` can be wrong in ways a picker cannot be: a contract that does
 * not exist, one that has stopped trading, one no LP quotes. The options come from the
 * reference-data registry (see {@link ../lib/instrumentPicker}), carry the terms a desk
 * recognises the instrument by, and mark which ones a live composite currently covers —
 * the difference between a hedge that executes and one that backstops.
 *
 * Follows the WAI-ARIA combobox pattern: the text input owns `role="combobox"` with
 * `aria-expanded` / `aria-activedescendant`, the popup is a `listbox` of `option`s with
 * group headings, and ArrowUp/ArrowDown/Enter/Escape all work without a mouse. `allowRaw`
 * keeps a value the registry does not carry addressable rather than silently discarding a
 * previously-configured id.
 */
import { useEffect, useId, useMemo, useRef, useState } from "react";

import {
  filterInstrumentGroups,
  findInstrumentOption,
  groupInstrumentOptions,
  type InstrumentOption,
} from "../lib/instrumentPicker";
import styles from "./InstrumentPicker.module.css";

interface InstrumentPickerProps {
  /** The options to choose from (already ordered — see `hedgeVehicleOptions`). */
  options: readonly InstrumentOption[];
  /** The currently-selected value (an instrument id or futures product symbol). */
  value: string;
  /** Called with the chosen option; `null` when the selection is cleared. */
  onChange: (option: InstrumentOption | null) => void;
  /** Accessible label for the input. */
  label: string;
  /** Placeholder shown when nothing is selected. */
  placeholder?: string;
  disabled?: boolean;
  /** `data-testid` for the input. */
  testId?: string;
  /**
   * Keep a `value` that matches no option selectable and visible, rather than showing
   * an empty field. A registry configured before an instrument was retired still has to
   * render what it says.
   */
  allowRaw?: boolean;
  /**
   * Commit free-typed text that matches no option (on blur, or Enter with nothing
   * highlighted). Reference data does not cover every instrument a desk may hedge with
   * — the committed universe is US-only today — so a picker that could ONLY pick would
   * block configurations that are perfectly valid. Picking stays the default path; this
   * is the escape hatch, and the value is flagged as unknown once committed.
   */
  onRawCommit?: (raw: string) => void;
}

export function InstrumentPicker({
  options,
  value,
  onChange,
  label,
  placeholder = "Search instruments…",
  disabled = false,
  testId,
  allowRaw = true,
  onRawCommit,
}: InstrumentPickerProps): React.ReactElement {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [activeIndex, setActiveIndex] = useState(0);
  const rootRef = useRef<HTMLDivElement>(null);
  const listId = useId();
  const optionId = (i: number): string => `${listId}-opt-${i}`;

  const selected = useMemo(
    () => findInstrumentOption(options, value),
    [options, value],
  );

  const groups = useMemo(
    () => filterInstrumentGroups(groupInstrumentOptions(options), query),
    [options, query],
  );

  // The visible options FLATTENED in render order — keyboard navigation moves through
  // this, so the highlighted row is always the one Enter will choose.
  const flat = useMemo(() => groups.flatMap((g) => g.options), [groups]);

  // A narrowed list can be shorter than the last active index; clamp rather than point
  // the highlight at a row that no longer exists.
  useEffect(() => {
    setActiveIndex((i) => (i < flat.length ? i : 0));
  }, [flat.length]);

  // Close on an outside click, so the popup never strands over the form.
  useEffect(() => {
    if (!open) return undefined;
    const onDocPointerDown = (e: PointerEvent): void => {
      if (!rootRef.current?.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("pointerdown", onDocPointerDown);
    return () => document.removeEventListener("pointerdown", onDocPointerDown);
  }, [open]);

  const choose = (opt: InstrumentOption): void => {
    onChange(opt);
    setQuery("");
    setOpen(false);
  };

  /**
   * Commit whatever was typed when it matches no option. Called on blur and on Enter
   * with nothing highlighted, so a desk can still name an instrument reference data
   * does not carry rather than being unable to express it at all.
   */
  const commitRaw = (): void => {
    const raw = query.trim();
    if (!allowRaw || !onRawCommit || raw.length === 0) return;
    const exact = findInstrumentOption(options, raw);
    if (exact) {
      choose(exact);
      return;
    }
    onRawCommit(raw);
    setQuery("");
    setOpen(false);
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLInputElement>): void => {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      if (!open) {
        setOpen(true);
        return;
      }
      if (flat.length === 0) return;
      const delta = e.key === "ArrowDown" ? 1 : -1;
      setActiveIndex((i) => (i + delta + flat.length) % flat.length);
      return;
    }
    if (e.key === "Enter") {
      if (!open) return;
      const opt = flat[activeIndex];
      e.preventDefault();
      if (opt) choose(opt);
      else commitRaw();
      return;
    }
    if (e.key === "Escape" && open) {
      e.preventDefault();
      setOpen(false);
      setQuery("");
    }
  };

  // What the input shows: the live query while searching, else the selection. A value
  // the registry does not carry is shown verbatim (never blanked) when `allowRaw`.
  const display = open
    ? query
    : (selected?.label ?? (allowRaw && value ? value : ""));

  const unknownValue = value.length > 0 && !selected;

  return (
    <div className={styles.root} ref={rootRef}>
      <input
        className={styles.input}
        type="text"
        role="combobox"
        aria-label={label}
        aria-expanded={open}
        aria-controls={listId}
        aria-autocomplete="list"
        aria-activedescendant={
          open && flat.length > 0 ? optionId(activeIndex) : undefined
        }
        value={display}
        placeholder={placeholder}
        disabled={disabled}
        data-testid={testId}
        onFocus={() => setOpen(true)}
        onChange={(e) => {
          setQuery(e.target.value);
          setActiveIndex(0);
          setOpen(true);
        }}
        onKeyDown={onKeyDown}
        onBlur={commitRaw}
      />

      {!open && selected && (
        <p className={styles.selectedMeta}>
          {selected.sublabel}
          {selected.isRollingProduct && (
            <span className={styles.rollBadge}>auto-rolls</span>
          )}
        </p>
      )}
      {!open && unknownValue && allowRaw && (
        <p className={styles.unknownMeta} role="status">
          Not in reference data — this vehicle cannot be priced or filled until it is.
        </p>
      )}

      {open && (
        <div className={styles.popup}>
          <ul
            className={styles.list}
            id={listId}
            role="listbox"
            aria-label={label}
          >
            {groups.length === 0 && (
              <li className={styles.empty} role="presentation">
                No instrument matches “{query}”.
              </li>
            )}
            {groups.map((g) => {
              // The running offset into `flat`, so an option's id matches the index
              // keyboard navigation uses.
              const first = flat.indexOf(g.options[0] as InstrumentOption);
              return (
                <li key={g.group} role="presentation">
                  <p className={styles.groupHeading}>{g.group}</p>
                  <ul
                    className={styles.groupList}
                    role="group"
                    aria-label={g.group}
                  >
                    {g.options.map((o, i) => {
                      const index = first + i;
                      return (
                        <li
                          key={o.value}
                          id={optionId(index)}
                          role="option"
                          aria-selected={o.value === value}
                          className={`${styles.option} ${
                            index === activeIndex ? styles.optionActive : ""
                          }`}
                          data-testid={`instrument-option-${o.value}`}
                          onPointerDown={(e) => {
                            // Choose before the input's blur can close the popup.
                            e.preventDefault();
                            choose(o);
                          }}
                          onPointerEnter={() => setActiveIndex(index)}
                        >
                          <span className={styles.optionLabel}>
                            {o.label}
                            {o.hasLiveLiquidity && (
                              <span
                                className={styles.liveBadge}
                                title="A live composite currently covers this instrument"
                              >
                                live
                              </span>
                            )}
                          </span>
                          <span className={styles.optionSub}>{o.sublabel}</span>
                        </li>
                      );
                    })}
                  </ul>
                </li>
              );
            })}
          </ul>
        </div>
      )}
    </div>
  );
}
