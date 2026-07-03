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
  AdditiveRisk,
  AggregateRatesRiskRequest,
  AggregateRatesRiskResponse,
  AggregateRiskRequest,
  AggregateRiskResponse,
  KeyRateDv01,
  RatesPosition,
  RatesRiskNode,
  RatesRiskScope,
  ArbReport,
  AttributionRecord,
  BookId,
  BrokerQuoteSet,
  BucketedRisk,
  Capability,
  CapabilityAction,
  CapabilityAsset,
  CcyExposureLeg,
  CcyPair,
  Conventions,
  CrossGamma,
  Deal,
  DealerQuote,
  AcceptDeskQuoteRequest,
  AcceptDeskQuoteResponse,
  BookRatesPositionRequest,
  BookRatesPositionResponse,
  DeskQuote,
  DeskRequest,
  DeskRequestScope,
  ListDealsRequest,
  ListDealsResponse,
  ListDeskRequestsRequest,
  ListDeskRequestsResponse,
  ListRatesPositionsRequest,
  ListRatesPositionsResponse,
  Notification,
  NotificationScope,
  RespondDeskRequestRequest,
  RespondDeskRequestResponse,
  SubmitDeskRequestRequest,
  SubmitDeskRequestResponse,
  DrillRiskRequest,
  DrillRiskResponse,
  EntitlementPrincipal,
  EntitlementRule,
  Executed,
  Execution,
  CreateUserInput,
  DeskDesc,
  EntityDesc,
  EntityInput,
  BookDesc,
  BookInput,
  InstrumentDef,
  InstrumentInput,
  ExternalIdEntry,
  ExternalIdScheme,
  BrokenDate,
  Calendar,
  RatesDayCount,
  BusinessDayConvention,
  Frequency,
  RollConvention,
  CouponType,
  DepositDef,
  FraDef,
  StirFutureDef,
  VanillaIrsDef,
  OisDef,
  BondDef,
  FixConnection,
  FixConnectionKind,
  FixConnectionSpec,
  FixMessage,
  LoginResult,
  UpdateUserInput,
  UserDesc,
  UserRole,
  FixMessagePage,
  FixMsgDirection,
  FixingSchedule,
  Greeks,
  Heartbeat,
  Instrument,
  Leg,
  LimitStatusRequest,
  LimitStatusResponse,
  LimitUtilization,
  ListPositionsRequest,
  ListPositionsResponse,
  MarkedSurface,
  MarketContext,
  MarketObservable,
  MarketSeriesPoint,
  MarketSeriesSnapshot,
  MultiDealerQuote,
  NonAdditiveRisk,
  NumeraireRate,
  OisInstrument,
  OrgKey,
  Owner,
  PillarTenor,
  Quote,
  BondPosition,
  PaymentFrequency,
  RatesAccrualBasis,
  RatesInstrument,
  RatesLegDayCount,
  RatesCurveSet,
  RatesPricingResult,
  BuildCurveRequest,
  CalibratedCurve,
  ReportingNumeraire,
  RiskBucketRequest,
  RiskNode,
  RiskPosition,
  RiskScope,
  RiskVegaPillar,
  ScenarioPoint,
  ScenarioResult,
  ShockAxis,
  Side,
  Smile,
  SmilePoint,
  SmileModel,
  Snapshot,
  Solve,
  StrategyKind,
  StreamReject,
  StrikeOrDelta,
  Tenor,
  TradableToken,
  TwoWayPrice,
  Underlying,
  Update,
  RoleCapabilities,
  UserCapabilities,
  VanillaInputs,
  VegaBucket,
  VegaLadderBucket,
  XvaPricingRequest,
  XvaResult,
  XvaSurvivalCurve,
} from "./contract";
import * as e from "./enums";

/** A decoded server frame is a JSON object with a `type` discriminator. */
export type WireObject = Record<string, unknown>;

// ---------------------------------------------------------------------------
// lossless 64-bit framing — same wire bytes, no precision loss
// ---------------------------------------------------------------------------
//
// The server (`serde_json`) emits proto `uint64`/`int64` fields (tradable `token`,
// `valid_until_nanos`, `epoch_nanos`, ids, `sequence`) as full-precision JSON
// integer literals — many of which exceed `Number.MAX_SAFE_INTEGER` (a minted
// token is a 64-bit value). Plain `JSON.parse` rounds those to the nearest f64,
// which silently corrupts a `token` so that a click-to-trade `Execute` is rejected
// `UNKNOWN_TOKEN`. We therefore parse inbound frames with a tokenizer that keeps
// any integer literal too large to be a safe `Number` as a `bigint`, and we send
// outbound frames with a serializer that writes `bigint` fields as bare integer
// literals. This is NOT a second contract — it is the SAME type-tagged JSON over
// the wire, only parsed/printed without losing the 64-bit identities the contract
// already defines (it is what the codec's `numToBigInt` always intended).

/** Max integer that survives a JS `Number` round-trip without rounding. */
const MAX_SAFE = "9007199254740991";

/** True iff a positive integer's digit string exceeds `Number.MAX_SAFE_INTEGER`. */
function exceedsSafeInteger(digits: string): boolean {
  const d = digits.replace(/^0+(?=\d)/, "");
  if (d.length !== MAX_SAFE.length) return d.length > MAX_SAFE.length;
  return d > MAX_SAFE;
}

/**
 * Parse a JSON text frame WITHOUT losing 64-bit integer precision. Plain
 * `JSON.parse` rounds an integer literal beyond `Number.MAX_SAFE_INTEGER` to the
 * nearest `f64` — which silently corrupts a tradable `token` (a minted 64-bit
 * value) so a click-to-trade `Execute` is rejected `UNKNOWN_TOKEN`. We rewrite any
 * top-level/structural integer literal that is too large into a JSON *string*
 * before parsing; the decoders' `numToBigInt` already recovers a string into the
 * exact `bigint`. Smaller integers and all non-integers are untouched, so the
 * frame is otherwise byte-identical to a normal parse. This is NOT a second
 * contract — it is the SAME type-tagged JSON, only read without precision loss.
 */
export function parseFrame(raw: string): unknown {
  return JSON.parse(requoteLargeIntegers(raw));
}

/**
 * Rewrite JSON `value` positions whose integer literal exceeds the JS safe range
 * into quoted strings, leaving string contents, smaller numbers and structure
 * intact. A small state machine tracks whether we are inside a string so digits
 * inside string values are never touched.
 */
function requoteLargeIntegers(raw: string): string {
  let out = "";
  let i = 0;
  let inString = false;
  const n = raw.length;
  while (i < n) {
    const ch = raw[i]!;
    if (inString) {
      out += ch;
      if (ch === "\\") {
        // Copy the escaped character verbatim.
        if (i + 1 < n) out += raw[i + 1]!;
        i += 2;
        continue;
      }
      if (ch === '"') inString = false;
      i += 1;
      continue;
    }
    if (ch === '"') {
      inString = true;
      out += ch;
      i += 1;
      continue;
    }
    // A number literal can start with `-` or a digit. Consume the WHOLE JSON
    // number (integer + optional fraction + optional exponent) in one pass — never
    // re-scan the fractional/exponent tail as a separate integer — then requote
    // ONLY when the literal is a pure integer beyond the JS safe range.
    if (ch === "-" || (ch >= "0" && ch <= "9")) {
      let j = i;
      if (raw[j] === "-") j += 1;
      const intStart = j;
      while (j < n && raw[j]! >= "0" && raw[j]! <= "9") j += 1;
      const intDigits = raw.slice(intStart, j);
      let isInteger = intDigits.length > 0;
      // Optional fraction.
      if (raw[j] === ".") {
        isInteger = false;
        j += 1;
        while (j < n && raw[j]! >= "0" && raw[j]! <= "9") j += 1;
      }
      // Optional exponent.
      if (raw[j] === "e" || raw[j] === "E") {
        isInteger = false;
        j += 1;
        if (raw[j] === "+" || raw[j] === "-") j += 1;
        while (j < n && raw[j]! >= "0" && raw[j]! <= "9") j += 1;
      }
      const literal = raw.slice(i, j);
      if (isInteger && exceedsSafeInteger(intDigits)) {
        // Quote it so JSON.parse yields a string; numToBigInt recovers the bigint.
        out += `"${literal}"`;
      } else {
        out += literal;
      }
      i = j;
      continue;
    }
    out += ch;
    i += 1;
  }
  return out;
}

/**
 * Serialize an outbound frame, writing any `bigint` value as a bare integer
 * literal (JSON has no bigint, and `JSON.stringify` throws on one). Used for the
 * `Execute` frame so the exact 64-bit `token` is echoed to the server verbatim.
 */
