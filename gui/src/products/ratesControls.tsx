/**
 * Shared, a11y-safe input controls for the fixed-income (rates) product families
 * (IRS / FRA / bond). Factored out of the per-family `InputBlock`s so the
 * segmented-tab pattern is homed once (GUIDE.md rule 10 — no duplication).
 *
 * The active tab is signalled with HIGH-CONTRAST primary text + an accent UNDERLINE
 * (`styles.rTabActive` — `box-shadow: inset 0 -2px 0 var(--accent)`), never with
 * accent-coloured text alone, so the selection reads at any contrast / for
 * colour-blind traders. `role="tablist"` + `aria-selected` give assistive tech the
 * selection semantics; the whole control is keyboard-operable (each tab is a native
 * `<button>`).
 */
import styles from "../workspaces/TicketWorkspace.module.css";

/** A labelled, a11y-safe segmented control over a small closed set of string options. */
export function RatesTabs<T extends string>({
  label,
  value,
  options,
  onChange,
}: {
  /** The control's accessible name (announced by `aria-label`). */
  label: string;
  /** The selected option value. */
  value: T;
  /** The `[value, display]` options in display order. */
  options: readonly (readonly [T, string])[];
  /** Select an option. */
  onChange: (next: T) => void;
}): React.ReactElement {
  return (
    <div className={styles.rTabs} role="tablist" aria-label={label}>
      {options.map(([v, text]) => (
        <button
          key={v}
          type="button"
          role="tab"
          aria-selected={value === v}
          className={`${styles.rTab} ${value === v ? styles.rTabActive : ""}`}
          onClick={() => onChange(v)}
        >
          {text}
        </button>
      ))}
    </div>
  );
}
