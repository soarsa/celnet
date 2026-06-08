/**
 * The vanilla + multi-leg strategy family (GW2) — the leg-ladder, last to migrate
 * out of the former `TicketWorkspace` monolith. Extracted verbatim so the wire
 * output is byte-identical: the monolith built these from fixed templates
 * (`vanillaInstrument(pair, t, "CALL", 0.25, mm)` for the lone vanilla; the
 * `strategyLegs(kind)` ladder for each strategy) and rendered the resulting legs
 * as a READ-ONLY ladder — the strikes are convention deltas, resolved to levels
 * server-side. We preserve exactly that: the legs are template-fixed, NOT freely
 * edited, so the inputs carry only the template identity and the block exposes
 * what the monolith exposed (a labelled ladder + the zero-cost solve affordance),
 * inventing no editability the monolith lacks.
 */
import type { Instrument, Leg, StrategyKind } from "../data/contract";
import { strategyInstrument, vanillaInstrument } from "../data/seed";
import styles from "../workspaces/TicketWorkspace.module.css";
import { defineProduct, withTenorAndModel, type InputBlockProps } from "./types";

/**
 * The vanilla / strategy ticket inputs. The legs are template-fixed (derived from
 * the structure identity, exactly as the monolith's `buildInstrument`), so the
 * editable surface is the template selector itself — `"VANILLA"` for the lone
 * 25Δ call, or a {@link StrategyKind} for the four multi-leg templates. There is
 * no per-leg editing in the monolith, so none is exposed here.
 */
export type StrategyInputs =
  | { template: "VANILLA" }
  | { template: StrategyKind };

/** Default inputs for the lone vanilla (the monolith's hardcoded 25Δ call). */
export const DEFAULT_VANILLA: StrategyInputs = { template: "VANILLA" };
/** Default inputs for the risk-reversal template. */
export const DEFAULT_RISK_REVERSAL: StrategyInputs = { template: "RISK_REVERSAL" };
/** Default inputs for the strangle template. */
export const DEFAULT_STRANGLE: StrategyInputs = { template: "STRANGLE" };
/** Default inputs for the straddle template. */
export const DEFAULT_STRADDLE: StrategyInputs = { template: "STRADDLE" };
/** Default inputs for the seagull template. */
export const DEFAULT_SEAGULL: StrategyInputs = { template: "SEAGULL" };

/**
 * Build the wire instrument for a vanilla / strategy template, reproducing the
 * monolith's `buildInstrument` VANILLA + strategy branches byte-for-byte: the
 * lone vanilla is the fixed 25Δ (`delta: 0.25`) call; every strategy delegates to
 * `strategyInstrument(pair, t, kind, mm)` (its `strategyLegs(kind)` ladder).
 */
function strategyBase(
  inputs: StrategyInputs,
  pair: { base: string; quote: string },
  tenorYears: number,
  notionalMm: number,
): Instrument {
  if (inputs.template === "VANILLA") {
    return vanillaInstrument(pair, tenorYears, "CALL", 0.25, notionalMm);
  }
  return strategyInstrument(pair, tenorYears, inputs.template, notionalMm);
}

/** Extract the option legs the wire instrument carries, for the read-only ladder. */
function instrumentLegs(instrument: Instrument): Leg[] {
  if (instrument.product.kind === "vanilla") {
    const v = instrument.product.vanilla;
    return [{ optionType: v.optionType, strike: v.strike, side: "BUY", ratio: 1 }];
  }
  if (instrument.product.kind === "strategy") {
    return instrument.product.strategy.legs;
  }
  return [];
}

/** Trader-facing delta label for a leg strike (e.g. `25Δ`, or `abs` for a level). */
function legDeltaLabel(leg: Leg): string {
  return leg.strike.kind === "delta"
    ? `${Math.round(Math.abs(leg.strike.delta) * 100)}Δ`
    : "abs";
}

/**
 * The leg-ladder input block. The legs are template-fixed (the monolith never
 * edited them inline — it rendered the derived ladder and offered an inline
 * zero-cost strike solve), so this is a faithful read-only ladder: side / type /
 * convention-delta per leg. Strikes resolve to levels server-side against the
 * marked surface, so the ladder honestly shows the convention delta rather than a
 * fabricated client-side level.
 */
