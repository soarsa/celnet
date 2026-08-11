/**
 * Variance swap — a volatility-class {@link ProductSpec} (GW2). A single strike
 * in VOL terms (`0` ⇒ price the fair variance strike off the priced reply, log-
 * contract static replication server-side). Extracted verbatim from the former
 * `TicketWorkspace` monolith so the wire output is byte-identical.
 */
import type { Instrument } from "../data/contract";
import { bookingModelsFor, varianceSwapInstrument } from "../data/seed";
import { fmtVol } from "../lib/format";
import styles from "../workspaces/TicketWorkspace.module.css";
import { defineProduct, withTenorAndModel, type InputBlockProps } from "./types";

import { NumberField } from "../components/NumberField";

/** The variance-swap ticket inputs: the strike in VOL terms (`0` ⇒ fair strike). */
export interface VarianceSwapInputs {
  /** Strike in VOL terms; `0` ⇒ request the fair variance strike off the priced reply. */
  swapStrikeVol: number;
}

/** The default variance-swap inputs at first render (fair-strike request). */
export const DEFAULT_VARIANCE_SWAP: VarianceSwapInputs = {
  swapStrikeVol: 0,
};

function VarianceSwapInputBlock({ value, onChange }: InputBlockProps<VarianceSwapInputs>) {
  return (
    <div className={styles.product}>
      <div className={styles.productRow}>
        <span className={styles.productLabel}>Strike vol</span>
        <label className={styles.productField}>
          <NumberField
            className="num"
            min={0}
            step={0.005}
            value={value.swapStrikeVol}
            aria-label="strike vol"
            onChange={(ev) => onChange({ ...value, swapStrikeVol: Math.max(0, Number(ev.target.value)) })}
          />
          <span>vol{value.swapStrikeVol > 0 ? ` = ${fmtVol(value.swapStrikeVol)}` : ""}</span>
        </label>
      </div>
      <p className={styles.productNote}>
        Fair variance strike K_var = strike_vol². Leave 0 to price the fair strike (log-contract
        static replication, server-side).
      </p>
    </div>
  );
}

/** The variance-swap {@link ProductSpec}. */
export const varianceSwapSpec = defineProduct<VarianceSwapInputs>({
  id: "VARIANCE_SWAP",
  label: "Variance Swap",
  group: "Volatility",
  assetClass: "FX",
  summary: "Swap on realised variance — pays the realised variance against a fixed variance strike.",
  keywords: ["variance", "swap", "realised variance", "log contract", "var swap"],
  kind: "varianceSwap",
  defaults: DEFAULT_VARIANCE_SWAP,
  allowedModels: bookingModelsFor("varianceSwap"),
  toInstrument: (inputs: VarianceSwapInputs, ctx): Instrument =>
    withTenorAndModel(
      varianceSwapInstrument(ctx.pair, ctx.tenorYears, ctx.notionalMm, inputs.swapStrikeVol),
      ctx,
    ),
  InputBlock: VarianceSwapInputBlock,
});
