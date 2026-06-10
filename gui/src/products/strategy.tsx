/**
 * The vanilla + multi-leg strategy family (GW2) — the leg ladder. Round 2: the
 * ladder is EDITABLE per leg (side / call-put / strike / ratio, add & remove),
 * so a trader can structure a custom-strike vanilla (any level, call OR put) or
 * reshape a strategy's legs within the template's structure law. Strikes accept
 * the platform grammar — an absolute level (`1.0850`), a convention delta
 * (`25dC` / `25dP`), or `ATM` — and a delta-keyed strike rides the wire's
 * `StrikeOrDelta.delta` arm, SOLVED to a level server-side (the engine's
 * delta→strike inversion under the request conventions) and echoed back on the
 * quote's `resolvedStrike`: the inline strike solve.
 *
 * The wire build still goes through the existing `data/seed` builders for the
 * frame (`vanillaInstrument` / `strategyInstrument` — pair, tenor, quantity,
 * side) with the edited legs riding the same proto `Strategy` (it carries
 * arbitrary legs; `kind` records the template). Each template's defaults
 * reproduce the original fixed ladders byte-for-byte, so an unedited ticket's
 * wire output is unchanged.
 */
import type { Instrument, StrategyKind } from "../data/contract";
import { strategyInstrument, vanillaInstrument } from "../data/seed";
import styles from "../workspaces/TicketWorkspace.module.css";
import { defineProduct, withTenorAndModel, type InputBlockProps, type ProductSpec } from "./types";
import {
  legLawMessage,
  legLawViolations,
  StrategyLegEditor,
  strikeEntryMessage,
  strikeEntryViolations,
  type StrategyLegInputs,
  type StrategyTemplate,
} from "./strategyLegEditor";

/**
 * The vanilla / strategy ticket inputs: the template identity plus the editable
 * legs. The committed legs are always wire-valid (the editor commits a strike
 * only once it parses), so `toInstrument` is total; the template's structure
 * law is enforced separately through {@link legLawViolations} (the spec's
 * `validate`), which gates the shell's Request quote with honest messages.
 */
export interface StrategyInputs {
  template: StrategyTemplate;
  legs: StrategyLegInputs[];
}

/** The vanilla default leg — the original hardcoded bought 25Δ call. */
const VANILLA_DEFAULT_LEG: StrategyLegInputs = {
  optionType: "CALL",
  strike: { kind: "delta", delta: 0.25 },
  side: "BUY",
  ratio: 1,
};

/**
 * The fixed convention-delta ladder each strategy template seeds with — mirrors
 * `data/seed`'s `strategyLegs` exactly (the round-trip test pins each template's
 * default build to `strategyInstrument` byte-for-byte, so any drift fails).
 */
function templateDefaultLegs(kind: StrategyKind): StrategyLegInputs[] {
  switch (kind) {
    case "RISK_REVERSAL":
      return [
        { optionType: "CALL", strike: { kind: "delta", delta: 0.25 }, side: "BUY", ratio: 1 },
        { optionType: "PUT", strike: { kind: "delta", delta: -0.25 }, side: "SELL", ratio: 1 },
      ];
    case "STRANGLE":
      return [
        { optionType: "CALL", strike: { kind: "delta", delta: 0.1 }, side: "BUY", ratio: 1 },
        { optionType: "PUT", strike: { kind: "delta", delta: -0.1 }, side: "BUY", ratio: 1 },
      ];
    case "STRADDLE":
      return [
        { optionType: "CALL", strike: { kind: "delta", delta: 0.5 }, side: "BUY", ratio: 1 },
        { optionType: "PUT", strike: { kind: "delta", delta: -0.5 }, side: "BUY", ratio: 1 },
      ];
    case "SEAGULL":
      return [
        { optionType: "CALL", strike: { kind: "delta", delta: 0.25 }, side: "BUY", ratio: 1 },
        { optionType: "CALL", strike: { kind: "delta", delta: 0.1 }, side: "SELL", ratio: 1 },
        { optionType: "PUT", strike: { kind: "delta", delta: -0.25 }, side: "SELL", ratio: 1 },
      ];
  }
}

