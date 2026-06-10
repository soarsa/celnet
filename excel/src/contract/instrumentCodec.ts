// ONE CONTRACT — the decode mirror of `wsCodec.ts`'s `instrumentToWire` plus the
// canonical (deterministic) serialization of a wire frame. Together they define
// the opaque instrument TOKEN `CELNET.INSTRUMENT` returns: the token IS the
// canonical compact JSON of the proto `Instrument`'s WS-mirror frame (the same
// snake_case, numeric-enum JSON `crates/celnet-server/src/ws/codec.rs` decodes),
// so a token-priced cell emits byte-for-byte the frame its instrument encodes.
// The token is a VALUE, not an API: the one contract stays `celnet.proto`, and
// this codec merely round-trips it losslessly (decode∘encode is the identity on
// every frame `instrumentToWire` can produce).

import type {
  Accumulator,
  AmericanOption,
  AsianOption,
  BasketLeg,
  BasketOption,
  CcyPair,
  Cliquet,
  Digital,
  DoubleBarrier,
  FixingSchedule,
  FxForward,
  Instrument,
  Leg,
  Lookback,
  Ndf,
  Product,
  Quanto,
  SingleBarrier,
  Solve,
  StrikeOrDelta,
  Tarf,
  Touch,
  Underlying,
  Vanilla,
  WindowBarrier,
} from "./contract";
import type { WireObject } from "./wsCodec";
import { ccyPairFromWire } from "./wsCodec";
import * as e from "./enums";

/** A malformed wire frame / instrument token (decode-side honest rejection). */
export class WireDecodeError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "WireDecodeError";
  }
}

// ---------------------------------------------------------------------------
// canonical serialization — the deterministic token form
// ---------------------------------------------------------------------------

/**
 * Serialize a wire object to canonical compact JSON: recursively key-sorted,
 * no whitespace. JSON objects are unordered key/value maps on the wire (the
 * server's `serde_json` decode is key-order-independent), so sorting keys fixes
 * the ONE representative per frame — two equal frames always canonicalize to the
 * same byte string, which is what makes the instrument token deterministic and
 * makes "byte-identical wire frame" a testable equality.
 */
export function canonicalWireJson(value: unknown): string {
  if (value === null || typeof value !== "object") return JSON.stringify(value);
  if (Array.isArray(value)) {
    return `[${value.map((v) => canonicalWireJson(v)).join(",")}]`;
  }
  const o = value as Record<string, unknown>;
  const keys = Object.keys(o).sort();
  const parts = keys.map((k) => `${JSON.stringify(k)}:${canonicalWireJson(o[k])}`);
  return `{${parts.join(",")}}`;
}

// ---------------------------------------------------------------------------
// scalar accessors — strict (a token decode REJECTS a malformed frame, unlike
// the lenient reply decoders in wsCodec.ts which default a missing field)
// ---------------------------------------------------------------------------

function reqNum(o: WireObject, key: string, where: string): number {
  const v = o[key];
  if (typeof v !== "number" || !Number.isFinite(v)) {
    throw new WireDecodeError(`${where}: missing/invalid number \`${key}\``);
  }
  return v;
}

function optNumOrZero(o: WireObject, key: string): number {
  const v = o[key];
  return typeof v === "number" && Number.isFinite(v) ? v : 0;
}

function reqStr(o: WireObject, key: string, where: string): string {
  const v = o[key];
  if (typeof v !== "string") {
    throw new WireDecodeError(`${where}: missing/invalid string \`${key}\``);
  }
  return v;
}

function enumOrZero(o: WireObject, key: string): number {
  const v = o[key];
  return typeof v === "number" ? v : 0;
}

function child(o: WireObject, key: string, where: string): WireObject {
  const v = o[key];
  if (v === null || typeof v !== "object" || Array.isArray(v)) {
    throw new WireDecodeError(`${where}: missing/invalid object \`${key}\``);
  }
  return v as WireObject;
}

function numArray(o: WireObject, key: string, where: string): number[] {
  const v = o[key];
  if (!Array.isArray(v) || v.some((x) => typeof x !== "number" || !Number.isFinite(x))) {
    throw new WireDecodeError(`${where}: missing/invalid number array \`${key}\``);
  }
  return v as number[];
}

/** A 64-bit wire integer (JSON number, the codec's convention) as a bigint. */
function u64(o: WireObject, key: string): bigint {
  return BigInt(Math.trunc(optNumOrZero(o, key)));
}

