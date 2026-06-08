/**
 * American / Bermudan family ({@link ProductSpec}) — an early-exercise vanilla.
 * The trader picks call/put, a strike (ATMF-defaulted), the exercise style
 * (AMERICAN = continuous up to expiry, BERMUDAN = a discrete equally-spaced
 * schedule over `(0, T]`), and the engine (exact free-boundary FD vs
 * Longstaff-Schwartz LSM). Extracted verbatim from the former `TicketWorkspace`
 * monolith so the wire output is byte-identical.
 */
import type { ExerciseStyle, Instrument, OptionType } from "../data/contract";
import { americanInstrument, bookingModelsFor, type AmericanTerms } from "../data/seed";
import { fmtRate } from "../lib/format";
import styles from "../workspaces/TicketWorkspace.module.css";
import { defineProduct, withTenorAndModel, type InputBlockProps } from "./types";

/**
 * The American / Bermudan ticket inputs. `strike` `0` ⇒ default to the
 * ATM-forward level at build. `bermudanDates` is the number of equally-spaced
 * exercise dates over `(0, T]` (BERMUDAN only; expiry always exercisable).
 * `lsmPaths` `0` ⇒ the server's exact free-boundary FD engine; `> 0` ⇒ the
 * Longstaff-Schwartz Monte-Carlo engine; `lsmSeed` makes the LSM reproducible.
 */
export interface AmericanInputs {
  optionType: OptionType;
  /** Strike as an absolute level; `0` ⇒ default to the ATM-forward level. */
  strike: number;
  exerciseStyle: ExerciseStyle;
  /** Number of equally-spaced BERMUDAN exercise dates over `(0, T]`; `≥ 1`. */
  bermudanDates: number;
  /** Longstaff-Schwartz path count: `0` ⇒ the exact FD engine; `> 0` ⇒ LSM. */
  lsmPaths: number;
  /** Sobol scramble seed for the LSM engine (reproducible; ignored for FD). */
  lsmSeed: bigint;
}

export const DEFAULT_AMERICAN: AmericanInputs = {
  optionType: "PUT",
  strike: 0,
  exerciseStyle: "AMERICAN",
  bermudanDates: 4,
  lsmPaths: 0,
  lsmSeed: 0xa3_e1n,
};

/**
 * Build `AmericanTerms` from the inputs (strike ATMF-defaulted). For BERMUDAN the
 * `bermudanDates` count becomes an equally-spaced schedule `t_k = k·T/n` over
 * `(0, T]` (expiry inclusive — `k = n` lands on `T`); for AMERICAN the date set is
 * empty (continuous exercise). `lsmExerciseDates` is left `0` (the server default
 * resolution) — the AMERICAN exercise opportunities are an engine concern, not a
 * trader input here.
 */
export function americanTerms(
  a: AmericanInputs,
  atmForward: number,
  expiryYears: number,
): AmericanTerms {
  const strike = a.strike > 0 ? a.strike : atmForward;
  let bermudanDates: number[] = [];
  if (a.exerciseStyle === "BERMUDAN") {
    const n = Math.max(1, Math.trunc(a.bermudanDates));
    bermudanDates = [];
    for (let k = 1; k <= n; k += 1) bermudanDates.push((expiryYears * k) / n);
  }
  return {
    optionType: a.optionType,
    strike,
    exerciseStyle: a.exerciseStyle,
    bermudanDates,
    lsmPaths: Math.max(0, Math.trunc(a.lsmPaths)),
    lsmExerciseDates: 0,
    lsmSeed: a.lsmSeed,
  };
}

function OptionToggle(props: { value: OptionType; onChange: (next: OptionType) => void }) {
  return (
    <div className={styles.toggleGroup} role="tablist" aria-label="option type">
      {(["CALL", "PUT"] as OptionType[]).map((ot) => (
        <button
          key={ot}
          role="tab"
          aria-selected={props.value === ot}
          className={`${styles.modeTab} ${props.value === ot ? styles.modeActive : ""}`}
          onClick={() => props.onChange(ot)}
        >
          {ot === "CALL" ? "Call" : "Put"}
        </button>
      ))}
    </div>
  );
}

