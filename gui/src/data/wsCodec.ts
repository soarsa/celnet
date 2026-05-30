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
  ArbReport,
  BrokerQuoteSet,
  BucketedRisk,
  CcyPair,
  Conventions,
  CrossGamma,
  Executed,
  Execution,
  Greeks,
  Instrument,
  Leg,
  MarkedSurface,
  MarketContext,
  Quote,
  RiskBucketRequest,
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
  TradableToken,
  TwoWayPrice,
  Update,
  VegaBucket,
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

export function instrumentToWire(i: Instrument): WireObject {
  const base: WireObject = {
    pair: ccyPairToWire(i.pair),
    tenor: { unit: e.tenorUnit.toWire(i.tenor.unit), count: i.tenor.count },
    expiry_years: i.expiryYears,
    quantity: { notional: i.quantity.notional, base_ccy: i.quantity.baseCcy },
    side: e.side.toWire(i.side),
  };
  if (i.solve) base["solve"] = solveToWire(i.solve);
  // The product oneof: nest the body under its own key, exactly like the proto.
  if (i.product.kind === "vanilla") {
    base["vanilla"] = {
      option_type: e.optionType.toWire(i.product.vanilla.optionType),
      strike: strikeOrDeltaToWire(i.product.vanilla.strike),
    };
  } else {
    base["strategy"] = {
      kind: e.strategyKind.toWire(i.product.strategy.kind),
      legs: i.product.strategy.legs.map(legToWire),
    };
  }
  return base;
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

/** Re-export the `StrategyKind` type guard surface for callers that need it. */
export type { StrategyKind };
