/**
 * Lookback — a path-dependent {@link ProductSpec} (GW2). Floating|fixed style,
 * call|put, continuous|discrete monitoring. CONTINUOUS prices by the exact closed
 * form on the running extremum (no std-error); DISCRETE prices by antithetic
 * Monte-Carlo (surfaces a stderr). The FIXED family's `strike` 0 ⇒ default to the
 * ATM-forward level. Extracted verbatim from the former `TicketWorkspace` monolith
 * so the wire output is byte-identical.
 */
import type {
  Instrument,
  LookbackMonitoring,
  LookbackStyle,
  OptionType,
} from "../data/contract";
import { bookingModelsFor, lookbackInstrument, type LookbackTerms } from "../data/seed";
import { fmtRate } from "../lib/format";
import styles from "../workspaces/TicketWorkspace.module.css";
import { defineProduct, withTenorAndModel, type InputBlockProps } from "./types";

import { NumberField } from "../components/NumberField";

/**
 * The lookback ticket inputs (style floating|fixed / option / monitoring
 * continuous|discrete / strike / observations / MC). CONTINUOUS prices by exact
 * closed form (no std-error); DISCRETE prices by Monte-Carlo (surfaces a stderr).
 * `strike` 0 ⇒ default to the ATM-forward level (FIXED family only).
 */
export interface LookbackInputs {
  style: LookbackStyle;
  optionType: OptionType;
  monitoring: LookbackMonitoring;
  strike: number;
  observations: number;
  mcPairs: number;
  mcSeed: bigint;
}

/** The default lookback inputs at first render (floating continuous call). */
export const DEFAULT_LOOKBACK: LookbackInputs = {
  style: "FLOATING",
  optionType: "CALL",
  monitoring: "CONTINUOUS",
  strike: 0,
  observations: 52,
  mcPairs: 0,
  mcSeed: 0x100_b_acen,
};

/** Build `LookbackTerms` from the inputs (strike ATMF-defaulted for the FIXED family). */
export function lookbackTerms(l: LookbackInputs, atmForward: number): LookbackTerms {
  return {
    style: l.style,
    optionType: l.optionType,
    monitoring: l.monitoring,
    strike: l.strike > 0 ? l.strike : atmForward,
    observations: Math.max(0, Math.trunc(l.observations)),
    mcPairs: Math.max(0, Math.trunc(l.mcPairs)),
    mcSeed: l.mcSeed,
  };
}

function LookbackInputBlock({ value, onChange, ctx }: InputBlockProps<LookbackInputs>) {
  const { atmForward, pipDecimals } = ctx;
  const isFixed = value.style === "FIXED";
  const isDiscrete = value.monitoring === "DISCRETE";
  return (
    <div className={styles.product}>
      <div className={styles.productRow}>
        <span className={styles.productLabel}>Strike</span>
        <div className={styles.toggleGroup} role="tablist" aria-label="style">
          {(["FLOATING", "FIXED"] as LookbackStyle[]).map((st) => (
            <button
              key={st}
              role="tab"
              aria-selected={value.style === st}
              className={`${styles.modeTab} ${value.style === st ? styles.modeActive : ""}`}
              onClick={() => onChange({ ...value, style: st })}
            >
              {st === "FLOATING" ? "Floating" : "Fixed"}
            </button>
          ))}
        </div>
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
      </div>
      <div className={styles.productRow}>
        <span className={styles.productLabel}>Monitoring</span>
        <div className={styles.toggleGroup} role="tablist" aria-label="monitoring">
          {(["CONTINUOUS", "DISCRETE"] as LookbackMonitoring[]).map((mon) => (
            <button
              key={mon}
              role="tab"
              aria-selected={value.monitoring === mon}
              className={`${styles.modeTab} ${value.monitoring === mon ? styles.modeActive : ""}`}
              onClick={() => onChange({ ...value, monitoring: mon })}
            >
              {mon === "CONTINUOUS" ? "Continuous" : "Discrete"}
            </button>
          ))}
        </div>
        {isFixed && (
          <label className={styles.productField}>
            <span>Strike K</span>
            <NumberField
              className="num"
              min={0}
              step={Math.pow(10, -pipDecimals)}
              value={value.strike}
              aria-label="strike"
              placeholder={atmForward.toFixed(pipDecimals)}
              onChange={(ev) =>
                onChange({ ...value, strike: Math.max(0, Number(ev.target.value)) })
              }
            />
            <span>{value.strike > 0 ? "" : `ATMF ${fmtRate(atmForward, pipDecimals)}`}</span>
          </label>
        )}
      </div>
      {isDiscrete && (
        <div className={styles.productRow}>
          <label className={styles.productField}>
            <span>Observations</span>
            <NumberField
              className="num"
              min={0}
              step={1}
              value={value.observations}
              aria-label="observations"
              onChange={(ev) =>
                onChange({
                  ...value,
                  observations: Math.max(0, Math.trunc(Number(ev.target.value))),
                })
              }
            />
            <span>{value.observations > 0 ? "" : "default"}</span>
          </label>
          <label className={styles.productField}>
            <span>MC pairs</span>
            <NumberField
              className="num"
              min={0}
              step={1000}
              value={value.mcPairs}
              aria-label="mc pairs"
              onChange={(ev) =>
                onChange({
                  ...value,
                  mcPairs: Math.max(0, Math.trunc(Number(ev.target.value))),
                })
              }
            />
            <span>{value.mcPairs > 0 ? "pairs" : "default"}</span>
          </label>
        </div>
      )}
      <p className={styles.productNote}>
        {isDiscrete
          ? "Discrete-monitored lookback: the extremum is sampled on a finite observation grid, so it is priced by antithetic Monte-Carlo and reports a standard error."
          : "Continuous-monitored lookback: priced by the exact closed form on the running path extremum (no Monte-Carlo error)."}
      </p>
    </div>
  );
}

/** The lookback {@link ProductSpec}. */
export const lookbackSpec = defineProduct<LookbackInputs>({
  id: "LOOKBACK",
  label: "Lookback",
  group: "Path-dependent",
  assetClass: "FX",
  summary: "Lookback option — payoff on the path extremum (floating-strike or fixed-strike).",
  keywords: ["lookback", "extremum", "running maximum", "running minimum", "floating strike"],
  kind: "lookback",
  defaults: DEFAULT_LOOKBACK,
  allowedModels: bookingModelsFor("lookback"),
  toInstrument: (inputs: LookbackInputs, ctx): Instrument =>
    withTenorAndModel(
      lookbackInstrument(
        ctx.pair,
        ctx.tenorYears,
        ctx.notionalMm,
        lookbackTerms(inputs, ctx.atmForward),
      ),
      ctx,
    ),
  InputBlock: LookbackInputBlock,
});
