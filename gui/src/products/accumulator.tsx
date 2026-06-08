/**
 * Accumulator — a {@link ProductSpec} (GW2). Periodic accrual at a pivot with an
 * up-and-out knock-out barrier and geared downside; always Monte-Carlo, so the
 * build surfaces a standard error. Extracted verbatim from the former
 * `TicketWorkspace` monolith so the wire output is byte-identical.
 */
import type { AccumulatorMonitoring, Instrument } from "../data/contract";
import {
  accumulatorInstrument,
  bookingModelsFor,
  equalFixingSchedule,
  type AccumulatorTerms,
} from "../data/seed";
import { fmtRate } from "../lib/format";
import styles from "../workspaces/TicketWorkspace.module.css";
import { defineProduct, withTenorAndModel, type InputBlockProps } from "./types";

/**
 * The accumulator ticket inputs (pivot / knock-out barrier / leverage / monitoring
 * / fixing count / MC). Always Monte-Carlo. `pivot` 0 ⇒ default to the ATM-forward
 * level; the barrier is held strictly above the pivot at build.
 */
export interface AccumulatorInputs {
  pivot: number;
  barrierOffset: number;
  leverage: number;
  monitoring: AccumulatorMonitoring;
  fixings: number;
  mcPairs: number;
  mcSeed: bigint;
}

export const DEFAULT_ACCUMULATOR: AccumulatorInputs = {
  pivot: 0,
  barrierOffset: 0.05,
  leverage: 2,
  monitoring: "DISCRETE",
  fixings: 12,
  mcPairs: 0,
  mcSeed: 0xacc_1en,
};

/**
 * Build `AccumulatorTerms` (pivot ATMF-defaulted; barrier = pivot·(1+offset), kept
 * strictly above the pivot; equally-spaced schedule).
 */
export function accumulatorTerms(
  a: AccumulatorInputs,
  atmForward: number,
  expiryYears: number,
): AccumulatorTerms {
  const pivot = a.pivot > 0 ? a.pivot : atmForward;
  const offset = Math.max(1e-4, a.barrierOffset);
  const fixings = Math.max(1, Math.trunc(a.fixings));
  return {
    pivot,
    barrier: pivot * (1 + offset),
    leverage: Math.max(0, a.leverage),
    monitoring: a.monitoring,
    schedule: equalFixingSchedule(fixings, expiryYears, 1),
    mcPairs: Math.max(0, Math.trunc(a.mcPairs)),
    mcSeed: a.mcSeed,
  };
}

function AccumulatorInputBlock({
  value: accumulator,
  onChange,
  ctx,
}: InputBlockProps<AccumulatorInputs>) {
  const { atmForward, pipDecimals } = ctx;
  const onAccumulator = onChange;
  return (
    <div className={styles.product}>
      <div className={styles.productRow}>
        <label className={styles.productField}>
          <span>Pivot</span>
          <input
            className="num"
            type="number"
            min={0}
            step={Math.pow(10, -pipDecimals)}
            value={accumulator.pivot}
            aria-label="pivot"
            placeholder={atmForward.toFixed(pipDecimals)}
            onChange={(ev) =>
              onAccumulator({ ...accumulator, pivot: Math.max(0, Number(ev.target.value)) })
            }
          />
          <span>{accumulator.pivot > 0 ? "" : `ATMF ${fmtRate(atmForward, pipDecimals)}`}</span>
        </label>
        <label className={styles.productField}>
          <span>KO +</span>
          <input
            className="num"
            type="number"
            min={0}
            step={0.005}
            value={accumulator.barrierOffset}
            aria-label="barrier offset"
            onChange={(ev) =>
              onAccumulator({
                ...accumulator,
                barrierOffset: Math.max(1e-4, Number(ev.target.value)),
              })
            }
          />
          <span>·pivot</span>
        </label>
        <label className={styles.productField}>
          <span>Leverage</span>
          <input
            className="num"
            type="number"
            min={0}
            step={0.5}
            value={accumulator.leverage}
            aria-label="leverage"
            onChange={(ev) =>
              onAccumulator({ ...accumulator, leverage: Math.max(0, Number(ev.target.value)) })
            }
          />
          <span>×</span>
        </label>
      </div>
      <div className={styles.productRow}>
        <span className={styles.productLabel}>Knock-out</span>
        <div className={styles.toggleGroup} role="tablist" aria-label="monitoring">
          {(["DISCRETE", "CONTINUOUS"] as AccumulatorMonitoring[]).map((mon) => (
            <button
              key={mon}
              role="tab"
              aria-selected={accumulator.monitoring === mon}
              className={`${styles.modeTab} ${accumulator.monitoring === mon ? styles.modeActive : ""}`}
              onClick={() => onAccumulator({ ...accumulator, monitoring: mon })}
            >
              {mon === "DISCRETE" ? "At fixings" : "Continuous"}
            </button>
          ))}
        </div>
        <label className={styles.productField}>
          <span>Fixings</span>
          <input
            className="num"
            type="number"
            min={1}
            step={1}
            value={accumulator.fixings}
            aria-label="fixings"
            onChange={(ev) =>
              onAccumulator({
                ...accumulator,
                fixings: Math.max(1, Math.trunc(Number(ev.target.value))),
              })
            }
          />
        </label>
        <label className={styles.productField}>
          <span>MC pairs</span>
          <input
            className="num"
            type="number"
            min={0}
            step={1000}
            value={accumulator.mcPairs}
            aria-label="mc pairs"
            onChange={(ev) =>
              onAccumulator({
                ...accumulator,
                mcPairs: Math.max(0, Math.trunc(Number(ev.target.value))),
              })
            }
          />
          <span>{accumulator.mcPairs > 0 ? "pairs" : "default"}</span>
        </label>
      </div>
      <p className={styles.productNote}>
        Accumulator: periodic accrual at the pivot with an up-and-out knock-out barrier and geared
        downside. Priced by antithetic Monte-Carlo (client present value) — it reports a standard
        error; continuous monitoring knocks out more often than at-fixings.
      </p>
    </div>
  );
}

/** The accumulator {@link ProductSpec}. */
export const accumulatorSpec = defineProduct<AccumulatorInputs>({
  id: "ACCUMULATOR",
  label: "Accumulator",
  group: "Structured",
  assetClass: "FX",
  summary:
    "Accumulator — periodic accrual at a pivot with an up-and-out knock-out and geared downside.",
  keywords: ["accumulator", "accrual", "pivot", "knock-out", "geared", "decumulator"],
  kind: "accumulator",
  defaults: DEFAULT_ACCUMULATOR,
  allowedModels: bookingModelsFor("accumulator"),
  toInstrument: (inputs: AccumulatorInputs, ctx): Instrument =>
    withTenorAndModel(
      accumulatorInstrument(
        ctx.pair,
        ctx.tenorYears,
        ctx.notionalMm,
        accumulatorTerms(inputs, ctx.atmForward, ctx.tenorYears),
      ),
      ctx,
    ),
  InputBlock: AccumulatorInputBlock,
});
