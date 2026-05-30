/**
 * A small, deterministic Garman-Kohlhagen vanilla pricer used ONLY by the
 * standalone in-app mock/replay source so the GUI produces real, internally
 * consistent prices, Greeks, and a coherent smile without a server. The
 * authoritative f64 pricing lives server-side in `celnet-vanilla`; this is a
 * presentation-side stand-in that the transport seam (src/data/transport)
 * replaces with live RPC results when wired.
 *
 * No method/person names appear in identifiers (CLAUDE.md rule 8); the
 * Garman-Kohlhagen provenance is documented here, never in API names.
 */

import type { Greeks, Instrument, Leg, MarketContext } from "./contract";

const INV_SQRT_2PI = 0.398_942_280_401_432_7;

/** Standard normal PDF. */
function phi(x: number): number {
  return INV_SQRT_2PI * Math.exp(-0.5 * x * x);
}

/**
 * Standard normal CDF via a high-accuracy rational approximation
 * (Cody-style erf complement). Deterministic and dependency-free.
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

/** Forward rate F = S e^{(r_dom - r_for) T}. */
export function forward(m: MarketContext, t: number): number {
  return m.spot * Math.exp((m.rDom - m.rFor) * t);
}

/**
 * Resolve a signed convention delta to an absolute strike under a
 * forward-unadjusted delta convention (the mock source's working convention):
 * K = F · exp(-σ√T · N⁻¹(|Δ|) · sign + ½σ²T). Good enough to produce a coherent,
 * monotone strike ladder for display; the server resolves the exact convention.
 */
export function strikeFromDelta(
  delta: number,
  m: MarketContext,
  t: number,
): number {
  const f = forward(m, t);
  const sigmaRtT = m.vol * Math.sqrt(t);
  // Inverse normal via Acklam's algorithm (deterministic).
  const n = invNorm(Math.min(0.999, Math.max(0.001, Math.abs(delta))));
  const sign = delta >= 0 ? 1 : -1;
  return f * Math.exp(-sign * sigmaRtT * n + 0.5 * sigmaRtT * sigmaRtT);
}

/** Acklam's inverse normal CDF approximation (deterministic, ~1e-9 accuracy). */
function invNorm(p: number): number {
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
    -2.400_758_277_161_838, -2.549_732_539_343_734,
    4.374_664_141_464_968, 2.938_163_982_698_783,
  ];
  const d = [
    7.784_695_709_041_462e-3, 3.224_671_290_700_398e-1,
    2.445_134_137_142_996, 3.754_408_661_907_416,
  ];
  const plow = 0.024_25;
  const phigh = 1 - plow;
  if (p < plow) {
    const q = Math.sqrt(-2 * Math.log(p));
    return (
      (((((c[0]! * q + c[1]!) * q + c[2]!) * q + c[3]!) * q + c[4]!) * q +
        c[5]!) /
      ((((d[0]! * q + d[1]!) * q + d[2]!) * q + d[3]!) * q + 1)
    );
  }
  if (p > phigh) {
    const q = Math.sqrt(-2 * Math.log(1 - p));
    return (
      -(((((c[0]! * q + c[1]!) * q + c[2]!) * q + c[3]!) * q + c[4]!) * q +
        c[5]!) /
      ((((d[0]! * q + d[1]!) * q + d[2]!) * q + d[3]!) * q + 1)
    );
  }
  const q = p - 0.5;
  const r = q * q;
  return (
    ((((((a[0]! * r + a[1]!) * r + a[2]!) * r + a[3]!) * r + a[4]!) * r +
      a[5]!) *
      q) /
    (((((b[0]! * r + b[1]!) * r + b[2]!) * r + b[3]!) * r + b[4]!) * r + 1)
  );
}

interface LegInputs {
  isCall: boolean;
  strike: number;
  signedRatio: number; // +ratio for buy, -ratio for sell
}

function resolveLeg(
  optionType: "CALL" | "PUT",
  strikeSpec: { kind: "strike"; strike: number } | { kind: "delta"; delta: number },
  side: "BUY" | "SELL" | "TWO_WAY",
  ratio: number,
  m: MarketContext,
  t: number,
): LegInputs {
  const strike =
    strikeSpec.kind === "strike"
      ? strikeSpec.strike
      : strikeFromDelta(strikeSpec.delta, m, t);
  const sign = side === "SELL" ? -1 : 1;
  return { isCall: optionType === "CALL", strike, signedRatio: sign * ratio };
}

/**
 * The forward (driftless) delta of a single vanilla leg at a given strike,
 * signed: positive for calls, negative for puts. Used to place an absolute
 * strike on the surface's signed-delta axis when reading the smile vol.
 */
export function forwardDelta(
  isCall: boolean,
  strike: number,
  m: MarketContext,
  t: number,
): number {
  const { spot: s, vol, rDom, rFor } = m;
  const sqrtT = Math.sqrt(t);
  const d1 = (Math.log(s / strike) + (rDom - rFor + 0.5 * vol * vol) * t) / (vol * sqrtT);
  const nd1 = normCdf(d1);
  return isCall ? nd1 : nd1 - 1;
}

/** The full 14-Greek set for a single vanilla leg (per unit base notional). */
export function vanillaLegGreeks(
  isCall: boolean,
  strike: number,
  m: MarketContext,
  t: number,
): Greeks {
  return vanillaGreeks(isCall, strike, m, t);
}

