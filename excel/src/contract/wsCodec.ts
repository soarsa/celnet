// ONE CONTRACT — minimal duplicate of `gui/src/data/wsCodec.ts`, semantics-identical
// (CLAUDE.md rule 9). This is the EXACT field-for-field mirror of the server's
// `crates/celnet-server/src/ws/codec.rs`; do not diverge from either.
/**
 * The browser-side JSON codec for the WebSocket mirror — the exact mirror of the
 * server's `crates/celnet-server/src/ws/codec.rs`. Every encoder here produces the
 * snake_case, numeric-enum JSON the server decodes; every decoder reads the
 * snake_case, numeric-enum JSON the server encodes. There is no second contract:
 * this file and codec.rs are two ends of the SAME single, current `celnet.wire`
 * contract (CLAUDE.md rule 9), so a value priced over WS is byte-identical to the
 * gRPC/direct price.
 *
 * Numbers on the wire are plain JSON numbers; identifiers that the proto types as
 * 64-bit (`uint64`/`int64` — token, sequence, ids, nanos) are carried as JSON
 * numbers by the server's `serde_json` (it does not stringify them), so we read
 * them with `numToBigInt` to recover the GUI's `bigint` shape and emit them as
 * plain numbers (their magnitude stays within JS safe-integer range for the
 * session-scoped ids and ms-resolution nanos the mirror produces). Optional
 * presence-tracked fields are `null`/absent ⇒ `undefined`.
 */

import type {
  Accumulator,
  AmericanOption,
  ArbReport,
  BasketOption,
  BrokerQuoteSet,
  BucketedRisk,
  CcyPair,
  Cliquet,
  Conventions,
  CrossGamma,
  Digital,
  DoubleBarrier,
  Executed,
  Execution,
  FixingSchedule,
  FxForward,
  Greeks,
  Heartbeat,
  Instrument,
  Leg,
  Lookback,
  SingleBarrier,
  Touch,
  Vanilla,
  MarkedSurface,
  MarketContext,
  MarketSeriesPoint,
  MarketSeriesSnapshot,
  Quote,
  ScenarioPoint,
  ScenarioResult,
  ShockAxis,
  Smile,
  SmilePoint,
  Snapshot,
  Solve,
  StrategyKind,
  StreamReject,
  StrikeOrDelta,
  Tarf,
  TradableToken,
  TwoWayPrice,
  Underlying,
  Update,
  VegaBucket,
  WindowBarrier,
} from "./contract";
import * as e from "./enums";

/** A decoded server frame is a JSON object with a `type` discriminator. */
export type WireObject = Record<string, unknown>;

// ---------------------------------------------------------------------------
// scalar accessors (decode side) — defensive against a malformed frame
// ---------------------------------------------------------------------------

function num(o: WireObject, key: string): number {
  const v = o[key];
  return typeof v === "number" ? v : 0;
}

/** A 64-bit wire integer (JSON number) recovered as a `bigint`. */
function numToBigInt(o: WireObject, key: string): bigint {
  const v = o[key];
  if (typeof v === "number" && Number.isFinite(v)) return BigInt(Math.trunc(v));
  if (typeof v === "bigint") return v;
  if (typeof v === "string" && v.length > 0) {
    try {
      return BigInt(v);
    } catch {
      return 0n;
    }
  }
  return 0n;
}

/** An optional presence-tracked finite number (`null`/absent/non-number ⇒ undefined). */
function optNum(o: WireObject, key: string): number | undefined {
  const v = o[key];
  return typeof v === "number" && Number.isFinite(v) ? v : undefined;
}

/** An optional presence-tracked 64-bit integer (`null`/absent ⇒ undefined). */
function optBigInt(o: WireObject, key: string): bigint | undefined {
  const v = o[key];
  if (v === null || v === undefined) return undefined;
  return numToBigInt(o, key);
}

function str(o: WireObject, key: string): string {
  const v = o[key];
  return typeof v === "string" ? v : "";
}

function enumNum(o: WireObject, key: string): number {
  const v = o[key];
  return typeof v === "number" ? v : 0;
}

function child(o: WireObject, key: string): WireObject {
  const v = o[key];
  return v && typeof v === "object" ? (v as WireObject) : {};
}

function array(o: WireObject, key: string): WireObject[] {
  const v = o[key];
  return Array.isArray(v) ? (v as WireObject[]) : [];
}

// ---------------------------------------------------------------------------
// vocabulary — encode (GUI → wire)
// ---------------------------------------------------------------------------

export function ccyPairToWire(p: CcyPair): WireObject {
  return { base: p.base, quote: p.quote };
}

