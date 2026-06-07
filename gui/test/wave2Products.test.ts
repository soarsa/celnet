/**
 * Wave-2 product parity for the GUI end of the ONE `celnet.wire` contract: the
 * forward-start vanilla, cliquet/ratchet and quanto are encoded onto the SAME
 * `Instrument.product` oneof as vanilla/strategy, with the EXACT appended wire
 * field NAMES (`forward_start`/`cliquet`/`quanto`, proto field numbers 16/17/18 —
 * no schema_version, no renumber; CLAUDE.md rule 9), and the standalone build
 * prices each with a genuine closed form (or, for a clamped cliquet, an honest
 * antithetic Monte-Carlo that reports a standard error) whose documented limits
 * are validated here against an independent in-test oracle.
 *
 * These exercise the REAL `src/data/wsCodec.ts`, `src/data/enums.ts`,
 * `src/data/seed.ts` and `src/data/pricing.ts` through their public surface with
 * NO server and NO mocks. The MC standard error is round-tripped through the same
 * `Quote.priceStdError` / `PriceResponse.price_std_error` representation the live
 * server uses (presence-tracked: set only for the MC product).
 */
import { describe, expect, it } from "vitest";

import { instrumentToWire, quoteFromWire } from "../src/data/wsCodec";
import * as e from "../src/data/enums";
import {
  cliquetInstrument,
  forwardStartInstrument,
  isPlainCliquet,
  quantoInstrument,
  type CliquetTerms,
  type ForwardStartTerms,
  type QuantoTerms,
} from "../src/data/seed";
import { priceInstrument } from "../src/data/pricing";
import type { CcyPair, Instrument, MarketContext } from "../src/data/contract";

const PAIR: CcyPair = { base: "EUR", quote: "USD" };
const MKT: MarketContext = { spot: 1.1, vol: 0.2, rDom: 0.03, rFor: 0.01 };

// ---------------------------------------------------------------------------
// wire encoding — the appended oneof arms, by their exact proto field NAMES
// ---------------------------------------------------------------------------

describe("wave-2 products — instrumentToWire oneof arms", () => {
  it("encodes a forward-start under `forward_start` (field 16) with numeric enum", () => {
    const terms: ForwardStartTerms = { optionType: "PUT", moneyness: 1.05, reset: 0.25 };
    const w = instrumentToWire(forwardStartInstrument(PAIR, 1, 10, terms));
    expect(w["forward_start"]).toEqual({
      // OptionType CALL=0, PUT=1.
      option_type: e.optionType.toWire("PUT"),
      moneyness: 1.05,
      reset: 0.25,
    });
    // A oneof carries exactly one body.
    expect(w["vanilla"]).toBeUndefined();
    expect(w["cliquet"]).toBeUndefined();
    expect(w["quanto"]).toBeUndefined();
  });

  it("encodes a plain cliquet under `cliquet` (field 17) with clamps OMITTED", () => {
    const terms: CliquetTerms = {
      optionType: "CALL",
      moneyness: 1,
      periods: 4,
      mcPairs: 0,
      mcSeed: 7n,
    };
    const w = instrumentToWire(cliquetInstrument(PAIR, 1, 10, terms));
    const body = w["cliquet"] as Record<string, unknown>;
    expect(body["option_type"]).toBe(e.optionType.toWire("CALL"));
    expect(body["moneyness"]).toBe(1);
    expect(body["periods"]).toBe(4);
    expect(body["mc_pairs"]).toBe(0);
    expect(body["mc_seed"]).toBe(7n);
    // Presence-tracked clamps are OMITTED when unset (proto3 optional).
    expect("local_cap" in body).toBe(false);
    expect("local_floor" in body).toBe(false);
    expect("global_cap" in body).toBe(false);
    expect("global_floor" in body).toBe(false);
  });

  it("encodes a clamped cliquet's present local cap/floor at fields 5/4", () => {
    const terms: CliquetTerms = {
      optionType: "CALL",
      moneyness: 1,
      periods: 4,
      localCap: 0.05,
      localFloor: 0.0,
      mcPairs: 5000,
      mcSeed: 42n,
    };
    const w = instrumentToWire(cliquetInstrument(PAIR, 1, 10, terms));
    const body = w["cliquet"] as Record<string, unknown>;
    expect(body["local_cap"]).toBe(0.05);
    expect(body["local_floor"]).toBe(0.0);
    expect(body["mc_pairs"]).toBe(5000);
    expect(body["mc_seed"]).toBe(42n);
  });

  it("encodes a quanto under `quanto` (field 18) with the QuantoPayoff enum", () => {
    const terms: QuantoTerms = {
      payoff: "DIGITAL",
      optionType: "PUT",
      strike: 1.15,
      conversionVol: 0.12,
      correlation: -0.3,
    };
    const w = instrumentToWire(quantoInstrument(PAIR, 1, 10, terms));
    expect(w["quanto"]).toEqual({
      // QUANTO_PAYOFF_VANILLA=0, QUANTO_PAYOFF_DIGITAL=1.
      payoff: e.quantoPayoff.toWire("DIGITAL"),
      option_type: e.optionType.toWire("PUT"),
      strike: 1.15,
      conversion_vol: 0.12,
      correlation: -0.3,
    });
  });

  it("pins the canonical QuantoPayoff enum numbers the server decodes by", () => {
    expect(e.quantoPayoff.toWire("VANILLA")).toBe(0);
    expect(e.quantoPayoff.toWire("DIGITAL")).toBe(1);
    expect(e.quantoPayoff.fromWire(0)).toBe("VANILLA");
    expect(e.quantoPayoff.fromWire(1)).toBe("DIGITAL");
  });

  it("carries the trader-facing tenor alongside the product body", () => {
    const inst: Instrument = forwardStartInstrument(PAIR, 0.5, 10, {
      optionType: "CALL",
      moneyness: 1,
      reset: 0.1,
    });
    const w = instrumentToWire(inst);
    expect(w["pair"]).toEqual({ base: "EUR", quote: "USD" });
    expect(w["expiry_years"]).toBe(0.5);
  });
});

