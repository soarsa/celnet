/**
 * GUI conformance lane (W0) for Celnet's FROZEN golden-vector corpus.
 *
 * The single source of pricing truth is the server-side `celnet-vanilla` /
 * `celnet-exotics` / `celnet-surface` Rust crates, whose reference prices are
 * frozen on disk at `crates/celnet-golden/vectors/*.json` (each vector pinned to a
 * named oracle: a QuantLib table, a published reference, or a code-disjoint
 * Monte-Carlo). This test loads that corpus DIRECTLY (read-only; the corpus is
 * never copied or mutated), rebuilds the SAME `Instrument` the GUI builds for each
 * supported family, prices it through the GUI's real in-process pricer
 * (`src/data/pricing.ts::priceInstrument`), and asserts the GUI's priced result
 * equals the frozen expected price within an HONEST conformance band:
 *
 *   allowed = max( corpusBand + k*(sigmaCorpus + sigmaGui),  guiAccuracyBand )
 *
 * where `corpusBand = max(rel*|expected|, abs)` is the vector's OWN frozen tolerance
 * (used verbatim - never lowered), `k*(sigmaCorpus + sigmaGui)` is a Monte-Carlo
 * band from BOTH independent estimators (the corpus MC oracle and the GUI's
 * code-disjoint MC; zero for closed forms), and `guiAccuracyBand` is the GUI's
 * DECLARED numerical accuracy class. The last term exists because the GUI's offline
 * pricers are an explicit presentation-side stand-in (see the header of
 * `src/data/pricing.ts`): they use the Abramowitz-Stegun 7.1.26 rational normal-CDF
 * (~1e-7 ABSOLUTE accuracy, amplified through nonlinear barrier/touch payoffs) and
 * finite-difference Greeks, whereas the authoritative server uses the exact
 * `libm::erfc` f64 path. The corpus's ~1e-7 tolerances are calibrated for that EXACT
 * server; the GUI cannot meet them and is not meant to. So the conformance lane
 * proves the GUI agrees with the server reference within the GUI's own (measured,
 * declared) accuracy - it does NOT relax the corpus band (kept as a floor), and the
 * GUI band is far tighter than any gross model error (it still catches the order-15%
 * discrete-lookback model gap that keeps the lookback family OUT of the
 * offline-exposed set; see below).
 *
 * Honesty (CLAUDE.md rules 2 & 5): every family the GUI faithfully prices offline is
 * covered. Two corpus families are deliberately NOT exposed offline and are listed
 * with their reason in `FAMILIES_NOT_EXPOSED_BY_GUI`, and the reason is asserted
 * (not assumed):
 *  - `window_barrier` - no closed form; server LOCAL_STOCH_VOL-only. The offline
 *    pricer THROWS rather than fabricate a value (CLAUDE.md rule 2).
 *  - `lookback` (discrete) - the corpus's discrete-monitoring lookback uses the
 *    Beaglehole-Dybvig-Zhou continuous-within-segment extremum correction; the GUI's
 *    offline discrete lookback samples the extremum only at the fixing points (a
 *    cruder, genuinely DIFFERENT model that systematically underestimates by ~15%).
 *    This real model gap is asserted to EXIST rather than papered over by an inflated
 *    tolerance.
 *
 * No `as any`, no skipped/`.only` tests, no lowered corpus assertions.
 */