/**
 * Encode the cross-asset {@link Underlying} oneof (proto `Underlying`) under the
 * `underlying` key: the active arm by its proto field NAME (snake_case —
 * `fx`/`metal`/`equity`/`commodity`/`digital_asset`, proto `oneof ref` field
 * numbers 1/3/4/5/6) plus `settlement_ccy`. The `metal` arm carries the numeric
 * `Metal` enum tag; the equity/commodity arms nest the `symbol` (ticker + venue).
 */
export function underlyingToWire(u: Underlying): WireObject {
  const settlement_ccy = u.settlementCcy;
  switch (u.kind) {
    case "fx":
      return { fx: { base: u.fx.base, quote: u.fx.quote }, settlement_ccy };
    case "metal":
      return {
        metal: { metal: e.metal.toWire(u.metal.metal), quote: u.metal.quote },
        settlement_ccy,
      };
    case "equity":
      return {
        equity: {
          symbol: { ticker: u.equity.symbol.ticker, venue: u.equity.symbol.venue },
          currency: u.equity.currency,
        },
        settlement_ccy,
      };
    case "commodity":
      return {
        commodity: {
          symbol: { ticker: u.commodity.symbol.ticker, venue: u.commodity.symbol.venue },
          currency: u.commodity.currency,
        },
        settlement_ccy,
      };
    case "digitalAsset":
      return {
        digital_asset: { base: u.digitalAsset.base, quote: u.digitalAsset.quote },
        settlement_ccy,
      };
  }
}

export function conventionsToWire(c: Conventions): WireObject {
  return {
    delta_convention: e.deltaConvention.toWire(c.deltaConvention),
    atm_convention: e.atmConvention.toWire(c.atmConvention),
    premium_style: e.premiumStyle.toWire(c.premiumStyle),
    cut: e.cut.toWire(c.cut),
    day_count: e.dayCount.toWire(c.dayCount),
    settlement: e.settlement.toWire(c.settlement),
  };
}

export function marketToWire(m: MarketContext): WireObject {
  return { spot: m.spot, vol: m.vol, r_dom: m.rDom, r_for: m.rFor };
}

function strikeOrDeltaToWire(s: StrikeOrDelta): WireObject {
  return s.kind === "strike" ? { strike: s.strike } : { delta: s.delta };
}

function solveToWire(s: Solve): WireObject {
  // Solve.Target proto enum: NONE=0, STRIKE=1, PREMIUM=2 (matches the GUI union).
  const target = s.target === "STRIKE" ? 1 : s.target === "PREMIUM" ? 2 : 0;
  return { target, target_premium: s.targetPremium };
}

function legToWire(leg: Leg): WireObject {
  return {
    option_type: e.optionType.toWire(leg.optionType),
    strike: strikeOrDeltaToWire(leg.strike),
    side: e.side.toWire(leg.side),
    ratio: leg.ratio,
  };
}

/**
 * Encode a vanilla payoff body (`{ option_type, strike }`). Shared by the bare
 * `vanilla` product arm and the single/double-barrier bodies that nest a vanilla
 * under their own `vanilla` key — exactly the shape the server's `vanilla_from_json`
 * reads (it descends into the `vanilla` value and decodes `option_type`/`strike`).
 */
function vanillaToWire(v: Vanilla): WireObject {
  return {
    option_type: e.optionType.toWire(v.optionType),
    strike: strikeOrDeltaToWire(v.strike),
  };
}

/**
 * Encode a single-barrier body (proto field 9). The vanilla payoff nests under
 * `vanilla`; `kind`/`side`/`monitoring` are numeric enums; `barrier`/`rebate` are
 * plain numbers — the EXACT shape `single_barrier_from_json` decodes.
 */
function singleBarrierToWire(b: SingleBarrier): WireObject {
  return {
    vanilla: vanillaToWire(b.vanilla),
    kind: e.barrierKind.toWire(b.kind),
    side: e.barrierSide.toWire(b.side),
    barrier: b.barrier,
    rebate: b.rebate,
    monitoring: e.monitoringStyle.toWire(b.monitoring),
  };
}

/** Encode a double-barrier body (proto field 10) — see `single_barrier` for conventions. */
function doubleBarrierToWire(b: DoubleBarrier): WireObject {
  return {
    vanilla: vanillaToWire(b.vanilla),
    kind: e.barrierKind.toWire(b.kind),
    lower_barrier: b.lowerBarrier,
    upper_barrier: b.upperBarrier,
    rebate: b.rebate,
    monitoring: e.monitoringStyle.toWire(b.monitoring),
  };
}

/** Encode a digital body (proto field 11) — `option_type`/`style` numeric enums. */
function digitalToWire(d: Digital): WireObject {
  return {
    option_type: e.optionType.toWire(d.optionType),
    strike: d.strike,
    style: e.digitalStyle.toWire(d.style),
    payout: d.payout,
  };
}

