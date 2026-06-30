//! A hand-coded, **independent** [`ExoticLegPricer`] for tests of the cube and its
//! downstream consumers that cannot depend on `celnet-exotics`.
//!
//! This module is the **single home** of the cube's test exotic pricer. It is gated
//! `#[cfg(any(test, feature = "test-support"))]`: the cube's own `#[cfg(test)]` code
//! sees it directly, the cube's integration tests `#[path]`-include the file, and
//! downstream **test** crates that cannot depend on `celnet-exotics` either
//! (`celnet-risk-fleet`, `celnet-limits`) enable the cube's `test-support` feature in
//! their `[dev-dependencies]` and reference [`crate::test_support::DigitalTestPricer`]
//! — so the digital/barrier closed form lives in exactly one place (no duplication).
//!
//! The cube does not depend on `celnet-exotics` (arch-program item E inverted that
//! edge). The exotic closed forms it re-prices under shocks are injected through the
//! [`celnet_core::ExoticLegPricer`] seam — in production by the server over the real
//! `celnet-exotics` engines. These tests inject **this** pricer instead: a
//! self-contained re-derivation of the same closed forms from `celnet-core` math
//! plus the `celnet-vanilla` (Garman-Kohlhagen) leaf the cube already depends on.
//!
//! (`celnet-parity` tests *do* depend on `celnet-exotics`, so they back the seam with
//! a thin `ExoticEngine` over the REAL engines — the byte-identity reference — rather
//! than this hand re-derivation.)
//!
//! # Why a hand re-derivation (not a thin wrapper over `celnet-exotics`)
//!
//! Re-introducing `celnet-exotics` here purely as a test dependency would re-couple
//! the cube to the heavy pricing crate the whole inversion removed (the
//! `cargo tree -p celnet-risk-cube` gate would no longer be empty). More
//! importantly, the cube's longhand / FRTB-provenance oracles are meant to be
//! **code-disjoint** from the production pricing path: a digital priced from first
//! principles here is a genuine, non-circular oracle (the FRTB circular-oracle
//! lesson), not the engine re-run as its own check.
//!
//! # Byte-identity contract
//!
//! Every digital expression below is the **identical IEEE-754 op sequence** the
//! `celnet-exotics` digital closed form runs for an FX [`VanillaInputs`] (verified
//! line-by-line against `crates/celnet-exotics/src/digital.rs`, re-derived from the
//! published Reiner-Rubinstein / generalized-BSM formulae — NOT copied):
//!
//! * `vsqt = vol·√t`, `d1 = [ln(S/K) + (b + ½σ²)·t]/vsqt`, `d2 = d1 − vsqt`,
//!   with `b = r_dom − r_for` (the FX carry), `df_dom = e^{−r_dom·t}`,
//!   `df_for = e^{−r_for·t}` — every transcendental routed through
//!   [`celnet_core::math`] (the same `libm` the exotics crate uses).
//!
//! Because the cube's digital tests compare a leaf/curvature/PnL produced through
//! THIS pricer against another quantity produced through the SAME pricer, the
//! results are identical by construction; the byte-identity contract guarantees the
//! values also match the historical `celnet-exotics`-priced numbers the tests were
//! first written against.

use celnet_core::ExoticLegPricer;
use celnet_core::math::{exp, ln, norm_cdf, norm_pdf, sqrt};
use celnet_types::{
    BarrierStyle, DigitalKind, DigitalStyle, ExoticKind, OptionType, SingleBarrier, VanillaInputs,
};

/// A self-contained closed-form exotic pricer for tests (digital + single barrier),
/// re-derived from `celnet-core` math and the `celnet-vanilla` leaf — sharing no code
/// with `celnet-exotics`. See the module docs for the byte-identity contract against
/// the production digital closed form.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DigitalTestPricer;

/// `(d1, d2)` of the Garman-Kohlhagen model for `inputs`, in the FX two-rate basis
/// (`b = r_dom − r_for`). Identical op order to `celnet-exotics`' `d12`.
#[inline]
fn d12(i: &VanillaInputs) -> (f64, f64) {
    let vsqt = i.vol * sqrt(i.t);
    let b = i.r_dom - i.r_for;
    let d1 = (ln(i.spot / i.strike) + (b + 0.5 * i.vol * i.vol) * i.t) / vsqt;
    (d1, d1 - vsqt)
}

/// The European-digital present value (per one payout unit), re-derived to match
/// `celnet_exotics::digital_price` bit-for-bit on an FX input.
#[inline]
fn digital_price(kind: DigitalKind, i: &VanillaInputs) -> f64 {
    let (d1, d2) = d12(i);
    let df_dom = exp(-i.r_dom * i.t);
    let df_for = exp(-i.r_for * i.t);
    match (kind.style, kind.option) {
        (DigitalStyle::CashOrNothing, OptionType::Call) => df_dom * norm_cdf(d2),
        (DigitalStyle::CashOrNothing, OptionType::Put) => df_dom * norm_cdf(-d2),
        (DigitalStyle::AssetOrNothing, OptionType::Call) => i.spot * df_for * norm_cdf(d1),
        (DigitalStyle::AssetOrNothing, OptionType::Put) => i.spot * df_for * norm_cdf(-d1),
    }
}

