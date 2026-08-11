/**
 * Vanilla IRS — the fixed-vs-float interest-rate swap arm of the fixed-income family
 * (fi-bond-ticket-gui, mirroring the OIS fold). A trader builds a two-leg USD swap
 * (fixed leg vs projected floating leg on the single self-discounting SOFR curve) and
 * prices it through the SHARED ticket: the same card that prices FX/cross-asset routes
 * the built {@link RatesInstrument} `irs` arm to `CelnetTransport.priceRates`.
 *
 * It builds the wire {@link VanillaIrsInstrument} (each leg at its own payment
 * frequency + accrual day-count) and the ticket prices it via `priceRates`, returning
 * PV / par (fair fixed) rate / PV01 / DV01 / the key-rate DV01 ladder — the same
 * result shape the OIS renders (the float leg telescopes to `1 − DF(T)` on the single
 * curve, so the swap is the OIS generalised to sub-annual, mixed-day-count legs). One
 * contract, two transports: the offline in-app source (`priceIrsOffline`, which
 * reproduces `celnet_rates::{swap_leg_schedule, VanillaSwap, swap_risk}` bit-for-bit)
 * and the live `price_rates` mirror compute the SAME swap.
 */
import type {
  OisDirection,
  PaymentFrequency,
  RatesInstrument,
  RatesLegDayCount,
} from "../data/contract";
import { pillarYears } from "../data/contract";
import { DEFAULT_USD_SOFR_CURVE } from "../data/ratesPricing";
import styles from "../workspaces/TicketWorkspace.module.css";
import { RatesTabs } from "./ratesControls";
import { defineRatesProduct, type InputBlockProps } from "./types";

import { NumberField } from "../components/NumberField";

/** The vanilla-IRS ticket inputs — direction, tenor, fixed rate, notional, per-leg conventions. */
export interface IrsInputs {
  /** Pay-fixed (payer) or receive-fixed (receiver). */
  direction: OisDirection;
  /** The swap tenor in whole years from spot (`>= 1`). */
  tenorYears: number;
  /** The fixed-leg rate as a percentage (4.05 = 4.05%). */
  fixedRatePct: number;
  /** The notional in millions of the curve currency (`> 0`). */
  notionalMm: number;
  /** Fixed-leg payment frequency. */
  fixedFrequency: PaymentFrequency;
  /** Fixed-leg accrual day-count. */
  fixedDayCount: RatesLegDayCount;
  /** Float-leg payment frequency. */
  floatFrequency: PaymentFrequency;
  /** Float-leg accrual day-count. */
  floatDayCount: RatesLegDayCount;
}

/** The default IRS — a standard 5y receive-fixed at 4.05% on 100mm, semi fixed vs quarterly float. */
export const DEFAULT_IRS: IrsInputs = {
  direction: "RECEIVE_FIXED",
  tenorYears: 5,
  fixedRatePct: 4.05,
  notionalMm: 100,
  fixedFrequency: "SEMI_ANNUAL",
  fixedDayCount: "ACT_360",
  floatFrequency: "QUARTERLY",
  floatDayCount: "ACT_360",
};

/** The stable structure id the ticket + the FI rail entry-point key the family on. */
export const IRS_STRUCTURE_ID = "IRS";

/** The standard quick-pick tenors — the whole-year calibrating pillars of the curve. */
const QUICK_TENORS: readonly number[] = DEFAULT_USD_SOFR_CURVE.pillars
  .map((p) => pillarYears(p.tenor))
  .filter((y): y is number => y !== undefined);

const DIRECTIONS: readonly (readonly [OisDirection, string])[] = [
  ["RECEIVE_FIXED", "Receive fixed"],
  ["PAY_FIXED", "Pay fixed"],
];

const FREQUENCIES: readonly (readonly [PaymentFrequency, string])[] = [
  ["ANNUAL", "Annual"],
  ["SEMI_ANNUAL", "Semi"],
  ["QUARTERLY", "Quarterly"],
];

