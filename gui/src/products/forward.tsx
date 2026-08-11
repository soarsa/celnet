/**
 * Outright FX forward — the first of the three W2 linear products surfaced as
 * {@link ProductSpec}s (the GUI end of the `celnet-linear` leaf). Unlike every
 * option family this is a LINEAR, closed-form discounted cashflow (no option
 * type, no strike-or-delta, no vol): a contract rate, a notional and a direction.
 * Follows the {@link asianSpec} template — an `Inputs` shape, the `*Terms`
 * transform, defaults, a `toInstrument` delegating to the `data/seed` wire
 * builder, and a self-contained `InputBlock`.
 */
import type { Instrument, Side } from "../data/contract";
import { forwardInstrument, type ForwardTerms } from "../data/seed";
import { fmtRate } from "../lib/format";
import styles from "../workspaces/TicketWorkspace.module.css";
import { defineProduct, withTenorAndModel, type InputBlockProps } from "./types";

import { NumberField } from "../components/NumberField";

/** The outright-forward ticket inputs. */
export interface ForwardInputs {
  /** Contract (delivery) rate; `0` ⇒ default to the ATM-forward when shown. */
  contractRate: number;
  /** The direction taken (`BUY` = long the base/asset forward; `SELL` = short). */
  side: Side;
}

/** The default forward inputs at first render (a BUY struck at the ATM-forward). */
export const DEFAULT_FORWARD: ForwardInputs = {
  contractRate: 0,
  side: "BUY",
};

/**
 * Build the `ForwardTerms` from the ticket inputs, defaulting a `0` contract rate
 * to the ATM-forward (the at-market rate ⇒ a zero-PV trade), mirroring the asian
 * strike-ATMF pattern.
 */
export function forwardTerms(f: ForwardInputs, atmForward: number): ForwardTerms {
  return {
    contractRate: f.contractRate > 0 ? f.contractRate : atmForward,
    side: f.side,
  };
}

function ForwardInputBlock({ value, onChange, ctx }: InputBlockProps<ForwardInputs>) {
  const { atmForward, pipDecimals } = ctx;
  return (
    <div className={styles.product}>
      <div className={styles.productRow}>
        <span className={styles.productLabel}>Direction</span>
        <div className={styles.toggleGroup} role="tablist" aria-label="direction">
          {(["BUY", "SELL"] as Side[]).map((sd) => (
            <button
              key={sd}
              role="tab"
              aria-selected={value.side === sd}
              className={`${styles.modeTab} ${value.side === sd ? styles.modeActive : ""}`}
              onClick={() => onChange({ ...value, side: sd })}
            >
              {sd === "BUY" ? "Buy" : "Sell"}
            </button>
          ))}
        </div>
        <label className={styles.productField}>
          <span>Rate</span>
          <NumberField
            className="num"
            min={0}
            step={Math.pow(10, -pipDecimals)}
            value={value.contractRate}
            aria-label="contract rate"
            placeholder={atmForward.toFixed(pipDecimals)}
            onChange={(ev) =>
              onChange({ ...value, contractRate: Math.max(0, Number(ev.target.value)) })
            }
          />
          <span>{value.contractRate > 0 ? "" : `ATMF ${fmtRate(atmForward, pipDecimals)}`}</span>
        </label>
      </div>
      <p className={styles.productNote}>
        Outright forward — a linear discounted cashflow `side · notional · df(t) · (F −
        K)`, NOT an option. Priced in exact closed form (no Monte-Carlo standard error).
        Deliverable underlying only; an at-market rate (the ATM-forward) books a zero-PV trade.
      </p>
    </div>
  );
}

/** The outright-forward {@link ProductSpec}. */
export const forwardSpec = defineProduct<ForwardInputs>({
  id: "FX_FORWARD",
  label: "Forward (outright)",
  group: "Linear (forwards & swaps)",
  assetClass: "FX",
  summary: "Outright deliverable forward — linear discounted cashflow at a contract rate.",
  keywords: ["forward", "outright", "fwd", "linear", "dcf", "deliverable"],
  kind: "fxForward",
  defaults: DEFAULT_FORWARD,
  allowedModels: ["DEFAULT"],
  toInstrument: (inputs: ForwardInputs, ctx): Instrument =>
    withTenorAndModel(
      forwardInstrument(ctx.pair, ctx.tenorYears, ctx.notionalMm, forwardTerms(inputs, ctx.atmForward)),
      ctx,
    ),
  InputBlock: ForwardInputBlock,
});
