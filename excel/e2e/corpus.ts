/**
 * Load the FROZEN golden corpus (`crates/celnet-golden/vectors/*.json`) and build,
 * for every family the Excel add-in exposes, the EXACT `Instrument` the
 * POLYMORPHIC surface shapes — `CELNET.INSTRUMENT(underlier, product, terms)`
 * (the production `shapeSpecInstrument` + the opaque-token codec, the same code
 * the worksheet function runs), round-tripped through the token exactly as a
 * `CELNET.PRICE(token)` cell would. The conformance spec then prices each
 * instrument through the REAL add-in `Connection` over a REAL WebSocket to a REAL
 * booted edge, asserting the server's price equals the vector's independent
 * oracle within the vector's frozen tolerance (a `k·stderr` band for the
 * Monte-Carlo families).
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
  decodeInstrumentToken,
  encodeInstrumentToken,
  shapeSpecInstrument,
  type InstrumentSpecArgs,
} from "../src/functions/instrumentSpec";

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

/** The 24 product-oneof family names that may appear in the corpus. */
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
  // Cross-asset vanilla underlyings (W1 `Underlying` oneof) — NOT priced by the
  // Excel WS corpus path (declared in `FAMILIES_NOT_EXPOSED`). The gap is NOT the
  // market: their vectors' `{spot, vol, r_dom, r_for}` IS the FX two-rate
  // projection (`r_dom` = discount rate, `r_for` = the carry yield — dividend+repo
  // / convenience / funding) the WS `MarketContext` transports and the server's
  // carry guard reads as `b = r_dom − r_for`, proven end-to-end by
  // `crates/celnet-server/tests/cross_asset_ws.rs`. The REAL gap is the instrument
  // seam: the add-in's `instrumentToWire` always emits the legacy FX `pair`
  // projection BESIDE `underlying` (keeping the FX-keyed surfaces total), and the
  // server's WS decoder gives `pair` precedence (`instrument_underlying_from_json`)
  // — so a client-emitted cross-asset frame routes down the FX path. That is
  // numerically invisible for the linear payoffs (a green row would prove the FX
  // path, not the cross-asset decode) and WRONG for the INVERSE_COIN crypto
  // vectors (`settlement_style` is ignored: the LINEAR USD value, not the
  // coin-margined `1/S_T` oracle) — pinned server-side by
  // `ws_legacy_pair_precedence_routes_client_shaped_frames_to_the_fx_path`. The
  // polymorphic `CELNET.INSTRUMENT` underlier grammar DOES shape these onto the
  // wire (gated by `crossAssetProducts.test.ts` and the polymorphic parity suite);
  // they move into `EXCEL_FAMILIES` the moment the codec seam prefers `underlying`
  // over the `pair` projection (the server pin fails loudly when it does).
  "equity_option",
  "commodity_option",
  "crypto_option",
] as const;

/**
 * The families the polymorphic Excel surface prices over the FX WS path: every
 * one is reachable as `CELNET.INSTRUMENT(underlier, family, terms)` + a verb
 * (`CELNET.PRICE`/`GREEKS`/`RFQ`/`SUBSCRIBE`). The multi-leg `strategy` family
 * is expressed with repeated ("legs", callPut, strike, side, ratio) terms rows
 * (exactly like BASKET's matrix keys), so every WS-priceable corpus family is
 * exposed; the remaining `FAMILIES_NOT_EXPOSED` are the three cross-asset
 * vanilla arms (`equity_option`/`commodity_option`/`crypto_option`), blocked by
 * the server WS decoder's legacy-`pair` precedence over the `underlying` object
 * the add-in emits beside it — see the rationale on `ALL_FAMILIES` above.
 */
export const EXCEL_FAMILIES = [
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
] as const;

/** Families in the corpus that the Excel price path does not (yet) expose. */
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

/** The corpus directional side (`BUY`/`SELL`) the linear shapers and strategy legs parse. */
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
 * Map a listed-future vector's underlying class token to the Excel underlier
 * grammar — the same WTI=NYMEX-crude(commodity) / ES=CME-E-mini-S&P-500(equity)
 * classes the Rust SDK conformance maps (`listed_future_underlying`). The Excel
 * grammar's commodity form carries no venue (TICKER@:CCY — the venue lives on
 * the option's `future_symbol`, the contract identity the arm prices), so the
 * commodity underlying is the venue-less projection of the SDK's; the listed-
 * future arm is asset-class-agnostic (the quoted futures price embodies the
 * carry), so the priced value is identical.
 */