// ---------------------------------------------------------------------------
// MC standard-error representation — Quote.priceStdError ↔ price_std_error (=7)
// ---------------------------------------------------------------------------

describe("wave-2 — price_std_error wire round-trip", () => {
  it("decodes a present price_std_error onto Quote.priceStdError (MC product)", () => {
    const q = quoteFromWire({
      quote_id: 1,
      idempotency_key: "k",
      price: { bid: 0.1, offer: 0.12 },
      greeks: {},
      conventions: {},
      resolved_strike: 1.1,
      epoch_nanos: 0,
      valid_until_nanos: 0,
      price_std_error: 0.000_42,
    });
    expect(q.priceStdError).toBeCloseTo(0.000_42, 12);
  });

  it("leaves Quote.priceStdError undefined when absent (closed-form product)", () => {
    const q = quoteFromWire({
      quote_id: 1,
      idempotency_key: "k",
      price: { bid: 0.1, offer: 0.12 },
      greeks: {},
      conventions: {},
      resolved_strike: 1.1,
      epoch_nanos: 0,
      valid_until_nanos: 0,
    });
    expect(q.priceStdError).toBeUndefined();
  });
});

// ---------------------------------------------------------------------------
// offline pricer — closed-form limits vs an independent in-test oracle
// ---------------------------------------------------------------------------

