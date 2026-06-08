/**
 * StdError — the data-honesty primitive for Monte-Carlo precision (GW0).
 *
 * Mirrors the contract's `Quote.price_std_error` / `PriceResponse.price_std_error`
 * (field 7): a Monte-Carlo-priced product reports a standard error; every
 * closed-form product reports NONE (`undefined`). The honest-data contract here:
 *   - `undefined`  → closed form, render nothing (never fabricate a band).
 *   - `0`          → MC reported no error ⇒ closed-form-grade precision, nothing.
 *   - `> 0`        → render the "± x" precision band so the trader sees the MC
 *                    noise on the quoted value, never an over-precise illusion.
 *
 * The value is the absolute standard error on the same scale as the priced value
 * (premium fraction). When a `pv` is supplied (≠ 0) the band also shows the error
 * as a percentage of PV — the same form the ticket already used, now centralised.
 */

import styles from "./StdError.module.css";

export function StdError({
  /** Absolute MC standard error (premium fraction). Omitted/0 ⇒ render nothing. */
  value,
  /** Optional priced value, to express the error as a % of PV. */
  pv,
  /** Whether to prefix the line with the "Monte-Carlo · " provenance label. */
  labelled = true,
}: {
  // Explicitly `| undefined`: the honesty contract is that an absent/undefined
  // error renders nothing, so callers may pass an optional value directly (this
  // is required under `exactOptionalPropertyTypes`).
  value?: number | undefined;
  pv?: number | undefined;
  labelled?: boolean;
}): React.ReactElement | null {
  // Closed form (undefined / 0): no band. Never render "± 0".
  if (value === undefined || value <= 0) return null;

  const band = (value * 100).toFixed(4);
  const pct = pv !== undefined && pv !== 0 ? `${((value / Math.abs(pv)) * 100).toFixed(2)}% of PV` : undefined;

  return (
    <span className={`num ${styles.stdError}`} aria-label="price std error">
      {labelled ? "Monte-Carlo · std error " : ""}±{band}
      {pct !== undefined ? ` (${pct})` : ""}
    </span>
  );
}