function AmericanInputBlock({
  value: american,
  onChange: onAmerican,
  ctx,
}: InputBlockProps<AmericanInputs>) {
  const { atmForward, pipDecimals } = ctx;
  const step = Math.pow(10, -pipDecimals);
  const isLsm = american.lsmPaths > 0;
  return (
    <div className={styles.product}>
      <div className={styles.productRow}>
        <span className={styles.productLabel}>Option</span>
        <OptionToggle
          value={american.optionType}
          onChange={(ot) => onAmerican({ ...american, optionType: ot })}
        />
        <label className={styles.productField}>
          <span>Strike</span>
          <input
            className="num"
            type="number"
            min={0}
            step={step}
            value={american.strike}
            aria-label="strike"
            placeholder={atmForward.toFixed(pipDecimals)}
            onChange={(ev) =>
              onAmerican({ ...american, strike: Math.max(0, Number(ev.target.value)) })
            }
          />
          <span>{american.strike > 0 ? "" : `ATMF ${fmtRate(atmForward, pipDecimals)}`}</span>
        </label>
      </div>
      <div className={styles.productRow}>
        <span className={styles.productLabel}>Exercise</span>
        <div className={styles.toggleGroup} role="tablist" aria-label="exercise style">
          {(["AMERICAN", "BERMUDAN"] as ExerciseStyle[]).map((es) => (
            <button
              key={es}
              role="tab"
              aria-selected={american.exerciseStyle === es}
              className={`${styles.modeTab} ${american.exerciseStyle === es ? styles.modeActive : ""}`}
              onClick={() => onAmerican({ ...american, exerciseStyle: es })}
            >
              {es === "AMERICAN" ? "American" : "Bermudan"}
            </button>
          ))}
        </div>
        {american.exerciseStyle === "BERMUDAN" && (
          <label className={styles.productField}>
            <span>Exercise dates</span>
            <input
              className="num"
              type="number"
              min={1}
              step={1}
              value={american.bermudanDates}
              aria-label="bermudan dates"
              onChange={(ev) =>
                onAmerican({
                  ...american,
                  bermudanDates: Math.max(1, Math.trunc(Number(ev.target.value))),
                })
              }
            />
            <span>over (0, T]</span>
          </label>
        )}
      </div>
      <div className={styles.productRow}>
        <span className={styles.productLabel}>Engine</span>
        <label className={styles.productField}>
          <span>LSM paths</span>
          <input
            className="num"
            type="number"
            min={0}
            step={1000}
            value={american.lsmPaths}
            aria-label="lsm paths"
            onChange={(ev) =>
              onAmerican({ ...american, lsmPaths: Math.max(0, Math.trunc(Number(ev.target.value))) })
            }
          />
          <span>{isLsm ? "Longstaff-Schwartz MC" : "exact FD"}</span>
        </label>
        {isLsm && (
          <label className={styles.productField}>
            <span>LSM seed</span>
            <input
              className="num"
              type="number"
              min={0}
              step={1}
              value={Number(american.lsmSeed)}
              aria-label="lsm seed"
              onChange={(ev) =>
                onAmerican({
                  ...american,
                  lsmSeed: BigInt(Math.max(0, Math.trunc(Number(ev.target.value)))),
                })
              }
            />
          </label>
        )}
      </div>
      <p className={styles.productNote}>
        {american.exerciseStyle === "AMERICAN" ? "American" : "Bermudan"}{" "}
        {american.optionType === "CALL" ? "call" : "put"}: an early-exercise vanilla
        {american.exerciseStyle === "AMERICAN"
          ? " exercisable continuously up to expiry"
          : ` exercisable on ${Math.max(1, Math.trunc(american.bermudanDates))} equally-spaced dates (expiry inclusive)`}
        . The offline build prices a genuine binomial tree; the server prices{" "}
        {isLsm
          ? "the Longstaff-Schwartz regression Monte-Carlo (reports a standard error)"
          : "the exact projected-SOR free-boundary finite difference (no standard error)"}
        .
      </p>
    </div>
  );
}

/** The American / Bermudan {@link ProductSpec}. */
export const americanSpec = defineProduct<AmericanInputs>({
  id: "AMERICAN",
  label: "American / Bermudan",
  group: "Path-dependent",
  assetClass: "FX",
  summary: "Early-exercise vanilla — exercisable continuously (American) or on a date set (Bermudan).",
  keywords: [
    "american",
    "bermudan",
    "early exercise",
    "free boundary",
    "longstaff",
    "schwartz",
    "lsm",
  ],
  kind: "american",
  defaults: DEFAULT_AMERICAN,
  allowedModels: bookingModelsFor("american"),
  toInstrument: (inputs: AmericanInputs, ctx): Instrument =>
    withTenorAndModel(
      americanInstrument(
        ctx.pair,
        ctx.tenorYears,
        ctx.notionalMm,
        americanTerms(inputs, ctx.atmForward, ctx.tenorYears),
      ),
      ctx,
    ),
  InputBlock: AmericanInputBlock,
});
