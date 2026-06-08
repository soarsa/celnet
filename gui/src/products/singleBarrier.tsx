/**
 * Single-barrier (in/out, up/down) — a {@link ProductSpec} migrated verbatim from
 * the former `TicketWorkspace` monolith. The barrier is entered as an absolute
 * quote level; the strike too (`0` ⇒ ATM-forward). Continuous monitoring is the
 * closed-form regime priced client-side (the server prices the same continuous
 * closed form). The wire output is byte-identical to the legacy `buildInstrument`.
 */
import type {
  BarrierKind,
  BarrierSide,
  Instrument,
  OptionType,
} from "../data/contract";
import {
  bookingModelsFor,
  singleBarrierInstrument,
  type SingleBarrierTerms,
} from "../data/seed";
import { fmtRate } from "../lib/format";
import styles from "../workspaces/TicketWorkspace.module.css";
import { defineProduct, withTenorAndModel, type InputBlockProps } from "./types";

/**
 * The single-barrier ticket inputs. The barrier is entered as a level (absolute
 * quote); the strike is entered as a level too (`0` ⇒ default to the ATM-forward
 * level). The trader picks the underlying call/put, the knock kind (in/out), the
 * barrier side (up/down), and an optional rebate.
 */
export interface SingleBarrierInputs {
  optionType: OptionType;
  /** Strike as an absolute level; `0` ⇒ default to the ATM-forward level. */
  strike: number;
  kind: BarrierKind;
  side: BarrierSide;
  /** The barrier level (absolute quote). */
  barrier: number;
  rebate: number;
}

export const DEFAULT_SINGLE_BARRIER: SingleBarrierInputs = {
  optionType: "CALL",
  strike: 0,
  kind: "KNOCK_OUT",
  side: "UP",
  barrier: 0,
  rebate: 0,
};

/**
 * Build `SingleBarrierTerms` from the inputs. The strike defaults to the
 * ATM-forward level; the barrier defaults to a sensible offset from spot on the
 * chosen side (so the ticket prices a live barrier before the trader types one).
 * Continuous monitoring is the closed-form regime (the only one priced
 * client-side; the server prices the same continuous closed form).
 */
export function singleBarrierTerms(
  s: SingleBarrierInputs,
  atmForward: number,
  spot: number,
): SingleBarrierTerms {
  const strike = s.strike > 0 ? s.strike : atmForward;
  const defaultBarrier = s.side === "UP" ? spot * 1.05 : spot * 0.95;
  const barrier = s.barrier > 0 ? s.barrier : defaultBarrier;
  return {
    optionType: s.optionType,
    strike: { kind: "strike", strike },
    kind: s.kind,
    side: s.side,
    barrier,
    rebate: Math.max(0, s.rebate),
    monitoring: "CONTINUOUS",
  };
}

function SingleBarrierInputBlock({ value, onChange, ctx }: InputBlockProps<SingleBarrierInputs>) {
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
        <div className={styles.toggleGroup} role="tablist" aria-label="barrier side">
          {(["UP", "DOWN"] as BarrierSide[]).map((bs) => (
            <button
              key={bs}
              role="tab"
              aria-selected={value.side === bs}
              className={`${styles.modeTab} ${value.side === bs ? styles.modeActive : ""}`}
              onClick={() => onChange({ ...value, side: bs })}
            >
              {bs === "UP" ? "Up" : "Down"}
            </button>
          ))}
        </div>
      </div>
      <div className={styles.productRow}>
        <label className={styles.productField}>
          <span>Barrier</span>
          <input
            className="num"
            type="number"
            min={0}
            step={step}
            value={value.barrier}
            aria-label="barrier"
            placeholder={(value.side === "UP" ? spot * 1.05 : spot * 0.95).toFixed(
              pipDecimals,
            )}
            onChange={(ev) =>
              onChange({ ...value, barrier: Math.max(0, Number(ev.target.value)) })
            }
          />
          <span>
            {value.barrier > 0
              ? ""
              : fmtRate(value.side === "UP" ? spot * 1.05 : spot * 0.95, pipDecimals)}
          </span>
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
        Single-barrier {value.side === "UP" ? "up" : "down"}-and-
        {value.kind === "KNOCK_IN" ? "in" : "out"} {value.optionType === "CALL" ? "call" : "put"}:
        a vanilla that {value.kind === "KNOCK_IN" ? "activates" : "extinguishes"} when the
        continuously-monitored spot touches the barrier. Priced by the reflection-principle closed
        form; in/out parity (knock-in + knock-out = vanilla) holds by construction.
      </p>
    </div>
  );
}

/** The single-barrier {@link ProductSpec}. */
export const singleBarrierSpec = defineProduct<SingleBarrierInputs>({
  id: "SINGLE_BARRIER",
  label: "Single Barrier",
  group: "Barriers & digitals",
  assetClass: "FX",
  summary:
    "Vanilla that knocks in/out (up or down) when continuously-monitored spot touches the barrier.",
  keywords: ["barrier", "knock-in", "knock-out", "up-and-out", "down-and-in", "reflection"],
  kind: "singleBarrier",
  defaults: DEFAULT_SINGLE_BARRIER,
  allowedModels: bookingModelsFor("singleBarrier"),
  toInstrument: (inputs: SingleBarrierInputs, ctx): Instrument =>
    withTenorAndModel(
      singleBarrierInstrument(
        ctx.pair,
        ctx.tenorYears,
        ctx.notionalMm,
        singleBarrierTerms(inputs, ctx.atmForward, ctx.spot),
      ),
      ctx,
    ),
  InputBlock: SingleBarrierInputBlock,
});
