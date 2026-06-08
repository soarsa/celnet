/**
 * Target-Redemption Forward (TARF) — a {@link ProductSpec} (GW2). A strip of
 * geared fixings accruing client gains until the cumulative target redeems the
 * structure; always Monte-Carlo, so the build surfaces a standard error.
 * Extracted verbatim from the former `TicketWorkspace` monolith so the wire
 * output is byte-identical.
 */
import type { Instrument, OptionType, TarfRedemption } from "../data/contract";
import { bookingModelsFor, equalFixingSchedule, tarfInstrument, type TarfTerms } from "../data/seed";
import { fmtRate } from "../lib/format";
import styles from "../workspaces/TicketWorkspace.module.css";
import { defineProduct, withTenorAndModel, type InputBlockProps } from "./types";

/**
 * The TARF ticket inputs (favourable side / strike / target / leverage / gap-risk
 * redemption / fixing count / MC). A TARF is always Monte-Carlo, so the build
 * always surfaces a standard error. `strike` 0 ⇒ default to the ATM-forward level.
 */
export interface TarfInputs {
  optionType: OptionType;
  strike: number;
  target: number;
  leverage: number;
  redemption: TarfRedemption;
  fixings: number;
  mcPairs: number;
  mcSeed: bigint;
}

export const DEFAULT_TARF: TarfInputs = {
  optionType: "PUT",
  strike: 0,
  target: 0.1,
  leverage: 2,
  redemption: "FULL_GAIN",
  fixings: 12,
  mcPairs: 0,
  mcSeed: 0x7a_2fn,
};

/** Build `TarfTerms` from the inputs (equally-spaced schedule; strike ATMF-defaulted). */
export function tarfTerms(t: TarfInputs, atmForward: number, expiryYears: number): TarfTerms {
  const fixings = Math.max(1, Math.trunc(t.fixings));
  return {
    optionType: t.optionType,
    strike: t.strike > 0 ? t.strike : atmForward,
    target: Math.max(0, t.target),
    leverage: Math.max(0, t.leverage),
    redemption: t.redemption,
    schedule: equalFixingSchedule(fixings, expiryYears, 1),
    mcPairs: Math.max(0, Math.trunc(t.mcPairs)),
    mcSeed: t.mcSeed,
  };
}

function TarfInputBlock({ value: tarf, onChange, ctx }: InputBlockProps<TarfInputs>) {
  const { atmForward, pipDecimals } = ctx;
  const onTarf = onChange;
  return (
    <div className={styles.product}>
      <div className={styles.productRow}>
        <span className={styles.productLabel}>Gain side</span>
        <div className={styles.toggleGroup} role="tablist" aria-label="option type">
          {(["CALL", "PUT"] as OptionType[]).map((ot) => (
            <button
              key={ot}
              role="tab"
              aria-selected={tarf.optionType === ot}
              className={`${styles.modeTab} ${tarf.optionType === ot ? styles.modeActive : ""}`}
              onClick={() => onTarf({ ...tarf, optionType: ot })}
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
            value={tarf.strike}
            aria-label="strike"
            placeholder={atmForward.toFixed(pipDecimals)}
            onChange={(ev) => onTarf({ ...tarf, strike: Math.max(0, Number(ev.target.value)) })}
          />
          <span>{tarf.strike > 0 ? "" : `ATMF ${fmtRate(atmForward, pipDecimals)}`}</span>
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
            value={tarf.target}
            aria-label="target"
            onChange={(ev) => onTarf({ ...tarf, target: Math.max(0, Number(ev.target.value)) })}
          />
        </label>
        <label className={styles.productField}>
          <span>Leverage</span>
          <input
            className="num"
            type="number"
            min={0}
            step={0.5}
            value={tarf.leverage}
            aria-label="leverage"
            onChange={(ev) => onTarf({ ...tarf, leverage: Math.max(0, Number(ev.target.value)) })}
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
            value={tarf.fixings}
            aria-label="fixings"
            onChange={(ev) =>
              onTarf({ ...tarf, fixings: Math.max(1, Math.trunc(Number(ev.target.value))) })
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
              aria-selected={tarf.redemption === r}
              className={`${styles.modeTab} ${tarf.redemption === r ? styles.modeActive : ""}`}
              onClick={() => onTarf({ ...tarf, redemption: r })}
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
            value={tarf.mcPairs}
            aria-label="mc pairs"
            onChange={(ev) =>
              onTarf({ ...tarf, mcPairs: Math.max(0, Math.trunc(Number(ev.target.value))) })
            }
          />
          <span>{tarf.mcPairs > 0 ? "pairs" : "default"}</span>
        </label>
      </div>
      <p className={styles.productNote}>
        Target-Redemption Forward: a strip of geared fixings accruing client gains until the
        cumulative target redeems the structure (full-gain carries the gap risk, capped-gain does
        not). Priced by antithetic Monte-Carlo (bank present value) — it reports a standard error.
      </p>
    </div>
  );
}

/** The TARF {@link ProductSpec}. */
export const tarfSpec = defineProduct<TarfInputs>({
  id: "TARF",
  label: "TARF",
  group: "Structured",
  assetClass: "FX",
  summary:
    "Target-Redemption Forward — a strip of geared fixings that redeems on a cumulative client-gain target.",
  keywords: ["tarf", "target redemption", "forward", "geared", "redemption", "accrual"],
  kind: "tarf",
  defaults: DEFAULT_TARF,
  allowedModels: bookingModelsFor("tarf"),
  toInstrument: (inputs: TarfInputs, ctx): Instrument =>
    withTenorAndModel(
      tarfInstrument(ctx.pair, ctx.tenorYears, ctx.notionalMm, tarfTerms(inputs, ctx.atmForward, ctx.tenorYears)),
      ctx,
    ),
  InputBlock: TarfInputBlock,
});
