/**
 * Deterministic seed data for the standalone build: default conventions, the
 * watched pairs and their broker quote ladders, and a starter set of streamed
 * structures for the RFS blotter. All values are plausible interbank marks; the
 * shapes match the contract exactly so the transport seam can swap in live data.
 */

import type {
  BrokerQuoteSet,
  CcyPair,
  Conventions,
  Instrument,
  Leg,
  MarketContext,
  StrategyKind,
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

export { vanillaInstrument, strategyInstrument };
