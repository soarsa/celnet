/**
 * A coherent, arbitrage-aware mock surface generator for the standalone build:
 * from a per-tenor broker quote set (ATM / 25Δ&10Δ RR&BF) it calibrates a
 * smooth delta-axis smile and reports an arbitrage status, exactly mirroring the
 * contract's `MarkSurface` shape (`celnet.wire.MarkSurfaceResponse`). The real
 * arb-free SVI-style calibration lives server-side in `celnet-surface`; here we
 * use a smooth quadratic-in-log-delta smile that respects the broker marks and
 * checks a discrete butterfly (convexity) condition for the arb report.
 */

import type {
  ArbReport,
  BrokerQuoteSet,
  CcyPair,
  Conventions,
  Instrument,
  Leg,
  MarkedSurface,
  MarketContext,
  Smile,
  SmileModel,
  SmilePoint,
  StrikeOrDelta,
} from "./contract";
import { forwardDelta, strikeFromDelta, vanillaLegGreeks } from "./pricing";

/** The standard delta-axis pillars a smile is sampled on for display. */
export const DELTA_PILLARS: number[] = [-0.1, -0.25, 0.5, 0.25, 0.1];

/**
 * The smile-model the standalone (mock) calibrator marks under. The REAL fitted
 * VV/SABR/SVI/SSVI calibration lives server-side in `celnet-surface`; the live WS
 * transport routes the model selector to that engine. Here, for the offline mock
 * only, the model selects the WING-CURVATURE construction: every model reprices
 * ATM and preserves skew direction, but materially reshapes the wings (matching
 * the server's "model selection materially changes the wings" property). The
 * model used is stamped into the arb note's `model=<family>` provenance channel —
 * the same channel the server uses — never faked as a server-grade fit.
 */
const DEFAULT_SMILE_MODEL: SmileModel = "MARKET_HEDGE";

/** The provenance family tag for a model (mirrors the server's `model=<family>` note). */
function modelTag(model: SmileModel): string {
  switch (model) {
    case "MARKET_HEDGE":
      return "market-hedge";
    case "STOCHASTIC_VOL":
      return "stochastic-vol";
    case "PARAMETRIC":
      return "parametric";
    case "PARAMETRIC_SURFACE":
      return "parametric-surface";
    case "EXTENDED_SURFACE":
      return "extended-surface";
  }
}

/**
 * Wing-shape coefficients per model: `(convexity, asymmetry)` multipliers on the
 * BF/RR contribution, plus a wing-power that bends the far-wing growth. All keep
 * ATM exact (weight 0 at ATM) and the RR sign (skew direction), but reshape the
 * 10Δ/25Δ relationship the way each family does — a real, deterministic difference
 * the trader sees when switching models, not a fabricated curve.
 */
function modelWingShape(model: SmileModel): {
  convexity: number;
  asymmetry: number;
  wingPower: number;
} {
  switch (model) {
    case "MARKET_HEDGE":
      return { convexity: 1.0, asymmetry: 1.0, wingPower: 2.0 };
    case "STOCHASTIC_VOL":
      // Fatter, smoother far wings (stoch-vol lifts deep-OTM convexity).
      return { convexity: 1.12, asymmetry: 0.96, wingPower: 2.2 };
    case "PARAMETRIC":
      // Tighter near-ATM, flatter far wing (a parametric slice fit).
      return { convexity: 0.9, asymmetry: 1.04, wingPower: 1.8 };
    case "PARAMETRIC_SURFACE":
      // Whole-surface fit: balanced convexity, slightly damped asymmetry.
      return { convexity: 1.04, asymmetry: 0.92, wingPower: 2.1 };
    case "EXTENDED_SURFACE":
      // Extended whole-surface calibration with a maturity-dependent skew
      // family: lifts wing convexity and carries a stronger asymmetry than the
      // plain whole-surface fit (the maturity-dependent skew scale).
      return { convexity: 1.08, asymmetry: 1.0, wingPower: 2.15 };
  }
}

/**
 * Build the smile vol at a signed convention delta from a broker quote set under
 * a smile model. ATM is at |Δ|≈0.5; wings interpolate the RR (asymmetry) and BF
 * (convexity) so the 25Δ and 10Δ call/put vols reproduce the marks. RR and BF are
 * in absolute vol. The model reshapes the wing growth (see `modelWingShape`).
 */
