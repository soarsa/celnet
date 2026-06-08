/**
 * Double-barrier (corridor knock-in/out) — a {@link ProductSpec} migrated verbatim
 * from the former `TicketWorkspace` monolith. Both barriers are entered as levels
 * (`0` ⇒ a symmetric ±10% corridor around spot); the strike too (`0` ⇒ ATM-forward).
 * The corridor is held strictly well-ordered (`0 < L < U`) at build so the pricer is
 * in domain. The wire output is byte-identical to the legacy `buildInstrument`.
 */
import type { BarrierKind, Instrument, OptionType } from "../data/contract";
import {
  bookingModelsFor,
  doubleBarrierInstrument,
  type DoubleBarrierTerms,
} from "../data/seed";
import { fmtRate } from "../lib/format";
import styles from "../workspaces/TicketWorkspace.module.css";
import { defineProduct, withTenorAndModel, type InputBlockProps } from "./types";

/**
 * The double-barrier ticket inputs. Both barriers are entered as levels
 * (`0` ⇒ default to a symmetric corridor around spot); the strike is entered as a
 * level (`0` ⇒ ATM-forward). The corridor is held well-ordered (`0 < L < U`) at
 * build.
 */
export interface DoubleBarrierInputs {
  optionType: OptionType;
  /** Strike as an absolute level; `0` ⇒ default to the ATM-forward level. */
  strike: number;
  kind: BarrierKind;
  lowerBarrier: number;
  upperBarrier: number;
  rebate: number;
}

export const DEFAULT_DOUBLE_BARRIER: DoubleBarrierInputs = {
  optionType: "CALL",
  strike: 0,
  kind: "KNOCK_OUT",
  lowerBarrier: 0,
  upperBarrier: 0,
  rebate: 0,
};

/**
 * Build `DoubleBarrierTerms` from the inputs (strike ATMF-defaulted; a symmetric
 * ±10% corridor when a level is left blank; the corridor forced well-ordered).
 */
export function doubleBarrierTerms(
  d: DoubleBarrierInputs,
  atmForward: number,
  spot: number,
): DoubleBarrierTerms {
  const strike = d.strike > 0 ? d.strike : atmForward;
  let lower = d.lowerBarrier > 0 ? d.lowerBarrier : spot * 0.9;
  let upper = d.upperBarrier > 0 ? d.upperBarrier : spot * 1.1;
  // Keep the corridor strictly well-ordered (0 < L < U) so the pricer is in domain.
  if (lower >= upper) {
    lower = spot * 0.9;
    upper = spot * 1.1;
  }
  return {
    optionType: d.optionType,
    strike: { kind: "strike", strike },
    kind: d.kind,
    lowerBarrier: lower,
    upperBarrier: upper,
    rebate: Math.max(0, d.rebate),
    monitoring: "CONTINUOUS",
  };
}

function DoubleBarrierInputBlock({ value, onChange, ctx }: InputBlockProps<DoubleBarrierInputs>) {
  const { atmForward, spot, pipDecimals } = ctx;
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
          <input
            className="num"
            type="number"
            min={0}
            step={step}
            value={value.strike}
            aria-label="strike"
            placeholder={atmForward.toFixed(pipDecimals)}
            onChange={(ev) =>
              onChange({ ...value, strike: Math.max(0, Number(ev.target.value)) })
            }
          />
          <span>{value.strike > 0 ? "" : `ATMF ${fmtRate(atmForward, pipDecimals)}`}</span>
        </label>
      </div>
      <div className={styles.productRow}>
        <span className={styles.productLabel}>Knock</span>
        <div className={styles.toggleGroup} role="tablist" aria-label="barrier kind">
          {(["KNOCK_IN", "KNOCK_OUT"] as BarrierKind[]).map((bk) => (
            <button
              key={bk}
              role="tab"
              aria-selected={value.kind === bk}
              className={`${styles.modeTab} ${value.kind === bk ? styles.modeActive : ""}`}
              onClick={() => onChange({ ...value, kind: bk })}
            >
              {bk === "KNOCK_IN" ? "Knock-in" : "Knock-out"}
            </button>
          ))}
        </div>
      </div>
      <div className={styles.productRow}>
        <label className={styles.productField}>
          <span>Lower</span>
          <input
            className="num"
            type="number"
            min={0}
            step={step}
            value={value.lowerBarrier}
            aria-label="lower barrier"
            placeholder={(spot * 0.9).toFixed(pipDecimals)}
            onChange={(ev) =>
              onChange({
                ...value,
                lowerBarrier: Math.max(0, Number(ev.target.value)),
              })
            }
          />
          <span>{value.lowerBarrier > 0 ? "" : fmtRate(spot * 0.9, pipDecimals)}</span>
        </label>
        <label className={styles.productField}>
          <span>Upper</span>
          <input
            className="num"
            type="number"
            min={0}
            step={step}
            value={value.upperBarrier}
            aria-label="upper barrier"
            placeholder={(spot * 1.1).toFixed(pipDecimals)}
            onChange={(ev) =>
              onChange({
                ...value,
                upperBarrier: Math.max(0, Number(ev.target.value)),
              })
            }
          />
          <span>{value.upperBarrier > 0 ? "" : fmtRate(spot * 1.1, pipDecimals)}</span>
        </label>
        <label className={styles.productField}>
          <span>Rebate</span>
          <input
            className="num"
            type="number"
            min={0}
            step={0.005}
            value={value.rebate}
            aria-label="rebate"
            onChange={(ev) =>
              onChange({ ...value, rebate: Math.max(0, Number(ev.target.value)) })
            }
          />
        </label>
      </div>
      <p className={styles.productNote}>
        Double-barrier {value.kind === "KNOCK_IN" ? "knock-in" : "knock-out"}: a vanilla
        bounded by a lower and upper barrier (0 &lt; L &lt; U). Priced by the method-of-images
        corridor series; a knock-in is priced by parity (KI = vanilla − KO).
      </p>
    </div>
  );
}

/** The double-barrier {@link ProductSpec}. */
export const doubleBarrierSpec = defineProduct<DoubleBarrierInputs>({
  id: "DOUBLE_BARRIER",
  label: "Double Barrier",
  group: "Barriers & digitals",
  assetClass: "FX",
  summary: "Vanilla bounded by a lower and upper barrier; knocks in/out on the corridor.",
  keywords: ["double barrier", "corridor", "knock-in", "knock-out", "method of images"],
  kind: "doubleBarrier",
  defaults: DEFAULT_DOUBLE_BARRIER,
  allowedModels: bookingModelsFor("doubleBarrier"),
  toInstrument: (inputs: DoubleBarrierInputs, ctx): Instrument =>
    withTenorAndModel(
      doubleBarrierInstrument(
        ctx.pair,
        ctx.tenorYears,
        ctx.notionalMm,
        doubleBarrierTerms(inputs, ctx.atmForward, ctx.spot),
      ),
      ctx,
    ),
  InputBlock: DoubleBarrierInputBlock,
});
