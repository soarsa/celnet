/**
 * Quanto — a legless {@link ProductSpec} (GW2). A settlement-currency-converted
 * vanilla or cash-or-nothing digital with the quanto-drift adjustment −ρ·σ_S·σ_Z.
 * Extracted verbatim from the former `TicketWorkspace` monolith so the wire output
 * is byte-identical.
 */
import type { Instrument, OptionType, QuantoPayoff } from "../data/contract";
import { quantoInstrument, type QuantoTerms } from "../data/seed";
import { bookingModelsFor } from "../data/seed";
import { fmtRate, fmtVol } from "../lib/format";
import styles from "../workspaces/TicketWorkspace.module.css";
import { defineProduct, withTenorAndModel, type InputBlockProps } from "./types";

/** The quanto ticket inputs (payoff kind / option type / strike / σ_Z / ρ). */
export interface QuantoInputs {
  payoff: QuantoPayoff;
  optionType: OptionType;
  /** Strike as an absolute level; defaults to ATM-forward when first shown. */
  strike: number;
  conversionVol: number;
  correlation: number;
}

/** The default quanto inputs at first render (an ATM-ish vanilla quanto call). */
export const DEFAULT_QUANTO: QuantoInputs = {
  payoff: "VANILLA",
  optionType: "CALL",
  strike: 0,
  conversionVol: 0.1,
  correlation: 0.3,
};

/** Build `QuantoTerms` from the inputs (strike falls back to ATM-forward). */
export function quantoTerms(q: QuantoInputs, atmForward: number): QuantoTerms {
  return {
    payoff: q.payoff,
    optionType: q.optionType,
    strike: q.strike > 0 ? q.strike : atmForward,
    conversionVol: Math.max(0, q.conversionVol),
    correlation: Math.min(1, Math.max(-1, q.correlation)),
  };
}

function OptionToggle({
  value,
  onChange,
}: {
  value: OptionType;
  onChange: (next: OptionType) => void;
}) {
  return (
    <div className={styles.toggleGroup} role="tablist" aria-label="option type">
      {(["CALL", "PUT"] as OptionType[]).map((ot) => (
        <button
          key={ot}
          role="tab"
          aria-selected={value === ot}
          className={`${styles.modeTab} ${value === ot ? styles.modeActive : ""}`}
          onClick={() => onChange(ot)}
        >
          {ot === "CALL" ? "Call" : "Put"}
        </button>
      ))}
    </div>
  );
}

function QuantoInputBlock({ value, onChange, ctx }: InputBlockProps<QuantoInputs>) {
  const { atmForward, pipDecimals } = ctx;
  return (
    <div className={styles.product}>
      <div className={styles.productRow}>
        <span className={styles.productLabel}>Payoff</span>
        <div className={styles.toggleGroup} role="tablist" aria-label="payoff">
          {(["VANILLA", "DIGITAL"] as QuantoPayoff[]).map((p) => (
            <button
              key={p}
              role="tab"
              aria-selected={value.payoff === p}
              className={`${styles.modeTab} ${value.payoff === p ? styles.modeActive : ""}`}
              onClick={() => onChange({ ...value, payoff: p })}
            >
              {p === "VANILLA" ? "Vanilla" : "Digital"}
            </button>
          ))}
        </div>
        <OptionToggle
          value={value.optionType}
          onChange={(ot) => onChange({ ...value, optionType: ot })}
        />
      </div>
      <div className={styles.productRow}>
        <label className={styles.productField}>
          <span>Strike</span>
          <input
            className="num"
            type="number"
            min={0}
            step={Math.pow(10, -pipDecimals)}
            value={value.strike}
            aria-label="strike"
            placeholder={atmForward.toFixed(pipDecimals)}
            onChange={(ev) => onChange({ ...value, strike: Math.max(0, Number(ev.target.value)) })}
          />
          <span>{value.strike > 0 ? "" : `ATMF ${fmtRate(atmForward, pipDecimals)}`}</span>
        </label>
        <label className={styles.productField}>
          <span>Conv vol σ_Z</span>
          <input
            className="num"
            type="number"
            min={0}
            step={0.005}
            value={value.conversionVol}
            aria-label="conversion vol"
            onChange={(ev) =>
              onChange({ ...value, conversionVol: Math.max(0, Number(ev.target.value)) })
            }
          />
          <span>{fmtVol(value.conversionVol)}</span>
        </label>
        <label className={styles.productField}>
          <span>Corr ρ</span>
          <input
            className="num"
            type="number"
            min={-1}
            max={1}
            step={0.05}
            value={value.correlation}
            aria-label="correlation"
            onChange={(ev) =>
              onChange({
                ...value,
                correlation: Math.min(1, Math.max(-1, Number(ev.target.value))),
              })
            }
          />
        </label>
      </div>
      <p className={styles.productNote}>
        Quanto {value.payoff === "VANILLA" ? "vanilla" : "cash-or-nothing digital"}: the payoff is
        settlement-currency converted with the quanto-drift adjustment −ρ·σ_S·σ_Z; at ρ=0 it
        collapses to the plain {value.payoff === "VANILLA" ? "vanilla" : "digital"}.
      </p>
    </div>
  );
}

/** The quanto {@link ProductSpec}. */
export const quantoSpec = defineProduct<QuantoInputs>({
  id: "QUANTO",
  label: "Quanto",
  group: "Structured",
  assetClass: "FX",
  summary:
    "Quanto vanilla / digital — settlement-currency converted with the −ρ·σ_S·σ_Z drift adjustment.",
  keywords: ["quanto", "conversion", "correlation", "settlement currency", "drift adjustment"],
  kind: "quanto",
  defaults: DEFAULT_QUANTO,
  allowedModels: bookingModelsFor("quanto"),
  toInstrument: (inputs: QuantoInputs, ctx): Instrument =>
    withTenorAndModel(
      quantoInstrument(ctx.pair, ctx.tenorYears, ctx.notionalMm, quantoTerms(inputs, ctx.atmForward)),
      ctx,
    ),
  InputBlock: QuantoInputBlock,
});
