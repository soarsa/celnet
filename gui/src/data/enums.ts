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
  CorpActionStatus,
  CorpEventType,
  CorpMandatory,
  Cut,
  DayCount,
  DeltaConvention,
  DeskRequestKind,
  DeskRequestState,
  DigitalStyle,
  Enforcement,
  FlowGroupBy,
  NotificationKind,
  ExerciseStyle,
  FixingSource,
  LimitMetricKind,
  LookbackMonitoring,
  LookbackStyle,
  ManualInterventionReason,
  Margining,
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
  TransferKind,
  TransferPriceBasis,
  TransferState,
  PriceBasis,
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

/**
 * Build a reversible enum codec whose first GUI member maps to the proto tag
 * `firstTag` (and each subsequent member to `firstTag + i`). Used by the proto3
 * enums that reserve `0` for an `*_UNSPECIFIED` member the GUI string union does
 * NOT name (the desk-quoting enums: `DeskRequestKind`, `DeskRequestState`,
 * `NotificationKind` all start their named members at proto tag 1). The first
 * named member is the unknown-number fallback on decode.
 */
function offsetEnumCodec<T extends string>(
  membersInWireOrder: readonly T[],
  firstTag: number,
): EnumCodec<T> {
  const toNumber = new Map<T, number>();
  membersInWireOrder.forEach((m, i) => toNumber.set(m, i + firstTag));
  const zero = membersInWireOrder[0]!;
  return {
    toWire: (value: T) => toNumber.get(value) ?? firstTag,
    fromWire: (n: number) => membersInWireOrder[n - firstTag] ?? zero,
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

/**
 * `Margining` ↔ proto `Margining`
 * (MARGINING_EQUITY_STYLE=0, MARGINING_FUTURES_STYLE=1). The premium margining
 * convention of an option on a listed future (`ListedFutureOption.margining`,
 * field 6): EQUITY_STYLE (upfront premium, discounted value) is the meaningful
 * proto3 zero; FUTURES_STYLE margins the premium daily (undiscounted value).
 */
export const margining = enumCodec<Margining>(["EQUITY_STYLE", "FUTURES_STYLE"]);

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

/**
 * `DeskRequestKind` ↔ proto `DeskRequestKind`
 * (DESK_REQUEST_KIND_UNSPECIFIED=0, RFQ=1, IOI=2, RFS=3). The GUI union names only
 * the meaningful members, so the codec offsets the first named member to tag 1; an
 * `RFS` fill (a request-for-stream market-data lift) rides at tag 3.
 */
export const deskRequestKind = offsetEnumCodec<DeskRequestKind>(["RFQ", "IOI", "RFS"], 1);

/**
 * `DeskRequestState` ↔ proto `DeskRequestState` (UNSPECIFIED=0, PENDING=1,
 * QUOTED=2, ACCEPTED=3, REJECTED=4, EXPIRED=5, WITHDRAWN=6).
 */
export const deskRequestState = offsetEnumCodec<DeskRequestState>(
  ["PENDING", "QUOTED", "ACCEPTED", "REJECTED", "EXPIRED", "WITHDRAWN"],
  1,
);

/**
 * `NotificationKind` ↔ proto `NotificationKind` (UNSPECIFIED=0, RFQ_RECEIVED=1,
 * IOI_RECEIVED=2, REQUEST_WITHDRAWN=3, REQUEST_EXPIRED=4, QUOTE_ACCEPTED=5,
 * QUOTE_REJECTED=6, MANUAL_INTERVENTION_REQUIRED=7, ORDER_RECEIVED=8, FILL=9).
 * The 7th member was added by the server exception contract (commit 542e547);
 * ORDER_RECEIVED=8 / FILL=9 are the notifications phase-5 additive arms (a FIX-venue
 * firm-order lift emits both). The array order MUST match the proto tag order.
 */
export const notificationKind = offsetEnumCodec<NotificationKind>(
  [
    "RFQ_RECEIVED",
    "IOI_RECEIVED",
    "REQUEST_WITHDRAWN",
    "REQUEST_EXPIRED",
    "QUOTE_ACCEPTED",
    "QUOTE_REJECTED",
    "MANUAL_INTERVENTION_REQUIRED",
    "ORDER_RECEIVED",
    "FILL",
  ],
  1,
);

/**
 * `ManualInterventionReason` ↔ proto `ManualInterventionReason` (UNSPECIFIED=0,
 * UNCONFIGURED_TENOR=1, CREDIT_RISK_BREAK=2, UNKNOWN_SECURITY=3,
 * PRICING_FAILURE=4). Carried on the wire `reason` field of a
 * `MANUAL_INTERVENTION_REQUIRED` notification (server exception contract, commit
 * 542e547). Ordinals start at 1; `0` (unspecified) decodes to the first member.
 */
export const manualInterventionReason = offsetEnumCodec<ManualInterventionReason>(
  [
    "UNCONFIGURED_TENOR",
    "CREDIT_RISK_BREAK",
    "UNKNOWN_SECURITY",
    "PRICING_FAILURE",
  ],
  1,
);

/**
 * `TransferKind` ↔ proto `TransferKind` (TRANSFER_KIND_UNSPECIFIED=0,
 * RE_ATTRIBUTE=1, DESK_TO_DESK=2, TRADER_TO_TRADER=3). The GUI union names only
 * the meaningful members, so the codec offsets the first named member to tag 1.
 */
export const transferKind = offsetEnumCodec<TransferKind>(
  ["RE_ATTRIBUTE", "DESK_TO_DESK", "TRADER_TO_TRADER"],
  1,
);

/**
 * `TransferState` ↔ proto `TransferState` (UNSPECIFIED=0, DRAFT=1, PENDING=2,
 * ACCEPTED=3, REJECTED=4, BOOKED=5, CANCELLED=6).
 */
export const transferState = offsetEnumCodec<TransferState>(
  ["DRAFT", "PENDING", "ACCEPTED", "REJECTED", "BOOKED", "CANCELLED"],
  1,
);

/**
 * `TransferPriceBasis` ↔ proto `TransferPriceBasis` (UNSPECIFIED=0, MID=1,
 * MARK_TO_MARKET=2, AGREED=3). The requested basis on the ticket / record.
 */
export const transferPriceBasis = offsetEnumCodec<TransferPriceBasis>(
  ["MID", "MARK_TO_MARKET", "AGREED"],
  1,
);

/**
 * `PriceBasis` ↔ proto `PriceBasis` (UNSPECIFIED=0, MID=1, MARK_TO_MARKET=2,
 * AGREED=3). The basis recorded on a booked transfer's immutable provenance —
 * the SAME value set as {@link transferPriceBasis}, kept a distinct codec to
 * mirror the domain (the record carries no override value on `AGREED`).
 */
export const priceBasis = offsetEnumCodec<PriceBasis>(
  ["MID", "MARK_TO_MARKET", "AGREED"],
  1,
);

/**
 * `CorpEventType` ↔ proto `CorpEventType` (CORP_EVENT_TYPE_UNSPECIFIED=0, REDM=1,
 * INTR=2, MCAL=3, PCAL=4, PRED=5, DRAW=6, BPUT=7, TEND=8, EXOF=9, CONV=10). The GUI
 * union names only the meaningful members, so the codec offsets the first named
 * member (`REDM`) to proto tag 1; decode-zero (`UNSPECIFIED`) clamps to `REDM`.
 */
export const corpEventType = offsetEnumCodec<CorpEventType>(
  ["REDM", "INTR", "MCAL", "PCAL", "PRED", "DRAW", "BPUT", "TEND", "EXOF", "CONV"],
  1,
);

/**
 * `CorpMandatory` ↔ proto `CorpMandatory` (CORP_MANDATORY_UNSPECIFIED=0, MAND=1,
 * VOLU=2, CHOS=3). Named members start at proto tag 1.
 */
export const corpMandatory = offsetEnumCodec<CorpMandatory>(["MAND", "VOLU", "CHOS"], 1);

/**
 * `CorpActionStatus` ↔ proto `CorpActionStatus` (CORP_ACTION_STATUS_UNSPECIFIED=0,
 * ANNOUNCED=1, ELECTED=2, CONFIRMED=3, APPLIED=4, REVERSED=5, CANCELLED=6). Named
 * members start at proto tag 1.
 */
export const corpActionStatus = offsetEnumCodec<CorpActionStatus>(
  ["ANNOUNCED", "ELECTED", "CONFIRMED", "APPLIED", "REVERSED", "CANCELLED"],
  1,
);

/**
 * `FlowGroupBy` ↔ proto `FlowGroupBy` (FLOW_GROUP_BY_CLIENT=0, COUNTERPARTY=1,
 * INSTRUMENT=2, ASSET=3). The GUI union NAMES the proto3 zero member (`client`),
 * so this is a plain zero-based codec (no offset); decode-zero = `client`.
 */
export const flowGroupBy = enumCodec<FlowGroupBy>([
  "client",
  "counterparty",
  "instrument",
  "asset",
]);
