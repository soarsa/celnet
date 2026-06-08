/**
 * EmptyValue — the platform's data-honesty primitive for absence (GW0).
 *
 * A missing/unknown value renders an em-dash "—", NEVER `0` and never a blank
 * cell: `0` is a real number a trader could mistake for a mark, and a blank cell
 * is indistinguishable from a layout bug. The dash carries an accessible reason
 * so screen-reader users (and the axe a11y sweep) learn WHY it is empty — e.g.
 * "no calibration for this tenor" — rather than just hearing "dash".
 *
 * This is the single component every grid/strip uses for honest empties, so the
 * data-honesty contract is encoded once and composed everywhere.
 */

import styles from "./EmptyValue.module.css";

export function EmptyValue({
  /** Why the value is absent — surfaced as the accessible label + tooltip. */
  reason = "no value",
}: {
  reason?: string;
}): React.ReactElement {
  return (
    <span className={`num ${styles.empty}`} role="img" aria-label={reason} title={reason}>
      —
    </span>
  );
}
