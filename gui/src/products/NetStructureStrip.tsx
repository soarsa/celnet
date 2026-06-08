/**
 * NetStructureStrip — a compact running summary of a multi-leg structure's net
 * economics, shown to the trader WHILE structuring (GW2). It aggregates the net
 * premium and the net first/second-order Greeks (Δ ν Γ) across the legs, each
 * leg contributing `sign(side) · ratio · value`.
 *
 * Honest-data discipline (the GW0 absence contract): a net measure is only shown
 * when EVERY leg carries that measure. If any leg is missing it (e.g. the leg has
 * not priced yet), summing the present legs would understate the net by the
 * absent leg's contribution — a *wrong* number a trader could lean on — so the
 * whole net renders "—" via {@link EmptyValue} with a small note, never a partial
 * sum and never a silent `0`. Styling follows the GreeksStrip cell conventions.
 */
import { EmptyValue } from "../components/EmptyValue";
import { fmtPremiumPct, fmtSigned } from "../lib/format";
import styles from "./NetStructureStrip.module.css";

/** One structuring leg's economics, as the strip needs them. */
export interface NetStructureLeg {
  /** Premium as a fraction of notional (PERCENT_FOREIGN style); absent ⇒ net premium is "—". */
  premium?: number;
  /** Spot delta of the leg; absent ⇒ net delta is "—". */
  delta?: number;
  /** Vega of the leg; absent ⇒ net vega is "—". */
  vega?: number;
  /** Gamma of the leg; absent ⇒ net gamma is "—". */
  gamma?: number;
  /** The leg ratio (e.g. 1, 1.5, 2) scaling its contribution. */
  ratio: number;
  /** The side the trader takes on this leg, which sets the contribution sign. */
  side: "BUY" | "SELL";
}

/** The directional sign a leg contributes by its side: long buys, short sells. */
function sideSign(side: NetStructureLeg["side"]): 1 | -1 {
  return side === "BUY" ? 1 : -1;
}

/**
 * Sum a single measure across the legs, honestly. Returns `null` (⇒ render "—")
 * the moment any leg is missing the measure, so the strip never shows a partial
 * net that silently drops an absent leg's contribution.
 */
function netOf(
  legs: readonly NetStructureLeg[],
  pick: (leg: NetStructureLeg) => number | undefined,
): number | null {
  let total = 0;
  for (const leg of legs) {
    const v = pick(leg);
    if (v === undefined) return null;
    total += sideSign(leg.side) * leg.ratio * v;
  }
  return total;
}

interface NetRow {
  /** The dt label / glyph shown in the strip. */
  label: string;
  /** Accessible description of the measure. */
  describe: string;
  /** The net value, or `null` when any leg lacks it. */
  value: number | null;
  /** Render the present value (units differ per measure). */
  render: (v: number) => string;
}

export function NetStructureStrip({
  legs,
  quoteCcy,
  baseCcy,
}: {
  legs: readonly NetStructureLeg[];
  quoteCcy: string;
  baseCcy: string;
}): React.ReactElement {
  const netPremium = netOf(legs, (l) => l.premium);
  const netDelta = netOf(legs, (l) => l.delta);
  const netVega = netOf(legs, (l) => l.vega);
  const netGamma = netOf(legs, (l) => l.gamma);

  const incomplete =
    netPremium === null || netDelta === null || netVega === null || netGamma === null;

  const rows: NetRow[] = [
    {
      label: "Premium",
      describe: `net premium, ${quoteCcy} per ${baseCcy} notional`,
      value: netPremium,
      render: (v) => `${fmtPremiumPct(v)} %`,
    },
    { label: "Δ", describe: "net delta", value: netDelta, render: (v) => fmtSigned(v, 3) },
    { label: "ν", describe: "net vega", value: netVega, render: (v) => fmtSigned(v, 4) },
    { label: "Γ", describe: "net gamma", value: netGamma, render: (v) => fmtSigned(v, 4) },
  ];

  return (
    <section
      className={styles.strip}
      role="group"
      aria-label={`net structure economics (${baseCcy}/${quoteCcy})`}
    >
      <dl className={styles.row}>
        {rows.map((r) => (
          <div key={r.label} className={styles.cell}>
            <dt className={styles.glyph} title={r.describe}>
              {r.label}
            </dt>
            <dd className={styles.value}>
              {r.value === null ? (
                <EmptyValue reason={`${r.describe}: a leg is missing this value`} />
              ) : (
                <span className="num">{r.render(r.value)}</span>
              )}
            </dd>
          </div>
        ))}
      </dl>
      {incomplete && (
        <p className={styles.note} role="note">
          Net is shown only when every leg carries the measure — a missing leg value reads "—" rather
          than an understated sum.
        </p>
      )}
    </section>
  );
}