/** Default inputs for the lone vanilla (the original 25Δ call, fully editable). */
export const DEFAULT_VANILLA: StrategyInputs = {
  template: "VANILLA",
  legs: [VANILLA_DEFAULT_LEG],
};
/** Default inputs for the risk-reversal template. */
export const DEFAULT_RISK_REVERSAL: StrategyInputs = {
  template: "RISK_REVERSAL",
  legs: templateDefaultLegs("RISK_REVERSAL"),
};
/** Default inputs for the strangle template. */
export const DEFAULT_STRANGLE: StrategyInputs = {
  template: "STRANGLE",
  legs: templateDefaultLegs("STRANGLE"),
};
/** Default inputs for the straddle template. */
export const DEFAULT_STRADDLE: StrategyInputs = {
  template: "STRADDLE",
  legs: templateDefaultLegs("STRADDLE"),
};
/** Default inputs for the seagull template. */
export const DEFAULT_SEAGULL: StrategyInputs = {
  template: "SEAGULL",
  legs: templateDefaultLegs("SEAGULL"),
};

/**
 * Build the wire instrument from the template + edited legs. The frame (pair,
 * tenor, quantity, two-way side) comes from the existing `data/seed` builders;
 * the product payload carries the EDITED legs — the proto `Strategy` supports
 * arbitrary legs, and the vanilla arm carries the leg's call/put + strike
 * directly. With the template defaults this reproduces the original
 * `vanillaInstrument` / `strategyInstrument` output byte-for-byte.
 */
function strategyBase(
  inputs: StrategyInputs,
  pair: { base: string; quote: string },
  tenorYears: number,
  notionalMm: number,
): Instrument {
  if (inputs.template === "VANILLA") {
    // The vanilla wire arm carries exactly one payoff; the editor keeps exactly
    // one leg (no add/remove), and the canonical default leg is the total-function
    // read of a structurally impossible empty ladder.
    const leg = inputs.legs[0] ?? VANILLA_DEFAULT_LEG;
    const base = vanillaInstrument(pair, tenorYears, leg.optionType, 0.25, notionalMm);
    return {
      ...base,
      product: {
        kind: "vanilla",
        vanilla: { optionType: leg.optionType, strike: leg.strike },
      },
    };
  }
  const base = strategyInstrument(pair, tenorYears, inputs.template, notionalMm);
  return {
    ...base,
    product: {
      kind: "strategy",
      strategy: {
        kind: inputs.template,
        // Pure wire legs: the editor's in-progress `strikeDraft` never reaches
        // the contract object (the committed strike is the booked one).
        legs: inputs.legs.map((l) => ({
          optionType: l.optionType,
          strike: l.strike,
          side: l.side,
          ratio: l.ratio,
        })),
      },
    },
  };
}

/**
 * The structure gate the shell reads: display-ready violation messages, empty ⇔
 * every quote request is honest about what it books. Composes (1) unparseable
 * in-progress strike entries (a visible bad entry must never silently price the
 * previously committed strike) and (2) the template's structure law.
 */
function strategyValidate(inputs: StrategyInputs): readonly string[] {
  const entries = strikeEntryViolations(inputs.legs).map(
    (v) => `leg ${v.legIndex + 1} strike: ${strikeEntryMessage(v.error)}`,
  );
  const laws = legLawViolations(inputs.template, inputs.legs).map(legLawMessage);
  return [...entries, ...laws];
}

/**
 * The leg-ladder input block: the editable per-leg ladder plus the honest
 * pricing note. Delta / ATM strikes resolve to levels server-side against the
 * marked surface (the inline strike solve); the solved strike is echoed on the
 * quote's `resolvedStrike`, which the shell renders beside the priced two-way.
 */
function StrategyInputBlock({ value, onChange, ctx }: InputBlockProps<StrategyInputs>) {
  const isVanilla = value.template === "VANILLA";
  return (
    <div className={styles.legs}>
      <StrategyLegEditor
        template={value.template}
        legs={value.legs}
        onChange={(legs) => onChange({ ...value, legs })}
      />
      <p className={styles.productNote}>
        {isVanilla
          ? `European vanilla — set call/put and the strike as a level (e.g. ${ctx.atmForward.toFixed(ctx.pipDecimals)}), a convention delta (25dC / 25dP) or ATM. `
          : "Strategy legs — edit each leg's side, type, strike (level, 25dC / 25dP delta, or ATM) and ratio within the template's structure law. "}
        Delta-keyed strikes solve to levels server-side under the request's delta
        convention; the solved K is echoed on the quote.
      </p>
    </div>
  );
}

