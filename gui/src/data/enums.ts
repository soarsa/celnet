/**
 * The wire projection of the vocabulary enums: the single source of truth that
 * maps the GUI's string-form enum members (src/data/contract.ts) to and from the
 * canonical `celnet.wire` proto **enum numbers** the WebSocket JSON mirror carries
 * (crates/celnet-server/src/ws/codec.rs encodes/decodes every enum by its numeric
 * proto tag). One contract, two encodings: the GUI's typed string form is purely a
 * presentation projection of the proto enum numbers — these tables make the two
 * directions exact and reversible, so the live WS transport cannot drift from the
 * wire contract (CLAUDE.md rule 9: one current contract, no fork).
 *
 * Every number here is the proto enum member's tag in `proto/celnet.proto`; the
 * doc on each table cites the proto enum it mirrors. Decoding clamps an unknown
 * number to the proto3 zero value (defensive against an evolved server emitting a
 * member this build does not yet name), exactly as a proto3 reader would.
 */

import type {
  AsianMethod,
  AtmConvention,
  AveragingStyle,
  Cut,
  DayCount,
  DeltaConvention,
  Enforcement,
  LimitMetricKind,
  MarketObservable,
  OptionType,
  PremiumStyle,
  RagStatus,
  RiskDimension,
  Settlement,
  ShockFactor,
  Side,
  SmileModel,
  StrategyKind,
  StreamRejectReason,
  TenorUnit,
} from "./contract";

/** A bidirectional string↔number enum projection plus a zero-value fallback. */
interface EnumCodec<T extends string> {
  /** GUI string member → proto enum number. */
  readonly toWire: (value: T) => number;
  /** Proto enum number → GUI string member (unknown ⇒ proto3 zero value). */
  readonly fromWire: (n: number) => T;
}

/**
 * Build a reversible enum codec from an ordered list of GUI members whose index
 * IS the proto enum number (proto3 members are declared 0,1,2,… in the same order
 * the GUI type lists them — verified against `proto/celnet.proto`). The first
 * member is the proto3 zero value used as the unknown-number fallback on decode.
 */
function enumCodec<T extends string>(membersInWireOrder: readonly T[]): EnumCodec<T> {
  const toNumber = new Map<T, number>();
  membersInWireOrder.forEach((m, i) => toNumber.set(m, i));
  const zero = membersInWireOrder[0]!;
  return {
    toWire: (value: T) => toNumber.get(value) ?? 0,
    fromWire: (n: number) => membersInWireOrder[n] ?? zero,
  };
}

/** `OptionType` ↔ proto `OptionType` (CALL=0, PUT=1). */
export const optionType = enumCodec<OptionType>(["CALL", "PUT"]);

/** `Side` ↔ proto `Side` (BUY=0, SELL=1, TWO_WAY=2). */
export const side = enumCodec<Side>(["BUY", "SELL", "TWO_WAY"]);

/** `DeltaConvention` ↔ proto `DeltaConvention`. */
export const deltaConvention = enumCodec<DeltaConvention>([
  "SPOT_UNADJUSTED",
  "FORWARD_UNADJUSTED",
  "SPOT_PREMIUM_ADJUSTED",
  "FORWARD_PREMIUM_ADJUSTED",
]);

/** `AtmConvention` ↔ proto `AtmConvention`. */
export const atmConvention = enumCodec<AtmConvention>([
  "ATM_FORWARD",
  "DELTA_NEUTRAL_STRADDLE",
]);

/** `PremiumStyle` ↔ proto `PremiumStyle`. */
export const premiumStyle = enumCodec<PremiumStyle>([
  "DOMESTIC_PIPS",
  "PERCENT_FOREIGN",
  "PERCENT_DOMESTIC",
  "FOREIGN_PIPS",
]);

/** `Cut` ↔ proto `Cut`. */
export const cut = enumCodec<Cut>(["NEW_YORK_1000", "TOKYO_1500"]);

/** `DayCount` ↔ proto `DayCount`. */
export const dayCount = enumCodec<DayCount>(["ACT_365_FIXED", "ACT_360"]);

/** `Settlement` ↔ proto `Settlement`. */
export const settlement = enumCodec<Settlement>(["DELIVERABLE", "NON_DELIVERABLE"]);

