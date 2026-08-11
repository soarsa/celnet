/**
 * Forward-start vanilla — a legless {@link ProductSpec} (GW2). The strike fixes at
 * t₁ to m·S(t₁); priced by the FX dual-carry closed form. Extracted verbatim from
 * the former `TicketWorkspace` monolith so the wire output is byte-identical.
 */
import type { Instrument, OptionType } from "../data/contract";
import { forwardStartInstrument, type ForwardStartTerms } from "../data/seed";
import { bookingModelsFor } from "../data/seed";
import styles from "../workspaces/TicketWorkspace.module.css";
import { defineProduct, withTenorAndModel, type InputBlockProps } from "./types";

import { NumberField } from "../components/NumberField";

/** The forward-start ticket inputs (option type / reset-moneyness / reset date). */
export interface ForwardStartInputs {
  optionType: OptionType;
  /** Strike-reset multiple `m` (`m = 1` is the ATM-forward reset). */
  moneyness: number;
  /** Reset (strike-fixing) date `t₁` in years (clamped to `[0, expiry]` at build). */
  reset: number;
}

/** The default forward-start inputs at first render. */
export const DEFAULT_FORWARD_START: ForwardStartInputs = {
  optionType: "CALL",
  moneyness: 1,
  reset: 0.25,
};

/** Build `ForwardStartTerms` from the inputs, clamping reset into `[0, expiry]`. */
export function forwardStartTerms(
  f: ForwardStartInputs,
  expiryYears: number,
): ForwardStartTerms {
  return {
    optionType: f.optionType,
    moneyness: f.moneyness,
    reset: Math.min(Math.max(f.reset, 0), expiryYears),
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

function ForwardStartInputBlock({ value, onChange }: InputBlockProps<ForwardStartInputs>) {
  return (
    <div className={styles.product}>
      <div className={styles.productRow}>
        <span className={styles.productLabel}>Option</span>
        <OptionToggle
          value={value.optionType}
          onChange={(ot) => onChange({ ...value, optionType: ot })}
        />
        <label className={styles.productField}>
          <span>Reset m</span>
          <NumberField
            className="num"
            min={0}
            step={0.01}
            value={value.moneyness}
            aria-label="moneyness"
            onChange={(ev) =>
              onChange({ ...value, moneyness: Math.max(0, Number(ev.target.value)) })
            }
          />
          <span>×S(t₁)</span>
        </label>
        <label className={styles.productField}>
          <span>Reset t₁</span>
          <NumberField
            className="num"
            min={0}
            step={0.05}
            value={value.reset}
            aria-label="reset"
            onChange={(ev) =>
              onChange({ ...value, reset: Math.max(0, Number(ev.target.value)) })
            }
          />
          <span>y</span>
        </label>
      </div>
      <p className={styles.productNote}>
        Forward-start vanilla: the strike fixes at t₁ to m·S(t₁). Priced by the FX dual-carry
        closed form V = e^(−r_f·t₁)·S₀·u(m, T−t₁); at t₁→0 it is a plain vanilla struck at m·S₀.
      </p>
    </div>
  );
}

/** The forward-start {@link ProductSpec}. */
export const forwardStartSpec = defineProduct<ForwardStartInputs>({
  id: "FORWARD_START",
  label: "Forward Start",
  group: "Path-dependent",
  assetClass: "FX",
  summary: "Forward-start vanilla — the strike fixes at a future reset date to m·S(t₁).",
  keywords: ["forward start", "forward-start", "reset", "strike reset", "rubinstein"],
  kind: "forwardStart",
  defaults: DEFAULT_FORWARD_START,
  allowedModels: bookingModelsFor("forwardStart"),
  toInstrument: (inputs: ForwardStartInputs, ctx): Instrument =>
    withTenorAndModel(
      forwardStartInstrument(
        ctx.pair,
        ctx.tenorYears,
        ctx.notionalMm,
        forwardStartTerms(inputs, ctx.tenorYears),
      ),
      ctx,
    ),
  InputBlock: ForwardStartInputBlock,
});
