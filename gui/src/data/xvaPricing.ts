/**
 * A deterministic, analytic in-browser XVA pricer used ONLY by the standalone
 * in-app source so the GUI produces real, internally-consistent counterparty
 * valuation adjustments (CVA / DVA / FVA) WITHOUT a server. It is the XVA analogue
 * of `src/data/pricing.ts` / `src/data/ratesPricing.ts`: the authoritative f64
 * pricing lives server-side in `celnet-xva` (`cva.rs` / `exposure.rs` /
 * `netting.rs` / `survival.rs`); this is a presentation-side stand-in that the
 * transport seam (`src/data/transport`) replaces with the live `price_xva` RPC
 * when wired.
 *
 * It is a GENUINE computation, not a stub — it reproduces the server's XVA
 * AGGREGATION bit-for-bit (`celnet_xva::compute_xva`): the discounted expected
 * exposure at each time bucket integrated against each party's marginal default
 * probability (from its hazard-rate survival curve) scaled by its LGD, plus the
 * funding spread on the net expected exposure over the joint-survival measure. The
 * netting-set marks reproduce the server's Garman-Kohlhagen FX-vanilla forward
 * value (`celnet_vanilla::price`) and the survival curves reproduce
 * `SurvivalCurve::{flat,piecewise}::cumulative_hazard` exactly.
 *
 * HONEST BOUNDARY — the one deliberate difference from the edge: the server
 * estimates the per-bucket expected exposure by SCRAMBLED-SOBOL Monte-Carlo over
 * `paths` paths (`ExposureProfile::simulate`), which is not reproducible bit-for-bit
 * in the browser. This module instead computes the SAME expectation by a
 * deterministic quadrature over the lognormal spot (a midpoint rule on the CDF —
 * the deterministic analogue of the low-discrepancy fill the server samples), so an
 * offline price CONVERGES to the live edge as both estimators refine but is not
 * byte-identical to a finite-path MC run. This is the standard offline stance for an
 * MC product (mirroring the GUI's other MC surfaces), stated plainly, never faked.
 *
 * No method/person/vendor names appear in identifiers (GUIDE.md rule 8); the
 * mathematical provenance is documented here, never in API names.
 */

import type { XvaPricingRequest, XvaResult, XvaSurvivalCurve } from "./contract";

/** A typed failure of the offline XVA pricer (mirrors the server's `XvaPriceError`). */
export class XvaPricingError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "XvaPricingError";
  }
}

// ---------------------------------------------------------------------------
// standard-normal helpers (dependency-free, deterministic)
// ---------------------------------------------------------------------------

const INV_SQRT_2PI = 0.398_942_280_401_432_7;

/** Standard-normal PDF `φ(x)`. */
function phi(x: number): number {
  return INV_SQRT_2PI * Math.exp(-0.5 * x * x);
}

/**
 * Standard-normal CDF via a high-accuracy rational approximation (Cody-style erf
 * complement), identical to `src/data/pricing.ts` so the netting-set marks agree
 * with the GUI's other closed-form pricers to floating-point precision.
 */
function normCdf(x: number): number {
  const t = 1 / (1 + 0.231_641_9 * Math.abs(x));
  const d = 0.319_381_53;
  const poly =
    t *
    (d +
      t *
        (-0.356_563_782 +
          t * (1.781_477_937 + t * (-1.821_255_978 + t * 1.330_274_429))));
  const cnd = 1 - phi(x) * poly;
  return x >= 0 ? cnd : 1 - cnd;
}

/**
 * Inverse standard-normal CDF `Φ⁻¹(u)` for `u ∈ (0, 1)` (Acklam's rational
 * approximation, ~1e-9 relative accuracy). Maps a uniform CDF sample to a spot
 * shock exactly as the server's exposure simulator maps its low-discrepancy points
 * (`inv_norm_cdf(u)`).
 */
