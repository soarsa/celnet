/**
 * Touch family ({@link ProductSpec}) — one-touch / no-touch and their corridor
 * (double) variants. Extracted verbatim from the former `TicketWorkspace`
 * monolith so the wire output is byte-identical: the inputs shape, the
 * `touchTerms` transform (spot-defaulted barriers), the defaults, the
 * `toInstrument` that delegates to the `data/seed` wire builder, and the
 * self-contained `InputBlock`.
 */
import type { Instrument, TouchKind } from "../data/contract";
import { bookingModelsFor, isDoubleTouch, touchInstrument, type TouchTerms } from "../data/seed";
import { fmtRate } from "../lib/format";
import styles from "../workspaces/TicketWorkspace.module.css";
import { defineProduct, withTenorAndModel, type InputBlockProps } from "./types";

/**
 * The touch ticket inputs (kind / barrier level(s) / rebate). The single-barrier
 * kinds (one-/no-touch) use only `lowerBarrier`; the double kinds use both. Levels
 * `0` ⇒ a sensible default around spot at build.
 */
export interface TouchInputs {
  kind: TouchKind;
  /** The (lower / sole) barrier level (absolute quote). */
  lowerBarrier: number;
  /** The upper barrier level for the double structures (absolute quote). */
  upperBarrier: number;
  rebate: number;
}

export const DEFAULT_TOUCH: TouchInputs = {
  kind: "ONE_TOUCH",
  lowerBarrier: 0,
  upperBarrier: 0,
  rebate: 1,
};

/**
 * Build `TouchTerms` from the inputs. A single-barrier touch defaults its sole
 * barrier 5% above spot; a double structure defaults a symmetric ±5% corridor,
 * forced well-ordered. Continuous monitoring is the closed-form regime.
 */
export function touchTerms(t: TouchInputs, spot: number): TouchTerms {
  const double = isDoubleTouch(t.kind);
  if (double) {
    let lower = t.lowerBarrier > 0 ? t.lowerBarrier : spot * 0.95;
    let upper = t.upperBarrier > 0 ? t.upperBarrier : spot * 1.05;
    if (lower >= upper) {
      lower = spot * 0.95;
      upper = spot * 1.05;
    }
    return {
      kind: t.kind,
      lowerBarrier: lower,
      upperBarrier: upper,
      rebate: Math.max(0, t.rebate),
      monitoring: "CONTINUOUS",
    };
  }
  const lower = t.lowerBarrier > 0 ? t.lowerBarrier : spot * 1.05;
  return {
    kind: t.kind,
    lowerBarrier: lower,
    upperBarrier: 0,
    rebate: Math.max(0, t.rebate),
    monitoring: "CONTINUOUS",
  };
}

function TouchInputBlock({ value: touch, onChange: onTouch, ctx }: InputBlockProps<TouchInputs>) {
  const { spot, pipDecimals } = ctx;
  const step = Math.pow(10, -pipDecimals);
  const double = isDoubleTouch(touch.kind);
  return (
    <div className={styles.product}>
      <div className={styles.productRow}>
        <span className={styles.productLabel}>Kind</span>
        <div className={styles.toggleGroup} role="tablist" aria-label="touch kind">
          {(
            [
              ["ONE_TOUCH", "One-touch"],
              ["NO_TOUCH", "No-touch"],
              ["DOUBLE_NO_TOUCH", "Double-no-touch"],
              ["DOUBLE_ONE_TOUCH", "Double-one-touch"],
            ] as [TouchKind, string][]
          ).map(([tk, label]) => (
            <button
              key={tk}
              role="tab"
              aria-selected={touch.kind === tk}
              className={`${styles.modeTab} ${touch.kind === tk ? styles.modeActive : ""}`}
              onClick={() => onTouch({ ...touch, kind: tk })}
            >
              {label}
            </button>
          ))}
        </div>
      </div>
      <div className={styles.productRow}>
        <label className={styles.productField}>
          <span>{double ? "Lower" : "Barrier"}</span>
          <input
            className="num"
            type="number"
            min={0}
            step={step}
            value={touch.lowerBarrier}
            aria-label={double ? "lower barrier" : "barrier"}
            placeholder={(double ? spot * 0.95 : spot * 1.05).toFixed(pipDecimals)}
            onChange={(ev) =>
              onTouch({ ...touch, lowerBarrier: Math.max(0, Number(ev.target.value)) })
            }
          />
          <span>
            {touch.lowerBarrier > 0 ? "" : fmtRate(double ? spot * 0.95 : spot * 1.05, pipDecimals)}
          </span>
        </label>
        {double && (
          <label className={styles.productField}>
            <span>Upper</span>
            <input
              className="num"
              type="number"
              min={0}
              step={step}
              value={touch.upperBarrier}
              aria-label="upper barrier"
              placeholder={(spot * 1.05).toFixed(pipDecimals)}
              onChange={(ev) =>
                onTouch({ ...touch, upperBarrier: Math.max(0, Number(ev.target.value)) })
              }
            />
            <span>{touch.upperBarrier > 0 ? "" : fmtRate(spot * 1.05, pipDecimals)}</span>
          </label>
        )}
        <label className={styles.productField}>
          <span>Rebate</span>
          <input
            className="num"
            type="number"
            min={0}
            step={0.005}
            value={touch.rebate}
            aria-label="rebate"
            onChange={(ev) => onTouch({ ...touch, rebate: Math.max(0, Number(ev.target.value)) })}
          />
        </label>
      </div>
      <p className={styles.productNote}>
        {touch.kind === "ONE_TOUCH"
          ? "One-touch: pays the rebate (at hit) if the barrier IS touched before expiry."
          : touch.kind === "NO_TOUCH"
            ? "No-touch: pays the rebate at expiry if the barrier is NOT touched."
            : touch.kind === "DOUBLE_NO_TOUCH"
              ? "Double-no-touch: pays the rebate at expiry if NEITHER corridor barrier is touched."
              : "Double-one-touch: pays the rebate if EITHER corridor barrier is touched."}{" "}
        Priced by the reflection-principle first-passage / corridor-survival closed forms; one-touch
        + no-touch = the discounted rebate.
      </p>
    </div>
  );
}

/** The touch {@link ProductSpec}. */
export const touchSpec = defineProduct<TouchInputs>({
  id: "TOUCH",
  label: "Touch",
  group: "Barriers & digitals",
  assetClass: "FX",
  summary: "One-touch / no-touch (and their corridor variants) — a rebate on barrier (non-)contact.",
  keywords: [
    "touch",
    "one-touch",
    "no-touch",
    "double-no-touch",
    "double-one-touch",
    "dnt",
    "rebate",
    "barrier",
  ],
  kind: "touch",
  defaults: DEFAULT_TOUCH,
  allowedModels: bookingModelsFor("touch"),
  toInstrument: (inputs: TouchInputs, ctx): Instrument =>
    withTenorAndModel(
      touchInstrument(ctx.pair, ctx.tenorYears, ctx.notionalMm, touchTerms(inputs, ctx.spot)),
      ctx,
    ),
  InputBlock: TouchInputBlock,
});