// ---------------------------------------------------------------------------
// piecewise decode — the exact inverse of each wsCodec encoder
// ---------------------------------------------------------------------------

function strikeOrDeltaFromWire(o: WireObject, where: string): StrikeOrDelta {
  if (typeof o["strike"] === "number") return { kind: "strike", strike: o["strike"] };
  if (typeof o["delta"] === "number") return { kind: "delta", delta: o["delta"] };
  throw new WireDecodeError(`${where}: strike-or-delta carries neither \`strike\` nor \`delta\``);
}

function vanillaFromWire(o: WireObject, where: string): Vanilla {
  return {
    optionType: e.optionType.fromWire(enumOrZero(o, "option_type")),
    strike: strikeOrDeltaFromWire(child(o, "strike", where), where),
  };
}

function legFromWire(o: WireObject, where: string): Leg {
  return {
    optionType: e.optionType.fromWire(enumOrZero(o, "option_type")),
    strike: strikeOrDeltaFromWire(child(o, "strike", where), where),
    side: e.side.fromWire(enumOrZero(o, "side")),
    ratio: reqNum(o, "ratio", where),
  };
}

function singleBarrierFromWire(o: WireObject): SingleBarrier {
  const where = "single_barrier";
  return {
    vanilla: vanillaFromWire(child(o, "vanilla", where), where),
    kind: e.barrierKind.fromWire(enumOrZero(o, "kind")),
    side: e.barrierSide.fromWire(enumOrZero(o, "side")),
    barrier: reqNum(o, "barrier", where),
    rebate: optNumOrZero(o, "rebate"),
    monitoring: e.monitoringStyle.fromWire(enumOrZero(o, "monitoring")),
  };
}

function doubleBarrierFromWire(o: WireObject): DoubleBarrier {
  const where = "double_barrier";
  return {
    vanilla: vanillaFromWire(child(o, "vanilla", where), where),
    kind: e.barrierKind.fromWire(enumOrZero(o, "kind")),
    lowerBarrier: reqNum(o, "lower_barrier", where),
    upperBarrier: reqNum(o, "upper_barrier", where),
    rebate: optNumOrZero(o, "rebate"),
    monitoring: e.monitoringStyle.fromWire(enumOrZero(o, "monitoring")),
  };
}

function digitalFromWire(o: WireObject): Digital {
  return {
    optionType: e.optionType.fromWire(enumOrZero(o, "option_type")),
    strike: reqNum(o, "strike", "digital"),
    style: e.digitalStyle.fromWire(enumOrZero(o, "style")),
    payout: optNumOrZero(o, "payout"),
  };
}

function touchFromWire(o: WireObject): Touch {
  return {
    kind: e.touchKind.fromWire(enumOrZero(o, "kind")),
    lowerBarrier: reqNum(o, "lower_barrier", "touch"),
    upperBarrier: optNumOrZero(o, "upper_barrier"),
    rebate: optNumOrZero(o, "rebate"),
    monitoring: e.monitoringStyle.fromWire(enumOrZero(o, "monitoring")),
  };
}

function windowBarrierFromWire(o: WireObject): WindowBarrier {
  const where = "window_barrier";
  return {
    vanilla: vanillaFromWire(child(o, "vanilla", where), where),
    barrier: reqNum(o, "barrier", where),
    side: e.barrierSide.fromWire(enumOrZero(o, "side")),
    windowStart: reqNum(o, "window_start", where),
    windowEnd: reqNum(o, "window_end", where),
    mcPairs: optNumOrZero(o, "mc_pairs"),
    mcSteps: optNumOrZero(o, "mc_steps"),
    mcSeed: u64(o, "mc_seed"),
  };
}

function asianFromWire(o: WireObject): AsianOption {
  return {
    optionType: e.optionType.fromWire(enumOrZero(o, "option_type")),
    strike: reqNum(o, "strike", "asian_option"),
    averaging: e.averagingStyle.fromWire(enumOrZero(o, "averaging")),
    observations: optNumOrZero(o, "observations"),
    method: e.asianMethod.fromWire(enumOrZero(o, "method")),
    elapsedAvg: optNumOrZero(o, "elapsed_avg"),
    elapsedWeight: optNumOrZero(o, "elapsed_weight"),
  };
}

