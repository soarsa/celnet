/**
 * FX swap — the second W2 linear product (the GUI end of the `celnet-linear`
 * leaf). A near leg + a far leg, each an outright forward trading OPPOSITE
 * directions; the swap PV is the independent sum of the two leg PVs. The near leg
 * settles at the spot date (`t = 0`) and the far leg at the instrument's
 * expiry/tenor (the single ticket tenor anchors the far leg). Linear, closed-form
 * — no option type / strike / vol. Follows the {@link asianSpec} template.
 */
import type { Instrument, Side } from "../data/contract";
import { oppositeSide, swapInstrument, type SwapTerms } from "../data/seed";
import { fmtRate } from "../lib/format";
import styles from "../workspaces/TicketWorkspace.module.css";
import { defineProduct, withTenorAndModel, type InputBlockProps } from "./types";

import { NumberField } from "../components/NumberField";

/** The FX-swap ticket inputs: the two leg rates + the near leg's direction. */
export interface SwapInputs {
  /** The near (spot-dated) leg's rate; `0` ⇒ default to the spot when shown. */
  nearRate: number;
  /** The far (tenor-dated) leg's rate; `0` ⇒ default to the ATM-forward when shown. */
  farRate: number;
  /** The near leg's direction; the far leg takes the opposite side by convention. */
  nearSide: Side;
}

/** The default swap inputs (a buy-near/sell-far at spot / ATM-forward). */
export const DEFAULT_SWAP: SwapInputs = {
  nearRate: 0,
  farRate: 0,
  nearSide: "BUY",
};

/**
 * Build the `SwapTerms` from the ticket inputs, defaulting a `0` near rate to the
 * spot (the near leg's at-market rate) and a `0` far rate to the ATM-forward (the
 * far leg's at-market rate), mirroring the asian strike-ATMF pattern.
 */
export function swapTerms(s: SwapInputs, spot: number, atmForward: number): SwapTerms {
  return {
    nearRate: s.nearRate > 0 ? s.nearRate : spot,
    farRate: s.farRate > 0 ? s.farRate : atmForward,
    nearSide: s.nearSide,
  };
}

function SwapInputBlock({ value, onChange, ctx }: InputBlockProps<SwapInputs>) {
  const { atmForward, spot, pipDecimals } = ctx;
  const farSide = oppositeSide(value.nearSide);
  return (
    <div className={styles.product}>
      <div className={styles.productRow}>
        <span className={styles.productLabel}>Near side</span>
        <div className={styles.toggleGroup} role="tablist" aria-label="near direction">
          {(["BUY", "SELL"] as Side[]).map((sd) => (
            <button
              key={sd}
              role="tab"
              aria-selected={value.nearSide === sd}
              className={`${styles.modeTab} ${value.nearSide === sd ? styles.modeActive : ""}`}
              onClick={() => onChange({ ...value, nearSide: sd })}
            >
              {sd === "BUY" ? "Buy" : "Sell"}
            </button>
          ))}
        </div>
        <label className={styles.productField}>
          <span>Near rate</span>
          <NumberField
            className="num"
            min={0}
            step={Math.pow(10, -pipDecimals)}
            value={value.nearRate}
            aria-label="near rate"
            placeholder={spot.toFixed(pipDecimals)}
            onChange={(ev) =>
              onChange({ ...value, nearRate: Math.max(0, Number(ev.target.value)) })
            }
          />
          <span>{value.nearRate > 0 ? "" : `spot ${fmtRate(spot, pipDecimals)}`}</span>
        </label>
      </div>
      <div className={styles.productRow}>
        <span className={styles.productLabel}>Far side</span>
        <span className={styles.productLabel}>{farSide === "BUY" ? "Buy" : "Sell"}</span>
        <label className={styles.productField}>
          <span>Far rate</span>
          <NumberField
            className="num"
            min={0}
            step={Math.pow(10, -pipDecimals)}
            value={value.farRate}
            aria-label="far rate"
            placeholder={atmForward.toFixed(pipDecimals)}
            onChange={(ev) =>
              onChange({ ...value, farRate: Math.max(0, Number(ev.target.value)) })
            }
          />
          <span>{value.farRate > 0 ? "" : `ATMF ${fmtRate(atmForward, pipDecimals)}`}</span>
        </label>
      </div>
      <p className={styles.productNote}>
        FX swap — a near leg (spot date, `t = 0`) plus a far leg (the ticket tenor), two
        outright forwards trading OPPOSITE directions (far side is derived). The PV is the
        exact sum of the two leg PVs; closed-form (no Monte-Carlo standard error).
        Deliverable underlying only.
      </p>
    </div>
  );
}

/** The FX-swap {@link ProductSpec}. */
export const swapSpec = defineProduct<SwapInputs>({
  id: "FX_SWAP",
  label: "Swap (near/far)",
  group: "Linear (forwards & swaps)",
  assetClass: "FX",
  summary: "Near + far outright legs (opposite directions) — the sum of two forward PVs.",
  keywords: ["swap", "fx swap", "near", "far", "linear", "roll", "deliverable"],
  kind: "fxSwap",
  defaults: DEFAULT_SWAP,
  allowedModels: ["DEFAULT"],
  toInstrument: (inputs: SwapInputs, ctx): Instrument =>
    withTenorAndModel(
      swapInstrument(
        ctx.pair,
        ctx.tenorYears,
        ctx.notionalMm,
        swapTerms(inputs, ctx.spot, ctx.atmForward),
      ),
      ctx,
    ),
  InputBlock: SwapInputBlock,
});
