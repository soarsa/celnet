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
  AccumulatorMonitoring,
  AsianMethod,
  AtmConvention,
  AveragingStyle,
  BarrierKind,
  BarrierSide,
  BasketKind,
  Cut,
  DayCount,
  DeltaConvention,
  DigitalStyle,
  Enforcement,
  ExerciseStyle,
  FixingSource,
  LimitMetricKind,
  LookbackMonitoring,
  LookbackStyle,
  MarketObservable,
  Metal,
  MonitoringStyle,
  OptionType,
  PremiumStyle,
  PricingModel,
  QuantoPayoff,
  RagStatus,
  RiskDimension,
  Settlement,
  SettlementStyle,
  ShockFactor,
  Side,
  SmileModel,
  StrategyKind,
  StreamRejectReason,
  TarfRedemption,
  TenorUnit,
  TouchKind,
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

/**
 * `PricingModel` ↔ proto `PricingModel`
 * (PRICING_MODEL_DEFAULT=0, PRICING_MODEL_LOCAL_STOCH_VOL=1). Default
 * (decode-zero) = DEFAULT (the per-product closed-form engine).
 */
export const pricingModel = enumCodec<PricingModel>(["DEFAULT", "LOCAL_STOCH_VOL"]);

/** `Side` ↔ proto `Side` (BUY=0, SELL=1, TWO_WAY=2). */
export const side = enumCodec<Side>(["BUY", "SELL", "TWO_WAY"]);

/**
 * `FixingSource` ↔ proto `FixingSource` (KRW_KFTC18=0, TWD_TAIPEI=1,
 * INR_RBI_REF=2, BRL_PTAX=3, CLP_DOLAR_OBS=4, COP_TRM=5). Identity only — names
 * which published settlement-rate the NDF fixes against, never a fixing value.
 */
export const fixingSource = enumCodec<FixingSource>([
  "KRW_KFTC18",
  "TWD_TAIPEI",
  "INR_RBI_REF",
  "BRL_PTAX",
  "CLP_DOLAR_OBS",
  "COP_TRM",
]);

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

/**
 * `Metal` ↔ proto `Metal` (METAL_GOLD=0, METAL_SILVER=1, METAL_PLATINUM=2,
 * METAL_PALLADIUM=3). The precious-metal base/asset leg of a metal pair.
 */
export const metal = enumCodec<Metal>(["GOLD", "SILVER", "PLATINUM", "PALLADIUM"]);

/**
 * `SettlementStyle` ↔ proto `SettlementStyle`
 * (SETTLEMENT_STYLE_LINEAR=0, SETTLEMENT_STYLE_INVERSE_COIN=1). The contract
 * settlement mechanics carried on `Instrument.settlement_style` (field 29):
 * LINEAR (quote-ccy-margined) is the proto3 zero default; INVERSE_COIN is the
 * coin-margined `1/S_T` digital-asset convention.
 */
export const settlementStyle = enumCodec<SettlementStyle>(["LINEAR", "INVERSE_COIN"]);

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
 * `BarrierKind` ↔ proto `BarrierKind`
 * (BARRIER_KIND_KNOCK_IN=0, BARRIER_KIND_KNOCK_OUT=1).
 */
export const barrierKind = enumCodec<BarrierKind>(["KNOCK_IN", "KNOCK_OUT"]);

/**
 * `BarrierSide` ↔ proto `BarrierSide`
 * (BARRIER_SIDE_UP=0, BARRIER_SIDE_DOWN=1).
 */
export const barrierSide = enumCodec<BarrierSide>(["UP", "DOWN"]);

/**
 * `MonitoringStyle` ↔ proto `MonitoringStyle`
 * (MONITORING_STYLE_CONTINUOUS=0, MONITORING_STYLE_DISCRETE=1).
 */
export const monitoringStyle = enumCodec<MonitoringStyle>(["CONTINUOUS", "DISCRETE"]);

