/**
 * The ONE composable Excel pricing surface: `CELNET.INSTRUMENT(underlier, product,
 * terms, [tenor], [notional])` — the spec token the polymorphic verbs
 * (`CELNET.PRICE` / `GREEKS` / `RFQ` / `SUBSCRIBE`) price.
 *
 * Three seams live here, all pure (no transport, no Office globals — exhaustively
 * unit-testable under node):
 *
 *  1. The UNDERLIER grammar — one string across all five asset classes:
 *       FX         "EURUSD" / "EUR/USD"
 *       metal      "XAUUSD" / "XAU/EUR" (a metal-X-code base leg vs a FIAT
 *                  quote — metal-vs-metal ratios are not priceable and rejected)
 *       equity     "AAPL@XNAS:USD"      (ticker@venue:ccy — venue PRESENT)
 *       commodity  "BRENT@:USD"         (ticker@:ccy — venue EMPTY)
 *       crypto     "BTC/USD" / "ETH-USDT" / "DOGEUSDT"
 *                  + an optional ":inverse" / ":linear" settlement suffix
 *                  (":inverse" = the coin-margined 1/S_T convention)
 *     A 3-letter/3-letter pair is FX unless its base is a metal X-code or a known
 *     liquid coin; any other coin selects the crypto arm via a non-3-letter leg
 *     ("FOO/USDT") or an explicit settlement suffix ("FOO/USD:linear").
 *
 *  2. The TERMS range — a named, order-free 2-column key/value range whose keys
 *     mirror the per-family parameters exactly (e.g. BARRIER: strike, callPut,
 *     barrier, kind, side, upperBarrier, rebate, monitoring, model). The two
 *     matrix keys (`legs` — BASKET's weighted underliers and STRATEGY's option
 *     ladder — and `correlations`, BASKET) repeat one row per matrix row with
 *     the payload in the cells to the right of the key. A missing/unknown
 *     key is a typed error NAMING the key and listing the family's key set.
 *
 *  3. The TOKEN — `CELNET.INSTRUMENT` returns the canonical compact JSON of the
 *     instrument's WS-mirror wire frame (`src/contract/instrumentCodec.ts`). It is
 *     an opaque VALUE, not an API: the one contract stays `celnet.proto`; the
 *     token round-trips it losslessly, so a token-priced cell emits the
 *     byte-identical frame the spec shaped.
 *
 * NO wire-building is duplicated: every family delegates to the SAME production
 * shapers (`shaping.ts`) the retired per-product functions used, with the
 * underlier's cross-asset arm overlaid exactly as the per-class vanilla shapers
 * overlay it.
 */

import type {
  CcyPair,
  Instrument,
  SettlementStyle,
  StrategyKind,
  Underlying,
} from "../contract/contract";
import {
  canonicalWireJson,
  instrumentFromWire,
  WireDecodeError,
} from "../contract/instrumentCodec";
import { instrumentToWire, type WireObject } from "../contract/wsCodec";
import {
  ShapingError,
  parsePair,
  parseStrategyKind,
  shapeAccumulator,
  shapeAmerican,
  shapeAsianOption,
  shapeBarrier,
  shapeBasket,
  shapeCliquet,
  shapeCommodityUnderlying,
  shapeCryptoUnderlying,
  shapeDigital,
  shapeEquityUnderlying,
  shapeForward,
  shapeForwardStart,
  shapeListedFutureOption,
  shapeLookback,
  shapeMetalUnderlying,
  shapeNdf,
  shapePerpetual,
  shapePivot,
  shapeQuanto,
  shapeSettlementStyle,
  shapeStrategy,
  shapeSwap,
  shapeTarf,
  shapeTouch,
  shapeVanillaInstrument,
  shapeVarianceSwap,
  shapeVolatilitySwap,
  shapeWindowBarrier,
  underlyingPairProjection,
} from "./shaping";

// ---------------------------------------------------------------------------
// underlier grammar
// ---------------------------------------------------------------------------

/** The parsed underlier: the (optional non-FX) arm, its FX projection, mechanics. */
export interface ParsedUnderlier {
  /** The cross-asset arm; ABSENT for FX (the frame stays byte-identical to FX). */
  readonly underlying?: Underlying;
  /** The FX `pair` leg-string projection the instrument carries. */
  readonly pair: CcyPair;
  /** LINEAR, or INVERSE_COIN from a crypto ":inverse" suffix. */
  readonly settlementStyle: SettlementStyle;
  /** A short display label, e.g. "EURUSD", "XAUUSD", "AAPL@XNAS:USD". */
  readonly label: string;
}

/** The four ISO metal X-codes that select the metal arm as a base leg. */
const METAL_BASE_CODES = new Set(["XAU", "XAG", "XPT", "XPD"]);

/**
 * The liquid digital-asset base tickers that select the crypto arm even in the
 * ambiguous 3-letter/3-letter form ("BTC/USD" would otherwise parse as FX). Any
 * coin outside this set is still fully expressible: use a non-3-letter leg
 * ("FOO/USDT") or an explicit settlement suffix ("FOO/USD:linear").
 */