/**
 * Encode a touch body (proto field 12). `upper_barrier` is always present (the
 * server ignores it for the single-barrier one-/no-touch kinds); `kind`/
 * `monitoring` are numeric enums — the EXACT shape `touch_from_json` decodes.
 */
function touchToWire(t: Touch): WireObject {
  return {
    kind: e.touchKind.toWire(t.kind),
    lower_barrier: t.lowerBarrier,
    upper_barrier: t.upperBarrier,
    rebate: t.rebate,
    monitoring: e.monitoringStyle.toWire(t.monitoring),
  };
}

/**
 * Encode a window-barrier body (proto field 23). The terminal vanilla payoff
 * nests under `vanilla`; `side` is the numeric `BarrierSide`; `window_start`/
 * `window_end` are plain year fractions; `mc_pairs`/`mc_steps` are plain integers
 * and `mc_seed` follows the codec's 64-bit-as-JSON-number convention (server reads
 * with `u64_or_zero`). The EXACT shape `window_barrier_from_json` decodes. A
 * window barrier is LSV-only — the instrument's `pricing_model` carries that.
 */
function windowBarrierToWire(w: WindowBarrier): WireObject {
  return {
    vanilla: vanillaToWire(w.vanilla),
    barrier: w.barrier,
    side: e.barrierSide.toWire(w.side),
    window_start: w.windowStart,
    window_end: w.windowEnd,
    mc_pairs: w.mcPairs,
    mc_steps: w.mcSteps,
    mc_seed: Number(w.mcSeed),
  };
}

export function instrumentToWire(i: Instrument): WireObject {
  const base: WireObject = {
    pair: ccyPairToWire(i.pair),
    tenor: { unit: e.tenorUnit.toWire(i.tenor.unit), count: i.tenor.count },
    expiry_years: i.expiryYears,
    quantity: { notional: i.quantity.notional, base_ccy: i.quantity.baseCcy },
    side: e.side.toWire(i.side),
  };
  // The cross-asset underlying (proto `Instrument.underlying`, field 1). Emit it
  // ONLY when set: an absent `underlying` keeps the FX `pair` projection the FX WS
  // surface keys on (byte-identical to the contract before the cross-asset arms).
  // A non-FX instrument carries the full `Underlying` oneof here AND the leg-string
  // `pair` so the FX-keyed surfaces stay total.
  if (i.underlying) base["underlying"] = underlyingToWire(i.underlying);
  if (i.solve) base["solve"] = solveToWire(i.solve);
  // The settlement-style selector (proto `Instrument.settlement_style`, field 29).
  // Emit it ONLY when non-LINEAR: LINEAR is the proto3 zero value, so omitting it
  // keeps the wire frame byte-identical to the contract before this field existed
  // (the server reads an absent key as LINEAR). INVERSE_COIN is carried as its
  // numeric proto tag — the coin-margined `1/S_T` digital-asset convention.
  if (i.settlementStyle !== undefined && i.settlementStyle !== "LINEAR") {
    base["settlement_style"] = e.settlementStyle.toWire(i.settlementStyle);
  }
  // The pricing-model selector (proto `Instrument.pricing_model`, field 22). Emit
  // it ONLY when non-DEFAULT: a DEFAULT/absent model is the proto3 zero value, so
  // omitting it keeps the wire frame byte-identical to the contract before this
  // field existed (the server's `enum_or_zero(o, "pricing_model")` reads an absent
  // key as DEFAULT). A non-DEFAULT model is carried as its numeric proto tag.
  if (i.pricingModel !== undefined && i.pricingModel !== "DEFAULT") {
    base["pricing_model"] = e.pricingModel.toWire(i.pricingModel);
  }
  // The product oneof: nest the body under its own key (the proto field name) with
  // the proto field number it occupies — vanilla=7, strategy=8, single_barrier=9,
  // double_barrier=10, digital=11, touch=12, variance_swap=13, volatility_swap=14,
  // asian_option=15, forward_start=16, cliquet=17, quanto=18, tarf=19,
  // accumulator=20, lookback=21, window_barrier=23. The WS JSON mirror keys by
  // name, exactly like `crates/celnet-server/src/ws/codec.rs` decodes.
  switch (i.product.kind) {
    case "vanilla":
      base["vanilla"] = vanillaToWire(i.product.vanilla);
      break;
    case "strategy":
      base["strategy"] = {
        kind: e.strategyKind.toWire(i.product.strategy.kind),
        legs: i.product.strategy.legs.map(legToWire),
      };
      break;
    case "singleBarrier":
      base["single_barrier"] = singleBarrierToWire(i.product.singleBarrier);
      break;
    case "doubleBarrier":
      base["double_barrier"] = doubleBarrierToWire(i.product.doubleBarrier);
      break;
    case "digital":
      base["digital"] = digitalToWire(i.product.digital);
      break;
    case "touch":
      base["touch"] = touchToWire(i.product.touch);
      break;
    case "varianceSwap":
      base["variance_swap"] = { strike_vol: i.product.varianceSwap.strikeVol };
      break;
    case "volatilitySwap":
      base["volatility_swap"] = { strike_vol: i.product.volatilitySwap.strikeVol };
      break;
    case "asianOption":
      base["asian_option"] = {
        option_type: e.optionType.toWire(i.product.asianOption.optionType),
        strike: i.product.asianOption.strike,
        averaging: e.averagingStyle.toWire(i.product.asianOption.averaging),
        observations: i.product.asianOption.observations,
        method: e.asianMethod.toWire(i.product.asianOption.method),
        elapsed_avg: i.product.asianOption.elapsedAvg,
        elapsed_weight: i.product.asianOption.elapsedWeight,
      };
      break;
    case "forwardStart":
      base["forward_start"] = {
        option_type: e.optionType.toWire(i.product.forwardStart.optionType),
        moneyness: i.product.forwardStart.moneyness,
        reset: i.product.forwardStart.reset,
      };
      break;
    case "cliquet":
      base["cliquet"] = cliquetToWire(i.product.cliquet);
      break;
    case "quanto":
      base["quanto"] = {
        payoff: e.quantoPayoff.toWire(i.product.quanto.payoff),
        option_type: e.optionType.toWire(i.product.quanto.optionType),
        strike: i.product.quanto.strike,
        conversion_vol: i.product.quanto.conversionVol,
        correlation: i.product.quanto.correlation,
      };
      break;
    case "tarf":
      base["tarf"] = tarfToWire(i.product.tarf);
      break;
    case "accumulator":
      base["accumulator"] = accumulatorToWire(i.product.accumulator);
      break;
    case "lookback":
      base["lookback"] = lookbackToWire(i.product.lookback);
      break;
    case "windowBarrier":
      base["window_barrier"] = windowBarrierToWire(i.product.windowBarrier);
      break;
    case "american":
      base["american"] = americanToWire(i.product.american);
      break;
    case "basket":
      base["basket"] = basketToWire(i.product.basket);
      break;
    case "fxForward":
      base["fx_forward"] = fxForwardToWire(i.product.fxForward);
      break;
    case "fxSwap":
      base["fx_swap"] = {
        near: fxForwardToWire(i.product.fxSwap.near),
        far: fxForwardToWire(i.product.fxSwap.far),
      };
      break;
    case "ndf":
      base["ndf"] = {
        contract_rate: i.product.ndf.contractRate,
        notional: i.product.ndf.notional,
        side: e.side.toWire(i.product.ndf.side),
        fixing: e.fixingSource.toWire(i.product.ndf.fixing),
        settlement_ccy: i.product.ndf.settlementCcy,
      };
      break;
  }
  return base;
}

