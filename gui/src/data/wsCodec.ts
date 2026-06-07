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
  AggregateRiskRequest,
  AggregateRiskResponse,
  ArbReport,
  AttributionRecord,
  BookId,
  BrokerQuoteSet,
  BucketedRisk,
  CcyExposureLeg,
  CcyPair,
  Conventions,
  CrossGamma,
  DrillRiskRequest,
  DrillRiskResponse,
  EntitlementPrincipal,
  EntitlementRule,
  Executed,
  Execution,
  FixingSchedule,
  Greeks,
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
  NonAdditiveRisk,
  NumeraireRate,
  OrgKey,
  Owner,
  Quote,
  ReportingNumeraire,
  RiskBucketRequest,
  RiskNode,
  RiskPosition,
  RiskScope,
  RiskVegaPillar,
  ScenarioPoint,
  ScenarioResult,
  ShockAxis,
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
  Update,
  VanillaInputs,
  VegaBucket,
  VegaLadderBucket,
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
  const base: WireObject = {
    pair: ccyPairToWire(i.pair),
    tenor: tenorToWire(i.tenor),
    expiry_years: i.expiryYears,
    quantity: { notional: i.quantity.notional, base_ccy: i.quantity.baseCcy },
    side: e.side.toWire(i.side),
  };
  if (i.solve) base["solve"] = solveToWire(i.solve);
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
      base["volatility_swap"] = { strike_vol: i.product.volatilitySwap.strikeVol };
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
  if (typeof ow["trader"] === "string") return { kind: "trader", trader: ow["trader"] };
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
export function attributionFromWire(o: WireObject): AttributionRecord | undefined {
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
  return o.kind === "trader" ? { trader: o.trader } : { autoPricer: o.autoPricer };
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

export function executionFromWire(o: WireObject): Omit<Execution, "instrument"> {
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
export function marketSeriesUnsubscribeToWire(subscriptionId: bigint): WireObject {
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

function arbReportFromWire(o: WireObject): ArbReport {
  return {
    butterflyArbitrageFree: Boolean(o["butterfly_arbitrage_free"]),
    calendarArbitrageFree: Boolean(o["calendar_arbitrage_free"]),
    worstDensity: num(o, "worst_density"),
    note: str(o, "note"),
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
  return { factor: e.shockFactor.toWire(a.factor), relative: a.relative, steps: a.steps };
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
    vega_pillars: r.vegaPillars.map((p) => ({ tenor_years: p.tenorYears, delta: p.delta })),
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
  // `bucketed_risk` is an optional proto message: the server emits it only when the
  // request carried a `RiskBucketRequest`, and serializes the absent case as JSON
  // `null` (or omits it). Decode that to `null` so the workspace shows an honest
  // "not requested / no position" empty-state rather than a row of zeros.
  const br = o["bucketed_risk"];
  const bucketedRisk =
    br && typeof br === "object" ? bucketedRiskFromWire(br as WireObject) : null;
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
  return { pillar: vegaPillarFromWire(child(o, "pillar")), vega: num(o, "vega") };
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

export function listPositionsRequestToWire(r: ListPositionsRequest): WireObject {
  const w: WireObject = {};
  if (r.scope) w["scope"] = riskScopeToWire(r.scope);
  if (r.principal) w["principal"] = principalToWire(r.principal);
  return w;
}

export function aggregateRiskRequestToWire(r: AggregateRiskRequest): WireObject {
  const w: WireObject = {
    dimension: e.riskDimension.toWire(r.dimension),
    numeraire: numeraireToWire(r.numeraire),
    vega_pillars: r.vegaPillars.map(vegaPillarToWire),
    var_spot_shocks: r.varSpotShocks,
    var_alpha: r.varAlpha,
    curvature_risk_weight: r.curvatureRiskWeight,
  };
  if (r.principal) w["principal"] = principalToWire(r.principal);
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
  if (r.principal) w["principal"] = principalToWire(r.principal);
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
  if (r.principal) w["principal"] = principalToWire(r.principal);
  return w;
}

// --- response decoders (wire → GUI) ----------------------------------------

export function listPositionsResponseFromWire(o: WireObject): ListPositionsResponse {
  const res: ListPositionsResponse = {
    positions: array(o, "positions").map(riskPositionFromWire),
  };
  const corr = optBigInt(o, "correlation_id");
  if (corr !== undefined) res.correlationId = corr;
  return res;
}

export function aggregateRiskResponseFromWire(o: WireObject): AggregateRiskResponse {
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

export function limitStatusResponseFromWire(o: WireObject): LimitStatusResponse {
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