const KNOWN_CRYPTO_BASES = new Set([
  "BTC",
  "ETH",
  "SOL",
  "XRP",
  "LTC",
  "BCH",
  "ADA",
  "DOT",
  "BNB",
  "TRX",
  "XLM",
]);

/**
 * Parse the one-string underlier grammar (see the module header). The settlement
 * suffix is meaningful ONLY for a digital-asset underlier (the proto carries
 * `settlement_style` for the coin-margined 1/S_T convention); on any other class
 * it is a typed error, never silently dropped.
 */
export function parseUnderlier(raw: string): ParsedUnderlier {
  const trimmed = raw.trim();
  if (trimmed.length === 0) {
    throw new ShapingError("underlier is required (e.g. EURUSD, XAUUSD, AAPL@XNAS:USD, BTC/USD)");
  }

  // 1. Peel the optional settlement suffix (":inverse" / ":linear" / ":coin").
  let body = trimmed;
  let suffixStyle: SettlementStyle | undefined;
  const suffix = /:(INVERSE_COIN|INVERSE|LINEAR|COIN)$/i.exec(trimmed);
  if (suffix !== null && suffix[1] !== undefined) {
    body = trimmed.slice(0, trimmed.length - suffix[0].length);
    suffixStyle = shapeSettlementStyle(suffix[1]);
  }

  // 2. Equity / commodity: TICKER@VENUE:CCY — a venue selects equity; an empty
  //    venue ("BRENT@:USD") selects commodity (the listed-vs-contract rule).
  if (body.includes("@")) {
    if (suffixStyle !== undefined) {
      throw new ShapingError(
        `settlement suffix \`:${suffixStyle.toLowerCase()}\` applies only to a crypto underlier`,
      );
    }
    const m = /^([A-Za-z0-9._-]+)@([A-Za-z0-9]*):([A-Za-z]{3})$/.exec(body);
    if (m === null || m[1] === undefined || m[2] === undefined || m[3] === undefined) {
      throw new ShapingError(
        `invalid underlier \`${raw}\` (expected TICKER@VENUE:CCY, e.g. AAPL@XNAS:USD, or TICKER@:CCY for a commodity)`,
      );
    }
    const ticker = m[1];
    const venue = m[2];
    const currency = m[3];
    const underlying =
      venue.length > 0
        ? shapeEquityUnderlying({ ticker, venue, currency })
        : shapeCommodityUnderlying({ symbol: ticker, currency });
    return {
      underlying,
      pair: underlyingPairProjection(underlying),
      settlementStyle: "LINEAR",
      label: body.toUpperCase(),
    };
  }

  // 3. Pair-shaped underliers: metal / crypto / FX.
  const upper = body.toUpperCase();
  const sep = /^([A-Z0-9]+)[-/_]([A-Z0-9]+)$/.exec(upper);
  const base = sep?.[1] ?? (upper.length === 6 && /^[A-Z]{6}$/.test(upper) ? upper.slice(0, 3) : undefined);
  const quote = sep?.[2] ?? (upper.length === 6 && /^[A-Z]{6}$/.test(upper) ? upper.slice(3, 6) : undefined);

  // 3a. Metal: a metal-X-code base leg vs a FIAT quote (XAUUSD, XAU/EUR).
  if (base !== undefined && quote !== undefined && METAL_BASE_CODES.has(base)) {
    if (suffixStyle !== undefined) {
      throw new ShapingError(
        `settlement suffix \`:${suffixStyle.toLowerCase()}\` applies only to a crypto underlier`,
      );
    }
    // HONESTY: metal-vs-METAL ratios (XAU/XAG, XPT/XPD, …) are not priceable —
    // the conventions registry and `MetalPair.quote` carry metal-vs-FIAT only
    // ("metal base, loco-London, premium in the fiat"). Reject with a typed
    // error rather than emit a wire shape the server can only refuse.
    if (METAL_BASE_CODES.has(quote)) {
      throw new ShapingError(
        `metal-quoted metal pair \`${base}/${quote}\` is not priceable — ` +
          `metals quote against a fiat currency (e.g. ${base}USD, ${base}/EUR)`,
      );
    }
    const underlying = shapeMetalUnderlying({ metal: base, quote });
    return {
      underlying,
      pair: underlyingPairProjection(underlying),
      settlementStyle: "LINEAR",
      label: `${base}${quote}`,
    };
  }

  // 3b. Crypto: an explicit settlement suffix, a known coin base, or a leg that
  //     does not fit the 3-letter ISO shape.
  const threeByThree =
    base !== undefined && quote !== undefined && /^[A-Z]{3}$/.test(base) && /^[A-Z]{3}$/.test(quote);
  const isCrypto =
    suffixStyle !== undefined ||
    (base !== undefined && KNOWN_CRYPTO_BASES.has(base)) ||
    (sep !== null && !threeByThree) ||
    (sep === null && !/^[A-Z]{6}$/.test(upper));
  if (isCrypto) {
    let underlying: Underlying;
    try {
      underlying = shapeCryptoUnderlying(upper);
    } catch {
      throw new ShapingError(
        `invalid underlier \`${raw}\` (expected FX "EURUSD", metal "XAUUSD", equity ` +
          `"AAPL@XNAS:USD", commodity "BRENT@:USD", or crypto "BTC/USD"[:inverse|:linear])`,
      );
    }
    return {
      underlying,
      pair: underlyingPairProjection(underlying),
      settlementStyle: suffixStyle ?? "LINEAR",
      label:
        underlying.kind === "digitalAsset"
          ? `${underlying.digitalAsset.base}/${underlying.digitalAsset.quote}`
          : upper,
    };
  }

  // 3c. FX — NO underlying arm: the frame stays byte-identical to the legacy
  //     FX-only contract (the `pair` projection IS the identity).
  const pair = parsePair(upper);
  return { pair, settlementStyle: "LINEAR", label: `${pair.base}${pair.quote}` };
}

