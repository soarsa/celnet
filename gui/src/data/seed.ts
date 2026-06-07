/**
 * Deterministic seed data for the standalone build: default conventions, the
 * watched pairs and their broker quote ladders, and a starter set of streamed
 * structures for the RFS blotter. All values are plausible interbank marks; the
 * shapes match the contract exactly so the transport seam can swap in live data.
 */

import type {
  AccumulatorMonitoring,
  AsianMethod,
  AveragingStyle,
  BarrierKind,
  BarrierSide,
  BrokerQuoteSet,
  CcyPair,
  Conventions,
  DigitalStyle,
  FixingSchedule,
  Instrument,
  Leg,
  LookbackMonitoring,
  LookbackStyle,
  MarketContext,
  MonitoringStyle,
  OptionType,
  QuantoPayoff,
  StrategyKind,
  StrikeOrDelta,
  TarfRedemption,
  TouchKind,
} from "./contract";

export const DEFAULT_CONVENTIONS: Conventions = {
  deltaConvention: "SPOT_PREMIUM_ADJUSTED",
  atmConvention: "DELTA_NEUTRAL_STRADDLE",
  premiumStyle: "PERCENT_FOREIGN",
  cut: "NEW_YORK_1000",
  dayCount: "ACT_365_FIXED",
  settlement: "DELIVERABLE",
};

export interface PairContext {
  pair: CcyPair;
  market: MarketContext;
  /** Pip decimal places for display (JPY pairs are 2/3, majors 4/5). */
  pipDecimals: number;
}

export const PAIRS: PairContext[] = [
  {
    pair: { base: "EUR", quote: "USD" },
    market: { spot: 1.0768, vol: 0.0755, rDom: 0.0432, rFor: 0.0218 },
    pipDecimals: 4,
  },
  {
    pair: { base: "GBP", quote: "USD" },
    market: { spot: 1.2712, vol: 0.0812, rDom: 0.0432, rFor: 0.0476 },
    pipDecimals: 4,
  },
  {
    pair: { base: "USD", quote: "JPY" },
    market: { spot: 156.42, vol: 0.1045, rDom: 0.0008, rFor: 0.0432 },
    pipDecimals: 2,
  },
  {
    pair: { base: "AUD", quote: "USD" },
    market: { spot: 0.6638, vol: 0.0931, rDom: 0.0432, rFor: 0.0412 },
    pipDecimals: 4,
  },
  {
    pair: { base: "USD", quote: "CHF" },
    market: { spot: 0.9018, vol: 0.0688, rDom: 0.0432, rFor: 0.0151 },
    pipDecimals: 4,
  },
];

/** The standard tenor ladder (year fractions) every surface is marked on. */
export const TENOR_LADDER: { label: string; years: number }[] = [
  { label: "ON", years: 1 / 365 },
  { label: "1W", years: 7 / 365 },
  { label: "2W", years: 14 / 365 },
  { label: "1M", years: 30 / 365 },
  { label: "2M", years: 60 / 365 },
  { label: "3M", years: 91 / 365 },
  { label: "6M", years: 182 / 365 },
  { label: "1Y", years: 365 / 365 },
];

/** A plausible broker quote ladder for a pair, anchored on its ATM vol. */
export function brokerLadder(ctx: PairContext): BrokerQuoteSet[] {
  const atm0 = ctx.market.vol;
  return TENOR_LADDER.map((t, i) => {
    // Term structure: short end richer, gentle upward slope to 1Y.
    const atm = atm0 * (1 + 0.18 * Math.log1p(t.years * 4)) - (i === 0 ? 0.004 : 0);
    const rr25 = -0.003 - 0.0009 * i; // put skew typical for risk-off majors
    const bf25 = 0.0016 + 0.0006 * i;
    const rr10 = rr25 * 1.85;
    const bf10 = bf25 * 2.3;
    return {
      tenorYears: t.years,
      atmVol: atm,
      rr25,
      bf25,
      rr10,
      bf10,
      hasTenDelta: true,
    };
  });
}

