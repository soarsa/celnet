/**
 * TicketWorkspace multi-asset basket builder (PC-BASKET) — the GUI ticket end of
 * the ONE `celnet.wire` contract. The Rust slice added a correlated multi-asset
 * `basket` product arm at proto field 25 (after `american`=24) — APPENDED
 * additively: no `schema_version`, no renumber, existing arms byte-identical
 * (GUIDE.md rule 9). The handoff `basketProducts.test.ts` already pins the wire
 * codec (enum tags + field names); THIS suite pins the ticket's PRICING behaviour
 * through the public data surface (`src/data/seed.ts` + `src/data/pricing.ts`)
 * with NO server and NO mocks.
 *
 * A basket / best-of / worst-of is a genuinely multi-asset Monte-Carlo product
 * (Cholesky-correlated GBM): the offline build prices a real antithetic correlated
 * terminal MC and carries an HONEST `priceStdError` (the W2 wire field), and
 * defers the multi-asset Greek strip (zeroed) rather than faking it. The numerics
 * are INDEPENDENTLY oracle'd:
 *  - Lesson c: a HAND-PINNED 2-asset Levy (1992) lognormal moment-matched basket
 *    call, recomputed in-test from a SEPARATE closed form (never calling the MC
 *    under test) AND frozen as a constant — the same independent oracle the Rust
 *    `celnet-parity` basket row uses;
 *  - structural ordering (worst ≤ basket ≤ best for a positive-weight call), the
 *    degenerate single-distinct-leg → Garman-Kohlhagen vanilla limit (against an
 *    in-test BS, no call into pricing.ts), and the comonotone ρ→1 collapse;
 *  - a non positive-definite correlation is REJECTED (the server's
 *    `NotPositiveDefinite`), never silently regularised.
 *
 * The offline MC uses the deterministic seeded RNG, so a fixed seed reproduces
 * bit-for-bit — the gates below are stable, not flaky.
 */
import { describe, expect, it } from "vitest";

import { instrumentToWire } from "../src/data/wsCodec";
import * as e from "../src/data/enums";
import { basketInstrument, type BasketTerms } from "../src/data/seed";
import { priceInstrument } from "../src/data/pricing";
import type { CcyPair, MarketContext } from "../src/data/contract";

const SETTLE: CcyPair = { base: "EUR", quote: "USD" };
const EUR: CcyPair = { base: "EUR", quote: "USD" };
const GBP: CcyPair = { base: "GBP", quote: "USD" };
const EXPIRY = 1;

/** The shared domestic/numeraire rate the basket prices under (Levy reference r_d=0.02). */
const MARKET: MarketContext = { spot: 1.0768, vol: 0.0755, rDom: 0.02, rFor: 0.0218 };

/** The hand-pinned 2-asset Levy reference legs (see PINNED_LEVY below). */
function refTerms(over: Partial<BasketTerms> = {}): BasketTerms {
  return {
    legs: [
      { pair: EUR, weight: 0.5, spot: 1.1, vol: 0.11, rFor: 0.015 },
      { pair: GBP, weight: 0.5, spot: 1.27, vol: 0.13, rFor: 0.02 },
    ],
    correlations: [1.0, 0.4, 0.4, 1.0],
    optionType: "CALL",
    strike: 1.18,
    kind: "BASKET",
    mcPaths: 8192,
    mcReplications: 16,
    mcSteps: 1,
    mcSeed: 0xc0ffeen,
    ...over,
  };
}

// ---------------------------------------------------------------------------
// independent oracles (NO call into pricing.ts)
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

/** A European Garman-Kohlhagen CALL value, computed independently in-test. */
function europeanCall(m: MarketContext, strike: number, t: number): number {
  const sqrtT = Math.sqrt(t);
  const d1 = (Math.log(m.spot / strike) + (m.rDom - m.rFor + 0.5 * m.vol * m.vol) * t) / (m.vol * sqrtT);
  const d2 = d1 - m.vol * sqrtT;
  const dfFor = Math.exp(-m.rFor * t);
  const dfDom = Math.exp(-m.rDom * t);
  return m.spot * dfFor * normCdf(d1) - strike * dfDom * normCdf(d2);
}

