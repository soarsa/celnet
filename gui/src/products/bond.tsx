/**
 * Bond — the fixed-coupon cash-bond arm of the fixed-income family (fi-bond-ticket-gui).
 * A trader builds a cash bond (coupon, frequency, day-count, maturity, redemption) and
 * prices it OFF the calibrated SOFR curve through the SHARED ticket: the built
 * {@link RatesInstrument} `bond` arm routes to `CelnetTransport.priceRates`.
 *
 * It builds the wire {@link BondInstrument} and the ticket prices it via `priceRates`,
 * discounting each cashflow off the bootstrapped curve for the dirty price and solving
 * the implied yield-to-maturity + analytic yield DV01. The wire {@link
 * RatesPricingResult} carries only `pv = dirty price`, `parRate = yield to maturity`,
 * `pv01 = dv01 = the yield DV01`, and an EMPTY ladder — the bond's wrapped risk is a
 * closed-form yield-space sensitivity with no per-pillar decomposition. The result view
 * therefore relabels PV → "Dirty PV" and the par metric → "Yield to maturity", and
 * hides the PV01 row + the ladder (only fields the wire carries are shown — no
 * fabricated clean price / duration, which are NOT on the contract). One contract, two
 * transports: the offline `priceBondOffline` (reproducing `celnet_bond::{price_from_curve,
 * bond_risk}`) and the live `price_rates` mirror compute the SAME bond.
 */
import type {
  BondPosition,
  BrokenDate,
  PaymentFrequency,
  RatesAccrualBasis,
  RatesInstrument,
} from "../data/contract";
import { DEFAULT_USD_SOFR_CURVE } from "../data/ratesPricing";
import styles from "../workspaces/TicketWorkspace.module.css";
import { RatesTabs } from "./ratesControls";
import { defineRatesProduct, type InputBlockProps, type RatesResultView } from "./types";

/** The cash-bond ticket inputs — position, coupon, frequency, day-count, maturity, redemption. */
export interface BondInputs {
  /** Long (bought, +PV) or short (sold, −PV). */
  position: BondPosition;
  /** The annual coupon rate as a percentage (4.0 = 4%); 0 for a zero-coupon bond. */
  couponPct: number;
  /** The coupon payment frequency (also the yield compounding basis). */
  couponFrequency: PaymentFrequency;
  /** The accrual day-count basis for accrued interest. */
  dayCount: RatesAccrualBasis;
  /** The maturity (final-redemption) date; must be strictly after settlement. */
  maturityDate: BrokenDate;
  /** The par redemption / face value (`> 0`). */
  redemption: number;
}

/** The curve reference (spot-anchor) date the bond settles on / validates its maturity against. */
const REFERENCE_DATE = DEFAULT_USD_SOFR_CURVE.referenceDate;

/** The default bond — a long 4% semi-annual 30/360 bond maturing in ~5y, redemption 100. */
export const DEFAULT_BOND: BondInputs = {
  position: "LONG",
  couponPct: 4.0,
  couponFrequency: "SEMI_ANNUAL",
  dayCount: "THIRTY_360_BOND_BASIS",
  maturityDate: { year: REFERENCE_DATE.year + 5, month: REFERENCE_DATE.month, day: REFERENCE_DATE.day },
  redemption: 100,
};

/** The stable structure id the ticket + the FI rail entry-point key the family on. */
export const BOND_STRUCTURE_ID = "BOND";

/** The bond result view — dirty PV, yield to maturity, DV01 (no PV01 row, no ladder). */
const BOND_RESULT_VIEW: RatesResultView = {
  pvLabel: "Dirty PV",
  pvHasCurrencyUnit: false,
  parLabel: "Yield to maturity",
  showPv01: false,
  showLadder: false,
};

const POSITIONS: readonly (readonly [BondPosition, string])[] = [
  ["LONG", "Long (buy)"],
  ["SHORT", "Short (sell)"],
];

const FREQUENCIES: readonly (readonly [PaymentFrequency, string])[] = [
  ["ANNUAL", "Annual"],
  ["SEMI_ANNUAL", "Semi"],
  ["QUARTERLY", "Quarterly"],
];

const DAY_COUNTS: readonly (readonly [RatesAccrualBasis, string])[] = [
  ["THIRTY_360_BOND_BASIS", "30/360"],
  ["ACT_360", "ACT/360"],
  ["ACT_365_FIXED", "ACT/365F"],
];

/** Format a {@link BrokenDate} as the `YYYY-MM-DD` a native date input reads. */
function toIsoDate(date: BrokenDate): string {
  const mm = String(date.month).padStart(2, "0");
  const dd = String(date.day).padStart(2, "0");
  return `${date.year}-${mm}-${dd}`;
}