const LEG_DAY_COUNTS: readonly (readonly [RatesLegDayCount, string])[] = [
  ["ACT_360", "ACT/360"],
  ["ACT_365_FIXED", "ACT/365F"],
];

function IrsInputBlock({ value, onChange }: InputBlockProps<IrsInputs>) {
  return (
    <div className={styles.product}>
      <div className={styles.productRow}>
        <span className={styles.productLabel}>Direction</span>
        <RatesTabs
          label="swap direction"
          value={value.direction}
          options={DIRECTIONS}
          onChange={(direction) => onChange({ ...value, direction })}
        />
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

      <div className={styles.productRow}>
        <span className={styles.productLabel}>Fixed leg</span>
        <RatesTabs
          label="fixed leg frequency"
          value={value.fixedFrequency}
          options={FREQUENCIES}
          onChange={(fixedFrequency) => onChange({ ...value, fixedFrequency })}
        />
        <RatesTabs
          label="fixed leg day count"
          value={value.fixedDayCount}
          options={LEG_DAY_COUNTS}
          onChange={(fixedDayCount) => onChange({ ...value, fixedDayCount })}
        />
      </div>

      <div className={styles.productRow}>
        <span className={styles.productLabel}>Float leg</span>
        <RatesTabs
          label="float leg frequency"
          value={value.floatFrequency}
          options={FREQUENCIES}
          onChange={(floatFrequency) => onChange({ ...value, floatFrequency })}
        />
        <RatesTabs
          label="float leg day count"
          value={value.floatDayCount}
          options={LEG_DAY_COUNTS}
          onChange={(floatDayCount) => onChange({ ...value, floatDayCount })}
        />
      </div>

      <p className={styles.productNote}>
        Vanilla fixed-vs-float interest-rate swap ({DEFAULT_USD_SOFR_CURVE.currency}-SOFR,
        single-curve): a fixed leg vs a projected floating leg, each at its own frequency and
        accrual basis. Priced through the shared ticket on the live `price_rates` seam — PV, par
        (fair fixed) rate, PV01, DV01 and the key-rate DV01 ladder. No option premium / Greeks —
        this is a linear-rates instrument.
      </p>
    </div>
  );
}

/** The vanilla-IRS {@link RatesProductSpec} — the fixed-vs-float swap in the shared ticket. */
export const irsSpec = defineRatesProduct<IrsInputs>({
  id: IRS_STRUCTURE_ID,
  family: "rates",
  label: "IRS (fixed vs float)",
  group: "Fixed income (rates)",
  summary: "Vanilla fixed-vs-float interest-rate swap — PV, par rate, PV01/DV01, key-rate ladder.",
  keywords: [
    "irs",
    "swap",
    "vanilla swap",
    "fixed float",
    "rates",
    "fixed income",
    "fi",
    "pv01",
    "dv01",
    "curve",
    "linear",
  ],
  defaults: DEFAULT_IRS,
  curve: DEFAULT_USD_SOFR_CURVE,
  priceActionLabel: "Price swap",
  emptyHint:
    "Build a fixed-vs-float swap and price it to see the PV, par (fair fixed) rate, PV01, DV01 and the key-rate DV01 ladder.",
  pinToPar: (inputs, parRate) => ({
    ...inputs,
    fixedRatePct: Number((parRate * 100).toFixed(4)),
  }),
  toRatesInstrument: (inputs): RatesInstrument => ({
    kind: "irs",
    irs: {
      tenorYears: inputs.tenorYears,
      fixedRate: inputs.fixedRatePct / 100,
      notional: inputs.notionalMm * 1_000_000,
      direction: inputs.direction,
      fixedFrequency: inputs.fixedFrequency,
      fixedDayCount: inputs.fixedDayCount,
      floatFrequency: inputs.floatFrequency,
      floatDayCount: inputs.floatDayCount,
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
  InputBlock: IrsInputBlock,
});
