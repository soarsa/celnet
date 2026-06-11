/**
 * Pivot Target-Redemption Accumulator (pivot TRA, wire arm 32) — a
 * {@link ProductSpec} (GW2). The TARF mechanic with a distinct pivot kink: each
 * fixing's leg is SELECTED by the pivot level and VALUED by the strike intrinsic,
 * the favourable leg accruing toward the cumulative target (knock-out on target,
 * the shared gap-risk redemption convention), the adverse leg geared by the
 * leverage. `pivot == strike` is the exact plain-TARF slice. Always Monte-Carlo,
 * so the build surfaces a standard error.
 */
import type { Instrument, OptionType, TarfRedemption } from "../data/contract";
import {
  bookingModelsFor,
  equalFixingSchedule,
  pivotInstrument,
  type PivotTerms,
} from "../data/seed";
import { fmtRate } from "../lib/format";
import styles from "../workspaces/TicketWorkspace.module.css";
import { defineProduct, withTenorAndModel, type InputBlockProps } from "./types";

/**
 * The pivot-TRA ticket inputs (favourable side / strike / pivot / target /
 * leverage / gap-risk redemption / fixing count / MC). Always Monte-Carlo, so the
 * build always surfaces a standard error. `strike` 0 ⇒ default to the ATM-forward
 * level; `pivot` 0 ⇒ default to the (resolved) strike — the exact TARF slice.
 */
export interface PivotInputs {
  optionType: OptionType;
  strike: number;
  pivot: number;
  target: number;
  leverage: number;
  redemption: TarfRedemption;
  fixings: number;
  mcPairs: number;
  mcSeed: bigint;
}

export const DEFAULT_PIVOT: PivotInputs = {
  optionType: "PUT",
  strike: 0,
  pivot: 0,
  target: 0.1,
  leverage: 2,
  redemption: "FULL_GAIN",
  fixings: 12,
  mcPairs: 0,
  mcSeed: 0x91_707n,
};

/**
 * Build `PivotTerms` from the inputs (equally-spaced schedule; strike
 * ATMF-defaulted; pivot strike-defaulted — the TARF slice — when left 0).
 */
export function pivotTerms(t: PivotInputs, atmForward: number, expiryYears: number): PivotTerms {
  const fixings = Math.max(1, Math.trunc(t.fixings));
  const strike = t.strike > 0 ? t.strike : atmForward;
  return {
    optionType: t.optionType,
    strike,
    pivot: t.pivot > 0 ? t.pivot : strike,
    target: Math.max(0, t.target),
    leverage: Math.max(0, t.leverage),
    redemption: t.redemption,
    schedule: equalFixingSchedule(fixings, expiryYears, 1),
    mcPairs: Math.max(0, Math.trunc(t.mcPairs)),
    mcSeed: t.mcSeed,
  };
}