/**
 * Encode an `FxForward` body — the EXACT shape `fx_forward_from_json` /
 * `fx_swap_from_json` (the near/far legs) decode: `contract_rate`, `notional`, and
 * the directional `side` (proto `Side` enum number; BUY=0). Reused for both an
 * outright forward and each swap leg.
 */
function fxForwardToWire(f: FxForward): WireObject {
  return {
    contract_rate: f.contractRate,
    notional: f.notional,
    side: e.side.toWire(f.side),
  };
}

/**
 * Encode a correlated multi-asset basket body (proto field 25). The EXACT shape
 * `basket_from_json` decodes: a `legs` array of `{pair, weight, spot, vol, r_for}`
 * leg objects (per-leg market data), a row-major `correlations` array (length
 * N²), the option/kind enums, the strike, and the MC knobs following the codec's
 * 64-bit-as-JSON-number convention (server reads with `u64_or_zero`). The reported
 * price carries a `priceStdError` (multi-asset Monte-Carlo).
 */
function basketToWire(b: BasketOption): WireObject {
  return {
    legs: b.legs.map((l) => ({
      pair: ccyPairToWire(l.pair),
      weight: l.weight,
      spot: l.spot,
      vol: l.vol,
      r_for: l.rFor,
    })),
    correlations: [...b.correlations],
    option_type: e.optionType.toWire(b.optionType),
    strike: b.strike,
    kind: e.basketKind.toWire(b.kind),
    mc_paths: b.mcPaths,
    mc_replications: b.mcReplications,
    mc_steps: b.mcSteps,
    mc_seed: Number(b.mcSeed),
  };
}