/** `StrategyKind` ↔ proto `StrategyKind`. */
export const strategyKind = enumCodec<StrategyKind>([
  "RISK_REVERSAL",
  "STRANGLE",
  "STRADDLE",
  "SEAGULL",
]);

/**
 * `TenorUnit` ↔ proto `Tenor.Unit` (OVERNIGHT=0, WEEKS=1, MONTHS=2, YEARS=3,
 * TOM_NEXT=4, SPOT_NEXT=5, IMM=6, BROKEN_DATE=7).
 */
export const tenorUnit = enumCodec<TenorUnit>([
  "OVERNIGHT",
  "WEEKS",
  "MONTHS",
  "YEARS",
  "TOM_NEXT",
  "SPOT_NEXT",
  "IMM",
  "BROKEN_DATE",
]);

/**
 * `AveragingStyle` ↔ proto `AveragingStyle`
 * (AVERAGING_STYLE_DISCRETE=0, AVERAGING_STYLE_CONTINUOUS=1).
 */
export const averagingStyle = enumCodec<AveragingStyle>(["DISCRETE", "CONTINUOUS"]);

/**
 * `AsianMethod` ↔ proto `AsianMethod`
 * (ASIAN_METHOD_CURRAN=0, ASIAN_METHOD_TURNBULL_WAKEMAN=1).
 */
export const asianMethod = enumCodec<AsianMethod>(["CURRAN", "TURNBULL_WAKEMAN"]);

/**
 * `SmileModel` ↔ proto `SmileModel` (MARKET_HEDGE=0, STOCHASTIC_VOL=1,
 * PARAMETRIC=2, PARAMETRIC_SURFACE=3). Default (decode-zero) = MARKET_HEDGE.
 */
export const smileModel = enumCodec<SmileModel>([
  "MARKET_HEDGE",
  "STOCHASTIC_VOL",
  "PARAMETRIC",
  "PARAMETRIC_SURFACE",
]);

/**
 * `MarketObservable` ↔ proto `MarketObservable` (ATM_VOL=0, SPOT=1,
 * RISK_REVERSAL=2, BUTTERFLY=3, FORWARD=4).
 */
export const marketObservable = enumCodec<MarketObservable>([
  "ATM_VOL",
  "SPOT",
  "RISK_REVERSAL",
  "BUTTERFLY",
  "FORWARD",
]);

/** `ShockFactor` ↔ proto `ShockAxis.Factor` (SPOT=0,…,TIME=4). */
export const shockFactor = enumCodec<ShockFactor>([
  "SPOT",
  "VOL",
  "RATE_DOM",
  "RATE_FOR",
  "TIME",
]);

/** `StreamRejectReason` ↔ proto `StreamReject.Reason` (EXPIRED=0,…). */
export const streamRejectReason = enumCodec<StreamRejectReason>([
  "EXPIRED",
  "UNKNOWN_TOKEN",
  "ALREADY_CONSUMED",
]);

/** `RiskDimension` ↔ proto `RiskDimension` (FIRM=0, TRADER=1, BOOK=2, DESK=3,
 * CCY_PAIR=4, LOCATION=5, ENTITY=6). */
export const riskDimension = enumCodec<RiskDimension>([
  "FIRM",
  "TRADER",
  "BOOK",
  "DESK",
  "CCY_PAIR",
  "LOCATION",
  "ENTITY",
]);

/** `LimitMetricKind` ↔ proto `LimitMetricKind` (DELTA=0,…,STOP_LOSS). */
export const limitMetricKind = enumCodec<LimitMetricKind>([
  "DELTA",
  "GAMMA",
  "VEGA",
  "VANNA",
  "VOLGA",
  "VEGA_BUCKET",
  "TENOR_VEGA",
  "CONCENTRATION_DELTA",
  "CONCENTRATION_VEGA",
  "VAR",
  "EXPECTED_SHORTFALL",
  "STOP_LOSS",
]);

/** `RagStatus` ↔ proto `RagStatus` (GREEN=0, AMBER=1, RED=2, BREACH=3). */
export const ragStatus = enumCodec<RagStatus>(["GREEN", "AMBER", "RED", "BREACH"]);

/** `Enforcement` ↔ proto `Enforcement` (SOFT=0, HARD=1). */
export const enforcement = enumCodec<Enforcement>(["SOFT", "HARD"]);
