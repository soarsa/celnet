/**
 * American / Bermudan early-exercise vanilla for the GUI end of the ONE
 * `celnet.wire` contract (PC-AMERICAN). The Rust slice added a new `american`
 * product arm at proto field 24 (after `pricing_model`=22 and `window_barrier`=23)
 * — APPENDED additively: no `schema_version`, no renumber, the existing arms
 * byte-identical (GUIDE.md rule 9). These tests exercise the REAL
 * `src/data/seed.ts`, `src/data/enums.ts`, `src/data/wsCodec.ts` and
 * `src/data/pricing.ts` through their public surface with NO server and NO mocks.
 *
 * The numerics claim is honest and INDEPENDENTLY oracle'd:
 *  - the offline pricer is a genuine Cox-Ross-Rubinstein binomial tree (a real
 *    early-exercise method, distinct from the server's PSOR free-boundary FD — so
 *    agreement is a cross-check, not an echo);
 *  - the published-value gate pins it to Longstaff & Schwartz (2001) Table 1
 *    (American put, S0=K=40, r=0.06, σ=0.20, T=1, no dividend ⇒ FD reference
 *    2.314) — the same value the Rust server gate pins (Lesson c: a hand-stated
 *    published number, not derived from the code under test);
 *  - the European lower bound is an INDEPENDENT in-test Black-Scholes put (no call
 *    into pricing.ts), so the early-exercise premium is measured against a
 *    genuinely separate oracle.
 *
 * The wire-encoding gate validates the exact field NAMES / numeric enum tags the
 * server WS codec (`american_from_json`) decodes by — the proto contract's pinned
 * tags are the independent oracle there.
 */
import { describe, expect, it } from "vitest";

import { instrumentToWire } from "../src/data/wsCodec";
import * as e from "../src/data/enums";
import { americanInstrument, type AmericanTerms } from "../src/data/seed";
import { priceInstrument } from "../src/data/pricing";
import type { CcyPair, MarketContext } from "../src/data/contract";

const PAIR: CcyPair = { base: "EUR", quote: "USD" };
const EXPIRY = 1;

/** The Longstaff-Schwartz (2001) Table-1 market: S0=K=40, r=0.06, σ=0.20, T=1, q=0. */
const LS_MARKET: MarketContext = { spot: 40, vol: 0.2, rDom: 0.06, rFor: 0 };

function american(over: Partial<AmericanTerms> = {}): AmericanTerms {
  return {
    optionType: "PUT",
    strike: 40,
    exerciseStyle: "AMERICAN",
    bermudanDates: [],
    lsmPaths: 0,
    lsmExerciseDates: 0,
    lsmSeed: 0n,
    ...over,
  };
}

// ---------------------------------------------------------------------------
// independent European Black-Scholes put oracle (NO call into pricing.ts)
// ---------------------------------------------------------------------------

/** Standard normal CDF (Abramowitz-Stegun 26.2.17) — hand-coded, independent. */
function normCdf(x: number): number {
  const t = 1 / (1 + 0.2316419 * Math.abs(x));
  const d = 0.3989422804014327 * Math.exp(-0.5 * x * x);
  const poly =
    t * (0.319381530 + t * (-0.356563782 + t * (1.781477937 + t * (-1.821255978 + t * 1.330274429))));
  const cnd = 1 - d * poly;
  return x >= 0 ? cnd : 1 - cnd;
}

/** A European Garman-Kohlhagen PUT value, computed independently in-test. */
function europeanPut(m: MarketContext, strike: number, t: number): number {
  const sqrtT = Math.sqrt(t);
  const d1 = (Math.log(m.spot / strike) + (m.rDom - m.rFor + 0.5 * m.vol * m.vol) * t) / (m.vol * sqrtT);
  const d2 = d1 - m.vol * sqrtT;
  return strike * Math.exp(-m.rDom * t) * normCdf(-d2) - m.spot * Math.exp(-m.rFor * t) * normCdf(-d1);
}

// ---------------------------------------------------------------------------
// ExerciseStyle enum — pinned to the canonical proto numbers (Lesson c)
// ---------------------------------------------------------------------------