// ---------------------------------------------------------------------------
// terms range — named, order-free key/value parsing
// ---------------------------------------------------------------------------

/** One cell of the terms range as Excel passes it. */
export type TermsCell = string | number | boolean | null | undefined;

/** Canonicalize a term key: case-/separator-insensitive ("upper_barrier" == "upperBarrier"). */
function canonKey(key: string): string {
  return key.trim().toLowerCase().replace(/[^a-z0-9]/g, "");
}

const isBlank = (c: TermsCell): boolean =>
  c === null || c === undefined || (typeof c === "string" && c.trim() === "");

/** The parsed terms: scalar key/values + the repeated matrix rows, consumption-tracked. */
class Terms {
  private readonly scalars = new Map<string, { key: string; value: string | number | boolean }>();
  private readonly matrices = new Map<string, (string | number)[][]>();
  private readonly consumed = new Set<string>();

  constructor(range: unknown) {
    if (!Array.isArray(range) || range.some((row) => !Array.isArray(row))) {
      throw new ShapingError(
        "terms must be a 2-column key/value range (one term per row, e.g. (\"strike\", 1.12))",
      );
    }
    for (const row of range as TermsCell[][]) {
      // Trim trailing blank cells; skip fully-blank rows (a ragged selection is fine).
      const cells = [...row];
      while (cells.length > 0 && isBlank(cells[cells.length - 1])) cells.pop();
      if (cells.length === 0) continue;
      const keyCell = cells[0];
      if (typeof keyCell !== "string" || keyCell.trim() === "") {
        throw new ShapingError(
          `terms row ${JSON.stringify(row)} has no key in its first cell (expected e.g. "strike")`,
        );
      }
      const key = keyCell.trim();
      const canon = canonKey(key);
      const payload = cells.slice(1);
      if (canon === "legs" || canon === "correlations") {
        // Matrix keys repeat: each row appends one matrix row from the payload cells.
        if (payload.length === 0 || payload.some(isBlank)) {
          throw new ShapingError(
            `term \`${key}\` expects its row cells to carry one full matrix row (no blanks)`,
          );
        }
        const rows = this.matrices.get(canon) ?? [];
        rows.push(payload.map((c) => (typeof c === "boolean" ? String(c) : (c as string | number))));
        this.matrices.set(canon, rows);
        continue;
      }
      if (payload.length === 0) {
        throw new ShapingError(`term \`${key}\` is missing its value (expected ("${key}", value))`);
      }
      if (payload.length > 1) {
        throw new ShapingError(`term \`${key}\` expects a single value cell, got ${payload.length}`);
      }
      if (this.scalars.has(canon)) {
        throw new ShapingError(`duplicate term \`${key}\``);
      }
      const value = payload[0];
      if (isBlank(value) || value === null || value === undefined) {
        throw new ShapingError(`term \`${key}\` is missing its value (expected ("${key}", value))`);
      }
      this.scalars.set(canon, { key, value });
    }
  }

  /** Read a raw scalar by any of its accepted key spellings, marking it consumed. */
  private take(keys: readonly string[]): string | number | boolean | undefined {
    for (const k of keys) {
      const canon = canonKey(k);
      this.consumed.add(canon);
      const hit = this.scalars.get(canon);
      if (hit !== undefined) return hit.value;
    }
    return undefined;
  }

  optStr(...keys: readonly string[]): string | undefined {
    const v = this.take(keys);
    if (v === undefined) return undefined;
    if (typeof v === "boolean") {
      throw new ShapingError(`term \`${keys[0]}\` expects text, got ${v}`);
    }
    return String(v);
  }

  reqStr(family: string, ...keys: readonly string[]): string {
    const v = this.optStr(...keys);
    if (v === undefined) {
      throw new ShapingError(`${family} requires the term \`${keys[0]}\``);
    }
    return v;
  }

  optNum(...keys: readonly string[]): number | undefined {
    const v = this.take(keys);
    if (v === undefined) return undefined;
    const n = typeof v === "number" ? v : Number(String(v).trim());
    if (typeof v === "boolean" || !Number.isFinite(n)) {
      throw new ShapingError(`term \`${keys[0]}\` expects a number, got \`${v}\``);
    }
    return n;
  }

  reqNum(family: string, ...keys: readonly string[]): number {
    const v = this.optNum(...keys);
    if (v === undefined) {
      throw new ShapingError(`${family} requires the numeric term \`${keys[0]}\``);
    }
    return v;
  }

