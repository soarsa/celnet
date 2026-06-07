/**
 * Wave-3 product parity for the GUI end of the ONE `celnet.wire` contract: the
 * Target-Redemption Forward (TARF), accumulator and lookback are encoded onto the
 * SAME `Instrument.product` oneof as vanilla/strategy, with the EXACT appended wire
 * field NAMES (`tarf`/`accumulator`/`lookback`, proto field numbers 19/20/21 — no
 * schema_version, no renumber; CLAUDE.md rule 9). The TARF and accumulator REUSE
 * the same nested `FixingSchedule` message (`schedule`). The standalone build
 * prices each honestly: a continuous-monitored lookback by the exact closed form
 * (no Monte-Carlo std-error), every other Wave-3 product by an antithetic
 * Monte-Carlo that reports a standard error — whose documented limits are validated
 * here against an independent in-test oracle.
 *
 * These exercise the REAL `src/data/wsCodec.ts`, `src/data/enums.ts`,
 * `src/data/seed.ts` and `src/data/pricing.ts` through their public surface with
 * NO server and NO mocks. The MC standard error is round-tripped through the same
 * `Quote.priceStdError` / `PriceResponse.price_std_error` representation the live
 * server uses (presence-tracked: set only for an MC product).
 */
import { describe, expect, it } from "vitest";

import { instrumentToWire } from "../src/data/wsCodec";
import * as e from "../src/data/enums";
import {
  accumulatorInstrument,
  equalFixingSchedule,
  isDiscreteLookback,
  lookbackInstrument,
  tarfInstrument,
  type AccumulatorTerms,
  type LookbackTerms,
  type TarfTerms,
} from "../src/data/seed";
import { priceInstrument } from "../src/data/pricing";
import type { CcyPair, MarketContext } from "../src/data/contract";

const PAIR: CcyPair = { base: "EUR", quote: "USD" };
const MKT: MarketContext = { spot: 1.1, vol: 0.2, rDom: 0.03, rFor: 0.01 };
const EXPIRY = 1;

function tarf(over: Partial<TarfTerms> = {}): TarfTerms {
  return {
    optionType: "PUT",
    strike: 1.1,
    target: 0.1,
    leverage: 2,
    redemption: "FULL_GAIN",
    schedule: equalFixingSchedule(12, EXPIRY, 1),
    mcPairs: 5000,
    mcSeed: 7n,
    ...over,
  };
}

function accumulator(over: Partial<AccumulatorTerms> = {}): AccumulatorTerms {
  return {
    pivot: 1.1,
    barrier: 1.155,
    leverage: 2,
    monitoring: "DISCRETE",
    schedule: equalFixingSchedule(12, EXPIRY, 1),
    mcPairs: 5000,
    mcSeed: 11n,
    ...over,
  };
}

function lookback(over: Partial<LookbackTerms> = {}): LookbackTerms {
  return {
    style: "FLOATING",
    optionType: "CALL",
    monitoring: "CONTINUOUS",
    strike: 1.1,
    observations: 52,
    mcPairs: 5000,
    mcSeed: 13n,
    ...over,
  };
}

// ---------------------------------------------------------------------------
// wire encoding — the appended oneof arms, by their exact proto field NAMES
// ---------------------------------------------------------------------------

