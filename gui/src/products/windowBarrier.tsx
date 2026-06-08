/**
 * Window-barrier family ({@link ProductSpec}) — an up/down-and-out vanilla whose
 * barrier is active only during a sub-window of the option's life. It has no
 * closed form, so it is LOCAL_STOCH_VOL-only: the builder pre-selects that model
 * and `toInstrument` locks it via {@link withTenorAndModel}'s `lockedModel`.
 * Extracted verbatim from the former `TicketWorkspace` monolith so the wire
 * output (and the model lock) is byte-identical.
 */
import type { BarrierSide, Instrument, OptionType } from "../data/contract";
import {
  bookingModelsFor,
  windowBarrierInstrument,
  type WindowBarrierTerms,
} from "../data/seed";
import { fmtRate } from "../lib/format";
import styles from "../workspaces/TicketWorkspace.module.css";
import { defineProduct, withTenorAndModel, type InputBlockProps } from "./types";

/**
 * The window-barrier ticket inputs. `strike` 0 ⇒ default to the ATM-forward
 * level; the barrier defaults off spot on the chosen side; the active window
 * defaults to the middle half of the option's life. `mcPairs` 0 ⇒ the exact ADI
 * PDE (no std-error); `mcPairs > 0` ⇒ Monte-Carlo (surfaces a stderr).
 */
export interface WindowBarrierInputs {
  optionType: OptionType;
  /** Strike as an absolute level; `0` ⇒ default to the ATM-forward level. */
  strike: number;
  side: BarrierSide;
  /** The barrier level (absolute quote); `0` ⇒ a sensible default off spot. */
  barrier: number;
  /** Active-window open as a FRACTION of the option's life (`0 ≤ start < end ≤ 1`). */
  windowStartFrac: number;
  /** Active-window close as a fraction of the option's life. */
  windowEndFrac: number;
  mcPairs: number;
  mcSteps: number;
  mcSeed: bigint;
}

export const DEFAULT_WINDOW_BARRIER: WindowBarrierInputs = {
  optionType: "CALL",
  strike: 0,
  side: "UP",
  barrier: 0,
  windowStartFrac: 0.25,
  windowEndFrac: 0.75,
  mcPairs: 0,
  mcSteps: 64,
  mcSeed: 0x42n,
};

/**
 * Build `WindowBarrierTerms` from the inputs. The strike defaults to the
 * ATM-forward level; the barrier defaults off spot on the chosen side; the active
 * window fractions are clamped well-ordered (`0 ≤ start < end ≤ 1`) and scaled by
 * the expiry to absolute year fractions.
 */