function invNormCdf(u: number): number {
  // Rational approximation coefficients (Acklam).
  const a = [
    -3.969_683_028_665_376e1, 2.209_460_984_245_205e2,
    -2.759_285_104_469_687e2, 1.383_577_518_672_69e2,
    -3.066_479_806_614_716e1, 2.506_628_277_459_239,
  ];
  const b = [
    -5.447_609_879_822_406e1, 1.615_858_368_580_409e2,
    -1.556_989_798_598_866e2, 6.680_131_188_771_972e1,
    -1.328_068_155_288_572e1,
  ];
  const c = [
    -7.784_894_002_430_293e-3, -3.223_964_580_411_365e-1,
    -2.400_758_277_161_838, -2.549_732_539_343_734, 4.374_664_141_464_968,
    2.938_163_982_698_783,
  ];
  const d = [
    7.784_695_709_041_462e-3, 3.224_671_290_700_398e-1,
    2.445_134_137_142_996, 3.754_408_661_907_416,
  ];
  const pLow = 0.024_25;
  const pHigh = 1 - pLow;
  if (u <= 0) return -Infinity;
  if (u >= 1) return Infinity;
  if (u < pLow) {
    const q = Math.sqrt(-2 * Math.log(u));
    return (
      (((((c[0]! * q + c[1]!) * q + c[2]!) * q + c[3]!) * q + c[4]!) * q + c[5]!) /
      ((((d[0]! * q + d[1]!) * q + d[2]!) * q + d[3]!) * q + 1)
    );
  }
  if (u <= pHigh) {
    const q = u - 0.5;
    const r = q * q;
    return (
      ((((((a[0]! * r + a[1]!) * r + a[2]!) * r + a[3]!) * r + a[4]!) * r + a[5]!) *
        q) /
      (((((b[0]! * r + b[1]!) * r + b[2]!) * r + b[3]!) * r + b[4]!) * r + 1)
    );
  }
  const q = Math.sqrt(-2 * Math.log(1 - u));
  return (
    -(((((c[0]! * q + c[1]!) * q + c[2]!) * q + c[3]!) * q + c[4]!) * q + c[5]!) /
    ((((d[0]! * q + d[1]!) * q + d[2]!) * q + d[3]!) * q + 1)
  );
}

// ---------------------------------------------------------------------------
// netting-set marks — the Garman-Kohlhagen FX-vanilla forward value
// ---------------------------------------------------------------------------

/** One validated trade of the netting set (internal, post-validation). */
interface Trade {
  readonly isCall: boolean;
  readonly strike: number;
  readonly expiry: number;
  readonly vol: number;
  readonly notional: number;
}

/**
 * The Garman-Kohlhagen FX-vanilla forward value of one unit-notional option,
 * reproducing `celnet_vanilla::price`. Returns 0 once the option has expired
 * (`tau ≤ 0`), matching `NettedTrade::mark`.
 */
function gkVanillaValue(
  isCall: boolean,
  spot: number,
  strike: number,
  vol: number,
  tau: number,
  rDom: number,
  rFor: number,
): number {
  if (tau <= 0) return 0;
  const sqrtT = Math.sqrt(tau);
  const dfFor = Math.exp(-rFor * tau);
  const dfDom = Math.exp(-rDom * tau);
  const d1 =
    (Math.log(spot / strike) + (rDom - rFor + 0.5 * vol * vol) * tau) /
    (vol * sqrtT);
  const d2 = d1 - vol * sqrtT;
  return isCall
    ? spot * dfFor * normCdf(d1) - strike * dfDom * normCdf(d2)
    : strike * dfDom * normCdf(-d2) - spot * dfFor * normCdf(-d1);
}

/** The netting-set value at `(tObs, spot)` — the signed sum of trade marks. */
function netValue(
  trades: readonly Trade[],
  tObs: number,
  spot: number,
  rDom: number,
  rFor: number,
): number {
  let acc = 0;
  for (const tr of trades) {
    acc +=
      tr.notional *
      gkVanillaValue(tr.isCall, spot, tr.strike, tr.vol, tr.expiry - tObs, rDom, rFor);
  }
  return acc;
}

// ---------------------------------------------------------------------------
// survival (hazard-rate) curves — reproduces `celnet_xva::SurvivalCurve`
// ---------------------------------------------------------------------------

/** A validated survival curve: parallel segment-end times + per-segment hazards. */
interface SurvivalCurve {
  /** Segment end times (years); a single `Infinity` for the flat curve. */
  readonly pillars: readonly number[];
  /** Per-segment constant hazard rates. */
  readonly hazards: readonly number[];
}

/**
 * Decode + validate an `XvaSurvivalCurve` exactly as the server's
 * `decode_survival_curve`: a FLAT curve is an empty `pillarTimes` with exactly one
 * hazard (modelled here as a single `Infinity` pillar); a PIECEWISE curve needs
 * equal-length, strictly-increasing positive pillar times.
 */