function cliquetFromWire(o: WireObject): Cliquet {
  const c: Cliquet = {
    optionType: e.optionType.fromWire(enumOrZero(o, "option_type")),
    moneyness: reqNum(o, "moneyness", "cliquet"),
    periods: reqNum(o, "periods", "cliquet"),
    mcPairs: optNumOrZero(o, "mc_pairs"),
    mcSeed: u64(o, "mc_seed"),
  };
  // Presence-tracked clamps (proto `optional double` / the server's `opt_f64`):
  // an absent key stays absent — never revived as a spurious 0-clamp.
  if (typeof o["local_floor"] === "number") c.localFloor = o["local_floor"];
  if (typeof o["local_cap"] === "number") c.localCap = o["local_cap"];
  if (typeof o["global_floor"] === "number") c.globalFloor = o["global_floor"];
  if (typeof o["global_cap"] === "number") c.globalCap = o["global_cap"];
  return c;
}

function quantoFromWire(o: WireObject): Quanto {
  return {
    payoff: e.quantoPayoff.fromWire(enumOrZero(o, "payoff")),
    optionType: e.optionType.fromWire(enumOrZero(o, "option_type")),
    strike: reqNum(o, "strike", "quanto"),
    conversionVol: reqNum(o, "conversion_vol", "quanto"),
    correlation: reqNum(o, "correlation", "quanto"),
  };
}

function fixingScheduleFromWire(o: WireObject, where: string): FixingSchedule {
  return {
    fixingYears: numArray(o, "fixing_years", where),
    fixingNotional: reqNum(o, "fixing_notional", where),
  };
}

function tarfFromWire(o: WireObject): Tarf {
  const where = "tarf";
  return {
    optionType: e.optionType.fromWire(enumOrZero(o, "option_type")),
    strike: reqNum(o, "strike", where),
    target: reqNum(o, "target", where),
    leverage: reqNum(o, "leverage", where),
    redemption: e.tarfRedemption.fromWire(enumOrZero(o, "redemption")),
    schedule: fixingScheduleFromWire(child(o, "schedule", where), where),
    mcPairs: optNumOrZero(o, "mc_pairs"),
    mcSeed: u64(o, "mc_seed"),
  };
}

function accumulatorFromWire(o: WireObject): Accumulator {
  const where = "accumulator";
  return {
    pivot: reqNum(o, "pivot", where),
    barrier: reqNum(o, "barrier", where),
    leverage: reqNum(o, "leverage", where),
    monitoring: e.accumulatorMonitoring.fromWire(enumOrZero(o, "monitoring")),
    schedule: fixingScheduleFromWire(child(o, "schedule", where), where),
    mcPairs: optNumOrZero(o, "mc_pairs"),
    mcSeed: u64(o, "mc_seed"),
  };
}

function lookbackFromWire(o: WireObject): Lookback {
  return {
    style: e.lookbackStyle.fromWire(enumOrZero(o, "style")),
    optionType: e.optionType.fromWire(enumOrZero(o, "option_type")),
    monitoring: e.lookbackMonitoring.fromWire(enumOrZero(o, "monitoring")),
    strike: optNumOrZero(o, "strike"),
    observations: optNumOrZero(o, "observations"),
    mcPairs: optNumOrZero(o, "mc_pairs"),
    mcSeed: u64(o, "mc_seed"),
  };
}

function americanFromWire(o: WireObject): AmericanOption {
  const where = "american";
  return {
    optionType: e.optionType.fromWire(enumOrZero(o, "option_type")),
    strike: reqNum(o, "strike", where),
    exerciseStyle: e.exerciseStyle.fromWire(enumOrZero(o, "exercise_style")),
    bermudanDates: numArray(o, "bermudan_dates", where),
    lsmPaths: optNumOrZero(o, "lsm_paths"),
    lsmExerciseDates: optNumOrZero(o, "lsm_exercise_dates"),
    lsmSeed: u64(o, "lsm_seed"),
  };
}

function basketLegFromWire(o: WireObject, where: string): BasketLeg {
  return {
    pair: ccyPairFromWire(child(o, "pair", where)),
    weight: reqNum(o, "weight", where),
    spot: reqNum(o, "spot", where),
    vol: reqNum(o, "vol", where),
    rFor: reqNum(o, "r_for", where),
  };
}