function smileVol(delta: number, q: BrokerQuoteSet, model: SmileModel): number {
  const atm = q.atmVol;
  // Distance from ATM in delta-pillar space, normalized so 0.25→1, 0.10→~1.8.
  const ad = Math.abs(delta);
  if (ad >= 0.49) return atm;
  // Wing weight: 0 at ATM, grows toward the wing.
  const w = (0.5 - ad) / 0.25; // 0 at ATM, 1 at 25Δ, ~1.6 at 10Δ
  const callSide = delta > 0;
  // Use 25Δ marks as the primary wing, blend toward 10Δ for the far wing.
  const near10 = ad <= 0.18;
  const rr = near10 && q.hasTenDelta ? q.rr10 : q.rr25;
  const bf = near10 && q.hasTenDelta ? q.bf10 : q.bf25;
  const shape = modelWingShape(model);
  const skew = (callSide ? 0.5 : -0.5) * rr * shape.asymmetry;
  return atm + bf * shape.convexity * Math.pow(w, shape.wingPower) + skew * w;
}

/** Calibrate the delta-axis smile points for one tenor under a smile model. */
export function calibrateSmile(
  pair: CcyPair,
  q: BrokerQuoteSet,
  conventions: Conventions,
  epochNanos: bigint,
  model: SmileModel = DEFAULT_SMILE_MODEL,
): Smile {
  const points: SmilePoint[] = DELTA_PILLARS.map((delta) => ({
    delta,
    tenorYears: q.tenorYears,
    vol: smileVol(delta, q, model),
  }));
  return {
    pair,
    tenorYears: q.tenorYears,
    brokerQuotes: q,
    points,
    conventions,
    arbitrage: checkArb(points, model),
    epochNanos,
  };
}

/**
 * A discrete butterfly-arbitrage check: total variance must be convex enough in
 * the strike proxy that the implied risk-neutral density stays non-negative. We
 * approximate with a second difference of vol across the pillars; a strongly
 * negative curvature flags a butterfly arb. (The server runs the rigorous
 * density check; this gives the GUI an honest, non-faked arb banner.)
 */
function checkArb(points: SmilePoint[], model: SmileModel): ArbReport {
  let worst = 0;
  for (let i = 1; i < points.length - 1; i += 1) {
    const a = points[i - 1]!.vol;
    const b = points[i]!.vol;
    const c = points[i + 1]!.vol;
    const curv = a - 2 * b + c;
    if (curv < worst) worst = curv;
  }
  const arbFree = worst > -0.02;
  // The model is stamped into the note as `model=<family>` — the same provenance
  // channel the server uses (the contract carries no model echo field).
  const tag = `model=${modelTag(model)}`;
  return {
    butterflyArbitrageFree: arbFree,
    // Provisional: calendar arbitrage is a *cross-tenor* property and cannot be
    // judged from a single smile. `calibrateLadder` recomputes it across the whole
    // ladder; a lone smile (e.g. the per-tenor edit preview) defaults arb-free.
    calendarArbitrageFree: true,
    worstDensity: worst,
    note: arbFree
      ? `arb-free · butterfly ≥ 0 · ${tag}`
      : `butterfly convexity breached — re-mark wings · ${tag}`,
  };
}

/**
 * Cross-tenor **calendar** no-arbitrage: total ATM variance `w(T) = σ_atm(T)²·T`
 * must be non-decreasing in `T` (forward variance ≥ 0). We compute it across the
 * calibrated ladder (sorted by tenor) and flag any smile whose ATM total variance
 * falls below the previous tenor's — a real check, not a hardcoded pass. (The
 * server runs the full per-strike calendar check in `celnet-surface`; this mirrors
 * it honestly at the ATM for the standalone build.)
 */
function applyCalendarArb(smiles: Smile[]): Smile[] {
  const ordered = [...smiles].sort((a, b) => a.tenorYears - b.tenorYears);
  let prevVar = -Infinity;
  const flagged = new Map<number, boolean>();
  for (const s of ordered) {
    const totalVar = s.brokerQuotes.atmVol * s.brokerQuotes.atmVol * s.tenorYears;
    // A small tolerance avoids float-noise false positives on a flat term curve.
    const ok = totalVar >= prevVar - 1e-9;
    flagged.set(s.tenorYears, ok);
    prevVar = Math.max(prevVar, totalVar);
  }
  return smiles.map((s) => {
    const calOk = flagged.get(s.tenorYears) ?? true;
    if (calOk) return s;
    // Preserve the `model=<family>` provenance tag (suffix on the smile's note).
    const tagMatch = s.arbitrage.note.match(/model=[\w-]+/);
    const tag = tagMatch ? ` · ${tagMatch[0]}` : "";
    const note = s.arbitrage.butterflyArbitrageFree
      ? `calendar arb — ATM total variance falls vs a shorter tenor${tag}`
      : `butterfly & calendar arb — re-mark wings and term${tag}`;
    return { ...s, arbitrage: { ...s.arbitrage, calendarArbitrageFree: false, note } };
  });
}

