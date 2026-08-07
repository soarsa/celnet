/**
 * RiskBreakdownGrid — the Risk Dashboard drill-down body: a portfolio's routed
 * flow broken down TWO ways side by side — "By tenor" (coarse tenor buckets) and
 * "By instrument" (distinct instruments) — each a compact table of gross notional,
 * fill count and DV01. Pure/presentational: it takes the portfolio's already-
 * filtered {@link Deal}s and folds them client-side via {@link riskBreakdownFor}
 * (see that module for the honest data-source rationale). DV01 renders "—" when the
 * source carries none (never a fabricated 0), consistent with the dashboard's
 * aggregate DV01 seam. Numbers are right-aligned + tabular so digits line up.
 */

import { useMemo } from "react";
import type { Deal } from "../data/contract";
import { riskBreakdownFor, type BreakdownRow } from "../data/riskBreakdown";
import { fmtCompact } from "../lib/format";
import styles from "./RiskBreakdownGrid.module.css";

/** An optional metric: `null` ⇒ the honest "—" (not evaluated), never a 0. */
function optMetric(n: number | null): string {
  return n === null ? "—" : fmtCompact(n);
}

/** One compact breakdown table (a single lens: by tenor OR by instrument). */
function BreakdownTable({
  caption,
  keyHead,
  rows,
}: {
  caption: string;
  keyHead: string;
  rows: readonly BreakdownRow[];
}): React.ReactElement {
  return (
    <div className={styles.lens}>
      <h4 className={styles.lensTitle}>{caption}</h4>
      <table className={styles.table}>
        <thead>
          <tr>
            <th scope="col">{keyHead}</th>
            <th scope="col" className={styles.numCol}>
              Notional
            </th>
            <th scope="col" className={styles.numCol}>
              Fills
            </th>
            <th scope="col" className={styles.numCol}>
              DV01
            </th>
          </tr>
        </thead>
        <tbody>
          {rows.map((r) => (
            <tr key={r.key}>
              <td>{r.key}</td>
              <td className={styles.num}>{fmtCompact(r.notional)}</td>
              <td className={styles.num}>{r.count}</td>
              <td className={`${styles.num} ${r.dv01 === null ? styles.muted : ""}`}>
                {optMetric(r.dv01)}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

/**
 * The drill-down grid for one portfolio. `deals` must already be filtered to the
 * portfolio (`deal.riskBookId === bookId`). Empty routed flow shows an honest note
 * rather than empty tables.
 */
export function RiskBreakdownGrid({
  deals,
  bookName,
}: {
  deals: readonly Deal[];
  bookName: string;
}): React.ReactElement {
  const breakdown = useMemo(() => riskBreakdownFor(deals), [deals]);

  if (breakdown.count === 0) {
    return (
      <p className={styles.empty}>
        No routed fills to break down for {bookName} yet — routed deals appear here
        grouped by tenor and instrument as they book.
      </p>
    );
  }

  return (
    <div className={styles.grid} aria-label={`Risk breakdown for ${bookName}`}>
      <p className={styles.note}>
        {breakdown.count} routed fill{breakdown.count === 1 ? "" : "s"} ·{" "}
        {fmtCompact(breakdown.totalNotional)} gross notional. DV01 shows “—” until the
        rates-book pass is wired (the Deal wire carries no per-fill DV01).
      </p>
      <div className={styles.lenses}>
        <BreakdownTable caption="By tenor" keyHead="Tenor" rows={breakdown.byTenor} />
        <BreakdownTable
          caption="By instrument"
          keyHead="Instrument"
          rows={breakdown.byInstrument}
        />
      </div>
    </div>
  );
}