  /** A strike-or-delta value: an absolute level (1.12) or a delta string ("25dP", "ATM"). */
  optStrike(...keys: readonly string[]): string | number | undefined {
    const v = this.take(keys);
    if (v === undefined) return undefined;
    if (typeof v === "boolean") {
      throw new ShapingError(`term \`${keys[0]}\` expects a strike or delta, got ${v}`);
    }
    return v;
  }

  reqStrike(family: string, ...keys: readonly string[]): string | number {
    const v = this.optStrike(...keys);
    if (v === undefined) {
      throw new ShapingError(`${family} requires the term \`${keys[0]}\` (a strike level or delta)`);
    }
    return v;
  }

  /** The accumulated rows of a matrix key (`legs` / `correlations`), or undefined. */
  matrixRows(key: string): (string | number)[][] | undefined {
    const canon = canonKey(key);
    this.consumed.add(canon);
    return this.matrices.get(canon);
  }

  /** After a family build: every supplied key must have been consumed. */
  assertAllConsumed(family: string, supported: readonly string[]): void {
    const unknown: string[] = [];
    for (const [canon, { key }] of this.scalars) {
      if (!this.consumed.has(canon)) unknown.push(key);
    }
    for (const canon of this.matrices.keys()) {
      if (!this.consumed.has(canon)) unknown.push(canon);
    }
    if (unknown.length > 0) {
      throw new ShapingError(
        `unknown term${unknown.length === 1 ? "" : "s"} \`${unknown.join("`, `")}\` for ${family} ` +
          `(expected: ${supported.join(", ")})`,
      );
    }
  }
}

// ---------------------------------------------------------------------------
// product families — each delegates to the SAME production shaper the retired
// per-product worksheet function used (no duplicated wire-building)
// ---------------------------------------------------------------------------

/** The shared per-build context: the underlier projection + tenor + notional. */
interface BuildCommon {
  readonly pair: CcyPair;
  readonly tenor: string;
  readonly notional: number;
}

interface FamilySpec {
  /** The per-family term keys, for the self-documenting unknown-key error. */
  readonly keys: readonly string[];
  /**
   * Set on the ONE expiryless family (PERPETUAL): it takes NO tenor — supplying
   * one (the tenor argument or a ("tenor", …) term) is a typed error, never a
   * silent drop. Its build never reads `BuildCommon.tenor`.
   */
  readonly tenorless?: boolean;
  readonly build: (t: Terms, c: BuildCommon) => Instrument;
}

/**
 * The multi-leg STRATEGY family build, shared by the generic family (the
 * template comes from a ("kind", …) term) and the four template-named families
 * (RISK_REVERSAL / STRADDLE / STRANGLE / SEAGULL — the template is bound by the
 * product name itself; a ("kind", …) term may restate it, but a CONTRADICTORY
 * kind is a typed error, never a silent override). The legs repeat as terms
 * rows exactly like BASKET's matrix keys: ("legs", callPut, strike, side,
 * ratio?) — one row per leg, delta strikes (`25dC`, `ATM`) resolved server-side.
 */
function strategyFamily(family: string, boundKind?: StrategyKind): FamilySpec {
  return {
    keys: ["kind", "legs"],
    build: (t, c) => {
      const kindTerm = t.optStr("kind");
      if (
        boundKind !== undefined &&
        kindTerm !== undefined &&
        parseStrategyKind(kindTerm) !== boundKind
      ) {
        throw new ShapingError(
          `${family} binds the strategy kind — the ("kind", ${kindTerm}) term contradicts it`,
        );
      }
      const kind = boundKind ?? kindTerm;
      if (kind === undefined) {
        throw new ShapingError(
          "STRATEGY requires the term `kind` (RISK_REVERSAL, STRADDLE, STRANGLE, SEAGULL) " +
            "— or name the template as the product",
        );
      }
      const legs = t.matrixRows("legs");
      if (legs === undefined) {
        throw new ShapingError(
          `${family} requires \`legs\` rows: ("legs", callPut, strike, side, ratio?) — one per leg`,
        );
      }
      return shapeStrategy({ ...c, kind, legs });
    },
  };
}

/**
 * The product-family table. Canonical names are the retired worksheet-function
 * names traders know; each family also answers to its proto product-arm name
 * (e.g. `single_barrier` → BARRIER) so the golden-corpus family tokens work
 * verbatim. `strike` / `strikeOrDelta` are interchangeable spellings.
 */