/**
 * Encode an American / Bermudan option body (proto field 24). The EXACT shape
 * `american_from_json` decodes: `strike` is required; `bermudan_dates` is a JSON
 * array (omitted/empty for AMERICAN); the LSM knobs follow the
 * 64-bit-as-JSON-number convention (server reads with `u64_or_zero`). `lsmPaths`
 * of `0` selects the exact finite-difference engine; `> 0` selects the
 * Longstaff-Schwartz Monte-Carlo engine (which carries a `priceStdError`).
 */
function americanToWire(a: AmericanOption): WireObject {
  return {
    option_type: e.optionType.toWire(a.optionType),
    strike: a.strike,
    exercise_style: e.exerciseStyle.toWire(a.exerciseStyle),
    bermudan_dates: [...a.bermudanDates],
    lsm_paths: a.lsmPaths,
    lsm_exercise_dates: a.lsmExerciseDates,
    lsm_seed: Number(a.lsmSeed),
  };
}

/**
 * Encode a fixing schedule (`fixing_years` + `fixing_notional`) — the same nested
 * shape the server's `fixing_schedule_from_json` reads. Carried under the product
 * body's `schedule` key by the TARF / accumulator encoders.
 */
function fixingScheduleToWire(s: FixingSchedule): WireObject {
  return { fixing_years: [...s.fixingYears], fixing_notional: s.fixingNotional };
}

/**
 * Encode a TARF body (proto field 19). All scalar fields are always present; the
 * fixing schedule nests under `schedule` exactly like the SDK's
 * `Tarf.schedule = Some(FixingSchedule { … })`. `mc_pairs`/`mc_seed` follow the
 * codec's 64-bit-as-JSON-number convention (server reads with `u64_or_zero`).
 */
function tarfToWire(t: Tarf): WireObject {
  return {
    option_type: e.optionType.toWire(t.optionType),
    strike: t.strike,
    target: t.target,
    leverage: t.leverage,
    redemption: e.tarfRedemption.toWire(t.redemption),
    schedule: fixingScheduleToWire(t.schedule),
    mc_pairs: t.mcPairs,
    mc_seed: Number(t.mcSeed),
  };
}

/** Encode an accumulator body (proto field 20) — see `tarfToWire` for the conventions. */
function accumulatorToWire(a: Accumulator): WireObject {
  return {
    pivot: a.pivot,
    barrier: a.barrier,
    leverage: a.leverage,
    monitoring: e.accumulatorMonitoring.toWire(a.monitoring),
    schedule: fixingScheduleToWire(a.schedule),
    mc_pairs: a.mcPairs,
    mc_seed: Number(a.mcSeed),
  };
}

/**
 * Encode a lookback body (proto field 21). `strike`/`observations` are always
 * present (the server ignores them for the FLOATING / CONTINUOUS variants);
 * `mc_pairs`/`mc_seed` follow the 64-bit-as-JSON-number convention.
 */
function lookbackToWire(l: Lookback): WireObject {
  return {
    style: e.lookbackStyle.toWire(l.style),
    option_type: e.optionType.toWire(l.optionType),
    monitoring: e.lookbackMonitoring.toWire(l.monitoring),
    strike: l.strike,
    observations: l.observations,
    mc_pairs: l.mcPairs,
    mc_seed: Number(l.mcSeed),
  };
}

/**
 * Encode a cliquet body. The four clamp fields are presence-tracked exactly like
 * the proto `optional double` / the server's `opt_f64` decoder: a `localFloor`
 * (etc.) left undefined is OMITTED from the JSON (⇒ unconstrained on that side),
 * never sent as a `0` that would mean "floor at zero". `mc_pairs`/`mc_seed` are
 * always present (`0` ⇒ the server's MC default; ignored for a plain ratchet).
 */
function cliquetToWire(c: Cliquet): WireObject {
  const body: WireObject = {
    option_type: e.optionType.toWire(c.optionType),
    moneyness: c.moneyness,
    periods: c.periods,
    mc_pairs: c.mcPairs,
    // 64-bit `mc_seed` is carried as a plain JSON number, exactly like every other
    // 64-bit wire field in this codec (token/ids/nanos) — the server reads it with
    // `u64_or_zero`. (A bigint would not JSON-serialise; seeds stay within the
    // JS safe-integer range.)
    mc_seed: Number(c.mcSeed),
  };
  if (c.localFloor !== undefined) body["local_floor"] = c.localFloor;
  if (c.localCap !== undefined) body["local_cap"] = c.localCap;
  if (c.globalFloor !== undefined) body["global_floor"] = c.globalFloor;
  if (c.globalCap !== undefined) body["global_cap"] = c.globalCap;
  return body;
}

// ---------------------------------------------------------------------------
// vocabulary — decode (wire → GUI)
// ---------------------------------------------------------------------------