const listedFutureUnderlier = (token: string): string => {
  switch (token) {
    case "WTI":
      return "WTI@:USD";
    case "ES":
      return "ES@XCME:USD";
    default:
      throw new Error(`unknown listed-future underlying \`${token}\``);
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

/** One 2-column terms row as a `CELNET.INSTRUMENT` cell would receive it. */
type TermsRow = (string | number)[];

/**
 * Build the EXACT `CELNET.INSTRUMENT(underlier, product, terms, tenor, notional)`
 * cell arguments for a vector: the vector's product-arm family name is passed
 * VERBATIM as the product (the family table answers to the proto arm names), and
 * the vector's terms become the named 2-column key/value rows a trader would type.
 * Monte-Carlo families are sent with the server-default budget (`mcPairs = 0`) —
 * exactly as the SDK conformance does — and the `k·(oracle_se + server_se)` band
 * governs; `window_barrier` carries its explicit MC budget from the corpus (its
 * only engine is MC).
 */
export function specOf(v: GoldenVector): InstrumentSpecArgs {
  const t = num(v.terms, "expiry_years");
  // The display tenor; cleared for the one TENORLESS family (perpetual), whose
  // spec takes no tenor at all (a supplied one is a typed error).
  let tenor: string | undefined = tenorFor(t);
  let underlier = v.underlying;
  const rows: TermsRow[] = [];
  let notional = 1.0;

  switch (v.family) {
    case "vanilla":
      rows.push(["strike", num(v.terms, "strike")], ["callPut", cp(str(v.terms, "option_type"))]);
      break;
    case "strategy": {
      // The template kind + one ("legs", callPut, strike, side, ratio) row per
      // leg — the same repeated-matrix-row terms grammar BASKET uses.
      rows.push(["kind", str(v.terms, "kind")]);
      const legsRaw = v.terms["legs"] as Array<Record<string, unknown>>;
      for (const leg of legsRaw) {
        rows.push([
          "legs",
          cp(String(leg["option_type"])),
          Number(leg["strike"]),
          linearSide(String(leg["side"])),
          Number(leg["ratio"]),
        ]);
      }
      break;
    }
    case "single_barrier":
      rows.push(
        ["strike", num(v.terms, "strike")],
        ["callPut", cp(str(v.terms, "option_type"))],
        ["barrier", num(v.terms, "barrier")],
        ["kind", str(v.terms, "kind")],
        ["side", barrierSide(str(v.terms, "side"))],
        ["rebate", num(v.terms, "rebate")],
        ["monitoring", str(v.terms, "monitoring")],
      );
      break;
    case "double_barrier":
      rows.push(
        ["strike", num(v.terms, "strike")],
        ["callPut", cp(str(v.terms, "option_type"))],
        ["barrier", num(v.terms, "lower_barrier")],
        ["upperBarrier", num(v.terms, "upper_barrier")],
        ["kind", str(v.terms, "kind")],
        ["rebate", num(v.terms, "rebate")],
        ["monitoring", str(v.terms, "monitoring")],
      );
      break;
    case "digital":
      rows.push(
        ["strike", num(v.terms, "strike")],
        ["callPut", cp(str(v.terms, "option_type"))],
        ["style", str(v.terms, "style")],
        ["payout", num(v.terms, "payout")],
      );
      break;
    case "touch": {
      const kind = str(v.terms, "kind");
      const isDouble = kind === "DOUBLE_NO_TOUCH" || kind === "DOUBLE_ONE_TOUCH";
      rows.push(["kind", kind], ["barrier", num(v.terms, "lower_barrier")]);
      if (isDouble) rows.push(["upperBarrier", num(v.terms, "upper_barrier")]);
      rows.push(["rebate", num(v.terms, "rebate")], ["monitoring", str(v.terms, "monitoring")]);
      break;
    }
    case "variance_swap":
    case "volatility_swap":
      rows.push(["strikeVol", num(v.terms, "strike_vol")]);
      break;
    case "asian_option":
      rows.push(
        ["strike", num(v.terms, "strike")],
        ["callPut", cp(str(v.terms, "option_type"))],
        ["averaging", str(v.terms, "averaging")],
        ["observations", num(v.terms, "observations")],
        ["method", str(v.terms, "method")],
      );
      break;
    case "forward_start":
      rows.push(
        ["callPut", cp(str(v.terms, "option_type"))],
        ["moneyness", num(v.terms, "moneyness")],
        ["reset", num(v.terms, "reset")],
      );
      break;
    case "cliquet": {
      rows.push(
        ["callPut", cp(str(v.terms, "option_type"))],
        ["moneyness", num(v.terms, "moneyness")],
        ["periods", num(v.terms, "periods")],
      );
      const lf = optNum(v.terms, "local_floor");
      const lc = optNum(v.terms, "local_cap");
      const gf = optNum(v.terms, "global_floor");
      const gc = optNum(v.terms, "global_cap");
      if (lf !== undefined) rows.push(["localFloor", lf]);
      if (lc !== undefined) rows.push(["localCap", lc]);
      if (gf !== undefined) rows.push(["globalFloor", gf]);
      if (gc !== undefined) rows.push(["globalCap", gc]);
      break;
    }
    case "quanto":
      rows.push(
        ["callPut", cp(str(v.terms, "option_type"))],
        ["strike", num(v.terms, "strike")],
        ["conversionVol", num(v.terms, "conversion_vol")],
        ["correlation", num(v.terms, "correlation")],
        ["payoff", str(v.terms, "payoff")],
      );
      break;
    case "tarf":
      rows.push(
        ["callPut", cp(str(v.terms, "option_type"))],
        ["strike", num(v.terms, "strike")],
        ["target", num(v.terms, "target")],
        ["leverage", num(v.terms, "leverage")],
        ["fixings", num(v.terms, "fixings")],
        ["redemption", str(v.terms, "redemption")],
        ["fixingNotional", num(v.terms, "fixing_notional")],
      );
      break;
    case "pivot":
      rows.push(
        ["callPut", cp(str(v.terms, "option_type"))],
        ["strike", num(v.terms, "strike")],
        ["pivot", num(v.terms, "pivot")],
        ["target", num(v.terms, "target")],
        ["leverage", num(v.terms, "leverage")],
        ["fixings", num(v.terms, "fixings")],
        ["redemption", str(v.terms, "redemption")],
        ["fixingNotional", num(v.terms, "fixing_notional")],
      );
      break;
    case "accumulator":
      rows.push(
        ["pivot", num(v.terms, "pivot")],
        ["barrier", num(v.terms, "barrier")],
        ["leverage", num(v.terms, "leverage")],
        ["fixings", num(v.terms, "fixings")],
        ["monitoring", str(v.terms, "monitoring")],
        ["fixingNotional", num(v.terms, "fixing_notional")],
      );
      break;
    case "lookback": {
      const style = str(v.terms, "style");
      rows.push(["callPut", cp(str(v.terms, "option_type"))], ["style", style]);
      rows.push(["monitoring", str(v.terms, "monitoring")]);
      if (style === "FIXED") rows.push(["strike", num(v.terms, "strike")]);
      const obs = optNum(v.terms, "observations");
      if (obs !== undefined) rows.push(["observations", obs]);
      break;
    }
    case "window_barrier":
      // The window barrier's ONLY engine is Monte-Carlo (no closed form); the corpus
      // encodes its exact MC budget/seed and the SDK conformance passes them through.
      rows.push(
        ["strike", num(v.terms, "strike")],
        ["callPut", cp(str(v.terms, "option_type"))],
        ["barrier", num(v.terms, "barrier")],
        ["side", barrierSide(str(v.terms, "side"))],
        ["windowStart", num(v.terms, "window_start")],
        ["windowEnd", num(v.terms, "window_end")],
        ["mcPairs", num(v.terms, "mc_pairs")],
        ["mcSteps", num(v.terms, "mc_steps")],
        ["mcSeed", num(v.terms, "mc_seed")],
      );
      break;
    case "american":
      rows.push(["strike", num(v.terms, "strike")], ["callPut", cp(str(v.terms, "option_type"))]);
      break;
    case "basket": {
      rows.push(
        ["callPut", cp(str(v.terms, "option_type"))],
        ["strike", num(v.terms, "strike")],
        ["kind", str(v.terms, "kind")],
      );
      const legsRaw = v.terms["legs"] as Array<Record<string, unknown>>;
      for (const leg of legsRaw) {
        rows.push([
          "legs",
          String(leg["pair"]),
          Number(leg["weight"]),
          Number(leg["spot"]),
          Number(leg["vol"]),
          Number(leg["r_for"]),
        ]);
      }
      const corrFlat = (v.terms["correlations"] as number[]).map(Number);
      const n = legsRaw.length;
      for (let i = 0; i < n; i++) {
        rows.push(["correlations", ...corrFlat.slice(i * n, i * n + n)]);
      }
      break;
    }
    case "fx_forward":
      notional = num(v.terms, "notional");
      rows.push(["rate", num(v.terms, "contract_rate")], ["side", linearSide(str(v.terms, "side"))]);
      break;
    case "fx_swap":
      notional = num(v.terms, "notional");
      rows.push(
        ["rate", num(v.terms, "contract_rate")],
        ["nearSide", linearSide(str(v.terms, "near_side"))],
      );
      break;
    case "ndf":
      notional = num(v.terms, "notional");
      rows.push(
        ["rate", num(v.terms, "contract_rate")],
        ["fixing", fixingToken(str(v.terms, "fixing"))],
        ["settlementCcy", str(v.terms, "settlement_ccy")],
        ["side", linearSide(str(v.terms, "side"))],
      );
      break;
    case "perpetual_option":
      // The one tenorless, expiryless product (proto arm 30): the spec takes NO
      // tenor (the shaper encodes the contract's canonical `expiry_years = 0`),
      // so the vector's `t = 0` / "PERP" label never enter.
      tenor = undefined;
      notional = num(v.terms, "notional");
      rows.push(["strike", num(v.terms, "strike")], ["callPut", cp(str(v.terms, "option_type"))]);
      break;
    case "listed_future_option": {
      underlier = listedFutureUnderlier(v.underlying);
      notional = num(v.terms, "notional");
      const sym = v.terms["future_symbol"] as Record<string, unknown>;
      const ticker = String(sym["ticker"]);
      const venue = typeof sym["venue"] === "string" ? sym["venue"] : "";
      rows.push(
        ["strike", num(v.terms, "strike")],
        ["callPut", cp(str(v.terms, "option_type"))],
        ["futureSymbol", venue.length > 0 ? `${ticker}@${venue}` : ticker],
        ["futureExpiry", num(v.terms, "future_expiry_years")],
        ["margining", str(v.terms, "margining")],
      );
      break;
    }
    default:
      throw new Error(`family \`${v.family}\` is not exposed by Excel`);
  }

  return tenor === undefined
    ? { underlier, product: v.family, terms: rows, notional }
    : { underlier, product: v.family, terms: rows, tenor, notional };
}

/**
 * Build the EXACT `Instrument` the polymorphic Excel surface prices for this
 * vector: shape the spec (the same `shapeSpecInstrument` the `CELNET.INSTRUMENT`
 * cell runs), then round-trip it through the opaque token codec exactly as a
 * `CELNET.PRICE(token)` cell would — so the e2e gate covers the full
 * spec → token → wire path, not just the shaper. The shaper derives `expiryYears`
 * from the coarse display tenor; we then override it with the vector's exact
 * `expiry_years` (the authoritative pricing maturity on the wire).
 */
export function instrumentOf(v: GoldenVector): Instrument {
  const inst = decodeInstrumentToken(encodeInstrumentToken(shapeSpecInstrument(specOf(v))));
  // The vector's `expiry_years` is the authoritative pricing maturity (the corpus
  // uses fractional broken-date expiries, e.g. 91/365 for a "3M"); the display
  // tenor above is coarse, so pin the exact maturity the server prices against.
  return { ...inst, expiryYears: num(v.terms, "expiry_years") };
}
