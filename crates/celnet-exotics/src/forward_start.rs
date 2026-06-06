//! Forward-start vanilla options and cliquet (ratchet) structures.
//!
//! # Forward-start vanilla
//!
//! A **forward-start** option fixes its strike not today but at a future *reset*
//! date `t₁`, to a chosen moneyness multiple of the spot then prevailing:
//! `K = m·S(t₁)` (`m = 1` is the at-the-money-forward reset). It then pays the
//! ordinary vanilla payoff over the residual period `[t₁, T]`:
//! `φ·(S(T) − m·S(t₁))⁺`, with `φ = +1` call / `−1` put.
//!
//! Under Garman-Kohlhagen geometric Brownian motion the value has a **closed
//! form**. Conditional on `S(t₁)`, the option is an ordinary GK vanilla with
//! spot `S(t₁)`, strike `m·S(t₁)` and maturity `T − t₁`; that value is
//! homogeneous of degree one in `(spot, strike)`, so it equals
//! `S(t₁) · u(m, T−t₁)` where `u(m, τ)` is the **unit-spot** GK vanilla (spot 1,
//! strike `m`, maturity `τ`, same vols/rates). Discounting the date-`t₁` value
//! to today and using the FX martingale identity
//! `E^Q[ e^{−r_d t₁} S(t₁) ] = S₀ · e^{−r_f t₁}` (the domestic-discounted spot
//! grows at the foreign-rate carry) gives
//!
//! ```text
//!   V_fwd-start = e^{−r_f·t₁} · S₀ · u(m, T − t₁),
//! ```
//!
//! independent of any further conditioning — the classic Rubinstein (1990)
//! result, here in its FX (dual-carry) form. As `t₁ → 0` this collapses exactly
//! to the plain GK vanilla struck at `m·S₀`.
//!
//! # Cliquet / ratchet
//!
//! A **cliquet** (ratchet) is a strip of consecutive forward-start options over
//! a schedule of reset dates `0 = t₀ < t₁ < … < t_n = T`. Each leg `k` resets
//! its strike at `t_{k−1}` to `m·S(t_{k−1})` and pays the period return at
//! `t_k`. With **no local cap/floor** the cliquet value is, by linearity of
//! expectation, the **exact sum of its forward-start legs** — a closed form.
//!
//! A **locally-capped / -floored** cliquet clamps each period return into
//! `[floor, cap]` before summing; this breaks the closed form (the clamp is a
//! call-spread on the period return) and is priced by Monte-Carlo on the shared
//! [`crate::rng::CounterRng`] / [`crate::normal::inverse_cdf`] machinery, with a
//! global floor/cap applied to the accumulated payoff. The two are
//! cross-validated: with the local clamp opened to `(−∞, +∞)` and no global
//! clamp the MC reproduces the closed-form sum-of-legs.
//!
//! # Method provenance (doc comments only)
//!
//! Forward-start closed form: Rubinstein (1990), *Pay Now, Choose Later*. Cliquet
//! as a strip of forward-starts and the locally-capped variant: Wilmott (2002);
//! Wystup (2017), *FX Options and Structured Products*. All identifiers here are
//! purpose-named; provenance lives only in documentation.

use celnet_core::math::exp;
use celnet_types::{OptionType, VanillaInputs};
use celnet_vanilla::price as vanilla_price;

use crate::normal::inverse_cdf;
use crate::rng::CounterRng;

/// Specification of a single forward-start vanilla option.
///
/// The strike is set at the reset date `reset` to `moneyness · S(reset)` and the
/// option pays the vanilla payoff at `expiry`. Volatility and the two FX carry
/// rates are taken from the [`VanillaInputs`] passed to the pricer (its `spot` is
/// `S₀`, its `strike` field is ignored — the strike is reset-determined).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ForwardStart {
    /// Call or put.
    pub option: OptionType,
    /// Strike-reset multiple `m`: at `reset` the strike becomes `m·S(reset)`
    /// (`m = 1` is the at-the-money-forward reset).
    pub moneyness: f64,
    /// Reset (strike-fixing) date `t₁` in years.
    pub reset: f64,
    /// Expiry `T` in years (`T ≥ t₁`).
    pub expiry: f64,
}

