/**
 * FRA — the forward-rate-agreement arm of the fixed-income family (fi-bond-ticket-gui).
 * A trader builds a single accrual window (a `3×6` FRA fixes at 3m, matures at 6m) on
 * the projected float index and prices it through the SHARED ticket: the built
 * {@link RatesInstrument} `fra` arm routes to `CelnetTransport.priceRates`.
 *
 * It builds the wire {@link FraInstrument} (the window `[startMonths, endMonths]` +
 * the accrual basis for τ) and the ticket prices it via `priceRates`, returning PV /
 * par (break-even forward) rate / PV01 / DV01 / the key-rate DV01 ladder. A FRA is
 * exactly a one-period OIS swaplet on the single self-discounting curve. One contract,
 * two transports: the offline in-app source (`priceFraOffline`, reproducing
 * `celnet_rates::{Fra::from_dates, fra_risk}`) and the live `price_rates` mirror
 * compute the SAME FRA.
 */
import type { OisDirection, RatesAccrualBasis, RatesInstrument } from "../data/contract";
import { DEFAULT_USD_SOFR_CURVE } from "../data/ratesPricing";
import styles from "../workspaces/TicketWorkspace.module.css";
import { RatesTabs } from "./ratesControls";
import { defineRatesProduct, type InputBlockProps } from "./types";

import { NumberField } from "../components/NumberField";

/** The FRA ticket inputs — direction, the `[startMonths, endMonths]` window, rate, notional, basis. */
export interface FraInputs {
  /** Pay-fixed (payer) or receive-fixed (receiver). */
  direction: OisDirection;
  /** The window start (fixing) tenor in months from spot (`>= 1`, `< endMonths`). */
  startMonths: number;
  /** The window end (maturity) tenor in months from spot (`> startMonths`). */
  endMonths: number;
  /** The contractual fixed rate as a percentage (3.30 = 3.30%). */
  fixedRatePct: number;
  /** The notional in millions of the curve currency (`> 0`). */
  notionalMm: number;
  /** The accrual day-count basis for τ. */
  accrualBasis: RatesAccrualBasis;
}

/** The default FRA — a standard 3×6 receive-fixed at 4.30% on 100mm, ACT/360 accrual. */
export const DEFAULT_FRA: FraInputs = {
  direction: "RECEIVE_FIXED",
  startMonths: 3,
  endMonths: 6,
  fixedRatePct: 4.3,
  notionalMm: 100,
  accrualBasis: "ACT_360",
};

/** The stable structure id the ticket + the FI rail entry-point key the family on. */
export const FRA_STRUCTURE_ID = "FRA";

/** Standard FRA windows `[start, end]` in months (the liquid quick-picks). */
const QUICK_WINDOWS: readonly (readonly [number, number])[] = [
  [1, 4],
  [3, 6],
  [6, 9],
  [6, 12],
  [9, 12],
  [12, 18],
];

const DIRECTIONS: readonly (readonly [OisDirection, string])[] = [
  ["RECEIVE_FIXED", "Receive fixed"],
  ["PAY_FIXED", "Pay fixed"],
];

const ACCRUAL_BASES: readonly (readonly [RatesAccrualBasis, string])[] = [
  ["ACT_360", "ACT/360"],
  ["ACT_365_FIXED", "ACT/365F"],
  ["THIRTY_360_BOND_BASIS", "30/360"],
];

/** The `s×e` label a FRA window quotes as (months). */
function windowLabel(start: number, end: number): string {
  return `${start}×${end}`;
}