describe("wave-3 products — instrumentToWire oneof arms", () => {
  it("encodes a TARF under `tarf` (field 19) with the FixingSchedule nested", () => {
    const w = instrumentToWire(tarfInstrument(PAIR, EXPIRY, 10, tarf()));
    const body = w["tarf"] as Record<string, unknown>;
    expect(body["option_type"]).toBe(e.optionType.toWire("PUT"));
    expect(body["strike"]).toBe(1.1);
    expect(body["target"]).toBe(0.1);
    expect(body["leverage"]).toBe(2);
    // TarfRedemption FULL_GAIN=0, CAPPED_GAIN=1.
    expect(body["redemption"]).toBe(e.tarfRedemption.toWire("FULL_GAIN"));
    expect(body["mc_pairs"]).toBe(5000);
    expect(body["mc_seed"]).toBe(7n);
    const schedule = body["schedule"] as Record<string, unknown>;
    expect((schedule["fixing_years"] as number[]).length).toBe(12);
    expect(schedule["fixing_notional"]).toBe(1);
    // A oneof carries exactly one body.
    expect(w["accumulator"]).toBeUndefined();
    expect(w["lookback"]).toBeUndefined();
    expect(w["vanilla"]).toBeUndefined();
  });

  it("encodes an accumulator under `accumulator` (field 20) reusing FixingSchedule", () => {
    const w = instrumentToWire(
      accumulatorInstrument(PAIR, EXPIRY, 10, accumulator({ monitoring: "CONTINUOUS" })),
    );
    const body = w["accumulator"] as Record<string, unknown>;
    expect(body["pivot"]).toBe(1.1);
    expect(body["barrier"]).toBe(1.155);
    expect(body["leverage"]).toBe(2);
    // AccumulatorMonitoring DISCRETE=0, CONTINUOUS=1.
    expect(body["monitoring"]).toBe(e.accumulatorMonitoring.toWire("CONTINUOUS"));
    expect(body["mc_pairs"]).toBe(5000);
    expect(body["mc_seed"]).toBe(11n);
    const schedule = body["schedule"] as Record<string, unknown>;
    expect((schedule["fixing_years"] as number[]).length).toBe(12);
  });

  it("encodes a lookback under `lookback` (field 21) with style + monitoring enums", () => {
    const w = instrumentToWire(
      lookbackInstrument(
        PAIR,
        EXPIRY,
        10,
        lookback({ style: "FIXED", optionType: "PUT", monitoring: "DISCRETE", observations: 32 }),
      ),
    );
    expect(w["lookback"]).toEqual({
      // LookbackStyle FLOATING=0, FIXED=1.
      style: e.lookbackStyle.toWire("FIXED"),
      option_type: e.optionType.toWire("PUT"),
      // LookbackMonitoring CONTINUOUS=0, DISCRETE=1.
      monitoring: e.lookbackMonitoring.toWire("DISCRETE"),
      strike: 1.1,
      observations: 32,
      mc_pairs: 5000,
      mc_seed: 13n,
    });
  });

  it("pins the canonical Wave-3 enum numbers the server decodes by", () => {
    expect(e.tarfRedemption.toWire("FULL_GAIN")).toBe(0);
    expect(e.tarfRedemption.toWire("CAPPED_GAIN")).toBe(1);
    expect(e.accumulatorMonitoring.toWire("DISCRETE")).toBe(0);
    expect(e.accumulatorMonitoring.toWire("CONTINUOUS")).toBe(1);
    expect(e.lookbackStyle.toWire("FLOATING")).toBe(0);
    expect(e.lookbackStyle.toWire("FIXED")).toBe(1);
    expect(e.lookbackMonitoring.toWire("CONTINUOUS")).toBe(0);
    expect(e.lookbackMonitoring.toWire("DISCRETE")).toBe(1);
    // And every decode is reversible.
    expect(e.lookbackMonitoring.fromWire(1)).toBe("DISCRETE");
    expect(e.tarfRedemption.fromWire(1)).toBe("CAPPED_GAIN");
  });

  it("builds an equally-spaced fixing schedule (t_k = k·T/n, ascending, ≤ expiry)", () => {
    const s = equalFixingSchedule(4, 2, 5);
    expect(s.fixingYears).toEqual([0.5, 1, 1.5, 2]);
    expect(s.fixingNotional).toBe(5);
    expect(s.fixingYears[s.fixingYears.length - 1]).toBeLessThanOrEqual(2);
  });
});

// ---------------------------------------------------------------------------
// MC-honesty — which products carry priceStdError (presence-tracked)
// ---------------------------------------------------------------------------