const FAMILIES: ReadonlyMap<string, FamilySpec> = new Map<string, FamilySpec>([
  [
    "VANILLA",
    {
      keys: ["strike", "callPut"],
      build: (t, c) =>
        shapeVanillaInstrument({
          ...c,
          strikeOrDelta: t.reqStrike("VANILLA", "strike", "strikeOrDelta"),
          callPut: t.reqStr("VANILLA", "callPut"),
        }),
    },
  ],
  // The multi-leg vol strategies (proto product field 8): the generic family +
  // the four template names, sharing ONE build (the template-name forms bind
  // their kind).
  ["STRATEGY", strategyFamily("STRATEGY")],
  ["RISKREVERSAL", strategyFamily("RISKREVERSAL", "RISK_REVERSAL")],
  ["STRADDLE", strategyFamily("STRADDLE", "STRADDLE")],
  ["STRANGLE", strategyFamily("STRANGLE", "STRANGLE")],
  ["SEAGULL", strategyFamily("SEAGULL", "SEAGULL")],
  [
    "BARRIER",
    {
      keys: [
        "strike",
        "callPut",
        "barrier",
        "kind",
        "side",
        "upperBarrier",
        "rebate",
        "monitoring",
        "model",
      ],
      build: (t, c) =>
        shapeBarrier({
          ...c,
          strikeOrDelta: t.reqStrike("BARRIER", "strike", "strikeOrDelta"),
          callPut: t.reqStr("BARRIER", "callPut"),
          barrier: t.reqNum("BARRIER", "barrier"),
          kind: t.optStr("kind"),
          side: t.optStr("side"),
          upperBarrier: t.optNum("upperBarrier"),
          rebate: t.optNum("rebate"),
          monitoring: t.optStr("monitoring"),
          model: t.optStr("model"),
        }),
    },
  ],
  [
    "WINDOWBARRIER",
    {
      keys: [
        "strike",
        "callPut",
        "barrier",
        "side",
        "windowStart",
        "windowEnd",
        "mcPairs",
        "mcSteps",
        "mcSeed",
      ],
      build: (t, c) =>
        shapeWindowBarrier({
          ...c,
          strikeOrDelta: t.reqStrike("WINDOWBARRIER", "strike", "strikeOrDelta"),
          callPut: t.reqStr("WINDOWBARRIER", "callPut"),
          barrier: t.reqNum("WINDOWBARRIER", "barrier"),
          side: t.optStr("side"),
          windowStart: t.optNum("windowStart"),
          windowEnd: t.optNum("windowEnd"),
          mcPairs: t.optNum("mcPairs"),
          mcSteps: t.optNum("mcSteps"),
          mcSeed: t.optNum("mcSeed"),
        }),
    },
  ],
  [
    "DIGITAL",
    {
      keys: ["strike", "callPut", "style", "payout"],
      build: (t, c) =>
        shapeDigital({
          ...c,
          strike: t.reqStrike("DIGITAL", "strike"),
          callPut: t.reqStr("DIGITAL", "callPut"),
          style: t.optStr("style"),
          payout: t.optNum("payout"),
        }),
    },
  ],
  [
    "TOUCH",
    {
      keys: ["kind", "barrier", "rebate", "upperBarrier", "monitoring"],
      build: (t, c) =>
        shapeTouch({
          ...c,
          kind: t.reqStr("TOUCH", "kind"),
          barrier: t.reqNum("TOUCH", "barrier"),
          rebate: t.optNum("rebate"),
          upperBarrier: t.optNum("upperBarrier"),
          monitoring: t.optStr("monitoring"),
        }),
    },
  ],
  [
    "VARSWAP",
    {
      keys: ["strikeVol"],
      build: (t, c) => shapeVarianceSwap({ ...c, strikeVol: t.optNum("strikeVol") }),
    },
  ],
  [
    "VOLSWAP",
    {
      keys: ["strikeVol"],
      build: (t, c) => shapeVolatilitySwap({ ...c, strikeVol: t.optNum("strikeVol") }),
    },
  ],
  [
    "ASIAN",
    {
      keys: [
        "strike",
        "callPut",
        "averaging",
        "observations",
        "method",
        "elapsedAvg",
        "elapsedWeight",
      ],
      build: (t, c) =>
        shapeAsianOption({
          ...c,
          strike: t.reqStrike("ASIAN", "strike"),
          callPut: t.reqStr("ASIAN", "callPut"),
          averaging: t.optStr("averaging"),
          observations: t.optNum("observations"),
          method: t.optStr("method"),
          elapsedAvg: t.optNum("elapsedAvg"),
          elapsedWeight: t.optNum("elapsedWeight"),
        }),
    },
  ],
  [
    "FORWARDSTART",
    {
      keys: ["callPut", "moneyness", "reset"],
      build: (t, c) =>
        shapeForwardStart({
          ...c,
          callPut: t.reqStr("FORWARDSTART", "callPut"),
          moneyness: t.reqNum("FORWARDSTART", "moneyness"),
          reset: t.reqNum("FORWARDSTART", "reset"),
        }),
    },
  ],
  [
    "CLIQUET",
    {
      keys: [
        "callPut",
        "moneyness",
        "periods",
        "localFloor",
        "localCap",
        "globalFloor",
        "globalCap",
        "mcPairs",
        "mcSeed",
      ],
      build: (t, c) =>
        shapeCliquet({
          ...c,
          callPut: t.reqStr("CLIQUET", "callPut"),
          moneyness: t.reqNum("CLIQUET", "moneyness"),
          periods: t.reqNum("CLIQUET", "periods"),
          localFloor: t.optNum("localFloor"),
          localCap: t.optNum("localCap"),
          globalFloor: t.optNum("globalFloor"),
          globalCap: t.optNum("globalCap"),
          mcPairs: t.optNum("mcPairs"),
          mcSeed: t.optNum("mcSeed"),
        }),
    },
  ],
  [
    "QUANTO",
    {
      keys: ["callPut", "strike", "conversionVol", "correlation", "payoff"],
      build: (t, c) =>
        shapeQuanto({
          ...c,
          callPut: t.reqStr("QUANTO", "callPut"),
          strike: t.reqStrike("QUANTO", "strike"),
          conversionVol: t.reqNum("QUANTO", "conversionVol"),
          correlation: t.reqNum("QUANTO", "correlation"),
          payoff: t.optStr("payoff"),
        }),
    },
  ],
  [
    "TARF",
    {
      keys: [
        "callPut",
        "strike",
        "target",
        "leverage",
        "fixings",
        "redemption",
        "fixingNotional",
        "mcPairs",
        "mcSeed",
      ],
      build: (t, c) =>
        shapeTarf({
          ...c,
          callPut: t.reqStr("TARF", "callPut"),
          strike: t.reqStrike("TARF", "strike"),
          target: t.reqNum("TARF", "target"),
          leverage: t.reqNum("TARF", "leverage"),
          fixings: t.reqNum("TARF", "fixings"),
          redemption: t.optStr("redemption"),
          fixingNotional: t.optNum("fixingNotional"),
          mcPairs: t.optNum("mcPairs"),
          mcSeed: t.optNum("mcSeed"),
        }),
    },
  ],
  [
    "PIVOT",
    {
      keys: [
        "callPut",
        "strike",
        "pivot",
        "target",
        "leverage",
        "fixings",
        "redemption",
        "fixingNotional",
        "mcPairs",
        "mcSeed",
      ],
      build: (t, c) =>
        shapePivot({
          ...c,
          callPut: t.reqStr("PIVOT", "callPut"),
          strike: t.reqStrike("PIVOT", "strike"),
          pivot: t.reqNum("PIVOT", "pivot"),
          target: t.reqNum("PIVOT", "target"),
          leverage: t.reqNum("PIVOT", "leverage"),
          fixings: t.reqNum("PIVOT", "fixings"),
          redemption: t.optStr("redemption"),
          fixingNotional: t.optNum("fixingNotional"),
          mcPairs: t.optNum("mcPairs"),
          mcSeed: t.optNum("mcSeed"),
        }),
    },
  ],
  [
    "ACCUMULATOR",
    {
      keys: [
        "pivot",
        "barrier",
        "leverage",
        "fixings",
        "monitoring",
        "fixingNotional",
        "mcPairs",
        "mcSeed",
      ],
      build: (t, c) =>
        shapeAccumulator({
          ...c,
          pivot: t.reqStrike("ACCUMULATOR", "pivot"),
          barrier: t.reqNum("ACCUMULATOR", "barrier"),
          leverage: t.reqNum("ACCUMULATOR", "leverage"),
          fixings: t.reqNum("ACCUMULATOR", "fixings"),
          monitoring: t.optStr("monitoring"),
          fixingNotional: t.optNum("fixingNotional"),
          mcPairs: t.optNum("mcPairs"),
          mcSeed: t.optNum("mcSeed"),
        }),
    },
  ],
  [
    "LOOKBACK",
    {
      keys: ["callPut", "style", "monitoring", "strike", "observations", "mcPairs", "mcSeed"],
      build: (t, c) =>
        shapeLookback({
          ...c,
          callPut: t.reqStr("LOOKBACK", "callPut"),
          style: t.optStr("style"),
          monitoring: t.optStr("monitoring"),
          strike: t.optStrike("strike"),
          observations: t.optNum("observations"),
          mcPairs: t.optNum("mcPairs"),
          mcSeed: t.optNum("mcSeed"),
        }),
    },
  ],
  [
    "AMERICAN",
    {
      keys: [
        "strike",
        "callPut",
        "style",
        "bermudanSteps",
        "lsmPaths",
        "lsmExerciseDates",
        "lsmSeed",
      ],
      build: (t, c) =>
        shapeAmerican({
          ...c,
          strike: t.reqStrike("AMERICAN", "strike"),
          callPut: t.reqStr("AMERICAN", "callPut"),
          style: t.optStr("style"),
          bermudanSteps: t.optNum("bermudanSteps"),
          lsmPaths: t.optNum("lsmPaths"),
          lsmExerciseDates: t.optNum("lsmExerciseDates"),
          lsmSeed: t.optNum("lsmSeed"),
        }),
    },
  ],
  [
    "BASKET",
    {
      keys: [
        "callPut",
        "strike",
        "legs",
        "correlations",
        "kind",
        "mcPaths",
        "mcReplications",
        "mcSteps",
        "mcSeed",
      ],
      build: (t, c) => {
        const legs = t.matrixRows("legs");
        if (legs === undefined) {
          throw new ShapingError(
            "BASKET requires `legs` rows: (\"legs\", pair, weight, spot, vol, rFor) — one per leg",
          );
        }
        const corrRows = t.matrixRows("correlations");
        if (corrRows === undefined) {
          throw new ShapingError(
            "BASKET requires `correlations` rows: (\"correlations\", ρ₁, ρ₂, …) — one per matrix row",
          );
        }
        const correlations = corrRows.map((row, i) =>
          row.map((cell, j) => {
            const n = typeof cell === "number" ? cell : Number(String(cell).trim());
            if (!Number.isFinite(n)) {
              throw new ShapingError(`correlation entry (${i + 1},${j + 1}) is not a number`);
            }
            return n;
          }),
        );
        return shapeBasket({
          ...c,
          callPut: t.reqStr("BASKET", "callPut"),
          strike: t.reqNum("BASKET", "strike"),
          kind: t.optStr("kind"),
          legs,
          correlations,
          mcPaths: t.optNum("mcPaths"),
          mcReplications: t.optNum("mcReplications"),
          mcSteps: t.optNum("mcSteps"),
          mcSeed: t.optNum("mcSeed"),
        });
      },
    },
  ],
  [
    "FORWARD",
    {
      keys: ["rate", "side"],
      build: (t, c) =>
        shapeForward({
          ...c,
          contractRate: t.reqNum("FORWARD", "rate", "contractRate"),
          side: t.optStr("side"),
        }),
    },
  ],
  [
    "SWAP",
    {
      keys: ["rate", "nearSide"],
      build: (t, c) =>
        shapeSwap({
          ...c,
          contractRate: t.reqNum("SWAP", "rate", "contractRate"),
          side: t.optStr("nearSide", "side"),
        }),
    },
  ],
  [
    "NDF",
    {
      keys: ["rate", "fixing", "settlementCcy", "side"],
      build: (t, c) =>
        shapeNdf({
          ...c,
          contractRate: t.reqNum("NDF", "rate", "contractRate"),
          fixing: t.reqStr("NDF", "fixing"),
          settlementCcy: t.optStr("settlementCcy"),
          side: t.optStr("side"),
        }),
    },
  ],
  [
    "PERPETUAL",
    {
      keys: ["strike", "callPut"],
      // The one expiryless product (proto arm 30): no tenor exists for it — the
      // wire shape is `expiry_years = 0` exactly with NO tenor key, and a
      // supplied tenor is a typed error (see `shapeSpecInstrument`).
      tenorless: true,
      build: (t, c) =>
        shapePerpetual({
          pair: c.pair,
          notional: c.notional,
          strike: t.reqNum("PERPETUAL", "strike"),
          callPut: t.reqStr("PERPETUAL", "callPut"),
        }),
    },
  ],
  [
    "FUTUREOPTION",
    {
      keys: ["strike", "callPut", "futureSymbol", "futureExpiry", "margining"],
      build: (t, c) =>
        shapeListedFutureOption({
          ...c,
          strike: t.reqNum("FUTUREOPTION", "strike"),
          callPut: t.reqStr("FUTUREOPTION", "callPut"),
          futureSymbol: t.reqStr("FUTUREOPTION", "futureSymbol"),
          futureExpiry: t.reqNum("FUTUREOPTION", "futureExpiry", "futureExpiryYears"),
          margining: t.optStr("margining"),
        }),
    },
  ],
]);