/// Closed-form Garman-Kohlhagen forward-start vanilla value (Rubinstein 1990, FX
/// dual-carry form).
///
/// `V = e^{−r_f·t₁}·S₀·u(m, T−t₁)`, with `u` the unit-spot vanilla. Requires
/// `0 ≤ t₁ ≤ T` and `T > t₁` for a non-degenerate residual maturity; at `t₁ = T`
/// the residual maturity is zero and the value is the discounted forward
/// intrinsic of a unit-spot option (`0` for `m ≥ 1` calls, etc.), handled by the
/// unit-spot vanilla at `τ = 0` (which the GK formula evaluates as the
/// discounted intrinsic in the limit; here we guard the degenerate `τ = 0` by
/// returning the discounted unit forward intrinsic).
#[must_use]
pub fn forward_start_price(i: &VanillaInputs, spec: ForwardStart) -> f64 {
    debug_assert!(
        spec.reset >= 0.0 && spec.expiry >= spec.reset,
        "require 0 ≤ reset ≤ expiry"
    );
    let residual = spec.expiry - spec.reset;
    let unit = unit_spot_value(i, spec.option, spec.moneyness, residual);
    exp(-i.r_for * spec.reset) * i.spot * unit
}

/// Value at the reset date of a **unit-spot** GK vanilla: spot `1`, strike
/// `moneyness`, residual maturity `residual`, with the volatility / rates of `i`.
///
/// For `residual > 0` this is the ordinary GK vanilla; at `residual = 0` the GK
/// formula degenerates and we return the (undiscounted) forward intrinsic of the
/// unit-spot option, i.e. `(φ·(1 − m))⁺`.
#[inline]
fn unit_spot_value(i: &VanillaInputs, option: OptionType, moneyness: f64, residual: f64) -> f64 {
    if residual <= 0.0 {
        return (option.sign() * (1.0 - moneyness)).max(0.0);
    }
    let unit = VanillaInputs {
        spot: 1.0,
        strike: moneyness,
        vol: i.vol,
        t: residual,
        r_dom: i.r_dom,
        r_for: i.r_for,
    };
    vanilla_price(option, &unit)
}

/// A reset schedule for a cliquet: the strictly-increasing reset dates
/// `0 = t₀ < t₁ < … < t_n = T`. Period `k` resets at `t_{k−1}` and observes at
/// `t_k`, for `k = 1 … n`.
#[derive(Debug, Clone, PartialEq)]
pub struct CliquetSchedule {
    /// The reset/observation dates in years, strictly increasing, starting at
    /// `0.0`. The first entry is the contract start; subsequent entries are the
    /// per-period observation dates.
    pub dates: Vec<f64>,
}

impl CliquetSchedule {
    /// Build an evenly-spaced schedule of `periods` periods over `[0, expiry]`.
    #[must_use]
    pub fn equal(periods: usize, expiry: f64) -> Self {
        assert!(periods >= 1, "cliquet needs ≥1 period");
        let dates = (0..=periods)
            .map(|k| expiry * k as f64 / periods as f64)
            .collect();
        Self { dates }
    }

    /// Number of periods (= number of forward-start legs).
    #[must_use]
    pub fn periods(&self) -> usize {
        self.dates.len().saturating_sub(1)
    }
}

/// A cliquet (ratchet) contract: a strip of consecutive forward-start vanillas
/// with an optional per-period local floor/cap on each leg's **option** return
/// and an optional global floor/cap on the accumulated payoff.
///
/// Each period's base per-unit payoff is the forward-start option intrinsic
/// `(φ·(S(t_k)/S(t_{k−1}) − m))⁺` (scaled inside the pricer by the opening spot
/// `S(t_{k−1})`, so the unclamped strip is exactly the sum of forward-start
/// vanillas). The optional `local_floor`/`local_cap` bound that per-unit option
/// return; `global_floor`/`global_cap` bound the accumulated, opening-spot-scaled
/// payoff. `None` bounds are unconstrained.
#[derive(Debug, Clone, PartialEq)]
pub struct Cliquet {
    /// Call (`+1`) or put (`−1`) period payoff direction.
    pub option: OptionType,
    /// Per-period strike-reset multiple `m` (applied to every leg).
    pub moneyness: f64,
    /// Reset schedule.
    pub schedule: CliquetSchedule,
    /// Optional per-period local floor on each clamped period return.
    pub local_floor: Option<f64>,
    /// Optional per-period local cap on each clamped period return.
    pub local_cap: Option<f64>,
    /// Optional global floor on the accumulated (summed) payoff.
    pub global_floor: Option<f64>,
    /// Optional global cap on the accumulated (summed) payoff.
    pub global_cap: Option<f64>,
}