function PivotInputBlock({ value: pivot, onChange, ctx }: InputBlockProps<PivotInputs>) {
  const { atmForward, pipDecimals } = ctx;
  const onPivot = onChange;
  const resolvedStrike = pivot.strike > 0 ? pivot.strike : atmForward;
  return (
    <div className={styles.product}>
      <div className={styles.productRow}>
        <span className={styles.productLabel}>Gain side</span>
        <div className={styles.toggleGroup} role="tablist" aria-label="option type">
          {(["CALL", "PUT"] as OptionType[]).map((ot) => (
            <button
              key={ot}
              role="tab"
              aria-selected={pivot.optionType === ot}
              className={`${styles.modeTab} ${pivot.optionType === ot ? styles.modeActive : ""}`}
              onClick={() => onPivot({ ...pivot, optionType: ot })}
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
            step={Math.pow(10, -pipDecimals)}
            value={pivot.strike}
            aria-label="strike"
            placeholder={atmForward.toFixed(pipDecimals)}
            onChange={(ev) => onPivot({ ...pivot, strike: Math.max(0, Number(ev.target.value)) })}
          />
          <span>{pivot.strike > 0 ? "" : `ATMF ${fmtRate(atmForward, pipDecimals)}`}</span>
        </label>
        <label className={styles.productField}>
          <span>Pivot</span>
          <input
            className="num"
            type="number"
            min={0}
            step={Math.pow(10, -pipDecimals)}
            value={pivot.pivot}
            aria-label="pivot"
            placeholder={resolvedStrike.toFixed(pipDecimals)}
            onChange={(ev) => onPivot({ ...pivot, pivot: Math.max(0, Number(ev.target.value)) })}
          />
          <span>{pivot.pivot > 0 ? "" : `= strike (TARF slice)`}</span>
        </label>
      </div>
      <div className={styles.productRow}>
        <label className={styles.productField}>
          <span>Target</span>
          <input
            className="num"
            type="number"
            min={0}
            step={0.01}
            value={pivot.target}
            aria-label="target"
            onChange={(ev) => onPivot({ ...pivot, target: Math.max(0, Number(ev.target.value)) })}
          />
        </label>
        <label className={styles.productField}>
          <span>Leverage</span>
          <input
            className="num"
            type="number"
            min={0}
            step={0.5}
            value={pivot.leverage}
            aria-label="leverage"
            onChange={(ev) =>
              onPivot({ ...pivot, leverage: Math.max(0, Number(ev.target.value)) })
            }
          />
          <span>×</span>
        </label>
        <label className={styles.productField}>
          <span>Fixings</span>
          <input
            className="num"
            type="number"
            min={1}
            step={1}
            value={pivot.fixings}
            aria-label="fixings"
            onChange={(ev) =>
              onPivot({ ...pivot, fixings: Math.max(1, Math.trunc(Number(ev.target.value))) })
            }
          />
        </label>
      </div>
      <div className={styles.productRow}>
        <span className={styles.productLabel}>Redemption</span>
        <div className={styles.toggleGroup} role="tablist" aria-label="redemption">
          {(["FULL_GAIN", "CAPPED_GAIN"] as TarfRedemption[]).map((r) => (
            <button
              key={r}
              role="tab"
              aria-selected={pivot.redemption === r}
              className={`${styles.modeTab} ${pivot.redemption === r ? styles.modeActive : ""}`}
              onClick={() => onPivot({ ...pivot, redemption: r })}
            >
              {r === "FULL_GAIN" ? "Full gain" : "Capped gain"}
            </button>
          ))}
        </div>
        <label className={styles.productField}>
          <span>MC pairs</span>
          <input
            className="num"
            type="number"
            min={0}
            step={1000}
            value={pivot.mcPairs}
            aria-label="mc pairs"
            onChange={(ev) =>
              onPivot({ ...pivot, mcPairs: Math.max(0, Math.trunc(Number(ev.target.value))) })
            }
          />
          <span>{pivot.mcPairs > 0 ? "pairs" : "default"}</span>
        </label>
      </div>
      <p className={styles.productNote}>
        Pivot Target-Redemption Accumulator: the TARF strip with a distinct pivot kink — the
        geared adverse leg engages only past the pivot, while gains accrue against the strike
        toward the redemption target. Setting the pivot equal to the strike recovers the plain
        TARF exactly. Priced by antithetic Monte-Carlo (bank present value) — it reports a
        standard error.
      </p>
    </div>
  );
}

/** The pivot-TRA {@link ProductSpec}. */
export const pivotSpec = defineProduct<PivotInputs>({
  id: "PIVOT",
  label: "Pivot TRA",
  group: "Structured",
  assetClass: "FX",
  summary:
    "Pivot Target-Redemption Accumulator — a TARF with a distinct pivot kink where the geared adverse leg engages.",
  keywords: ["pivot", "tra", "target redemption", "accumulator", "tarf", "geared", "kink"],
  kind: "pivot",
  defaults: DEFAULT_PIVOT,
  allowedModels: bookingModelsFor("pivot"),
  toInstrument: (inputs: PivotInputs, ctx): Instrument =>
    withTenorAndModel(
      pivotInstrument(
        ctx.pair,
        ctx.tenorYears,
        ctx.notionalMm,
        pivotTerms(inputs, ctx.atmForward, ctx.tenorYears),
      ),
      ctx,
    ),
  InputBlock: PivotInputBlock,
});