/** Proto product-arm aliases → the canonical trader-facing family name. */
const FAMILY_ALIASES: ReadonlyMap<string, string> = new Map([
  ["SINGLEBARRIER", "BARRIER"],
  ["DOUBLEBARRIER", "BARRIER"],
  ["VARIANCESWAP", "VARSWAP"],
  ["VOLATILITYSWAP", "VOLSWAP"],
  ["ASIANOPTION", "ASIAN"],
  ["FXFORWARD", "FORWARD"],
  ["FXSWAP", "SWAP"],
  ["PERPETUALOPTION", "PERPETUAL"],
  ["LISTEDFUTUREOPTION", "FUTUREOPTION"],
]);

/** The canonical family names, for the unknown-family error message. */
export const FAMILY_NAMES: readonly string[] = [...FAMILIES.keys()];

/** Resolve a product-family token (canonical / proto-arm / case-insensitive). */
function familyOf(product: string): { name: string; spec: FamilySpec } {
  const canon = product.trim().toUpperCase().replace(/[^A-Z0-9]/g, "");
  const name = FAMILIES.has(canon) ? canon : FAMILY_ALIASES.get(canon);
  const spec = name !== undefined ? FAMILIES.get(name) : undefined;
  if (name === undefined || spec === undefined) {
    throw new ShapingError(
      `unknown product family \`${product}\` (expected one of: ${FAMILY_NAMES.join(", ")})`,
    );
  }
  return { name, spec };
}