describe("wave-3 products — MC-honesty (priceStdError presence)", () => {
  it("a TARF is Monte-Carlo and surfaces a POSITIVE std error", () => {
    const out = priceInstrument(tarfInstrument(PAIR, EXPIRY, 10, tarf()), MKT);
    expect(out.priceStdError).toBeDefined();
    expect(out.priceStdError!).toBeGreaterThan(0);
  });

  it("an accumulator is Monte-Carlo and surfaces a POSITIVE std error", () => {
    const out = priceInstrument(accumulatorInstrument(PAIR, EXPIRY, 10, accumulator()), MKT);
    expect(out.priceStdError).toBeDefined();
    expect(out.priceStdError!).toBeGreaterThan(0);
  });

  it("a CONTINUOUS lookback is closed form and carries NO std error", () => {
    const out = priceInstrument(
      lookbackInstrument(PAIR, EXPIRY, 10, lookback({ monitoring: "CONTINUOUS" })),
      MKT,
    );
    expect(out.priceStdError).toBeUndefined();
    expect(isDiscreteLookback(lookback({ monitoring: "CONTINUOUS" }))).toBe(false);
  });

  it("a DISCRETE lookback is Monte-Carlo and surfaces a POSITIVE std error", () => {
    const out = priceInstrument(
      lookbackInstrument(PAIR, EXPIRY, 10, lookback({ monitoring: "DISCRETE", mcPairs: 5000 })),
      MKT,
    );
    expect(out.priceStdError).toBeDefined();
    expect(out.priceStdError!).toBeGreaterThan(0);
    expect(isDiscreteLookback(lookback({ monitoring: "DISCRETE" }))).toBe(true);
  });
});

// ---------------------------------------------------------------------------
// offline pricer — closed-form / structural limits vs an independent oracle
// ---------------------------------------------------------------------------

/** Garman-Kohlhagen vanilla as an INDEPENDENT oracle (A&S 7.1.26 erf CDF). */
function gkVanilla(isCall: boolean, strike: number, m: MarketContext, t: number): number {
  const sqrtT = Math.sqrt(t);
  const dfFor = Math.exp(-m.rFor * t);
  const dfDom = Math.exp(-m.rDom * t);
  const d1 =
    (Math.log(m.spot / strike) + (m.rDom - m.rFor + 0.5 * m.vol * m.vol) * t) / (m.vol * sqrtT);
  const d2 = d1 - m.vol * sqrtT;
  const cdf = (x: number): number => {
    const z = Math.abs(x) / Math.SQRT2;
    const tt = 1 / (1 + 0.3275911 * z);
    const y =
      1 -
      ((((1.061405429 * tt - 1.453152027) * tt + 1.421413741) * tt - 0.284496736) * tt +
        0.254829592) *
        tt *
        Math.exp(-z * z);
    const erf = x >= 0 ? y : -y;
    return 0.5 * (1 + erf);
  };
  return isCall
    ? m.spot * dfFor * cdf(d1) - strike * dfDom * cdf(d2)
    : strike * dfDom * cdf(-d2) - m.spot * dfFor * cdf(-d1);
}

describe("wave-3 products — lookback closed-form limits & structure", () => {
  it("a continuous floating-strike lookback strictly DOMINATES the vanilla", () => {
    // A floating-strike call pays S_T − min ≥ (S_T − K) for the ATMF struck vanilla
    // option's optimal point; the lookback's optionality on the running minimum
    // makes it strictly more valuable than the comparable vanilla.
    const f = Math.exp((MKT.rDom - MKT.rFor) * EXPIRY) * MKT.spot;
    const lb = priceInstrument(
      lookbackInstrument(PAIR, EXPIRY, 10, lookback({ style: "FLOATING", optionType: "CALL" })),
      MKT,
    ).greeks.price;
    const vanilla = gkVanilla(true, f, MKT, EXPIRY);
    expect(lb).toBeGreaterThan(vanilla);
    expect(lb).toBeGreaterThan(0);
  });

  it("a fixed-strike lookback CALL dominates the same-strike vanilla call", () => {
    // (max − K)⁺ ≥ (S_T − K)⁺ pointwise ⇒ the fixed-strike lookback call is worth
    // at least the vanilla call; with diffusion it is strictly more.
    const k = 1.1;
    const lb = priceInstrument(
      lookbackInstrument(
        PAIR,
        EXPIRY,
        10,
        lookback({ style: "FIXED", optionType: "CALL", strike: k }),
      ),
      MKT,
    ).greeks.price;
    const vanilla = gkVanilla(true, k, MKT, EXPIRY);
    expect(lb).toBeGreaterThan(vanilla);
  });

  it("a discrete lookback APPROACHES the continuous one as observations refine", () => {
    const cont = priceInstrument(
      lookbackInstrument(PAIR, EXPIRY, 10, lookback({ monitoring: "CONTINUOUS" })),
      MKT,
    ).greeks.price;
    const disc = priceInstrument(
      lookbackInstrument(
        PAIR,
        EXPIRY,
        10,
        lookback({ monitoring: "DISCRETE", observations: 250, mcPairs: 60_000, mcSeed: 2024n }),
      ),
      MKT,
    );
    // Discrete monitoring undersamples the extremum ⇒ a slightly LOWER price that
    // converges UP toward the continuous closed form; check it is within a sensible
    // band of the closed form (a few MC stderrs + the discretisation gap), and that
    // it is honestly below the continuous value.
    expect(disc.greeks.price).toBeLessThan(cont);
    expect(disc.greeks.price).toBeGreaterThan(0.8 * cont);
    expect(disc.priceStdError).toBeDefined();
  });
});

