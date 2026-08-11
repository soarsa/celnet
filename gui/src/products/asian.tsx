/**
 * Arithmetic-average-rate Asian — the reference {@link ProductSpec} (GW2). This
 * is the template every legless exotic family follows: an `Inputs` shape, the
 * `*Terms` transform, the default inputs, a `toInstrument` that delegates to the
 * `data/seed` wire builder, and a self-contained `InputBlock`. Extracted verbatim
 * from the former `TicketWorkspace` monolith so the wire output is byte-identical.
 */
import type {
  AsianMethod,
  AveragingStyle,
  Instrument,
  OptionType,
} from "../data/contract";
import { asianInstrument, type AsianTerms } from "../data/seed";
import { fmtRate } from "../lib/format";
import styles from "../workspaces/TicketWorkspace.module.css";
import { defineProduct, withTenorAndModel, type InputBlockProps } from "./types";

import { NumberField } from "../components/NumberField";

/** The Asian ticket inputs (fresh — no in-progress average / seasoning). */
export interface AsianInputs {
  optionType: OptionType;
  /** Strike as an absolute level; `0` ⇒ default to the ATM-forward when shown. */
  strike: number;
  averaging: AveragingStyle;
  observations: number;
  method: AsianMethod;
}

/** The default Asian inputs at first render (a fresh, discrete, ATM-ish call). */
export const DEFAULT_ASIAN: AsianInputs = {
  optionType: "CALL",
  strike: 0,
  averaging: "DISCRETE",
  observations: 12,
  method: "CURRAN",
};

/** Build the `AsianTerms` from the ticket inputs (fresh — `elapsed*` zeroed). */
export function asianTerms(a: AsianInputs): AsianTerms {
  return {
    optionType: a.optionType,
    strike: a.strike,
    averaging: a.averaging,
    observations: a.observations,
    method: a.method,
    elapsedAvg: 0,
    elapsedWeight: 0,
  };
}

function AsianInputBlock({ value, onChange, ctx }: InputBlockProps<AsianInputs>) {
  const { atmForward, pipDecimals } = ctx;
  return (
    <div className={styles.product}>
      <div className={styles.productRow}>
        <span className={styles.productLabel}>Option</span>
        <div className={styles.toggleGroup} role="tablist" aria-label="option type">
          {(["CALL", "PUT"] as OptionType[]).map((ot) => (
            <button
              key={ot}
              role="tab"
              aria-selected={value.optionType === ot}
              className={`${styles.modeTab} ${value.optionType === ot ? styles.modeActive : ""}`}
              onClick={() => onChange({ ...value, optionType: ot })}
            >
              {ot === "CALL" ? "Call" : "Put"}
            </button>
          ))}
        </div>
        <label className={styles.productField}>
          <span>Strike</span>
          <NumberField
            className="num"
            min={0}
            step={Math.pow(10, -pipDecimals)}
            value={value.strike}
            aria-label="strike"
            placeholder={atmForward.toFixed(pipDecimals)}
            onChange={(ev) => onChange({ ...value, strike: Math.max(0, Number(ev.target.value)) })}
          />
          <span>{value.strike > 0 ? "" : `ATMF ${fmtRate(atmForward, pipDecimals)}`}</span>
        </label>
      </div>
      <div className={styles.productRow}>
        <span className={styles.productLabel}>Averaging</span>
        <div className={styles.toggleGroup} role="tablist" aria-label="averaging">
          {(["DISCRETE", "CONTINUOUS"] as AveragingStyle[]).map((av) => (
            <button
              key={av}
              role="tab"
              aria-selected={value.averaging === av}
              className={`${styles.modeTab} ${value.averaging === av ? styles.modeActive : ""}`}
              onClick={() => onChange({ ...value, averaging: av })}
            >
              {av === "DISCRETE" ? "Discrete" : "Continuous"}
            </button>
          ))}
        </div>
        {value.averaging === "DISCRETE" && (
          <label className={styles.productField}>
            <span>Fixings</span>
            <NumberField
              className="num"
              min={1}
              step={1}
              value={value.observations}
              aria-label="observations"
              onChange={(ev) =>
                onChange({
                  ...value,
                  observations: Math.max(1, Math.trunc(Number(ev.target.value))),
                })
              }
            />
          </label>
        )}
      </div>
      <div className={styles.productRow}>
        <span className={styles.productLabel}>Method</span>
        <div className={styles.toggleGroup} role="tablist" aria-label="method">
          {(["CURRAN", "TURNBULL_WAKEMAN"] as AsianMethod[]).map((mm) => (
            <button
              key={mm}
              role="tab"
              aria-selected={value.method === mm}
              className={`${styles.modeTab} ${value.method === mm ? styles.modeActive : ""}`}
              onClick={() => onChange({ ...value, method: mm })}
            >
              {mm === "CURRAN" ? "Curran" : "Turnbull-Wakeman"}
            </button>
          ))}
        </div>
      </div>
      <p className={styles.productNote}>
        Arithmetic-average-rate Asian. The selected analytic method is priced server-side; the
        standalone build prices a two-moment closed form (exact in the single-fixing / zero-vol
        limits).
      </p>
    </div>
  );
}

/** The Asian {@link ProductSpec}. */
export const asianSpec = defineProduct<AsianInputs>({
  id: "ASIAN",
  label: "Asian (average-rate)",
  group: "Path-dependent",
  assetClass: "FX",
  summary: "Arithmetic-average-rate option — payoff on the average fixing, not the terminal spot.",
  keywords: ["average", "asian", "curran", "turnbull", "wakeman", "average rate"],
  kind: "asianOption",
  defaults: DEFAULT_ASIAN,
  allowedModels: ["DEFAULT"],
  toInstrument: (inputs: AsianInputs, ctx): Instrument =>
    withTenorAndModel(
      asianInstrument(ctx.pair, ctx.tenorYears, ctx.notionalMm, asianTerms(inputs)),
      ctx,
    ),
  InputBlock: AsianInputBlock,
});
