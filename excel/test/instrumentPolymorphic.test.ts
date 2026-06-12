/**
 * The polymorphic-surface PARITY PROOF + grammar gate — the evidence behind
 * retiring the per-product CELNET.* function table (CLAUDE.md #10: zero legacy,
 * retired ONLY at proven parity).
 *
 * 1. PARITY, corpus-wide: for EVERY frozen golden vector (every product family
 *    the retired functions exposed), the `CELNET.INSTRUMENT` spec path
 *    (`specOf(v)` → `shapeSpecInstrument`) emits the BYTE-IDENTICAL wire frame
 *    the retired per-product function emitted — asserted as deep key-for-key
 *    equality of the encoded frames AND as equality of their canonical
 *    (key-sorted, compact) JSON byte strings. The legacy reference here is a
 *    verbatim copy of the retired per-family dispatch (the same production
 *    shapers, called exactly as `CELNET.BARRIER`/`CELNET.TARF`/… called them).
 *
 * 2. PARITY, optionals: a representative case per retired function exercising
 *    the optional parameters the corpus does not (LSV model, Asian seasoning,
 *    clamped-cliquet MC knobs, Bermudan/LSM American, discrete lookback, MC
 *    budgets, defaults-omitted forms).
 *
 * 3. PARITY, cross-asset: the underlier grammar reproduces the per-class vanilla
 *    shapers (`shapeEquityVanilla` / `shapeCommodityVanilla` / `shapeCryptoVanilla`
 *    / `shapeMetalVanilla`) byte-for-byte, and overlays non-vanilla families onto
 *    non-FX underliers exactly as those shapers overlay vanillas.
 *
 * 4. The TOKEN is deterministic (terms order-free) and LOSSLESS:
 *    `decode(encode(i))` re-encodes to the same byte string, so a
 *    `CELNET.PRICE(token)` cell emits exactly the frame the spec shaped.
 *
 * 5. The grammar/terms layer rejects bad input with typed errors NAMING the
 *    missing/unknown key — never a silent default, never a wrong number.
 */
import { describe, expect, it } from "vitest";

import type {
  Instrument,
  Margining,
  OptionType,
  Side,
  StrategyKind,
  Tenor,
  Underlying,
} from "../src/contract/contract";
import { canonicalWireJson } from "../src/contract/instrumentCodec";
import { instrumentToWire } from "../src/contract/wsCodec";
import {
  decodeInstrumentToken,
  encodeInstrumentToken,
  FAMILY_NAMES,
  isInstrumentToken,
  parseUnderlier,
  shapeSpecInstrument,
} from "../src/functions/instrumentSpec";
import {
  DEFAULT_CONVENTIONS,
  ShapingError,
  shapeAccumulator,
  shapeAmerican,
  shapeAsianOption,
  shapeBarrier,
  shapeBasket,
  shapeCliquet,
  shapeCommodityVanilla,
  shapeCryptoVanilla,
  shapeDigital,
  shapeEquityVanilla,
  shapeForward,
  shapeForwardStart,
  shapeLookback,
  shapeMetalVanilla,
  shapeNdf,
  shapePivot,
  shapeQuanto,
  shapeSwap,
  shapeTarf,
  shapeTouch,
  shapeVanillaInstrument,
  shapeVarianceSwap,
  shapeVolatilitySwap,
  shapeWindowBarrier,
  subscriptionKey,
} from "../src/functions/shaping";
import { EXCEL_FAMILIES, loadVectors, specOf, type GoldenVector } from "../e2e/corpus";

// ---------------------------------------------------------------------------
// the byte-identity assertion
// ---------------------------------------------------------------------------

/** Assert the spec-path instrument emits the byte-identical frame of the legacy one. */
function expectWireParity(spec: Instrument, legacy: Instrument): void {
  const specWire = instrumentToWire(spec);
  const legacyWire = instrumentToWire(legacy);
  // Key-for-key deep equality of the encoded frames…
  expect(specWire).toEqual(legacyWire);
  // …and byte equality of the canonical serialization (the token form).
  const token = encodeInstrumentToken(spec);
  expect(token).toBe(canonicalWireJson(legacyWire));
  // …and the token round-trip is lossless: decode → re-encode reproduces it.
  expect(encodeInstrumentToken(decodeInstrumentToken(token))).toBe(token);
}

// ---------------------------------------------------------------------------
// 1. corpus-wide parity — the retired per-family dispatch, verbatim
// ---------------------------------------------------------------------------

const num = (terms: Record<string, unknown>, key: string): number => {
  const x = terms[key];
  if (typeof x !== "number" || !Number.isFinite(x)) throw new Error(`term \`${key}\` missing`);
  return x;
};
const str = (terms: Record<string, unknown>, key: string): string => {
  const x = terms[key];
  if (typeof x !== "string") throw new Error(`term \`${key}\` missing`);
  return x;
};
const optNum = (terms: Record<string, unknown>, key: string): number | undefined => {
  const x = terms[key];
  return typeof x === "number" && Number.isFinite(x) ? x : undefined;
};
const cp = (token: string): "C" | "P" => (token === "CALL" ? "C" : "P");
const barrierSide = (token: string): "UP" | "DOWN" => (token === "UPPER" ? "UP" : "DOWN");
const linearSide = (token: string): "BUY" | "SELL" => (token === "BUY" ? "BUY" : "SELL");
const fixingToken = (variant: string): string =>
  ({
    KrwKftc18: "KRW.KFTC18",
    TwdTaipei: "TWD.TAIPEI",
    InrRbiRef: "INR.RBIB",
    BrlPtax: "BRL.PTAX",
    ClpDolarObs: "CLP.DOLAROBS",
    CopTrm: "COP.TRM",
  })[variant] as string;

function tenorFor(t: number): string {
  const months = Math.round(t * 12);
  if (months > 0 && months % 12 === 0) return `${months / 12}Y`;
  return `${Math.max(1, months)}M`;
}

/**
 * The RETIRED per-product dispatch, verbatim: how the deleted `CELNET.BARRIER` /
 * `CELNET.TARF` / … worksheet functions shaped each corpus vector (this is the
 * exact pre-retirement `e2e/corpus.ts` builder, preserved here as the parity
 * reference). It calls the same production shapers those functions called, with
 * the same arguments.
 *
 * Four families never had a per-product worksheet function — `strategy` (the
 * GUI/SDK leg-ladder, now the STRATEGY family's repeated ("legs", …) terms
 * rows) and the three post-retirement arms (`perpetual_option` 30 /
 * `listed_future_option` 31 / `pivot` 32). Their reference instruments are
 * HAND-BUILT typed literals (no production shaper involved), so the parity
 * proof for them is spec-path == hand-built frame, the strongest form.
 */
