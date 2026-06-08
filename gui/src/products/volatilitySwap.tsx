/**
 * Volatility swap — a volatility-class {@link ProductSpec} (GW2). A single strike
 * in VOL terms (`0` ⇒ price the fair, convexity-adjusted volatility strike server-
 * side). Extracted verbatim from the former `TicketWorkspace` monolith so the wire
 * output is byte-identical.
 */
import type { Instrument } from "../data/contract";
import { bookingModelsFor, volatilitySwapInstrument } from "../data/seed";
import { fmtVol } from "../lib/format";
import styles from "../workspaces/TicketWorkspace.module.css";
import { defineProduct, withTenorAndModel, type InputBlockProps } from "./types";

/** The volatility-swap ticket inputs: the strike in VOL terms (`0` ⇒ fair strike). */
export interface VolatilitySwapInputs {
  /** Strike in VOL terms; `0` ⇒ request the fair volatility strike off the priced reply. */
  swapStrikeVol: number;
}

/** The default volatility-swap inputs at first render (fair-strike request). */
export const DEFAULT_VOLATILITY_SWAP: VolatilitySwapInputs = {
  swapStrikeVol: 0,
};

function VolatilitySwapInputBlock({ value, onChange }: InputBlockProps<VolatilitySwapInputs>) {
  return (
    <div className={styles.product}>
      <div className={styles.productRow}>
        <span className={styles.productLabel}>Strike vol</span>
        <label className={styles.productField}>
          <input
            className="num"
            type="number"
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
        Fair volatility strike K_vol (convexity-adjusted). Leave 0 to price the fair strike
        server-side.
      </p>
    </div>
  );
}

/** The volatility-swap {@link ProductSpec}. */
export const volatilitySwapSpec = defineProduct<VolatilitySwapInputs>({
  id: "VOLATILITY_SWAP",
  label: "Volatility Swap",
  group: "Volatility",
  assetClass: "FX",
  summary: "Swap on realised volatility — pays realised vol against a fixed (convexity-adjusted) vol strike.",
  keywords: ["volatility", "swap", "realised vol", "convexity", "vol swap"],
  kind: "volatilitySwap",
  defaults: DEFAULT_VOLATILITY_SWAP,
  allowedModels: bookingModelsFor("volatilitySwap"),
  toInstrument: (inputs: VolatilitySwapInputs, ctx): Instrument =>
    withTenorAndModel(
      volatilitySwapInstrument(ctx.pair, ctx.tenorYears, ctx.notionalMm, inputs.swapStrikeVol),
      ctx,
    ),
  InputBlock: VolatilitySwapInputBlock,
});