impl Cliquet {
    /// `true` iff this cliquet has **no** local or global clamp — the plain
    /// ratchet, which prices in closed form as the exact sum of forward-start
    /// legs.
    #[must_use]
    pub fn is_plain(&self) -> bool {
        self.local_floor.is_none()
            && self.local_cap.is_none()
            && self.global_floor.is_none()
            && self.global_cap.is_none()
    }
}

/// Exact closed-form value of a **plain** (unclamped) cliquet: the sum of its
/// per-period forward-start legs.
///
/// Each leg `k` (reset at `t_{k−1}`, expiry `t_k`) is priced by
/// [`forward_start_price`] and the values are summed. Panics in debug if the
/// cliquet carries any local/global clamp (use [`cliquet_price_capped_mc`] for
/// the clamped variant).
#[must_use]
pub fn cliquet_price_plain(i: &VanillaInputs, c: &Cliquet) -> f64 {
    debug_assert!(
        c.is_plain(),
        "cliquet_price_plain is only valid for an unclamped ratchet"
    );
    let d = &c.schedule.dates;
    let mut total = 0.0;
    for k in 1..d.len() {
        let leg = ForwardStart {
            option: c.option,
            moneyness: c.moneyness,
            reset: d[k - 1],
            expiry: d[k],
        };
        total += forward_start_price(i, leg);
    }
    total
}

/// Monte-Carlo configuration for the locally-capped/floored cliquet pricer.
#[derive(Debug, Clone, Copy)]
pub struct CliquetMcConfig {
    /// Number of antithetic path **pairs**.
    pub pairs: usize,
    /// Counter-RNG seed (identical seeds reproduce results bit-for-bit).
    pub seed: u64,
}

/// The result of a cliquet Monte-Carlo run: discounted price and the standard
/// error of the mean.
#[derive(Debug, Clone, Copy)]
pub struct CliquetEstimate {
    /// Discounted price estimate (per unit notional).
    pub price: f64,
    /// Standard error of the mean.
    pub std_error: f64,
}

/// Welford online mean/variance accumulator (local copy — kept here so this
/// module is self-contained and the MC estimator reports an honest standard
/// error).
#[derive(Default)]
struct Welford {
    n: u64,
    mean: f64,
    m2: f64,
}

impl Welford {
    #[inline]
    fn push(&mut self, x: f64) {
        self.n += 1;
        let d = x - self.mean;
        self.mean += d / self.n as f64;
        self.m2 += d * (x - self.mean);
    }
    fn std_error(&self) -> f64 {
        if self.n < 2 {
            return 0.0;
        }
        celnet_core::math::sqrt(self.m2 / ((self.n - 1) as f64) / self.n as f64)
    }
}

/// One period's per-unit (per opening-spot) payoff: the forward-start **option**
/// return `(φ·(ratio − m))⁺`, then bounded into the optional local
/// `[floor, cap]`.
///
/// The base payoff is the forward-start vanilla intrinsic over the period — an
/// *option*, hence the `max(·, 0)` — so with no local clamp the strip is exactly
/// the sum of forward-start vanillas (the plain ratchet's closed form). A
/// `local_floor` raises the option's zero floor (a guaranteed minimum coupon); a
/// `local_cap` caps the upside (the classic capped cliquet).
#[inline]
fn clamped_return(c: &Cliquet, phi: f64, ratio: f64) -> f64 {
    let mut ret = (phi * (ratio - c.moneyness)).max(0.0);
    if let Some(f) = c.local_floor {
        ret = ret.max(f);
    }
    if let Some(cap) = c.local_cap {
        ret = ret.min(cap);
    }
    ret
}