function StrategyInputBlock({ value, ctx }: InputBlockProps<StrategyInputs>) {
  const instrument = strategyBase(value, ctx.pair, ctx.tenorYears, ctx.notionalMm);
  const legs = instrumentLegs(instrument);
  const isVanilla = value.template === "VANILLA";
  return (
    <div className={styles.legs}>
      <ul className={styles.legs} role="list" aria-label="strategy legs">
        {legs.map((leg, i) => (
          <li className={styles.leg} key={i} role="listitem">
            <span className={styles.legNo}>LEG {i + 1}</span>
            <span
              className={`${styles.legSide} ${leg.side === "SELL" ? styles.sell : styles.buy}`}
            >
              {leg.side === "SELL" ? "SELL" : "BUY"}
            </span>
            <span className={styles.legType}>{leg.optionType === "CALL" ? "Call" : "Put"}</span>
            <span className={`num ${styles.legDelta}`}>{legDeltaLabel(leg)}</span>
            <span className={styles.legArrow} aria-hidden="true">
              ▸
            </span>
            <span className={`num ${styles.legStrike}`} aria-label="strike">
              K —
            </span>
          </li>
        ))}
      </ul>
      <p className={styles.productNote}>
        {isVanilla
          ? "Vanilla — a single 25Δ call. Strike resolves to a level server-side against the marked surface."
          : "Template strategy — fixed convention-delta legs. Strikes resolve to levels server-side against the marked surface."}
      </p>
    </div>
  );
}

/** The lone vanilla {@link ProductSpec} (the monolith's hardcoded 25Δ call). */
export const vanillaSpec = defineProduct<StrategyInputs>({
  id: "VANILLA",
  label: "Vanilla",
  group: "Vanilla & strategies",
  assetClass: "FX",
  summary: "Single-leg European vanilla — a 25Δ call against the marked surface.",
  keywords: ["vanilla", "european", "call", "put", "single leg", "25 delta"],
  kind: "vanilla",
  defaults: DEFAULT_VANILLA,
  allowedModels: ["DEFAULT", "LOCAL_STOCH_VOL"],
  toInstrument: (inputs: StrategyInputs, ctx): Instrument =>
    withTenorAndModel(strategyBase(inputs, ctx.pair, ctx.tenorYears, ctx.notionalMm), ctx),
  InputBlock: StrategyInputBlock,
});

/** The risk-reversal {@link ProductSpec} (25Δ call vs 25Δ put). */
export const riskReversalSpec = defineProduct<StrategyInputs>({
  id: "RISK_REVERSAL",
  label: "Risk Reversal",
  group: "Vanilla & strategies",
  assetClass: "FX",
  summary: "Long 25Δ call vs short 25Δ put — the smile-skew structure.",
  keywords: ["risk reversal", "rr", "skew", "collar", "25 delta"],
  kind: "strategy",
  defaults: DEFAULT_RISK_REVERSAL,
  allowedModels: ["DEFAULT"],
  toInstrument: (inputs: StrategyInputs, ctx): Instrument =>
    withTenorAndModel(strategyBase(inputs, ctx.pair, ctx.tenorYears, ctx.notionalMm), ctx),
  InputBlock: StrategyInputBlock,
});

/** The strangle {@link ProductSpec} (long 10Δ call + long 10Δ put). */
export const strangleSpec = defineProduct<StrategyInputs>({
  id: "STRANGLE",
  label: "Strangle",
  group: "Vanilla & strategies",
  assetClass: "FX",
  summary: "Long 10Δ call + long 10Δ put — the smile-convexity (wing) structure.",
  keywords: ["strangle", "wings", "convexity", "butterfly", "10 delta"],
  kind: "strategy",
  defaults: DEFAULT_STRANGLE,
  allowedModels: ["DEFAULT"],
  toInstrument: (inputs: StrategyInputs, ctx): Instrument =>
    withTenorAndModel(strategyBase(inputs, ctx.pair, ctx.tenorYears, ctx.notionalMm), ctx),
  InputBlock: StrategyInputBlock,
});

/** The straddle {@link ProductSpec} (long ATM call + long ATM put). */
export const straddleSpec = defineProduct<StrategyInputs>({
  id: "STRADDLE",
  label: "Straddle",
  group: "Vanilla & strategies",
  assetClass: "FX",
  summary: "Long ATM call + long ATM put — the at-the-money volatility structure.",
  keywords: ["straddle", "atm", "volatility", "vega", "50 delta"],
  kind: "strategy",
  defaults: DEFAULT_STRADDLE,
  allowedModels: ["DEFAULT"],
  toInstrument: (inputs: StrategyInputs, ctx): Instrument =>
    withTenorAndModel(strategyBase(inputs, ctx.pair, ctx.tenorYears, ctx.notionalMm), ctx),
  InputBlock: StrategyInputBlock,
});

/** The seagull {@link ProductSpec} (long 25Δ call / short 10Δ call / short 25Δ put). */
export const seagullSpec = defineProduct<StrategyInputs>({
  id: "SEAGULL",
  label: "Seagull",
  group: "Vanilla & strategies",
  assetClass: "FX",
  summary: "Long 25Δ call, short 10Δ call, short 25Δ put — a financed directional structure.",
  keywords: ["seagull", "three leg", "financed", "collar", "ratio"],
  kind: "strategy",
  defaults: DEFAULT_SEAGULL,
  allowedModels: ["DEFAULT"],
  toInstrument: (inputs: StrategyInputs, ctx): Instrument =>
    withTenorAndModel(strategyBase(inputs, ctx.pair, ctx.tenorYears, ctx.notionalMm), ctx),
  InputBlock: StrategyInputBlock,
});
