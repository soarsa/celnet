//! Quanto FX-options — payoffs settled in a *third* currency at a fixed
//! conversion rate, so the holder takes the underlying's move but none of the
//! settlement-currency translation risk.
//!
//! # The quanto adjustment
//!
//! Let the underlying exchange rate be `S` (units of currency `Y` per unit of
//! base `X`) with Black volatility `σ_S`, and let `Z` be the rate that converts
//! the natural payoff currency `Y` into the (fixed) settlement currency `M`, with
//! volatility `σ_Z` and instantaneous correlation `ρ` between `S` and `Z`.
//! Settling a `Y`-denominated payoff in `M` at a *fixed* unit rate (a "quanto")
//! changes the risk-neutral drift of `S` seen from the `M`-measure by the
//! **quanto correction**
//!
//! ```text
//!   adjustment = − ρ · σ_S · σ_Z .
//! ```
//!
//! Concretely, under the settlement-currency (`M`) risk-neutral measure the spot
//! grows at the carry `b_Q = (r_dom − r_for) − ρ σ_S σ_Z` while discounting still
//! uses the settlement (domestic-equivalent) rate `r_dom`. Everything else is the
//! ordinary Garman-Kohlhagen / Black-Scholes machinery evaluated at that shifted
//! carry — so the quanto vanilla and the quanto digital both have **exact closed
//! forms**, which the Monte-Carlo engine here is cross-validated against.
//!
//! Provenance (doc-only): the change-of-numéraire / Girsanov derivation of the
//! quanto drift in Reiner (1992) "Quanto Mechanics"; the textbook treatment in
//! Hull, *Options, Futures and Other Derivatives*, and Wystup (2017),
//! *FX Options and Structured Products*. Identifiers are purpose-named and
//! vendor/research-neutral.

use celnet_core::math::{exp, ln, norm_cdf, sqrt};
use celnet_types::{OptionType, VanillaInputs};

use crate::normal::inverse_cdf;
use crate::rng::CounterRng;

/// The market data the quanto correction needs beyond the underlying's own
/// [`VanillaInputs`]: the settlement-conversion-rate volatility and its
/// correlation with the underlying.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QuantoParams {
    /// Annualised volatility `σ_Z` of the rate converting the natural payoff
    /// currency into the fixed settlement currency.
    pub conversion_vol: f64,
    /// Instantaneous correlation `ρ ∈ [−1, 1]` between the underlying spot and
    /// the settlement-conversion rate.
    pub correlation: f64,
}

impl QuantoParams {
    /// Construct, asserting the correlation is a valid coefficient in `[−1, 1]`.
    #[must_use]
    pub fn new(conversion_vol: f64, correlation: f64) -> Self {
        assert!(
            (-1.0..=1.0).contains(&correlation),
            "quanto correlation must lie in [-1, 1], got {correlation}"
        );
        assert!(
            conversion_vol >= 0.0,
            "conversion vol must be non-negative, got {conversion_vol}"
        );
        Self {
            conversion_vol,
            correlation,
        }
    }

    /// The additive quanto drift correction `− ρ · σ_S · σ_Z` applied to the
    /// underlying's carry under the settlement-currency measure.
    #[inline]
    #[must_use]
    pub fn drift_adjustment(&self, underlying_vol: f64) -> f64 {
        -self.correlation * underlying_vol * self.conversion_vol
    }
}

/// Build the quanto-adjusted view of the underlying: the same inputs but with the
/// carry shifted by the quanto correction. The shift is realised by adjusting the
/// *foreign* rate so that `r_dom − r_for_adj = (r_dom − r_for) + adjustment`,
/// while the (settlement) `r_dom` used for discounting is untouched.
#[inline]
fn quanto_adjusted_inputs(i: &VanillaInputs, q: QuantoParams) -> VanillaInputs {
    let adjustment = q.drift_adjustment(i.vol);
    VanillaInputs {
        // carry_new = carry_old + adjustment  ⇒  r_for_new = r_for − adjustment
        r_for: i.r_for - adjustment,
        ..*i
    }
}

/// Closed-form price of a **quanto vanilla** (cash-settled in the fixed settlement
/// currency at unit conversion), per one unit of base notional.
///
/// This is the ordinary Garman-Kohlhagen vanilla evaluated at the quanto-adjusted
/// carry. The settlement-currency discounting uses `i.r_dom`.
#[must_use]
pub fn quanto_vanilla_price(option: OptionType, i: &VanillaInputs, q: QuantoParams) -> f64 {
    celnet_vanilla::price(option, &quanto_adjusted_inputs(i, q))
}

