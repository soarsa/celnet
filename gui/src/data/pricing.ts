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

import type {
  Accumulator,
  AsianOption,
  Cliquet,
  Digital,
  DoubleBarrier,
  ForwardStart,
  Greeks,
  Instrument,
  Leg,
  Lookback,
  MarketContext,
  Quanto,
  SingleBarrier,
  StrikeOrDelta,
  Tarf,
  Touch,
  VarianceSwap,
  VolatilitySwap,
} from "./contract";
import { Rng } from "./rng";

/**
 * A priced result: the aggregated 14-Greek set, the resolved strike, and — for a
 * Monte-Carlo-priced product only (a clamped cliquet) — the standard error of the
 * priced value. `priceStdError` is `undefined` for every closed-form product, so
 * a caller renders an honest precision band for MC and never claims a stderr for a
 * closed form (mirrors `PriceResponse.price_std_error`, field 7).
 */
export interface PriceOutcome {
  greeks: Greeks;
  resolvedStrike: number;
  priceStdError?: number;
}

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
 * Price an instrument and return the aggregated 14-Greek set plus the resolved
 * strike. Premium is in percent-of-foreign. Vanilla/strategy aggregate the GK
 * legs; the swap and Asian products dispatch to their own closed forms below.
 */
export function priceInstrument(
  instrument: Instrument,
  m: MarketContext,
): PriceOutcome {
  const t = instrument.expiryYears;
  switch (instrument.product.kind) {
    case "singleBarrier":
      return priceSingleBarrier(instrument.product.singleBarrier, m, t);
    case "doubleBarrier":
      return priceDoubleBarrier(instrument.product.doubleBarrier, m, t);
    case "digital":
      return priceDigital(instrument.product.digital, m, t);
    case "touch":
      return priceTouch(instrument.product.touch, m, t);
    case "varianceSwap":
      return priceVarianceSwap(instrument.product.varianceSwap, m);
    case "volatilitySwap":
      return priceVolatilitySwap(instrument.product.volatilitySwap, m);
    case "asianOption":
      return priceAsian(instrument.product.asianOption, m, t);
    case "forwardStart":
      return priceForwardStart(instrument.product.forwardStart, m, t);
    case "cliquet":
      return priceCliquet(instrument.product.cliquet, m, t);
    case "quanto":
      return priceQuanto(instrument.product.quanto, m, t);
    case "tarf":
      return priceTarf(instrument.product.tarf, m, t);
    case "accumulator":
      return priceAccumulator(instrument.product.accumulator, m, t);
    case "lookback":
      return priceLookback(instrument.product.lookback, m, t);
    case "windowBarrier":
      // The window barrier has NO closed form — it is priced ONLY by the server's
      // local-stochastic-volatility ADI-PDE / Monte-Carlo engine (pricing model
      // LOCAL_STOCH_VOL). The offline mock deliberately does NOT fabricate an LSV
      // number (CLAUDE.md: no mocks/placeholders/overclaim). The TicketWorkspace
      // gates the window barrier to the live transport and never requests an
      // offline price for it, so this arm is unreachable offline; if some caller
      // does reach it, fail LOUDLY rather than invent a value.
      throw new Error(
        "window-barrier pricing is server-side only (LOCAL_STOCH_VOL); the offline mock does not price it",
      );
    case "vanilla":
    case "strategy": {
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
  }
}

// ---------------------------------------------------------------------------
// variance / volatility swaps — flat-σ fair strikes (the mock surface is flat)
// ---------------------------------------------------------------------------
//
// The standalone mock carries a single volatility per `MarketContext` (a flat
// smile). Under a flat implied-vol surface the model-free log-contract static
// replication of variance collapses **exactly** to `K_var = σ²` (the
// smile-weighted strip integrand is the single Black variance at every strike),
// and the convexity (Jensen) adjustment to the volatility strike vanishes
// because the variance of realised variance about its mean is zero for a flat
// vol — so `K_vol = √K_var = σ`. These are the EXACT closed-form limits of the
// server's `fair_variance` / `fair_volatility` (validated in celnet-exotics by
// the `*_flat_sigma_recovers_*` parity rows), NOT a placeholder. The live WS
// transport prices the genuine smile-weighted strip server-side; offline, a flat
// surface makes the strip degenerate to this exact value.

/**
 * Variance-swap fair strike on the flat-σ mock surface. The headline `price` and
 * `resolvedStrike` carry the fair VARIANCE strike `K_var = σ²` (matching the
 * server pricer, which echoes `K_var` in both); the sensitivities are the
 * sensitivity of `K_var` to the market (only vol moves it: `dK_var/dσ = 2σ`).
 */
function priceVarianceSwap(_spec: VarianceSwap, m: MarketContext): PriceOutcome {
  const kVar = m.vol * m.vol;
  const greeks = zeroGreeks();
  greeks.price = kVar;
  greeks.vega = 2 * m.vol; // dK_var/dσ
  return { greeks, resolvedStrike: kVar };
}

/**
 * Volatility-swap fair strike on the flat-σ mock surface: `K_vol = σ` (zero
 * convexity correction for a flat vol). The headline `price`/`resolvedStrike`
 * carry `K_vol`; `dK_vol/dσ = 1`.
 */
function priceVolatilitySwap(_spec: VolatilitySwap, m: MarketContext): PriceOutcome {
  const kVol = m.vol;
  const greeks = zeroGreeks();
  greeks.price = kVol;
  greeks.vega = 1; // dK_vol/dσ
  return { greeks, resolvedStrike: kVol };
}

// ---------------------------------------------------------------------------
// single / double barrier, digital, touch — reflection-principle closed forms
// ---------------------------------------------------------------------------
//
// These four products are already on the ONE `celnet.wire` contract (proto field
// numbers single_barrier=9, double_barrier=10, digital=11, touch=12) and priced
// server-side by `celnet-exotics` (barrier.rs / digital.rs / touch.rs) under
// Garman-Kohlhagen via the Reiner-Rubinstein / Ikeda-Kunitomo reflection-principle
// closed forms. The offline mock here mirrors those exact closed forms on the SAME
// GK math the mock already uses, so the standalone price is a genuine closed-form
// value (not a stub) that agrees with the server's `celnet-exotics` reference and
// satisfies the same structural identities (in/out parity, touch complementarity,
// the digital−vanilla decomposition). The live WS transport prices the genuine
// server form; offline this reproduces it. Greeks are by central finite difference
// on the closed form so the ticket's strip is populated honestly. Provenance is in
// doc comments only (never in identifiers; CLAUDE.md rule 8).

/** Resolve a `StrikeOrDelta` to an absolute strike (delta via the mock convention). */
function resolveStrikeOrDelta(spec: StrikeOrDelta, m: MarketContext, t: number): number {
  return spec.kind === "strike" ? spec.strike : strikeFromDelta(spec.delta, m, t);
}

/** Populate delta/gamma/vega of a closed-form scalar price by central FD. */
function fdGreeks(value: (mk: MarketContext) => number, m: MarketContext): Greeks {
  const greeks = zeroGreeks();
  greeks.price = value(m);
  const hS = m.spot * 1e-4;
  const hV = 1e-4;
  const bump = (over: Partial<MarketContext>): MarketContext => ({ ...m, ...over });
  greeks.deltaSpot =
    (value(bump({ spot: m.spot + hS })) - value(bump({ spot: m.spot - hS }))) / (2 * hS);
  greeks.gamma =
    (value(bump({ spot: m.spot + hS })) - 2 * greeks.price + value(bump({ spot: m.spot - hS }))) /
    (hS * hS);
  greeks.vega = (value(bump({ vol: m.vol + hV })) - value(bump({ vol: m.vol - hV }))) / (2 * hV);
  return greeks;
}

/** Plain Garman-Kohlhagen vanilla value (mirrors celnet-vanilla::price). */
function gkVanillaValue(isCall: boolean, strike: number, m: MarketContext, t: number): number {
  return vanillaGreeks(isCall, strike, m, t).price;
}

// --- single barrier (Reiner-Rubinstein A…D block selection) ----------------
//
// Mirrors `celnet-exotics::single_barrier::single_barrier_no_rebate`: the
// KNOCK-IN value is the signed A…D block combination selected by {up/down,
// call/put, K vs H}; KNOCK-OUT is derived by in/out parity (KO = vanilla − KI) so
// the identity holds by construction. The rebate leg is a one-touch (at hit) for a
// knocked-out out-option, or a no-touch on the barrier for an in-option.

/** The Reiner-Rubinstein A…D building blocks for a single barrier. */
interface RrBlocks {
  a: number;
  b: number;
  c: number;
  d: number;
}

function rrBlocks(
  isCall: boolean,
  up: boolean,
  strike: number,
  barrier: number,
  m: MarketContext,
  t: number,
): RrBlocks {
  const { spot: s, vol, rDom, rFor } = m;
  const carry = rDom - rFor;
  const vsqt = vol * Math.sqrt(t);
  const mu = carry / (vol * vol) - 0.5;
  const phi = isCall ? 1 : -1;
  const eta = up ? -1 : 1;
  const h = barrier;
  const k = strike;

  const x1 = Math.log(s / k) / vsqt + (1 + mu) * vsqt;
  const x2 = Math.log(s / h) / vsqt + (1 + mu) * vsqt;
  const y1 = Math.log((h * h) / (s * k)) / vsqt + (1 + mu) * vsqt;
  const y2 = Math.log(h / s) / vsqt + (1 + mu) * vsqt;

  const hs = h / s;
  const pow2mu2 = Math.exp(2 * (mu + 1) * Math.log(hs));
  const pow2mu = Math.exp(2 * mu * Math.log(hs));

  const sDisc = s * Math.exp(-rFor * t);
  const kDisc = k * Math.exp(-rDom * t);

  const a =
    phi * sDisc * normCdf(phi * x1) - phi * kDisc * normCdf(phi * (x1 - vsqt));
  const b =
    phi * sDisc * normCdf(phi * x2) - phi * kDisc * normCdf(phi * (x2 - vsqt));
  const c =
    phi * sDisc * pow2mu2 * normCdf(eta * y1) -
    phi * kDisc * pow2mu * normCdf(eta * (y1 - vsqt));
  const d =
    phi * sDisc * pow2mu2 * normCdf(eta * y2) -
    phi * kDisc * pow2mu * normCdf(eta * (y2 - vsqt));
  return { a, b, c, d };
}

/** The rebate-free single-barrier value (mirrors `single_barrier_no_rebate`). */
function singleBarrierBareValue(
  spec: SingleBarrier,
  strike: number,
  m: MarketContext,
  t: number,
): number {
  const isCall = spec.vanilla.optionType === "CALL";
  const up = spec.side === "UP";
  const vanilla = gkVanillaValue(isCall, strike, m, t);

  // Already-breached resolution (mirrors the server's breached branch).
  const breached = up ? m.spot >= spec.barrier : m.spot <= spec.barrier;
  if (breached) {
    return spec.kind === "KNOCK_OUT" ? 0 : vanilla;
  }

  const { a, b, c, d } = rrBlocks(isCall, up, strike, spec.barrier, m, t);
  const kGeH = strike >= spec.barrier;
  let knockIn: number;
  if (up) {
    knockIn = isCall ? (kGeH ? a : b - c + d) : kGeH ? a - b + d : c;
  } else {
    knockIn = isCall ? (kGeH ? c : a - b + d) : kGeH ? b - c + d : a;
  }
  return spec.kind === "KNOCK_IN" ? knockIn : vanilla - knockIn;
}

/** The single-barrier value INCLUDING the rebate leg (mirrors `single_barrier_price`). */
function singleBarrierValue(
  spec: SingleBarrier,
  strike: number,
  m: MarketContext,
  t: number,
): number {
  const bare = singleBarrierBareValue(spec, strike, m, t);
  if (spec.rebate === 0) return bare;
  // A knocked-out out-option pays the rebate AT HIT (one-touch); a knock-in pays
  // the rebate at expiry if it never knocks in (no-touch on the barrier).
  const reb =
    spec.kind === "KNOCK_OUT"
      ? oneTouchValue(m, spec.barrier, spec.rebate, "AT_HIT", t)
      : noTouchValue(m, spec.barrier, spec.rebate, t);
  return bare + reb;
}

function priceSingleBarrier(spec: SingleBarrier, m: MarketContext, t: number): PriceOutcome {
  const strike = resolveStrikeOrDelta(spec.vanilla.strike, m, t);
  const greeks = fdGreeks((mk) => singleBarrierValue(spec, strike, mk, t), m);
  return { greeks, resolvedStrike: strike };
}

// --- double barrier (Ikeda-Kunitomo method-of-images corridor series) -------
//
// Mirrors `celnet-exotics::barrier::double_knock_out_price`: the knock-out value is
// the alternating sum of reflected Black-Scholes asset/cash legs over the corridor
// (K,U) for a call or (L,K) for a put; a knock-in is priced by parity
// (KI = vanilla − KO), exactly as the server does.

/** Image-series term count (matches the exotics crate's `DKO_TERMS`). */
const DKO_TERMS = 10;

/**
 * The double knock-out value (mirrors `celnet-exotics::barrier::double_knock_out_price`
 * term-for-term: the Ikeda-Kunitomo method-of-images series with `μ = b/σ² − ½`,
 * `μ₁ = 2(μ+1)`, `drift = (1+μ)·σ√T`, the direct image `S·(U/L)^{2n}` and the
 * lower-wall mirror image `L^{2n+2}/(U^{2n}·S)`, summed over `±DKO_TERMS`; `φ`
 * orients every CDF argument inside the loop and the value is floored at zero).
 */
function doubleKnockOutValue(
  isCall: boolean,
  strike: number,
  lower: number,
  upper: number,
  m: MarketContext,
  t: number,
): number {
  const s = m.spot;
  if (s <= lower || s >= upper) return 0;

  const { vol, rDom, rFor } = m;
  const vsqt = vol * Math.sqrt(t);
  const mu = (rDom - rFor) / (vol * vol) - 0.5;
  const mu1 = 2 * (mu + 1); // = 2b/σ² + 1
  const k = strike;
  const dfDom = Math.exp(-rDom * t);
  const dfFor = Math.exp(-rFor * t);
  const phi = isCall ? 1 : -1;

  // Payoff integration band on the corridor: call ⇒ (K, U); put ⇒ (L, K).
  const fLo = isCall ? Math.max(k, lower) : lower;
  const fHi = isCall ? upper : Math.min(k, upper);
  if (fLo >= fHi) return 0;

  const drift = (1 + mu) * vsqt;
  const lnUl = Math.log(upper / lower);
  const lnS = Math.log(s);
  const lnL = Math.log(lower);
  const lnU = Math.log(upper);
  const pow = (b: number, ex: number): number => Math.exp(ex * Math.log(b));

  let sum = 0;
  for (let n = -DKO_TERMS; n <= DKO_TERMS; n += 1) {
    // Direct image spot S·(U/L)^{2n}.
    const directLog = lnS + 2 * n * lnUl;
    const ddLo = (directLog - Math.log(fLo)) / vsqt + drift;
    const ddHi = (directLog - Math.log(fHi)) / vsqt + drift;
    const pow1 = pow(upper / lower, n * mu1);

    // Lower-wall mirror image spot S' = L^{2n+2}/(U^{2n}·S).
    const imgLog = (2 * n + 2) * lnL - 2 * n * lnU - lnS;
    const dmLo = (imgLog - Math.log(fLo)) / vsqt + drift;
    const dmHi = (imgLog - Math.log(fHi)) / vsqt + drift;
    const mirrorBase = pow(lower, n + 1) / (pow(upper, n) * s);
    const pow2 = pow(mirrorBase, mu1);

    const asset =
      s *
      dfFor *
      (pow1 * (normCdf(phi * ddLo) - normCdf(phi * ddHi)) -
        pow2 * (normCdf(phi * dmLo) - normCdf(phi * dmHi)));

    // Cash leg: power exponent μ₁ − 2 = 2μ, arguments shifted by −σ√T.
    const pow1c = pow(upper / lower, n * (mu1 - 2));
    const pow2c = pow(mirrorBase, mu1 - 2);
    const cash =
      k *
      dfDom *
      (pow1c * (normCdf(phi * (ddLo - vsqt)) - normCdf(phi * (ddHi - vsqt))) -
        pow2c * (normCdf(phi * (dmLo - vsqt)) - normCdf(phi * (dmHi - vsqt))));

    sum += asset - cash;
  }
  return Math.max(sum, 0);
}

function priceDoubleBarrier(spec: DoubleBarrier, m: MarketContext, t: number): PriceOutcome {
  const strike = resolveStrikeOrDelta(spec.vanilla.strike, m, t);
  const isCall = spec.vanilla.optionType === "CALL";
  const knockIn = spec.kind === "KNOCK_IN";
  const value = (mk: MarketContext): number => {
    const ko = doubleKnockOutValue(isCall, strike, spec.lowerBarrier, spec.upperBarrier, mk, t);
    return knockIn ? gkVanillaValue(isCall, strike, mk, t) - ko : ko;
  };
  const greeks = fdGreeks(value, m);
  return { greeks, resolvedStrike: strike };
}

// --- digital (binary) ------------------------------------------------------
//
// Mirrors `celnet-exotics::digital::digital_price`, scaled by the payout amount
// exactly as the server does (`payout * digital_price(...)`).

function digitalUnitValue(spec: Digital, m: MarketContext, t: number): number {
  const { spot: s, vol, rDom, rFor } = m;
  const sqrtT = Math.sqrt(t);
  const d1 = (Math.log(s / spec.strike) + (rDom - rFor + 0.5 * vol * vol) * t) / (vol * sqrtT);
  const d2 = d1 - vol * sqrtT;
  const dfDom = Math.exp(-rDom * t);
  const dfFor = Math.exp(-rFor * t);
  const isCall = spec.optionType === "CALL";
  if (spec.style === "CASH_OR_NOTHING") {
    return isCall ? dfDom * normCdf(d2) : dfDom * normCdf(-d2);
  }
  // Asset-or-nothing: pays one unit of the foreign asset (worth S_T).
  return isCall ? s * dfFor * normCdf(d1) : s * dfFor * normCdf(-d1);
}

function priceDigital(spec: Digital, m: MarketContext, t: number): PriceOutcome {
  const value = (mk: MarketContext): number => spec.payout * digitalUnitValue(spec, mk, t);
  const greeks = fdGreeks(value, m);
  return { greeks, resolvedStrike: spec.strike };
}

// --- touch (one-/no-/double-no-/double-one-touch) --------------------------
//
// Mirrors `celnet-exotics::touch`: a one-touch pays AT HIT via the Reiner-
// Rubinstein discounted-hit expectation; a no-touch is the deferred complement
// `e^{−r_d T} − one_touch_at_expiry`; the double-no-touch is the corridor survival
// from the method-of-images series; the double-one-touch is its complement. Each is
// clamped exactly as the crate does. The server echoes the lower barrier as the
// resolved strike (a touch has no strike).

type RebateTiming = "AT_HIT" | "AT_EXPIRY";

/** `(H/S)^p · Φ(arg)` in log-space (no Inf×0 NaN; mirrors `pow_cdf`). */
function powCdf(lnHs: number, p: number, arg: number): number {
  const phi = normCdf(arg);
  if (phi <= 0) return 0;
  const v = Math.exp(p * lnHs + Math.log(phi));
  return Number.isFinite(v) ? v : 0;
}

/** One-touch value (mirrors `one_touch_price` / `one_touch_with_side`). */
function oneTouchValue(
  m: MarketContext,
  barrier: number,
  rebate: number,
  timing: RebateTiming,
  t: number,
): number {
  const upper = barrier >= m.spot; // upper barrier ⇒ TouchSide::Upper
  const through = upper ? m.spot >= barrier : m.spot <= barrier;
  if (through) {
    return timing === "AT_HIT" ? rebate : rebate * Math.exp(-m.rDom * t);
  }
  const { vol, rDom, rFor } = m;
  const vsqt = vol * Math.sqrt(t);
  const mu = (rDom - rFor) / (vol * vol) - 0.5;
  const z = Math.log(barrier / m.spot);
  const sideSign = upper ? -1 : 1;
  const base = (sideSign * z) / vsqt;
  const driftSign = -sideSign;

  let value: number;
  if (timing === "AT_HIT") {
    const lam = Math.sqrt(mu * mu + (2 * rDom) / (vol * vol));
    const a1 = base + driftSign * lam * vsqt;
    const a2 = base - driftSign * lam * vsqt;
    value = rebate * (powCdf(z, mu + lam, a1) + powCdf(z, mu - lam, a2));
  } else {
    const a1 = base + driftSign * mu * vsqt;
    const a2 = base - driftSign * mu * vsqt;
    const prob = normCdf(a1) + powCdf(z, 2 * mu, a2);
    value = rebate * Math.exp(-rDom * t) * prob;
  }
  const maxDf = Math.max(Math.exp(-rDom * t), 1);
  const cap = Math.max(rebate, 0) * maxDf;
  return Number.isFinite(value) ? Math.min(Math.max(value, 0), cap) : 0;
}

/** No-touch value (mirrors `no_touch_price`). */
function noTouchValue(m: MarketContext, barrier: number, rebate: number, t: number): number {
  const df = Math.exp(-m.rDom * t);
  const ot = oneTouchValue(m, barrier, rebate, "AT_EXPIRY", t);
  const cap = Math.max(rebate, 0) * Math.max(df, 1);
  return Math.min(Math.max(rebate * df - ot, 0), cap);
}

/** Single-wall hit probability (mirrors `single_wall_hit_prob`). */
function singleWallHitProb(mu: number, vsqt: number, z: number): number {
  const base = z >= 0 ? -z / vsqt : z / vsqt;
  const ds = z >= 0 ? 1 : -1;
  const a1 = base + ds * mu * vsqt;
  const a2 = base - ds * mu * vsqt;
  const p = normCdf(a1) + powCdf(z, 2 * mu, a2);
  return Math.min(Math.max(p, 0), 1);
}

/** Image reflections summed in the DNT corridor survival (matches `IMAGE_TERMS`). */
const DNT_IMAGE_TERMS = 12;

/** Corridor (double-no-touch) survival probability (mirrors `dnt_survival`). */
function dntSurvival(m: MarketContext, lower: number, upper: number, t: number): number {
  const s = m.spot;
  if (s <= lower || s >= upper) return 0;
  const { vol, rDom, rFor } = m;
  const vsqt = vol * Math.sqrt(t);
  const mu = (rDom - rFor) / (vol * vol) - 0.5;
  const sigma2t = vol * vol * t;
  const zU = Math.log(upper / s);
  const zL = Math.log(lower / s);
  const bigZ = Math.log(upper / lower);
  const drift = mu * sigma2t;

  const weighted = (logW: number, hi: number, lo: number): number => {
    const p = normCdf(hi) - normCdf(lo);
    if (p <= 0) return 0;
    const v = Math.exp(logW + Math.log(p));
    return Number.isFinite(v) ? v : 0;
  };

  let sum = 0;
  for (let n = -DNT_IMAGE_TERMS; n <= DNT_IMAGE_TERMS; n += 1) {
    const img = 2 * n * bigZ;
    const term1 = weighted(mu * img, (zU - img - drift) / vsqt, (zL - img - drift) / vsqt);
    const rimg = 2 * zU - img;
    const term2 = weighted(mu * rimg, (zU - rimg - drift) / vsqt, (zL - rimg - drift) / vsqt);
    sum += term1 - term2;
  }
  const series = Number.isFinite(sum) ? Math.min(Math.max(sum, 0), 1) : 0;
  const capUpper = 1 - singleWallHitProb(mu, vsqt, zU);
  const capLower = 1 - singleWallHitProb(mu, vsqt, zL);
  return Math.min(Math.max(Math.min(series, Math.min(capUpper, capLower)), 0), 1);
}

/** Double-no-touch value (mirrors `double_no_touch_price`). */
function doubleNoTouchValue(
  m: MarketContext,
  lower: number,
  upper: number,
  rebate: number,
  t: number,
): number {
  const df = Math.exp(-m.rDom * t);
  const surv = dntSurvival(m, lower, upper, t);
  const cap = Math.max(rebate, 0) * Math.max(df, 1);
  return Math.min(Math.max(rebate * df * surv, 0), cap);
}

/** Double-(one-)touch value (mirrors `double_touch_price`). */
function doubleTouchValue(
  m: MarketContext,
  lower: number,
  upper: number,
  rebate: number,
  t: number,
): number {
  const df = Math.exp(-m.rDom * t);
  const nt = doubleNoTouchValue(m, lower, upper, rebate, t);
  const cap = Math.max(rebate, 0) * Math.max(df, 1);
  return Math.min(Math.max(rebate * df - nt, 0), cap);
}

/** The touch value for the selected kind (mirrors the server's `Product::Touch` arm). */
function touchValue(spec: Touch, m: MarketContext, t: number): number {
  switch (spec.kind) {
    case "ONE_TOUCH":
      return oneTouchValue(m, spec.lowerBarrier, spec.rebate, "AT_HIT", t);
    case "NO_TOUCH":
      return noTouchValue(m, spec.lowerBarrier, spec.rebate, t);
    case "DOUBLE_NO_TOUCH":
      return doubleNoTouchValue(m, spec.lowerBarrier, spec.upperBarrier, spec.rebate, t);
    case "DOUBLE_ONE_TOUCH":
      return doubleTouchValue(m, spec.lowerBarrier, spec.upperBarrier, spec.rebate, t);
  }
}

function priceTouch(spec: Touch, m: MarketContext, t: number): PriceOutcome {
  const greeks = fdGreeks((mk) => touchValue(spec, mk, t), m);
  // A touch carries no strike; the headline strike echoes the lower barrier so the
  // ticket has a sensible level to display (matches the server).
  return { greeks, resolvedStrike: spec.lowerBarrier };
}

// ---------------------------------------------------------------------------
// arithmetic-average-rate Asian — two-moment lognormal matching (closed form)
// ---------------------------------------------------------------------------
//
// MC-free analytic Asian by matching the random part of the arithmetic average
// to a lognormal with the same first two moments, then pricing the average with
// the Black formula on that lognormal (Turnbull-Wakeman, 1991; provenance in
// docs only, never in identifiers). This mirrors the celnet-exotics
// `discrete_moments` / `continuous_moments` / `seasoned_match` / `black_on_average`
// closed forms field-for-field, on the SAME GK math the mock already uses, so the
// offline price is a genuine closed-form value — exact in the documented limits
// (single fixing, zero vol). The live WS transport prices the trader-selected
// method (Curran or Turnbull-Wakeman) on the server; offline the mock uses this
// two-moment analytic for both selections (an honest closed form, not a stub).

interface AverageMoments {
  /** First moment E[Ā] of the (future) average. */
  m1: number;
  /** Second moment E[Ā²] of the (future) average. */
  m2: number;
}

/** Discrete-fixing future-average moments (Σ over the future observation grid). */
function discreteMoments(m: MarketContext, t: number, tStart: number, n: number): AverageMoments {
  const b = m.rDom - m.rFor;
  const v2 = m.vol * m.vol;
  const tRem = t - tStart;
  const dt = tRem / n;
  const times: number[] = [];
  for (let k = 1; k <= n; k += 1) times.push(tStart + k * dt);
  const invN = 1 / n;

  let m1 = 0;
  for (const tk of times) m1 += Math.exp(b * tk);
  m1 *= m.spot * invN;

  let m2 = 0;
  for (let a = 0; a < times.length; a += 1) {
    const ti = times[a]!;
    for (let j = a; j < times.length; j += 1) {
      const tj = times[j]!;
      // times sorted ascending ⇒ min(ti,tj) = ti for j ≥ a.
      const term = Math.exp(b * (ti + tj) + v2 * ti);
      m2 += tj > ti ? 2 * term : term; // off-diagonal counted twice (symmetric)
    }
  }
  m2 *= m.spot * m.spot * invN * invN;
  return { m1, m2 };
}

/** Continuous-average moments over `(t_start, T]` in exact closed form. */
function continuousMoments(m: MarketContext, t: number, tStart: number): AverageMoments {
  const b = m.rDom - m.rFor;
  const v2 = m.vol * m.vol;
  const tau = t - tStart;
  const s0 = m.spot;

  const m1 =
    Math.abs(b) < 1e-12
      ? s0 * Math.exp(b * tStart)
      : (s0 * Math.exp(b * tStart) * (Math.exp(b * tau) - 1)) / (b * tau);

  const pref = s0 * s0 * Math.exp(2 * b * tStart + v2 * tStart);
  const p = b + v2;
  const q = b;
  const expm1Over = (rate: number, x: number): number =>
    Math.abs(rate) < 1e-12 ? x : (Math.exp(rate * x) - 1) / rate;
  let innerOuter: number;
  if (Math.abs(p) < 1e-12) {
    innerOuter =
      Math.abs(q) < 1e-12
        ? 0.5 * tau * tau
        : (Math.exp(q * tau) * (q * tau - 1) + 1) / (q * q);
  } else {
    innerOuter = (expm1Over(p + q, tau) - expm1Over(q, tau)) / p;
  }
  const m2 = (2 * pref * innerOuter) / (tau * tau);
  return { m1, m2 };
}

/**
 * Black-style closed form on a lognormal with forward `ex = E[X]`, second moment
 * `ex2 = E[X²]`, struck at `kEff`, discounted by `df`. Handles the deterministic
 * and below-floor strike limits exactly (mirrors `black_on_average`).
 */
function blackOnAverage(
  isCall: boolean,
  ex: number,
  ex2: number,
  kEff: number,
  df: number,
): number {
  const sign = isCall ? 1 : -1;
  const ratio = ex > 0 ? ex2 / (ex * ex) : 1;
  const variance = ratio > 1 ? Math.log(ratio) : 0;
  if (variance <= 0 || ex <= 0) {
    return df * Math.max(0, sign * (ex - kEff)); // deterministic ⇒ intrinsic
  }
  if (kEff <= 0) {
    return isCall ? df * (ex - kEff) : 0;
  }
  const sd = Math.sqrt(variance);
  const d1 = (Math.log(ex / kEff) + 0.5 * variance) / sd;
  const d2 = d1 - sd;
  return df * sign * (ex * normCdf(sign * d1) - kEff * normCdf(sign * d2));
}

/**
 * Price an arithmetic-average-rate Asian by two-moment lognormal matching. The
 * random part `X = (1−w)·Ā_fut` of the average is matched to a lognormal; the
 * seasoned strike `K' = K − w·elapsed_avg` shifts off the deterministic part.
 * The headline `price` is the discounted option value; deltas/vega are by
 * central finite difference on the same closed form (so the ticket's Greek strip
 * is populated honestly rather than zeroed).
 */
function priceAsian(spec: AsianOption, m: MarketContext, t: number): PriceOutcome {
  const value = (mk: MarketContext): number => asianValue(spec, mk, t);

  const greeks = zeroGreeks();
  greeks.price = value(m);

  // Central finite differences for the displayed sensitivities.
  const hS = m.spot * 1e-4;
  const hV = 1e-4;
  const hT = Math.min(1e-4, t * 0.5);
  const bump = (over: Partial<MarketContext>): MarketContext => ({ ...m, ...over });
  greeks.deltaSpot = (value(bump({ spot: m.spot + hS })) - value(bump({ spot: m.spot - hS }))) / (2 * hS);
  greeks.gamma =
    (value(bump({ spot: m.spot + hS })) - 2 * greeks.price + value(bump({ spot: m.spot - hS }))) /
    (hS * hS);
  greeks.vega = (value(bump({ vol: m.vol + hV })) - value(bump({ vol: m.vol - hV }))) / (2 * hV);
  // Theta as decay of value with a shrinking horizon (per-day display convention).
  if (t - hT > 0) {
    greeks.theta = -((asianValue(spec, m, t) - asianValue(spec, m, t - hT)) / hT) / 365;
  }

  return { greeks, resolvedStrike: spec.strike };
}

/** The discounted Asian option value at a market/horizon (no Greeks). */
function asianValue(spec: AsianOption, m: MarketContext, t: number): number {
  const w = Math.min(Math.max(spec.elapsedWeight, 0), 1 - 1e-12);
  const tStart = 0; // fresh-window start; seasoning enters via the weight/avg below
  const moments =
    spec.averaging === "CONTINUOUS"
      ? continuousMoments(m, t, tStart)
      : discreteMoments(m, t, tStart, Math.max(1, Math.trunc(spec.observations)));
  const fixed = w * spec.elapsedAvg;
  const randW = 1 - w;
  const ex = randW * moments.m1;
  const ex2 = randW * randW * moments.m2;
  const df = Math.exp(-m.rDom * t);
  const kEff = spec.strike - fixed;
  return blackOnAverage(spec.optionType === "CALL", ex, ex2, kEff, df);
}

// ---------------------------------------------------------------------------
// forward-start vanilla — FX dual-carry closed form
// ---------------------------------------------------------------------------
//
// The strike resets at t₁ to m·S(t₁); under GBM the value scales out the random
// reset spot, giving V = e^{−r_f·t₁}·S₀·u(m, T−t₁), where u is a UNIT-spot GK
// vanilla (spot 1, strike m) over the residual maturity T−t₁ (Rubinstein 1990;
// provenance in docs only). This mirrors celnet-exotics `forward_start_price`
// field-for-field on the same GK math the mock already uses, so the offline price
// is a genuine closed form — exact, and at t₁→0 it collapses to a plain GK
// vanilla struck at m·S₀ (the documented limit). Greeks are by central FD on the
// closed form so the ticket's strip is populated honestly.

/** Value of a unit-spot GK vanilla (spot 1, strike `moneyness`) over `residual`. */
function unitSpotVanilla(
  isCall: boolean,
  moneyness: number,
  m: MarketContext,
  residual: number,
): number {
  if (residual <= 0) {
    // Degenerate residual maturity ⇒ the (undiscounted) unit forward intrinsic.
    const sign = isCall ? 1 : -1;
    return Math.max(0, sign * (1 - moneyness));
  }
  return vanillaGreeks(isCall, moneyness, { ...m, spot: 1 }, residual).price;
}

/** The discounted forward-start value at a market/horizon (no Greeks). */
function forwardStartValue(spec: ForwardStart, m: MarketContext, t: number): number {
  const reset = Math.min(Math.max(spec.reset, 0), t);
  const residual = t - reset;
  const unit = unitSpotVanilla(spec.optionType === "CALL", spec.moneyness, m, residual);
  return Math.exp(-m.rFor * reset) * m.spot * unit;
}

/**
 * Price a forward-start vanilla by the FX dual-carry closed form, with the
 * resolved strike reported as the ATM-forward strike the reset would set today
 * (`m · S₀`) for the ticket's display. Greeks by central finite difference.
 */
function priceForwardStart(spec: ForwardStart, m: MarketContext, t: number): PriceOutcome {
  const value = (mk: MarketContext): number => forwardStartValue(spec, mk, t);
  const greeks = zeroGreeks();
  greeks.price = value(m);

  const hS = m.spot * 1e-4;
  const hV = 1e-4;
  const bump = (over: Partial<MarketContext>): MarketContext => ({ ...m, ...over });
  greeks.deltaSpot =
    (value(bump({ spot: m.spot + hS })) - value(bump({ spot: m.spot - hS }))) / (2 * hS);
  greeks.gamma =
    (value(bump({ spot: m.spot + hS })) - 2 * greeks.price + value(bump({ spot: m.spot - hS }))) /
    (hS * hS);
  greeks.vega = (value(bump({ vol: m.vol + hV })) - value(bump({ vol: m.vol - hV }))) / (2 * hV);

  return { greeks, resolvedStrike: spec.moneyness * m.spot };
}

// ---------------------------------------------------------------------------
// cliquet / ratchet — plain = Σ forward-start legs; clamped = Monte-Carlo
// ---------------------------------------------------------------------------
//
// A cliquet is a strip of consecutive forward-start vanillas over an evenly-spaced
// reset schedule. The PLAIN (unclamped) ratchet is EXACTLY the sum of the
// forward-start legs (closed form, mirrors celnet-exotics `cliquet_price_plain`).
// ANY local/global clamp makes the per-period payoff a non-linear clamp of the
// period return, which has no closed form ⇒ a Monte-Carlo estimator that reports
// the price AND its standard error (mirrors `cliquet_price_capped_mc`). The MC
// uses the deterministic seeded Rng (so a fixed seed reproduces bit-for-bit) and
// terminal-settles each opening-spot-scaled clamped leg, exactly like the server's
// path payoff. The live WS transport prices the genuine server MC; offline this is
// an honest MC with a real stderr — never a closed-form claim for the clamped case.

/** `true` iff a cliquet carries no local/global clamp (the plain ratchet). */
function cliquetIsPlain(spec: Cliquet): boolean {
  return (
    spec.localFloor === undefined &&
    spec.localCap === undefined &&
    spec.globalFloor === undefined &&
    spec.globalCap === undefined
  );
}

/** The discounted value of one PLAIN forward-start leg (reset→expiry). */
function cliquetPlainValue(spec: Cliquet, m: MarketContext, t: number): number {
  const n = Math.max(1, Math.trunc(spec.periods));
  let total = 0;
  for (let k = 1; k <= n; k += 1) {
    const reset = (t * (k - 1)) / n;
    const expiry = (t * k) / n;
    total += forwardStartValue(
      { optionType: spec.optionType, moneyness: spec.moneyness, reset },
      m,
      expiry,
    );
  }
  return total;
}

/** One period's per-unit clamped option return `clamp(φ·(ratio − m); [floor,cap])`. */
function clampedReturn(spec: Cliquet, phi: number, ratio: number): number {
  let ret = Math.max(0, phi * (ratio - spec.moneyness));
  if (spec.localFloor !== undefined) ret = Math.max(ret, spec.localFloor);
  if (spec.localCap !== undefined) ret = Math.min(ret, spec.localCap);
  return ret;
}

/**
 * One cliquet path's accumulated (undiscounted) terminal payoff for antithetic
 * sign `s`. `z` carries one standard-normal increment per period; `dtYears` is the
 * (equal) period length in years. The log-ratio over period `k` is GK-distributed
 * `N((r_d − r_f − ½σ²)Δt, σ²Δt)`, exactly the server's path simulation.
 */
function cliquetPathPayoff(
  spec: Cliquet,
  m: MarketContext,
  z: number[],
  s: number,
  dtYears: number,
): number {
  const n = Math.max(1, Math.trunc(spec.periods));
  const phi = spec.optionType === "CALL" ? 1 : -1;
  const drift = (m.rDom - m.rFor - 0.5 * m.vol * m.vol) * dtYears;
  const diffusionScale = m.vol * Math.sqrt(dtYears);
  let spot = m.spot;
  let acc = 0;
  for (let k = 1; k <= n; k += 1) {
    const ratio = Math.exp(drift + diffusionScale * s * z[k - 1]!);
    acc += spot * clampedReturn(spec, phi, ratio);
    spot *= ratio;
  }
  if (spec.globalFloor !== undefined) acc = Math.max(acc, spec.globalFloor);
  if (spec.globalCap !== undefined) acc = Math.min(acc, spec.globalCap);
  return acc;
}

/** Default antithetic path-pairs for a clamped cliquet when the trader leaves 0. */
const CLIQUET_DEFAULT_PAIRS = 20_000;

/**
 * Price a cliquet. A plain ratchet is the exact Σ of forward-start legs (closed
 * form, no stderr). A clamped cliquet is priced by an antithetic Monte-Carlo over
 * the period-return path, scaled by the period drift/diffusion at `t`; it reports
 * the discounted mean AND its standard error (Welford), so the ticket shows an
 * honest precision band — never a closed-form claim.
 */
function priceCliquet(spec: Cliquet, m: MarketContext, t: number): PriceOutcome {
  if (cliquetIsPlain(spec)) {
    const value = (mk: MarketContext): number => cliquetPlainValue(spec, mk, t);
    const greeks = zeroGreeks();
    greeks.price = value(m);
    const hS = m.spot * 1e-4;
    const hV = 1e-4;
    const bump = (over: Partial<MarketContext>): MarketContext => ({ ...m, ...over });
    greeks.deltaSpot =
      (value(bump({ spot: m.spot + hS })) - value(bump({ spot: m.spot - hS }))) / (2 * hS);
    greeks.vega = (value(bump({ vol: m.vol + hV })) - value(bump({ vol: m.vol - hV }))) / (2 * hV);
    return { greeks, resolvedStrike: spec.moneyness * m.spot };
  }

  const n = Math.max(1, Math.trunc(spec.periods));
  const dtYears = t / n;
  const pairs = spec.mcPairs > 0 ? Math.trunc(spec.mcPairs) : CLIQUET_DEFAULT_PAIRS;
  const rng = new Rng(spec.mcSeed === 0n ? 0xc11_c0e7n : spec.mcSeed);
  const df = Math.exp(-m.rDom * t);

  let count = 0;
  let mean = 0;
  let m2 = 0;
  const push = (x: number): void => {
    count += 1;
    const d = x - mean;
    mean += d / count;
    m2 += d * (x - mean);
  };

  // Draw one standard normal per period; build both antithetic legs from the same
  // normals (the path function applies the period drift/diffusion at dtYears).
  const z = new Array<number>(n).fill(0);
  for (let p = 0; p < pairs; p += 1) {
    for (let k = 0; k < n; k += 1) z[k] = rng.normal();
    const up = df * cliquetPathPayoff(spec, m, z, 1, dtYears);
    const dn = df * cliquetPathPayoff(spec, m, z, -1, dtYears);
    push(0.5 * (up + dn)); // antithetic pair mean is one sample of the estimator
  }
  const stdError = count >= 2 ? Math.sqrt(m2 / (count - 1) / count) : 0;

  const greeks = zeroGreeks();
  greeks.price = mean;
  return { greeks, resolvedStrike: spec.moneyness * m.spot, priceStdError: stdError };
}

// ---------------------------------------------------------------------------
// quanto — closed-form drift adjustment under the settlement-currency measure
// ---------------------------------------------------------------------------
//
// A quanto pays its natural payoff converted into a fixed settlement currency at a
// fixed rate. Under the settlement measure the underlying's carry shifts by the
// quanto-drift adjustment −ρ·σ_S·σ_Z; the price is then the ordinary GK vanilla
// (or a cash-or-nothing digital) at the adjusted carry, discounted by r_dom. At
// ρ=0 the adjustment vanishes and the price collapses to the plain vanilla/digital
// (the documented limit). Mirrors celnet-exotics `quanto_vanilla_price` /
// `quanto_digital_price` field-for-field on the same GK math.

/** The carry-adjusted market for a quanto (`r_for' = r_for − (−ρ·σ_S·σ_Z)`). */
function quantoAdjustedMarket(spec: Quanto, m: MarketContext): MarketContext {
  const adjustment = -spec.correlation * m.vol * spec.conversionVol;
  // carry_new = carry_old + adjustment ⇒ r_for_new = r_for − adjustment.
  return { ...m, rFor: m.rFor - adjustment };
}

/** The discounted quanto value at a market/horizon (no Greeks). */
function quantoValue(spec: Quanto, m: MarketContext, t: number): number {
  const adj = quantoAdjustedMarket(spec, m);
  const isCall = spec.optionType === "CALL";
  if (spec.payoff === "VANILLA") {
    return vanillaGreeks(isCall, spec.strike, adj, t).price;
  }
  // Cash-or-nothing digital: e^{−r_dom·t}·Φ(±d₂) at the adjusted carry.
  const sqrtT = Math.sqrt(t);
  const d1 =
    (Math.log(adj.spot / spec.strike) + (adj.rDom - adj.rFor + 0.5 * adj.vol * adj.vol) * t) /
    (adj.vol * sqrtT);
  const d2 = d1 - adj.vol * sqrtT;
  const df = Math.exp(-adj.rDom * t);
  return isCall ? df * normCdf(d2) : df * normCdf(-d2);
}

/**
 * Price a quanto (vanilla or cash-or-nothing digital) by the closed-form drift
 * adjustment. Greeks by central finite difference on the closed form so the
 * ticket's strip is populated honestly.
 */
function priceQuanto(spec: Quanto, m: MarketContext, t: number): PriceOutcome {
  const value = (mk: MarketContext): number => quantoValue(spec, mk, t);
  const greeks = zeroGreeks();
  greeks.price = value(m);

  const hS = m.spot * 1e-4;
  const hV = 1e-4;
  const bump = (over: Partial<MarketContext>): MarketContext => ({ ...m, ...over });
  greeks.deltaSpot =
    (value(bump({ spot: m.spot + hS })) - value(bump({ spot: m.spot - hS }))) / (2 * hS);
  greeks.gamma =
    (value(bump({ spot: m.spot + hS })) - 2 * greeks.price + value(bump({ spot: m.spot - hS }))) /
    (hS * hS);
  greeks.vega = (value(bump({ vol: m.vol + hV })) - value(bump({ vol: m.vol - hV }))) / (2 * hV);

  return { greeks, resolvedStrike: spec.strike };
}

// ---------------------------------------------------------------------------
// shared Monte-Carlo accumulator (Welford mean + std-error of the mean)
// ---------------------------------------------------------------------------
//
// A streaming mean / standard-error accumulator (Welford), shared by the Wave-3
// Monte-Carlo products (TARF, accumulator, discrete lookback). The reported std
// error is the standard error OF THE MEAN — `√(M₂ / (n−1) / n)` — matching the
// server's `Welford::std_error` so the offline `priceStdError` carries the same
// honest precision band the live `Quote.priceStdError` does.

class McAccumulator {
  private count = 0;
  private mean = 0;
  private m2 = 0;

  push(x: number): void {
    this.count += 1;
    const d = x - this.mean;
    this.mean += d / this.count;
    this.m2 += d * (x - this.mean);
  }

  get value(): number {
    return this.mean;
  }

  /** Standard error of the mean (`0` until at least two samples). */
  get stdError(): number {
    return this.count >= 2 ? Math.sqrt(this.m2 / (this.count - 1) / this.count) : 0;
  }
}

// ---------------------------------------------------------------------------
// TARF — Target-Redemption Forward, always Monte-Carlo (bank present value)
// ---------------------------------------------------------------------------
//
// A strip of geared fixings at a single strike. The favourable side accrues client
// gains (a PUT gains when S<K, a CALL when S>K); once the accumulated gain reaches
// the target the structure REDEEMS (the breaching fixing pays its full intrinsic
// gain, possibly overshooting, under FULL_GAIN, or only the remaining target under
// CAPPED_GAIN — the explicit gap-risk premium). The adverse side pays the bank a
// geared loss. The reported value is the BANK's present value (positive = value to
// the bank). There is no closed form ⇒ an antithetic Monte-Carlo with a real
// standard error, mirroring celnet-exotics `tarf_price` per-unit path math (one
// unit of base notional per fixing; the fixing notional scales linearly and is not
// part of the per-unit ticket display). The schedule is the EQUALLY-spaced grid
// `t_k = k·T/n`, the server's convention.

/** Default antithetic path-pairs for a TARF when the trader leaves `mcPairs = 0`. */
const TARF_DEFAULT_PAIRS = 20_000;

/** One TARF path's discounted bank PV for antithetic sign `s` (per unit, notional 1). */
function tarfPathBankPv(
  spec: Tarf,
  m: MarketContext,
  z: number[],
  s: number,
  dtYears: number,
): number {
  const n = z.length;
  // A PUT gains the client when S<K ⇒ gain-sign −1; a CALL when S>K ⇒ +1.
  const gainSign = spec.optionType === "CALL" ? 1 : -1;
  const drift = (m.rDom - m.rFor - 0.5 * m.vol * m.vol) * dtYears;
  const diffusionScale = m.vol * Math.sqrt(dtYears);
  let lnS = Math.log(m.spot);
  let accumulatedGain = 0;
  let bankPv = 0;
  for (let k = 0; k < n; k += 1) {
    lnS += drift + diffusionScale * s * z[k]!;
    const sK = Math.exp(lnS);
    const df = Math.exp(-m.rDom * (k + 1) * dtYears);
    const signed = gainSign * (sK - spec.strike);
    if (signed > 0) {
      const remaining = spec.target - accumulatedGain;
      if (signed >= remaining) {
        // Breaching (redeeming) fixing: settle per the gap-risk convention.
        const settled = spec.redemption === "FULL_GAIN" ? signed : remaining;
        bankPv -= settled * df;
        return bankPv; // structure redeems — no further fixings.
      }
      bankPv -= signed * df;
      accumulatedGain += signed;
    } else if (signed < 0) {
      // Adverse fixing: the bank receives the geared client loss.
      bankPv += spec.leverage * -signed * df;
    }
  }
  return bankPv;
}

/**
 * Price a TARF by antithetic Monte-Carlo over the equally-spaced fixing grid,
 * reporting the discounted bank PV AND its standard error (so the ticket shows an
 * honest precision band — a TARF has no closed form). The MC uses the deterministic
 * seeded `Rng`, so a fixed seed reproduces bit-for-bit.
 */
function priceTarf(spec: Tarf, m: MarketContext, t: number): PriceOutcome {
  const n = Math.max(1, spec.schedule.fixingYears.length);
  const dtYears = t / n;
  const pairs = spec.mcPairs > 0 ? Math.trunc(spec.mcPairs) : TARF_DEFAULT_PAIRS;
  const rng = new Rng(spec.mcSeed === 0n ? 0x7a_2fn : spec.mcSeed);

  const acc = new McAccumulator();
  const z = new Array<number>(n).fill(0);
  for (let p = 0; p < pairs; p += 1) {
    for (let k = 0; k < n; k += 1) z[k] = rng.normal();
    const up = tarfPathBankPv(spec, m, z, 1, dtYears);
    const dn = tarfPathBankPv(spec, m, z, -1, dtYears);
    acc.push(0.5 * (up + dn));
  }

  const greeks = zeroGreeks();
  greeks.price = acc.value;
  return { greeks, resolvedStrike: spec.strike, priceStdError: acc.stdError };
}

// ---------------------------------------------------------------------------
// accumulator — periodic accrual with up-and-out knock-out, always Monte-Carlo
// ---------------------------------------------------------------------------
//
// Periodic accumulation at a pivot strike with an up-and-out knock-out barrier
// (barrier > pivot) and gearing on the below-pivot (loss) leg. Above pivot the
// client gains S−K on one unit; below pivot the client loses leverage·(K−S). The
// reported value is the CLIENT's present value. DISCRETE monitoring tests the
// barrier only at fixings (a node at/above barrier knocks the structure dead from
// then on); CONTINUOUS monitoring weights each fixing by the Brownian-bridge
// no-crossing probability between fixings (knocks out more often ⇒ shrinks |PV|).
// No closed form ⇒ antithetic Monte-Carlo with a real standard error, mirroring
// celnet-exotics `accumulator_price` per-unit path math.

/** Default antithetic path-pairs for an accumulator when `mcPairs = 0`. */
const ACCUMULATOR_DEFAULT_PAIRS = 20_000;

/** One accumulator path's discounted client PV for antithetic sign `s` (per unit). */
function accumulatorPathClientPv(
  spec: Accumulator,
  m: MarketContext,
  z: number[],
  s: number,
  dtYears: number,
): number {
  const n = z.length;
  const drift = (m.rDom - m.rFor - 0.5 * m.vol * m.vol) * dtYears;
  const diffusionScale = m.vol * Math.sqrt(dtYears);
  const varStep = diffusionScale * diffusionScale;
  const lnB = Math.log(spec.barrier);
  const continuous = spec.monitoring === "CONTINUOUS";
  let lnPrev = Math.log(m.spot);
  let clientPv = 0;
  let survival = 1; // probability the structure is still alive (continuous)
  for (let k = 0; k < n; k += 1) {
    const lnNext = lnPrev + drift + diffusionScale * s * z[k]!;
    // Continuous: probability the up-barrier was NOT crossed on the bridge between
    // the two endpoints, conditional on both being below the barrier.
    const stepSurvival = continuous
      ? lnPrev < lnB && lnNext < lnB
        ? 1 - Math.exp((-2 * (lnB - lnPrev) * (lnB - lnNext)) / varStep)
        : 0
      : 1;
    const sK = Math.exp(lnNext);
    const nodeKnocked = sK >= spec.barrier;
    const aliveWeight = continuous ? survival * stepSurvival : nodeKnocked ? 0 : 1;
    if (aliveWeight > 0) {
      const leg = sK > spec.pivot ? sK - spec.pivot : -spec.leverage * (spec.pivot - sK);
      const df = Math.exp(-m.rDom * (k + 1) * dtYears);
      clientPv += aliveWeight * leg * df;
    }
    if (continuous) {
      survival *= stepSurvival;
    } else if (nodeKnocked) {
      return clientPv; // discrete knock-out: dead from here on.
    }
    lnPrev = lnNext;
  }
  return clientPv;
}

/**
 * Price an accumulator by antithetic Monte-Carlo over the equally-spaced fixing
 * grid, reporting the discounted client PV AND its standard error. Reproducible
 * bit-for-bit at a fixed seed.
 */
function priceAccumulator(spec: Accumulator, m: MarketContext, t: number): PriceOutcome {
  const n = Math.max(1, spec.schedule.fixingYears.length);
  const dtYears = t / n;
  const pairs = spec.mcPairs > 0 ? Math.trunc(spec.mcPairs) : ACCUMULATOR_DEFAULT_PAIRS;
  const rng = new Rng(spec.mcSeed === 0n ? 0xacc_1en : spec.mcSeed);

  const acc = new McAccumulator();
  const z = new Array<number>(n).fill(0);
  for (let p = 0; p < pairs; p += 1) {
    for (let k = 0; k < n; k += 1) z[k] = rng.normal();
    const up = accumulatorPathClientPv(spec, m, z, 1, dtYears);
    const dn = accumulatorPathClientPv(spec, m, z, -1, dtYears);
    acc.push(0.5 * (up + dn));
  }

  const greeks = zeroGreeks();
  greeks.price = acc.value;
  return { greeks, resolvedStrike: spec.pivot, priceStdError: acc.stdError };
}

// ---------------------------------------------------------------------------
// lookback — continuous = exact closed form; discrete = Monte-Carlo + stderr
// ---------------------------------------------------------------------------
//
// A lookback on the running path extremum, started at-inception (the running
// extremum equals the current spot). The CONTINUOUS-monitoring variant prices by
// the exact closed form (floating-strike via the Goldman-Sosin-Gatto
// representation, fixed-strike via the Conze-Viswanathan representation —
// provenance in docs only); it carries NO Monte-Carlo std-error. The DISCRETE
// variant samples the extremum over a finite observation grid and prices by
// antithetic Monte-Carlo, reporting a standard error. Mirrors celnet-exotics
// `floating_lookback_price` / `fixed_lookback_price` / `lookback_mc`.

/** The discounted continuous-monitoring lookback value (exact closed form). */
function lookbackContinuousValue(spec: Lookback, m: MarketContext, t: number): number {
  const s = m.spot;
  const sig = m.vol;
  const sst = sig * Math.sqrt(t);
  const b = m.rDom - m.rFor;
  const dfDom = Math.exp(-m.rDom * t);
  const dfFor = Math.exp(-m.rFor * t);
  const twoBOverSig2 = (2 * b) / (sig * sig);
  const isCall = spec.optionType === "CALL";

  if (spec.style === "FLOATING") {
    // At inception the running extremum ξ = S, so (S/ξ) = 1.
    const a1 = ((b + 0.5 * sig * sig) * t) / sst;
    const a2 = a1 - sst;
    if (isCall) {
      const main = s * dfFor * normCdf(a1) - s * dfDom * normCdf(a2);
      const refl =
        s *
        dfDom *
        ((sig * sig) / (2 * b)) *
        (normCdf(-a1 + twoBOverSig2 * sst) - Math.exp(b * t) * normCdf(-a1));
      return main + refl;
    }
    const main = s * dfDom * normCdf(-a2) - s * dfFor * normCdf(-a1);
    const refl =
      s *
      dfDom *
      ((sig * sig) / (2 * b)) *
      (-normCdf(a1 - twoBOverSig2 * sst) + Math.exp(b * t) * normCdf(a1));
    return main + refl;
  }

  // FIXED-strike (Conze-Viswanathan), running extremum = S at inception.
  const k = spec.strike;
  const d1 = (Math.log(s / k) + (b + 0.5 * sig * sig) * t) / sst;
  const d2 = d1 - sst;
  const e1 = ((b + 0.5 * sig * sig) * t) / sst;
  const e2 = e1 - sst;
  if (isCall) {
    if (k >= s) {
      const main = s * dfFor * normCdf(d1) - k * dfDom * normCdf(d2);
      const refl =
        s *
        dfDom *
        ((sig * sig) / (2 * b)) *
        (-Math.pow(s / k, -twoBOverSig2) * normCdf(d1 - twoBOverSig2 * sst) +
          Math.exp(b * t) * normCdf(d1));
      return main + refl;
    }
    const main =
      dfDom * (s * Math.exp(b * t) - k) +
      s * dfFor * normCdf(e1) -
      s * dfDom * Math.exp(b * t) * normCdf(e2);
    const refl =
      s *
      dfDom *
      ((sig * sig) / (2 * b)) *
      (-normCdf(e1 - twoBOverSig2 * sst) + Math.exp(b * t) * normCdf(e1));
    return main + refl;
  }
  if (k <= s) {
    const main = k * dfDom * normCdf(-d2) - s * dfFor * normCdf(-d1);
    const refl =
      s *
      dfDom *
      ((sig * sig) / (2 * b)) *
      (Math.pow(s / k, -twoBOverSig2) * normCdf(-d1 + twoBOverSig2 * sst) -
        Math.exp(b * t) * normCdf(-d1));
    return main + refl;
  }
  const main =
    dfDom * (k - s * Math.exp(b * t)) +
    s * dfDom * Math.exp(b * t) * normCdf(-e2) -
    s * dfFor * normCdf(-e1);
  const refl =
    s *
    dfDom *
    ((sig * sig) / (2 * b)) *
    (normCdf(-e1 + twoBOverSig2 * sst) - Math.exp(b * t) * normCdf(-e1));
  return main + refl;
}

/** Default antithetic path-pairs / observations for a discrete lookback. */
const LOOKBACK_DEFAULT_PAIRS = 20_000;
const LOOKBACK_DEFAULT_OBSERVATIONS = 52;

/** One discrete-lookback path's discounted payoff for antithetic sign `s`. */
function lookbackPathPayoff(
  spec: Lookback,
  m: MarketContext,
  z: number[],
  s: number,
  dtYears: number,
  df: number,
): number {
  const drift = (m.rDom - m.rFor - 0.5 * m.vol * m.vol) * dtYears;
  const diffusionScale = m.vol * Math.sqrt(dtYears);
  let lnS = Math.log(m.spot);
  let runMin = m.spot;
  let runMax = m.spot;
  let last = m.spot;
  for (let k = 0; k < z.length; k += 1) {
    lnS += drift + diffusionScale * s * z[k]!;
    last = Math.exp(lnS);
    if (last < runMin) runMin = last;
    if (last > runMax) runMax = last;
  }
  const isCall = spec.optionType === "CALL";
  let payoff: number;
  if (spec.style === "FLOATING") {
    payoff = isCall ? last - runMin : runMax - last;
  } else {
    payoff = isCall ? Math.max(0, runMax - spec.strike) : Math.max(0, spec.strike - runMin);
  }
  return df * payoff;
}

/**
 * Price a discretely-monitored lookback by antithetic Monte-Carlo over the
 * observation grid, reporting the discounted price AND its standard error.
 * Reproducible bit-for-bit at a fixed seed.
 */
function priceLookbackDiscrete(spec: Lookback, m: MarketContext, t: number): PriceOutcome {
  const obs = spec.observations > 0 ? Math.trunc(spec.observations) : LOOKBACK_DEFAULT_OBSERVATIONS;
  const dtYears = t / obs;
  const pairs = spec.mcPairs > 0 ? Math.trunc(spec.mcPairs) : LOOKBACK_DEFAULT_PAIRS;
  const rng = new Rng(spec.mcSeed === 0n ? 0x100_b_acen : spec.mcSeed);
  const df = Math.exp(-m.rDom * t);

  const acc = new McAccumulator();
  const z = new Array<number>(obs).fill(0);
  for (let p = 0; p < pairs; p += 1) {
    for (let k = 0; k < obs; k += 1) z[k] = rng.normal();
    const up = lookbackPathPayoff(spec, m, z, 1, dtYears, df);
    const dn = lookbackPathPayoff(spec, m, z, -1, dtYears, df);
    acc.push(0.5 * (up + dn));
  }

  const greeks = zeroGreeks();
  greeks.price = acc.value;
  const resolvedStrike = spec.style === "FIXED" ? spec.strike : m.spot;
  return { greeks, resolvedStrike, priceStdError: acc.stdError };
}

/**
 * Price a lookback. CONTINUOUS monitoring uses the exact closed form (no MC
 * std-error) with Greeks by central finite difference; DISCRETE monitoring is an
 * honest antithetic Monte-Carlo that reports a standard error.
 */
function priceLookback(spec: Lookback, m: MarketContext, t: number): PriceOutcome {
  if (spec.monitoring === "DISCRETE") {
    return priceLookbackDiscrete(spec, m, t);
  }

  const value = (mk: MarketContext): number => lookbackContinuousValue(spec, mk, t);
  const greeks = zeroGreeks();
  greeks.price = value(m);

  const hS = m.spot * 1e-4;
  const hV = 1e-4;
  const bump = (over: Partial<MarketContext>): MarketContext => ({ ...m, ...over });
  greeks.deltaSpot =
    (value(bump({ spot: m.spot + hS })) - value(bump({ spot: m.spot - hS }))) / (2 * hS);
  greeks.gamma =
    (value(bump({ spot: m.spot + hS })) - 2 * greeks.price + value(bump({ spot: m.spot - hS }))) /
    (hS * hS);
  greeks.vega = (value(bump({ vol: m.vol + hV })) - value(bump({ vol: m.vol - hV }))) / (2 * hV);

  const resolvedStrike = spec.style === "FIXED" ? spec.strike : m.spot;
  return { greeks, resolvedStrike };
}