export function ccyPairFromWire(o: WireObject): CcyPair {
  return { base: str(o, "base"), quote: str(o, "quote") };
}

export function conventionsFromWire(o: WireObject): Conventions {
  return {
    deltaConvention: e.deltaConvention.fromWire(enumNum(o, "delta_convention")),
    atmConvention: e.atmConvention.fromWire(enumNum(o, "atm_convention")),
    premiumStyle: e.premiumStyle.fromWire(enumNum(o, "premium_style")),
    cut: e.cut.fromWire(enumNum(o, "cut")),
    dayCount: e.dayCount.fromWire(enumNum(o, "day_count")),
    settlement: e.settlement.fromWire(enumNum(o, "settlement")),
  };
}

export function marketFromWire(o: WireObject): MarketContext {
  return { spot: num(o, "spot"), vol: num(o, "vol"), rDom: num(o, "r_dom"), rFor: num(o, "r_for") };
}

export function greeksFromWire(o: WireObject): Greeks {
  return {
    price: num(o, "price"),
    deltaSpot: num(o, "delta_spot"),
    deltaForward: num(o, "delta_forward"),
    gamma: num(o, "gamma"),
    vega: num(o, "vega"),
    theta: num(o, "theta"),
    rhoDom: num(o, "rho_dom"),
    rhoFor: num(o, "rho_for"),
    vanna: num(o, "vanna"),
    volga: num(o, "volga"),
    charm: num(o, "charm"),
    speed: num(o, "speed"),
    zomma: num(o, "zomma"),
    color: num(o, "color"),
  };
}

export function twoWayFromWire(o: WireObject): TwoWayPrice {
  return { bid: num(o, "bid"), offer: num(o, "offer") };
}

function tradableFromWire(o: WireObject): TradableToken {
  return {
    token: numToBigInt(o, "token"),
    side: e.side.fromWire(enumNum(o, "side")),
    premium: num(o, "premium"),
    validUntilNanos: numToBigInt(o, "valid_until_nanos"),
  };
}

function tradableVec(o: WireObject): TradableToken[] {
  return array(o, "tradable").map(tradableFromWire);
}

// ---------------------------------------------------------------------------
// RFQ / pricing — encode requests, decode replies
// ---------------------------------------------------------------------------

export function quoteFromWire(o: WireObject): Quote {
  const q: Quote = {
    quoteId: numToBigInt(o, "quote_id"),
    idempotencyKey: str(o, "idempotency_key"),
    price: twoWayFromWire(child(o, "price")),
    greeks: greeksFromWire(child(o, "greeks")),
    conventions: conventionsFromWire(child(o, "conventions")),
    resolvedStrike: num(o, "resolved_strike"),
    epochNanos: numToBigInt(o, "epoch_nanos"),
    validUntilNanos: numToBigInt(o, "valid_until_nanos"),
  };
  const corr = optBigInt(o, "correlation_id");
  if (corr !== undefined) q.correlationId = corr;
  const surf = optBigInt(o, "surface_version");
  if (surf !== undefined) q.surfaceVersion = surf;
  // The MC standard error (proto `price_std_error`, presence-tracked): set ONLY
  // for a Monte-Carlo-priced product (the clamped cliquet); absent for the
  // closed-form products whose price is exact. Decoded as `undefined` when the
  // server omits it, so the spill never fabricates a precision claim.
  const stdErr = optNum(o, "price_std_error");
  if (stdErr !== undefined) q.priceStdError = stdErr;
  return q;
}

export function executionFromWire(o: WireObject): Omit<Execution, "instrument"> {
  // The wire Execution carries no instrument echo; the caller pairs it with the
  // instrument it accepted from its own quote cache (mirrors the SDK).
  return {
    executionId: numToBigInt(o, "execution_id"),
    quoteId: numToBigInt(o, "quote_id"),
    side: e.side.fromWire(enumNum(o, "side")),
    tradedPremium: num(o, "traded_premium"),
    epochNanos: numToBigInt(o, "epoch_nanos"),
  };
}

// ---------------------------------------------------------------------------
// RFS server messages — decode
// ---------------------------------------------------------------------------

/** A SubscriptionId is `{ value }`; recover the bigint id the GUI keys rows on. */
export function subscriptionIdFromWire(o: WireObject): bigint {
  return numToBigInt(child(o, "subscription"), "value");
}