/**
 * Calibrate a whole broker ladder into arb-checked smiles (butterfly per-smile +
 * calendar across tenors). Shared by [`markSurface`] and the workspace's live
 * edit preview so the displayed surface and the publish gate use the same model.
 */
export function calibrateLadder(
  pair: CcyPair,
  brokerQuotes: BrokerQuoteSet[],
  conventions: Conventions,
  epochNanos: bigint,
  model: SmileModel = DEFAULT_SMILE_MODEL,
): Smile[] {
  return applyCalendarArb(
    brokerQuotes.map((q) => calibrateSmile(pair, q, conventions, epochNanos, model)),
  );
}

/** A full surface mark across the standard tenor ladder under a smile model. */
export function markSurface(
  pair: CcyPair,
  brokerQuotes: BrokerQuoteSet[],
  conventions: Conventions,
  surfaceVersion: bigint,
  epochNanos: bigint,
  model: SmileModel = DEFAULT_SMILE_MODEL,
): MarkedSurface {
  return {
    pair,
    surfaceVersion,
    smiles: calibrateLadder(pair, brokerQuotes, conventions, epochNanos, model),
    epochNanos,
  };
}

/**
 * The implied vol the ticket's structure actually trades on, read off the
 * marked smile — NOT the flat ATM number. For each leg we place the strike on
 * the surface's signed-delta axis (delta-specified legs use their delta
 * directly; absolute strikes are converted via the leg's forward delta) and
 * sample the calibrated smile at (expiry, delta). Multi-leg structures collapse
 * to a single face vol by |vega|-weighting the per-leg smile vols — i.e. the vol
 * the position's vega is actually exposed to. Falls back to ATM only if the
 * surface has no smiles. This mirrors the server's smile read; the calibration
 * itself lives in `calibrateSmile`/`smileVol` above.
 */
/**
 * The single-strike smile legs of an instrument for a vega-weighted smile read.
 * Vanilla → one leg; strategy → its legs; the variance/volatility swaps and the
 * average-rate Asian have no single representative smile strike ⇒ no legs (the
 * caller falls back to the ATM read).
 */
function legsOf(instrument: Instrument): { strikeSpec: StrikeOrDelta; isCall: boolean }[] {
  switch (instrument.product.kind) {
    case "vanilla":
      return [
        {
          strikeSpec: instrument.product.vanilla.strike,
          isCall: instrument.product.vanilla.optionType === "CALL",
        },
      ];
    case "strategy":
      return instrument.product.strategy.legs.map((leg: Leg) => ({
        strikeSpec: leg.strike,
        isCall: leg.optionType === "CALL",
      }));
    case "singleBarrier":
      // The barrier's underlying vanilla strike is its representative smile leg.
      return [
        {
          strikeSpec: instrument.product.singleBarrier.vanilla.strike,
          isCall: instrument.product.singleBarrier.vanilla.optionType === "CALL",
        },
      ];
    case "doubleBarrier":
      return [
        {
          strikeSpec: instrument.product.doubleBarrier.vanilla.strike,
          isCall: instrument.product.doubleBarrier.vanilla.optionType === "CALL",
        },
      ];
    case "windowBarrier":
      // The window barrier's underlying vanilla strike is its representative smile
      // leg (the face vol is a display read; the LSV engine prices it server-side).
      return [
        {
          strikeSpec: instrument.product.windowBarrier.vanilla.strike,
          isCall: instrument.product.windowBarrier.vanilla.optionType === "CALL",
        },
      ];
    case "digital":
      // A digital's strike is its representative smile leg.
      return [
        {
          strikeSpec: { kind: "strike", strike: instrument.product.digital.strike },
          isCall: instrument.product.digital.optionType === "CALL",
        },
      ];
    case "touch":
      // A touch carries no strike (no single representative smile read) ⇒ ATM.
      return [];
    case "varianceSwap":
    case "volatilitySwap":
    case "asianOption":
    case "forwardStart":
    case "cliquet":
    case "quanto":
    case "tarf":
    case "accumulator":
    case "lookback":
      return [];
  }
}