function vanillaInstrument(
  pair: CcyPair,
  tenorYears: number,
  optionType: "CALL" | "PUT",
  delta: number,
  notionalMm: number,
): Instrument {
  return {
    pair,
    tenor: tenorYearsToTenor(tenorYears),
    expiryYears: tenorYears,
    quantity: { notional: notionalMm * 1e6, baseCcy: true },
    side: "TWO_WAY",
    product: { kind: "vanilla", vanilla: { optionType, strike: { kind: "delta", delta } } },
  };
}

function strategyInstrument(
  pair: CcyPair,
  tenorYears: number,
  kind: StrategyKind,
  notionalMm: number,
): Instrument {
  const legs = strategyLegs(kind);
  return {
    pair,
    tenor: tenorYearsToTenor(tenorYears),
    expiryYears: tenorYears,
    quantity: { notional: notionalMm * 1e6, baseCcy: true },
    side: "TWO_WAY",
    product: { kind: "strategy", strategy: { kind, legs } },
  };
}

/** The inputs for a single-barrier option (`product.singleBarrier`). */
export interface SingleBarrierTerms {
  optionType: OptionType;
  /** Strike as an absolute level or a signed convention delta. */
  strike: StrikeOrDelta;
  kind: BarrierKind;
  side: BarrierSide;
  barrier: number;
  rebate: number;
  monitoring: MonitoringStyle;
}

/** A single-barrier instrument (`product.singleBarrier`). */
function singleBarrierInstrument(
  pair: CcyPair,
  tenorYears: number,
  notionalMm: number,
  terms: SingleBarrierTerms,
): Instrument {
  return {
    pair,
    tenor: tenorYearsToTenor(tenorYears),
    expiryYears: tenorYears,
    quantity: { notional: notionalMm * 1e6, baseCcy: true },
    side: "TWO_WAY",
    product: {
      kind: "singleBarrier",
      singleBarrier: {
        vanilla: { optionType: terms.optionType, strike: terms.strike },
        kind: terms.kind,
        side: terms.side,
        barrier: terms.barrier,
        rebate: terms.rebate,
        monitoring: terms.monitoring,
      },
    },
  };
}

/** The inputs for a double-barrier option (`product.doubleBarrier`). */
export interface DoubleBarrierTerms {
  optionType: OptionType;
  /** Strike as an absolute level or a signed convention delta. */
  strike: StrikeOrDelta;
  kind: BarrierKind;
  lowerBarrier: number;
  upperBarrier: number;
  rebate: number;
  monitoring: MonitoringStyle;
}

/** A double-barrier instrument (`product.doubleBarrier`). */
function doubleBarrierInstrument(
  pair: CcyPair,
  tenorYears: number,
  notionalMm: number,
  terms: DoubleBarrierTerms,
): Instrument {
  return {
    pair,
    tenor: tenorYearsToTenor(tenorYears),
    expiryYears: tenorYears,
    quantity: { notional: notionalMm * 1e6, baseCcy: true },
    side: "TWO_WAY",
    product: {
      kind: "doubleBarrier",
      doubleBarrier: {
        vanilla: { optionType: terms.optionType, strike: terms.strike },
        kind: terms.kind,
        lowerBarrier: terms.lowerBarrier,
        upperBarrier: terms.upperBarrier,
        rebate: terms.rebate,
        monitoring: terms.monitoring,
      },
    },
  };
}

/** The inputs for a digital (binary) option (`product.digital`). */
export interface DigitalTerms {
  optionType: OptionType;
  strike: number;
  style: DigitalStyle;
  payout: number;
}

/** A digital instrument (`product.digital`). */
function digitalInstrument(
  pair: CcyPair,
  tenorYears: number,
  notionalMm: number,
  terms: DigitalTerms,
): Instrument {
  return {
    pair,
    tenor: tenorYearsToTenor(tenorYears),
    expiryYears: tenorYears,
    quantity: { notional: notionalMm * 1e6, baseCcy: true },
    side: "TWO_WAY",
    product: { kind: "digital", digital: { ...terms } },
  };
}

