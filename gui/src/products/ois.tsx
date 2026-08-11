/**
 * OIS — the fixed-income (linear rates) product family (fe-fi-migration #3). A
 * trader builds an overnight-indexed swap (USD-SOFR, self-discounting) and prices
 * it through the SHARED ticket: the same card that prices FX/cross-asset now routes
 * a rates instrument to `CelnetTransport.priceRates`, collapsing the standalone FI
 * pricing silo (the former `RatesWorkspace`).
 *
 * This is a {@link RatesProductSpec}, not a {@link ProductSpec}: it builds a wire
 * {@link OisInstrument} (not the option `Instrument`) and the ticket prices it via
 * `priceRates(curve, instrument)` against the calibrated {@link RatesCurveSet},
 * returning PV / par (fair fixed) rate / PV01 / DV01 / the key-rate DV01 ladder —
 * NOT a premium two-way + Greeks. Its `InputBlock` owns the whole rates ticket
 * (direction / tenor / fixed rate / notional); the ticket shell's FX expiry /
 * booking-model / RFQ chrome is suppressed for it.
 *
 * One contract, two transports (GUI-DESIGN §6.2): the SAME builder prices through
 * the deterministic in-app source (the genuine in-browser OIS bootstrap in
 * `src/data/ratesPricing.ts`) and through the live WebSocket `price_rates` mirror,
 * so an offline price agrees with the live edge to floating-point precision.
 */
import type { OisDirection, RatesInstrument } from "../data/contract";
import { pillarYears } from "../data/contract";
import { DEFAULT_USD_SOFR_CURVE } from "../data/ratesPricing";
import styles from "../workspaces/TicketWorkspace.module.css";
import { defineRatesProduct, type InputBlockProps } from "./types";

import { NumberField } from "../components/NumberField";

/** The OIS ticket inputs — direction, whole-year tenor, fixed rate (%), notional (mm). */
export interface OisInputs {
  /** Pay-fixed (payer) or receive-fixed (receiver). */
  direction: OisDirection;
  /** The swap tenor in whole years from spot (`>= 1`). */
  tenorYears: number;
  /** The fixed-leg rate as a percentage (4.05 = 4.05%). */
  fixedRatePct: number;
  /** The notional in millions of the curve currency (`> 0`). */
  notionalMm: number;
}

/** The default OIS inputs — a 5y receive-fixed at 4.05% on 100mm (the P0 USD-SOFR arm). */
export const DEFAULT_OIS: OisInputs = {
  direction: "RECEIVE_FIXED",
  tenorYears: 5,
  fixedRatePct: 4.05,
  notionalMm: 100,
};

/** The stable structure id the ticket + the FI rail entry-point key the family on. */
export const OIS_STRUCTURE_ID = "OIS";

/** The standard quick-pick tenors — the whole-year calibrating pillars of the curve. */
const QUICK_TENORS: readonly number[] = DEFAULT_USD_SOFR_CURVE.pillars
  .map((p) => pillarYears(p.tenor))
  .filter((y): y is number => y !== undefined);

const DIRECTIONS: readonly [OisDirection, string][] = [
  ["RECEIVE_FIXED", "Receive fixed"],
  ["PAY_FIXED", "Pay fixed"],
];

function OisInputBlock({ value, onChange }: InputBlockProps<OisInputs>) {
  return (
    <div className={styles.product}>
      <div className={styles.productRow}>
        <span className={styles.productLabel}>Direction</span>
        <div className={styles.rTabs} role="tablist" aria-label="swap direction">
          {DIRECTIONS.map(([d, label]) => (
            <button
              key={d}
              type="button"
              role="tab"
              aria-selected={value.direction === d}
              className={`${styles.rTab} ${value.direction === d ? styles.rTabActive : ""}`}
              onClick={() => onChange({ ...value, direction: d })}
            >
              {label}
            </button>
          ))}
        </div>
      </div>

      <div className={styles.productRow}>
        <span className={styles.productLabel}>Tenor</span>
        <div className={styles.rPills}>
          {QUICK_TENORS.map((t) => (
            <button
              key={t}
              type="button"
              aria-pressed={value.tenorYears === t}
              className={`${styles.rPill} ${value.tenorYears === t ? styles.rPillActive : ""}`}
              onClick={() => onChange({ ...value, tenorYears: t })}
            >
              {t}y
            </button>
          ))}
          <label className={styles.productField}>
            <span>yrs</span>
            <NumberField
              className="num"
              min={1}
              step={1}
              value={value.tenorYears}
              aria-label="swap tenor in years"
              onChange={(e) => onChange({ ...value, tenorYears: Math.trunc(Number(e.target.value)) })}
            />
          </label>
        </div>
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

      <p className={styles.productNote}>
        Overnight-indexed swap ({DEFAULT_USD_SOFR_CURVE.currency}-SOFR, self-discounting):
        fixed vs compounded overnight floating on one curve. Priced through the shared ticket
        on the live `price_rates` seam — PV, par (fair fixed) rate, PV01, DV01 and the key-rate
        DV01 ladder. No option premium / Greeks — this is a linear-rates instrument.
      </p>
    </div>
  );
}

/** The OIS {@link RatesProductSpec} — the fixed-income family in the shared ticket. */
export const oisSpec = defineRatesProduct<OisInputs>({
  id: OIS_STRUCTURE_ID,
  family: "rates",
  label: "OIS (SOFR swap)",
  group: "Fixed income (rates)",
  summary: "Overnight-indexed swap vs USD-SOFR — PV, par rate, PV01/DV01, key-rate ladder.",
  keywords: ["ois", "swap", "rates", "sofr", "fixed income", "fi", "pv01", "dv01", "curve", "linear"],
  defaults: DEFAULT_OIS,
  curve: DEFAULT_USD_SOFR_CURVE,
  priceActionLabel: "Price OIS",
  emptyHint:
    "Build an OIS and price it to see the PV, par (fair fixed) rate, PV01, DV01 and the key-rate DV01 ladder.",
  pinToPar: (inputs, parRate) => ({
    ...inputs,
    fixedRatePct: Number((parRate * 100).toFixed(4)),
  }),
  toRatesInstrument: (inputs): RatesInstrument => ({
    kind: "ois",
    ois: {
      tenorYears: inputs.tenorYears,
      fixedRate: inputs.fixedRatePct / 100,
      notional: inputs.notionalMm * 1_000_000,
      direction: inputs.direction,
    },
  }),
  validate: (inputs) => {
    const violations: string[] = [];
    if (!Number.isInteger(inputs.tenorYears) || inputs.tenorYears < 1) {
      violations.push("Tenor must be a whole number of years ≥ 1.");
    }
    if (!(inputs.notionalMm > 0)) {
      violations.push("Notional must be greater than 0.");
    }
    return violations;
  },
  InputBlock: OisInputBlock,
});