// ---------------------------------------------------------------------------
// the spec → Instrument → token assembly
// ---------------------------------------------------------------------------

/** The raw `CELNET.INSTRUMENT` cell arguments. */
export interface InstrumentSpecArgs {
  readonly underlier: string;
  readonly product: string;
  /** The 2-column key/value terms range (rows may extend for `legs`/`correlations`). */
  readonly terms: unknown;
  /** The tenor (e.g. "1Y"); may instead be supplied as a ("tenor", …) term.
   *  The no-expiry PERPETUAL family takes none (a supplied tenor is a typed error). */
  readonly tenor?: string | undefined;
  /** The notional (base/asset leg); a ("notional", …) term also works; default 1. */
  readonly notional?: number | undefined;
}

/**
 * Shape the full polymorphic spec into the typed `Instrument`: parse the
 * underlier, resolve tenor/notional (positional argument or term — not both),
 * build the product through its family's production shaper, then overlay the
 * cross-asset arm exactly as the per-class vanilla shapers do (the FX projection
 * carries the surfaces; LINEAR settlement is presence-omitted so an FX/linear
 * frame is byte-identical to the legacy per-product frame).
 */
export function shapeSpecInstrument(args: InstrumentSpecArgs): Instrument {
  const u = parseUnderlier(args.underlier);
  const terms = new Terms(args.terms);
  const { name, spec } = familyOf(args.product);

  const tenorTerm = terms.optStr("tenor");
  if (args.tenor !== undefined && args.tenor.trim() !== "" && tenorTerm !== undefined) {
    throw new ShapingError("tenor supplied twice (both the tenor argument and a (\"tenor\", …) term)");
  }
  const tenor = args.tenor !== undefined && args.tenor.trim() !== "" ? args.tenor : tenorTerm;
  if (spec.tenorless) {
    // The one expiryless family: a tenor is contradictory, so it is a typed
    // error — never silently dropped (mirrors the server's exact
    // `expiry_years == 0` perpetual guard).
    if (tenor !== undefined) {
      throw new ShapingError(
        `${name} is the no-expiry (perpetual) family — it takes no tenor ` +
          "(omit the tenor argument and any (\"tenor\", …) term)",
      );
    }
  } else if (tenor === undefined) {
    throw new ShapingError("a tenor is required (the tenor argument or a (\"tenor\", …) term, e.g. 1Y)");
  }

  const notionalTerm = terms.optNum("notional");
  if (args.notional !== undefined && notionalTerm !== undefined) {
    throw new ShapingError(
      "notional supplied twice (both the notional argument and a (\"notional\", …) term)",
    );
  }
  const notional = args.notional ?? notionalTerm ?? 1;

  // A tenorless family's build never reads `tenor` (see `FamilySpec.tenorless`).
  const instrument = spec.build(terms, { pair: u.pair, tenor: tenor ?? "", notional });
  terms.assertAllConsumed(name, [
    ...spec.keys,
    ...(spec.tenorless ? [] : ["tenor"]),
    "notional",
  ]);

  if (u.underlying === undefined) {
    // FX: no overlay — the frame is byte-identical to the legacy per-product one.
    if (u.settlementStyle !== "LINEAR") {
      throw new ShapingError("settlement style applies only to a crypto underlier");
    }
    return instrument;
  }
  const overlaid: Instrument = { ...instrument, underlying: u.underlying };
  // LINEAR is the proto3 zero value — presence-omit it (mirrors shapeCrossAssetVanilla).
  if (u.settlementStyle !== "LINEAR") overlaid.settlementStyle = u.settlementStyle;
  return overlaid;
}