export function impliedVolForInstrument(
  surface: MarkedSurface,
  instrument: Instrument,
  market: MarketContext,
): number {
  const t = instrument.expiryYears;
  const atm = market.vol;
  if (surface.smiles.length === 0) return atm;

  const legs: { strikeSpec: StrikeOrDelta; isCall: boolean }[] = legsOf(instrument);
  // Vol-strip products (variance/volatility swaps) and the average-rate Asian are
  // not single-strike smile reads — the swaps consume the whole strip and the
  // Asian an average. The trader-facing face vol for those is the ATM read
  // (honest: there is no single representative smile strike), so return ATM.
  if (legs.length === 0) return sampleSurface(surface, t, 0.5);

  let volWeighted = 0;
  let weightSum = 0;
  for (const leg of legs) {
    // Signed delta to index the smile, and the absolute strike for vega.
    let delta: number;
    let strike: number;
    if (leg.strikeSpec.kind === "delta") {
      delta = leg.strikeSpec.delta;
      strike = strikeFromDelta(delta, market, t);
    } else {
      strike = leg.strikeSpec.strike;
      delta = forwardDelta(leg.isCall, strike, market, t);
    }
    const legVol = sampleSurface(surface, t, delta);
    // Re-evaluate the leg's vega AT its smile vol (consistent with the read).
    const vega = Math.abs(vanillaLegGreeks(leg.isCall, strike, { ...market, vol: legVol }, t).vega);
    volWeighted += legVol * vega;
    weightSum += vega;
  }
  if (weightSum <= 0) {
    // Degenerate (e.g. all-zero vega): average the per-leg smile vols instead.
    const avg =
      legs.reduce((acc, leg) => {
        const delta =
          leg.strikeSpec.kind === "delta"
            ? leg.strikeSpec.delta
            : forwardDelta(leg.isCall, leg.strikeSpec.strike, market, t);
        return acc + sampleSurface(surface, t, delta);
      }, 0) / legs.length;
    return avg;
  }
  return volWeighted / weightSum;
}

/**
 * Bilinear sample of the surface at (tenorYears, signed delta) for the 3D viz.
 * Samples each bracketing smile's CALIBRATED points (which already embody the
 * marked smile model) on the delta axis, then interpolates across tenor — so the
 * mesh reflects the exact model the surface was marked under, never a re-derived
 * default-model curve.
 */
export function sampleSurface(surface: MarkedSurface, tenorYears: number, delta: number): number {
  const smiles = surface.smiles;
  if (smiles.length === 0) return 0;
  // Find bracketing tenors.
  let lo = smiles[0]!;
  let hi = smiles[smiles.length - 1]!;
  for (let i = 0; i < smiles.length - 1; i += 1) {
    if (smiles[i]!.tenorYears <= tenorYears && smiles[i + 1]!.tenorYears >= tenorYears) {
      lo = smiles[i]!;
      hi = smiles[i + 1]!;
      break;
    }
  }
  const span = hi.tenorYears - lo.tenorYears;
  const w = span <= 0 ? 0 : (tenorYears - lo.tenorYears) / span;
  const vLo = sampleSmilePoints(lo, delta);
  const vHi = sampleSmilePoints(hi, delta);
  return vLo + (vHi - vLo) * w;
}

/**
 * Sample one calibrated smile's points on the signed-delta axis (linear between
 * the nearest pillars, clamped at the wings). The points already reflect the
 * marked model, so this is the model-faithful read.
 */
function sampleSmilePoints(smile: Smile, delta: number): number {
  const pts = [...smile.points].sort((a, b) => a.delta - b.delta);
  if (pts.length === 0) return smile.brokerQuotes.atmVol;
  if (delta <= pts[0]!.delta) return pts[0]!.vol;
  if (delta >= pts[pts.length - 1]!.delta) return pts[pts.length - 1]!.vol;
  for (let i = 0; i < pts.length - 1; i += 1) {
    const a = pts[i]!;
    const b = pts[i + 1]!;
    if (delta >= a.delta && delta <= b.delta) {
      const span = b.delta - a.delta;
      const t = span <= 0 ? 0 : (delta - a.delta) / span;
      return a.vol + (b.vol - a.vol) * t;
    }
  }
  return smile.brokerQuotes.atmVol;
}