/** Parse a `YYYY-MM-DD` value to a {@link BrokenDate}, or `null` when malformed. */
function fromIsoDate(value: string): BrokenDate | null {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value);
  if (!m) return null;
  const year = Number(m[1]);
  const month = Number(m[2]);
  const day = Number(m[3]);
  if (month < 1 || month > 12 || day < 1 || day > 31) return null;
  return { year, month, day };
}

/** A YYYYMMDD comparison key — strict-ordering of civil dates without epoch arithmetic. */
function dateKey(date: BrokenDate): number {
  return date.year * 10000 + date.month * 100 + date.day;
}

function BondInputBlock({ value, onChange }: InputBlockProps<BondInputs>) {
  return (
    <div className={styles.product}>
      <div className={styles.productRow}>
        <span className={styles.productLabel}>Position</span>
        <RatesTabs
          label="bond position"
          value={value.position}
          options={POSITIONS}
          onChange={(position) => onChange({ ...value, position })}
        />
      </div>

      <div className={styles.productRow}>
        <label className={styles.productField}>
          <span>Coupon</span>
          <input
            className="num"
            type="number"
            step={0.01}
            value={value.couponPct}
            aria-label="annual coupon rate in percent"
            onChange={(e) => onChange({ ...value, couponPct: Number(e.target.value) })}
          />
          <span>%</span>
        </label>
        <label className={styles.productField}>
          <span>Redemption</span>
          <input
            className="num"
            type="number"
            min={0}
            step={1}
            value={value.redemption}
            aria-label="redemption face value"
            onChange={(e) => onChange({ ...value, redemption: Number(e.target.value) })}
          />
        </label>
      </div>

      <div className={styles.productRow}>
        <label className={styles.productField}>
          <span>Maturity</span>
          <input
            type="date"
            value={toIsoDate(value.maturityDate)}
            aria-label="bond maturity date"
            onChange={(e) => {
              const parsed = fromIsoDate(e.target.value);
              if (parsed) onChange({ ...value, maturityDate: parsed });
            }}
          />
        </label>
      </div>

      <div className={styles.productRow}>
        <span className={styles.productLabel}>Coupon freq</span>
        <RatesTabs
          label="coupon frequency"
          value={value.couponFrequency}
          options={FREQUENCIES}
          onChange={(couponFrequency) => onChange({ ...value, couponFrequency })}
        />
      </div>

      <div className={styles.productRow}>
        <span className={styles.productLabel}>Day count</span>
        <RatesTabs
          label="accrual day-count basis"
          value={value.dayCount}
          options={DAY_COUNTS}
          onChange={(dayCount) => onChange({ ...value, dayCount })}
        />
      </div>

      <p className={styles.productNote}>
        Fixed-coupon cash bond priced off the {DEFAULT_USD_SOFR_CURVE.currency}-SOFR curve: each
        cashflow discounted for the dirty price, with the implied yield-to-maturity and analytic
        yield DV01. Settles on the curve spot date. Priced through the shared ticket on the live
        `price_rates` seam — dirty PV, yield to maturity and DV01. No option premium / Greeks — this
        is a linear-rates instrument.
      </p>
    </div>
  );
}

/** The cash-bond {@link RatesProductSpec} — the fixed-coupon bond in the shared ticket. */
export const bondSpec = defineRatesProduct<BondInputs>({
  id: BOND_STRUCTURE_ID,
  family: "rates",
  label: "Bond (cash)",
  group: "Fixed income (rates)",
  summary: "Fixed-coupon cash bond off the curve — dirty PV, yield to maturity, DV01.",
  keywords: [
    "bond",
    "cash bond",
    "fixed coupon",
    "ytm",
    "yield",
    "rates",
    "fixed income",
    "fi",
    "dv01",
    "curve",
    "linear",
  ],
  defaults: DEFAULT_BOND,
  curve: DEFAULT_USD_SOFR_CURVE,
  resultView: BOND_RESULT_VIEW,
  priceActionLabel: "Price bond",
  emptyHint:
    "Build a cash bond and price it to see the dirty PV, yield to maturity and DV01 off the curve.",
  toRatesInstrument: (inputs): RatesInstrument => ({
    kind: "bond",
    bond: {
      couponRate: inputs.couponPct / 100,
      couponFrequency: inputs.couponFrequency,
      dayCount: inputs.dayCount,
      maturityDate: inputs.maturityDate,
      redemption: inputs.redemption,
      position: inputs.position,
    },
  }),
  validate: (inputs) => {
    const violations: string[] = [];
    if (!(inputs.redemption > 0)) {
      violations.push("Redemption must be greater than 0.");
    }
    if (dateKey(inputs.maturityDate) <= dateKey(REFERENCE_DATE)) {
      violations.push("Maturity must be after the curve spot date.");
    }
    if (!Number.isFinite(inputs.couponPct)) {
      violations.push("Coupon rate must be a finite number.");
    }
    return violations;
  },
  InputBlock: BondInputBlock,
});