export function windowBarrierTerms(
  w: WindowBarrierInputs,
  atmForward: number,
  spot: number,
  expiryYears: number,
): WindowBarrierTerms {
  const strike = w.strike > 0 ? w.strike : atmForward;
  const defaultBarrier = w.side === "UP" ? spot * 1.05 : spot * 0.95;
  const barrier = w.barrier > 0 ? w.barrier : defaultBarrier;
  let startFrac = Math.min(Math.max(w.windowStartFrac, 0), 1);
  let endFrac = Math.min(Math.max(w.windowEndFrac, 0), 1);
  // Keep the window strictly well-ordered (0 ≤ start < end ≤ 1) so the engine is
  // in domain; fall back to the middle half if the trader inverted them.
  if (startFrac >= endFrac) {
    startFrac = 0.25;
    endFrac = 0.75;
  }
  return {
    optionType: w.optionType,
    strike: { kind: "strike", strike },
    barrier,
    side: w.side,
    windowStart: startFrac * expiryYears,
    windowEnd: endFrac * expiryYears,
    mcPairs: Math.max(0, Math.trunc(w.mcPairs)),
    mcSteps: Math.max(0, Math.trunc(w.mcSteps)),
    mcSeed: w.mcSeed,
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

function WindowBarrierInputBlock({
  value: windowBarrier,
  onChange: onWindowBarrier,
  ctx,
}: InputBlockProps<WindowBarrierInputs>) {
  const { atmForward, spot, pipDecimals } = ctx;
  const step = Math.pow(10, -pipDecimals);
  return (
    <div className={styles.product}>
      <div className={styles.productRow}>
        <span className={styles.productLabel}>Option</span>
        <OptionToggle
          value={windowBarrier.optionType}
          onChange={(ot) => onWindowBarrier({ ...windowBarrier, optionType: ot })}
        />
        <label className={styles.productField}>
          <span>Strike</span>
          <input
            className="num"
            type="number"
            min={0}
            step={step}
            value={windowBarrier.strike}
            aria-label="strike"
            placeholder={atmForward.toFixed(pipDecimals)}
            onChange={(ev) =>
              onWindowBarrier({ ...windowBarrier, strike: Math.max(0, Number(ev.target.value)) })
            }
          />
          <span>{windowBarrier.strike > 0 ? "" : `ATMF ${fmtRate(atmForward, pipDecimals)}`}</span>
        </label>
      </div>
      <div className={styles.productRow}>
        <span className={styles.productLabel}>Knock-out</span>
        <div className={styles.toggleGroup} role="tablist" aria-label="barrier side">
          {(["UP", "DOWN"] as BarrierSide[]).map((bs) => (
            <button
              key={bs}
              role="tab"
              aria-selected={windowBarrier.side === bs}
              className={`${styles.modeTab} ${windowBarrier.side === bs ? styles.modeActive : ""}`}
              onClick={() => onWindowBarrier({ ...windowBarrier, side: bs })}
            >
              {bs === "UP" ? "Up" : "Down"}
            </button>
          ))}
        </div>
        <label className={styles.productField}>
          <span>Barrier</span>
          <input
            className="num"
            type="number"
            min={0}
            step={step}
            value={windowBarrier.barrier}
            aria-label="barrier"
            placeholder={(windowBarrier.side === "UP" ? spot * 1.05 : spot * 0.95).toFixed(
              pipDecimals,
            )}
            onChange={(ev) =>
              onWindowBarrier({ ...windowBarrier, barrier: Math.max(0, Number(ev.target.value)) })
            }
          />
          <span>
            {windowBarrier.barrier > 0
              ? ""
              : fmtRate(windowBarrier.side === "UP" ? spot * 1.05 : spot * 0.95, pipDecimals)}
          </span>
        </label>
      </div>
      <div className={styles.productRow}>
        <label className={styles.productField}>
          <span>Window start</span>
          <input
            className="num"
            type="number"
            min={0}
            max={1}
            step={0.05}
            value={windowBarrier.windowStartFrac}
            aria-label="window start"
            onChange={(ev) =>
              onWindowBarrier({
                ...windowBarrier,
                windowStartFrac: Math.min(1, Math.max(0, Number(ev.target.value))),
              })
            }
          />
          <span>·T</span>
        </label>
        <label className={styles.productField}>
          <span>Window end</span>
          <input
            className="num"
            type="number"
            min={0}
            max={1}
            step={0.05}
            value={windowBarrier.windowEndFrac}
            aria-label="window end"
            onChange={(ev) =>
              onWindowBarrier({
                ...windowBarrier,
                windowEndFrac: Math.min(1, Math.max(0, Number(ev.target.value))),
              })
            }
          />
          <span>·T</span>
        </label>
        <label className={styles.productField}>
          <span>MC pairs</span>
          <input
            className="num"
            type="number"
            min={0}
            step={1000}
            value={windowBarrier.mcPairs}
            aria-label="mc pairs"
            onChange={(ev) =>
              onWindowBarrier({
                ...windowBarrier,
                mcPairs: Math.max(0, Math.trunc(Number(ev.target.value))),
              })
            }
          />
          <span>{windowBarrier.mcPairs > 0 ? "pairs" : "PDE"}</span>
        </label>
      </div>
      <p className={styles.productNote}>
        Window barrier: a {windowBarrier.side === "UP" ? "up" : "down"}-and-out{" "}
        {windowBarrier.optionType === "CALL" ? "call" : "put"} whose barrier is active only during
        the window [{windowBarrier.windowStartFrac.toFixed(2)}·T, {windowBarrier.windowEndFrac.toFixed(2)}·T].
        No closed form — priced server-side on the local-stochastic-volatility engine:{" "}
        {windowBarrier.mcPairs > 0
          ? "Monte-Carlo (reports a standard error)"
          : "the exact ADI PDE (no standard error)"}
        .
      </p>
    </div>
  );
}

/** The window-barrier {@link ProductSpec} (LOCAL_STOCH_VOL-locked). */
export const windowBarrierSpec = defineProduct<WindowBarrierInputs>({
  id: "WINDOW_BARRIER",
  label: "Window Barrier",
  group: "Barriers & digitals",
  assetClass: "FX",
  summary: "Up/down-and-out vanilla whose barrier is active only within a sub-window of its life.",
  keywords: ["window", "barrier", "knock-out", "partial barrier", "local stochastic volatility", "lsv"],
  kind: "windowBarrier",
  defaults: DEFAULT_WINDOW_BARRIER,
  allowedModels: bookingModelsFor("windowBarrier"),
  toInstrument: (inputs: WindowBarrierInputs, ctx): Instrument =>
    withTenorAndModel(
      windowBarrierInstrument(
        ctx.pair,
        ctx.tenorYears,
        ctx.notionalMm,
        windowBarrierTerms(inputs, ctx.atmForward, ctx.spot, ctx.tenorYears),
      ),
      ctx,
      "LOCAL_STOCH_VOL",
    ),
  InputBlock: WindowBarrierInputBlock,
});