/// One cliquet path's accumulated (undiscounted) payoff for antithetic sign
/// `s ∈ {+1, −1}`, **terminal-settled** at `T`.
///
/// The path is simulated period-by-period in log-return increments: the log-ratio
/// `ln(S(t_k)/S(t_{k−1}))` over period `k` is GK-distributed
/// `N((r_d − r_f − ½σ²)Δt_k, σ²Δt_k)`, so the running spot is carried forward.
/// Each period's payoff is the **forward-start leg** payoff
/// `S(t_{k−1})·clamp(φ·(S(t_k)/S(t_{k−1}) − m); [floor, cap])` — the clamp is on
/// the per-unit return and the leg is scaled by the opening spot `S(t_{k−1})`, so
/// the unclamped strip is exactly the sum of forward-start vanillas. The legs are
/// summed and the global floor/cap is applied last.
fn cliquet_path_payoff(i: &VanillaInputs, c: &Cliquet, z: &[f64], s: f64) -> f64 {
    let d = &c.schedule.dates;
    let phi = c.option.sign();
    let mut spot = i.spot;
    let mut acc = 0.0;
    for k in 1..d.len() {
        let dt = d[k] - d[k - 1];
        let drift = (i.r_dom - i.r_for - 0.5 * i.vol * i.vol) * dt;
        let diffusion = i.vol * celnet_core::math::sqrt(dt) * s * z[k - 1];
        let ratio = exp(drift + diffusion);
        acc += spot * clamped_return(c, phi, ratio);
        spot *= ratio;
    }
    if let Some(f) = c.global_floor {
        acc = acc.max(f);
    }
    if let Some(cap) = c.global_cap {
        acc = acc.min(cap);
    }
    acc
}

/// Price a **locally-capped / -floored** cliquet by Monte-Carlo over the shared
/// counter-based RNG, with antithetic variates.
///
/// Each period's independent log-return increment is drawn from the path's own
/// counter sub-stream (one normal per period via the monotone inverse-CDF), so
/// the estimator is bit-reproducible from `(seed, path)`. The accumulated,
/// clamped payoff is discounted to today at the domestic rate over the full
/// expiry `T` (each leg pays at its observation date `t_k`; for a fair-value
/// comparison against the closed-form sum-of-legs the discounting must be applied
/// per leg — see [`cliquet_price_plain_mc`] which matches the closed form; this
/// capped pricer discounts the **terminal** accumulated payoff at `T`, the market
/// convention for a single-settlement ratchet note).
///
/// For the cross-validation against the closed-form sum-of-legs we use the
/// per-leg-discounted estimator [`cliquet_price_plain_mc`]; this terminal-settled
/// estimator is the product variant a structured-note desk quotes.
#[must_use]
pub fn cliquet_price_capped_mc(
    i: &VanillaInputs,
    c: &Cliquet,
    cfg: CliquetMcConfig,
) -> CliquetEstimate {
    let periods = c.schedule.periods();
    let df_t = exp(-i.r_dom * c.schedule.dates[c.schedule.dates.len() - 1]);
    let mut acc = Welford::default();
    let mut z = vec![0.0f64; periods];
    for pair in 0..cfg.pairs {
        draw_period_normals(cfg.seed, pair as u64, periods, &mut z);
        let a = cliquet_path_payoff(i, c, &z, 1.0);
        let b = cliquet_path_payoff(i, c, &z, -1.0);
        acc.push(0.5 * (a + b));
    }
    CliquetEstimate {
        price: df_t * acc.mean,
        std_error: df_t * acc.std_error(),
    }
}

/// Per-leg-discounted Monte-Carlo cliquet estimator: each period `k`'s clamped
/// return is discounted at the domestic rate to its **own** observation date
/// `t_k` before summing.
///
/// This is the estimator that must reconcile with the closed-form
/// [`cliquet_price_plain`] sum-of-forward-starts (each forward-start leg pays at
/// `t_k` and is discounted to today over `[0, t_k]`). With the local clamp opened
/// (no floor/cap) and no global clamp it is an unbiased MC of the same quantity.
#[must_use]
pub fn cliquet_price_plain_mc(
    i: &VanillaInputs,
    c: &Cliquet,
    cfg: CliquetMcConfig,
) -> CliquetEstimate {
    let d = &c.schedule.dates;
    let periods = c.schedule.periods();
    let phi = c.option.sign();
    let mut acc = Welford::default();
    let mut z = vec![0.0f64; periods];
    for pair in 0..cfg.pairs {
        draw_period_normals(cfg.seed, pair as u64, periods, &mut z);
        let leg_sum = |s: f64| {
            let mut total = 0.0;
            let mut spot = i.spot;
            for k in 1..d.len() {
                let dt = d[k] - d[k - 1];
                let drift = (i.r_dom - i.r_for - 0.5 * i.vol * i.vol) * dt;
                let diffusion = i.vol * celnet_core::math::sqrt(dt) * s * z[k - 1];
                let ratio = exp(drift + diffusion);
                // Forward-start leg payoff S(t_{k−1})·clamped-return, paid at t_k
                // and discounted to today over [0, t_k].
                total += exp(-i.r_dom * d[k]) * spot * clamped_return(c, phi, ratio);
                spot *= ratio;
            }
            total
        };
        acc.push(0.5 * (leg_sum(1.0) + leg_sum(-1.0)));
    }
    CliquetEstimate {
        price: acc.mean,
        std_error: acc.std_error(),
    }
}