function decodeSurvival(c: XvaSurvivalCurve, who: string): SurvivalCurve {
  if (c.hazardRates.length === 0) {
    throw new XvaPricingError(`${who} survival curve has no hazard rates`);
  }
  for (const h of c.hazardRates) {
    if (!(Number.isFinite(h) && h >= 0)) {
      throw new XvaPricingError(`${who} hazard rate must be finite and ≥ 0`);
    }
  }
  if (c.pillarTimes.length === 0) {
    if (c.hazardRates.length !== 1) {
      throw new XvaPricingError(
        `${who} flat survival curve needs exactly one hazard rate`,
      );
    }
    return { pillars: [Infinity], hazards: [c.hazardRates[0]!] };
  }
  if (c.pillarTimes.length !== c.hazardRates.length) {
    throw new XvaPricingError(
      `${who} survival curve pillar/hazard lengths must match`,
    );
  }
  let prev = 0;
  for (const p of c.pillarTimes) {
    if (!(Number.isFinite(p) && p > prev)) {
      throw new XvaPricingError(
        `${who} survival curve pillars must be strictly increasing and positive`,
      );
    }
    prev = p;
  }
  return { pillars: [...c.pillarTimes], hazards: [...c.hazardRates] };
}

/** Cumulative hazard `H(t) = ∫₀ᵗ λ(s) ds` — reproduces `cumulative_hazard`. */
function cumulativeHazard(curve: SurvivalCurve, t: number): number {
  let acc = 0;
  let lo = 0;
  for (let i = 0; i < curve.pillars.length; i += 1) {
    const p = curve.pillars[i]!;
    const h = curve.hazards[i]!;
    const hi = Math.min(p, t);
    if (hi > lo) acc += h * (hi - lo);
    lo = p;
    if (t <= p) return acc;
  }
  // Beyond the last finite pillar: hold the final hazard flat.
  const lastPillar = curve.pillars[curve.pillars.length - 1]!;
  if (Number.isFinite(lastPillar) && t > lastPillar) {
    acc += curve.hazards[curve.hazards.length - 1]! * (t - lastPillar);
  }
  return acc;
}

/** Survival probability `S(t) = e^{−H(t)}` (`= 1` at `t = 0`). */
function survival(curve: SurvivalCurve, t: number): number {
  return Math.exp(-cumulativeHazard(curve, t));
}

// ---------------------------------------------------------------------------
// exposure profile — deterministic quadrature over the lognormal spot
// ---------------------------------------------------------------------------

/**
 * Quadrature nodes for the per-bucket expected-exposure integral. A midpoint rule
 * on the CDF (`u_i = (i + ½)/N`, `z_i = Φ⁻¹(u_i)`) — the deterministic analogue of
 * the server's low-discrepancy path fill; `N = 1024` resolves the `max(V, 0)` kink
 * to well within a basis point of the true expectation for a smooth netting set.
 */
const QUADRATURE_NODES = 1024;

/** Precomputed standard-normal shocks `z_i = Φ⁻¹((i + ½)/N)`. */
const SHOCKS: readonly number[] = Array.from(
  { length: QUADRATURE_NODES },
  (_, i) => invNormCdf((i + 0.5) / QUADRATURE_NODES),
);

/**
 * Expected positive / negative exposure `(EPE, ENE ≥ 0)` of the netting set at time
 * `t`, under a single-factor lognormal spot `S(t) = S₀·exp((r_dom − r_for −
 * ½σ²)t + σ√t·z)`, computed by the CDF-midpoint quadrature. Mirrors the estimand of
 * `ExposureProfile::simulate` (the mean of `E⁺`/`E⁻` over the spot distribution).
 */
function expectedExposure(
  trades: readonly Trade[],
  t: number,
  spot0: number,
  sigma: number,
  rDom: number,
  rFor: number,
): { epe: number; ene: number } {
  if (t <= 0) {
    const v0 = netValue(trades, 0, spot0, rDom, rFor);
    return { epe: Math.max(v0, 0), ene: Math.max(-v0, 0) };
  }
  const drift = (rDom - rFor - 0.5 * sigma * sigma) * t;
  const volStep = sigma * Math.sqrt(t);
  let sumPos = 0;
  let sumNeg = 0;
  for (const z of SHOCKS) {
    const spot = spot0 * Math.exp(drift + volStep * z);
    const v = netValue(trades, t, spot, rDom, rFor);
    if (v > 0) sumPos += v;
    else if (v < 0) sumNeg += -v;
  }
  const invN = 1 / QUADRATURE_NODES;
  return { epe: sumPos * invN, ene: sumNeg * invN };
}

// ---------------------------------------------------------------------------
// the public offline pricer
// ---------------------------------------------------------------------------