/// What a quanto digital pays when in the money — here always **one unit of the
/// settlement currency** (cash-or-nothing in the fixed settlement currency), which
/// is the standard quanto-binary convention.
///
/// Present value of a **quanto cash-or-nothing digital**, per one unit of
/// settlement cash. A call pays if `S_T > K`, a put if `S_T < K`.
///
/// The exercise probability is taken under the settlement-currency measure (so it
/// uses the quanto-adjusted drift), but the payout is a fixed unit of settlement
/// cash, so the price is `e^{−r_dom T}·Φ(±d₂_Q)` with `d₂_Q` formed at the
/// adjusted carry.
#[must_use]
pub fn quanto_digital_price(option: OptionType, i: &VanillaInputs, q: QuantoParams) -> f64 {
    let adj = quanto_adjusted_inputs(i, q);
    let vsqt = adj.vol * sqrt(adj.t);
    let d1 = (ln(adj.spot / adj.strike)
        + (adj.r_dom - adj.r_for + 0.5 * adj.vol * adj.vol) * adj.t)
        / vsqt;
    let d2 = d1 - vsqt;
    let df = exp(-adj.r_dom * adj.t);
    match option {
        OptionType::Call => df * norm_cdf(d2),
        OptionType::Put => df * norm_cdf(-d2),
    }
}

/// Welford accumulator (mean + std-error of the mean) over antithetic-pair means.
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
            0.0
        } else {
            sqrt(self.m2 / ((self.n - 1) as f64) / self.n as f64)
        }
    }
}

/// A Monte-Carlo quanto estimate: discounted price plus standard error of the
/// mean.
#[derive(Debug, Clone, Copy)]
pub struct QuantoEstimate {
    /// Discounted price estimate (settlement currency).
    pub price: f64,
    /// Standard error of the mean.
    pub std_error: f64,
}

/// Monte-Carlo configuration for the quanto cross-validation engine.
#[derive(Debug, Clone, Copy)]
pub struct QuantoMcConfig {
    /// Number of antithetic path **pairs**.
    pub pairs: usize,
    /// Seed for the counter-based RNG.
    pub seed: u64,
}

/// Price a quanto vanilla by Monte-Carlo under the **settlement-currency measure**
/// — i.e. simulate terminal spot at the quanto-adjusted drift directly — with
/// antithetic variates. This is an independent route to the closed form and is the
/// cross-validation oracle in the test suite.
///
/// A single-step terminal simulation is exact for a path-independent vanilla
/// (geometric Brownian motion has a closed-form terminal law), so no time
/// discretisation bias is introduced.
#[must_use]
pub fn quanto_vanilla_mc(
    option: OptionType,
    i: &VanillaInputs,
    q: QuantoParams,
    cfg: QuantoMcConfig,
) -> QuantoEstimate {
    let adj = quanto_adjusted_inputs(i, q);
    let ln_s0 = ln(adj.spot);
    let drift = (adj.r_dom - adj.r_for - 0.5 * adj.vol * adj.vol) * adj.t;
    let vol_sqrt_t = adj.vol * sqrt(adj.t);
    let df = exp(-adj.r_dom * adj.t);
    let sign = option.sign();

    let mut acc = Welford::default();
    for pair in 0..cfg.pairs as u64 {
        let mut rng = CounterRng::new(cfg.seed, 0, pair, 0);
        let z = inverse_cdf(rng.next_u01());
        let term = |s: f64| {
            let st = exp(ln_s0 + drift + vol_sqrt_t * s * z);
            (sign * (st - adj.strike)).max(0.0)
        };
        acc.push(0.5 * (term(1.0) + term(-1.0)));
    }
    QuantoEstimate {
        price: df * acc.mean,
        std_error: df * acc.std_error(),
    }
}