function legacyInstrumentOf(v: GoldenVector): Instrument {
  const t = num(v.terms, "expiry_years");
  const tenor = tenorFor(t);
  const pair = v.underlying;
  const notional = 1.0;

  switch (v.family) {
    case "vanilla":
      return shapeVanillaInstrument({
        pair,
        tenor,
        strikeOrDelta: num(v.terms, "strike"),
        callPut: cp(str(v.terms, "option_type")),
        notional,
      });
    case "strategy": {
      // HAND-BUILT reference (no retired function): strategy never had a single
      // `CELNET.*` worksheet function (it was the GUI/SDK leg-ladder), so the
      // parity proof is spec-path == hand-built frame, the strongest form.
      const legsRaw = v.terms["legs"] as Array<Record<string, unknown>>;
      const months = Math.round(t * 12);
      const handTenor: Tenor =
        months % 12 === 0 ? { unit: "YEARS", count: months / 12 } : { unit: "MONTHS", count: months };
      return {
        pair: { base: pair.slice(0, 3), quote: pair.slice(3, 6) },
        tenor: handTenor,
        expiryYears: t,
        quantity: { notional, baseCcy: true },
        side: "TWO_WAY",
        product: {
          kind: "strategy",
          strategy: {
            kind: str(v.terms, "kind") as StrategyKind,
            legs: legsRaw.map((leg) => ({
              optionType: String(leg["option_type"]) as OptionType,
              strike: { kind: "strike", strike: Number(leg["strike"]) },
              side: String(leg["side"]) as Side,
              ratio: Number(leg["ratio"]),
            })),
          },
        },
      };
    }
    case "single_barrier":
      return shapeBarrier({
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
    case "double_barrier":
      return shapeBarrier({
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
    case "digital":
      return shapeDigital({
        pair,
        tenor,
        strike: num(v.terms, "strike"),
        callPut: cp(str(v.terms, "option_type")),
        notional,
        style: str(v.terms, "style"),
        payout: num(v.terms, "payout"),
      });
    case "touch": {
      const kind = str(v.terms, "kind");
      const isDouble = kind === "DOUBLE_NO_TOUCH" || kind === "DOUBLE_ONE_TOUCH";
      return shapeTouch({
        pair,
        tenor,
        notional,
        kind,
        barrier: num(v.terms, "lower_barrier"),
        ...(isDouble ? { upperBarrier: num(v.terms, "upper_barrier") } : {}),
        rebate: num(v.terms, "rebate"),
        monitoring: str(v.terms, "monitoring"),
      });
    }
    case "variance_swap":
      return shapeVarianceSwap({ pair, tenor, notional, strikeVol: num(v.terms, "strike_vol") });
    case "volatility_swap":
      return shapeVolatilitySwap({ pair, tenor, notional, strikeVol: num(v.terms, "strike_vol") });
    case "asian_option":
      return shapeAsianOption({
        pair,
        tenor,
        strike: num(v.terms, "strike"),
        callPut: cp(str(v.terms, "option_type")),
        notional,
        averaging: str(v.terms, "averaging"),
        observations: num(v.terms, "observations"),
        method: str(v.terms, "method"),
      });
    case "forward_start":
      return shapeForwardStart({
        pair,
        tenor,
        callPut: cp(str(v.terms, "option_type")),
        moneyness: num(v.terms, "moneyness"),
        reset: num(v.terms, "reset"),
        notional,
      });
    case "cliquet": {
      const lf = optNum(v.terms, "local_floor");
      const lc = optNum(v.terms, "local_cap");
      const gf = optNum(v.terms, "global_floor");
      const gc = optNum(v.terms, "global_cap");
      return shapeCliquet({
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
    }
    case "quanto":
      return shapeQuanto({
        pair,
        tenor,
        callPut: cp(str(v.terms, "option_type")),
        strike: num(v.terms, "strike"),
        notional,
        conversionVol: num(v.terms, "conversion_vol"),
        correlation: num(v.terms, "correlation"),
        payoff: str(v.terms, "payoff"),
      });
    case "tarf":
      return shapeTarf({
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
    case "pivot": {
      // HAND-BUILT reference (post-retirement arm 32 — there was never a
      // `CELNET.PIVOT` worksheet function): the strongest parity form,
      // spec-path == hand-built typed frame. The equally-spaced k/n fixing
      // grid is written out literally (bit-identical to the SDK's
      // `equal_fixing_years`).
      const fixings = num(v.terms, "fixings");
      const fixingYears: number[] = [];
      for (let k = 1; k <= fixings; k += 1) fixingYears.push(k / fixings);
      const months = Math.round(t * 12);
      const handTenor: Tenor =
        months % 12 === 0 ? { unit: "YEARS", count: months / 12 } : { unit: "MONTHS", count: months };
      return {
        pair: { base: pair.slice(0, 3), quote: pair.slice(3, 6) },
        tenor: handTenor,
        expiryYears: t,
        quantity: { notional, baseCcy: true },
        side: "TWO_WAY",
        product: {
          kind: "pivot",
          pivot: {
            optionType: str(v.terms, "option_type") as OptionType,
            strike: num(v.terms, "strike"),
            pivot: num(v.terms, "pivot"),
            target: num(v.terms, "target"),
            leverage: num(v.terms, "leverage"),
            redemption: str(v.terms, "redemption") as "FULL_GAIN" | "CAPPED_GAIN",
            schedule: {
              fixingYears,
              fixingNotional: num(v.terms, "fixing_notional"),
            },
            mcPairs: 0,
            mcSeed: 0n,
          },
        },
      };
    }
    case "accumulator":
      return shapeAccumulator({
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
    case "lookback": {
      const style = str(v.terms, "style");
      return shapeLookback({
        pair,
        tenor,
        callPut: cp(str(v.terms, "option_type")),
        notional,
        style,
        monitoring: str(v.terms, "monitoring"),
        ...(style === "FIXED" ? { strike: num(v.terms, "strike") } : {}),
        observations: optNum(v.terms, "observations"),
      });
    }
    case "window_barrier":
      return shapeWindowBarrier({
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
    case "american":
      return shapeAmerican({
        pair,
        tenor,
        strike: num(v.terms, "strike"),
        callPut: cp(str(v.terms, "option_type")),
        notional,
      });
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
      return shapeBasket({
        pair,
        tenor,
        notional,
        callPut: cp(str(v.terms, "option_type")),
        strike: num(v.terms, "strike"),
        kind: str(v.terms, "kind"),
        legs,
        correlations: corr,
      });
    }
    case "fx_forward":
      return shapeForward({
        pair,
        tenor,
        contractRate: num(v.terms, "contract_rate"),
        notional: num(v.terms, "notional"),
        side: linearSide(str(v.terms, "side")),
      });
    case "fx_swap":
      return shapeSwap({
        pair,
        tenor,
        contractRate: num(v.terms, "contract_rate"),
        notional: num(v.terms, "notional"),
        side: linearSide(str(v.terms, "near_side")),
      });
    case "ndf":
      return shapeNdf({
        pair,
        tenor,
        contractRate: num(v.terms, "contract_rate"),
        notional: num(v.terms, "notional"),
        side: linearSide(str(v.terms, "side")),
        fixing: fixingToken(str(v.terms, "fixing")),
        settlementCcy: str(v.terms, "settlement_ccy"),
      });
    case "perpetual_option": {
      // HAND-BUILT reference (no shaper): the contract's canonical tenorless
      // shape — NO `tenor`, `expiry_years = 0` exactly, the product notional
      // mirroring the one quantity.
      const notional = num(v.terms, "notional");
      return {
        pair: { base: pair.slice(0, 3), quote: pair.slice(3, 6) },
        expiryYears: 0,
        quantity: { notional, baseCcy: true },
        side: "TWO_WAY",
        product: {
          kind: "perpetualOption",
          perpetualOption: {
            optionType: str(v.terms, "option_type") as OptionType,
            strike: num(v.terms, "strike"),
            notional,
          },
        },
      };
    }
    case "listed_future_option": {
      // HAND-BUILT reference (no shaper): the cross-asset underlying the corpus
      // class token names (same WTI=commodity / ES=equity mapping as the Rust
      // SDK conformance, projected onto the Excel grammar's venue-less
      // commodity form), the future's contract identity, and the margining tag.
      const notional = num(v.terms, "notional");
      const sym = v.terms["future_symbol"] as Record<string, unknown>;
      const handBuilt: Record<string, { underlying: Underlying; pair: Instrument["pair"] }> = {
        WTI: {
          underlying: {
            kind: "commodity",
            commodity: { symbol: { ticker: "WTI", venue: "" }, currency: "USD" },
            settlementCcy: "USD",
          },
          pair: { base: "WTI", quote: "USD" },
        },
        ES: {
          underlying: {
            kind: "equity",
            equity: { symbol: { ticker: "ES", venue: "XCME" }, currency: "USD" },
            settlementCcy: "USD",
          },
          pair: { base: "ES", quote: "USD" },
        },
      };
      const cls = handBuilt[v.underlying];
      if (!cls) throw new Error(`unknown listed-future underlying \`${v.underlying}\``);
      const months = Math.round(t * 12);
      const handTenor: Tenor =
        months % 12 === 0 ? { unit: "YEARS", count: months / 12 } : { unit: "MONTHS", count: months };
      return {
        pair: cls.pair,
        underlying: cls.underlying,
        tenor: handTenor,
        expiryYears: t,
        quantity: { notional, baseCcy: true },
        side: "TWO_WAY",
        product: {
          kind: "listedFutureOption",
          listedFutureOption: {
            futureSymbol: {
              ticker: String(sym["ticker"]),
              venue: typeof sym["venue"] === "string" ? sym["venue"] : "",
            },
            futureExpiryYears: num(v.terms, "future_expiry_years"),
            optionType: str(v.terms, "option_type") as OptionType,
            strike: num(v.terms, "strike"),
            notional,
            margining: str(v.terms, "margining") as Margining,
          },
        },
      };
    }
    default:
      throw new Error(`family \`${v.family}\` was not exposed by the retired functions`);
  }
}

describe("corpus-wide wire parity: INSTRUMENT spec == retired per-product functions", () => {
  // The cross-asset classes (equity / commodity / crypto) were born on the
  // polymorphic INSTRUMENT spec — they never had a retired per-product function, so
  // there is no legacy frame to compare them against here. Their wire encoding is
  // covered by the polymorphic round-trip tests above and the live e2e corpus. This
  // parity check is therefore scoped to the families that DID have a retired
  // per-product function (the legacy-parity invariant the INSTRUMENT spec preserves).
  const POLYMORPHIC_NATIVE = new Set<string>([
    "equity_option",
    "commodity_option",
    "crypto_option",
  ]);
  const LEGACY_FAMILIES = EXCEL_FAMILIES.filter((f) => !POLYMORPHIC_NATIVE.has(f));
  const legacy = new Set<string>(LEGACY_FAMILIES);
  const vectors = loadVectors().filter((v) => legacy.has(v.family));

  it("covers every retired family with at least one frozen vector", () => {
    const seen = new Set(vectors.map((v) => v.family));
    for (const family of LEGACY_FAMILIES) {
      expect(seen.has(family), `corpus has no \`${family}\` vector`).toBe(true);
    }
  });

  for (const family of LEGACY_FAMILIES) {
    it(`emits byte-identical frames for every \`${family}\` vector`, () => {
      const familyVectors = vectors.filter((v) => v.family === family);
      expect(familyVectors.length).toBeGreaterThan(0);
      for (const v of familyVectors) {
        expectWireParity(shapeSpecInstrument(specOf(v)), legacyInstrumentOf(v));
      }
    });
  }
});

// ---------------------------------------------------------------------------
// 2. per-retired-function parity for the optionals the corpus does not exercise
// ---------------------------------------------------------------------------

type Row = (string | number)[];
const FX = "EURUSD";

interface ParityCase {
  readonly name: string;
  readonly product: string;
  readonly terms: Row[];
  readonly tenor: string;
  readonly notional: number;
  readonly legacy: () => Instrument;
}

const CASES: readonly ParityCase[] = [
  {
    name: "PRICE/RFQ/SUBSCRIBE vanilla — delta strike form",
    product: "VANILLA",
    terms: [
      ["strike", "25dP"],
      ["callPut", "P"],
    ],
    tenor: "6M",
    notional: 2_500_000,
    legacy: () =>
      shapeVanillaInstrument({
        pair: FX,
        tenor: "6M",
        strikeOrDelta: "25dP",
        callPut: "P",
        notional: 2_500_000,
      }),
  },
  {
    name: "VANILLA — ATM (delta-neutral) form",
    product: "VANILLA",
    terms: [
      ["strike", "ATM"],
      ["callPut", "C"],
    ],
    tenor: "1Y",
    notional: 1e6,
    legacy: () =>
      shapeVanillaInstrument({ pair: FX, tenor: "1Y", strikeOrDelta: "ATM", callPut: "C", notional: 1e6 }),
  },
  {
    name: "BARRIER — single KO under the LSV booking model, delta strike",
    product: "BARRIER",
    terms: [
      ["strike", "25dC"],
      ["callPut", "C"],
      ["barrier", 1.3],
      ["kind", "KNOCK_OUT"],
      ["side", "UP"],
      ["model", "LSV"],
    ],
    tenor: "1Y",
    notional: 1e6,
    legacy: () =>
      shapeBarrier({
        pair: FX,
        tenor: "1Y",
        strikeOrDelta: "25dC",
        callPut: "C",
        notional: 1e6,
        barrier: 1.3,
        kind: "KNOCK_OUT",
        side: "UP",
        model: "LSV",
      }),
  },
  {
    name: "BARRIER — double, defaults (kind/rebate/monitoring omitted)",
    product: "BARRIER",
    terms: [
      ["strike", 1.1],
      ["callPut", "P"],
      ["barrier", 1.0],
      ["upperBarrier", 1.25],
    ],
    tenor: "3M",
    notional: 5e5,
    legacy: () =>
      shapeBarrier({
        pair: FX,
        tenor: "3M",
        strikeOrDelta: 1.1,
        callPut: "P",
        notional: 5e5,
        barrier: 1.0,
        upperBarrier: 1.25,
      }),
  },
  {
    name: "WINDOWBARRIER — full-life defaults (window/mc omitted)",
    product: "WINDOWBARRIER",
    terms: [
      ["strike", 1.1],
      ["callPut", "C"],
      ["barrier", 1.3],
    ],
    tenor: "1Y",
    notional: 1e6,
    legacy: () =>
      shapeWindowBarrier({
        pair: FX,
        tenor: "1Y",
        strikeOrDelta: 1.1,
        callPut: "C",
        notional: 1e6,
        barrier: 1.3,
      }),
  },
  {
    name: "DIGITAL — minimal (style/payout omitted)",
    product: "DIGITAL",
    terms: [
      ["strike", 1.1],
      ["callPut", "C"],
    ],
    tenor: "1Y",
    notional: 1e6,
    legacy: () =>
      shapeDigital({ pair: FX, tenor: "1Y", strike: 1.1, callPut: "C", notional: 1e6 }),
  },
  {
    name: "TOUCH — one-touch minimal",
    product: "TOUCH",
    terms: [
      ["kind", "OT"],
      ["barrier", 1.2],
      ["rebate", 100000],
    ],
    tenor: "6M",
    notional: 1e6,
    legacy: () =>
      shapeTouch({ pair: FX, tenor: "6M", kind: "OT", barrier: 1.2, notional: 1e6, rebate: 100000 }),
  },
  {
    name: "VARSWAP — fresh (strikeVol omitted)",
    product: "VARSWAP",
    terms: [],
    tenor: "1Y",
    notional: 1e6,
    legacy: () => shapeVarianceSwap({ pair: FX, tenor: "1Y", notional: 1e6 }),
  },
  {
    name: "VOLSWAP — pinned strike vol",
    product: "VOLSWAP",
    terms: [["strikeVol", 0.11]],
    tenor: "1Y",
    notional: 1e6,
    legacy: () => shapeVolatilitySwap({ pair: FX, tenor: "1Y", notional: 1e6, strikeVol: 0.11 }),
  },
  {
    name: "ASIAN — seasoned (elapsedAvg/elapsedWeight) + TW method",
    product: "ASIAN",
    terms: [
      ["strike", 1.1],
      ["callPut", "C"],
      ["averaging", "DISCRETE"],
      ["observations", 12],
      ["method", "TW"],
      ["elapsedAvg", 1.08],
      ["elapsedWeight", 0.25],
    ],
    tenor: "1Y",
    notional: 1e6,
    legacy: () =>
      shapeAsianOption({
        pair: FX,
        tenor: "1Y",
        strike: 1.1,
        callPut: "C",
        notional: 1e6,
        averaging: "DISCRETE",
        observations: 12,
        method: "TW",
        elapsedAvg: 1.08,
        elapsedWeight: 0.25,
      }),
  },
  {
    name: "FORWARDSTART",
    product: "FORWARDSTART",
    terms: [
      ["callPut", "C"],
      ["moneyness", 1.0],
      ["reset", 0.25],
    ],
    tenor: "1Y",
    notional: 1e6,
    legacy: () =>
      shapeForwardStart({ pair: FX, tenor: "1Y", callPut: "C", moneyness: 1.0, reset: 0.25, notional: 1e6 }),
  },
  {
    name: "CLIQUET — clamped with MC knobs",
    product: "CLIQUET",
    terms: [
      ["callPut", "C"],
      ["moneyness", 1.0],
      ["periods", 4],
      ["localFloor", 0],
      ["localCap", 0.05],
      ["globalFloor", 0.01],
      ["mcPairs", 20000],
      ["mcSeed", 42],
    ],
    tenor: "1Y",
    notional: 1e6,
    legacy: () =>
      shapeCliquet({
        pair: FX,
        tenor: "1Y",
        callPut: "C",
        moneyness: 1.0,
        periods: 4,
        notional: 1e6,
        localFloor: 0,
        localCap: 0.05,
        globalFloor: 0.01,
        mcPairs: 20000,
        mcSeed: 42,
      }),
  },
  {
    name: "QUANTO — digital payoff",
    product: "QUANTO",
    terms: [
      ["callPut", "P"],
      ["strike", 1.1],
      ["conversionVol", 0.09],
      ["correlation", -0.3],
      ["payoff", "DIGITAL"],
    ],
    tenor: "1Y",
    notional: 1e6,
    legacy: () =>
      shapeQuanto({
        pair: FX,
        tenor: "1Y",
        callPut: "P",
        strike: 1.1,
        notional: 1e6,
        conversionVol: 0.09,
        correlation: -0.3,
        payoff: "DIGITAL",
      }),
  },
  {
    name: "TARF — capped gain + MC knobs",
    product: "TARF",
    terms: [
      ["callPut", "P"],
      ["strike", 1.1],
      ["target", 0.12],
      ["leverage", 2],
      ["fixings", 12],
      ["redemption", "CAPPED_GAIN"],
      ["fixingNotional", 0.5],
      ["mcPairs", 30000],
      ["mcSeed", 7],
    ],
    tenor: "1Y",
    notional: 1e6,
    legacy: () =>
      shapeTarf({
        pair: FX,
        tenor: "1Y",
        callPut: "P",
        strike: 1.1,
        target: 0.12,
        leverage: 2,
        fixings: 12,
        notional: 1e6,
        redemption: "CAPPED_GAIN",
        fixingNotional: 0.5,
        mcPairs: 30000,
        mcSeed: 7,
      }),
  },
  {
    name: "PIVOT — overlap geometry, capped gain + MC knobs",
    product: "PIVOT",
    terms: [
      ["callPut", "P"],
      ["strike", 1.12],
      ["pivot", 1.08],
      ["target", 0.12],
      ["leverage", 2.5],
      ["fixings", 12],
      ["redemption", "CAPPED_GAIN"],
      ["fixingNotional", 0.5],
      ["mcPairs", 30000],
      ["mcSeed", 1707],
    ],
    tenor: "1Y",
    notional: 1e6,
    legacy: () =>
      shapePivot({
        pair: FX,
        tenor: "1Y",
        callPut: "P",
        strike: 1.12,
        pivot: 1.08,
        target: 0.12,
        leverage: 2.5,
        fixings: 12,
        notional: 1e6,
        redemption: "CAPPED_GAIN",
        fixingNotional: 0.5,
        mcPairs: 30000,
        mcSeed: 1707,
      }),
  },
  {
    name: "ACCUMULATOR — continuous monitoring + MC knobs",
    product: "ACCUMULATOR",
    terms: [
      ["pivot", 1.1],
      ["barrier", 1.15],
      ["leverage", 2],
      ["fixings", 12],
      ["monitoring", "CONTINUOUS"],
      ["fixingNotional", 0.25],
      ["mcPairs", 25000],
      ["mcSeed", 11],
    ],
    tenor: "1Y",
    notional: 1e6,
    legacy: () =>
      shapeAccumulator({
        pair: FX,
        tenor: "1Y",
        pivot: 1.1,
        barrier: 1.15,
        leverage: 2,
        fixings: 12,
        notional: 1e6,
        monitoring: "CONTINUOUS",
        fixingNotional: 0.25,
        mcPairs: 25000,
        mcSeed: 11,
      }),
  },
  {
    name: "LOOKBACK — fixed-strike, discrete MC with budget",
    product: "LOOKBACK",
    terms: [
      ["callPut", "C"],
      ["style", "FIXED"],
      ["monitoring", "DISCRETE"],
      ["strike", 1.1],
      ["observations", 52],
      ["mcPairs", 40000],
      ["mcSeed", 9],
    ],
    tenor: "1Y",
    notional: 1e6,
    legacy: () =>
      shapeLookback({
        pair: FX,
        tenor: "1Y",
        callPut: "C",
        notional: 1e6,
        style: "FIXED",
        monitoring: "DISCRETE",
        strike: 1.1,
        observations: 52,
        mcPairs: 40000,
        mcSeed: 9,
      }),
  },
  {
    name: "AMERICAN — Bermudan via step count",
    product: "AMERICAN",
    terms: [
      ["strike", 1.1],
      ["callPut", "P"],
      ["style", "BERMUDAN"],
      ["bermudanSteps", 4],
    ],
    tenor: "1Y",
    notional: 1e6,
    legacy: () =>
      shapeAmerican({
        pair: FX,
        tenor: "1Y",
        strike: 1.1,
        callPut: "P",
        notional: 1e6,
        style: "BERMUDAN",
        bermudanSteps: 4,
      }),
  },
  {
    name: "AMERICAN — Longstaff-Schwartz MC engine knobs",
    product: "AMERICAN",
    terms: [
      ["strike", 1.1],
      ["callPut", "P"],
      ["lsmPaths", 50000],
      ["lsmExerciseDates", 50],
      ["lsmSeed", 13],
    ],
    tenor: "1Y",
    notional: 1e6,
    legacy: () =>
      shapeAmerican({
        pair: FX,
        tenor: "1Y",
        strike: 1.1,
        callPut: "P",
        notional: 1e6,
        lsmPaths: 50000,
        lsmExerciseDates: 50,
        lsmSeed: 13,
      }),
  },
  {
    name: "BASKET — best-of with MC knobs (repeated legs/correlations rows)",
    product: "BASKET",
    terms: [
      ["callPut", "C"],
      ["strike", 1.18],
      ["kind", "BEST_OF"],
      ["legs", "EURUSD", 0.6, 1.1, 0.1, 0.02],
      ["legs", "GBPUSD", 0.4, 1.27, 0.11, 0.03],
      ["correlations", 1, 0.6],
      ["correlations", 0.6, 1],
      ["mcPaths", 4096],
      ["mcReplications", 8],
      ["mcSteps", 16],
      ["mcSeed", 5],
    ],
    tenor: "1Y",
    notional: 1e6,
    legacy: () =>
      shapeBasket({
        pair: FX,
        tenor: "1Y",
        notional: 1e6,
        callPut: "C",
        strike: 1.18,
        kind: "BEST_OF",
        legs: [
          ["EURUSD", 0.6, 1.1, 0.1, 0.02],
          ["GBPUSD", 0.4, 1.27, 0.11, 0.03],
        ],
        correlations: [
          [1, 0.6],
          [0.6, 1],
        ],
        mcPaths: 4096,
        mcReplications: 8,
        mcSteps: 16,
        mcSeed: 5,
      }),
  },
  {
    name: "FORWARD — default BUY side",
    product: "FORWARD",
    terms: [["rate", 1.105]],
    tenor: "1Y",
    notional: 1e6,
    legacy: () => shapeForward({ pair: FX, tenor: "1Y", contractRate: 1.105, notional: 1e6 }),
  },
  {
    name: "SWAP — explicit near side",
    product: "SWAP",
    terms: [
      ["rate", 1.105],
      ["nearSide", "SELL"],
    ],
    tenor: "1Y",
    notional: 1e6,
    legacy: () =>
      shapeSwap({ pair: FX, tenor: "1Y", contractRate: 1.105, notional: 1e6, side: "SELL" }),
  },
  {
    name: "NDF — default settlement ccy",
    product: "NDF",
    terms: [
      ["rate", 5.45],
      ["fixing", "BRL.PTAX"],
      ["side", "SELL"],
    ],
    tenor: "6M",
    notional: 1e6,
    legacy: () =>
      shapeNdf({
        pair: "USDBRL",
        tenor: "6M",
        contractRate: 5.45,
        notional: 1e6,
        fixing: "BRL.PTAX",
        side: "SELL",
      }),
  },
];

describe("per-retired-function parity (optional parameters the corpus omits)", () => {
  for (const c of CASES) {
    it(c.name, () => {
      const underlier = c.name.startsWith("NDF") ? "USDBRL" : FX;
      const spec = shapeSpecInstrument({
        underlier,
        product: c.product,
        terms: c.terms,
        tenor: c.tenor,
        notional: c.notional,
      });
      expectWireParity(spec, c.legacy());
    });
  }

  it("is order-free: shuffled terms rows encode the identical token", () => {
    const base = CASES.find((c) => c.product === "TARF") as ParityCase;
    const shuffled = [...base.terms].reverse();
    const a = encodeInstrumentToken(
      shapeSpecInstrument({
        underlier: FX,
        product: base.product,
        terms: base.terms,
        tenor: base.tenor,
        notional: base.notional,
      }),
    );
    const b = encodeInstrumentToken(
      shapeSpecInstrument({
        underlier: FX,
        product: base.product,
        terms: shuffled,
        tenor: base.tenor,
        notional: base.notional,
      }),
    );
    expect(a).toBe(b);
  });

  it("accepts tenor/notional as terms instead of positional arguments", () => {
    const viaArgs = shapeSpecInstrument({
      underlier: FX,
      product: "VANILLA",
      terms: [
        ["strike", 1.12],
        ["callPut", "C"],
      ],
      tenor: "1Y",
      notional: 1e6,
    });
    const viaTerms = shapeSpecInstrument({
      underlier: FX,
      product: "VANILLA",
      terms: [
        ["strike", 1.12],
        ["callPut", "C"],
        ["tenor", "1Y"],
        ["notional", 1e6],
      ],
    });
    expect(encodeInstrumentToken(viaTerms)).toBe(encodeInstrumentToken(viaArgs));
  });
});

// ---------------------------------------------------------------------------
// 3. cross-asset parity — the underlier grammar vs the per-class vanilla shapers
// ---------------------------------------------------------------------------

describe("cross-asset underlier parity vs the per-class vanilla shapers", () => {
  const vanillaTerms = (strike: string | number, callPut: string): Row[] => [
    ["strike", strike],
    ["callPut", callPut],
  ];

  it("equity: AAPL@XNAS:USD == shapeEquityVanilla", () => {
    const spec = shapeSpecInstrument({
      underlier: "AAPL@XNAS:USD",
      product: "VANILLA",
      terms: vanillaTerms(200, "C"),
      tenor: "3M",
      notional: 1e4,
    });
    const legacy = shapeEquityVanilla({
      ticker: "AAPL",
      currency: "USD",
      venue: "XNAS",
      tenor: "3M",
      strikeOrDelta: 200,
      callPut: "C",
      notional: 1e4,
    });
    expectWireParity(spec, legacy);
  });

  it("commodity: BRENT@:USD (empty venue) == shapeCommodityVanilla", () => {
    const spec = shapeSpecInstrument({
      underlier: "BRENT@:USD",
      product: "VANILLA",
      terms: vanillaTerms(85, "P"),
      tenor: "6M",
      notional: 1e3,
    });
    const legacy = shapeCommodityVanilla({
      symbol: "BRENT",
      currency: "USD",
      tenor: "6M",
      strikeOrDelta: 85,
      callPut: "P",
      notional: 1e3,
    });
    expectWireParity(spec, legacy);
  });

  it("crypto LINEAR: BTC-USDT == shapeCryptoVanilla (settlement_style omitted)", () => {
    const spec = shapeSpecInstrument({
      underlier: "BTC-USDT",
      product: "VANILLA",
      terms: vanillaTerms(70000, "C"),
      tenor: "1M",
      notional: 10,
    });
    const legacy = shapeCryptoVanilla({
      pair: "BTC-USDT",
      tenor: "1M",
      strikeOrDelta: 70000,
      callPut: "C",
      notional: 10,
    });
    expectWireParity(spec, legacy);
  });

  it("crypto INVERSE: BTC/USD:inverse == shapeCryptoVanilla(INVERSE_COIN)", () => {
    const spec = shapeSpecInstrument({
      underlier: "BTC/USD:inverse",
      product: "VANILLA",
      terms: vanillaTerms(65000, "P"),
      tenor: "3M",
      notional: 5,
    });
    const legacy = shapeCryptoVanilla({
      pair: "BTC-USD",
      tenor: "3M",
      strikeOrDelta: 65000,
      callPut: "P",
      notional: 5,
      settlementStyle: "INVERSE_COIN",
    });
    expectWireParity(spec, legacy);
    expect(instrumentToWire(spec)["settlement_style"]).toBe(1);
  });

  it("metal: XAUUSD == shapeMetalVanilla (ISO X-code projection)", () => {
    const spec = shapeSpecInstrument({
      underlier: "XAUUSD",
      product: "VANILLA",
      terms: vanillaTerms(2400, "C"),
      tenor: "1Y",
      notional: 100,
    });
    const legacy = shapeMetalVanilla({
      metal: "gold",
      quote: "USD",
      tenor: "1Y",
      strikeOrDelta: 2400,
      callPut: "C",
      notional: 100,
    });
    expectWireParity(spec, legacy);
    expect(spec.pair).toEqual({ base: "XAU", quote: "USD" });
  });

  it("metal-quoted metal pair (XAU/XAG) is rejected with a typed error — metals quote vs fiat only", () => {
    // HONESTY: the conventions registry and `MetalPair.quote` are metal-vs-FIAT
    // only; a metal-vs-metal ratio would emit a wire shape the server can only
    // refuse, so the grammar rejects it up-front with a typed, named error.
    for (const underlier of ["XAU/XAG", "XAUXAG", "XPT/XPD"]) {
      expect(() =>
        shapeSpecInstrument({
          underlier,
          product: "VANILLA",
          terms: vanillaTerms(80, "C"),
          tenor: "1Y",
          notional: 100,
        }),
      ).toThrowError(/metal-quoted metal pair .* not priceable/);
    }
    // The metal-vs-FIAT crosses the registry DOES carry stay accepted.
    const spec = shapeSpecInstrument({
      underlier: "XAU/EUR",
      product: "VANILLA",
      terms: vanillaTerms(2100, "C"),
      tenor: "1Y",
      notional: 100,
    });
    expect(spec.underlying).toEqual({
      kind: "metal",
      metal: { metal: "GOLD", quote: "EUR" },
      settlementCcy: "EUR",
    });
    expect(spec.pair).toEqual({ base: "XAU", quote: "EUR" });
  });

  it("overlays a NON-vanilla family onto a non-FX underlier (one verb, any family, any class)", () => {
    const spec = shapeSpecInstrument({
      underlier: "XAUUSD",
      product: "BARRIER",
      terms: [
        ["strike", 2400],
        ["callPut", "C"],
        ["barrier", 2600],
        ["kind", "KNOCK_OUT"],
        ["side", "UP"],
      ],
      tenor: "1Y",
      notional: 100,
    });
    // The product body is EXACTLY what the FX barrier shaper builds on the
    // metal projection; the underlying arm rides alongside it.
    const fxBody = shapeBarrier({
      pair: { base: "XAU", quote: "USD" },
      tenor: "1Y",
      strikeOrDelta: 2400,
      callPut: "C",
      notional: 100,
      barrier: 2600,
      kind: "KNOCK_OUT",
      side: "UP",
    });
    const wire = instrumentToWire(spec);
    expect(wire["single_barrier"]).toEqual(instrumentToWire(fxBody)["single_barrier"]);
    expect(wire["underlying"]).toEqual({
      metal: { metal: 0, quote: "USD" },
      settlement_ccy: "USD",
    });
    // Lossless through the token, like every other family.
    const token = encodeInstrumentToken(spec);
    expect(encodeInstrumentToken(decodeInstrumentToken(token))).toBe(token);
  });

  it("rejects a cross-asset (equity/commodity/crypto) EXOTIC at build time — those classes price only VANILLA/PERPETUAL/FUTUREOPTION", () => {
    // The cost-of-carry leaves (VANILLA) plus the two asset-class-agnostic arms
    // (PERPETUAL, FUTUREOPTION) are the ONLY cross-asset engines on the server
    // (`price_cross_asset`); every other family is FX/metal-only. The grammar
    // rejects the unpriceable combination up-front — like the metal-vs-metal guard —
    // rather than emitting a frame the server refuses at price time with the
    // misleading "pricing model DEFAULT does not support …" error.
    const rejected: Array<[string, string, Row[]]> = [
      ["AAPL@XNAS:USD", "BARRIER", [["strike", 200], ["callPut", "C"], ["barrier", 240], ["kind", "KNOCK_OUT"], ["side", "UP"]]],
      ["BRENT@:USD", "ASIAN", [["strike", 85], ["callPut", "C"], ["averaging", "DISCRETE"], ["observations", 12]]],
      ["BTC/USDT", "TARF", [["callPut", "C"], ["strike", 70000], ["target", 0.1], ["leverage", 2], ["fixings", 12]]],
      ["ETH/USD:inverse", "VARSWAP", [["strikeVol", 0.6]]],
      ["AAPL@XNAS:USD", "FORWARD", [["rate", 200]]],
    ];
    for (const [underlier, product, terms] of rejected) {
      expect(() => shapeSpecInstrument({ underlier, product, terms, tenor: "1Y", notional: 1 })).toThrowError(
        /supports only VANILLA, PERPETUAL and FUTUREOPTION/,
      );
    }
    // The three SUPPORTED cross-asset families build cleanly (no false rejection)…
    expect(() => shapeSpecInstrument({ underlier: "AAPL@XNAS:USD", product: "VANILLA", terms: [["strike", 200], ["callPut", "C"]], tenor: "3M" })).not.toThrow();
    expect(() => shapeSpecInstrument({ underlier: "BTC/USD", product: "PERPETUAL", terms: [["strike", 70000], ["callPut", "C"]] })).not.toThrow();
    expect(() =>
      shapeSpecInstrument({ underlier: "ES@XCME:USD", product: "FUTUREOPTION", terms: [["strike", 5000], ["callPut", "C"], ["futureSymbol", "ESZ5@XCME"], ["futureExpiry", 0.3]], tenor: "3M" }),
    ).not.toThrow();
    // …and METAL/FX keep the FULL product set (they route to the FX engine), so a
    // metal barrier is NOT rejected.
    expect(() =>
      shapeSpecInstrument({ underlier: "XAUUSD", product: "BARRIER", terms: [["strike", 2400], ["callPut", "C"], ["barrier", 2600], ["kind", "KNOCK_OUT"], ["side", "UP"]], tenor: "1Y", notional: 100 }),
    ).not.toThrow();
  });

  it("FX byte-identity: the spec path never leaks cross-asset keys onto an FX frame", () => {
    const spec = shapeSpecInstrument({
      underlier: "EUR/USD",
      product: "VANILLA",
      terms: vanillaTerms("25dC", "C"),
      tenor: "3M",
      notional: 1e6,
    });
    const wire = instrumentToWire(spec);
    expect("underlying" in wire).toBe(false);
    expect("settlement_style" in wire).toBe(false);
    expect(wire["pair"]).toEqual({ base: "EUR", quote: "USD" });
  });

  it("inverse vs linear settlement on the same pair must NOT coalesce one stream", () => {
    const linear = shapeSpecInstrument({
      underlier: "BTC/USD",
      product: "VANILLA",
      terms: vanillaTerms(65000, "C"),
      tenor: "3M",
      notional: 5,
    });
    const inverse = shapeSpecInstrument({
      underlier: "BTC/USD:inverse",
      product: "VANILLA",
      terms: vanillaTerms(65000, "C"),
      tenor: "3M",
      notional: 5,
    });
    expect(subscriptionKey(linear, DEFAULT_CONVENTIONS)).not.toBe(
      subscriptionKey(inverse, DEFAULT_CONVENTIONS),
    );
  });
});

// ---------------------------------------------------------------------------
// 4. underlier grammar + 5. typed-error surface
// ---------------------------------------------------------------------------

describe("underlier grammar", () => {
  it("parses the five classes from the one grammar", () => {
    expect(parseUnderlier("EURUSD").underlying).toBeUndefined();
    expect(parseUnderlier("EUR/USD").pair).toEqual({ base: "EUR", quote: "USD" });
    expect(parseUnderlier("XAU/EUR").underlying?.kind).toBe("metal");
    expect(parseUnderlier("xpdusd").underlying?.kind).toBe("metal");
    expect(parseUnderlier("AAPL@XNAS:USD").underlying?.kind).toBe("equity");
    expect(parseUnderlier("BRENT@:USD").underlying?.kind).toBe("commodity");
    expect(parseUnderlier("BTC/USD").underlying?.kind).toBe("digitalAsset");
    expect(parseUnderlier("DOGEUSDT").underlying?.kind).toBe("digitalAsset");
    expect(parseUnderlier("FOO/USDT").underlying?.kind).toBe("digitalAsset");
  });

  it("reads the settlement suffix (crypto only) and defaults LINEAR", () => {
    expect(parseUnderlier("BTC/USD").settlementStyle).toBe("LINEAR");
    expect(parseUnderlier("BTC/USD:inverse").settlementStyle).toBe("INVERSE_COIN");
    expect(parseUnderlier("FOO/USD:linear").underlying?.kind).toBe("digitalAsset");
    expect(() => parseUnderlier("XAUUSD:inverse")).toThrow(ShapingError);
    expect(() => parseUnderlier("AAPL@XNAS:USD:inverse")).toThrow(ShapingError);
  });

  it("rejects an unparseable underlier with the grammar in the message", () => {
    expect(() => parseUnderlier("")).toThrow(ShapingError);
    expect(() => parseUnderlier("AAPL")).toThrow(/invalid underlier/);
    expect(() => parseUnderlier("AAPL@XNAS")).toThrow(ShapingError);
  });
});

describe("terms + family typed errors (honest, key-naming)", () => {
  const ok: Row[] = [
    ["strike", 1.12],
    ["callPut", "C"],
  ];

  it("rejects an unknown product family, listing the family set", () => {
    expect(() =>
      shapeSpecInstrument({ underlier: FX, product: "SNOWBALL", terms: ok, tenor: "1Y" }),
    ).toThrow(new RegExp(FAMILY_NAMES[0] as string));
  });

  it("names a missing required key", () => {
    expect(() =>
      shapeSpecInstrument({ underlier: FX, product: "BARRIER", terms: ok, tenor: "1Y" }),
    ).toThrow(/requires the numeric term `barrier`/);
  });

  it("names an unknown key and lists the family's key set", () => {
    expect(() =>
      shapeSpecInstrument({
        underlier: FX,
        product: "VANILLA",
        terms: [...ok, ["pivot", 1.1]],
        tenor: "1Y",
      }),
    ).toThrow(/unknown term `pivot` for VANILLA \(expected: strike, callPut/);
  });

  it("rejects a duplicate key and a key without a value", () => {
    expect(() =>
      shapeSpecInstrument({
        underlier: FX,
        product: "VANILLA",
        terms: [...ok, ["strike", 1.2]],
        tenor: "1Y",
      }),
    ).toThrow(/duplicate term `strike`/);
    expect(() =>
      shapeSpecInstrument({
        underlier: FX,
        product: "VANILLA",
        terms: [["strike", 1.12], ["callPut"]],
        tenor: "1Y",
      }),
    ).toThrow(/missing its value/);
  });

  it("rejects tenor/notional supplied both positionally and as terms", () => {
    expect(() =>
      shapeSpecInstrument({
        underlier: FX,
        product: "VANILLA",
        terms: [...ok, ["tenor", "1Y"]],
        tenor: "1Y",
      }),
    ).toThrow(/tenor supplied twice/);
    expect(() =>
      shapeSpecInstrument({
        underlier: FX,
        product: "VANILLA",
        terms: [...ok, ["notional", 1e6]],
        tenor: "1Y",
        notional: 1e6,
      }),
    ).toThrow(/notional supplied twice/);
  });

  it("requires a tenor from one of the two places", () => {
    expect(() => shapeSpecInstrument({ underlier: FX, product: "VANILLA", terms: ok })).toThrow(
      /a tenor is required/,
    );
  });
});

describe("instrument token", () => {
  it("detects tokens vs legacy positional first arguments", () => {
    const token = encodeInstrumentToken(
      shapeVanillaInstrument({ pair: FX, tenor: "1Y", strikeOrDelta: 1.12, callPut: "C", notional: 1e6 }),
    );
    expect(isInstrumentToken(token)).toBe(true);
    expect(isInstrumentToken("EURUSD")).toBe(false);
    expect(isInstrumentToken(undefined)).toBe(false);
  });

  it("rejects a malformed token with a typed error", () => {
    expect(() => decodeInstrumentToken("not json")).toThrow(ShapingError);
    expect(() => decodeInstrumentToken("[1,2]")).toThrow(ShapingError);
    expect(() => decodeInstrumentToken("{}")).toThrow(/missing\/invalid object `pair`/);
    expect(() =>
      decodeInstrumentToken('{"pair":{"base":"EUR","quote":"USD"},"tenor":{"unit":3,"count":1}}'),
    ).toThrow(/exactly one product arm/);
  });

  it("is canonical: key-sorted, compact, deterministic", () => {
    const token = encodeInstrumentToken(
      shapeVanillaInstrument({ pair: FX, tenor: "1Y", strikeOrDelta: 1.12, callPut: "C", notional: 1e6 }),
    );
    expect(token).toBe(
      '{"expiry_years":1,"pair":{"base":"EUR","quote":"USD"},"quantity":{"base_ccy":true,"notional":1000000},"side":2,"tenor":{"count":1,"unit":3},"vanilla":{"option_type":0,"strike":{"strike":1.12}}}',
    );
  });
});