export function snapshotFromWire(o: WireObject): Snapshot {
  const s: Snapshot = {
    subscriptionId: subscriptionIdFromWire(o),
    sequence: numToBigInt(o, "sequence"),
    price: twoWayFromWire(child(o, "price")),
    greeks: greeksFromWire(child(o, "greeks")),
    vol: num(o, "vol"),
    conventions: conventionsFromWire(child(o, "conventions")),
    resolvedStrike: num(o, "resolved_strike"),
    tradable: tradableVec(o),
    epochNanos: numToBigInt(o, "epoch_nanos"),
  };
  const surf = optBigInt(o, "surface_version");
  if (surf !== undefined) s.surfaceVersion = surf;
  const corr = optBigInt(o, "correlation_id");
  if (corr !== undefined) s.correlationId = corr;
  return s;
}

export function updateFromWire(o: WireObject): Update {
  const u: Update = {
    subscriptionId: subscriptionIdFromWire(o),
    sequence: numToBigInt(o, "sequence"),
    price: twoWayFromWire(child(o, "price")),
    greeks: greeksFromWire(child(o, "greeks")),
    vol: num(o, "vol"),
    tradable: tradableVec(o),
    epochNanos: numToBigInt(o, "epoch_nanos"),
  };
  const surf = optBigInt(o, "surface_version");
  if (surf !== undefined) u.surfaceVersion = surf;
  return u;
}

/**
 * Decode a `heartbeat` frame, including the appended server observability fields.
 * A connection-level beat carries `subscription: null`/absent (⇒ `subscriptionId`
 * undefined); a per-subscription beat carries the id. The conflation-drop count
 * and the three drain-side latency percentiles (ns) and the provenance echo are
 * read verbatim — `0` is an honest value (never lagged / no timed price yet),
 * never fabricated.
 */
export function heartbeatFromWire(o: WireObject): Heartbeat {
  const sub = o["subscription"];
  const hb: Heartbeat = {
    sequence: numToBigInt(o, "sequence"),
    conflationDrops: numToBigInt(o, "conflation_drops"),
    serverPriceP50Nanos: numToBigInt(o, "server_price_p50_nanos"),
    serverPriceP99Nanos: numToBigInt(o, "server_price_p99_nanos"),
    serverPriceP999Nanos: numToBigInt(o, "server_price_p999_nanos"),
    surfaceVersion: numToBigInt(o, "surface_version"),
    correlationId: numToBigInt(o, "correlation_id"),
    epochNanos: numToBigInt(o, "epoch_nanos"),
  };
  if (sub != null && typeof sub === "object") {
    hb.subscriptionId = numToBigInt(child(o, "subscription"), "value");
  }
  return hb;
}

export function executedFromWire(o: WireObject): Executed {
  const ex: Executed = {
    subscriptionId: subscriptionIdFromWire(o),
    token: numToBigInt(o, "token"),
    executionId: numToBigInt(o, "execution_id"),
    side: e.side.fromWire(enumNum(o, "side")),
    tradedPremium: num(o, "traded_premium"),
    epochNanos: numToBigInt(o, "epoch_nanos"),
  };
  const corr = optBigInt(o, "correlation_id");
  if (corr !== undefined) ex.correlationId = corr;
  return ex;
}

export function streamRejectFromWire(o: WireObject): StreamReject {
  const r: StreamReject = {
    subscriptionId: subscriptionIdFromWire(o),
    token: numToBigInt(o, "token"),
    reason: e.streamRejectReason.fromWire(enumNum(o, "reason")),
    epochNanos: numToBigInt(o, "epoch_nanos"),
  };
  const corr = optBigInt(o, "correlation_id");
  if (corr !== undefined) r.correlationId = corr;
  return r;
}

// ---------------------------------------------------------------------------
// market series (trend feed) — decode server frames
// ---------------------------------------------------------------------------

/** Decode one `market_series_point` body (also reused inside a snapshot). */
export function marketSeriesPointFromWire(o: WireObject): MarketSeriesPoint {
  return {
    subscriptionId: subscriptionIdFromWire(o),
    sequence: numToBigInt(o, "sequence"),
    value: num(o, "value"),
    epochNanos: numToBigInt(o, "epoch_nanos"),
  };
}

/** Decode a `market_series_snapshot` body (baseline history + observable id). */
export function marketSeriesSnapshotFromWire(o: WireObject): MarketSeriesSnapshot {
  return {
    subscriptionId: subscriptionIdFromWire(o),
    sequence: numToBigInt(o, "sequence"),
    pair: ccyPairFromWire(child(o, "pair")),
    observable: e.marketObservable.fromWire(enumNum(o, "observable")),
    points: array(o, "points").map(marketSeriesPointFromWire),
    epochNanos: numToBigInt(o, "epoch_nanos"),
  };
}

// ---------------------------------------------------------------------------
// surface — decode replies
// ---------------------------------------------------------------------------