/** Garman-Kohlhagen vanilla as an INDEPENDENT oracle for the degenerate limits. */
function gkVanilla(isCall: boolean, strike: number, m: MarketContext, t: number): number {
  const sqrtT = Math.sqrt(t);
  const dfFor = Math.exp(-m.rFor * t);
  const dfDom = Math.exp(-m.rDom * t);
  const d1 = (Math.log(m.spot / strike) + (m.rDom - m.rFor + 0.5 * m.vol * m.vol) * t) / (m.vol * sqrtT);
  const d2 = d1 - m.vol * sqrtT;
  const cdf = (x: number): number => {
    // Abramowitz-Stegun 7.1.26 erf complement (independent of the pricer's CDF).
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

describe("wave-2 products — forward-start closed-form limits", () => {
  it("a t₁→0 reset reduces EXACTLY to the GK vanilla struck at m·S₀", () => {
    const t = 1;
    const m = 1.05;
    const inst = forwardStartInstrument(PAIR, t, 10, {
      optionType: "CALL",
      moneyness: m,
      reset: 1e-9,
    });
    const fs = priceInstrument(inst, MKT).greeks.price;
    const vanilla = gkVanilla(true, m * MKT.spot, MKT, t);
    // At t₁=0 the strike fixes at m·S₀ immediately ⇒ a plain GK vanilla; the
    // residual ~1e-7 is only the pricer CDF vs this oracle's A&S erf.
    expect(fs).toBeCloseTo(vanilla, 6);
  });

  it("a forward-start CALL has positive value and no MC std error", () => {
    const inst = forwardStartInstrument(PAIR, 1, 10, {
      optionType: "CALL",
      moneyness: 1,
      reset: 0.25,
    });
    const out = priceInstrument(inst, MKT);
    expect(out.greeks.price).toBeGreaterThan(0);
    // A closed-form product never claims a Monte-Carlo standard error.
    expect(out.priceStdError).toBeUndefined();
  });

  it("the reset is clamped into [0, expiry] (reset > T degenerates safely)", () => {
    const inst = forwardStartInstrument(PAIR, 0.5, 10, {
      optionType: "CALL",
      moneyness: 1,
      reset: 2, // > expiry ⇒ clamped to T ⇒ zero residual ⇒ unit intrinsic = 0 for m=1
    });
    const out = priceInstrument(inst, MKT);
    expect(out.greeks.price).toBeCloseTo(0, 12);
  });
});

describe("wave-2 products — cliquet (plain = Σ legs; clamped = MC + stderr)", () => {
  function plainTerms(periods: number): CliquetTerms {
    return { optionType: "CALL", moneyness: 1, periods, mcPairs: 0, mcSeed: 1n };
  }

  it("a plain ratchet equals the EXACT sum of its forward-start legs", () => {
    const t = 1;
    const n = 4;
    const cliq = priceInstrument(cliquetInstrument(PAIR, t, 10, plainTerms(n)), MKT).greeks.price;
    // Independent recompute: the sum of the n forward-start legs over the equal
    // schedule (reset t_{k-1} = (k-1)·T/n, expiry t_k = k·T/n).
    let sum = 0;
    for (let k = 1; k <= n; k += 1) {
      const reset = (t * (k - 1)) / n;
      const expiry = (t * k) / n;
      const leg = forwardStartInstrument(PAIR, expiry, 10, {
        optionType: "CALL",
        moneyness: 1,
        reset,
      });
      sum += priceInstrument(leg, MKT).greeks.price;
    }
    expect(cliq).toBeCloseTo(sum, 10);
  });

  it("a plain ratchet reports NO Monte-Carlo std error", () => {
    const out = priceInstrument(cliquetInstrument(PAIR, 1, 10, plainTerms(4)), MKT);
    expect(out.priceStdError).toBeUndefined();
  });

  it("isPlainCliquet flips to false once a local clamp is set", () => {
    expect(isPlainCliquet(plainTerms(4))).toBe(true);
    expect(isPlainCliquet({ ...plainTerms(4), localCap: 0.05 })).toBe(false);
    expect(isPlainCliquet({ ...plainTerms(4), localFloor: 0.01 })).toBe(false);
  });

  it("a clamped cliquet is MC-priced and surfaces a POSITIVE std error", () => {
    const terms: CliquetTerms = {
      ...plainTerms(4),
      localCap: 0.05,
      mcPairs: 5000,
      mcSeed: 12345n,
    };
    const out = priceInstrument(cliquetInstrument(PAIR, 1, 10, terms), MKT);
    expect(out.priceStdError).toBeDefined();
    expect(out.priceStdError!).toBeGreaterThan(0);
    expect(out.greeks.price).toBeGreaterThan(0);
  });

  it("the clamped MC is reproducible bit-for-bit at a fixed seed", () => {
    const terms: CliquetTerms = {
      ...plainTerms(4),
      localCap: 0.05,
      mcPairs: 4000,
      mcSeed: 99n,
    };
    const a = priceInstrument(cliquetInstrument(PAIR, 1, 10, terms), MKT);
    const b = priceInstrument(cliquetInstrument(PAIR, 1, 10, terms), MKT);
    expect(b.greeks.price).toBe(a.greeks.price);
    expect(b.priceStdError).toBe(a.priceStdError);
  });

  it("a tighter local cap strictly lowers the price (structural)", () => {
    const base: CliquetTerms = { ...plainTerms(4), mcPairs: 40_000, mcSeed: 2024n };
    const loose = priceInstrument(
      cliquetInstrument(PAIR, 1, 10, { ...base, localCap: 0.06 }),
      MKT,
    ).greeks.price;
    const tight = priceInstrument(
      cliquetInstrument(PAIR, 1, 10, { ...base, localCap: 0.02 }),
      MKT,
    ).greeks.price;
    // Common random numbers (same seed/paths) ⇒ a tighter cap caps each path's
    // return no higher, so the estimator is strictly lower.
    expect(tight).toBeLessThan(loose);
  });
});

describe("wave-2 products — quanto closed-form limits", () => {
  it("a ρ=0 quanto vanilla collapses EXACTLY to the plain GK vanilla", () => {
    const strike = 1.1;
    const q = priceInstrument(
      quantoInstrument(PAIR, 1, 10, {
        payoff: "VANILLA",
        optionType: "CALL",
        strike,
        conversionVol: 0.15,
        correlation: 0,
      }),
      MKT,
    ).greeks.price;
    const vanilla = gkVanilla(true, strike, MKT, 1);
    // At ρ=0 the −ρ·σ_S·σ_Z drift adjustment vanishes ⇒ the plain GK vanilla; the
    // residual ~1e-7 is only the pricer CDF vs this oracle's A&S erf.
    expect(q).toBeCloseTo(vanilla, 6);
  });

  it("a positive correlation LOWERS a quanto call (negative drift adjustment)", () => {
    const strike = 1.1;
    const make = (rho: number): number =>
      priceInstrument(
        quantoInstrument(PAIR, 1, 10, {
          payoff: "VANILLA",
          optionType: "CALL",
          strike,
          conversionVol: 0.2,
          correlation: rho,
        }),
        MKT,
      ).greeks.price;
    // −ρ·σ_S·σ_Z: ρ>0 lowers the carry ⇒ lowers a call; ρ<0 raises it.
    expect(make(0.6)).toBeLessThan(make(0));
    expect(make(-0.6)).toBeGreaterThan(make(0));
  });

  it("a quanto cash-or-nothing digital prices in (0, e^{-r_dom·T}) and carries no stderr", () => {
    const out = priceInstrument(
      quantoInstrument(PAIR, 1, 10, {
        payoff: "DIGITAL",
        optionType: "CALL",
        strike: 1.1,
        conversionVol: 0.1,
        correlation: 0.2,
      }),
      MKT,
    );
    const df = Math.exp(-MKT.rDom * 1);
    expect(out.greeks.price).toBeGreaterThan(0);
    expect(out.greeks.price).toBeLessThan(df);
    expect(out.priceStdError).toBeUndefined();
  });

  it("a digital put + digital call equal the discounted unit (cash-or-nothing parity)", () => {
    const strike = 1.1;
    const call = priceInstrument(
      quantoInstrument(PAIR, 1, 10, {
        payoff: "DIGITAL",
        optionType: "CALL",
        strike,
        conversionVol: 0.1,
        correlation: 0.2,
      }),
      MKT,
    ).greeks.price;
    const put = priceInstrument(
      quantoInstrument(PAIR, 1, 10, {
        payoff: "DIGITAL",
        optionType: "PUT",
        strike,
        conversionVol: 0.1,
        correlation: 0.2,
      }),
      MKT,
    ).greeks.price;
    // A cash-or-nothing call pays on S>K, the put on S<K; together they pay one
    // unit in every state ⇒ the discounted unit e^{-r_dom·T}.
    expect(call + put).toBeCloseTo(Math.exp(-MKT.rDom * 1), 10);
  });
});