import { readFileSync, readdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { describe, expect, it } from "vitest";

import type {
  AsianOption,
  BasketKind,
  BasketLeg,
  CcyPair,
  Instrument,
  Leg,
  MarketContext,
  OptionType,
  Product,
  StrategyKind,
} from "../src/data/contract";
import { priceInstrument } from "../src/data/pricing";
import { equalFixingSchedule, tenorYearsToTenor } from "../src/data/seed";

// ---------------------------------------------------------------------------
// corpus location - read the FROZEN vectors in place (never copied / mutated)
// ---------------------------------------------------------------------------

const TEST_DIR = dirname(fileURLToPath(import.meta.url));
/** `gui/test` -> repo root -> `crates/celnet-golden/vectors`. */
const VECTORS_DIR = join(TEST_DIR, "..", "..", "crates", "celnet-golden", "vectors");

// ---------------------------------------------------------------------------
// the frozen on-disk vector shape (read-only mirror; we do not own this schema)
// ---------------------------------------------------------------------------

interface VectorMarket {
  spot: number;
  vol: number;
  r_dom: number;
  r_for: number;
}

interface VectorExpected {
  price: number;
  greeks?: Record<string, number>;
  price_std_error: number | null;
  oracle: string;
}

interface VectorTolerance {
  rel: number;
  abs: number;
}

/** One frozen golden vector (the union of all families' `terms` payloads). */
interface GoldenVector {
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
function loadCorpus(): Map<string, GoldenVector[]> {
  const files = readdirSync(VECTORS_DIR).filter((f) => f.endsWith(".json"));
  const byFamily = new Map<string, GoldenVector[]>();
  for (const file of files) {
    const raw = readFileSync(join(VECTORS_DIR, file), "utf8");
    const vectors = JSON.parse(raw) as GoldenVector[];
    for (const v of vectors) {
      const list = byFamily.get(v.family) ?? [];
      list.push(v);
      byFamily.set(v.family, list);
    }
  }
  return byFamily;
}

const CORPUS = loadCorpus();

// ---------------------------------------------------------------------------
// family coverage declaration (honest; every faithfully-priced family is here)
// ---------------------------------------------------------------------------

/**
 * The families the GUI's in-process `priceInstrument` prices faithfully enough to
 * conform to the server reference within the GUI accuracy class. Every entry MUST
 * have a builder below and a non-empty vector set in the corpus.
 */
const FAMILIES_COVERED = [
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
  "american",
  "basket",
] as const;

/**
 * The corpus families the GUI deliberately does NOT price with its OFFLINE
 * in-process pricer, each with a concrete, asserted reason (never a silent gap;
 * CLAUDE.md rule 2):
 *  - `window_barrier` - no closed form, server LOCAL_STOCH_VOL-only: the offline
 *    pricer THROWS.
 *  - `lookback` - the corpus's discrete lookback uses a Beaglehole-Dybvig-Zhou
 *    continuous-within-segment extremum correction the GUI's fixing-point-only
 *    offline pricer does not model (a ~15% systematic gap), so the offline price is
 *    a genuinely different, cruder model and is NOT claimed to conform.
 *  - `fx_forward` / `fx_swap` / `ndf` - the W2 LINEAR products are discounted
 *    cashflows, not option payoffs: the GUI's offline OPTION pricer does not price
 *    them. The GUI DOES book them faithfully onto the wire (the `forward`/`swap`/
 *    `ndf` ProductSpecs, gated by `linearProducts.test.ts`); their numerical
 *    conformance is the server/SDK gate (`celnet-linear` vs its independent oracle),
 *    not the offline GUI pricer.
 *  - `equity_option` / `commodity_option` / `crypto_option` - the cross-asset
 *    vanillas price through the GENERALIZED cost-of-carry seam (a single net carry
 *    `b` over equity dividend/repo, commodity convenience yield, or crypto funding),
 *    which the GUI's FX-two-rate offline pricer does not model. The GUI DOES book
 *    them faithfully onto the wire (the `crossAssetSpec` over the `Underlying` oneof
 *    + `settlement_style`, gated by `crossAssetProducts.test.ts`); their numerical
 *    conformance is the server/SDK gate (the cross-asset leaves vs their independent
 *    generalized-BSM / Black-76 oracles), not the offline GUI pricer.
 */
const FAMILIES_NOT_EXPOSED_BY_GUI = [
  "window_barrier",
  "lookback",
  "fx_forward",
  "fx_swap",
  "ndf",
  "equity_option",
  "commodity_option",
  "crypto_option",
] as const;

// ---------------------------------------------------------------------------
// GUI declared numerical-accuracy class (NOT a relaxation of the corpus band)
// ---------------------------------------------------------------------------

/** Standard-error multiplier for the MC sigma-band (~5 sigma => vanishing flake). */
const SIGMA_MULTIPLIER = 5;

/**
 * The GUI's documented offline-pricer accuracy class. The offline pricers are a
 * presentation-side stand-in using the Abramowitz-Stegun 7.1.26 rational normal CDF
 * (~1e-7 absolute, amplified through nonlinear payoffs) and finite-difference
 * Greeks - NOT the server's exact `libm::erfc` f64 path. Measured worst-case over
 * the whole closed-form corpus is ~5e-4 relative (a one-touch); 1e-3 / 1e-5 is a
 * principled upper bound on that class - far tighter than any gross model error
 * (e.g. the order-15% discrete-lookback gap), so it still fails LOUDLY on a real
 * regression while not demanding exact-server precision the GUI never promises.
 */
const GUI_ACCURACY_REL = 1e-3;
const GUI_ACCURACY_ABS = 1e-5;

/**
 * The allowed absolute gap between the GUI price and the frozen expected price:
 *   max( corpusBand + k*(sigmaCorpus + sigmaGui),  guiAccuracyBand )
 * The corpus's own frozen `max(rel*|expected|, abs)` is kept as a floor (so the
 * looser MC bands are honoured); the MC sigma term adds both independent estimators'
 * noise; the GUI accuracy band covers the GUI's documented approximation where the
 * corpus band (calibrated for the exact server) is tighter than the GUI can achieve.
 */
function allowedGap(v: GoldenVector, guiStdError: number | undefined): number {
  const corpusBand = Math.max(v.tolerance.rel * Math.abs(v.expected.price), v.tolerance.abs);
  const sigmaCorpus = v.expected.price_std_error ?? 0;
  const sigmaGui = guiStdError ?? 0;
  const corpusWithSigma = corpusBand + SIGMA_MULTIPLIER * (sigmaCorpus + sigmaGui);
  const guiBand = Math.max(GUI_ACCURACY_REL * Math.abs(v.expected.price), GUI_ACCURACY_ABS);
  return Math.max(corpusWithSigma, guiBand);
}

// ---------------------------------------------------------------------------
// typed accessors over a vector's untyped `terms` payload
// ---------------------------------------------------------------------------

function num(terms: Record<string, unknown>, key: string): number {
  const v = terms[key];
  if (typeof v !== "number") {
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

const PAIR: CcyPair = { base: "EUR", quote: "USD" };

function market(v: GoldenVector): MarketContext {
  return { spot: v.market.spot, vol: v.market.vol, rDom: v.market.r_dom, rFor: v.market.r_for };
}

/** Wrap a `Product` in the minimal `Instrument` the GUI prices (price is per-unit). */
function instrumentOf(v: GoldenVector, product: Product): Instrument {
  const t = num(v.terms, "expiry_years");
  return {
    pair: PAIR,
    tenor: tenorYearsToTenor(t),
    expiryYears: t,
    quantity: { notional: 1, baseCcy: true },
    side: "TWO_WAY",
    product,
  };
}

// ---------------------------------------------------------------------------
// per-family Instrument builders - exactly the GUI's `Product` shapes
// ---------------------------------------------------------------------------

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
    const side = str(lt, "side");
    if (side !== "BUY" && side !== "SELL" && side !== "TWO_WAY") {
      throw new Error(`unknown leg side ${side}`);
    }
    return {
      optionType: optionType(lt),
      strike: { kind: "strike", strike: num(lt, "strike") },
      side,
      ratio: num(lt, "ratio"),
    };
  });
  return instrumentOf(v, { kind: "strategy", strategy: { kind, legs } });
}

function buildSingleBarrier(v: GoldenVector): Instrument {
  const kind = str(v.terms, "kind");
  if (kind !== "KNOCK_IN" && kind !== "KNOCK_OUT") throw new Error(`barrier kind ${kind}`);
  // The corpus encodes a single-barrier side as LOWER/UPPER (relative to spot); the
  // GUI `BarrierSide` enum is DOWN/UP. Map them (LOWER = below spot = DOWN).
  const rawSide = str(v.terms, "side");
  const side = rawSide === "UPPER" ? "UP" : rawSide === "LOWER" ? "DOWN" : rawSide;
  if (side !== "UP" && side !== "DOWN") throw new Error(`barrier side ${rawSide}`);
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
      side,
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
  if (averaging !== "DISCRETE" && averaging !== "CONTINUOUS") throw new Error(`averaging ${averaging}`);
  const method = str(v.terms, "method");
  if (method !== "CURRAN" && method !== "TURNBULL_WAKEMAN") throw new Error(`asian method ${method}`);
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

/** Dispatch a vector to the GUI Instrument its family builds. */
const BUILDERS: Record<(typeof FAMILIES_COVERED)[number], (v: GoldenVector) => Instrument> = {
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
  accumulator: buildAccumulator,
  american: buildAmerican,
  basket: buildBasket,
};

// ---------------------------------------------------------------------------
// the conformance suite - one assertion per vector, per covered family
// ---------------------------------------------------------------------------

describe("golden-vector conformance - GUI price == frozen server reference", () => {
  it("loaded the frozen corpus with every covered family present", () => {
    for (const family of FAMILIES_COVERED) {
      const vectors = CORPUS.get(family);
      expect(vectors, `corpus has vectors for family ${family}`).toBeDefined();
      expect(vectors!.length, `family ${family} has at least one vector`).toBeGreaterThan(0);
    }
  });

  it("every corpus family is either covered or explicitly declared not-exposed", () => {
    const declared = new Set<string>([...FAMILIES_COVERED, ...FAMILIES_NOT_EXPOSED_BY_GUI]);
    for (const family of CORPUS.keys()) {
      expect(declared.has(family), `corpus family ${family} is accounted for`).toBe(true);
    }
  });

  for (const family of FAMILIES_COVERED) {
    describe(family, () => {
      const vectors = CORPUS.get(family) ?? [];
      for (const v of vectors) {
        it(`${v.id} (oracle: ${v.expected.oracle})`, () => {
          const instrument = BUILDERS[family](v);
          const outcome = priceInstrument(instrument, market(v));

          // Whenever the GUI prices by Monte-Carlo it MUST surface a genuine,
          // positive stderr (the honest precision band; mirrors `Quote.priceStdError`).
          // Note this is about the GUI's OWN method, not the corpus oracle's: some
          // corpus vectors carry a `price_std_error` because the REFERENCE was MC
          // while the GUI prices the same product in closed form (e.g. the analytic
          // Asian), so the GUI legitimately reports no stderr there.
          if (outcome.priceStdError !== undefined) {
            expect(
              outcome.priceStdError,
              `${family} ${v.id}: GUI MC stderr is positive`,
            ).toBeGreaterThan(0);
          }

          const gap = Math.abs(outcome.greeks.price - v.expected.price);
          const allowed = allowedGap(v, outcome.priceStdError);
          expect(
            gap,
            `${family} ${v.id}: GUI ${outcome.greeks.price} vs expected ${v.expected.price} ` +
              `(gap ${gap.toExponential(3)} > allowed ${allowed.toExponential(3)})`,
          ).toBeLessThanOrEqual(allowed);
        });
      }
    });
  }
});

// ---------------------------------------------------------------------------
// declared NON-coverage - asserted, never a silent gap (CLAUDE.md rule 2)
// ---------------------------------------------------------------------------

describe("window_barrier - declared not exposed offline (server LSV-only)", () => {
  const vectors = CORPUS.get("window_barrier") ?? [];

  it("the corpus carries window-barrier vectors (so the gap is real, not absent)", () => {
    expect(vectors.length).toBeGreaterThan(0);
  });

  for (const v of vectors) {
    it(`${v.id}: offline pricing throws (never fabricates an LSV value)`, () => {
      const t = num(v.terms, "expiry_years");
      // The window barrier never prices offline, so the BarrierSide value is
      // immaterial here; map the corpus's LOWER/UPPER onto the GUI DOWN/UP enum so
      // the Instrument is well-typed before the (expected) throw.
      const rawSide = str(v.terms, "side");
      const side = rawSide === "UPPER" || rawSide === "UP" ? "UP" : "DOWN";
      const instrument: Instrument = {
        pair: PAIR,
        tenor: tenorYearsToTenor(t),
        expiryYears: t,
        quantity: { notional: 1, baseCcy: true },
        side: "TWO_WAY",
        pricingModel: "LOCAL_STOCH_VOL",
        product: {
          kind: "windowBarrier",
          windowBarrier: {
            vanilla: {
              optionType: optionType(v.terms),
              strike: { kind: "strike", strike: num(v.terms, "strike") },
            },
            barrier: num(v.terms, "barrier"),
            side,
            windowStart: num(v.terms, "window_start"),
            windowEnd: num(v.terms, "window_end"),
            mcPairs: num(v.terms, "mc_pairs"),
            mcSteps: num(v.terms, "mc_steps"),
            mcSeed: BigInt(num(v.terms, "mc_seed")),
          },
        },
      };
      expect(() => priceInstrument(instrument, market(v))).toThrow();
    });
  }
});

describe("lookback - discrete-monitoring model gap is real (not papered over)", () => {
  const vectors = CORPUS.get("lookback") ?? [];

  it("the corpus carries discrete-lookback vectors", () => {
    expect(vectors.length).toBeGreaterThan(0);
    for (const v of vectors) {
      expect(str(v.terms, "monitoring")).toBe("DISCRETE");
    }
  });

  // Prove the GUI's offline discrete lookback genuinely DIFFERS from the corpus
  // reference (it omits the Beaglehole-Dybvig-Zhou continuous-within-segment
  // extremum correction => a material underestimate), so excluding it from the
  // conformance set is honest non-coverage, not a hidden failure. We assert the gap
  // is LARGER than the GUI accuracy class would allow (it is genuinely a different
  // model) AND in the expected direction (the discrete-point extremum under-captures
  // the true extremum => floating/fixed lookback value is biased DOWN).
  for (const v of vectors) {
    it(`${v.id}: GUI offline price is materially below the corpus reference`, () => {
      const t = num(v.terms, "expiry_years");
      const style = str(v.terms, "style");
      if (style !== "FLOATING" && style !== "FIXED") throw new Error(`lookback style ${style}`);
      const monitoring = str(v.terms, "monitoring");
      if (monitoring !== "CONTINUOUS" && monitoring !== "DISCRETE") {
        throw new Error(`lookback monitoring ${monitoring}`);
      }
      const instrument: Instrument = {
        pair: PAIR,
        tenor: tenorYearsToTenor(t),
        expiryYears: t,
        quantity: { notional: 1, baseCcy: true },
        side: "TWO_WAY",
        product: {
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
        },
      };
      const outcome = priceInstrument(instrument, market(v));
      const guiBand = Math.max(
        GUI_ACCURACY_REL * Math.abs(v.expected.price),
        GUI_ACCURACY_ABS,
      );
      const gap = v.expected.price - outcome.greeks.price;
      // The gap exists (well beyond the GUI accuracy class) and is positive (the GUI
      // under-captures the extremum). This documents the real model difference.
      expect(gap).toBeGreaterThan(guiBand);
    });
  }
});