function brokerQuoteSetFromWire(o: WireObject): BrokerQuoteSet {
  return {
    tenorYears: num(o, "tenor_years"),
    atmVol: num(o, "atm_vol"),
    rr25: num(o, "rr_25"),
    bf25: num(o, "bf_25"),
    rr10: num(o, "rr_10"),
    bf10: num(o, "bf_10"),
    hasTenDelta: Boolean(o["has_ten_delta"]),
  };
}

function smilePointFromWire(o: WireObject): SmilePoint {
  return { delta: num(o, "delta"), tenorYears: num(o, "tenor_years"), vol: num(o, "vol") };
}

export function arbReportFromWire(o: WireObject): ArbReport {
  return {
    butterflyArbitrageFree: Boolean(o["butterfly_arbitrage_free"]),
    calendarArbitrageFree: Boolean(o["calendar_arbitrage_free"]),
    worstDensity: num(o, "worst_density"),
    note: str(o, "note"),
    // TYPED authoritative provenance: read the proto `smile_model` enum tag
    // directly (the server stamps it from the calibrated smile's own family).
    // An absent tag is the proto3 zero value (MARKET_HEDGE), matching a server
    // built before this field existed. We never parse the `model=` note token.
    smileModel: e.smileModel.fromWire(enumNum(o, "smile_model")),
  };
}

export function smileFromWire(o: WireObject): Smile {
  return {
    pair: ccyPairFromWire(child(o, "pair")),
    tenorYears: num(o, "tenor_years"),
    // Optional: a calibrated/returned smile may omit the input broker marks
    // (server sends `broker_quotes: null`); only decode when actually present.
    ...(o["broker_quotes"] != null
      ? { brokerQuotes: brokerQuoteSetFromWire(child(o, "broker_quotes")) }
      : {}),
    points: array(o, "points").map(smilePointFromWire),
    conventions: conventionsFromWire(child(o, "conventions")),
    arbitrage: arbReportFromWire(child(o, "arbitrage")),
    epochNanos: numToBigInt(o, "epoch_nanos"),
  };
}

export function markedSurfaceFromWire(o: WireObject): MarkedSurface {
  return {
    pair: ccyPairFromWire(child(o, "pair")),
    surfaceVersion: numToBigInt(o, "surface_version"),
    smiles: array(o, "smiles").map(smileFromWire),
    epochNanos: numToBigInt(o, "epoch_nanos"),
  };
}

export function brokerQuoteSetToWire(b: BrokerQuoteSet): WireObject {
  return {
    tenor_years: b.tenorYears,
    atm_vol: b.atmVol,
    rr_25: b.rr25,
    bf_25: b.bf25,
    rr_10: b.rr10,
    bf_10: b.bf10,
    has_ten_delta: b.hasTenDelta,
  };
}

// ---------------------------------------------------------------------------
// scenario — encode axes, decode the grid + bucketed risk
// ---------------------------------------------------------------------------

export function shockAxisToWire(a: ShockAxis): WireObject {
  return { factor: e.shockFactor.toWire(a.factor), relative: a.relative, steps: a.steps };
}

function scenarioPointFromWire(o: WireObject): ScenarioPoint {
  return {
    appliedShocks: numberArray(o, "applied_shocks"),
    shockedMarket: marketFromWire(child(o, "shocked_market")),
    greeks: greeksFromWire(child(o, "greeks")),
    expiryYears: num(o, "expiry_years"),
  };
}

function numberArray(o: WireObject, key: string): number[] {
  const v = o[key];
  return Array.isArray(v) ? v.filter((x): x is number => typeof x === "number") : [];
}

function vegaBucketFromWire(o: WireObject): VegaBucket {
  return { tenorYears: num(o, "tenor_years"), delta: num(o, "delta"), vega: num(o, "vega") };
}

function crossGammaFromWire(o: WireObject): CrossGamma {
  return {
    factorA: e.shockFactor.fromWire(enumNum(o, "factor_a")),
    factorB: e.shockFactor.fromWire(enumNum(o, "factor_b")),
    value: num(o, "value"),
  };
}

function bucketedRiskFromWire(o: WireObject): BucketedRisk {
  return {
    vegaBuckets: array(o, "vega_buckets").map(vegaBucketFromWire),
    crossGammas: array(o, "cross_gammas").map(crossGammaFromWire),
    thetaRoll: numberArray(o, "theta_roll"),
    rollHorizonsYears: numberArray(o, "roll_horizons_years"),
  };
}

export function scenarioResultFromWire(o: WireObject): ScenarioResult {
  return {
    points: array(o, "points").map(scenarioPointFromWire),
    bucketedRisk: bucketedRiskFromWire(child(o, "bucketed_risk")),
  };
}

/** Re-export the `StrategyKind` type guard surface for callers that need it. */
export type { StrategyKind };