/** The inputs for a touch structure (`product.touch`). */
export interface TouchTerms {
  kind: TouchKind;
  lowerBarrier: number;
  upperBarrier: number;
  rebate: number;
  monitoring: MonitoringStyle;
}

/** `true` iff a touch kind uses two barriers (the corridor structures). */
export function isDoubleTouch(kind: TouchKind): boolean {
  return kind === "DOUBLE_NO_TOUCH" || kind === "DOUBLE_ONE_TOUCH";
}

/** A touch instrument (`product.touch`). */
function touchInstrument(
  pair: CcyPair,
  tenorYears: number,
  notionalMm: number,
  terms: TouchTerms,
): Instrument {
  return {
    pair,
    tenor: tenorYearsToTenor(tenorYears),
    expiryYears: tenorYears,
    quantity: { notional: notionalMm * 1e6, baseCcy: true },
    side: "TWO_WAY",
    product: { kind: "touch", touch: { ...terms } },
  };
}

/**
 * A variance-swap instrument (`product.varianceSwap`). `strikeVol` is the strike
 * in vol terms (the fair variance strike is `strikeVol²`); `0` requests the fair
 * strike off the priced reply.
 */
function varianceSwapInstrument(
  pair: CcyPair,
  tenorYears: number,
  notionalMm: number,
  strikeVol: number,
): Instrument {
  return {
    pair,
    tenor: tenorYearsToTenor(tenorYears),
    expiryYears: tenorYears,
    quantity: { notional: notionalMm * 1e6, baseCcy: true },
    side: "TWO_WAY",
    product: { kind: "varianceSwap", varianceSwap: { strikeVol } },
  };
}

/** A volatility-swap instrument (`product.volatilitySwap`). */
function volatilitySwapInstrument(
  pair: CcyPair,
  tenorYears: number,
  notionalMm: number,
  strikeVol: number,
): Instrument {
  return {
    pair,
    tenor: tenorYearsToTenor(tenorYears),
    expiryYears: tenorYears,
    quantity: { notional: notionalMm * 1e6, baseCcy: true },
    side: "TWO_WAY",
    product: { kind: "volatilitySwap", volatilitySwap: { strikeVol } },
  };
}

/** The inputs for an arithmetic-average-rate Asian instrument. */
export interface AsianTerms {
  optionType: OptionType;
  strike: number;
  averaging: AveragingStyle;
  observations: number;
  method: AsianMethod;
  elapsedAvg: number;
  elapsedWeight: number;
}

/** An Asian-option instrument (`product.asianOption`). */
function asianInstrument(
  pair: CcyPair,
  tenorYears: number,
  notionalMm: number,
  terms: AsianTerms,
): Instrument {
  return {
    pair,
    tenor: tenorYearsToTenor(tenorYears),
    expiryYears: tenorYears,
    quantity: { notional: notionalMm * 1e6, baseCcy: true },
    side: "TWO_WAY",
    product: { kind: "asianOption", asianOption: { ...terms } },
  };
}

/** The inputs for a forward-start vanilla (`product.forwardStart`). */
export interface ForwardStartTerms {
  optionType: OptionType;
  /** Strike-reset multiple `m` (`m = 1` is the ATM-forward reset). */
  moneyness: number;
  /** Reset (strike-fixing) date in years, with `0 ≤ reset ≤ expiryYears`. */
  reset: number;
}

/** A forward-start-vanilla instrument (`product.forwardStart`). */
function forwardStartInstrument(
  pair: CcyPair,
  tenorYears: number,
  notionalMm: number,
  terms: ForwardStartTerms,
): Instrument {
  return {
    pair,
    tenor: tenorYearsToTenor(tenorYears),
    expiryYears: tenorYears,
    quantity: { notional: notionalMm * 1e6, baseCcy: true },
    side: "TWO_WAY",
    product: { kind: "forwardStart", forwardStart: { ...terms } },
  };
}