/// The digital's closed-form `(delta, gamma, vega)` (per one payout unit),
/// re-derived to match `celnet_exotics::digital_greeks` bit-for-bit on an FX input.
#[inline]
fn digital_greeks(kind: DigitalKind, i: &VanillaInputs) -> (f64, f64, f64) {
    let (d1, d2) = d12(i);
    let vsqt = i.vol * sqrt(i.t);
    let df_dom = exp(-i.r_dom * i.t);
    let df_for = exp(-i.r_for * i.t);
    let s = i.spot;

    match kind.style {
        DigitalStyle::CashOrNothing => {
            let sgn = kind.option.sign();
            let n2 = norm_pdf(d2);
            let delta = sgn * df_dom * n2 / (s * vsqt);
            let gamma = -sgn * df_dom * n2 * d1 / (s * s * i.vol * i.vol * i.t);
            let vega = -sgn * df_dom * n2 * d1 / i.vol;
            (delta, gamma, vega)
        }
        DigitalStyle::AssetOrNothing => {
            let omega = kind.option.sign();
            let n1 = norm_pdf(d1);
            let cdf_term = match kind.option {
                OptionType::Call => norm_cdf(d1),
                OptionType::Put => norm_cdf(-d1),
            };
            let delta = df_for * (cdf_term + omega * n1 / vsqt);
            let gamma = omega * df_for * (n1 / (s * vsqt) - d1 * n1 / (s * vsqt * vsqt));
            let vega = -omega * s * df_for * n1 * d2 / i.vol;
            (delta, gamma, vega)
        }
    }
}

/// Deterministic power for positive `base` (`base^exp_ = e^{exp_·ln base}`),
/// matching the exotics crate's `pow` helper op order.
#[inline]
fn pow(base: f64, exp_: f64) -> f64 {
    exp(exp_ * ln(base))
}

/// The genuine Reiner-Rubinstein single-barrier closed form, re-derived from the
/// `celnet-core` math + the `celnet-vanilla` (Garman-Kohlhagen) parity leaf. This
/// arm completes the [`ExoticLegPricer`] honestly (no placeholder); the cube's own
/// tests price only digitals, so the barrier-oracle tests live in
/// `celnet-parity/tests/exotic_leg_pricing.rs` (which has `celnet-exotics`).
///
/// Knock-out is computed by in/out parity (`KO = vanilla − KI`) so parity holds by
/// construction, exactly as the production engine does.
#[allow(clippy::similar_names)] // x1/x2/y1/y2 are canonical Reiner-Rubinstein names
fn single_barrier_price(i: &VanillaInputs, spec: SingleBarrier) -> f64 {
    let SingleBarrier {
        kind,
        strike,
        barrier,
        rebate,
    } = spec;
    // First-generation cube test legs carry no rebate; the rebate touch leg is
    // outside this digital-only test oracle's scope (no cube test exercises it).
    assert!(
        rebate == 0.0,
        "the cube test barrier oracle covers rebate-free barriers only"
    );

    let vanilla = celnet_vanilla::price(
        kind.option,
        &VanillaInputs::new(i.spot, strike, i.vol, i.t, i.r_dom, i.r_for),
    );

    let breached = if kind.up {
        i.spot >= barrier
    } else {
        i.spot <= barrier
    };
    if breached {
        return match kind.style {
            BarrierStyle::KnockOut => 0.0,
            BarrierStyle::KnockIn => vanilla,
        };
    }

    let s = i.spot;
    let h = barrier;
    let k = strike;
    let vsqt = i.vol * sqrt(i.t);
    let b = i.r_dom - i.r_for;
    let mu = b / (i.vol * i.vol) - 0.5;
    let df_for = exp(-i.r_for * i.t);
    let df_dom = exp(-i.r_dom * i.t);
    let s_disc = s * df_for;
    let k_disc = k * df_dom;

    let x1 = ln(s / k) / vsqt + (1.0 + mu) * vsqt;
    let x2 = ln(s / h) / vsqt + (1.0 + mu) * vsqt;
    let y1 = ln(h * h / (s * k)) / vsqt + (1.0 + mu) * vsqt;
    let y2 = ln(h / s) / vsqt + (1.0 + mu) * vsqt;

    let hs = h / s;
    let pow_2mu2 = pow(hs, 2.0 * (mu + 1.0));
    let pow_2mu = pow(hs, 2.0 * mu);

    let phi = kind.option.sign();
    let eta = if kind.up { -1.0 } else { 1.0 };

    let a = phi * s_disc * norm_cdf(phi * x1) - phi * k_disc * norm_cdf(phi * (x1 - vsqt));
    let bb = phi * s_disc * norm_cdf(phi * x2) - phi * k_disc * norm_cdf(phi * (x2 - vsqt));
    let c = phi * s_disc * pow_2mu2 * norm_cdf(eta * y1)
        - phi * k_disc * pow_2mu * norm_cdf(eta * (y1 - vsqt));
    let d = phi * s_disc * pow_2mu2 * norm_cdf(eta * y2)
        - phi * k_disc * pow_2mu * norm_cdf(eta * (y2 - vsqt));

    let call = matches!(kind.option, OptionType::Call);
    let k_ge_h = strike >= barrier;

    let knock_in = if kind.up {
        if call {
            if k_ge_h { a } else { bb - c + d }
        } else if k_ge_h {
            a - bb + d
        } else {
            c
        }
    } else if call {
        if k_ge_h { c } else { a - bb + d }
    } else if k_ge_h {
        bb - c + d
    } else {
        a
    };

    match kind.style {
        BarrierStyle::KnockIn => knock_in,
        BarrierStyle::KnockOut => vanilla - knock_in,
    }
}

impl ExoticLegPricer for DigitalTestPricer {
    fn unit_price(&self, kind: ExoticKind, inputs: &VanillaInputs) -> f64 {
        match kind {
            ExoticKind::Digital(k) => digital_price(k, inputs),
            ExoticKind::SingleBarrier(spec) => single_barrier_price(inputs, spec),
        }
    }

    fn digital_greeks(&self, kind: DigitalKind, inputs: &VanillaInputs) -> (f64, f64, f64) {
        digital_greeks(kind, inputs)
    }
}