/**
 * Independent Levy (1992) lognormal moment-matched basket CALL — a SEPARATE
 * closed form (no MC, no call into pricing.ts). Forward of leg a is
 * `F_a = S_a·exp((r_d − r_f,a)T)`; basket forward `Fb = Σ w_a F_a`; second moment
 * `M2 = Σ_a Σ_b w_a w_b F_a F_b exp(ρ_ab σ_a σ_b T)`; matched vol
 * `σ_b = √(ln(M2/Fb²)/T)`; then Black-76 on `(Fb, K, σ_b, T)` discounted at
 * `e^{−r_d T}`. Source: E. Levy, "Pricing European average rate currency options,"
 * J. Int. Money & Finance 11 (1992).
 */
function levyBasketCall(terms: BasketTerms, m: MarketContext, t: number): number {
  const legs = terms.legs;
  const n = legs.length;
  const fwd = legs.map((l) => l.spot * Math.exp((m.rDom - l.rFor) * t));
  const fb = legs.reduce((acc, l, i) => acc + l.weight * fwd[i]!, 0);
  let m2 = 0;
  for (let a = 0; a < n; a += 1) {
    for (let b = 0; b < n; b += 1) {
      const rho = terms.correlations[a * n + b]!;
      m2 += legs[a]!.weight * legs[b]!.weight * fwd[a]! * fwd[b]! * Math.exp(rho * legs[a]!.vol * legs[b]!.vol * t);
    }
  }
  const sigma = Math.sqrt(Math.log(m2 / (fb * fb)) / t);
  const df = Math.exp(-m.rDom * t);
  const sq = sigma * Math.sqrt(t);
  const d1 = (Math.log(fb / terms.strike) + 0.5 * sigma * sigma * t) / sq;
  const d2 = d1 - sq;
  return df * (fb * normCdf(d1) - terms.strike * normCdf(d2));
}

/** The frozen Lesson-c reference (the same constant the Rust parity row pins). */
const PINNED_LEVY = 0.0508837560;

// ---------------------------------------------------------------------------
// (a) the hand-pinned Levy reference — recompute + frozen constant
// ---------------------------------------------------------------------------

describe("basket — Lesson-c hand-pinned Levy reference", () => {
  it("recomputes the independent Levy basket call to the frozen constant", () => {
    const terms = refTerms();
    const levy = levyBasketCall(terms, MARKET, EXPIRY);
    // The in-test closed form reproduces the frozen pin (Acklam/A-S normCdf gives
    // ~2e-8 vs the libm reference — well inside this band).
    expect(Math.abs(levy - PINNED_LEVY)).toBeLessThan(1e-6);
  });

  it("the offline correlated MC matches Levy within its approximation band + MC stderr", () => {
    const out = priceInstrument(basketInstrument(SETTLE, EXPIRY, 1, refTerms()), MARKET);
    expect(out.priceStdError).toBeDefined();
    const se = out.priceStdError!;
    const levy = levyBasketCall(refTerms(), MARKET, EXPIRY);
    // Levy is an APPROXIMATION (a moment-matched lognormal), not an MC-precision
    // claim: gate within its documented ~0.5% relative band PLUS 4·MC stderr.
    const band = 0.005 * levy + 4 * se;
    expect(Math.abs(out.greeks.price - levy)).toBeLessThan(band);
  });
});

// ---------------------------------------------------------------------------
// (b) honest Monte-Carlo std-error + deferred Greeks
// ---------------------------------------------------------------------------

/**
 * Unit-weight legs with a strike near a single underlying — the natural in-the-
 * money regime for BEST_OF / WORST_OF (whose aggregate is one weighted leg, not
 * the basket sum), so all three kinds price meaningfully positive.
 */
function rainbowTerms(kind: "BASKET" | "BEST_OF" | "WORST_OF"): BasketTerms {
  return {
    legs: [
      { pair: EUR, weight: 1, spot: 1.1, vol: 0.11, rFor: 0.015 },
      { pair: GBP, weight: 1, spot: 1.27, vol: 0.13, rFor: 0.02 },
    ],
    correlations: [1.0, 0.4, 0.4, 1.0],
    optionType: "CALL",
    strike: 1.0,
    kind,
    mcPaths: 8192,
    mcReplications: 16,
    mcSteps: 1,
    mcSeed: 0xc0ffeen,
  };
}