// ---------------------------------------------------------------------------
// the opaque instrument token
// ---------------------------------------------------------------------------

/**
 * Encode an instrument as the opaque spec token: the canonical compact JSON of
 * its WS-mirror wire frame. Deterministic (key-sorted), versionless, and exactly
 * the frame a token-priced cell emits.
 */
export function encodeInstrumentToken(instrument: Instrument): string {
  return canonicalWireJson(instrumentToWire(instrument));
}

/**
 * First-argument detection for the polymorphic verbs: an instrument token is a
 * JSON object literal; a legacy positional first argument ("EURUSD") never is.
 */
export function isInstrumentToken(value: unknown): value is string {
  return typeof value === "string" && value.trimStart().startsWith("{");
}

/** Decode a spec token back to the typed `Instrument`; a malformed token is a typed error. */
export function decodeInstrumentToken(token: string): Instrument {
  let parsed: unknown;
  try {
    parsed = JSON.parse(token);
  } catch {
    throw new ShapingError("invalid instrument token (not JSON — pass the CELNET.INSTRUMENT cell)");
  }
  if (parsed === null || typeof parsed !== "object" || Array.isArray(parsed)) {
    throw new ShapingError("invalid instrument token (expected the CELNET.INSTRUMENT object form)");
  }
  try {
    return instrumentFromWire(parsed as WireObject);
  } catch (err) {
    if (err instanceof WireDecodeError) {
      throw new ShapingError(`invalid instrument token: ${err.message}`);
    }
    throw err;
  }
}

/** A short trader-facing label for a decoded instrument (streaming cells). */
export function instrumentLabel(instrument: Instrument): string {
  const pair = `${instrument.pair.base}${instrument.pair.quote}`;
  // A perpetual has NO expiry (`expiryYears` is the wire's exact 0): label it
  // honestly as perpetual rather than as a zero-year maturity.
  const maturity =
    instrument.product.kind === "perpetualOption"
      ? "perp"
      : `${instrument.expiryYears.toPrecision(3)}y`;
  return `${pair} ${instrument.product.kind} ${maturity}`;
}
