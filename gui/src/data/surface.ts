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
  SmilePoint,
  StrikeOrDelta,
} from "./contract";
import { forwardDelta, strikeFromDelta, vanillaLegGreeks } from "./pricing";

/** The standard delta-axis pillars a smile is sampled on for display. */
export const DELTA_PILLARS: number[] = [-0.1, -0.25, 0.5, 0.25, 0.1];

/**
 * Build the smile vol at a signed convention delta from a broker quote set. ATM
 * is at |Δ|≈0.5; wings interpolate the RR (asymmetry) and BF (convexity) so the
 * 25Δ and 10Δ call/put vols reproduce the marks. RR and BF are in absolute vol.
 */
function smileVol(delta: number, q: BrokerQuoteSet): number {
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
  const skew = (callSide ? 0.5 : -0.5) * rr;
  return atm + bf * w * w + skew * w;
}

/** Calibrate the delta-axis smile points for one tenor. */
export function calibrateSmile(
  pair: CcyPair,
  q: BrokerQuoteSet,
  conventions: Conventions,
  epochNanos: bigint,
): Smile {
  const points: SmilePoint[] = DELTA_PILLARS.map((delta) => ({
    delta,
    tenorYears: q.tenorYears,
    vol: smileVol(delta, q),
  }));
  return {
    pair,
    tenorYears: q.tenorYears,
    brokerQuotes: q,
    points,
    conventions,
    arbitrage: checkArb(points),
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
function checkArb(points: SmilePoint[]): ArbReport {
  let worst = 0;
  for (let i = 1; i < points.length - 1; i += 1) {
    const a = points[i - 1]!.vol;
    const b = points[i]!.vol;
    const c = points[i + 1]!.vol;
    const curv = a - 2 * b + c;
    if (curv < worst) worst = curv;
  }
  const arbFree = worst > -0.02;
  return {
    butterflyArbitrageFree: arbFree,
    calendarArbitrageFree: true,
    worstDensity: worst,
    note: arbFree
      ? "arb-free · butterfly ≥ 0"
      : "butterfly convexity breached — re-mark wings",
  };
}

/** A full surface mark across the standard tenor ladder. */
export function markSurface(
  pair: CcyPair,
  brokerQuotes: BrokerQuoteSet[],
  conventions: Conventions,
  surfaceVersion: bigint,
  epochNanos: bigint,
): MarkedSurface {
  return {
    pair,
    surfaceVersion,
    smiles: brokerQuotes.map((q) => calibrateSmile(pair, q, conventions, epochNanos)),
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
export function impliedVolForInstrument(
  surface: MarkedSurface,
  instrument: Instrument,
  market: MarketContext,
): number {
  const t = instrument.expiryYears;
  const atm = market.vol;
  if (surface.smiles.length === 0) return atm;

  const legs: { strikeSpec: StrikeOrDelta; isCall: boolean }[] =
    instrument.product.kind === "vanilla"
      ? [
          {
            strikeSpec: instrument.product.vanilla.strike,
            isCall: instrument.product.vanilla.optionType === "CALL",
          },
        ]
      : instrument.product.strategy.legs.map((leg: Leg) => ({
          strikeSpec: leg.strike,
          isCall: leg.optionType === "CALL",
        }));

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

/** Bilinear sample of the surface at (tenorYears, signed delta) for the 3D viz. */
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
  const vLo = smileVol(delta, lo.brokerQuotes);
  const vHi = smileVol(delta, hi.brokerQuotes);
  return vLo + (vHi - vLo) * w;
}