export function serializeFrame(frame: WireObject): string {
  // Tag a bigint as a string `@celnet-bigint@<digits>@`, then unwrap the quoted
  // tag into a bare integer literal. The delimiters are pure ASCII (no whitespace,
  // no JSON-escapable character), so the unwrap regex matches deterministically and
  // the tag cannot collide with any contract string field.
  const text = JSON.stringify(frame, (_key, value: unknown) =>
    typeof value === "bigint" ? `@celnet-bigint@${value.toString()}@` : value,
  );
  return text.replace(/"@celnet-bigint@(-?\d+)@"/g, "$1");
}

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
 * Encode the cross-asset {@link Underlying} oneof (`celnet.wire.Underlying`) under
 * the `underlying` key: the active arm by its proto field NAME (snake_case —
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
          symbol: {
            ticker: u.equity.symbol.ticker,
            venue: u.equity.symbol.venue,
          },
          currency: u.equity.currency,
        },
        settlement_ccy,
      };
    case "commodity":
      return {
        commodity: {
          symbol: {
            ticker: u.commodity.symbol.ticker,
            venue: u.commodity.symbol.venue,
          },
          currency: u.commodity.currency,
        },
        settlement_ccy,
      };
    case "digitalAsset":
      return {
        digital_asset: {
          base: u.digitalAsset.base,
          quote: u.digitalAsset.quote,
        },
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

/** Encode a `Tenor`, including the `broken_date` body for `BROKEN_DATE`. */
export function tenorToWire(t: Tenor): WireObject {
  const w: WireObject = { unit: e.tenorUnit.toWire(t.unit), count: t.count };
  if (t.brokenDate) {
    w["broken_date"] = {
      year: t.brokenDate.year,
      month: t.brokenDate.month,
      day: t.brokenDate.day,
    };
  }
  return w;
}

export function instrumentToWire(i: Instrument): WireObject {
  // The tenor label is presence-tracked: absent ONLY for the one tenorless
  // product (the perpetual option, whose canonical wire shape is
  // `expiry_years: 0` with no `tenor` key — matching the SDK's optional tenor).
  // The conditional spread keeps every dated family's frame byte-identical.
  const base: WireObject = {
    pair: ccyPairToWire(i.pair),
    ...(i.tenor ? { tenor: tenorToWire(i.tenor) } : {}),
    expiry_years: i.expiryYears,
    quantity: { notional: i.quantity.notional, base_ccy: i.quantity.baseCcy },
    side: e.side.toWire(i.side),
  };
  // The cross-asset underlying (`Instrument.underlying`, proto field 1). Emit it
  // ONLY when set: an absent `underlying` keeps the FX `pair` projection the FX
  // surfaces key on (byte-identical to the contract before the cross-asset arms).
  // A non-FX instrument carries the full `Underlying` oneof here AND the leg-string
  // `pair` so the FX-keyed surfaces stay total.
  if (i.underlying) base["underlying"] = underlyingToWire(i.underlying);
  if (i.solve) base["solve"] = solveToWire(i.solve);
  // The settlement-style selector (`Instrument.settlement_style`, proto field 29).
  // Emit it ONLY when non-LINEAR: LINEAR is the proto3 zero value, so omitting it
  // keeps the wire frame byte-identical to the contract before this field existed
  // (the server reads an absent key as LINEAR). INVERSE_COIN is the coin-margined
  // `1/S_T` digital-asset convention, carried as its numeric proto tag.
  if (i.settlementStyle !== undefined && i.settlementStyle !== "LINEAR") {
    base["settlement_style"] = e.settlementStyle.toWire(i.settlementStyle);
  }
  // The pricing/booking model (`Instrument.pricing_model`, proto field 22). It
  // travels uniformly through every flow (price/quote/stream/scenario), exactly
  // like the `Solve` directive — appended additively, no schema_version, no
  // renumber (CLAUDE.md rule 9). Proto3 default-0 (DEFAULT) is OMITTED from the
  // wire object so an analytic instrument is byte-identical to the legacy frame;
  // only LOCAL_STOCH_VOL is emitted. The server's `opt_pricing_model` reads the
  // numeric tag (absent ⇒ DEFAULT).
  if (i.pricingModel !== undefined && i.pricingModel !== "DEFAULT") {
    base["pricing_model"] = e.pricingModel.toWire(i.pricingModel);
  }
  // The product oneof: nest the body under its own key, exactly like the proto.
  // The wire field numbers are: vanilla=7, strategy=8, …, digital=11, touch=12,
  // variance_swap=13, volatility_swap=14, asian_option=15 (appended additively —
  // one current contract, no schema_version; CLAUDE.md rule 9). The WS mirror
  // keys each arm by the proto field NAME (snake_case), matching the server codec.
  switch (i.product.kind) {
    case "vanilla":
      base["vanilla"] = {
        option_type: e.optionType.toWire(i.product.vanilla.optionType),
        strike: strikeOrDeltaToWire(i.product.vanilla.strike),
      };
      break;
    case "strategy":
      base["strategy"] = {
        kind: e.strategyKind.toWire(i.product.strategy.kind),
        legs: i.product.strategy.legs.map(legToWire),
      };
      break;
    // The already-contracted barrier/digital/touch arms (proto field numbers
    // single_barrier=9, double_barrier=10, digital=11, touch=12 — already on the
    // wire AND priced server-side; the GUI ticket now builds them). Each arm uses
    // the EXACT snake_case field NAMES + numeric enum tags the server WS codec
    // decodes (crates/celnet-server/src/ws/codec.rs single_barrier_from_json /
    // double_barrier_from_json / digital_from_json / touch_from_json). The single-
    // and double-barrier reuse the SAME nested `vanilla` message (option_type +
    // strike-or-delta), exactly like the proto.
    case "singleBarrier": {
      const b = i.product.singleBarrier;
      base["single_barrier"] = {
        vanilla: {
          option_type: e.optionType.toWire(b.vanilla.optionType),
          strike: strikeOrDeltaToWire(b.vanilla.strike),
        },
        kind: e.barrierKind.toWire(b.kind),
        side: e.barrierSide.toWire(b.side),
        barrier: b.barrier,
        rebate: b.rebate,
        monitoring: e.monitoringStyle.toWire(b.monitoring),
      };
      break;
    }
    case "doubleBarrier": {
      const b = i.product.doubleBarrier;
      base["double_barrier"] = {
        vanilla: {
          option_type: e.optionType.toWire(b.vanilla.optionType),
          strike: strikeOrDeltaToWire(b.vanilla.strike),
        },
        kind: e.barrierKind.toWire(b.kind),
        lower_barrier: b.lowerBarrier,
        upper_barrier: b.upperBarrier,
        rebate: b.rebate,
        monitoring: e.monitoringStyle.toWire(b.monitoring),
      };
      break;
    }
    case "digital": {
      const d = i.product.digital;
      base["digital"] = {
        option_type: e.optionType.toWire(d.optionType),
        strike: d.strike,
        style: e.digitalStyle.toWire(d.style),
        payout: d.payout,
      };
      break;
    }
    case "touch": {
      const t = i.product.touch;
      base["touch"] = {
        kind: e.touchKind.toWire(t.kind),
        lower_barrier: t.lowerBarrier,
        upper_barrier: t.upperBarrier,
        rebate: t.rebate,
        monitoring: e.monitoringStyle.toWire(t.monitoring),
      };
      break;
    }
    case "varianceSwap":
      base["variance_swap"] = { strike_vol: i.product.varianceSwap.strikeVol };
      break;
    case "volatilitySwap":
      base["volatility_swap"] = {
        strike_vol: i.product.volatilitySwap.strikeVol,
      };
      break;
    case "asianOption": {
      const a = i.product.asianOption;
      base["asian_option"] = {
        option_type: e.optionType.toWire(a.optionType),
        strike: a.strike,
        averaging: e.averagingStyle.toWire(a.averaging),
        observations: a.observations,
        method: e.asianMethod.toWire(a.method),
        elapsed_avg: a.elapsedAvg,
        elapsed_weight: a.elapsedWeight,
      };
      break;
    }
    // Wave-2 products, appended additively at the next field numbers:
    // forward_start=16, cliquet=17, quanto=18 (one current contract, no
    // schema_version, no renumber; CLAUDE.md rule 9). The cliquet's local/global
    // clamps are presence-tracked — an absent clamp is OMITTED from the wire
    // object (proto3 optional), matching the server codec's `opt_f64` reader.
    case "forwardStart": {
      const f = i.product.forwardStart;
      base["forward_start"] = {
        option_type: e.optionType.toWire(f.optionType),
        moneyness: f.moneyness,
        reset: f.reset,
      };
      break;
    }
    case "cliquet": {
      const c = i.product.cliquet;
      const body: WireObject = {
        option_type: e.optionType.toWire(c.optionType),
        moneyness: c.moneyness,
        periods: c.periods,
        mc_pairs: c.mcPairs,
        mc_seed: c.mcSeed,
      };
      if (c.localFloor !== undefined) body["local_floor"] = c.localFloor;
      if (c.localCap !== undefined) body["local_cap"] = c.localCap;
      if (c.globalFloor !== undefined) body["global_floor"] = c.globalFloor;
      if (c.globalCap !== undefined) body["global_cap"] = c.globalCap;
      base["cliquet"] = body;
      break;
    }
    case "quanto": {
      const q = i.product.quanto;
      base["quanto"] = {
        payoff: e.quantoPayoff.toWire(q.payoff),
        option_type: e.optionType.toWire(q.optionType),
        strike: q.strike,
        conversion_vol: q.conversionVol,
        correlation: q.correlation,
      };
      break;
    }
    // Wave-3 products, appended additively at the next field numbers:
    // tarf=19, accumulator=20, lookback=21 (one current contract, no
    // schema_version, no renumber; CLAUDE.md rule 9). TARF and accumulator reuse
    // the SAME nested `FixingSchedule` message (`schedule`); every one is priced by
    // Monte-Carlo except a CONTINUOUS-monitored lookback (exact closed form).
    case "tarf": {
      const t = i.product.tarf;
      base["tarf"] = {
        option_type: e.optionType.toWire(t.optionType),
        strike: t.strike,
        target: t.target,
        leverage: t.leverage,
        redemption: e.tarfRedemption.toWire(t.redemption),
        schedule: fixingScheduleToWire(t.schedule),
        mc_pairs: t.mcPairs,
        mc_seed: t.mcSeed,
      };
      break;
    }
    // The pivot TRA, appended additively at pivot=32 (one current contract, no
    // schema_version, no renumber; CLAUDE.md rule 9): the TARF body plus the
    // distinct `pivot` level, reusing the SAME nested `FixingSchedule` and
    // `TarfRedemption` vocabulary. Fields match the server WS codec's
    // `pivot_from_json` (option_type=1, strike=2, pivot=3, target=4, leverage=5,
    // redemption=6, schedule=7, mc_pairs=8, mc_seed=9). Always Monte-Carlo.
    case "pivot": {
      const p = i.product.pivot;
      base["pivot"] = {
        option_type: e.optionType.toWire(p.optionType),
        strike: p.strike,
        pivot: p.pivot,
        target: p.target,
        leverage: p.leverage,
        redemption: e.tarfRedemption.toWire(p.redemption),
        schedule: fixingScheduleToWire(p.schedule),
        mc_pairs: p.mcPairs,
        mc_seed: p.mcSeed,
      };
      break;
    }
    case "accumulator": {
      const a = i.product.accumulator;
      base["accumulator"] = {
        pivot: a.pivot,
        barrier: a.barrier,
        leverage: a.leverage,
        monitoring: e.accumulatorMonitoring.toWire(a.monitoring),
        schedule: fixingScheduleToWire(a.schedule),
        mc_pairs: a.mcPairs,
        mc_seed: a.mcSeed,
      };
      break;
    }
    case "lookback": {
      const l = i.product.lookback;
      base["lookback"] = {
        style: e.lookbackStyle.toWire(l.style),
        option_type: e.optionType.toWire(l.optionType),
        monitoring: e.lookbackMonitoring.toWire(l.monitoring),
        strike: l.strike,
        observations: l.observations,
        mc_pairs: l.mcPairs,
        mc_seed: l.mcSeed,
      };
      break;
    }
    // The window barrier, appended additively at the next field number
    // window_barrier=23 (one current contract, no schema_version, no renumber;
    // CLAUDE.md rule 9). It REUSES the same nested `vanilla` message (option_type +
    // strike-or-delta), exactly like the single/double barrier. Fields match the
    // server WS codec's `window_barrier_from_json` (vanilla=1, barrier=2, side=3,
    // window_start=4, window_end=5, mc_pairs=6, mc_steps=7, mc_seed=8). It is
    // LOCAL_STOCH_VOL-only; the model rides on the instrument's `pricing_model`.
    case "windowBarrier": {
      const wb = i.product.windowBarrier;
      base["window_barrier"] = {
        vanilla: {
          option_type: e.optionType.toWire(wb.vanilla.optionType),
          strike: strikeOrDeltaToWire(wb.vanilla.strike),
        },
        barrier: wb.barrier,
        side: e.barrierSide.toWire(wb.side),
        window_start: wb.windowStart,
        window_end: wb.windowEnd,
        mc_pairs: wb.mcPairs,
        mc_steps: wb.mcSteps,
        mc_seed: wb.mcSeed,
      };
      break;
    }
    // The American / Bermudan early-exercise vanilla, appended additively at
    // american=24 (one current contract, no schema_version; CLAUDE.md rule 9).
    // Fields match the server WS codec's `american_from_json` (option_type=1,
    // strike=2, exercise_style=3, bermudan_dates=4, lsm_paths=5,
    // lsm_exercise_dates=6, lsm_seed=7). `lsm_paths == 0` selects the exact FD
    // engine; `> 0` selects the Longstaff-Schwartz MC engine (carries a
    // price_std_error).
    case "american": {
      const a = i.product.american;
      base["american"] = {
        option_type: e.optionType.toWire(a.optionType),
        strike: a.strike,
        exercise_style: e.exerciseStyle.toWire(a.exerciseStyle),
        bermudan_dates: [...a.bermudanDates],
        lsm_paths: a.lsmPaths,
        lsm_exercise_dates: a.lsmExerciseDates,
        lsm_seed: a.lsmSeed,
      };
      break;
    }
    // The correlated multi-asset basket, appended additively at the next field
    // number basket=25 (one current contract, no schema_version, no renumber;
    // CLAUDE.md rule 9). Per-leg market data travels IN each leg (the single-pair
    // market context cannot hold N underlyings); the shared domestic rate is the
    // request market context's r_dom. Fields match the server WS codec's
    // `basket_from_json` (legs=1, correlations=2, option_type=3, strike=4, kind=5,
    // mc_paths=6, mc_replications=7, mc_steps=8, mc_seed=9). Multi-asset Monte-
    // Carlo, so the price carries a `price_std_error`.
    case "basket": {
      const b = i.product.basket;
      base["basket"] = {
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
        mc_seed: b.mcSeed,
      };
      break;
    }
    // The W2 linear products (the `celnet-linear` leaf), appended additively at
    // fx_forward=26, fx_swap=27, ndf=28 (one current contract, no schema_version,
    // no renumber; CLAUDE.md rule 9). Closed-form discounted cashflows (NOT option
    // payoffs); each arm uses the EXACT snake_case field NAMES + numeric enum tags
    // the server WS codec decodes. The Side enum rides each leg (BUY=0/SELL=1).
    case "fxForward": {
      const f = i.product.fxForward;
      base["fx_forward"] = {
        contract_rate: f.contractRate,
        notional: f.notional,
        side: e.side.toWire(f.side),
      };
      break;
    }
    // The FX swap nests two FxForward legs (near=1, far=2). The near leg settles at
    // the spot date and the far leg at the instrument's tenor; by convention the
    // far leg trades the OPPOSITE side to the near leg (enforced by the builder).
    case "fxSwap": {
      const s = i.product.fxSwap;
      const legToWire = (leg: typeof s.near): WireObject => ({
        contract_rate: leg.contractRate,
        notional: leg.notional,
        side: e.side.toWire(leg.side),
      });
      base["fx_swap"] = {
        near: legToWire(s.near),
        far: legToWire(s.far),
      };
      break;
    }
    // The non-deliverable forward (fields contract_rate=1, notional=2, side=3,
    // fixing=4, settlement_ccy=5). `fixing` carries the published settlement-rate
    // IDENTITY only (the live fixing value is an estate-gated feed, never in-repo).
    case "ndf": {
      const n = i.product.ndf;
      base["ndf"] = {
        contract_rate: n.contractRate,
        notional: n.notional,
        side: e.side.toWire(n.side),
        fixing: e.fixingSource.toWire(n.fixing),
        settlement_ccy: n.settlementCcy,
      };
      break;
    }
    // The new payoff shapes, appended additively at perpetual_option=30 /
    // listed_future_option=31 (one current contract, no schema_version, no
    // renumber; CLAUDE.md rule 9). Fields match the server WS codec's
    // `perpetual_option_from_json` (option_type=1, strike=2, notional=3) and
    // `listed_future_option_from_json` (future_symbol=1, future_expiry_years=2,
    // option_type=3, strike=4, notional=5, margining=6). The perpetual rides an
    // instrument with `expiry_years: 0` exactly and NO `tenor` key (the
    // contract's canonical no-expiry shape, enforced by the server validator).
    case "perpetualOption": {
      const p = i.product.perpetualOption;
      base["perpetual_option"] = {
        option_type: e.optionType.toWire(p.optionType),
        strike: p.strike,
        notional: p.notional,
      };
      break;
    }
    case "listedFutureOption": {
      const o = i.product.listedFutureOption;
      base["listed_future_option"] = {
        future_symbol: {
          ticker: o.futureSymbol.ticker,
          venue: o.futureSymbol.venue,
        },
        future_expiry_years: o.futureExpiryYears,
        option_type: e.optionType.toWire(o.optionType),
        strike: o.strike,
        notional: o.notional,
        margining: e.margining.toWire(o.margining),
      };
      break;
    }
  }
  return base;
}

/**
 * Encode a `FixingSchedule` into its nested wire body (the `schedule` field of a
 * TARF / accumulator). The server reads `fixing_years` (the ascending year
 * fractions) and `fixing_notional` (matching `fixing_schedule_from_json`).
 */
function fixingScheduleToWire(s: FixingSchedule): WireObject {
  return {
    fixing_years: [...s.fixingYears],
    fixing_notional: s.fixingNotional,
  };
}

// ---------------------------------------------------------------------------
// vocabulary — decode (wire → GUI)
// ---------------------------------------------------------------------------

export function ccyPairFromWire(o: WireObject): CcyPair {
  return { base: str(o, "base"), quote: str(o, "quote") };
}

// --- fixed-income (rates): PriceRates request/response ----------------------
//
// The browser JSON projection of the `celnet.wire` rates messages. The request
// body matches `RatesPriceRequest` minus the framing the `WsConnection` injects
// (request_id / correlation_id); the reply is the `rates_price_response` frame's
// `result` child. The OIS `direction` projects onto the wire `Side` integer
// (PAY_FIXED = SIDE_BUY = 0, RECEIVE_FIXED = SIDE_SELL = 1), exactly as the
// server's `ois_instrument_from_json` decodes it.

/** Encode a `PillarTenor` to its single-arm wire object `{ years | months | maturity_date }`. */
function pillarTenorToWire(tenor: PillarTenor): WireObject {
  switch (tenor.kind) {
    case "years":
      return { years: tenor.years };
    case "months":
      return { months: tenor.months };
    case "date":
      return {
        maturity_date: {
          year: tenor.maturityDate.year,
          month: tenor.maturityDate.month,
          day: tenor.maturityDate.day,
        },
      };
  }
}

/** Encode a `RatesCurveSet` to the wire `curve_set` object. */
export function ratesCurveSetToWire(curve: RatesCurveSet): WireObject {
  return {
    currency: curve.currency,
    reference_date: {
      year: curve.referenceDate.year,
      month: curve.referenceDate.month,
      day: curve.referenceDate.day,
    },
    ois_pillars: curve.pillars.map((p) => ({
      tenor: pillarTenorToWire(p.tenor),
      par_rate: p.parRate,
    })),
  };
}

/** The wire `Side` code for an OIS direction (PAY_FIXED → BUY = 0; RECEIVE_FIXED → SELL = 1). */
function oisDirectionToSide(direction: OisInstrument["direction"]): number {
  return direction === "RECEIVE_FIXED" ? 1 : 0;
}

/** Encode an `OisInstrument` to the wire `instrument` object (the OIS oneof arm). */
export function ratesInstrumentToWire(instrument: OisInstrument): WireObject {
  return {
    ois: {
      tenor_years: instrument.tenorYears,
      fixed_rate: instrument.fixedRate,
      notional: instrument.notional,
      side: oisDirectionToSide(instrument.direction),
    },
  };
}

/** The wire `Side` code for a bond position (LONG → BUY = 0; SHORT → SELL = 1). */
function bondPositionToSide(position: BondPosition): number {
  return position === "SHORT" ? 1 : 0;
}

/** The wire `PaymentFrequency` int: ANNUAL = 0, SEMI_ANNUAL = 1, QUARTERLY = 2. */
function paymentFrequencyToWire(freq: PaymentFrequency): number {
  switch (freq) {
    case "ANNUAL":
      return 0;
    case "SEMI_ANNUAL":
      return 1;
    case "QUARTERLY":
      return 2;
  }
}

/** The wire `DayCount` int (the curve/leg enum): ACT_365_FIXED = 0, ACT_360 = 1. */
function ratesLegDayCountToWire(dc: RatesLegDayCount): number {
  return dc === "ACT_360" ? 1 : 0;
}

/** The wire `AccrualBasis` int: ACT_360 = 0, ACT_365_FIXED = 1, THIRTY_360_BOND_BASIS = 2. */
function ratesAccrualBasisToWire(basis: RatesAccrualBasis): number {
  switch (basis) {
    case "ACT_360":
      return 0;
    case "ACT_365_FIXED":
      return 1;
    case "THIRTY_360_BOND_BASIS":
      return 2;
  }
}

/**
 * Encode a `RatesInstrument` oneof to its wire `instrument` object — the
 * `ois` / `irs` / `fra` / `bond` arm. Byte-compatible with the server
 * `rates_instrument_from_json` decoder (`crates/celnet-server/src/ws/codec.rs`):
 * the exact snake_case field names and integer enum codes it reads. The OIS arm
 * reuses {@link ratesInstrumentToWire} verbatim (one encoding, no duplication).
 */
export function ratesInstrumentUnionToWire(instrument: RatesInstrument): WireObject {
  switch (instrument.kind) {
    case "ois":
      return ratesInstrumentToWire(instrument.ois);
    case "irs": {
      const i = instrument.irs;
      return {
        irs: {
          tenor_years: i.tenorYears,
          fixed_rate: i.fixedRate,
          notional: i.notional,
          side: oisDirectionToSide(i.direction),
          fixed_frequency: paymentFrequencyToWire(i.fixedFrequency),
          fixed_day_count: ratesLegDayCountToWire(i.fixedDayCount),
          float_frequency: paymentFrequencyToWire(i.floatFrequency),
          float_day_count: ratesLegDayCountToWire(i.floatDayCount),
        },
      };
    }
    case "fra": {
      const f = instrument.fra;
      return {
        fra: {
          start_months: f.startMonths,
          end_months: f.endMonths,
          fixed_rate: f.fixedRate,
          notional: f.notional,
          side: oisDirectionToSide(f.direction),
          accrual_basis: ratesAccrualBasisToWire(f.accrualBasis),
        },
      };
    }
    case "bond": {
      const b = instrument.bond;
      return {
        bond: {
          coupon_rate: b.couponRate,
          coupon_frequency: paymentFrequencyToWire(b.couponFrequency),
          day_count: ratesAccrualBasisToWire(b.dayCount),
          maturity_date: {
            year: b.maturityDate.year,
            month: b.maturityDate.month,
            day: b.maturityDate.day,
          },
          redemption: b.redemption,
          side: bondPositionToSide(b.position),
        },
      };
    }
  }
}

/** Decode the `rates_price_response` frame's `result` into a `RatesPricingResult`. */
export function ratesPricingResultFromWire(o: WireObject): RatesPricingResult {
  const result = child(o, "result");
  const raw = result["key_rate_ladder"];
  if (!Array.isArray(raw)) {
    throw new Error(
      "`rates_price_response.result.key_rate_ladder` must be an array",
    );
  }
  const keyRateLadder = raw.map((v, i) => {
    if (typeof v !== "number" || !Number.isFinite(v)) {
      throw new Error(
        `\`result.key_rate_ladder[${i}]\` must be a finite number`,
      );
    }
    return v;
  });
  return {
    pv: num(result, "pv"),
    parRate: num(result, "par_rate"),
    pv01: num(result, "pv01"),
    dv01: num(result, "dv01"),
    keyRateLadder,
  };
}

// --- XVA (PricingService.PriceXva) -------------------------------------------
// Byte-compatible with the server `ws::codec` XVA codec (`price_xva_request_from_json`
// / `price_xva_response_to_json`): the request is the snake_case, numeric-enum
// netting-set body the server decodes; the response reads the `result` object the
// server encodes ({ cva, dva, fva, total_adjustment }). One unversioned contract.

/** Encode one `XvaSurvivalCurve` (`{ pillar_times, hazard_rates }`). */
function xvaSurvivalCurveToWire(c: XvaSurvivalCurve): WireObject {
  return {
    pillar_times: [...c.pillarTimes],
    hazard_rates: [...c.hazardRates],
  };
}

/** Encode a `PriceXvaRequest` to the wire body the server's `price_xva` decodes. */
export function priceXvaRequestToWire(r: XvaPricingRequest): WireObject {
  return {
    trades: r.trades.map((t) => ({
      option_type: e.optionType.toWire(t.optionType),
      strike: t.strike,
      expiry_years: t.expiryYears,
      vol: t.vol,
      notional: t.notional,
    })),
    r_dom: r.rDom,
    r_for: r.rFor,
    spot0: r.spot0,
    sigma: r.sigma,
    paths: r.paths,
    seed: r.seed,
    exposure_steps: r.exposureSteps,
    counterparty: xvaSurvivalCurveToWire(r.counterparty),
    own: xvaSurvivalCurveToWire(r.own),
    lgd_counterparty: r.lgdCounterparty,
    lgd_own: r.lgdOwn,
    funding_spread: r.fundingSpread,
  };
}

/** Decode the `price_xva_response` frame's `result` into an `XvaResult`. */
export function xvaResultFromWire(o: WireObject): XvaResult {
  const result = child(o, "result");
  return {
    cva: num(result, "cva"),
    dva: num(result, "dva"),
    fva: num(result, "fva"),
    totalAdjustment: num(result, "total_adjustment"),
  };
}

// --- rates portfolio risk (RiskService.AggregateRatesRisk) -------------------
// Byte-compatible with the server `ws::codec` rates-risk codec: the request
// reuses the shared `price_rates` curve/instrument encoders, and the response
// mirrors the per-currency `RatesRiskNode` + tenor ladder the server emits.

/** Encode one `RatesPosition` to its wire object (the OIS oneof + booking cell). */
function ratesPositionToWire(p: RatesPosition): WireObject {
  return {
    // `position_id` is a wire `uint64`; the connection's other ids ride as JSON
    // numbers, so narrow the bigint exactly as the correlation id is narrowed.
    position_id: Number(p.positionId),
    entity: p.entity,
    book: p.book,
    instrument: ratesInstrumentToWire(p.instrument),
  };
}

/** Encode the optional `(entity, book, ccy)` scope; absent fields are omitted. */
function ratesRiskScopeToWire(s: RatesRiskScope): WireObject {
  const w: WireObject = {};
  if (s.entity !== undefined) w["entity"] = s.entity;
  if (s.book !== undefined) w["book"] = s.book;
  if (s.ccy !== undefined) w["ccy"] = s.ccy;
  return w;
}

export function aggregateRatesRiskRequestToWire(
  r: AggregateRatesRiskRequest,
): WireObject {
  const w: WireObject = {
    curve_set: ratesCurveSetToWire(r.curveSet),
    positions: r.positions.map(ratesPositionToWire),
  };
  // The audited explicit grant-all default clears the server's deny-by-default
  // boundary, exactly as the options `aggregate_risk` request does.
  w["principal"] = principalOrGrantAllToWire(r.principal);
  if (r.scope) w["scope"] = ratesRiskScopeToWire(r.scope);
  return w;
}

/** Decode one key-rate DV01 ladder bucket. */
function keyRateDv01FromWire(o: WireObject): KeyRateDv01 {
  return { tenorYears: num(o, "tenor_years"), dv01: num(o, "dv01") };
}

/** Decode one per-currency `RatesRiskNode` (netted scalars + tenor ladder). */
function ratesRiskNodeFromWire(o: WireObject): RatesRiskNode {
  return {
    ccy: str(o, "ccy"),
    netPv: num(o, "net_pv"),
    netPv01: num(o, "net_pv01"),
    netDv01: num(o, "net_dv01"),
    keyRateLadder: array(o, "key_rate_ladder").map(keyRateDv01FromWire),
  };
}

export function aggregateRatesRiskResponseFromWire(
  o: WireObject,
): AggregateRatesRiskResponse {
  const res: AggregateRatesRiskResponse = {
    nodes: array(o, "nodes").map(ratesRiskNodeFromWire),
  };
  const corr = optBigInt(o, "correlation_id");
  if (corr !== undefined) res.correlationId = corr;
  return res;
}

// ---------------------------------------------------------------------------
// dealer-quoting desk + rates Book/List + notification push — the WS mirror of
// RfqDeskService / RiskService(BookRatesPosition, ListRatesPositions) /
// NotificationService (crates/celnet-server/src/ws/codec.rs). Reuses the shared
// `price_rates` curve/instrument encoders and the `RatesPosition` codec so the
// desk path speaks the IDENTICAL market shape. Enums are numeric proto tags; the
// desk enums reserve `0` for an UNSPECIFIED member (see enums.ts offset codecs).
// Request `correlation_id` is the WS-framing routing id the `WsConnection`
// injects, so the GUI omits the (optional) business `correlation_id` here.
// ---------------------------------------------------------------------------

/**
 * Decode the OIS arm of a wire `RatesInstrument` into an `OisInstrument`. The
 * inverse of `ratesInstrumentToWire`: the wire `side` carries the OIS direction
 * (PAY_FIXED → BUY = 0, RECEIVE_FIXED → SELL = 1).
 */
function ratesInstrumentFromWire(o: WireObject): OisInstrument {
  const ois = child(o, "ois");
  return {
    tenorYears: num(ois, "tenor_years"),
    fixedRate: num(ois, "fixed_rate"),
    notional: num(ois, "notional"),
    direction: enumNum(ois, "side") === 1 ? "RECEIVE_FIXED" : "PAY_FIXED",
  };
}

/** Decode a wire `PillarTenor` `{ years | months | maturity_date }` into its arm. */
function pillarTenorFromWire(o: WireObject): PillarTenor {
  const t = child(o, "tenor");
  if ("years" in t) return { kind: "years", years: num(t, "years") };
  if ("months" in t) return { kind: "months", months: num(t, "months") };
  if ("maturity_date" in t) {
    const d = child(t, "maturity_date");
    return {
      kind: "date",
      maturityDate: {
        year: num(d, "year"),
        month: num(d, "month"),
        day: num(d, "day"),
      },
    };
  }
  throw new Error(
    "pillar tenor: expected a years, months, or maturity_date arm",
  );
}

/** Decode a wire `CurveSet` into a `RatesCurveSet`. */
function ratesCurveSetFromWire(o: WireObject): RatesCurveSet {
  const ref = child(o, "reference_date");
  return {
    currency: str(o, "currency"),
    referenceDate: {
      year: num(ref, "year"),
      month: num(ref, "month"),
      day: num(ref, "day"),
    },
    pillars: array(o, "ois_pillars").map((p) => ({
      tenor: pillarTenorFromWire(p),
      parRate: num(p, "par_rate"),
    })),
  };
}

/**
 * Encode a {@link BuildCurveRequest} to the wire `build_curve` request body. The
 * bearer `session_token` + framing `correlation_id` are auto-injected by
 * `WsConnection.request`, so they are NOT set here (mirrors the other gated frames).
 */
export function buildCurveRequestToWire(req: BuildCurveRequest): WireObject {
  return {
    request_id: req.requestId,
    currency: req.currency,
    reference_date: {
      year: req.referenceDate.year,
      month: req.referenceDate.month,
      day: req.referenceDate.day,
    },
    pillars: req.pillars.map((p) => ({
      instrument_id: p.instrumentId,
      quote: p.quote,
    })),
    date_pillars: req.datePillars.map((p) => ({
      maturity_date: {
        year: p.maturityDate.year,
        month: p.maturityDate.month,
        day: p.maturityDate.day,
      },
      quote: p.quote,
    })),
  };
}

/** Decode a wire `calibrated_curve` frame into a {@link CalibratedCurve}. */
export function calibratedCurveFromWire(o: WireObject): CalibratedCurve {
  const ref = child(o, "reference_date");
  return {
    requestId: str(o, "request_id"),
    currency: str(o, "currency"),
    referenceDate: {
      year: num(ref, "year"),
      month: num(ref, "month"),
      day: num(ref, "day"),
    },
    points: array(o, "points").map((p) => ({
      instrumentId: str(p, "instrument_id"),
      timeYears: num(p, "time_years"),
      discountFactor: num(p, "discount_factor"),
      zeroRate: num(p, "zero_rate"),
      label: str(p, "label"),
    })),
  };
}

/** Decode a wire `RatesPosition` (the inverse of `ratesPositionToWire`). */
export function ratesPositionFromWire(o: WireObject): RatesPosition {
  return {
    positionId: numToBigInt(o, "position_id"),
    entity: num(o, "entity"),
    book: num(o, "book"),
    instrument: ratesInstrumentFromWire(child(o, "instrument")),
  };
}

/** Encode a `DeskQuote` to its wire object. */
export function deskQuoteToWire(q: DeskQuote): WireObject {
  return {
    price: q.price,
    notional: q.notional,
    valid_for_ms: q.validForMs,
    trader: q.trader,
  };
}

/** Decode a presence-tracked wire `DeskQuote` (`null`/absent ⇒ undefined). */
function deskQuoteFromWire(o: WireObject, key: string): DeskQuote | undefined {
  const v = o[key];
  if (!v || typeof v !== "object") return undefined;
  const q = v as WireObject;
  return {
    price: num(q, "price"),
    notional: num(q, "notional"),
    validForMs: num(q, "valid_for_ms"),
    trader: str(q, "trader"),
  };
}

/** Decode a wire `DeskRequest`. */
export function deskRequestFromWire(o: WireObject): DeskRequest {
  const r: DeskRequest = {
    requestId: str(o, "request_id"),
    kind: e.deskRequestKind.fromWire(enumNum(o, "kind")),
    counterparty: str(o, "counterparty"),
    desk: str(o, "desk"),
    instrument: ratesInstrumentFromWire(child(o, "instrument")),
    curveSet: ratesCurveSetFromWire(child(o, "curve_set")),
    side: e.side.fromWire(enumNum(o, "side")),
    notional: num(o, "notional"),
    receivedAtNanos: numToBigInt(o, "received_at_nanos"),
    expiresAtNanos: numToBigInt(o, "expires_at_nanos"),
    state: e.deskRequestState.fromWire(enumNum(o, "state")),
  };
  const quote = deskQuoteFromWire(o, "quote");
  if (quote !== undefined) r.quote = quote;
  const corr = o["correlation_id"];
  if (typeof corr === "string" && corr.length > 0) r.correlationId = corr;
  return r;
}

/** Decode a wire `Deal`. */
export function dealFromWire(o: WireObject): Deal {
  const d: Deal = {
    dealId: str(o, "deal_id"),
    requestId: str(o, "request_id"),
    kind: e.deskRequestKind.fromWire(enumNum(o, "kind")),
    counterparty: str(o, "counterparty"),
    desk: str(o, "desk"),
    instrument: ratesInstrumentFromWire(child(o, "instrument")),
    curveSet: ratesCurveSetFromWire(child(o, "curve_set")),
    side: e.side.fromWire(enumNum(o, "side")),
    notional: num(o, "notional"),
    price: num(o, "price"),
    executedAtNanos: numToBigInt(o, "executed_at_nanos"),
    trader: str(o, "trader"),
  };
  const pid = optBigInt(o, "position_id");
  if (pid !== undefined) d.positionId = pid;
  const corr = o["correlation_id"];
  if (typeof corr === "string" && corr.length > 0) d.correlationId = corr;
  return d;
}

/** Decode a wire `Notification` push frame (`type: "notification"`). */
export function notificationFromWire(o: WireObject): Notification {
  const n: Notification = {
    notificationId: str(o, "notification_id"),
    kind: e.notificationKind.fromWire(enumNum(o, "kind")),
    atNanos: numToBigInt(o, "at_nanos"),
    desk: str(o, "desk"),
    counterparty: str(o, "counterparty"),
    requestKind: e.deskRequestKind.fromWire(enumNum(o, "request_kind")),
    headline: str(o, "headline"),
  };
  const rid = o["request_id"];
  if (typeof rid === "string" && rid.length > 0) n.requestId = rid;
  const detail = o["detail"];
  if (typeof detail === "string" && detail.length > 0) n.detail = detail;
  return n;
}

// --- desk request/response encoders + decoders ------------------------------

export function submitDeskRequestToWire(
  r: SubmitDeskRequestRequest,
): WireObject {
  return {
    kind: e.deskRequestKind.toWire(r.kind),
    counterparty: r.counterparty,
    desk: r.desk,
    instrument: ratesInstrumentToWire(r.instrument),
    curve_set: ratesCurveSetToWire(r.curveSet),
    side: e.side.toWire(r.side),
    notional: r.notional,
    ttl_ms: r.ttlMs,
    principal: principalOrGrantAllToWire(r.principal),
  };
}

export function submitDeskRequestResponseFromWire(
  o: WireObject,
): SubmitDeskRequestResponse {
  return { request: deskRequestFromWire(child(o, "request")) };
}

export function respondDeskRequestToWire(
  r: RespondDeskRequestRequest,
): WireObject {
  const w: WireObject = {
    request_id: r.requestId,
    principal: principalOrGrantAllToWire(r.principal),
  };
  // Exactly one oneof arm — `quote` OR `reject` — matching the server codec's
  // `respond_desk_request_from_json` (it rejects a frame carrying both).
  if (r.response.kind === "quote") {
    w["quote"] = deskQuoteToWire(r.response.quote);
  } else {
    w["reject"] = { reason: r.response.reject.reason };
  }
  return w;
}

export function respondDeskRequestResponseFromWire(
  o: WireObject,
): RespondDeskRequestResponse {
  return { request: deskRequestFromWire(child(o, "request")) };
}

export function acceptDeskQuoteToWire(r: AcceptDeskQuoteRequest): WireObject {
  return {
    request_id: r.requestId,
    principal: principalOrGrantAllToWire(r.principal),
  };
}

export function acceptDeskQuoteResponseFromWire(
  o: WireObject,
): AcceptDeskQuoteResponse {
  return {
    deal: dealFromWire(child(o, "deal")),
    request: deskRequestFromWire(child(o, "request")),
  };
}

/** Encode the optional `(states, desk)` desk-request scope; absent fields omitted. */
function deskRequestScopeToWire(s: DeskRequestScope): WireObject {
  const w: WireObject = {};
  if (s.states && s.states.length > 0) {
    w["states"] = s.states.map((st) => e.deskRequestState.toWire(st));
  }
  if (s.desk !== undefined) w["desk"] = s.desk;
  return w;
}

export function listDeskRequestsToWire(r: ListDeskRequestsRequest): WireObject {
  const w: WireObject = { principal: principalOrGrantAllToWire(r.principal) };
  if (r.scope) w["scope"] = deskRequestScopeToWire(r.scope);
  return w;
}

export function listDeskRequestsResponseFromWire(
  o: WireObject,
): ListDeskRequestsResponse {
  return { requests: array(o, "requests").map(deskRequestFromWire) };
}

export function listDealsToWire(r: ListDealsRequest): WireObject {
  const w: WireObject = { principal: principalOrGrantAllToWire(r.principal) };
  if (r.scope) {
    const s: WireObject = {};
    if (r.scope.desk !== undefined) s["desk"] = r.scope.desk;
    w["scope"] = s;
  }
  return w;
}

export function listDealsResponseFromWire(o: WireObject): ListDealsResponse {
  return { deals: array(o, "deals").map(dealFromWire) };
}

// --- rates Book/List encoders + decoders ------------------------------------

export function bookRatesPositionToWire(
  r: BookRatesPositionRequest,
): WireObject {
  return {
    position: ratesPositionToWire(r.position),
    principal: principalOrGrantAllToWire(r.principal),
  };
}

export function bookRatesPositionResponseFromWire(
  o: WireObject,
): BookRatesPositionResponse {
  return { position: ratesPositionFromWire(child(o, "position")) };
}

export function listRatesPositionsToWire(
  r: ListRatesPositionsRequest,
): WireObject {
  const w: WireObject = { principal: principalOrGrantAllToWire(r.principal) };
  if (r.scope) w["scope"] = ratesRiskScopeToWire(r.scope);
  return w;
}

export function listRatesPositionsResponseFromWire(
  o: WireObject,
): ListRatesPositionsResponse {
  return { positions: array(o, "positions").map(ratesPositionFromWire) };
}

// --- notification push subscribe body ---------------------------------------

/**
 * Encode the `subscribe_notifications` control-frame body (the `type` /
 * `session_token` are added by the connection). Asserts an explicit grant-all
 * principal exactly as the risk requests do, so the stream clears the server's
 * deny-by-default boundary.
 */
export function subscribeNotificationsToWire(
  scope: NotificationScope | undefined,
): WireObject {
  const w: WireObject = { principal: principalOrGrantAllToWire(undefined) };
  if (scope) w["scope"] = { desks: [...scope.desks] };
  return w;
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
  return {
    spot: num(o, "spot"),
    vol: num(o, "vol"),
    rDom: num(o, "r_dom"),
    rFor: num(o, "r_for"),
  };
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
// attribution (book / seat identity) — decode + encode
// ---------------------------------------------------------------------------
//
// The contract's `AttributionRecord` uses camelCase keys on the wire mirror
// (`quotedBy`/`heldBy`/`bookId`/`owner`/`trader`/`autoPricer`/`won`/`lpCount`) —
// matching the server codec's `attribution_from_json`. Exactly one owner-seat arm
// is set. Absent ⇒ `undefined`, so a consumer renders an honest "—" rather than a
// fabricated seat.

function ownerFromWire(o: WireObject): Owner | undefined {
  const v = o["owner"];
  if (!v || typeof v !== "object") return undefined;
  const ow = v as WireObject;
  if (typeof ow["trader"] === "string")
    return { kind: "trader", trader: ow["trader"] };
  if (typeof ow["autoPricer"] === "string") {
    return { kind: "autoPricer", autoPricer: ow["autoPricer"] };
  }
  return undefined;
}

function bookIdFromWire(o: WireObject, key: string): BookId | undefined {
  const v = o[key];
  if (!v || typeof v !== "object") return undefined;
  const b = v as WireObject;
  const id: BookId = { book: str(b, "book") };
  const owner = ownerFromWire(b);
  if (owner !== undefined) id.owner = owner;
  return id;
}

/** Decode an optional `AttributionRecord` (`null`/absent ⇒ undefined). */
export function attributionFromWire(
  o: WireObject,
): AttributionRecord | undefined {
  const v = o["attribution"];
  if (!v || typeof v !== "object") return undefined;
  const a = v as WireObject;
  const rec: AttributionRecord = {};
  const quotedBy = bookIdFromWire(a, "quotedBy");
  if (quotedBy !== undefined) rec.quotedBy = quotedBy;
  const heldBy = bookIdFromWire(a, "heldBy");
  if (heldBy !== undefined) rec.heldBy = heldBy;
  if (typeof a["won"] === "boolean") rec.won = a["won"];
  if (typeof a["lpCount"] === "number") rec.lpCount = a["lpCount"];
  // Nothing decoded at all ⇒ treat as absent (honest empty-state, not `{}`).
  return Object.keys(rec).length > 0 ? rec : undefined;
}

function ownerToWire(o: Owner): WireObject {
  return o.kind === "trader"
    ? { trader: o.trader }
    : { autoPricer: o.autoPricer };
}

function bookIdToWire(b: BookId): WireObject {
  const w: WireObject = { book: b.book };
  if (b.owner) w["owner"] = ownerToWire(b.owner);
  return w;
}

/** Encode an `AttributionRecord` (the requesting seat a client may stamp on a request). */
export function attributionToWire(a: AttributionRecord): WireObject {
  const w: WireObject = {};
  if (a.quotedBy) w["quotedBy"] = bookIdToWire(a.quotedBy);
  if (a.heldBy) w["heldBy"] = bookIdToWire(a.heldBy);
  if (a.won !== undefined) w["won"] = a.won;
  if (a.lpCount !== undefined) w["lpCount"] = a.lpCount;
  return w;
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
  const attribution = attributionFromWire(o);
  if (attribution !== undefined) q.attribution = attribution;
  // The Monte-Carlo standard error of an MC-priced product (`PriceResponse
  // .price_std_error`, field 7). Presence-tracked: absent/null for every
  // closed-form product, set only for a clamped cliquet.
  const stdErr = optNum(o, "price_std_error");
  if (stdErr !== undefined) q.priceStdError = stdErr;
  return q;
}

export function executionFromWire(
  o: WireObject,
): Omit<Execution, "instrument"> {
  // The wire Execution carries no instrument echo; the caller pairs it with the
  // instrument it accepted from its own quote cache (mirrors the SDK).
  const ex: Omit<Execution, "instrument"> = {
    executionId: numToBigInt(o, "execution_id"),
    quoteId: numToBigInt(o, "quote_id"),
    side: e.side.fromWire(enumNum(o, "side")),
    tradedPremium: num(o, "traded_premium"),
    epochNanos: numToBigInt(o, "epoch_nanos"),
  };
  const attribution = attributionFromWire(o);
  if (attribution !== undefined) ex.attribution = attribution;
  return ex;
}

/**
 * Decode one liquidity provider's panel line, the mirror of the server codec's
 * `dealer_quote_to_json` (snake_case keys; presence-tracked `greeks`/
 * `price_std_error` are `null` on a non-native row — decoded to `undefined`, an
 * honest absence, never zeros).
 */
function dealerQuoteFromWire(o: WireObject): DealerQuote {
  const d: DealerQuote = {
    lpId: str(o, "lp_id"),
    price: twoWayFromWire(child(o, "price")),
    resolvedStrike: num(o, "resolved_strike"),
    validUntilNanos: numToBigInt(o, "valid_until_nanos"),
  };
  const g = o["greeks"];
  if (g && typeof g === "object") d.greeks = greeksFromWire(g as WireObject);
  const attribution = attributionFromWire(o);
  if (attribution !== undefined) d.attribution = attribution;
  const stdErr = optNum(o, "price_std_error");
  if (stdErr !== undefined) d.priceStdError = stdErr;
  return d;
}

/**
 * Decode the multi-dealer (RFQ-to-many) panel frame, the mirror of the server
 * codec's `multi_dealer_quote_to_json`. The `dealers` array is kept in FRAME
 * ORDER (the server's deterministic audit order) — a consumer renders it as-is;
 * the ranking rides in `best_bid_lp_id`/`best_offer_lp_id`.
 */
export function multiDealerQuoteFromWire(o: WireObject): MultiDealerQuote {
  const m: MultiDealerQuote = {
    quoteId: numToBigInt(o, "quote_id"),
    idempotencyKey: str(o, "idempotency_key"),
    dealers: array(o, "dealers").map(dealerQuoteFromWire),
    bestBidLpId: str(o, "best_bid_lp_id"),
    bestOfferLpId: str(o, "best_offer_lp_id"),
    conventions: conventionsFromWire(child(o, "conventions")),
    epochNanos: numToBigInt(o, "epoch_nanos"),
  };
  const corr = optBigInt(o, "correlation_id");
  if (corr !== undefined) m.correlationId = corr;
  const surf = optBigInt(o, "surface_version");
  if (surf !== undefined) m.surfaceVersion = surf;
  return m;
}

/**
 * Encode the `accept_quote` body, the mirror of the server codec's
 * `quote_accept_from_json`. The multi-dealer line selector `lp_id` is emitted
 * ONLY when a non-empty `lpId` names a panel row — an absent key selects the
 * single-dealer quote, keeping that path byte-identical to the pre-panel frame
 * (the server's `string_or_empty` reads an absent key as `""`).
 */
export function quoteAcceptToWire(
  quoteId: bigint,
  side: Side,
  idempotencyKey: string,
  lpId?: string,
): WireObject {
  const w: WireObject = {
    // The quote_id is the server's exact 64-bit minted identity (splitmix64 over
    // the full u64 range, so it routinely exceeds Number.MAX_SAFE_INTEGER). Pass
    // it as a `bigint` so `serializeFrame` writes the full-precision integer
    // literal back verbatim — a lossy `Number(quoteId)` rounds the id and the
    // server refuses the accept as `unknown quote_id`.
    quote_id: quoteId,
    idempotency_key: idempotencyKey,
    side: e.side.toWire(side),
    // Caller-authz (item B §2): the server binds an accept to the recording
    // requester, so the accept must carry the SAME caller the request did — the
    // grant-all default principal (and the bearer `session_token` the connection
    // auto-injects into every envelope, the same one the stream `authenticate`
    // frame uses). A mismatch is refused `permission_denied`.
    principal: principalOrGrantAllToWire(undefined),
  };
  if (lpId !== undefined && lpId.length > 0) w["lp_id"] = lpId;
  return w;
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
  const attribution = attributionFromWire(o);
  if (attribution !== undefined) s.attribution = attribution;
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
 * Decode a server [`Heartbeat`] frame. The `subscription`/`sequence`/`epoch_nanos`
 * are the liveness fields; `conflation_drops` and the `server_price_p*_nanos`
 * percentiles are the additive observability the server surfaces off the hot path
 * (read straight from the ring's skip count and the drain-side HdrHistogram —
 * the GUI carries them as plain `bigint` ns/counts). `surface_version`/
 * `correlation_id` are the optional provenance echo (absent/0 ⇒ live/none).
 */
export function heartbeatFromWire(o: WireObject): Heartbeat {
  const h: Heartbeat = {
    subscriptionId: subscriptionIdFromWire(o),
    sequence: numToBigInt(o, "sequence"),
    conflationDrops: numToBigInt(o, "conflation_drops"),
    serverPriceP50Nanos: numToBigInt(o, "server_price_p50_nanos"),
    serverPriceP99Nanos: numToBigInt(o, "server_price_p99_nanos"),
    serverPriceP999Nanos: numToBigInt(o, "server_price_p999_nanos"),
    epochNanos: numToBigInt(o, "epoch_nanos"),
  };
  const surf = optBigInt(o, "surface_version");
  if (surf !== undefined) h.surfaceVersion = surf;
  const corr = optBigInt(o, "correlation_id");
  if (corr !== undefined) h.correlationId = corr;
  return h;
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
  const attribution = attributionFromWire(o);
  if (attribution !== undefined) ex.attribution = attribution;
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
// market-series feed — encode subscribe/unsubscribe, decode snapshot/point
// ---------------------------------------------------------------------------
//
// The client→server frames mirror the server codec's `market_series_subscribe_from_json`
// / `market_series_unsubscribe_from_json` (snake_case fields, numeric `observable`
// tag, presence-tracked `tenor`/`delta`). The server→client frames are decoded from
// the server codec's `market_series_snapshot_to_json` / `market_series_point_to_json`.

/** Encode a `MarketSeriesSubscribe` control frame body (the `type`/`correlation` are added by the caller). */
export function marketSeriesSubscribeToWire(args: {
  subscriptionId: bigint;
  pair: CcyPair;
  observable: MarketObservable;
  tenor?: Tenor;
  delta?: number;
  throttleNanos: bigint;
  historyLimit: number;
}): WireObject {
  const w: WireObject = {
    subscription: { value: Number(args.subscriptionId) },
    pair: ccyPairToWire(args.pair),
    observable: e.marketObservable.toWire(args.observable),
    throttle_nanos: Number(args.throttleNanos),
    history_limit: args.historyLimit,
  };
  if (args.tenor) w["tenor"] = tenorToWire(args.tenor);
  if (args.delta !== undefined) w["delta"] = args.delta;
  return w;
}

/** Encode a `MarketSeriesUnsubscribe` control frame body. */
export function marketSeriesUnsubscribeToWire(
  subscriptionId: bigint,
): WireObject {
  return { subscription: { value: Number(subscriptionId) } };
}

export function marketSeriesPointFromWire(o: WireObject): MarketSeriesPoint {
  return {
    subscriptionId: subscriptionIdFromWire(o),
    sequence: numToBigInt(o, "sequence"),
    value: num(o, "value"),
    epochNanos: numToBigInt(o, "epoch_nanos"),
  };
}

export function marketSeriesSnapshotFromWire(
  o: WireObject,
): MarketSeriesSnapshot {
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
  return {
    delta: num(o, "delta"),
    tenorYears: num(o, "tenor_years"),
    vol: num(o, "vol"),
  };
}

export function arbReportFromWire(o: WireObject): ArbReport {
  return {
    butterflyArbitrageFree: Boolean(o["butterfly_arbitrage_free"]),
    calendarArbitrageFree: Boolean(o["calendar_arbitrage_free"]),
    worstDensity: num(o, "worst_density"),
    note: str(o, "note"),
    // The TYPED, authoritative calibration-family provenance the server stamps
    // (`arb_report_to_json` → `smile_model`, the numeric SmileModel tag). The GUI
    // reads this directly — never the `model=` token in `note`. Absent ⇒ the
    // codec's decode-zero default (MARKET_HEDGE), matching proto3 enum semantics.
    model: e.smileModel.fromWire(enumNum(o, "smile_model")),
  };
}

export function smileFromWire(o: WireObject): Smile {
  return {
    pair: ccyPairFromWire(child(o, "pair")),
    tenorYears: num(o, "tenor_years"),
    brokerQuotes: brokerQuoteSetFromWire(child(o, "broker_quotes")),
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

/**
 * The proto enum tag for a `SmileModel`, for the optional `smile_model` field on
 * `MarkSurfaceRequest`/`ScenarioRequest`. The server's `opt_smile_model` accepts
 * the numeric tag (or the `SMILE_MODEL_*` name); we emit the numeric tag.
 */
export function smileModelToWire(m: SmileModel): number {
  return e.smileModel.toWire(m);
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
  return {
    factor: e.shockFactor.toWire(a.factor),
    relative: a.relative,
    steps: a.steps,
  };
}

/**
 * Encode the book-shaped risk decomposition request that rides on a `Scenario`
 * call. The mirror's `RiskBucketRequest` (crates/celnet-server/src/ws/codec.rs
 * `risk_bucket_request_from_json`) reads each vega pillar FLAT as
 * `{tenor_years, delta}` and each cross-gamma pair FLAT as `{factor_a, factor_b}`
 * (proto enum tags), plus a bare `roll_horizons_years` array — supplying this is
 * what makes the server populate `bucketed_risk` at all (it is `null` otherwise).
 */
export function riskBucketRequestToWire(r: RiskBucketRequest): WireObject {
  return {
    vega_pillars: r.vegaPillars.map((p) => ({
      tenor_years: p.tenorYears,
      delta: p.delta,
    })),
    cross_gamma_pairs: r.crossGammaPairs.map((c) => ({
      factor_a: e.shockFactor.toWire(c.factorA),
      factor_b: e.shockFactor.toWire(c.factorB),
    })),
    roll_horizons_years: r.rollHorizonsYears,
  };
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
  return Array.isArray(v)
    ? v.filter((x): x is number => typeof x === "number")
    : [];
}

function vegaBucketFromWire(o: WireObject): VegaBucket {
  return {
    tenorYears: num(o, "tenor_years"),
    delta: num(o, "delta"),
    vega: num(o, "vega"),
  };
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
  // `bucketed_risk` is an optional proto message: the server emits it only when the
  // request carried a `RiskBucketRequest`, and serializes the absent case as JSON
  // `null` (or omits it). Decode that to `null` so the workspace shows an honest
  // "not requested / no position" empty-state rather than a row of zeros.
  const br = o["bucketed_risk"];
  const bucketedRisk =
    br && typeof br === "object"
      ? bucketedRiskFromWire(br as WireObject)
      : null;
  return {
    points: array(o, "points").map(scenarioPointFromWire),
    bucketedRisk,
  };
}

// ---------------------------------------------------------------------------
// hierarchical risk (RiskService) — encode requests, decode responses
// ---------------------------------------------------------------------------
//
// The same single contract, second encoding (rule 9): every object maps the proto
// message field-for-field by the proto snake_case field name, every enum rides by
// its canonical proto enum number, and `optional` (presence-tracked) fields are
// `null`/absent when `None`. These mirror the server codec in
// `crates/celnet-server/src/ws/codec.rs` (the four `*_request_from_json` /
// `*_response_to_json` fns). The request `type`/`correlation_id` are added by the
// transport; these encoders/decoders cover only the message body.

/** An optional presence-tracked number (`null`/absent ⇒ undefined). */
function optNum(o: WireObject, key: string): number | undefined {
  const v = o[key];
  return typeof v === "number" ? v : undefined;
}

// --- shared value codecs (encode GUI → wire) -------------------------------

export function riskScopeToWire(s: RiskScope): WireObject {
  return { dimension: e.riskDimension.toWire(s.dimension), value: s.value };
}

function entitlementRuleToWire(r: EntitlementRule): WireObject {
  return { scopes: r.scopes.map(riskScopeToWire) };
}

export function principalToWire(p: EntitlementPrincipal): WireObject {
  return {
    grant_all: p.grantAll,
    grants: p.grants.map(entitlementRuleToWire),
    denies: p.denies.map(entitlementRuleToWire),
  };
}

/**
 * The wire principal a risk request carries: the asserted principal, or — when the
 * caller passed none — an **explicit** grant-all (the audited show-all-now default
 * every client shares). The headline risk workflow therefore clears the server's
 * production deny-by-default boundary (`AccessMode::Enforce`), which denies a
 * *genuinely* absent principal; the GUI never relies on the server granting an
 * absent request. A deployment's authenticating gateway injects/validates the real
 * principal in production.
 */
export function principalOrGrantAllToWire(
  p: EntitlementPrincipal | undefined,
): WireObject {
  return principalToWire(p ?? { grantAll: true, grants: [], denies: [] });
}

function numeraireRateToWire(r: NumeraireRate): WireObject {
  return { ccy: r.ccy, rate: r.rate };
}

export function numeraireToWire(n: ReportingNumeraire): WireObject {
  return { numeraire: n.numeraire, rates: n.rates.map(numeraireRateToWire) };
}

export function vegaPillarToWire(p: RiskVegaPillar): WireObject {
  return { tenor_days: p.tenorDays, delta_bp: p.deltaBp };
}

// --- shared value codecs (decode wire → GUI) -------------------------------

function riskScopeFromWire(o: WireObject): RiskScope {
  return {
    dimension: e.riskDimension.fromWire(enumNum(o, "dimension")),
    value: numToBigInt(o, "value"),
  };
}

function orgKeyFromWire(o: WireObject): OrgKey {
  return {
    trader: num(o, "trader"),
    book: num(o, "book"),
    desk: num(o, "desk"),
    ccyPair: ccyPairFromWire(child(o, "ccy_pair")),
    location: num(o, "location"),
    entity: num(o, "entity"),
  };
}

function vanillaInputsFromWire(o: WireObject): VanillaInputs {
  return {
    spot: num(o, "spot"),
    strike: num(o, "strike"),
    vol: num(o, "vol"),
    t: num(o, "t"),
    rDom: num(o, "r_dom"),
    rFor: num(o, "r_for"),
  };
}

function vegaPillarFromWire(o: WireObject): RiskVegaPillar {
  return { tenorDays: num(o, "tenor_days"), deltaBp: num(o, "delta_bp") };
}

function ccyExposureLegFromWire(o: WireObject): CcyExposureLeg {
  return { ccy: str(o, "ccy"), amount: num(o, "amount") };
}

function vegaLadderBucketFromWire(o: WireObject): VegaLadderBucket {
  return {
    pillar: vegaPillarFromWire(child(o, "pillar")),
    vega: num(o, "vega"),
  };
}

function additiveRiskFromWire(o: WireObject): AdditiveRisk {
  return {
    deltaNumeraire: num(o, "delta_numeraire"),
    deltaVector: array(o, "delta_vector").map(ccyExposureLegFromWire),
    gamma: num(o, "gamma"),
    vegaNumeraire: num(o, "vega_numeraire"),
    theta: num(o, "theta"),
    vanna: num(o, "vanna"),
    volga: num(o, "volga"),
    charm: num(o, "charm"),
    speed: num(o, "speed"),
    zomma: num(o, "zomma"),
    color: num(o, "color"),
    premiumNumeraire: num(o, "premium_numeraire"),
    vegaLadder: array(o, "vega_ladder").map(vegaLadderBucketFromWire),
  };
}

function nonAdditiveRiskFromWire(o: WireObject): NonAdditiveRisk {
  // Each field is presence-tracked: absent/`null` ⇒ undefined (never a spurious 0).
  const r: NonAdditiveRisk = {};
  const v = optNum(o, "var");
  if (v !== undefined) r.var = v;
  const es = optNum(o, "es");
  if (es !== undefined) r.es = es;
  const alpha = optNum(o, "var_alpha");
  if (alpha !== undefined) r.varAlpha = alpha;
  const curv = optNum(o, "curvature_spot");
  if (curv !== undefined) r.curvatureSpot = curv;
  return r;
}

function riskNodeFromWire(o: WireObject): RiskNode {
  return {
    dimension: e.riskDimension.fromWire(enumNum(o, "dimension")),
    group: numToBigInt(o, "group"),
    additive: additiveRiskFromWire(child(o, "additive")),
    nonadditive: nonAdditiveRiskFromWire(child(o, "nonadditive")),
    positionCount: num(o, "position_count"),
  };
}

export function riskPositionFromWire(o: WireObject): RiskPosition {
  const p: RiskPosition = {
    positionId: numToBigInt(o, "position_id"),
    org: orgKeyFromWire(child(o, "org")),
    optionType: e.optionType.fromWire(enumNum(o, "option_type")),
    notionalBase: num(o, "notional_base"),
    inputs: vanillaInputsFromWire(child(o, "inputs")),
    quotedDelta: e.deltaConvention.fromWire(enumNum(o, "quoted_delta")),
    premiumStyle: e.premiumStyle.fromWire(enumNum(o, "premium_style")),
    surfaceVersion: numToBigInt(o, "surface_version"),
  };
  const attribution = attributionFromWire(o);
  if (attribution !== undefined) p.attribution = attribution;
  return p;
}

function limitUtilizationFromWire(o: WireObject): LimitUtilization {
  return {
    metric: e.limitMetricKind.fromWire(enumNum(o, "metric")),
    vegaPillar: vegaPillarFromWire(child(o, "vega_pillar")),
    tenorDays: num(o, "tenor_days"),
    cap: num(o, "cap"),
    exposure: num(o, "exposure"),
    ratio: num(o, "ratio"),
    status: e.ragStatus.fromWire(enumNum(o, "status")),
    enforcement: e.enforcement.fromWire(enumNum(o, "enforcement")),
    headroom: num(o, "headroom"),
  };
}

// --- request encoders (GUI → wire body) ------------------------------------

export function listPositionsRequestToWire(
  r: ListPositionsRequest,
): WireObject {
  const w: WireObject = {};
  if (r.scope) w["scope"] = riskScopeToWire(r.scope);
  w["principal"] = principalOrGrantAllToWire(r.principal);
  return w;
}

export function aggregateRiskRequestToWire(
  r: AggregateRiskRequest,
): WireObject {
  const w: WireObject = {
    dimension: e.riskDimension.toWire(r.dimension),
    numeraire: numeraireToWire(r.numeraire),
    vega_pillars: r.vegaPillars.map(vegaPillarToWire),
    var_spot_shocks: r.varSpotShocks,
    var_alpha: r.varAlpha,
    curvature_risk_weight: r.curvatureRiskWeight,
  };
  w["principal"] = principalOrGrantAllToWire(r.principal);
  if (r.scope) w["scope"] = riskScopeToWire(r.scope);
  return w;
}

export function drillRiskRequestToWire(r: DrillRiskRequest): WireObject {
  const w: WireObject = {
    node: riskScopeToWire(r.node),
    child_dimension: e.riskDimension.toWire(r.childDimension),
    numeraire: numeraireToWire(r.numeraire),
    vega_pillars: r.vegaPillars.map(vegaPillarToWire),
    include_children: r.includeChildren,
    include_positions: r.includePositions,
  };
  w["principal"] = principalOrGrantAllToWire(r.principal);
  return w;
}

export function limitStatusRequestToWire(r: LimitStatusRequest): WireObject {
  const w: WireObject = {
    scope: riskScopeToWire(r.scope),
    numeraire: numeraireToWire(r.numeraire),
    vega_pillars: r.vegaPillars.map(vegaPillarToWire),
    var_spot_shocks: r.varSpotShocks,
    var_alpha: r.varAlpha,
  };
  w["principal"] = principalOrGrantAllToWire(r.principal);
  return w;
}

// --- response decoders (wire → GUI) ----------------------------------------

export function listPositionsResponseFromWire(
  o: WireObject,
): ListPositionsResponse {
  const res: ListPositionsResponse = {
    positions: array(o, "positions").map(riskPositionFromWire),
  };
  const corr = optBigInt(o, "correlation_id");
  if (corr !== undefined) res.correlationId = corr;
  return res;
}

export function aggregateRiskResponseFromWire(
  o: WireObject,
): AggregateRiskResponse {
  const res: AggregateRiskResponse = {
    dimension: e.riskDimension.fromWire(enumNum(o, "dimension")),
    numeraire: str(o, "numeraire"),
    nodes: array(o, "nodes").map(riskNodeFromWire),
  };
  const corr = optBigInt(o, "correlation_id");
  if (corr !== undefined) res.correlationId = corr;
  return res;
}

export function drillRiskResponseFromWire(o: WireObject): DrillRiskResponse {
  const res: DrillRiskResponse = {
    node: riskScopeFromWire(child(o, "node")),
    children: array(o, "children").map(riskNodeFromWire),
    positions: array(o, "positions").map(riskPositionFromWire),
  };
  const corr = optBigInt(o, "correlation_id");
  if (corr !== undefined) res.correlationId = corr;
  return res;
}

export function limitStatusResponseFromWire(
  o: WireObject,
): LimitStatusResponse {
  const res: LimitStatusResponse = {
    scope: riskScopeFromWire(child(o, "scope")),
    limits: array(o, "limits").map(limitUtilizationFromWire),
    worst: e.ragStatus.fromWire(enumNum(o, "worst")),
    hardBreach: Boolean(o["hard_breach"]),
  };
  const corr = optBigInt(o, "correlation_id");
  if (corr !== undefined) res.correlationId = corr;
  return res;
}

/** Re-export the `StrategyKind` type guard surface for callers that need it. */
export type { StrategyKind };

// ---------------------------------------------------------------------------
// fix-admin: manage the inbound FIX acceptor connections
// ---------------------------------------------------------------------------

/** The wire enum tags for the FIX acceptor dialects (`FixAcceptorKind`). */
const FIX_KIND_OPTIONS = 0;
const FIX_KIND_FIXED_INCOME_QUOTE = 1;
const FIX_KIND_FIXED_INCOME_STREAM = 2;

/** Domain kind → wire enum tag. */
export function fixConnectionKindToWire(kind: FixConnectionKind): number {
  switch (kind) {
    case "FIXED_INCOME_QUOTE":
      return FIX_KIND_FIXED_INCOME_QUOTE;
    case "FIXED_INCOME_STREAM":
      return FIX_KIND_FIXED_INCOME_STREAM;
    case "OPTIONS":
      return FIX_KIND_OPTIONS;
  }
}

/** Wire enum tag → domain kind (an unknown tag falls back to `OPTIONS`). */
export function fixConnectionKindFromWire(tag: number): FixConnectionKind {
  switch (tag) {
    case FIX_KIND_FIXED_INCOME_QUOTE:
      return "FIXED_INCOME_QUOTE";
    case FIX_KIND_FIXED_INCOME_STREAM:
      return "FIXED_INCOME_STREAM";
    default:
      return "OPTIONS";
  }
}

/** A managed connection descriptor from its wire form. */
export function fixConnectionFromWire(o: WireObject): FixConnection {
  return {
    id: str(o, "id"),
    name: str(o, "name"),
    kind: fixConnectionKindFromWire(enumNum(o, "kind")),
    bindAddr: str(o, "bind_addr"),
    senderCompId: str(o, "sender_comp_id"),
    targetCompId: str(o, "target_comp_id"),
    enabled: o["enabled"] === true,
    running: o["running"] === true,
    boundAddr: str(o, "bound_addr"),
    desk: str(o, "desk"),
  };
}

/** The editable connection fields → the wire `spec` object. */
function fixSpecToWire(spec: FixConnectionSpec): WireObject {
  return {
    id: spec.id ?? "",
    name: spec.name,
    kind: fixConnectionKindToWire(spec.kind),
    bind_addr: spec.bindAddr,
    sender_comp_id: spec.senderCompId,
    target_comp_id: spec.targetCompId,
    enabled: spec.enabled,
    desk: spec.desk ?? "",
  };
}

/**
 * The GUI asserts an explicit grant-all principal on every admin call (the same
 * stance the risk requests take), so an enforcing edge authorizes it rather than
 * denying by default; a production gateway substitutes the real principal.
 */
function adminPrincipal(): WireObject {
  return principalOrGrantAllToWire(undefined);
}

export function listFixConnectionsRequestToWire(): WireObject {
  return { principal: adminPrincipal() };
}

export function listFixConnectionsResponseFromWire(
  o: WireObject,
): FixConnection[] {
  const arr = o["connections"];
  return Array.isArray(arr)
    ? (arr as WireObject[]).map(fixConnectionFromWire)
    : [];
}

/** A single-connection response (`{ connection: {...} }`) from create/update/enable. */
export function fixConnectionResponseFromWire(o: WireObject): FixConnection {
  const c = o["connection"];
  return fixConnectionFromWire(
    c && typeof c === "object" ? (c as WireObject) : {},
  );
}

export function createFixConnectionRequestToWire(
  spec: FixConnectionSpec,
): WireObject {
  return { spec: fixSpecToWire(spec), principal: adminPrincipal() };
}

export function updateFixConnectionRequestToWire(
  id: string,
  spec: FixConnectionSpec,
): WireObject {
  return { id, spec: fixSpecToWire(spec), principal: adminPrincipal() };
}

export function deleteFixConnectionRequestToWire(id: string): WireObject {
  return { id, principal: adminPrincipal() };
}

export function setFixConnectionEnabledRequestToWire(
  id: string,
  enabled: boolean,
): WireObject {
  return { id, enabled, principal: adminPrincipal() };
}

// --- fix-admin: captured session traffic (monitor screen) -------------------

/** The wire enum tags for FixMsgDirection. */
const FIX_DIR_OUTBOUND = 1;

/** Wire enum tag → domain direction. */
export function fixMsgDirectionFromWire(tag: number): FixMsgDirection {
  return tag === FIX_DIR_OUTBOUND ? "OUTBOUND" : "INBOUND";
}

/** A captured frame from its wire form. `seq`/`epoch_nanos` parse as bigint. */
export function fixMessageFromWire(o: WireObject): FixMessage {
  return {
    seq: numToBigInt(o, "seq"),
    connectionId: str(o, "connection_id"),
    direction: fixMsgDirectionFromWire(enumNum(o, "direction")),
    msgType: str(o, "msg_type"),
    summary: str(o, "summary"),
    epochNanos: numToBigInt(o, "epoch_nanos"),
    raw: str(o, "raw"),
  };
}

export function listFixMessagesRequestToWire(
  connectionId: string | undefined,
  afterSeq: bigint,
  limit: number,
): WireObject {
  const body: WireObject = {
    after_seq: afterSeq,
    limit,
    principal: adminPrincipal(),
  };
  if (connectionId && connectionId.length > 0)
    body.connection_id = connectionId;
  return body;
}

export function listFixMessagesResponseFromWire(o: WireObject): FixMessagePage {
  const arr = o["messages"];
  return {
    messages: Array.isArray(arr)
      ? (arr as WireObject[]).map(fixMessageFromWire)
      : [],
    latestSeq: numToBigInt(o, "latest_seq"),
  };
}

// --- auth: server-enforced sessions + user/desk admin -----------------------
//
// The bearer `session_token` is injected by the connection into EVERY request
// envelope when set (exactly like the correlation id), so the admin request
// encoders below never carry it themselves — they encode only the call's own
// fields. `login` is the one call made while anonymous (no token to inject).

/** Wire enum tags for `UserRole` (proto3 zero = least-privileged TRADER). */
const USER_ROLE_TRADER = 0;
const USER_ROLE_ADMIN = 1;

/** Domain role → wire enum tag. */
export function userRoleToWire(role: UserRole): number {
  return role === "ADMIN" ? USER_ROLE_ADMIN : USER_ROLE_TRADER;
}

/** Wire enum tag → domain role (any non-admin tag is TRADER — never accidental admin). */
export function userRoleFromWire(tag: number): UserRole {
  return tag === USER_ROLE_ADMIN ? "ADMIN" : "TRADER";
}

/** A user descriptor from its wire form (`desk_id` absent/empty ⇒ unassigned). */
export function userDescFromWire(o: WireObject): UserDesc {
  const deskId = o["desk_id"];
  const user: UserDesc = {
    id: str(o, "id"),
    email: str(o, "email"),
    displayName: str(o, "display_name"),
    role: userRoleFromWire(enumNum(o, "role")),
    disabled: o["disabled"] === true,
  };
  if (typeof deskId === "string" && deskId.length > 0) user.deskId = deskId;
  return user;
}

/** A desk descriptor from its wire form. */
export function deskDescFromWire(o: WireObject): DeskDesc {
  return { id: str(o, "id"), name: str(o, "name") };
}

// login / logout -------------------------------------------------------------

export function loginRequestToWire(
  email: string,
  password: string,
): WireObject {
  return { email, password };
}

export function loginResultFromWire(o: WireObject): LoginResult {
  const user = o["user"];
  if (!user || typeof user !== "object") {
    throw new Error("login response is missing the authenticated user");
  }
  return {
    token: str(o, "session_token"),
    user: userDescFromWire(user as WireObject),
    expiresNanos: numToBigInt(o, "expires_nanos"),
    // The caller's OWN fully-resolved effective set (`role bundle ∪ grants ∖
    // denies`), enumerated server-side over every action × asset — the source
    // for the client's affordance gating. Absent ⇒ empty (deny-everything).
    capabilities: capabilityListFromWire(o, "capabilities"),
  };
}

/** The logout body is empty — the connection injects the bearer token to invalidate. */
export function logoutRequestToWire(): WireObject {
  return {};
}

// user CRUD ------------------------------------------------------------------

export function listUsersRequestToWire(): WireObject {
  return {};
}

export function listUsersResponseFromWire(o: WireObject): UserDesc[] {
  const arr = o["users"];
  return Array.isArray(arr) ? (arr as WireObject[]).map(userDescFromWire) : [];
}

export function createUserRequestToWire(input: CreateUserInput): WireObject {
  const body: WireObject = {
    email: input.email,
    display_name: input.displayName,
    role: userRoleToWire(input.role),
    password: input.password,
  };
  if (input.deskId && input.deskId.length > 0) body.desk_id = input.deskId;
  return body;
}

export function updateUserRequestToWire(
  id: string,
  input: UpdateUserInput,
): WireObject {
  const body: WireObject = {
    id,
    display_name: input.displayName,
    role: userRoleToWire(input.role),
    disabled: input.disabled,
  };
  if (input.deskId && input.deskId.length > 0) body.desk_id = input.deskId;
  return body;
}

/** A single-user response (`{ user: {...} }`) from create/update. */
export function userResponseFromWire(o: WireObject): UserDesc {
  const u = o["user"];
  return userDescFromWire(u && typeof u === "object" ? (u as WireObject) : {});
}

export function deleteUserRequestToWire(id: string): WireObject {
  return { id };
}

export function resetPasswordRequestToWire(
  id: string,
  newPassword: string,
): WireObject {
  return { id, new_password: newPassword };
}

// per-user capability overlay ------------------------------------------------
//
// `get_user_capabilities` reads the overlay + resolved effective set; the codec
// auto-injects the bearer `session_token`. `set_user_capabilities` replaces the
// overlay wholesale. The action/asset labels are the canonical snake_case the
// server round-trips through `Action`/`AssetClass::from_label`.

function capabilityFromWire(o: WireObject): Capability {
  return {
    action: str(o, "action") as CapabilityAction,
    asset: str(o, "asset") as CapabilityAsset,
  };
}

function capabilityToWire(cap: Capability): WireObject {
  return { action: cap.action, asset: cap.asset };
}

function capabilityListFromWire(o: WireObject, key: string): Capability[] {
  const arr = o[key];
  return Array.isArray(arr)
    ? (arr as WireObject[]).map(capabilityFromWire)
    : [];
}

export function getUserCapabilitiesRequestToWire(id: string): WireObject {
  return { id };
}

export function setUserCapabilitiesRequestToWire(
  id: string,
  grants: readonly Capability[],
  denies: readonly Capability[],
): WireObject {
  return {
    id,
    grants: grants.map(capabilityToWire),
    denies: denies.map(capabilityToWire),
  };
}

/** Decode a `user_capabilities` / `user_capabilities_set` frame. */
export function userCapabilitiesFromWire(o: WireObject): UserCapabilities {
  return {
    grants: capabilityListFromWire(o, "grants"),
    denies: capabilityListFromWire(o, "denies"),
    effective: capabilityListFromWire(o, "effective"),
  };
}

// per-role capability bundle -------------------------------------------------
//
// `get_role_capabilities` reads a role's base bundle; `set_role_capabilities`
// replaces a non-admin role's bundle wholesale. The codec auto-injects the bearer
// `session_token`. The role rides as its proto enum tag (TRADER = 0, ADMIN = 1).

export function getRoleCapabilitiesRequestToWire(role: UserRole): WireObject {
  return { role: userRoleToWire(role) };
}

export function setRoleCapabilitiesRequestToWire(
  role: UserRole,
  capabilities: readonly Capability[],
): WireObject {
  return {
    role: userRoleToWire(role),
    capabilities: capabilities.map(capabilityToWire),
  };
}

/** Decode a `role_capabilities` / `role_capabilities_set` frame. */
export function roleCapabilitiesFromWire(o: WireObject): RoleCapabilities {
  return { capabilities: capabilityListFromWire(o, "capabilities") };
}

// desk CRUD ------------------------------------------------------------------

export function listDesksRequestToWire(): WireObject {
  return {};
}

export function listDesksResponseFromWire(o: WireObject): DeskDesc[] {
  const arr = o["desks"];
  return Array.isArray(arr) ? (arr as WireObject[]).map(deskDescFromWire) : [];
}

export function createDeskRequestToWire(name: string): WireObject {
  return { name };
}

/** A single-desk response (`{ desk: {...} }`) from create. */
export function deskResponseFromWire(o: WireObject): DeskDesc {
  const d = o["desk"];
  return deskDescFromWire(d && typeof d === "object" ? (d as WireObject) : {});
}

export function deleteDeskRequestToWire(id: string): WireObject {
  return { id };
}

// --- legal-entity / netting-book registry (entity/book admin) ---------------
//
// The WS mirror of `AuthService.{List,Create,Update,Delete}{Entity,Book}`. The
// wire JSON carries snake_case `entity_key`; these codecs map it to the GUI's
// camelCase `entityKey` (the only snake↔camel rename on this surface). The
// `session_token` + framing `correlation_id` are auto-injected by
// `WsConnection.request`, so the request encoders carry only the business body.
// A create with `key: 0` asks the server to auto-assign the lowest free key.

/** A legal entity from its wire form. */
export function entityDescFromWire(o: WireObject): EntityDesc {
  return { key: num(o, "key"), name: str(o, "name"), code: str(o, "code") };
}

/** A netting book from its wire form (maps `entity_key` → `entityKey`). */
export function bookDescFromWire(o: WireObject): BookDesc {
  return {
    key: num(o, "key"),
    name: str(o, "name"),
    entityKey: num(o, "entity_key"),
  };
}

export function listEntitiesRequestToWire(): WireObject {
  return {};
}

export function entitiesResponseFromWire(o: WireObject): EntityDesc[] {
  const arr = o["entities"];
  return Array.isArray(arr)
    ? (arr as WireObject[]).map(entityDescFromWire)
    : [];
}

export function createEntityRequestToWire(input: EntityInput): WireObject {
  // `key: 0` ⇒ the server auto-assigns the lowest free key.
  return { name: input.name, code: input.code, key: 0 };
}

export function updateEntityRequestToWire(
  key: number,
  input: EntityInput,
): WireObject {
  return { key, name: input.name, code: input.code };
}

/** A single-entity response (`{ entity: {...} }`) from create / update. */
export function entityResponseFromWire(o: WireObject): EntityDesc {
  const e = o["entity"];
  return entityDescFromWire(
    e && typeof e === "object" ? (e as WireObject) : {},
  );
}

export function deleteEntityRequestToWire(key: number): WireObject {
  return { key };
}

export function listBooksRequestToWire(): WireObject {
  return {};
}

export function booksResponseFromWire(o: WireObject): BookDesc[] {
  const arr = o["books"];
  return Array.isArray(arr) ? (arr as WireObject[]).map(bookDescFromWire) : [];
}

export function createBookRequestToWire(input: BookInput): WireObject {
  // `key: 0` ⇒ the server auto-assigns the lowest free key.
  return { name: input.name, entity_key: input.entityKey, key: 0 };
}

export function updateBookRequestToWire(
  key: number,
  input: BookInput,
): WireObject {
  return { key, name: input.name, entity_key: input.entityKey };
}

/** A single-book response (`{ book: {...} }`) from create / update. */
export function bookResponseFromWire(o: WireObject): BookDesc {
  const b = o["book"];
  return bookDescFromWire(b && typeof b === "object" ? (b as WireObject) : {});
}

export function deleteBookRequestToWire(key: number): WireObject {
  return { key };
}

// --- instrument reference-data registry (instrument admin) -----------------
//
// The WS mirror of `AuthService.{List,Get,Create,Update,Delete}Instrument`. A
// definition carries exactly ONE family sub-object keyed by its family token;
// the wire key is snake_case (`stir_future`, `vanilla_irs`) while the GUI holds
// it under a camelCase key (`stirFuture`, `vanillaIrs`). Every scalar field is
// snake_case on the wire (`day_count`, `spot_lag_days`, `external_ids`). Bond
// dates are `{ year, month, day }`; the optional ones are omitted on the wire
// when absent. `session_token` is auto-injected by `WsConnection.request`, so
// these encoders carry only the business body.

function strArray(o: WireObject, key: string): string[] {
  const v = o[key];
  return Array.isArray(v)
    ? v.filter((x): x is string => typeof x === "string")
    : [];
}

function externalIdsToWire(ids: ExternalIdEntry[]): WireObject[] {
  return ids.map((id) => ({ scheme: id.scheme, value: id.value }));
}

function externalIdsFromWire(o: WireObject): ExternalIdEntry[] {
  const arr = o["external_ids"];
  if (!Array.isArray(arr)) return [];
  return (arr as WireObject[]).map((entry) => ({
    scheme: str(entry, "scheme") as ExternalIdScheme,
    value: str(entry, "value"),
  }));
}

function brokenDateToWire(d: BrokenDate): WireObject {
  return { year: d.year, month: d.month, day: d.day };
}

function brokenDateFromWire(o: WireObject): BrokenDate {
  return { year: num(o, "year"), month: num(o, "month"), day: num(o, "day") };
}

/** An optional bond date: present ⇒ decoded, `null`/absent ⇒ undefined. */
function optBrokenDate(o: WireObject, key: string): BrokenDate | undefined {
  const v = o[key];
  if (!v || typeof v !== "object") return undefined;
  return brokenDateFromWire(v as WireObject);
}

function depositToWire(d: DepositDef): WireObject {
  return {
    index: d.index,
    tenor: d.tenor,
    day_count: d.dayCount,
    business_day_convention: d.businessDayConvention,
    calendars: d.calendars,
    spot_lag_days: d.spotLagDays,
  };
}

function depositFromWire(o: WireObject): DepositDef {
  return {
    index: str(o, "index"),
    tenor: str(o, "tenor"),
    dayCount: str(o, "day_count") as RatesDayCount,
    businessDayConvention: str(
      o,
      "business_day_convention",
    ) as BusinessDayConvention,
    calendars: strArray(o, "calendars") as Calendar[],
    spotLagDays: num(o, "spot_lag_days"),
  };
}

function fraToWire(d: FraDef): WireObject {
  return {
    float_index: d.floatIndex,
    start_tenor: d.startTenor,
    end_tenor: d.endTenor,
    accrual_day_count: d.accrualDayCount,
    business_day_convention: d.businessDayConvention,
    calendars: d.calendars,
    spot_lag_days: d.spotLagDays,
  };
}

function fraFromWire(o: WireObject): FraDef {
  return {
    floatIndex: str(o, "float_index"),
    startTenor: str(o, "start_tenor"),
    endTenor: str(o, "end_tenor"),
    accrualDayCount: str(o, "accrual_day_count") as RatesDayCount,
    businessDayConvention: str(
      o,
      "business_day_convention",
    ) as BusinessDayConvention,
    calendars: strArray(o, "calendars") as Calendar[],
    spotLagDays: num(o, "spot_lag_days"),
  };
}

function stirFutureToWire(d: StirFutureDef): WireObject {
  return {
    contract_code: d.contractCode,
    reference_start: d.referenceStart,
    reference_end: d.referenceEnd,
    day_count: d.dayCount,
    calendars: d.calendars,
    convexity_vol: d.convexityVol,
    contract_size: d.contractSize,
  };
}

function stirFutureFromWire(o: WireObject): StirFutureDef {
  return {
    contractCode: str(o, "contract_code"),
    referenceStart: str(o, "reference_start"),
    referenceEnd: str(o, "reference_end"),
    dayCount: str(o, "day_count") as RatesDayCount,
    calendars: strArray(o, "calendars") as Calendar[],
    convexityVol: num(o, "convexity_vol"),
    contractSize: num(o, "contract_size"),
  };
}

function vanillaIrsToWire(d: VanillaIrsDef): WireObject {
  return {
    tenor: d.tenor,
    fixed_frequency: d.fixedFrequency,
    fixed_day_count: d.fixedDayCount,
    float_index: d.floatIndex,
    float_frequency: d.floatFrequency,
    float_day_count: d.floatDayCount,
    business_day_convention: d.businessDayConvention,
    calendars: d.calendars,
    roll_convention: d.rollConvention,
    spot_lag_days: d.spotLagDays,
  };
}

function vanillaIrsFromWire(o: WireObject): VanillaIrsDef {
  return {
    tenor: str(o, "tenor"),
    fixedFrequency: str(o, "fixed_frequency") as Frequency,
    fixedDayCount: str(o, "fixed_day_count") as RatesDayCount,
    floatIndex: str(o, "float_index"),
    floatFrequency: str(o, "float_frequency") as Frequency,
    floatDayCount: str(o, "float_day_count") as RatesDayCount,
    businessDayConvention: str(
      o,
      "business_day_convention",
    ) as BusinessDayConvention,
    calendars: strArray(o, "calendars") as Calendar[],
    rollConvention: str(o, "roll_convention") as RollConvention,
    spotLagDays: num(o, "spot_lag_days"),
  };
}

function oisToWire(d: OisDef): WireObject {
  return {
    tenor: d.tenor,
    index: d.index,
    fixed_frequency: d.fixedFrequency,
    fixed_day_count: d.fixedDayCount,
    float_day_count: d.floatDayCount,
    business_day_convention: d.businessDayConvention,
    calendars: d.calendars,
    spot_lag_days: d.spotLagDays,
  };
}

function oisFromWire(o: WireObject): OisDef {
  return {
    tenor: str(o, "tenor"),
    index: str(o, "index"),
    fixedFrequency: str(o, "fixed_frequency") as Frequency,
    fixedDayCount: str(o, "fixed_day_count") as RatesDayCount,
    floatDayCount: str(o, "float_day_count") as RatesDayCount,
    businessDayConvention: str(
      o,
      "business_day_convention",
    ) as BusinessDayConvention,
    calendars: strArray(o, "calendars") as Calendar[],
    spotLagDays: num(o, "spot_lag_days"),
  };
}

function bondToWire(b: BondDef): WireObject {
  const w: WireObject = {
    issuer: b.issuer,
    coupon_rate: b.couponRate,
    coupon_type: b.couponType,
    coupon_frequency: b.couponFrequency,
    day_count: b.dayCount,
    maturity_date: brokenDateToWire(b.maturityDate),
    redemption: b.redemption,
    calendars: b.calendars,
  };
  if (b.issueDate) w.issue_date = brokenDateToWire(b.issueDate);
  if (b.datedDate) w.dated_date = brokenDateToWire(b.datedDate);
  if (b.firstCouponDate)
    w.first_coupon_date = brokenDateToWire(b.firstCouponDate);
  return w;
}

function bondFromWire(o: WireObject): BondDef {
  const def: BondDef = {
    issuer: str(o, "issuer"),
    couponRate: num(o, "coupon_rate"),
    couponType: str(o, "coupon_type") as CouponType,
    couponFrequency: str(o, "coupon_frequency") as Frequency | "",
    dayCount: str(o, "day_count") as RatesDayCount,
    maturityDate: brokenDateFromWire(child(o, "maturity_date")),
    redemption: num(o, "redemption"),
    calendars: strArray(o, "calendars") as Calendar[],
  };
  const issue = optBrokenDate(o, "issue_date");
  const dated = optBrokenDate(o, "dated_date");
  const firstCoupon = optBrokenDate(o, "first_coupon_date");
  if (issue) def.issueDate = issue;
  if (dated) def.datedDate = dated;
  if (firstCoupon) def.firstCouponDate = firstCoupon;
  return def;
}

/** An instrument definition → its wire form (single family sub-object). */
export function instrumentDefToWire(def: InstrumentDef): WireObject {
  const wire: WireObject = {
    instrument_id: def.instrumentId,
    name: def.name,
    description: def.description,
    currency: def.currency,
    external_ids: externalIdsToWire(def.externalIds),
  };
  switch (def.family) {
    case "deposit":
      wire.deposit = depositToWire(def.deposit);
      break;
    case "fra":
      wire.fra = fraToWire(def.fra);
      break;
    case "stir_future":
      wire.stir_future = stirFutureToWire(def.stirFuture);
      break;
    case "vanilla_irs":
      wire.vanilla_irs = vanillaIrsToWire(def.vanillaIrs);
      break;
    case "ois":
      wire.ois = oisToWire(def.ois);
      break;
    case "bond":
      wire.bond = bondToWire(def.bond);
      break;
  }
  return wire;
}

/** An instrument definition from its wire form (detects the family sub-object). */
export function instrumentDefFromWire(o: WireObject): InstrumentDef {
  const base = {
    instrumentId: str(o, "instrument_id"),
    name: str(o, "name"),
    description: str(o, "description"),
    currency: str(o, "currency"),
    externalIds: externalIdsFromWire(o),
  };
  const has = (key: string): boolean =>
    Boolean(o[key]) && typeof o[key] === "object";
  if (has("deposit")) {
    return {
      ...base,
      family: "deposit",
      deposit: depositFromWire(child(o, "deposit")),
    };
  }
  if (has("fra")) {
    return { ...base, family: "fra", fra: fraFromWire(child(o, "fra")) };
  }
  if (has("stir_future")) {
    return {
      ...base,
      family: "stir_future",
      stirFuture: stirFutureFromWire(child(o, "stir_future")),
    };
  }
  if (has("vanilla_irs")) {
    return {
      ...base,
      family: "vanilla_irs",
      vanillaIrs: vanillaIrsFromWire(child(o, "vanilla_irs")),
    };
  }
  if (has("ois")) {
    return { ...base, family: "ois", ois: oisFromWire(child(o, "ois")) };
  }
  // Exactly one family is always present; bond is the remaining case.
  return { ...base, family: "bond", bond: bondFromWire(child(o, "bond")) };
}

export function listInstrumentsRequestToWire(): WireObject {
  return {};
}

export function instrumentsResponseFromWire(o: WireObject): InstrumentDef[] {
  const arr = o["instruments"];
  return Array.isArray(arr)
    ? (arr as WireObject[]).map(instrumentDefFromWire)
    : [];
}

export function getInstrumentRequestToWire(id: string): WireObject {
  return { instrument_id: id };
}

/** A single-instrument response (`{ instrument: {...} | null }`). */
export function instrumentResponseFromWire(
  o: WireObject,
): InstrumentDef | null {
  const v = o["instrument"];
  if (!v || typeof v !== "object") return null;
  return instrumentDefFromWire(v as WireObject);
}

export function createInstrumentRequestToWire(
  input: InstrumentInput,
): WireObject {
  return { instrument: instrumentDefToWire(input) };
}

export function updateInstrumentRequestToWire(
  input: InstrumentInput,
): WireObject {
  return { instrument: instrumentDefToWire(input) };
}

export function deleteInstrumentRequestToWire(id: string): WireObject {
  return { instrument_id: id };
}

/** A delete response (`{ removed: boolean }`). */
export function deleteInstrumentResponseFromWire(o: WireObject): boolean {
  return o["removed"] === true;
}