describe("PC-AMERICAN ExerciseStyle enum — pinned to the canonical proto numbers", () => {
  it("maps AMERICAN=0 / BERMUDAN=1 reversibly", () => {
    // Authoritative tags: EXERCISE_STYLE_AMERICAN=0, EXERCISE_STYLE_BERMUDAN=1.
    // Hand-stated literals (not derived from the codec) so a mis-stated enum
    // cannot pass against itself.
    expect(e.exerciseStyle.toWire("AMERICAN")).toBe(0);
    expect(e.exerciseStyle.toWire("BERMUDAN")).toBe(1);
    expect(e.exerciseStyle.fromWire(0)).toBe("AMERICAN");
    expect(e.exerciseStyle.fromWire(1)).toBe("BERMUDAN");
    // An unknown tag clamps to the proto3 zero value (AMERICAN), like a proto reader.
    expect(e.exerciseStyle.fromWire(99)).toBe("AMERICAN");
  });
});

// ---------------------------------------------------------------------------
// american product arm (proto field 24) — exact wire shape
// ---------------------------------------------------------------------------

describe("PC-AMERICAN american — instrumentToWire oneof arm (field 24)", () => {
  it("encodes an American put under `american` with the exact server field names/tags", () => {
    const w = instrumentToWire(
      americanInstrument(PAIR, EXPIRY, 10, american({ optionType: "PUT", strike: 1.1 })),
    );
    const body = w["american"] as Record<string, unknown>;
    // american_from_json: option_type=1, strike=2, exercise_style=3,
    // bermudan_dates=4, lsm_paths=5, lsm_exercise_dates=6, lsm_seed=7.
    expect(body["option_type"]).toBe(e.optionType.toWire("PUT"));
    expect(body["strike"]).toBe(1.1);
    expect(body["exercise_style"]).toBe(e.exerciseStyle.toWire("AMERICAN"));
    expect(body["exercise_style"]).toBe(0);
    expect(body["bermudan_dates"]).toEqual([]);
    expect(body["lsm_paths"]).toBe(0);
    expect(body["lsm_exercise_dates"]).toBe(0);
    expect(body["lsm_seed"]).toBe(0n);
    // A oneof carries exactly one body (no other product arm leaks in).
    expect(w["vanilla"]).toBeUndefined();
    expect(w["window_barrier"]).toBeUndefined();
    // It is a DEFAULT-engine product (no LSV) ⇒ no pricing_model on the wire.
    expect("pricing_model" in w).toBe(false);
  });

  it("encodes a Bermudan with its explicit date set + LSM controls", () => {
    const dates = [0.25, 0.5, 0.75, 1];
    const w = instrumentToWire(
      americanInstrument(
        PAIR,
        EXPIRY,
        10,
        american({
          optionType: "CALL",
          strike: 1.2,
          exerciseStyle: "BERMUDAN",
          bermudanDates: dates,
          lsmPaths: 50000,
          lsmExerciseDates: 64,
          lsmSeed: 7n,
        }),
      ),
    );
    const body = w["american"] as Record<string, unknown>;
    expect(body["option_type"]).toBe(e.optionType.toWire("CALL"));
    expect(body["exercise_style"]).toBe(e.exerciseStyle.toWire("BERMUDAN"));
    expect(body["exercise_style"]).toBe(1);
    expect(body["bermudan_dates"]).toEqual(dates);
    expect(body["lsm_paths"]).toBe(50000);
    expect(body["lsm_exercise_dates"]).toBe(64);
    expect(body["lsm_seed"]).toBe(7n);
  });

  it("the encoded bermudan_dates array is a copy (the seed builder does not alias)", () => {
    const dates = [0.5, 1];
    const inst = americanInstrument(
      PAIR,
      EXPIRY,
      10,
      american({ exerciseStyle: "BERMUDAN", bermudanDates: dates }),
    );
    if (inst.product.kind !== "american") throw new Error("expected american");
    expect(inst.product.american.bermudanDates).not.toBe(dates);
    expect(inst.product.american.bermudanDates).toEqual(dates);
  });
});

// ---------------------------------------------------------------------------
// offline binomial pricer — genuine early exercise, published-value oracle
// ---------------------------------------------------------------------------