function basketFromWire(o: WireObject): BasketOption {
  const where = "basket";
  const legsRaw = o["legs"];
  if (!Array.isArray(legsRaw) || legsRaw.length < 1) {
    throw new WireDecodeError(`${where}: missing/empty \`legs\``);
  }
  return {
    legs: legsRaw.map((l) => basketLegFromWire(l as WireObject, where)),
    correlations: numArray(o, "correlations", where),
    optionType: e.optionType.fromWire(enumOrZero(o, "option_type")),
    strike: reqNum(o, "strike", where),
    kind: e.basketKind.fromWire(enumOrZero(o, "kind")),
    mcPaths: optNumOrZero(o, "mc_paths"),
    mcReplications: optNumOrZero(o, "mc_replications"),
    mcSteps: optNumOrZero(o, "mc_steps"),
    mcSeed: u64(o, "mc_seed"),
  };
}

function fxForwardFromWire(o: WireObject, where: string): FxForward {
  return {
    contractRate: reqNum(o, "contract_rate", where),
    notional: reqNum(o, "notional", where),
    side: e.side.fromWire(enumOrZero(o, "side")),
  };
}

function ndfFromWire(o: WireObject): Ndf {
  const where = "ndf";
  return {
    contractRate: reqNum(o, "contract_rate", where),
    notional: reqNum(o, "notional", where),
    side: e.side.fromWire(enumOrZero(o, "side")),
    fixing: e.fixingSource.fromWire(enumOrZero(o, "fixing")),
    settlementCcy: reqStr(o, "settlement_ccy", where),
  };
}

/** Decode the cross-asset `underlying` oneof — the inverse of `underlyingToWire`. */
export function underlyingFromWire(o: WireObject): Underlying {
  const where = "underlying";
  const settlementCcy = reqStr(o, "settlement_ccy", where);
  if (o["fx"] !== undefined) {
    return { kind: "fx", fx: ccyPairFromWire(child(o, "fx", where)), settlementCcy };
  }
  if (o["metal"] !== undefined) {
    const m = child(o, "metal", where);
    return {
      kind: "metal",
      metal: { metal: e.metal.fromWire(enumOrZero(m, "metal")), quote: reqStr(m, "quote", where) },
      settlementCcy,
    };
  }
  if (o["equity"] !== undefined) {
    const eq = child(o, "equity", where);
    const sym = child(eq, "symbol", where);
    return {
      kind: "equity",
      equity: {
        symbol: { ticker: reqStr(sym, "ticker", where), venue: reqStr(sym, "venue", where) },
        currency: reqStr(eq, "currency", where),
      },
      settlementCcy,
    };
  }
  if (o["commodity"] !== undefined) {
    const c = child(o, "commodity", where);
    const sym = child(c, "symbol", where);
    return {
      kind: "commodity",
      commodity: {
        symbol: { ticker: reqStr(sym, "ticker", where), venue: reqStr(sym, "venue", where) },
        currency: reqStr(c, "currency", where),
      },
      settlementCcy,
    };
  }
  if (o["digital_asset"] !== undefined) {
    const d = child(o, "digital_asset", where);
    return {
      kind: "digitalAsset",
      digitalAsset: { base: reqStr(d, "base", where), quote: reqStr(d, "quote", where) },
      settlementCcy,
    };
  }
  throw new WireDecodeError(`${where}: no ref arm (fx/metal/equity/commodity/digital_asset) set`);
}

