/**
 * Load the FROZEN golden corpus (`crates/celnet-golden/vectors/*.json`) and build,
 * for every family the GUI books over the FX WebSocket price path, the EXACT
 * `Instrument` the GUI's product registry puts on the wire — the same contract
 * shapes the per-family registry round-trip tests pin byte-for-byte against each
 * `ProductSpec.toInstrument` (`gui/test/products/*.test.ts`), here re-stated from
 * each vector's frozen terms so the REAL-edge conformance spec can price them.
 *
 * This is the GUI sibling of `excel/e2e/corpus.ts` and of the Rust SDK gate
 * (`crates/celnet-client/tests/conformance.rs`): same families, same vector-own
 * market context, same conformance conventions, same tolerance discipline — so
 * "server == GUI == oracle" is a wire-level gate for the GUI's codec/transport
 * path, not a GUI-self round-trip. The corpus is READ-ONLY here (never copied or
 * mutated); nothing under `crates/` is touched.
 *
 * Builder provenance: the per-family `Instrument` shapes are the ones the GUI's
 * OFFLINE conformance lane (`gui/test/conformance.test.ts`) builds — extended with
 * the families that lane honestly cannot price offline but the REAL server prices
 * exactly (`lookback`, `window_barrier`, and the linear `fx_forward` / `fx_swap` /
 * `ndf`), which is precisely the coverage this real-edge gate adds.
 */