describe("PC-AMERICAN offline pricer — genuine binomial early-exercise value", () => {
  it("matches the Longstaff-Schwartz (2001) Table-1 published American put (2.314)", () => {
    // Hand-pinned PUBLISHED value (the same one the Rust server gate pins): the FD
    // reference is 2.314. The binomial tree is an INDEPENDENT scheme, so reproducing
    // it cross-validates the method rather than echoing the server.
    const out = priceInstrument(
      americanInstrument(PAIR, EXPIRY, 10, american({ optionType: "PUT", strike: 40 })),
      LS_MARKET,
    );
    const PUBLISHED = 2.314;
    expect(Math.abs(out.greeks.price - PUBLISHED)).toBeLessThan(1e-2);
    // A binomial (FD-class) tree carries no Monte-Carlo error ⇒ no std-error.
    expect(out.priceStdError).toBeUndefined();
    // resolvedStrike echoes the absolute strike.
    expect(out.resolvedStrike).toBe(40);
  });

  it("an American put is worth strictly MORE than its European twin (early-exercise premium)", () => {
    const amr = priceInstrument(
      americanInstrument(PAIR, EXPIRY, 10, american({ optionType: "PUT", strike: 40 })),
      LS_MARKET,
    );
    const euro = europeanPut(LS_MARKET, 40, EXPIRY); // INDEPENDENT in-test oracle
    // The early-exercise right has positive value for a put with r>0 ⇒ amr > euro.
    expect(amr.greeks.price).toBeGreaterThan(euro);
    // And the premium is a real, material amount (not float noise).
    expect(amr.greeks.price - euro).toBeGreaterThan(1e-3);
    // It is still bounded above by the strike (a put can never be worth more).
    expect(amr.greeks.price).toBeLessThan(40);
  });

  it("an American call with NO carry equals the European call (never exercise early)", () => {
    // With r_for = 0 (no dividend) an American CALL is never optimally exercised
    // early, so it collapses to the European value — the classic Merton result.
    const market: MarketContext = { spot: 40, vol: 0.2, rDom: 0.06, rFor: 0 };
    const amr = priceInstrument(
      americanInstrument(PAIR, EXPIRY, 10, american({ optionType: "CALL", strike: 40 })),
      market,
    );
    // INDEPENDENT European call via put-call parity off the in-test put oracle:
    // C = P + S·e^{-r_f T} − K·e^{-r_d T}.
    const put = europeanPut(market, 40, EXPIRY);
    const euroCall =
      put + market.spot * Math.exp(-market.rFor * EXPIRY) - 40 * Math.exp(-market.rDom * EXPIRY);
    expect(Math.abs(amr.greeks.price - euroCall)).toBeLessThan(1e-2);
  });
});

// ---------------------------------------------------------------------------
// Bermudan endpoints — a single date at expiry = European; dense set → American
// ---------------------------------------------------------------------------

describe("PC-AMERICAN Bermudan endpoints — single-date European, dense-set American", () => {
  it("a Bermudan with ONLY the expiry date equals the European value", () => {
    // A single exercise opportunity at T is exactly a European option.
    const berm = priceInstrument(
      americanInstrument(
        PAIR,
        EXPIRY,
        10,
        american({ optionType: "PUT", strike: 40, exerciseStyle: "BERMUDAN", bermudanDates: [1] }),
      ),
      LS_MARKET,
    );
    const euro = europeanPut(LS_MARKET, 40, EXPIRY); // INDEPENDENT oracle
    expect(Math.abs(berm.greeks.price - euro)).toBeLessThan(1e-2);
  });

  it("a dense Bermudan date set converges UP toward the American value, and is monotone in dates", () => {
    const amrPrice = priceInstrument(
      americanInstrument(PAIR, EXPIRY, 10, american({ optionType: "PUT", strike: 40 })),
      LS_MARKET,
    ).greeks.price;

    const bermudanPut = (n: number): number => {
      const dates: number[] = [];
      for (let k = 1; k <= n; k += 1) dates.push(k / n);
      return priceInstrument(
        americanInstrument(
          PAIR,
          EXPIRY,
          10,
          american({ optionType: "PUT", strike: 40, exerciseStyle: "BERMUDAN", bermudanDates: dates }),
        ),
        LS_MARKET,
      ).greeks.price;
    };

    const few = bermudanPut(4);
    const many = bermudanPut(50);
    const euro = europeanPut(LS_MARKET, 40, EXPIRY);
    // More exercise opportunities ⇒ more valuable, bounded above by the American.
    expect(few).toBeGreaterThanOrEqual(euro - 1e-9);
    expect(many).toBeGreaterThanOrEqual(few - 1e-6);
    expect(many).toBeLessThanOrEqual(amrPrice + 1e-6);
    // A dense Bermudan is close to the continuous-exercise American.
    expect(Math.abs(many - amrPrice)).toBeLessThan(5e-2);
  });
});
