/**
 * Load the FROZEN golden corpus (`crates/celnet-golden/vectors/*.json`) and build,
 * for every family the Excel `CELNET.*` functions expose, the EXACT `Instrument`
 * that function shapes — using the production add-in shaping functions
 * (`src/functions/shaping.ts`), the same code the worksheet functions call. The
 * conformance spec then prices each instrument through the REAL add-in `Connection`
 * over a REAL WebSocket to a REAL booted edge, asserting the server's price equals
 * the vector's independent oracle within the vector's frozen tolerance (a `k·stderr`
 * band for the Monte-Carlo families).
 *
 * This mirrors the Rust SDK conformance gate (`celnet-client/tests/conformance.rs`)
 * field-for-field — same instruments, same vector-own market context, same `k=4`
 * band — so "server == Excel == oracle" is a wire-level gate, not a claim. The
 * corpus is READ-ONLY here; nothing under `crates/` is modified.
 */
import { readFileSync, readdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import type { Instrument, MarketContext } from "../src/contract/contract";
import {
  shapeAccumulator,
  shapeAmerican,
  shapeAsianOption,
  shapeBarrier,
  shapeBasket,
  shapeCliquet,
  shapeDigital,
  shapeForward,
  shapeForwardStart,
  shapeLookback,
  shapeNdf,
  shapeQuanto,
  shapeSwap,
  shapeTarf,
  shapeTouch,
  shapeVanillaInstrument,
  shapeVarianceSwap,
  shapeVolatilitySwap,
  shapeWindowBarrier,
} from "../src/functions/shaping";

const HERE = dirname(fileURLToPath(import.meta.url));
const VECTORS_DIR = resolve(HERE, "..", "..", "crates", "celnet-golden", "vectors");

/** The four Garman-Kohlhagen market parameters a vector is priced against. */
export interface VectorMarket {
  spot: number;
  vol: number;
  r_dom: number;
  r_for: number;
}

/** The independent-oracle expectation (and any quoted Greeks). */
export interface VectorExpected {
  price: number;
  /** Present (a positive number) iff the family is Monte-Carlo priced. */
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

/** The 21 product-oneof family names that may appear in the corpus. */
export const ALL_FAMILIES = [
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
  "accumulator",
  "lookback",
  "window_barrier",
  "american",
  "basket",
  "fx_forward",
  "fx_swap",
  "ndf",
] as const;

/**
 * The families the Excel `CELNET.*` worksheet functions actually expose (each has a
 * `shape*` builder + a worksheet function in `src/functions/functions.ts`). The one
 * family present in the corpus but NOT exposed by Excel is `strategy` (there is no
 * `CELNET.STRATEGY` worksheet function — strategies are built leg-by-leg in the GUI
 * ticket / SDK, not as a single Excel cell). It is reported in `FAMILIES_NOT_EXPOSED`.
 */
export const EXCEL_FAMILIES = [
  "vanilla",
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
  "accumulator",
  "lookback",
  "window_barrier",
  "american",
  "basket",
  "fx_forward",
  "fx_swap",
  "ndf",
] as const;

/** Families in the corpus that no `CELNET.*` worksheet function exposes. */
export const FAMILIES_NOT_EXPOSED = ALL_FAMILIES.filter(
  (f) => !(EXCEL_FAMILIES as readonly string[]).includes(f),
);

/** Load every frozen vector from the read-only corpus directory. */
export function loadVectors(): GoldenVector[] {
  const out: GoldenVector[] = [];
  for (const file of readdirSync(VECTORS_DIR)) {
    if (!file.endsWith(".json")) continue;
    const raw = readFileSync(join(VECTORS_DIR, file), "utf8");
    const parsed = JSON.parse(raw) as GoldenVector[];
    for (const v of parsed) out.push(v);
  }
  return out;
}

/** The market context a vector is priced against (the add-in `MarketContext`). */
export function marketOf(v: GoldenVector): MarketContext {
  return { spot: v.market.spot, vol: v.market.vol, rDom: v.market.r_dom, rFor: v.market.r_for };
}

const num = (terms: Record<string, unknown>, key: string): number => {
  const x = terms[key];
  if (typeof x !== "number" || !Number.isFinite(x)) {
    throw new Error(`term \`${key}\` missing/not a number`);
  }
  return x;
};
const str = (terms: Record<string, unknown>, key: string): string => {
  const x = terms[key];
  if (typeof x !== "string") throw new Error(`term \`${key}\` missing/not a string`);
  return x;
};
const optNum = (terms: Record<string, unknown>, key: string): number | undefined => {
  const x = terms[key];
  return typeof x === "number" && Number.isFinite(x) ? x : undefined;
};
const cp = (token: string): "C" | "P" => {
  if (token === "CALL") return "C";
  if (token === "PUT") return "P";
  throw new Error(`unknown option type \`${token}\``);
};

/**
 * Map the corpus barrier side (`UPPER`/`LOWER`, the proto `BarrierSide` enum names
 * the generator writes) to the trader-facing `UP`/`DOWN` the Excel shaper parses —
 * exactly the `UPPER => Up / LOWER => Down` translation the Rust SDK conformance does.
 */
const barrierSide = (token: string): "UP" | "DOWN" => {
  if (token === "UPPER") return "UP";
  if (token === "LOWER") return "DOWN";
  throw new Error(`unknown barrier side \`${token}\``);
};

/** The corpus directional side (`BUY`/`SELL`) the linear shapers parse. */
const linearSide = (token: string): "BUY" | "SELL" => {
  if (token === "BUY" || token === "SELL") return token;
  throw new Error(`unknown side \`${token}\``);
};

/**
 * Map the corpus NDF fixing (the `celnet_types::FixingSource` variant name the
 * generator writes, e.g. `BrlPtax`) to the dotted code the Excel shaper parses
 * (`BRL.PTAX`). The shaper itself accepts several spellings; the dotted code is the
 * canonical wire-stable form.
 */
const fixingToken = (variant: string): string => {
  switch (variant) {
    case "KrwKftc18":
      return "KRW.KFTC18";
    case "TwdTaipei":
      return "TWD.TAIPEI";
    case "InrRbiRef":
      return "INR.RBIB";
    case "BrlPtax":
      return "BRL.PTAX";
    case "ClpDolarObs":
      return "CLP.DOLAROBS";
    case "CopTrm":
      return "COP.TRM";
    default:
      throw new Error(`unknown fixing \`${variant}\``);
  }
};

/**
 * A display tenor for an arbitrary expiry year-fraction. The PRICED maturity is the
 * vector's exact `expiry_years` (we override `expiryYears` below); the tenor is a
 * coarse display label only (the server prices off `expiry_years`, never the tenor),
 * exactly as the Rust conformance `tenor_for` does.
 */
function tenorFor(t: number): string {
  const months = Math.round(t * 12);
  if (months > 0 && months % 12 === 0) return `${months / 12}Y`;
  return `${Math.max(1, months)}M`;
}

/**
 * Build the EXACT `Instrument` the Excel `CELNET.*` function for this family shapes,
 * via the production `shape*` functions. The shaper derives `expiryYears` from the
 * coarse display tenor; we then override it with the vector's exact `expiry_years`
 * (the authoritative pricing maturity on the wire). Monte-Carlo families are sent
 * with the server-default budget (`mcPairs = 0`) — exactly as the SDK conformance
 * does — and the `k·(oracle_se + server_se)` band governs; `window_barrier` carries
 * its explicit MC budget from the corpus (its only engine is MC).
 */
export function instrumentOf(v: GoldenVector): Instrument {
  const t = num(v.terms, "expiry_years");
  const tenor = tenorFor(t);
  const pair = v.underlying;
  const notional = 1.0;
  let inst: Instrument;

  switch (v.family) {
    case "vanilla":
      inst = shapeVanillaInstrument({
        pair,
        tenor,
        strikeOrDelta: num(v.terms, "strike"),
        callPut: cp(str(v.terms, "option_type")),
        notional,
      });
      break;
    case "single_barrier":
      inst = shapeBarrier({
        pair,
        tenor,
        strikeOrDelta: num(v.terms, "strike"),
        callPut: cp(str(v.terms, "option_type")),
        notional,
        barrier: num(v.terms, "barrier"),
        kind: str(v.terms, "kind"),
        side: barrierSide(str(v.terms, "side")),
        rebate: num(v.terms, "rebate"),
        monitoring: str(v.terms, "monitoring"),
      });
      break;
    case "double_barrier":
      inst = shapeBarrier({
        pair,
        tenor,
        strikeOrDelta: num(v.terms, "strike"),
        callPut: cp(str(v.terms, "option_type")),
        notional,
        barrier: num(v.terms, "lower_barrier"),
        upperBarrier: num(v.terms, "upper_barrier"),
        kind: str(v.terms, "kind"),
        rebate: num(v.terms, "rebate"),
        monitoring: str(v.terms, "monitoring"),
      });
      break;
    case "digital":
      inst = shapeDigital({
        pair,
        tenor,
        strike: num(v.terms, "strike"),
        callPut: cp(str(v.terms, "option_type")),
        notional,
        style: str(v.terms, "style"),
        payout: num(v.terms, "payout"),
      });
      break;
    case "touch": {
      const kind = str(v.terms, "kind");
      const isDouble = kind === "DOUBLE_NO_TOUCH" || kind === "DOUBLE_ONE_TOUCH";
      inst = shapeTouch({
        pair,
        tenor,
        notional,
        kind,
        barrier: num(v.terms, "lower_barrier"),
        ...(isDouble ? { upperBarrier: num(v.terms, "upper_barrier") } : {}),
        rebate: num(v.terms, "rebate"),
        monitoring: str(v.terms, "monitoring"),
      });
      break;
    }
    case "variance_swap":
      inst = shapeVarianceSwap({ pair, tenor, notional, strikeVol: num(v.terms, "strike_vol") });
      break;
    case "volatility_swap":
      inst = shapeVolatilitySwap({ pair, tenor, notional, strikeVol: num(v.terms, "strike_vol") });
      break;
    case "asian_option":
      inst = shapeAsianOption({
        pair,
        tenor,
        strike: num(v.terms, "strike"),
        callPut: cp(str(v.terms, "option_type")),
        notional,
        averaging: str(v.terms, "averaging"),
        observations: num(v.terms, "observations"),
        method: str(v.terms, "method"),
      });
      break;
    case "forward_start":
      inst = shapeForwardStart({
        pair,
        tenor,
        callPut: cp(str(v.terms, "option_type")),
        moneyness: num(v.terms, "moneyness"),
        reset: num(v.terms, "reset"),
        notional,
      });
      break;
    case "cliquet": {
      const lf = optNum(v.terms, "local_floor");
      const lc = optNum(v.terms, "local_cap");
      const gf = optNum(v.terms, "global_floor");
      const gc = optNum(v.terms, "global_cap");
      inst = shapeCliquet({
        pair,
        tenor,
        callPut: cp(str(v.terms, "option_type")),
        moneyness: num(v.terms, "moneyness"),
        periods: num(v.terms, "periods"),
        notional,
        ...(lf !== undefined ? { localFloor: lf } : {}),
        ...(lc !== undefined ? { localCap: lc } : {}),
        ...(gf !== undefined ? { globalFloor: gf } : {}),
        ...(gc !== undefined ? { globalCap: gc } : {}),
      });
      break;
    }
    case "quanto":
      inst = shapeQuanto({
        pair,
        tenor,
        callPut: cp(str(v.terms, "option_type")),
        strike: num(v.terms, "strike"),
        notional,
        conversionVol: num(v.terms, "conversion_vol"),
        correlation: num(v.terms, "correlation"),
        payoff: str(v.terms, "payoff"),
      });
      break;
    case "tarf":
      inst = shapeTarf({
        pair,
        tenor,
        callPut: cp(str(v.terms, "option_type")),
        strike: num(v.terms, "strike"),
        target: num(v.terms, "target"),
        leverage: num(v.terms, "leverage"),
        fixings: num(v.terms, "fixings"),
        notional,
        redemption: str(v.terms, "redemption"),
        fixingNotional: num(v.terms, "fixing_notional"),
      });
      break;
    case "accumulator":
      inst = shapeAccumulator({
        pair,
        tenor,
        pivot: num(v.terms, "pivot"),
        barrier: num(v.terms, "barrier"),
        leverage: num(v.terms, "leverage"),
        fixings: num(v.terms, "fixings"),
        notional,
        monitoring: str(v.terms, "monitoring"),
        fixingNotional: num(v.terms, "fixing_notional"),
      });
      break;
    case "lookback": {
      const style = str(v.terms, "style");
      inst = shapeLookback({
        pair,
        tenor,
        callPut: cp(str(v.terms, "option_type")),
        notional,
        style,
        monitoring: str(v.terms, "monitoring"),
        ...(style === "FIXED" ? { strike: num(v.terms, "strike") } : {}),
        observations: optNum(v.terms, "observations"),
      });
      break;
    }
    case "window_barrier":
      // The window barrier's ONLY engine is Monte-Carlo (no closed form); the corpus
      // encodes its exact MC budget/seed and the SDK conformance passes them through.
      inst = shapeWindowBarrier({
        pair,
        tenor,
        strikeOrDelta: num(v.terms, "strike"),
        callPut: cp(str(v.terms, "option_type")),
        notional,
        barrier: num(v.terms, "barrier"),
        side: barrierSide(str(v.terms, "side")),
        windowStart: num(v.terms, "window_start"),
        windowEnd: num(v.terms, "window_end"),
        mcPairs: num(v.terms, "mc_pairs"),
        mcSteps: num(v.terms, "mc_steps"),
        mcSeed: num(v.terms, "mc_seed"),
      });
      break;
    case "american":
      inst = shapeAmerican({
        pair,
        tenor,
        strike: num(v.terms, "strike"),
        callPut: cp(str(v.terms, "option_type")),
        notional,
      });
      break;
    case "basket": {
      const legsRaw = v.terms["legs"] as Array<Record<string, unknown>>;
      const legs = legsRaw.map((leg) => [
        String(leg["pair"]),
        Number(leg["weight"]),
        Number(leg["spot"]),
        Number(leg["vol"]),
        Number(leg["r_for"]),
      ]);
      const corrFlat = (v.terms["correlations"] as number[]).map(Number);
      const n = legs.length;
      const corr: number[][] = [];
      for (let i = 0; i < n; i++) corr.push(corrFlat.slice(i * n, i * n + n));
      inst = shapeBasket({
        pair,
        tenor,
        notional,
        callPut: cp(str(v.terms, "option_type")),
        strike: num(v.terms, "strike"),
        kind: str(v.terms, "kind"),
        legs,
        correlations: corr,
      });
      break;
    }
    case "fx_forward":
      inst = shapeForward({
        pair,
        tenor,
        contractRate: num(v.terms, "contract_rate"),
        notional: num(v.terms, "notional"),
        side: linearSide(str(v.terms, "side")),
      });
      break;
    case "fx_swap":
      inst = shapeSwap({
        pair,
        tenor,
        contractRate: num(v.terms, "contract_rate"),
        notional: num(v.terms, "notional"),
        side: linearSide(str(v.terms, "near_side")),
      });
      break;
    case "ndf":
      inst = shapeNdf({
        pair,
        tenor,
        contractRate: num(v.terms, "contract_rate"),
        notional: num(v.terms, "notional"),
        side: linearSide(str(v.terms, "side")),
        fixing: fixingToken(str(v.terms, "fixing")),
        settlementCcy: str(v.terms, "settlement_ccy"),
      });
      break;
    default:
      throw new Error(`family \`${v.family}\` is not exposed by Excel`);
  }

  // The vector's `expiry_years` is the authoritative pricing maturity (the corpus
  // uses fractional broken-date expiries, e.g. 91/365 for a "3M"); the display
  // tenor above is coarse, so pin the exact maturity the server prices against.
  return { ...inst, expiryYears: t };
}