/** Validate + normalise the netting set; throws exactly where the server refuses. */
function decodeTrades(req: XvaPricingRequest): Trade[] {
  if (req.trades.length === 0) {
    throw new XvaPricingError("netting set has no trades");
  }
  return req.trades.map((t) => {
    if (!(Number.isFinite(t.strike) && t.strike > 0)) {
      throw new XvaPricingError("trade strike must be finite and > 0");
    }
    if (!(Number.isFinite(t.expiryYears) && t.expiryYears > 0)) {
      throw new XvaPricingError("trade expiry must be finite and > 0");
    }
    if (!(Number.isFinite(t.vol) && t.vol > 0)) {
      throw new XvaPricingError("trade vol must be finite and > 0");
    }
    if (!Number.isFinite(t.notional)) {
      throw new XvaPricingError("trade notional must be finite");
    }
    return {
      isCall: t.optionType === "CALL",
      strike: t.strike,
      expiry: t.expiryYears,
      vol: t.vol,
      notional: t.notional,
    };
  });
}

/**
 * Price a netting set's XVA offline, returning the CVA / DVA / FVA legs + their
 * signed total. Reproduces the server's `compute_xva` aggregation over a
 * deterministic-quadrature exposure profile (see the module docstring for the
 * one honest MC-vs-quadrature boundary). Validates the request exactly as the
 * server's `price_xva` does, so an offline rejection matches a live one.
 *
 * @throws {XvaPricingError} on a malformed netting set / market / survival curve.
 */
export function computeXvaOffline(req: XvaPricingRequest): XvaResult {
  const trades = decodeTrades(req);

  if (!(Number.isFinite(req.rDom) && Number.isFinite(req.rFor))) {
    throw new XvaPricingError("rates must be finite");
  }
  if (!(Number.isFinite(req.spot0) && req.spot0 > 0)) {
    throw new XvaPricingError("spot0 must be finite and > 0");
  }
  if (!(Number.isFinite(req.sigma) && req.sigma >= 0)) {
    throw new XvaPricingError("sigma must be finite and ≥ 0");
  }
  if (!(Number.isInteger(req.paths) && req.paths >= 1)) {
    throw new XvaPricingError("paths must be a positive integer");
  }
  if (!(Number.isInteger(req.exposureSteps) && req.exposureSteps >= 1)) {
    throw new XvaPricingError("exposure steps must be a positive integer");
  }
  if (!((req.lgdCounterparty >= 0 && req.lgdCounterparty <= 1) &&
    req.lgdOwn >= 0 && req.lgdOwn <= 1)) {
    throw new XvaPricingError("LGD must be in [0, 1]");
  }
  if (!Number.isFinite(req.fundingSpread)) {
    throw new XvaPricingError("funding spread must be finite");
  }

  const counterparty = decodeSurvival(req.counterparty, "counterparty");
  const own = decodeSurvival(req.own, "own");

  // Horizon = the latest trade expiry (the set's outermost exposure date).
  let horizon = 0;
  for (const t of trades) if (t.expiry > horizon) horizon = t.expiry;

  const steps = req.exposureSteps;
  const dt = horizon / steps;
  const grid: number[] = Array.from({ length: steps + 1 }, (_, k) => k * dt);

  // Per-node EPE / ENE + discount factor (mirrors ExposureProfile fields). The
  // quadrature runs once per node; both exposures come out of the single pass.
  const profile = grid.map((t) =>
    expectedExposure(trades, t, req.spot0, req.sigma, req.rDom, req.rFor),
  );
  const epe = profile.map((p) => p.epe);
  const ene = profile.map((p) => p.ene);
  const disc = grid.map((t) => Math.exp(-req.rDom * t));

  // The aggregation loop is `celnet_xva::compute_xva`, field-for-field: interval k
  // spans (t_{k-1}, t_k]; the marginal default probability is the survival drop.
  let cva = 0;
  let dva = 0;
  let fva = 0;
  let sCPrev = survival(counterparty, grid[0]!); // = 1 at t = 0
  let sOPrev = survival(own, grid[0]!);
  for (let k = 1; k <= steps; k += 1) {
    const sC = survival(counterparty, grid[k]!);
    const sO = survival(own, grid[k]!);
    const dpC = sCPrev - sC; // counterparty marginal default prob on the interval
    const dpO = sOPrev - sO; // own marginal default prob

    cva += req.lgdCounterparty * disc[k]! * epe[k]! * dpC;
    dva += req.lgdOwn * disc[k]! * ene[k]! * dpO;

    // FVA: funding spread on the net expected exposure, time-weighted over the
    // interval, on the joint-survival measure (both parties alive at t_k).
    const stepDt = grid[k]! - grid[k - 1]!;
    const netEe = epe[k]! - ene[k]!;
    fva += req.fundingSpread * disc[k]! * netEe * stepDt * sC * sO;

    sCPrev = sC;
    sOPrev = sO;
  }

  return { cva, dva, fva, totalAdjustment: cva - dva + fva };
}
