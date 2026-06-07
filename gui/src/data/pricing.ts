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
  AsianOption,
  Cliquet,
  ForwardStart,
  Greeks,
  Instrument,
  Leg,
  MarketContext,
  Quanto,
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