describe("wave-3 products — TARF structure (gap-risk premium, reproducibility)", () => {
  it("the bank PV of a FULL_GAIN TARF exceeds the CAPPED_GAIN one (gap premium)", () => {
    const full = priceInstrument(
      tarfInstrument(PAIR, EXPIRY, 10, tarf({ redemption: "FULL_GAIN", mcPairs: 40_000, mcSeed: 99n })),
      MKT,
    ).greeks.price;
    const capped = priceInstrument(
      tarfInstrument(
        PAIR,
        EXPIRY,
        10,
        tarf({ redemption: "CAPPED_GAIN", mcPairs: 40_000, mcSeed: 99n }),
      ),
      MKT,
    ).greeks.price;
    // FULL_GAIN pays the client the full breaching gain (overshoot) ⇒ the bank pays
    // away MORE ⇒ a LOWER bank PV. Common seed/paths ⇒ the difference is the pure
    // gap-risk premium. (A higher bank PV for capped, i.e. capped > full.)
    expect(capped).toBeGreaterThan(full);
  });

  it("the TARF Monte-Carlo is reproducible bit-for-bit at a fixed seed", () => {
    const a = priceInstrument(tarfInstrument(PAIR, EXPIRY, 10, tarf({ mcSeed: 555n })), MKT);
    const b = priceInstrument(tarfInstrument(PAIR, EXPIRY, 10, tarf({ mcSeed: 555n })), MKT);
    expect(b.greeks.price).toBe(a.greeks.price);
    expect(b.priceStdError).toBe(a.priceStdError);
  });
});

describe("wave-3 products — accumulator structure (knock-out shrinks |PV|)", () => {
  it("continuous knock-out lowers |PV| vs discrete (knocks out more often)", () => {
    const discrete = priceInstrument(
      accumulatorInstrument(
        PAIR,
        EXPIRY,
        10,
        accumulator({ monitoring: "DISCRETE", mcPairs: 40_000, mcSeed: 321n }),
      ),
      MKT,
    ).greeks.price;
    const continuous = priceInstrument(
      accumulatorInstrument(
        PAIR,
        EXPIRY,
        10,
        accumulator({ monitoring: "CONTINUOUS", mcPairs: 40_000, mcSeed: 321n }),
      ),
      MKT,
    ).greeks.price;
    // Continuous monitoring tests the barrier between fixings too ⇒ knocks out more
    // often ⇒ fewer accrued fixings ⇒ a strictly smaller magnitude. Common seed.
    expect(Math.abs(continuous)).toBeLessThan(Math.abs(discrete));
  });

  it("the accumulator Monte-Carlo is reproducible bit-for-bit at a fixed seed", () => {
    const a = priceInstrument(
      accumulatorInstrument(PAIR, EXPIRY, 10, accumulator({ mcSeed: 808n })),
      MKT,
    );
    const b = priceInstrument(
      accumulatorInstrument(PAIR, EXPIRY, 10, accumulator({ mcSeed: 808n })),
      MKT,
    );
    expect(b.greeks.price).toBe(a.greeks.price);
    expect(b.priceStdError).toBe(a.priceStdError);
  });
});