/**
 * The inputs for a cliquet / ratchet (`product.cliquet`). Any local/global clamp
 * (`localFloor`/`localCap`/`globalFloor`/`globalCap` set) switches the pricer to
 * the Monte-Carlo estimator that reports a standard error; `mcPairs`/`mcSeed`
 * tune it (`0` pairs ⇒ a server default) and are ignored for a plain ratchet.
 */
export interface CliquetTerms {
  optionType: OptionType;
  moneyness: number;
  periods: number;
  localFloor?: number;
  localCap?: number;
  globalFloor?: number;
  globalCap?: number;
  mcPairs: number;
  mcSeed: bigint;
}

/** `true` iff a cliquet carries no local/global clamp (the plain ratchet). */
export function isPlainCliquet(c: CliquetTerms): boolean {
  return (
    c.localFloor === undefined &&
    c.localCap === undefined &&
    c.globalFloor === undefined &&
    c.globalCap === undefined
  );
}

/** A cliquet / ratchet instrument (`product.cliquet`). */
function cliquetInstrument(
  pair: CcyPair,
  tenorYears: number,
  notionalMm: number,
  terms: CliquetTerms,
): Instrument {
  return {
    pair,
    tenor: tenorYearsToTenor(tenorYears),
    expiryYears: tenorYears,
    quantity: { notional: notionalMm * 1e6, baseCcy: true },
    side: "TWO_WAY",
    product: { kind: "cliquet", cliquet: { ...terms } },
  };
}

/** The inputs for a quanto option (`product.quanto`). */
export interface QuantoTerms {
  payoff: QuantoPayoff;
  optionType: OptionType;
  strike: number;
  conversionVol: number;
  correlation: number;
}

/** A quanto-option instrument (`product.quanto`). */
function quantoInstrument(
  pair: CcyPair,
  tenorYears: number,
  notionalMm: number,
  terms: QuantoTerms,
): Instrument {
  return {
    pair,
    tenor: tenorYearsToTenor(tenorYears),
    expiryYears: tenorYears,
    quantity: { notional: notionalMm * 1e6, baseCcy: true },
    side: "TWO_WAY",
    product: { kind: "quanto", quanto: { ...terms } },
  };
}

/**
 * Build an equally-spaced `FixingSchedule` of `fixings` dates over `(0, expiry]`,
 * mirroring how the exotics engine spaces a TARF / accumulator schedule (`t_k =
 * k·T/n`, ascending, each ≤ expiry). `notional` is the per-fixing accrual notional.
 */
export function equalFixingSchedule(
  fixings: number,
  expiryYears: number,
  notional: number,
): FixingSchedule {
  const n = Math.max(1, Math.trunc(fixings));
  const fixingYears: number[] = [];
  for (let k = 1; k <= n; k += 1) fixingYears.push((expiryYears * k) / n);
  return { fixingYears, fixingNotional: notional };
}

/** The inputs for a Target-Redemption Forward (`product.tarf`). */
export interface TarfTerms {
  optionType: OptionType;
  strike: number;
  target: number;
  leverage: number;
  redemption: TarfRedemption;
  schedule: FixingSchedule;
  mcPairs: number;
  mcSeed: bigint;
}

/** A TARF instrument (`product.tarf`). */
function tarfInstrument(
  pair: CcyPair,
  tenorYears: number,
  notionalMm: number,
  terms: TarfTerms,
): Instrument {
  return {
    pair,
    tenor: tenorYearsToTenor(tenorYears),
    expiryYears: tenorYears,
    quantity: { notional: notionalMm * 1e6, baseCcy: true },
    side: "TWO_WAY",
    product: { kind: "tarf", tarf: { ...terms, schedule: { ...terms.schedule } } },
  };
}

/** The inputs for an accumulator (`product.accumulator`). */
export interface AccumulatorTerms {
  pivot: number;
  barrier: number;
  leverage: number;
  monitoring: AccumulatorMonitoring;
  schedule: FixingSchedule;
  mcPairs: number;
  mcSeed: bigint;
}

