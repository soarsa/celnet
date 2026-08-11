/**
 * Digital (binary, cash-or-asset) — a {@link ProductSpec} migrated verbatim from
 * the former `TicketWorkspace` monolith. Pays a fixed payout if it finishes in the
 * money at expiry; the cash-or-nothing call is exactly the tight call-spread limit
 * of the vanilla. The strike defaults to ATM-forward (`0`). The wire output is
 * byte-identical to the legacy `buildInstrument`.
 */
import type { DigitalStyle, Instrument, OptionType } from "../data/contract";
import { bookingModelsFor, digitalInstrument, type DigitalTerms } from "../data/seed";
import { fmtRate } from "../lib/format";
import styles from "../workspaces/TicketWorkspace.module.css";
import { defineProduct, withTenorAndModel, type InputBlockProps } from "./types";

import { NumberField } from "../components/NumberField";

/**
 * The digital ticket inputs (call/put / strike / cash-or-asset settlement /
 * payout). `strike` `0` ⇒ ATM-forward.
 */
export interface DigitalInputs {
  optionType: OptionType;
  strike: number;
  style: DigitalStyle;
  payout: number;
}

export const DEFAULT_DIGITAL: DigitalInputs = {
  optionType: "CALL",
  strike: 0,
  style: "CASH_OR_NOTHING",
  payout: 1,
};

/** Build `DigitalTerms` from the inputs (strike ATMF-defaulted; payout ≥ 0). */
export function digitalTerms(d: DigitalInputs, atmForward: number): DigitalTerms {
  return {
    optionType: d.optionType,
    strike: d.strike > 0 ? d.strike : atmForward,
    style: d.style,
    payout: Math.max(0, d.payout),
  };
}

function DigitalInputBlock({ value, onChange, ctx }: InputBlockProps<DigitalInputs>) {
  const { atmForward, pipDecimals } = ctx;
  const step = Math.pow(10, -pipDecimals);
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
            step={step}
            value={value.strike}
            aria-label="strike"
            placeholder={atmForward.toFixed(pipDecimals)}
            onChange={(ev) => onChange({ ...value, strike: Math.max(0, Number(ev.target.value)) })}
          />
          <span>{value.strike > 0 ? "" : `ATMF ${fmtRate(atmForward, pipDecimals)}`}</span>
        </label>
      </div>
      <div className={styles.productRow}>
        <span className={styles.productLabel}>Settles</span>
        <div className={styles.toggleGroup} role="tablist" aria-label="digital style">
          {(["CASH_OR_NOTHING", "ASSET_OR_NOTHING"] as DigitalStyle[]).map((ds) => (
            <button
              key={ds}
              role="tab"
              aria-selected={value.style === ds}
              className={`${styles.modeTab} ${value.style === ds ? styles.modeActive : ""}`}
              onClick={() => onChange({ ...value, style: ds })}
            >
              {ds === "CASH_OR_NOTHING" ? "Cash" : "Asset"}
            </button>
          ))}
        </div>
        <label className={styles.productField}>
          <span>Payout</span>
          <NumberField
            className="num"
            min={0}
            step={0.5}
            value={value.payout}
            aria-label="payout"
            onChange={(ev) => onChange({ ...value, payout: Math.max(0, Number(ev.target.value)) })}
          />
        </label>
      </div>
      <p className={styles.productNote}>
        {value.style === "CASH_OR_NOTHING" ? "Cash-or-nothing" : "Asset-or-nothing"} digital{" "}
        {value.optionType === "CALL" ? "call" : "put"}: pays the fixed payout if it finishes{" "}
        {value.optionType === "CALL" ? "above" : "below"} the strike at expiry. The
        cash-or-nothing call is exactly the tight call-spread limit −∂C/∂K of the vanilla.
      </p>
    </div>
  );
}

/** The digital {@link ProductSpec}. */
export const digitalSpec = defineProduct<DigitalInputs>({
  id: "DIGITAL",
  label: "Digital",
  group: "Barriers & digitals",
  assetClass: "FX",
  summary: "Binary option paying a fixed payout (cash or asset) when in the money at expiry.",
  keywords: ["digital", "binary", "cash-or-nothing", "asset-or-nothing", "call spread"],
  kind: "digital",
  defaults: DEFAULT_DIGITAL,
  allowedModels: bookingModelsFor("digital"),
  toInstrument: (inputs: DigitalInputs, ctx): Instrument =>
    withTenorAndModel(
      digitalInstrument(
        ctx.pair,
        ctx.tenorYears,
        ctx.notionalMm,
        digitalTerms(inputs, ctx.atmForward),
      ),
      ctx,
    ),
  InputBlock: DigitalInputBlock,
});