/// Draw the `periods` independent standard-normal period increments of one path
/// from its own counter-based sub-stream, via the monotone inverse-CDF.
#[inline]
fn draw_period_normals(seed: u64, path: u64, periods: usize, out: &mut [f64]) {
    let mut rng = CounterRng::new(seed, 0, path, 0);
    for z in out.iter_mut().take(periods) {
        *z = inverse_cdf(rng.next_u01());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_core::assert_close;

    fn base() -> VanillaInputs {
        // EURUSD-like 1Y, 10 vol, dual carry.
        VanillaInputs::new(1.30, 1.30, 0.10, 1.0, 0.03, 0.01)
    }

    /// At `reset = 0` the forward-start collapses exactly to the plain GK vanilla
    /// struck at `m·S₀` — the degenerate limit of the closed form.
    #[test]
    fn reset_zero_is_plain_vanilla() {
        let i = base();
        for &m in &[0.9, 1.0, 1.1] {
            for opt in [OptionType::Call, OptionType::Put] {
                let fs = forward_start_price(
                    &i,
                    ForwardStart {
                        option: opt,
                        moneyness: m,
                        reset: 0.0,
                        expiry: i.t,
                    },
                );
                let plain = vanilla_price(
                    opt,
                    &VanillaInputs {
                        strike: m * i.spot,
                        ..i
                    },
                );
                assert_close!(fs, plain, 1e-12, 1e-12);
            }
        }
    }

    /// The plain cliquet equals the exact sum of its per-period forward-start
    /// legs (definitional — the closed form is that sum).
    #[test]
    fn plain_cliquet_is_sum_of_legs() {
        let i = base();
        let c = Cliquet {
            option: OptionType::Call,
            moneyness: 1.0,
            schedule: CliquetSchedule::equal(4, 1.0),
            local_floor: None,
            local_cap: None,
            global_floor: None,
            global_cap: None,
        };
        let total = cliquet_price_plain(&i, &c);
        let mut hand = 0.0;
        let d = &c.schedule.dates;
        for k in 1..d.len() {
            hand += forward_start_price(
                &i,
                ForwardStart {
                    option: OptionType::Call,
                    moneyness: 1.0,
                    reset: d[k - 1],
                    expiry: d[k],
                },
            );
        }
        assert_close!(total, hand, 1e-12, 1e-12);
        assert!(total > 0.0);
    }

    /// A tighter local cap reduces the capped-cliquet value (monotone,
    /// structural): clamping the upside of each leg can only lower the payoff.
    #[test]
    fn tighter_cap_reduces_value() {
        let i = base();
        let mk = |cap: Option<f64>| Cliquet {
            option: OptionType::Call,
            moneyness: 1.0,
            schedule: CliquetSchedule::equal(4, 1.0),
            local_floor: Some(0.0),
            local_cap: cap,
            global_floor: None,
            global_cap: None,
        };
        let cfg = CliquetMcConfig {
            pairs: 60_000,
            seed: 0xC119_0001,
        };
        let loose = cliquet_price_capped_mc(&i, &mk(Some(0.05)), cfg);
        let tight = cliquet_price_capped_mc(&i, &mk(Some(0.02)), cfg);
        // Common random numbers (same seed) ⇒ the difference is pure structure,
        // not MC noise; the tighter cap is strictly lower.
        assert!(
            tight.price < loose.price,
            "tighter cap {} should be < looser cap {}",
            tight.price,
            loose.price
        );
    }
}