/** An accumulator instrument (`product.accumulator`). */
function accumulatorInstrument(
  pair: CcyPair,
  tenorYears: number,
  notionalMm: number,
  terms: AccumulatorTerms,
): Instrument {
  return {
    pair,
    tenor: tenorYearsToTenor(tenorYears),
    expiryYears: tenorYears,
    quantity: { notional: notionalMm * 1e6, baseCcy: true },
    side: "TWO_WAY",
    product: {
      kind: "accumulator",
      accumulator: { ...terms, schedule: { ...terms.schedule } },
    },
  };
}

/**
 * The inputs for a lookback option (`product.lookback`). A CONTINUOUS-monitored
 * lookback prices by exact closed form (no MC std-error); a DISCRETE-monitored
 * lookback prices by Monte-Carlo over `observations` observations and reports a
 * standard error. `strike` is used only by the FIXED family.
 */
export interface LookbackTerms {
  style: LookbackStyle;
  optionType: OptionType;
  monitoring: LookbackMonitoring;
  strike: number;
  observations: number;
  mcPairs: number;
  mcSeed: bigint;
}

/** `true` iff a lookback is discretely monitored (the Monte-Carlo variant). */
export function isDiscreteLookback(l: LookbackTerms): boolean {
  return l.monitoring === "DISCRETE";
}

/** A lookback instrument (`product.lookback`). */
function lookbackInstrument(
  pair: CcyPair,
  tenorYears: number,
  notionalMm: number,
  terms: LookbackTerms,
): Instrument {
  return {
    pair,
    tenor: tenorYearsToTenor(tenorYears),
    expiryYears: tenorYears,
    quantity: { notional: notionalMm * 1e6, baseCcy: true },
    side: "TWO_WAY",
    product: { kind: "lookback", lookback: { ...terms } },
  };
}

function strategyLegs(kind: StrategyKind): Leg[] {
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

/** Convert a year fraction to the nearest standard Tenor label. */
export function tenorYearsToTenor(years: number): Instrument["tenor"] {
  if (years <= 2 / 365) return { unit: "OVERNIGHT", count: 1 };
  if (years < 25 / 365) return { unit: "WEEKS", count: Math.round(years * 52) };
  if (years < 360 / 365) return { unit: "MONTHS", count: Math.round(years * 12) };
  return { unit: "YEARS", count: Math.max(1, Math.round(years)) };
}

/** The starter subscriptions for the RFS blotter (matches GUI-DESIGN §4.2). */
export function seedSubscriptions(): { instrument: Instrument; label: string }[] {
  const eur = PAIRS[0]!.pair;
  const gbp = PAIRS[1]!.pair;
  const jpy = PAIRS[2]!.pair;
  const aud = PAIRS[3]!.pair;
  return [
    { instrument: strategyInstrument(eur, 30 / 365, "STRADDLE", 10), label: "ATM straddle" },
    { instrument: strategyInstrument(eur, 30 / 365, "RISK_REVERSAL", 10), label: "25Δ RR" },
    { instrument: strategyInstrument(gbp, 60 / 365, "STRANGLE", 10), label: "10Δ strangle" },
    { instrument: vanillaInstrument(jpy, 1 / 365, "CALL", 0.5, 25), label: "ATM" },
    { instrument: strategyInstrument(aud, 91 / 365, "STRANGLE", 10), label: "10Δ strangle" },
    { instrument: strategyInstrument(eur, 91 / 365, "SEAGULL", 15), label: "seagull" },
  ];
}

export {
  vanillaInstrument,
  strategyInstrument,
  singleBarrierInstrument,
  doubleBarrierInstrument,
  digitalInstrument,
  touchInstrument,
  varianceSwapInstrument,
  volatilitySwapInstrument,
  asianInstrument,
  forwardStartInstrument,
  cliquetInstrument,
  quantoInstrument,
  tarfInstrument,
  accumulatorInstrument,
  lookbackInstrument,
};