/** The product-oneof wire keys, in proto field-number order, with their decoders. */
const PRODUCT_DECODERS: ReadonlyArray<readonly [string, (o: WireObject) => Product]> = [
  ["vanilla", (o) => ({ kind: "vanilla", vanilla: vanillaFromWire(o, "vanilla") })],
  [
    "strategy",
    (o) => ({
      kind: "strategy",
      strategy: {
        kind: e.strategyKind.fromWire(enumOrZero(o, "kind")),
        legs: (Array.isArray(o["legs"]) ? (o["legs"] as WireObject[]) : []).map((l) =>
          legFromWire(l, "strategy"),
        ),
      },
    }),
  ],
  ["single_barrier", (o) => ({ kind: "singleBarrier", singleBarrier: singleBarrierFromWire(o) })],
  ["double_barrier", (o) => ({ kind: "doubleBarrier", doubleBarrier: doubleBarrierFromWire(o) })],
  ["digital", (o) => ({ kind: "digital", digital: digitalFromWire(o) })],
  ["touch", (o) => ({ kind: "touch", touch: touchFromWire(o) })],
  [
    "variance_swap",
    (o) => ({ kind: "varianceSwap", varianceSwap: { strikeVol: optNumOrZero(o, "strike_vol") } }),
  ],
  [
    "volatility_swap",
    (o) => ({
      kind: "volatilitySwap",
      volatilitySwap: { strikeVol: optNumOrZero(o, "strike_vol") },
    }),
  ],
  ["asian_option", (o) => ({ kind: "asianOption", asianOption: asianFromWire(o) })],
  [
    "forward_start",
    (o) => ({
      kind: "forwardStart",
      forwardStart: {
        optionType: e.optionType.fromWire(enumOrZero(o, "option_type")),
        moneyness: reqNum(o, "moneyness", "forward_start"),
        reset: reqNum(o, "reset", "forward_start"),
      },
    }),
  ],
  ["cliquet", (o) => ({ kind: "cliquet", cliquet: cliquetFromWire(o) })],
  ["quanto", (o) => ({ kind: "quanto", quanto: quantoFromWire(o) })],
  ["tarf", (o) => ({ kind: "tarf", tarf: tarfFromWire(o) })],
  ["accumulator", (o) => ({ kind: "accumulator", accumulator: accumulatorFromWire(o) })],
  ["lookback", (o) => ({ kind: "lookback", lookback: lookbackFromWire(o) })],
  ["window_barrier", (o) => ({ kind: "windowBarrier", windowBarrier: windowBarrierFromWire(o) })],
  ["american", (o) => ({ kind: "american", american: americanFromWire(o) })],
  ["basket", (o) => ({ kind: "basket", basket: basketFromWire(o) })],
  ["fx_forward", (o) => ({ kind: "fxForward", fxForward: fxForwardFromWire(o, "fx_forward") })],
  [
    "fx_swap",
    (o) => ({
      kind: "fxSwap",
      fxSwap: {
        near: fxForwardFromWire(child(o, "near", "fx_swap"), "fx_swap"),
        far: fxForwardFromWire(child(o, "far", "fx_swap"), "fx_swap"),
      },
    }),
  ],
  ["ndf", (o) => ({ kind: "ndf", ndf: ndfFromWire(o) })],
];

/**
 * Decode a WS-mirror instrument frame to the typed `Instrument` — the exact
 * inverse of `instrumentToWire`, with the server's presence semantics: an absent
 * `underlying` is the FX projection, an absent `settlement_style` is LINEAR, an
 * absent `pricing_model` is DEFAULT (all three stay ABSENT on the typed shape so
 * re-encoding omits them identically — `instrumentToWire(instrumentFromWire(f))`
 * reproduces `f` byte-for-byte under the canonical serialization). Exactly one
 * product arm must be present; anything else is rejected loudly.
 */
export function instrumentFromWire(o: WireObject): Instrument {
  const where = "instrument";
  const pair: CcyPair = ccyPairFromWire(child(o, "pair", where));
  const tenorWire = child(o, "tenor", where);
  const present = PRODUCT_DECODERS.filter(([key]) => o[key] !== undefined);
  if (present.length !== 1) {
    const arms = present.map(([key]) => key).join(", ") || "(none)";
    throw new WireDecodeError(`${where}: exactly one product arm required; got ${arms}`);
  }
  const [armKey, decode] = present[0] as readonly [string, (w: WireObject) => Product];
  const instrument: Instrument = {
    pair,
    tenor: {
      unit: e.tenorUnit.fromWire(enumOrZero(tenorWire, "unit")),
      count: reqNum(tenorWire, "count", where),
    },
    expiryYears: reqNum(o, "expiry_years", where),
    quantity: {
      notional: reqNum(child(o, "quantity", where), "notional", where),
      baseCcy: Boolean(child(o, "quantity", where)["base_ccy"]),
    },
    side: e.side.fromWire(enumOrZero(o, "side")),
    product: decode(child(o, armKey, where)),
  };
  if (o["underlying"] !== undefined) {
    instrument.underlying = underlyingFromWire(child(o, "underlying", where));
  }
  if (o["solve"] !== undefined) {
    const s = child(o, "solve", where);
    const target = enumOrZero(s, "target");
    const solve: Solve = {
      target: target === 1 ? "STRIKE" : target === 2 ? "PREMIUM" : "NONE",
      targetPremium: optNumOrZero(s, "target_premium"),
    };
    instrument.solve = solve;
  }
  if (o["settlement_style"] !== undefined) {
    instrument.settlementStyle = e.settlementStyle.fromWire(enumOrZero(o, "settlement_style"));
  }
  if (o["pricing_model"] !== undefined) {
    instrument.pricingModel = e.pricingModel.fromWire(enumOrZero(o, "pricing_model"));
  }
  return instrument;
}