/**
 * `TouchKind` ↔ proto `TouchKind` (TOUCH_KIND_ONE_TOUCH=0, TOUCH_KIND_NO_TOUCH=1,
 * TOUCH_KIND_DOUBLE_NO_TOUCH=2, TOUCH_KIND_DOUBLE_ONE_TOUCH=3).
 */
export const touchKind = enumCodec<TouchKind>([
  "ONE_TOUCH",
  "NO_TOUCH",
  "DOUBLE_NO_TOUCH",
  "DOUBLE_ONE_TOUCH",
]);

/**
 * `DigitalStyle` ↔ proto `DigitalStyle`
 * (DIGITAL_STYLE_CASH_OR_NOTHING=0, DIGITAL_STYLE_ASSET_OR_NOTHING=1).
 */
export const digitalStyle = enumCodec<DigitalStyle>(["CASH_OR_NOTHING", "ASSET_OR_NOTHING"]);

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
 * `QuantoPayoff` ↔ proto `QuantoPayoff`
 * (QUANTO_PAYOFF_VANILLA=0, QUANTO_PAYOFF_DIGITAL=1).
 */
export const quantoPayoff = enumCodec<QuantoPayoff>(["VANILLA", "DIGITAL"]);

/**
 * `TarfRedemption` ↔ proto `TarfRedemption`
 * (TARF_REDEMPTION_FULL_GAIN=0, TARF_REDEMPTION_CAPPED_GAIN=1).
 */
export const tarfRedemption = enumCodec<TarfRedemption>(["FULL_GAIN", "CAPPED_GAIN"]);

/**
 * `AccumulatorMonitoring` ↔ proto `AccumulatorMonitoring`
 * (ACCUMULATOR_MONITORING_DISCRETE=0, ACCUMULATOR_MONITORING_CONTINUOUS=1).
 */
export const accumulatorMonitoring = enumCodec<AccumulatorMonitoring>(["DISCRETE", "CONTINUOUS"]);

/**
 * `LookbackStyle` ↔ proto `LookbackStyle`
 * (LOOKBACK_STYLE_FLOATING=0, LOOKBACK_STYLE_FIXED=1).
 */
export const lookbackStyle = enumCodec<LookbackStyle>(["FLOATING", "FIXED"]);

/**
 * `LookbackMonitoring` ↔ proto `LookbackMonitoring`
 * (LOOKBACK_MONITORING_CONTINUOUS=0, LOOKBACK_MONITORING_DISCRETE=1).
 */
export const lookbackMonitoring = enumCodec<LookbackMonitoring>(["CONTINUOUS", "DISCRETE"]);

/**
 * `ExerciseStyle` ↔ proto `ExerciseStyle`
 * (EXERCISE_STYLE_AMERICAN=0, EXERCISE_STYLE_BERMUDAN=1).
 */
export const exerciseStyle = enumCodec<ExerciseStyle>(["AMERICAN", "BERMUDAN"]);

/**
 * `BasketKind` ↔ proto `BasketKind`
 * (BASKET_KIND_BASKET=0, BASKET_KIND_BEST_OF=1, BASKET_KIND_WORST_OF=2).
 */
export const basketKind = enumCodec<BasketKind>(["BASKET", "BEST_OF", "WORST_OF"]);

/**
 * `SmileModel` ↔ proto `SmileModel` (MARKET_HEDGE=0, STOCHASTIC_VOL=1,
 * PARAMETRIC=2, PARAMETRIC_SURFACE=3, EXTENDED_SURFACE=4). Array index MUST
 * equal the proto number. Default (decode-zero) = MARKET_HEDGE.
 */
export const smileModel = enumCodec<SmileModel>([
  "MARKET_HEDGE",
  "STOCHASTIC_VOL",
  "PARAMETRIC",
  "PARAMETRIC_SURFACE",
  "EXTENDED_SURFACE",
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