/// Price a quanto cash-or-nothing digital by Monte-Carlo under the settlement
/// measure (one unit of settlement cash on exercise). Independent cross-check of
/// [`quanto_digital_price`].
#[must_use]
pub fn quanto_digital_mc(
    option: OptionType,
    i: &VanillaInputs,
    q: QuantoParams,
    cfg: QuantoMcConfig,
) -> QuantoEstimate {
    let adj = quanto_adjusted_inputs(i, q);
    let ln_s0 = ln(adj.spot);
    let drift = (adj.r_dom - adj.r_for - 0.5 * adj.vol * adj.vol) * adj.t;
    let vol_sqrt_t = adj.vol * sqrt(adj.t);
    let df = exp(-adj.r_dom * adj.t);

    let mut acc = Welford::default();
    for pair in 0..cfg.pairs as u64 {
        let mut rng = CounterRng::new(cfg.seed, 0, pair, 0);
        let z = inverse_cdf(rng.next_u01());
        let pays = |s: f64| {
            let st = exp(ln_s0 + drift + vol_sqrt_t * s * z);
            let itm = match option {
                OptionType::Call => st > adj.strike,
                OptionType::Put => st < adj.strike,
            };
            f64::from(u8::from(itm))
        };
        acc.push(0.5 * (pays(1.0) + pays(-1.0)));
    }
    QuantoEstimate {
        price: df * acc.mean,
        std_error: df * acc.std_error(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_core::assert_close;

    fn base() -> VanillaInputs {
        VanillaInputs::new(1.30, 1.30, 0.12, 1.0, 0.03, 0.01)
    }

    /// Zero correlation (or zero conversion vol) ⇒ the quanto adjustment vanishes
    /// and the quanto vanilla collapses to the plain Garman-Kohlhagen vanilla.
    #[test]
    fn zero_correlation_collapses_to_vanilla() {
        let i = base();
        for opt in [OptionType::Call, OptionType::Put] {
            let plain = celnet_vanilla::price(opt, &i);
            let q0 = quanto_vanilla_price(opt, &i, QuantoParams::new(0.10, 0.0));
            let qz = quanto_vanilla_price(opt, &i, QuantoParams::new(0.0, 0.7));
            assert_close!(q0, plain, 1e-13, 1e-13);
            assert_close!(qz, plain, 1e-13, 1e-13);
        }
    }

    /// The quanto correction has the documented sign and magnitude: shifting the
    /// carry by `−ρ σ_S σ_Z` is exactly repricing a vanilla at an adjusted foreign
    /// rate. Verify against a hand-built adjusted-rate vanilla.
    #[test]
    fn drift_adjustment_matches_adjusted_rate_vanilla() {
        let i = base();
        let q = QuantoParams::new(0.15, 0.4);
        let adjustment = -0.4 * i.vol * 0.15;
        let by_hand = VanillaInputs {
            r_for: i.r_for - adjustment,
            ..i
        };
        for opt in [OptionType::Call, OptionType::Put] {
            let via_quanto = quanto_vanilla_price(opt, &i, q);
            let direct = celnet_vanilla::price(opt, &by_hand);
            assert_close!(via_quanto, direct, 1e-14, 1e-14);
        }
        // Positive correlation lowers the call value (drift correction is
        // negative ⇒ lower forward), the documented direction.
        let call_q = quanto_vanilla_price(OptionType::Call, &i, q);
        let call_plain = celnet_vanilla::price(OptionType::Call, &i);
        assert!(
            call_q < call_plain,
            "ρ>0 must lower the quanto call: {call_q} < {call_plain}"
        );
    }

    /// Monte-Carlo (settlement-measure simulation) reproduces the quanto-vanilla
    /// closed form within Monte-Carlo tolerance — the independent cross-check.
    #[test]
    fn mc_matches_closed_form_vanilla() {
        let i = base();
        let q = QuantoParams::new(0.18, 0.55);
        let cfg = QuantoMcConfig {
            pairs: 400_000,
            seed: 0xA17E,
        };
        for opt in [OptionType::Call, OptionType::Put] {
            let closed = quanto_vanilla_price(opt, &i, q);
            let mc = quanto_vanilla_mc(opt, &i, q, cfg);
            let tol = 4.0 * mc.std_error + 1e-6;
            assert!(
                (mc.price - closed).abs() < tol,
                "quanto MC {} vs closed {closed} (se {}, tol {tol})",
                mc.price,
                mc.std_error
            );
        }
    }

    /// Monte-Carlo reproduces the quanto-digital closed form.
    #[test]
    fn mc_matches_closed_form_digital() {
        let i = base();
        let q = QuantoParams::new(0.14, -0.3);
        let cfg = QuantoMcConfig {
            pairs: 500_000,
            seed: 0xD161,
        };
        for opt in [OptionType::Call, OptionType::Put] {
            let closed = quanto_digital_price(opt, &i, q);
            let mc = quanto_digital_mc(opt, &i, q, cfg);
            let tol = 4.0 * mc.std_error + 1e-6;
            assert!(
                (mc.price - closed).abs() < tol,
                "quanto digital MC {} vs closed {closed} (se {}, tol {tol})",
                mc.price,
                mc.std_error
            );
        }
    }

    /// Digital call + digital put = the discounted unit of settlement cash (one of
    /// the two strictly pays at expiry, up to the zero-probability tie).
    #[test]
    fn digital_call_put_sum_is_discount_factor() {
        let i = base();
        let q = QuantoParams::new(0.16, 0.25);
        let c = quanto_digital_price(OptionType::Call, &i, q);
        let p = quanto_digital_price(OptionType::Put, &i, q);
        assert_close!(c + p, exp(-i.r_dom * i.t), 1e-13, 1e-13);
    }

    /// Reproducibility: identical seed ⇒ bit-identical MC price.
    #[test]
    fn mc_is_reproducible() {
        let i = base();
        let q = QuantoParams::new(0.18, 0.55);
        let cfg = QuantoMcConfig {
            pairs: 50_000,
            seed: 0x5EED,
        };
        let a = quanto_vanilla_mc(OptionType::Call, &i, q, cfg);
        let b = quanto_vanilla_mc(OptionType::Call, &i, q, cfg);
        assert_eq!(a.price.to_bits(), b.price.to_bits());
        assert_eq!(a.std_error.to_bits(), b.std_error.to_bits());
    }

    /// Invalid correlation is rejected (defensive constructor).
    #[test]
    #[should_panic(expected = "correlation must lie in")]
    fn invalid_correlation_panics() {
        let _ = QuantoParams::new(0.1, 1.5);
    }
}
