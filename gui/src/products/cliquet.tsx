/**
 * Cliquet / ratchet — a legless {@link ProductSpec} (GW2). A plain ratchet prices
 * in closed form (sum of forward-start legs); any local cap/floor switches to
 * antithetic Monte-Carlo (a standard error is surfaced). Extracted verbatim from
 * the former `TicketWorkspace` monolith so the wire output is byte-identical.
 */
import type { Instrument, OptionType } from "../data/contract";
import { cliquetInstrument, isPlainCliquet, type CliquetTerms } from "../data/seed";
import { bookingModelsFor } from "../data/seed";
import styles from "../workspaces/TicketWorkspace.module.css";
import { defineProduct, withTenorAndModel, type InputBlockProps } from "./types";

/**
 * The cliquet ticket inputs. The local cap/floor are presence-tracked via
 * `useCap`/`useFloor` toggles; any clamp on switches the pricer to Monte-Carlo
 * (the build surfaces the standard error). `mcPairs`/`mcSeed` tune the clamped MC
 * and are ignored for a plain (unclamped) ratchet.
 */
export interface CliquetInputs {
  optionType: OptionType;
  moneyness: number;
  periods: number;
  useLocalCap: boolean;
  localCap: number;
  useLocalFloor: boolean;
  localFloor: number;
  mcPairs: number;
  mcSeed: bigint;
}

/** The default cliquet inputs at first render (a plain 4-period ratchet). */
export const DEFAULT_CLIQUET: CliquetInputs = {
  optionType: "CALL",
  moneyness: 1,
  periods: 4,
  useLocalCap: false,
  localCap: 0.05,
  useLocalFloor: false,
  localFloor: 0,
  mcPairs: 0,
  mcSeed: 0xc11_c0e7n,
};

/** Build `CliquetTerms` from the inputs (clamps omitted unless their toggle is on). */
export function cliquetTerms(c: CliquetInputs): CliquetTerms {
  const terms: CliquetTerms = {
    optionType: c.optionType,
    moneyness: c.moneyness,
    periods: Math.max(1, Math.trunc(c.periods)),
    mcPairs: Math.max(0, Math.trunc(c.mcPairs)),
    mcSeed: c.mcSeed,
  };
  if (c.useLocalCap) terms.localCap = c.localCap;
  if (c.useLocalFloor) terms.localFloor = c.localFloor;
  return terms;
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

function CliquetInputBlock({ value, onChange }: InputBlockProps<CliquetInputs>) {
  const plain = isPlainCliquet(cliquetTerms(value));
  return (
    <div className={styles.product}>
      <div className={styles.productRow}>
        <span className={styles.productLabel}>Option</span>
        <OptionToggle
          value={value.optionType}
          onChange={(ot) => onChange({ ...value, optionType: ot })}
        />
        <label className={styles.productField}>
          <span>Per-period m</span>
          <input
            className="num"
            type="number"
            min={0}
            step={0.01}
            value={value.moneyness}
            aria-label="moneyness"
            onChange={(ev) =>
              onChange({ ...value, moneyness: Math.max(0, Number(ev.target.value)) })
            }
          />
        </label>
        <label className={styles.productField}>
          <span>Periods</span>
          <input
            className="num"
            type="number"
            min={1}
            step={1}
            value={value.periods}
            aria-label="periods"
            onChange={(ev) =>
              onChange({ ...value, periods: Math.max(1, Math.trunc(Number(ev.target.value))) })
            }
          />
        </label>
      </div>
      <div className={styles.productRow}>
        <span className={styles.productLabel}>Local clamp</span>
        <label className={styles.productField}>
          <input
            type="checkbox"
            checked={value.useLocalCap}
            aria-label="use local cap"
            onChange={(ev) => onChange({ ...value, useLocalCap: ev.target.checked })}
          />
          <span>Cap</span>
          <input
            className="num"
            type="number"
            min={0}
            step={0.005}
            value={value.localCap}
            disabled={!value.useLocalCap}
            aria-label="local cap"
            onChange={(ev) =>
              onChange({ ...value, localCap: Math.max(0, Number(ev.target.value)) })
            }
          />
        </label>
        <label className={styles.productField}>
          <input
            type="checkbox"
            checked={value.useLocalFloor}
            aria-label="use local floor"
            onChange={(ev) => onChange({ ...value, useLocalFloor: ev.target.checked })}
          />
          <span>Floor</span>
          <input
            className="num"
            type="number"
            min={0}
            step={0.005}
            value={value.localFloor}
            disabled={!value.useLocalFloor}
            aria-label="local floor"
            onChange={(ev) =>
              onChange({ ...value, localFloor: Math.max(0, Number(ev.target.value)) })
            }
          />
        </label>
      </div>
      {!plain && (
        <div className={styles.productRow}>
          <span className={styles.productLabel}>MC pairs</span>
          <label className={styles.productField}>
            <input
              className="num"
              type="number"
              min={0}
              step={1000}
              value={value.mcPairs}
              aria-label="mc pairs"
              onChange={(ev) =>
                onChange({ ...value, mcPairs: Math.max(0, Math.trunc(Number(ev.target.value))) })
              }
            />
            <span>{value.mcPairs > 0 ? "pairs" : "default"}</span>
          </label>
        </div>
      )}
      <p className={styles.productNote}>
        {plain
          ? "Plain ratchet: priced in closed form as the exact sum of forward-start legs (no Monte-Carlo error)."
          : "Clamped cliquet: a local cap/floor has no closed form, so it is priced by antithetic Monte-Carlo and reports a standard error alongside the price."}
      </p>
    </div>
  );
}

/** The cliquet {@link ProductSpec}. */
export const cliquetSpec = defineProduct<CliquetInputs>({
  id: "CLIQUET",
  label: "Cliquet",
  group: "Path-dependent",
  assetClass: "FX",
  summary: "Cliquet / ratchet — periodically-resetting forward-start legs, optionally locally clamped.",
  keywords: ["cliquet", "ratchet", "reset", "local cap", "local floor"],
  kind: "cliquet",
  defaults: DEFAULT_CLIQUET,
  allowedModels: bookingModelsFor("cliquet"),
  toInstrument: (inputs: CliquetInputs, ctx): Instrument =>
    withTenorAndModel(
      cliquetInstrument(ctx.pair, ctx.tenorYears, ctx.notionalMm, cliquetTerms(inputs)),
      ctx,
    ),
  InputBlock: CliquetInputBlock,
});