import { readFileSync, readdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import type {
  AsianOption,
  BasketKind,
  BasketLeg,
  CcyPair,
  Conventions,
  FixingSource,
  Instrument,
  Leg,
  Margining,
  MarketContext,
  OptionType,
  Product,
  SettlementStyle,
  Side,
  StrategyKind,
  Underlying,
} from "../src/data/contract";
import {
  crossAssetVanillaInstrument,
  equalFixingSchedule,
  tenorYearsToTenor,
} from "../src/data/seed";

// ---------------------------------------------------------------------------
// corpus location — read the FROZEN vectors in place (never copied / mutated)
// ---------------------------------------------------------------------------

const HERE = dirname(fileURLToPath(import.meta.url));
/** `gui/e2e` → repo root → `crates/celnet-golden/vectors`. */
const VECTORS_DIR = join(HERE, "..", "..", "crates", "celnet-golden", "vectors");

// ---------------------------------------------------------------------------
// the frozen on-disk vector shape (read-only mirror; we do not own this schema)
// ---------------------------------------------------------------------------

/** The four flat market parameters a vector is priced against. */
export interface VectorMarket {
  spot: number;
  vol: number;
  r_dom: number;
  r_for: number;
}

/** The independent-oracle expectation (and any quoted Greeks). */
export interface VectorExpected {
  price: number;
  /** Present (a positive number) iff the reference was Monte-Carlo priced. */
  price_std_error: number | null;
  greeks?: Record<string, number>;
  oracle?: string;
}

/** The frozen conformance tolerance pair (loose for MC; the `k·stderr` band governs). */
export interface VectorTolerance {
  rel: number;
  abs: number;
}

/** One frozen golden vector (the on-disk JSON shape — the cross-client contract). */
export interface GoldenVector {
  id: string;
  family: string;
  underlying: string;
  tenor: string;
  market: VectorMarket;
  terms: Record<string, unknown>;
  expected: VectorExpected;
  tolerance: VectorTolerance;
}

/** Load every family file in the frozen corpus, keyed by family name. */
export function loadCorpus(): Map<string, GoldenVector[]> {
  const byFamily = new Map<string, GoldenVector[]>();
  for (const file of readdirSync(VECTORS_DIR)) {
    if (!file.endsWith(".json")) continue;
    const raw = readFileSync(join(VECTORS_DIR, file), "utf8");
    for (const v of JSON.parse(raw) as GoldenVector[]) {
      const list = byFamily.get(v.family) ?? [];
      list.push(v);
      byFamily.set(v.family, list);
    }
  }
  return byFamily;
}

// ---------------------------------------------------------------------------
// family coverage declaration (honest; every WS-priceable family is here)
// ---------------------------------------------------------------------------

/**
 * Every corpus family the GUI books onto the wire AND the server prices over the
 * cross-asset-capable WS `price` path — the real-edge conformance set. This
 * includes the families the GUI's OFFLINE lane declares not-exposed-offline but
 * the server prices exactly (`lookback` with the continuous-within-segment
 * extremum correction, `window_barrier` under LOCAL_STOCH_VOL, and the three
 * linear DCF products), AND the three cross-asset vanilla arms
 * (`equity_option` / `commodity_option` / `crypto_option`): the GUI books each
 * over the `Underlying` oneof + `settlement_style` (`crossAssetVanillaInstrument`)
 * and the server's WS decoder now treats the richer `underlying` as authoritative
 * over the legacy FX `pair` projection the GUI also emits — so the cross-asset arm
 * (and the coin-margined INVERSE_COIN crypto economics) survives end-to-end,
 * proven by `crates/celnet-server/tests/cross_asset_ws.rs`
 * (`ws_underlying_precedence_routes_client_shaped_frames_to_the_cross_asset_arm`).
 */
export const WS_PRICED_FAMILIES = [
  "vanilla",
  "strategy",
  "single_barrier",
  "double_barrier",
  "digital",
  "touch",
  "variance_swap",
  "volatility_swap",
  "asian_option",
  "forward_start",
  "cliquet",
  "quanto",
  "tarf",
  "pivot",
  "accumulator",
  "lookback",
  "window_barrier",
  "american",
  "basket",
  "fx_forward",
  "fx_swap",
  "ndf",
  "perpetual_option",
  "listed_future_option",
  "equity_option",
  "commodity_option",
  "crypto_option",
] as const;

/**
 * The corpus families this gate does NOT price over the WS path, each with a
 * concrete reason (never a silent gap — the conformance spec asserts this set).
 * The set is currently EMPTY: every corpus family — including the three
 * cross-asset vanilla arms, now that the server WS decoder routes by the
 * authoritative `underlying` oneof rather than the legacy FX `pair` projection —
 * is priced over the WS path. If a future family is genuinely not WS-priceable,
 * it lands here with a TRUE reason (never the routing bug). Identical boundary to
 * `excel/e2e/corpus.ts`.
 */
export const FAMILIES_NOT_EXPOSED_ON_FX_WS = [] as const;

// ---------------------------------------------------------------------------
// conformance conventions + per-vector market context
// ---------------------------------------------------------------------------

/**
 * The conventions every golden vector is priced under — the cross-client corpus
 * pin, identical to the Rust SDK's `Conventions::major_default()` and the Excel
 * gate's `DEFAULT_CONVENTIONS`, so the three client gates assert the SAME server
 * number. Deliberately NOT the GUI's interactive seed defaults (`src/data/seed.ts`
 * uses the premium-adjusted trader conventions): the conventions are a free
 * request parameter, and the frozen expected prices are pinned under this set.
 */
export const CONFORMANCE_CONVENTIONS: Conventions = {
  deltaConvention: "SPOT_UNADJUSTED",
  atmConvention: "ATM_FORWARD",
  premiumStyle: "DOMESTIC_PIPS",
  cut: "NEW_YORK_1000",
  dayCount: "ACT_365_FIXED",
  settlement: "DELIVERABLE",
};

/** The market context a vector is priced against (the GUI `MarketContext`). */
export function marketOf(v: GoldenVector): MarketContext {
  return { spot: v.market.spot, vol: v.market.vol, rDom: v.market.r_dom, rFor: v.market.r_for };
}

/**
 * The instrument's currency pair: the vector's own 6-letter FX underlying (the
 * NDF vectors are USDBRL/USDCOP/USDINR — the server's non-deliverable validity
 * matrix reads the pair, so it must be honest). The listed-future vectors carry a
 * contract class token (`ES`/`WTI`) instead of a pair: the GUI books that arm on
 * the active FX pair (no `underlying` oneof — the futures-measure closed form is
 * asset-class-agnostic, the quoted futures price embodies the carry), so those
 * fall back to EURUSD exactly as the GUI offline conformance lane pins.
 */
export function pairOf(v: GoldenVector): CcyPair {
  const u = v.underlying;
  if (/^[A-Z]{6}$/.test(u)) return { base: u.slice(0, 3), quote: u.slice(3) };
  return { base: "EUR", quote: "USD" };
}

// ---------------------------------------------------------------------------
// typed accessors over a vector's untyped `terms` payload
// ---------------------------------------------------------------------------

function num(terms: Record<string, unknown>, key: string): number {
  const v = terms[key];
  if (typeof v !== "number" || !Number.isFinite(v)) {
    throw new Error(`expected numeric terms.${key}, got ${JSON.stringify(v)}`);
  }
  return v;
}

function str(terms: Record<string, unknown>, key: string): string {
  const v = terms[key];
  if (typeof v !== "string") {
    throw new Error(`expected string terms.${key}, got ${JSON.stringify(v)}`);
  }
  return v;
}

function optNum(terms: Record<string, unknown>, key: string): number | undefined {
  const v = terms[key];
  if (v === null || v === undefined) return undefined;
  if (typeof v !== "number") {
    throw new Error(`expected numeric|null terms.${key}, got ${JSON.stringify(v)}`);
  }
  return v;
}

function optionType(terms: Record<string, unknown>, key = "option_type"): OptionType {
  const v = str(terms, key);
  if (v !== "CALL" && v !== "PUT") throw new Error(`unknown option_type ${v}`);
  return v;
}

/** The corpus directional side (`BUY`/`SELL`) the linear products and strategy legs carry. */
function buySell(token: string): Side {
  if (token !== "BUY" && token !== "SELL") throw new Error(`unknown side ${token}`);
  return token;
}

/** The far leg of an FX swap trades the OPPOSITE side to the near leg (market convention). */
function oppositeSide(side: Side): Side {
  return side === "BUY" ? "SELL" : "BUY";
}

/**
 * Map the corpus NDF fixing (the `celnet_types::FixingSource` variant name the
 * generator writes, e.g. `BrlPtax`) to the GUI contract's `FixingSource` token —
 * the same translation the Rust SDK and Excel conformance lanes apply.
 */
function fixingSource(variant: string): FixingSource {
  switch (variant) {
    case "KrwKftc18":
      return "KRW_KFTC18";
    case "TwdTaipei":
      return "TWD_TAIPEI";
    case "InrRbiRef":
      return "INR_RBI_REF";
    case "BrlPtax":
      return "BRL_PTAX";
    case "ClpDolarObs":
      return "CLP_DOLAR_OBS";
    case "CopTrm":
      return "COP_TRM";
    default:
      throw new Error(`unknown fixing ${variant}`);
  }
}

/**
 * Map the corpus barrier side (`UPPER`/`LOWER`, the proto `BarrierSide` enum names
 * the generator writes) to the GUI `BarrierSide` (`UP`/`DOWN`) — exactly the
 * `UPPER ⇒ UP / LOWER ⇒ DOWN` translation the other client gates do.
 */
function barrierSide(token: string): "UP" | "DOWN" {
  if (token === "UPPER") return "UP";
  if (token === "LOWER") return "DOWN";
  throw new Error(`unknown barrier side ${token}`);
}

// ---------------------------------------------------------------------------
// per-family Instrument builders — exactly the GUI's `Product` shapes
// ---------------------------------------------------------------------------

/**
 * Wrap a `Product` in the minimal `Instrument` the GUI prices (price is per-unit,
 * `quantity.notional = 1`, mirroring the GUI offline conformance lane — the
 * vector's own product-level notionals ride the product body where the family
 * carries one). The display tenor is coarse; the server prices off `expiryYears`.
 */
function instrumentOf(v: GoldenVector, product: Product): Instrument {
  const t = num(v.terms, "expiry_years");
  return {
    pair: pairOf(v),
    tenor: tenorYearsToTenor(t),
    expiryYears: t,
    quantity: { notional: 1, baseCcy: true },
    side: "TWO_WAY",
    product,
  };
}

function buildVanilla(v: GoldenVector): Instrument {
  return instrumentOf(v, {
    kind: "vanilla",
    vanilla: {
      optionType: optionType(v.terms),
      strike: { kind: "strike", strike: num(v.terms, "strike") },
    },
  });
}

function buildStrategy(v: GoldenVector): Instrument {
  const kind = str(v.terms, "kind") as StrategyKind;
  const rawLegs = v.terms["legs"];
  if (!Array.isArray(rawLegs)) throw new Error("strategy.legs is not an array");
  const legs: Leg[] = rawLegs.map((raw) => {
    const lt = raw as Record<string, unknown>;
    return {
      optionType: optionType(lt),
      strike: { kind: "strike", strike: num(lt, "strike") },
      side: buySell(str(lt, "side")),
      ratio: num(lt, "ratio"),
    };
  });
  return instrumentOf(v, { kind: "strategy", strategy: { kind, legs } });
}

function buildSingleBarrier(v: GoldenVector): Instrument {
  const kind = str(v.terms, "kind");
  if (kind !== "KNOCK_IN" && kind !== "KNOCK_OUT") throw new Error(`barrier kind ${kind}`);
  const monitoring = str(v.terms, "monitoring");
  if (monitoring !== "CONTINUOUS" && monitoring !== "DISCRETE") {
    throw new Error(`monitoring ${monitoring}`);
  }
  return instrumentOf(v, {
    kind: "singleBarrier",
    singleBarrier: {
      vanilla: {
        optionType: optionType(v.terms),
        strike: { kind: "strike", strike: num(v.terms, "strike") },
      },
      kind,
      side: barrierSide(str(v.terms, "side")),
      barrier: num(v.terms, "barrier"),
      rebate: num(v.terms, "rebate"),
      monitoring,
    },
  });
}

function buildDoubleBarrier(v: GoldenVector): Instrument {
  const kind = str(v.terms, "kind");
  if (kind !== "KNOCK_IN" && kind !== "KNOCK_OUT") throw new Error(`barrier kind ${kind}`);
  const monitoring = str(v.terms, "monitoring");
  if (monitoring !== "CONTINUOUS" && monitoring !== "DISCRETE") {
    throw new Error(`monitoring ${monitoring}`);
  }
  return instrumentOf(v, {
    kind: "doubleBarrier",
    doubleBarrier: {
      vanilla: {
        optionType: optionType(v.terms),
        strike: { kind: "strike", strike: num(v.terms, "strike") },
      },
      kind,
      lowerBarrier: num(v.terms, "lower_barrier"),
      upperBarrier: num(v.terms, "upper_barrier"),
      rebate: num(v.terms, "rebate"),
      monitoring,
    },
  });
}

function buildDigital(v: GoldenVector): Instrument {
  const style = str(v.terms, "style");
  if (style !== "CASH_OR_NOTHING" && style !== "ASSET_OR_NOTHING") {
    throw new Error(`digital style ${style}`);
  }
  return instrumentOf(v, {
    kind: "digital",
    digital: {
      optionType: optionType(v.terms),
      strike: num(v.terms, "strike"),
      style,
      payout: num(v.terms, "payout"),
    },
  });
}

function buildTouch(v: GoldenVector): Instrument {
  const kind = str(v.terms, "kind");
  if (
    kind !== "ONE_TOUCH" &&
    kind !== "NO_TOUCH" &&
    kind !== "DOUBLE_NO_TOUCH" &&
    kind !== "DOUBLE_ONE_TOUCH"
  ) {
    throw new Error(`touch kind ${kind}`);
  }
  const monitoring = str(v.terms, "monitoring");
  if (monitoring !== "CONTINUOUS" && monitoring !== "DISCRETE") {
    throw new Error(`monitoring ${monitoring}`);
  }
  return instrumentOf(v, {
    kind: "touch",
    touch: {
      kind,
      lowerBarrier: num(v.terms, "lower_barrier"),
      upperBarrier: num(v.terms, "upper_barrier"),
      rebate: num(v.terms, "rebate"),
      monitoring,
    },
  });
}

function buildVarianceSwap(v: GoldenVector): Instrument {
  return instrumentOf(v, {
    kind: "varianceSwap",
    varianceSwap: { strikeVol: num(v.terms, "strike_vol") },
  });
}

function buildVolatilitySwap(v: GoldenVector): Instrument {
  return instrumentOf(v, {
    kind: "volatilitySwap",
    volatilitySwap: { strikeVol: num(v.terms, "strike_vol") },
  });
}

function buildAsian(v: GoldenVector): Instrument {
  const averaging = str(v.terms, "averaging");
  if (averaging !== "DISCRETE" && averaging !== "CONTINUOUS") {
    throw new Error(`averaging ${averaging}`);
  }
  const method = str(v.terms, "method");
  if (method !== "CURRAN" && method !== "TURNBULL_WAKEMAN") {
    throw new Error(`asian method ${method}`);
  }
  const asian: AsianOption = {
    optionType: optionType(v.terms),
    strike: num(v.terms, "strike"),
    averaging,
    observations: num(v.terms, "observations"),
    method,
    elapsedAvg: num(v.terms, "elapsed_avg"),
    elapsedWeight: num(v.terms, "elapsed_weight"),
  };
  return instrumentOf(v, { kind: "asianOption", asianOption: asian });
}

function buildForwardStart(v: GoldenVector): Instrument {
  return instrumentOf(v, {
    kind: "forwardStart",
    forwardStart: {
      optionType: optionType(v.terms),
      moneyness: num(v.terms, "moneyness"),
      reset: num(v.terms, "reset"),
    },
  });
}

function buildCliquet(v: GoldenVector): Instrument {
  const localFloor = optNum(v.terms, "local_floor");
  const localCap = optNum(v.terms, "local_cap");
  const globalFloor = optNum(v.terms, "global_floor");
  const globalCap = optNum(v.terms, "global_cap");
  const cliquet = {
    optionType: optionType(v.terms),
    moneyness: num(v.terms, "moneyness"),
    periods: num(v.terms, "periods"),
    ...(localFloor !== undefined ? { localFloor } : {}),
    ...(localCap !== undefined ? { localCap } : {}),
    ...(globalFloor !== undefined ? { globalFloor } : {}),
    ...(globalCap !== undefined ? { globalCap } : {}),
    mcPairs: num(v.terms, "mc_pairs"),
    mcSeed: BigInt(num(v.terms, "mc_seed")),
  };
  return instrumentOf(v, { kind: "cliquet", cliquet });
}

function buildQuanto(v: GoldenVector): Instrument {
  const payoff = str(v.terms, "payoff");
  if (payoff !== "VANILLA" && payoff !== "DIGITAL") throw new Error(`quanto payoff ${payoff}`);
  return instrumentOf(v, {
    kind: "quanto",
    quanto: {
      payoff,
      optionType: optionType(v.terms),
      strike: num(v.terms, "strike"),
      conversionVol: num(v.terms, "conversion_vol"),
      correlation: num(v.terms, "correlation"),
    },
  });
}

function buildTarf(v: GoldenVector): Instrument {
  const redemption = str(v.terms, "redemption");
  if (redemption !== "FULL_GAIN" && redemption !== "CAPPED_GAIN") {
    throw new Error(`tarf redemption ${redemption}`);
  }
  const t = num(v.terms, "expiry_years");
  return instrumentOf(v, {
    kind: "tarf",
    tarf: {
      optionType: optionType(v.terms),
      strike: num(v.terms, "strike"),
      target: num(v.terms, "target"),
      leverage: num(v.terms, "leverage"),
      redemption,
      schedule: equalFixingSchedule(num(v.terms, "fixings"), t, num(v.terms, "fixing_notional")),
      mcPairs: num(v.terms, "mc_pairs"),
      mcSeed: BigInt(num(v.terms, "mc_seed")),
    },
  });
}

function buildPivot(v: GoldenVector): Instrument {
  const redemption = str(v.terms, "redemption");
  if (redemption !== "FULL_GAIN" && redemption !== "CAPPED_GAIN") {
    throw new Error(`pivot redemption ${redemption}`);
  }
  const t = num(v.terms, "expiry_years");
  return instrumentOf(v, {
    kind: "pivot",
    pivot: {
      optionType: optionType(v.terms),
      strike: num(v.terms, "strike"),
      pivot: num(v.terms, "pivot"),
      target: num(v.terms, "target"),
      leverage: num(v.terms, "leverage"),
      redemption,
      schedule: equalFixingSchedule(num(v.terms, "fixings"), t, num(v.terms, "fixing_notional")),
      mcPairs: num(v.terms, "mc_pairs"),
      mcSeed: BigInt(num(v.terms, "mc_seed")),
    },
  });
}

function buildAccumulator(v: GoldenVector): Instrument {
  const monitoring = str(v.terms, "monitoring");
  if (monitoring !== "DISCRETE" && monitoring !== "CONTINUOUS") {
    throw new Error(`acc monitoring ${monitoring}`);
  }
  const t = num(v.terms, "expiry_years");
  return instrumentOf(v, {
    kind: "accumulator",
    accumulator: {
      pivot: num(v.terms, "pivot"),
      barrier: num(v.terms, "barrier"),
      leverage: num(v.terms, "leverage"),
      monitoring,
      schedule: equalFixingSchedule(num(v.terms, "fixings"), t, num(v.terms, "fixing_notional")),
      mcPairs: num(v.terms, "mc_pairs"),
      mcSeed: BigInt(num(v.terms, "mc_seed")),
    },
  });
}

function buildLookback(v: GoldenVector): Instrument {
  const style = str(v.terms, "style");
  if (style !== "FLOATING" && style !== "FIXED") throw new Error(`lookback style ${style}`);
  const monitoring = str(v.terms, "monitoring");
  if (monitoring !== "CONTINUOUS" && monitoring !== "DISCRETE") {
    throw new Error(`lookback monitoring ${monitoring}`);
  }
  return instrumentOf(v, {
    kind: "lookback",
    lookback: {
      style,
      optionType: optionType(v.terms),
      monitoring,
      strike: num(v.terms, "strike"),
      observations: num(v.terms, "observations"),
      mcPairs: num(v.terms, "mc_pairs"),
      mcSeed: BigInt(num(v.terms, "mc_seed")),
    },
  });
}

/**
 * The window barrier has NO closed form: its only engines are the server's
 * LOCAL_STOCH_VOL routes (ADI-PDE, or MC when `mcPairs > 0` — the corpus pins an
 * explicit MC budget/seed), so the instrument carries the locked booking model
 * exactly as the GUI's `windowBarrierSpec` locks it.
 */
function buildWindowBarrier(v: GoldenVector): Instrument {
  return {
    ...instrumentOf(v, {
      kind: "windowBarrier",
      windowBarrier: {
        vanilla: {
          optionType: optionType(v.terms),
          strike: { kind: "strike", strike: num(v.terms, "strike") },
        },
        barrier: num(v.terms, "barrier"),
        side: barrierSide(str(v.terms, "side")),
        windowStart: num(v.terms, "window_start"),
        windowEnd: num(v.terms, "window_end"),
        mcPairs: num(v.terms, "mc_pairs"),
        mcSteps: num(v.terms, "mc_steps"),
        mcSeed: BigInt(num(v.terms, "mc_seed")),
      },
    }),
    pricingModel: "LOCAL_STOCH_VOL",
  };
}

function buildAmerican(v: GoldenVector): Instrument {
  const style = str(v.terms, "style");
  if (style !== "AMERICAN" && style !== "BERMUDAN") throw new Error(`exercise style ${style}`);
  const rawDates = v.terms["bermudan_dates"];
  const bermudanDates = Array.isArray(rawDates) ? (rawDates as number[]) : [];
  return instrumentOf(v, {
    kind: "american",
    american: {
      optionType: optionType(v.terms),
      strike: num(v.terms, "strike"),
      exerciseStyle: style,
      bermudanDates,
      lsmPaths: num(v.terms, "lsm_paths"),
      lsmExerciseDates: num(v.terms, "lsm_exercise_dates"),
      lsmSeed: BigInt(num(v.terms, "lsm_seed")),
    },
  });
}

function buildBasket(v: GoldenVector): Instrument {
  const kind = str(v.terms, "kind") as BasketKind;
  const rawLegs = v.terms["legs"];
  if (!Array.isArray(rawLegs)) throw new Error("basket.legs is not an array");
  const legs: BasketLeg[] = rawLegs.map((raw) => {
    const lt = raw as Record<string, unknown>;
    const pairStr = str(lt, "pair");
    if (pairStr.length !== 6) throw new Error(`basket leg pair ${pairStr}`);
    return {
      pair: { base: pairStr.slice(0, 3), quote: pairStr.slice(3) },
      weight: num(lt, "weight"),
      spot: num(lt, "spot"),
      vol: num(lt, "vol"),
      rFor: num(lt, "r_for"),
    };
  });
  const rawCorr = v.terms["correlations"];
  if (!Array.isArray(rawCorr)) throw new Error("basket.correlations is not an array");
  return instrumentOf(v, {
    kind: "basket",
    basket: {
      legs,
      correlations: rawCorr as number[],
      optionType: optionType(v.terms),
      strike: num(v.terms, "strike"),
      kind,
      mcPaths: num(v.terms, "mc_paths"),
      mcReplications: num(v.terms, "mc_replications"),
      mcSteps: num(v.terms, "mc_steps"),
      mcSeed: BigInt(num(v.terms, "mc_seed")),
    },
  });
}

function buildFxForward(v: GoldenVector): Instrument {
  return instrumentOf(v, {
    kind: "fxForward",
    fxForward: {
      contractRate: num(v.terms, "contract_rate"),
      notional: num(v.terms, "notional"),
      side: buySell(str(v.terms, "side")),
    },
  });
}

/**
 * An FX swap from a corpus vector: one `contract_rate` rides both legs and the
 * far leg takes the OPPOSITE side to the near leg — the same near-leg-only
 * derivation the Rust SDK conformance (`SwapTerms::new(near)`) and the GUI's
 * `swapInstrument` apply.
 */
function buildFxSwap(v: GoldenVector): Instrument {
  const rate = num(v.terms, "contract_rate");
  const notional = num(v.terms, "notional");
  const nearSide = buySell(str(v.terms, "near_side"));
  return instrumentOf(v, {
    kind: "fxSwap",
    fxSwap: {
      near: { contractRate: rate, notional, side: nearSide },
      far: { contractRate: rate, notional, side: oppositeSide(nearSide) },
    },
  });
}

function buildNdf(v: GoldenVector): Instrument {
  return instrumentOf(v, {
    kind: "ndf",
    ndf: {
      contractRate: num(v.terms, "contract_rate"),
      notional: num(v.terms, "notional"),
      side: buySell(str(v.terms, "side")),
      fixing: fixingSource(str(v.terms, "fixing")),
      settlementCcy: str(v.terms, "settlement_ccy"),
    },
  });
}

function buildPerpetual(v: GoldenVector): Instrument {
  // The ONE tenorless product: the corpus encodes the contract's canonical
  // no-expiry shape (`expiry_years` exactly 0), asserted here — a perpetual
  // vector carrying a dated expiry would be a corpus defect, never coerced.
  const t = num(v.terms, "expiry_years");
  if (t !== 0) throw new Error(`perpetual vector ${v.id} carries a non-zero expiry ${t}`);
  return {
    pair: pairOf(v),
    expiryYears: 0,
    quantity: { notional: 1, baseCcy: true },
    side: "TWO_WAY",
    product: {
      kind: "perpetualOption",
      perpetualOption: {
        optionType: optionType(v.terms),
        strike: num(v.terms, "strike"),
        notional: num(v.terms, "notional"),
      },
    },
  };
}

function buildListedFutureOption(v: GoldenVector): Instrument {
  const margining = str(v.terms, "margining") as Margining;
  if (margining !== "EQUITY_STYLE" && margining !== "FUTURES_STYLE") {
    throw new Error(`margining ${margining}`);
  }
  const rawSymbol = v.terms["future_symbol"];
  if (rawSymbol === null || typeof rawSymbol !== "object") {
    throw new Error("listed_future_option.future_symbol is not an object");
  }
  const symbolTerms = rawSymbol as Record<string, unknown>;
  return instrumentOf(v, {
    kind: "listedFutureOption",
    listedFutureOption: {
      futureSymbol: {
        ticker: str(symbolTerms, "ticker"),
        venue: str(symbolTerms, "venue"),
      },
      futureExpiryYears: num(v.terms, "future_expiry_years"),
      optionType: optionType(v.terms),
      strike: num(v.terms, "strike"),
      notional: num(v.terms, "notional"),
      margining,
    },
  });
}

// ---------------------------------------------------------------------------
// cross-asset vanilla builders (equity / commodity / digital-asset)
// ---------------------------------------------------------------------------

/**
 * The listing venue MIC for each corpus equity underlying — the contract identity
 * the `EquityRef.symbol.venue` carries (the price is venue-independent; the venue
 * is identity only, ADR-0008). Mirrors the server gate's `underlying_json` map so
 * the GUI books the identical `Underlying` arm the server-side test exercises.
 */
const EQUITY_VENUE: Record<string, { venue: string; currency: string }> = {
  SPX: { venue: "XCBO", currency: "USD" },
  AAPL: { venue: "XNAS", currency: "USD" },
  STOXX: { venue: "XEUR", currency: "EUR" },
};

/**
 * Wrap a `crossAssetVanillaInstrument` to the conformance discipline: the vector's
 * EXACT `expiry_years` is the authoritative pricing maturity (the seed builder
 * derives it from the coarse display tenor) and `quantity.notional = 1` matches
 * the per-unit oracle (`USD/coin per notional-1`), exactly as the FX `instrumentOf`
 * does. The `underlying` oneof is authoritative; the FX `pair` projection rides
 * alongside (the GUI keeps the FX-keyed surfaces total), and the server's WS
 * decoder now prefers `underlying` — so the cross-asset arm prices, not its FX
 * projection.
 */
function crossAssetInstrumentOf(
  v: GoldenVector,
  underlying: Underlying,
  settlementStyle: SettlementStyle,
): Instrument {
  const t = num(v.terms, "expiry_years");
  const inst = crossAssetVanillaInstrument(t, 1, {
    optionType: optionType(v.terms),
    strike: { kind: "strike", strike: num(v.terms, "strike") },
    underlying,
    settlementStyle,
  });
  return { ...inst, expiryYears: t, quantity: { notional: 1, baseCcy: true } };
}

function buildEquityOption(v: GoldenVector): Instrument {
  const map = EQUITY_VENUE[v.underlying];
  if (!map) throw new Error(`unmapped equity underlying ${v.underlying}`);
  return crossAssetInstrumentOf(
    v,
    {
      kind: "equity",
      equity: { symbol: { ticker: v.underlying, venue: map.venue }, currency: map.currency },
      settlementCcy: map.currency,
    },
    "LINEAR",
  );
}

function buildCommodityOption(v: GoldenVector): Instrument {
  // Venue-less contract identity (the corpus commodities quote in USD); the venue
  // lives on a listed-future's `future_symbol`, never on the spot commodity arm.
  return crossAssetInstrumentOf(
    v,
    {
      kind: "commodity",
      commodity: { symbol: { ticker: v.underlying, venue: "" }, currency: "USD" },
      settlementCcy: "USD",
    },
    "LINEAR",
  );
}

function buildCryptoOption(v: GoldenVector): Instrument {
  const u = v.underlying;
  if (u.length !== 6) throw new Error(`crypto underlying ${u} is not a 3+3 pair token`);
  const base = u.slice(0, 3);
  const quote = u.slice(3);
  const style = str(v.terms, "settlement_style");
  if (style !== "LINEAR" && style !== "INVERSE_COIN") {
    throw new Error(`crypto settlement_style ${style}`);
  }
  // The settlement currency is the pair's numeraire (quote) leg; INVERSE_COIN
  // selects the coin-margined `1/S_T` payoff (priced in coins, the oracle's unit).
  return crossAssetInstrumentOf(
    v,
    { kind: "digitalAsset", digitalAsset: { base, quote }, settlementCcy: quote },
    style,
  );
}

/** Dispatch a vector to the GUI `Instrument` its family books. */
const BUILDERS: Record<(typeof WS_PRICED_FAMILIES)[number], (v: GoldenVector) => Instrument> = {
  vanilla: buildVanilla,
  strategy: buildStrategy,
  single_barrier: buildSingleBarrier,
  double_barrier: buildDoubleBarrier,
  digital: buildDigital,
  touch: buildTouch,
  variance_swap: buildVarianceSwap,
  volatility_swap: buildVolatilitySwap,
  asian_option: buildAsian,
  forward_start: buildForwardStart,
  cliquet: buildCliquet,
  quanto: buildQuanto,
  tarf: buildTarf,
  pivot: buildPivot,
  accumulator: buildAccumulator,
  lookback: buildLookback,
  window_barrier: buildWindowBarrier,
  american: buildAmerican,
  basket: buildBasket,
  fx_forward: buildFxForward,
  fx_swap: buildFxSwap,
  ndf: buildNdf,
  perpetual_option: buildPerpetual,
  listed_future_option: buildListedFutureOption,
  equity_option: buildEquityOption,
  commodity_option: buildCommodityOption,
  crypto_option: buildCryptoOption,
};

/** Build the EXACT GUI `Instrument` for a WS-priced vector. */
export function instrumentOfVector(v: GoldenVector): Instrument {
  const build = BUILDERS[v.family as (typeof WS_PRICED_FAMILIES)[number]];
  if (!build) throw new Error(`family ${v.family} is not WS-priced by the GUI`);
  return build(v);
}