describe("basket — Monte-Carlo std-error and deferred Greek strip", () => {
  it("carries a positive priceStdError (the W2 wire field) for every kind", () => {
    for (const kind of ["BASKET", "BEST_OF", "WORST_OF"] as const) {
      const out = priceInstrument(basketInstrument(SETTLE, EXPIRY, 1, rainbowTerms(kind)), MARKET);
      expect(out.priceStdError).toBeDefined();
      expect(out.priceStdError!).toBeGreaterThan(0);
      expect(out.greeks.price).toBeGreaterThan(0);
    }
  });

  it("zeroes the deferred multi-asset Greek strip (not faked)", () => {
    const out = priceInstrument(basketInstrument(SETTLE, EXPIRY, 1, refTerms()), MARKET);
    expect(out.greeks.deltaSpot).toBe(0);
    expect(out.greeks.vega).toBe(0);
    expect(out.greeks.gamma).toBe(0);
    expect(out.greeks.theta).toBe(0);
  });

  it("is bit-reproducible for a fixed seed", () => {
    const a = priceInstrument(basketInstrument(SETTLE, EXPIRY, 1, refTerms()), MARKET);
    const b = priceInstrument(basketInstrument(SETTLE, EXPIRY, 1, refTerms()), MARKET);
    expect(a.greeks.price).toBe(b.greeks.price);
    expect(a.priceStdError).toBe(b.priceStdError);
  });
});

// ---------------------------------------------------------------------------
// (c) structural ordering worst ≤ basket ≤ best (positive-weight call)
// ---------------------------------------------------------------------------

describe("basket — rainbow ordering and limits", () => {
  it("orders worst-of ≤ best-of ≤ basket-sum for a positive-weight call", () => {
    // For equal positive weights the aggregate LEVELS satisfy
    // `min ≤ max ≤ Σ`, so the call price on the aggregate inherits that order
    // (the basket SUM is the deepest in-the-money). The handoff parity row pins
    // the same ordering server-side.
    const price = (kind: "BASKET" | "BEST_OF" | "WORST_OF"): number =>
      priceInstrument(basketInstrument(SETTLE, EXPIRY, 1, rainbowTerms(kind)), MARKET).greeks.price;
    const worst = price("WORST_OF");
    const best = price("BEST_OF");
    const basket = price("BASKET");
    // Allow a small MC-noise slack on the boundary orderings (each ~stderr).
    const slack = 3e-3;
    expect(worst).toBeLessThanOrEqual(best + slack);
    expect(best).toBeLessThanOrEqual(basket + slack);
    expect(worst).toBeLessThan(basket);
  });

  it("comonotone (ρ→1) shrinks the best-of/worst-of spread vs the decorrelated case", () => {
    // With ρ≈1 the legs move together, so worst-of and best-of nearly coincide;
    // the spread is strictly smaller than in the decorrelated (ρ=0) case.
    const spread = (rho: number): number => {
      const best = priceInstrument(
        basketInstrument(SETTLE, EXPIRY, 1, { ...rainbowTerms("BEST_OF"), correlations: [1, rho, rho, 1] }),
        MARKET,
      ).greeks.price;
      const worst = priceInstrument(
        basketInstrument(SETTLE, EXPIRY, 1, { ...rainbowTerms("WORST_OF"), correlations: [1, rho, rho, 1] }),
        MARKET,
      ).greeks.price;
      return best - worst;
    };
    expect(spread(0.999)).toBeLessThan(spread(0.0));
  });

  it("a single distinct underlying (degenerate basket) recovers the GK vanilla", () => {
    // One leg of weight 1 ⇒ the aggregate is exactly that leg's S(T); the basket
    // call collapses to the Garman-Kohlhagen vanilla on that underlying.
    const legMkt: MarketContext = { ...MARKET, spot: 1.1, vol: 0.11, rFor: 0.015 };
    const terms: BasketTerms = {
      legs: [{ pair: EUR, weight: 1, spot: 1.1, vol: 0.11, rFor: 0.015 }],
      correlations: [1.0],
      optionType: "CALL",
      strike: 1.1,
      kind: "BASKET",
      mcPaths: 200_000,
      mcReplications: 1,
      mcSteps: 1,
      mcSeed: 0x1234n,
    };
    const out = priceInstrument(basketInstrument(SETTLE, EXPIRY, 1, terms), MARKET);
    const gk = europeanCall(legMkt, 1.1, EXPIRY);
    // Single-asset MC vs the closed-form GK: within a few MC stderr.
    expect(Math.abs(out.greeks.price - gk)).toBeLessThan(5 * out.priceStdError!);
  });
});