/** Shared spec body for the five templates (identity/metadata vary per spec). */
function strategySpecBody(): Pick<
  ProductSpec<StrategyInputs>,
  "toInstrument" | "InputBlock" | "validate"
> {
  return {
    toInstrument: (inputs: StrategyInputs, ctx): Instrument =>
      withTenorAndModel(strategyBase(inputs, ctx.pair, ctx.tenorYears, ctx.notionalMm), ctx),
    InputBlock: StrategyInputBlock,
    validate: (inputs: StrategyInputs): readonly string[] => strategyValidate(inputs),
  };
}

/** The vanilla {@link ProductSpec} — custom strike (level / delta / ATM), call or put. */
export const vanillaSpec = defineProduct<StrategyInputs>({
  id: "VANILLA",
  label: "Vanilla",
  group: "Vanilla & strategies",
  assetClass: "FX",
  summary: "Single-leg European vanilla — call or put at a custom strike (level, delta or ATM).",
  keywords: ["vanilla", "european", "call", "put", "single leg", "25 delta", "custom strike"],
  kind: "vanilla",
  defaults: DEFAULT_VANILLA,
  allowedModels: ["DEFAULT", "LOCAL_STOCH_VOL"],
  ...strategySpecBody(),
});

/** The risk-reversal {@link ProductSpec} (a call against a put, one bought one sold). */
export const riskReversalSpec = defineProduct<StrategyInputs>({
  id: "RISK_REVERSAL",
  label: "Risk Reversal",
  group: "Vanilla & strategies",
  assetClass: "FX",
  summary: "Long 25Δ call vs short 25Δ put — the smile-skew structure; legs editable.",
  keywords: ["risk reversal", "rr", "skew", "collar", "25 delta"],
  kind: "strategy",
  defaults: DEFAULT_RISK_REVERSAL,
  allowedModels: ["DEFAULT"],
  ...strategySpecBody(),
});

/** The strangle {@link ProductSpec} (a call and a put, same side, distinct strikes). */
export const strangleSpec = defineProduct<StrategyInputs>({
  id: "STRANGLE",
  label: "Strangle",
  group: "Vanilla & strategies",
  assetClass: "FX",
  summary: "Long 10Δ call + long 10Δ put — the smile-convexity (wing) structure; legs editable.",
  keywords: ["strangle", "wings", "convexity", "butterfly", "10 delta"],
  kind: "strategy",
  defaults: DEFAULT_STRANGLE,
  allowedModels: ["DEFAULT"],
  ...strategySpecBody(),
});

/** The straddle {@link ProductSpec} (a call and a put sharing one strike). */
export const straddleSpec = defineProduct<StrategyInputs>({
  id: "STRADDLE",
  label: "Straddle",
  group: "Vanilla & strategies",
  assetClass: "FX",
  summary: "Long ATM call + long ATM put — the at-the-money volatility structure; legs editable.",
  keywords: ["straddle", "atm", "volatility", "vega", "50 delta"],
  kind: "strategy",
  defaults: DEFAULT_STRADDLE,
  allowedModels: ["DEFAULT"],
  ...strategySpecBody(),
});

/** The seagull {@link ProductSpec} (three legs mixing calls/puts and buys/sells). */
export const seagullSpec = defineProduct<StrategyInputs>({
  id: "SEAGULL",
  label: "Seagull",
  group: "Vanilla & strategies",
  assetClass: "FX",
  summary:
    "Long 25Δ call, short 10Δ call, short 25Δ put — a financed directional structure; legs editable.",
  keywords: ["seagull", "three leg", "financed", "collar", "ratio"],
  kind: "strategy",
  defaults: DEFAULT_SEAGULL,
  allowedModels: ["DEFAULT"],
  ...strategySpecBody(),
});