/** The full 14-Greek set for a single vanilla leg (per unit base notional). */
function vanillaGreeks(
  isCall: boolean,
  strike: number,
  m: MarketContext,
  t: number,
): Greeks {
  const { spot: s, vol, rDom, rFor } = m;
  const sqrtT = Math.sqrt(t);
  const dfFor = Math.exp(-rFor * t);
  const dfDom = Math.exp(-rDom * t);
  const d1 = (Math.log(s / strike) + (rDom - rFor + 0.5 * vol * vol) * t) / (vol * sqrtT);
  const d2 = d1 - vol * sqrtT;
  const nd1 = normCdf(d1);
  const nd2 = normCdf(d2);
  const pdf = phi(d1);

  const price = isCall
    ? s * dfFor * nd1 - strike * dfDom * nd2
    : strike * dfDom * normCdf(-d2) - s * dfFor * normCdf(-d1);

  const deltaSpot = isCall ? dfFor * nd1 : -dfFor * normCdf(-d1);
  const deltaForward = isCall ? nd1 : nd1 - 1;
  const gamma = (dfFor * pdf) / (s * vol * sqrtT);
  const vega = s * dfFor * pdf * sqrtT;
  const theta =
    (-(s * dfFor * pdf * vol) / (2 * sqrtT) +
      (isCall
        ? rFor * s * dfFor * nd1 - rDom * strike * dfDom * nd2
        : -rFor * s * dfFor * normCdf(-d1) + rDom * strike * dfDom * normCdf(-d2)));
  const rhoDom = isCall
    ? strike * t * dfDom * nd2
    : -strike * t * dfDom * normCdf(-d2);
  const rhoFor = isCall ? -s * t * dfFor * nd1 : s * t * dfFor * normCdf(-d1);
  const vanna = -dfFor * pdf * (d2 / vol);
  const volga = vega * ((d1 * d2) / vol);
  const charm =
    -dfFor *
    (pdf * ((rDom - rFor) / (vol * sqrtT) - d2 / (2 * t)) -
      (isCall ? -rFor * nd1 : rFor * normCdf(-d1)));
  const speed = -(gamma / s) * (d1 / (vol * sqrtT) + 1);
  const zomma = gamma * ((d1 * d2 - 1) / vol);
  const color =
    -((dfFor * pdf) / (2 * s * t * vol * sqrtT)) *
    (2 * (rFor * t) + 1 + ((2 * (rDom - rFor) * t - d2 * vol * sqrtT) / (vol * sqrtT)) * d1);

  return {
    price,
    deltaSpot,
    deltaForward,
    gamma,
    vega,
    theta: theta / 365, // per-day display convention
    rhoDom,
    rhoFor,
    vanna,
    volga,
    charm,
    speed,
    zomma,
    color,
  };
}

function zeroGreeks(): Greeks {
  return {
    price: 0,
    deltaSpot: 0,
    deltaForward: 0,
    gamma: 0,
    vega: 0,
    theta: 0,
    rhoDom: 0,
    rhoFor: 0,
    vanna: 0,
    volga: 0,
    charm: 0,
    speed: 0,
    zomma: 0,
    color: 0,
  };
}

function addScaled(acc: Greeks, g: Greeks, w: number): Greeks {
  return {
    price: acc.price + g.price * w,
    deltaSpot: acc.deltaSpot + g.deltaSpot * w,
    deltaForward: acc.deltaForward + g.deltaForward * w,
    gamma: acc.gamma + g.gamma * w,
    vega: acc.vega + g.vega * w,
    theta: acc.theta + g.theta * w,
    rhoDom: acc.rhoDom + g.rhoDom * w,
    rhoFor: acc.rhoFor + g.rhoFor * w,
    vanna: acc.vanna + g.vanna * w,
    volga: acc.volga + g.volga * w,
    charm: acc.charm + g.charm * w,
    speed: acc.speed + g.speed * w,
    zomma: acc.zomma + g.zomma * w,
    color: acc.color + g.color * w,
  };
}

/**
 * Price an instrument (vanilla or strategy) and return the aggregated 14-Greek
 * set plus the (first-leg) resolved strike. Premium is in percent-of-foreign.
 */
export function priceInstrument(
  instrument: Instrument,
  m: MarketContext,
): { greeks: Greeks; resolvedStrike: number } {
  const t = instrument.expiryYears;
  const legs: LegInputs[] =
    instrument.product.kind === "vanilla"
      ? [
          resolveLeg(
            instrument.product.vanilla.optionType,
            instrument.product.vanilla.strike,
            instrument.side === "TWO_WAY" ? "BUY" : instrument.side,
            1,
            m,
            t,
          ),
        ]
      : instrument.product.strategy.legs.map((leg: Leg) =>
          resolveLeg(leg.optionType, leg.strike, leg.side, leg.ratio, m, t),
        );

  let acc = zeroGreeks();
  for (const leg of legs) {
    const g = vanillaGreeks(leg.isCall, leg.strike, m, t);
    acc = addScaled(acc, g, leg.signedRatio);
  }
  return { greeks: acc, resolvedStrike: legs[0]?.strike ?? m.spot };
}