// ---------------------------------------------------------------------------
// (d) non positive-definite correlation is rejected, never regularised
// ---------------------------------------------------------------------------

describe("basket — correlation admissibility", () => {
  it("throws on a non positive-definite correlation matrix", () => {
    // ρ = 1.5 is outside [−1, 1] ⇒ the matrix is not SPD (a non-positive pivot).
    const terms = refTerms({ correlations: [1.0, 1.5, 1.5, 1.0] });
    expect(() => priceInstrument(basketInstrument(SETTLE, EXPIRY, 1, terms), MARKET)).toThrow(
      /positive-definite/,
    );
  });

  it("throws on a correlation matrix of the wrong length", () => {
    const terms = refTerms({ correlations: [1.0, 0.4, 0.4] });
    expect(() => priceInstrument(basketInstrument(SETTLE, EXPIRY, 1, terms), MARKET)).toThrow();
  });

  it("accepts a valid 3-leg equicorrelation matrix and prices with a std-error", () => {
    const rho = 0.3;
    const terms: BasketTerms = {
      legs: [
        { pair: EUR, weight: 0.4, spot: 1.1, vol: 0.11, rFor: 0.015 },
        { pair: GBP, weight: 0.3, spot: 1.27, vol: 0.13, rFor: 0.02 },
        { pair: { base: "AUD", quote: "USD" }, weight: 0.3, spot: 0.66, vol: 0.09, rFor: 0.04 },
      ],
      correlations: [1, rho, rho, rho, 1, rho, rho, rho, 1],
      optionType: "CALL",
      strike: 1.0,
      kind: "BASKET",
      mcPaths: 8192,
      mcReplications: 8,
      mcSteps: 1,
      mcSeed: 0xfeedn,
    };
    const out = priceInstrument(basketInstrument(SETTLE, EXPIRY, 1, terms), MARKET);
    expect(out.greeks.price).toBeGreaterThan(0);
    expect(out.priceStdError!).toBeGreaterThan(0);
  });
});

// ---------------------------------------------------------------------------
// (e) the ticket-built instrument encodes the EXACT proto field-25 wire shape
// ---------------------------------------------------------------------------

describe("basket — ticket instrument → wire (proto field-25 contract)", () => {
  it("encodes legs[]/correlations[] at the pinned field names + numeric enum tags", () => {
    const wire = instrumentToWire(basketInstrument(SETTLE, EXPIRY, 1, refTerms({ kind: "WORST_OF" }))) as Record<
      string,
      unknown
    >;
    const basket = wire["basket"] as Record<string, unknown>;
    expect(basket).toBeDefined();
    const legs = basket["legs"] as Array<Record<string, unknown>>;
    expect(legs).toHaveLength(2);
    expect(legs[0]).toEqual({ pair: EUR, weight: 0.5, spot: 1.1, vol: 0.11, r_for: 0.015 });
    expect(basket["correlations"]).toEqual([1.0, 0.4, 0.4, 1.0]);
    expect(basket["kind"]).toBe(e.basketKind.toWire("WORST_OF"));
    expect(basket["option_type"]).toBe(e.optionType.toWire("CALL"));
    expect(basket["strike"]).toBe(1.18);
    expect(basket["mc_paths"]).toBe(8192);
    // Additive append: the settlement pair is preserved and no other arm appears.
    expect(wire["pair"]).toEqual(EUR);
    expect(wire["american"]).toBeUndefined();
    expect(wire["vanilla"]).toBeUndefined();
  });
});