function FraInputBlock({ value, onChange }: InputBlockProps<FraInputs>) {
  return (
    <div className={styles.product}>
      <div className={styles.productRow}>
        <span className={styles.productLabel}>Direction</span>
        <RatesTabs
          label="FRA direction"
          value={value.direction}
          options={DIRECTIONS}
          onChange={(direction) => onChange({ ...value, direction })}
        />
      </div>

      <div className={styles.productRow}>
        <span className={styles.productLabel}>Window</span>
        <div className={styles.rPills}>
          {QUICK_WINDOWS.map(([s, e]) => {
            const active = value.startMonths === s && value.endMonths === e;
            return (
              <button
                key={windowLabel(s, e)}
                type="button"
                aria-pressed={active}
                className={`${styles.rPill} ${active ? styles.rPillActive : ""}`}
                onClick={() => onChange({ ...value, startMonths: s, endMonths: e })}
              >
                {windowLabel(s, e)}
              </button>
            );
          })}
        </div>
      </div>

      <div className={styles.productRow}>
        <label className={styles.productField}>
          <span>Start</span>
          <NumberField
            className="num"
            min={1}
            step={1}
            value={value.startMonths}
            aria-label="window start in months"
            onChange={(e) => onChange({ ...value, startMonths: Math.trunc(Number(e.target.value)) })}
          />
          <span>m</span>
        </label>
        <label className={styles.productField}>
          <span>End</span>
          <NumberField
            className="num"
            min={1}
            step={1}
            value={value.endMonths}
            aria-label="window end in months"
            onChange={(e) => onChange({ ...value, endMonths: Math.trunc(Number(e.target.value)) })}
          />
          <span>m</span>
        </label>
      </div>

      <div className={styles.productRow}>
        <label className={styles.productField}>
          <span>Fixed rate</span>
          <NumberField
            className="num"
            step={0.01}
            value={value.fixedRatePct}
            aria-label="fixed rate in percent"
            onChange={(e) => onChange({ ...value, fixedRatePct: Number(e.target.value) })}
          />
          <span>%</span>
        </label>
        <label className={styles.productField}>
          <span>Notional</span>
          <NumberField
            className="num"
            min={0}
            step={5}
            value={value.notionalMm}
            aria-label="notional in millions"
            onChange={(e) => onChange({ ...value, notionalMm: Number(e.target.value) })}
          />
          <span>mm {DEFAULT_USD_SOFR_CURVE.currency}</span>
        </label>
      </div>

      <div className={styles.productRow}>
        <span className={styles.productLabel}>Accrual</span>
        <RatesTabs
          label="accrual day-count basis"
          value={value.accrualBasis}
          options={ACCRUAL_BASES}
          onChange={(accrualBasis) => onChange({ ...value, accrualBasis })}
        />
      </div>

      <p className={styles.productNote}>
        Forward rate agreement ({DEFAULT_USD_SOFR_CURVE.currency}-SOFR, single-curve): one accrual
        window on the projected forward — a one-period swaplet. Priced through the shared ticket on
        the live `price_rates` seam — PV, par (break-even forward) rate, PV01, DV01 and the key-rate
        DV01 ladder. No option premium / Greeks — this is a linear-rates instrument.
      </p>
    </div>
  );
}

/** The FRA {@link RatesProductSpec} — the single-window forward rate agreement in the shared ticket. */
export const fraSpec = defineRatesProduct<FraInputs>({
  id: FRA_STRUCTURE_ID,
  family: "rates",
  label: "FRA (forward rate)",
  group: "Fixed income (rates)",
  summary: "Forward rate agreement — one accrual window; PV, par rate, PV01/DV01, key-rate ladder.",
  keywords: [
    "fra",
    "forward rate agreement",
    "swaplet",
    "rates",
    "fixed income",
    "fi",
    "pv01",
    "dv01",
    "curve",
    "linear",
  ],
  defaults: DEFAULT_FRA,
  curve: DEFAULT_USD_SOFR_CURVE,
  priceActionLabel: "Price FRA",
  emptyHint:
    "Build a FRA window and price it to see the PV, par (break-even forward) rate, PV01, DV01 and the key-rate DV01 ladder.",
  pinToPar: (inputs, parRate) => ({
    ...inputs,
    fixedRatePct: Number((parRate * 100).toFixed(4)),
  }),
  toRatesInstrument: (inputs): RatesInstrument => ({
    kind: "fra",
    fra: {
      startMonths: inputs.startMonths,
      endMonths: inputs.endMonths,
      fixedRate: inputs.fixedRatePct / 100,
      notional: inputs.notionalMm * 1_000_000,
      direction: inputs.direction,
      accrualBasis: inputs.accrualBasis,
    },
  }),
  validate: (inputs) => {
    const violations: string[] = [];
    if (!Number.isInteger(inputs.startMonths) || inputs.startMonths < 1) {
      violations.push("Start must be a whole number of months ≥ 1.");
    }
    if (!Number.isInteger(inputs.endMonths) || inputs.endMonths <= inputs.startMonths) {
      violations.push("End must be a whole number of months after the start.");
    }
    if (!(inputs.notionalMm > 0)) {
      violations.push("Notional must be greater than 0.");
    }
    return violations;
  },
  InputBlock: FraInputBlock,
});
