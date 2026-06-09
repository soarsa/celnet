//! European digital (binary) options under Garman-Kohlhagen.
//!
//! A **cash-or-nothing** binary pays one unit of domestic cash if the option
//! finishes in the money; an **asset-or-nothing** binary pays one unit of the
//! foreign asset (worth `S_T` domestic). With `F = S·e^{(r_d−r_f)T}`,
//! `d_1 = [ln(S/K) + (b + ½σ²)T]/(σ√T)` and `d_2 = d_1 − σ√T`:
//!
//! ```text
//!   CashCall  = e^{−r_d T}·Φ(d_2)            CashPut  = e^{−r_d T}·Φ(−d_2)
//!   AssetCall = S·e^{−r_f T}·Φ(d_1)          AssetPut = S·e^{−r_f T}·Φ(−d_1)
//! ```
//!
//! The cash-or-nothing call is exactly the **strike-derivative limit** of the
//! vanilla: `CashCall = −∂C/∂K` (the digital replicates an infinitesimally tight
//! call spread). That identity is the independent finite-difference cross-check
//! in the test suite.
//!
//! Provenance (doc-only): Reiner-Rubinstein (1991b) "Unscrambling the Binary
//! Code"; the generalised-BSM presentation of Haug (2007). Identifiers are
//! purpose-named and vendor/research-neutral.

use crate::inputs::ExoticInputs;
use celnet_core::math::{ln, norm_cdf, norm_pdf, sqrt};
use celnet_types::OptionType;

/// What a digital pays when it finishes in the money.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DigitalStyle {
    /// Cash-or-nothing: pays one unit of **domestic** cash if in the money.
    CashOrNothing,
    /// Asset-or-nothing: pays one unit of the **foreign asset** (worth `S_T`).
    AssetOrNothing,
}

/// The direction of a digital (which side finishes in the money).
///
/// A digital *call* pays when `S_T > K`; a digital *put* pays when `S_T < K`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DigitalKind {
    /// Cash- or asset-settled.
    pub style: DigitalStyle,
    /// Call (`S_T > K`) or put (`S_T < K`).
    pub option: OptionType,
}

impl DigitalKind {
    /// A cash-or-nothing digital of the given direction.
    #[must_use]
    pub const fn cash(option: OptionType) -> Self {
        Self {
            style: DigitalStyle::CashOrNothing,
            option,
        }
    }

    /// An asset-or-nothing digital of the given direction.
    #[must_use]
    pub const fn asset(option: OptionType) -> Self {
        Self {
            style: DigitalStyle::AssetOrNothing,
            option,
        }
    }
}

/// `(d_1, d_2)` of the Garman-Kohlhagen model for the digital's strike.
#[inline]
fn d12(i: &ExoticInputs) -> (f64, f64) {
    let vsqt = i.vol * sqrt(i.t);
    let d1 = (ln(i.spot / i.strike) + (i.carry_rate() + 0.5 * i.vol * i.vol) * i.t) / vsqt;
    (d1, d1 - vsqt)
}

/// Present value of a European digital (per **one unit** of the stated payout:
/// one unit of domestic cash for cash-or-nothing, one unit of foreign asset for
/// asset-or-nothing). The `strike`, `spot`, `vol`, `t`, and rates come from `i`.
///
/// For a notional `N` of the payout unit, multiply by `N`.
#[must_use]
pub fn digital_price(kind: DigitalKind, i: &ExoticInputs) -> f64 {
    let (d1, d2) = d12(i);
    let df_dom = i.discount_df();
    let df_for = i.carry_df();
    match (kind.style, kind.option) {
        (DigitalStyle::CashOrNothing, OptionType::Call) => df_dom * norm_cdf(d2),
        (DigitalStyle::CashOrNothing, OptionType::Put) => df_dom * norm_cdf(-d2),
        (DigitalStyle::AssetOrNothing, OptionType::Call) => i.spot * df_for * norm_cdf(d1),
        (DigitalStyle::AssetOrNothing, OptionType::Put) => i.spot * df_for * norm_cdf(-d1),
    }
}

/// The key closed-form Greeks of a European digital.
///
/// `delta` and `gamma` are w.r.t. spot `S`; `vega` is `∂V/∂σ` (per `1.0`
/// absolute vol). These are the sensitivities a digital desk hedges; higher
/// orders follow the same construction but are not part of the first-generation
/// surface.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DigitalGreeks {
    /// Present value (per one payout unit).
    pub price: f64,
    /// Spot delta `∂V/∂S`.
    pub delta: f64,
    /// Gamma `∂²V/∂S²`.
    pub gamma: f64,
    /// Vega `∂V/∂σ` (per `1.0` absolute vol).
    pub vega: f64,
}

/// Price plus the key closed-form Greeks (`delta`, `gamma`, `vega`) of a digital.
///
/// Derivations (cash-or-nothing call, with `n = φ(d_2)`, `vsqt = σ√T`):
/// `∂d_2/∂S = 1/(S·vsqt)`, so `Δ = e^{−r_d T}·n/(S·vsqt)`; differentiating again
/// gives `Γ = −e^{−r_d T}·n·d_1/(S²·σ²T)`; and `∂d_2/∂σ = −d_1/σ` gives
/// `vega = −e^{−r_d T}·n·d_1/σ`. The asset-or-nothing forms carry the extra
/// `S·e^{−r_f T}·Φ(±d_1)` term and use `∂d_1/∂σ = −d_2/σ`.
#[must_use]
#[allow(clippy::similar_names)] // d1/d2 are canonical option-pricing names
pub fn digital_greeks(kind: DigitalKind, i: &ExoticInputs) -> DigitalGreeks {
    let (d1, d2) = d12(i);
    let vsqt = i.vol * sqrt(i.t);
    let df_dom = i.discount_df();
    let df_for = i.carry_df();
    let s = i.spot;

    let price = digital_price(kind, i);

    let (delta, gamma, vega) = match kind.style {
        DigitalStyle::CashOrNothing => {
            // sign: +1 for call (d_2), −1 for put (−d_2): Φ'(−d_2) = φ(d_2).
            let sgn = kind.option.sign();
            let n2 = norm_pdf(d2);
            let delta = sgn * df_dom * n2 / (s * vsqt);
            let gamma = -sgn * df_dom * n2 * d1 / (s * s * i.vol * i.vol * i.t);
            let vega = -sgn * df_dom * n2 * d1 / i.vol;
            (delta, gamma, vega)
        }
        DigitalStyle::AssetOrNothing => {
            // V = S·e^{−r_f T}·Φ(ω·d_1), ω = +1 call / −1 put. With
            // ∂d_1/∂S = 1/(S·vsqt) and ∂d_1/∂σ = −d_2/σ:
            //   Δ    = e^{−r_f T}·[ Φ(ω d_1) + ω·φ(d_1)/vsqt ]
            //   Γ    = ω·e^{−r_f T}·[ φ(d_1)/(S·vsqt) − d_1·φ(d_1)/(S·vsqt²) ]
            //   vega = −ω·S·e^{−r_f T}·φ(d_1)·d_2/σ.
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
    };

    DigitalGreeks {
        price,
        delta,
        gamma,
        vega,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_core::assert_close;
    use celnet_core::math::exp;
    use celnet_types::VanillaInputs;
    use celnet_vanilla::price as vanilla_price;

    fn base() -> VanillaInputs {
        // EURUSD-like 1Y: S=K=1.30, σ=10%, r_d=3%, r_f=1%.
        VanillaInputs::new(1.30, 1.30, 0.10, 1.0, 0.03, 0.01)
    }

    /// Cash-or-nothing call + cash-or-nothing put = the discounted certainty of
    /// paying one unit of domestic cash either way = `e^{−r_d T}`.
    #[test]
    fn cash_call_put_complementary() {
        let i = base();
        let e: ExoticInputs = (&i).into();
        let c = digital_price(DigitalKind::cash(OptionType::Call), &e);
        let p = digital_price(DigitalKind::cash(OptionType::Put), &e);
        assert_close!(c + p, exp(-i.r_dom * i.t), 1e-12, 1e-12);
    }

    /// Asset-or-nothing call + put = the discounted asset = `S·e^{−r_f T}`.
    #[test]
    fn asset_call_put_complementary() {
        let i = base();
        let e: ExoticInputs = (&i).into();
        let c = digital_price(DigitalKind::asset(OptionType::Call), &e);
        let p = digital_price(DigitalKind::asset(OptionType::Put), &e);
        assert_close!(c + p, i.spot * exp(-i.r_for * i.t), 1e-12, 1e-12);
    }

    /// A vanilla call decomposes into asset-or-nothing minus `K`·cash-or-nothing:
    /// `C = AssetCall − K·CashCall`. Likewise the put.
    #[test]
    fn vanilla_decomposition() {
        let i = base();
        let e: ExoticInputs = (&i).into();
        let asset_c = digital_price(DigitalKind::asset(OptionType::Call), &e);
        let cash_c = digital_price(DigitalKind::cash(OptionType::Call), &e);
        assert_close!(
            vanilla_price(OptionType::Call, &i),
            asset_c - i.strike * cash_c,
            1e-10,
            1e-12
        );
        let asset_p = digital_price(DigitalKind::asset(OptionType::Put), &e);
        let cash_p = digital_price(DigitalKind::cash(OptionType::Put), &e);
        assert_close!(
            vanilla_price(OptionType::Put, &i),
            i.strike * cash_p - asset_p,
            1e-10,
            1e-12
        );
    }

    /// THE defining cross-check: the cash-or-nothing call equals `−∂C/∂K` of the
    /// vanilla (tight call-spread limit), validated by central finite difference
    /// in the strike.
    #[test]
    fn cash_call_is_minus_dvanilla_dk() {
        let i = base();
        let hk = 1e-5 * i.strike;
        let up = VanillaInputs {
            strike: i.strike + hk,
            ..i
        };
        let dn = VanillaInputs {
            strike: i.strike - hk,
            ..i
        };
        let fd = -(vanilla_price(OptionType::Call, &up) - vanilla_price(OptionType::Call, &dn))
            / (2.0 * hk);
        let cash = digital_price(DigitalKind::cash(OptionType::Call), &(&i).into());
        assert_close!(cash, fd, 1e-5, 1e-8);

        // And the cash-or-nothing put equals +∂P/∂K.
        let fd_put = (vanilla_price(OptionType::Put, &up) - vanilla_price(OptionType::Put, &dn))
            / (2.0 * hk);
        let cash_put = digital_price(DigitalKind::cash(OptionType::Put), &(&i).into());
        assert_close!(cash_put, fd_put, 1e-5, 1e-8);
    }

    /// All four digital Greeks agree with central finite differences of the
    /// closed-form price, across a spread of regimes (ITM/OTM, both styles).
    #[test]
    fn greeks_match_finite_difference() {
        let cases = [
            base(),
            VanillaInputs::new(1.20, 1.30, 0.14, 0.5, 0.02, 0.015),
            VanillaInputs::new(1.40, 1.30, 0.09, 2.0, 0.04, 0.01),
        ];
        for i in &cases {
            let e: ExoticInputs = i.into();
            for style in [DigitalStyle::CashOrNothing, DigitalStyle::AssetOrNothing] {
                for option in [OptionType::Call, OptionType::Put] {
                    let kind = DigitalKind { style, option };
                    let g = digital_greeks(kind, &e);

                    let hs = 1e-5 * i.spot;
                    let up: ExoticInputs = (&VanillaInputs {
                        spot: i.spot + hs,
                        ..*i
                    })
                        .into();
                    let dn: ExoticInputs = (&VanillaInputs {
                        spot: i.spot - hs,
                        ..*i
                    })
                        .into();
                    let d_fd = (digital_price(kind, &up) - digital_price(kind, &dn)) / (2.0 * hs);
                    assert_close!(g.delta, d_fd, 1e-4, 1e-6);

                    let g_fd = (digital_price(kind, &up) - 2.0 * g.price
                        + digital_price(kind, &dn))
                        / (hs * hs);
                    assert_close!(g.gamma, g_fd, 5e-3, 1e-4);

                    let hv = 1e-5;
                    let vu: ExoticInputs = (&VanillaInputs {
                        vol: i.vol + hv,
                        ..*i
                    })
                        .into();
                    let vd: ExoticInputs = (&VanillaInputs {
                        vol: i.vol - hv,
                        ..*i
                    })
                        .into();
                    let v_fd = (digital_price(kind, &vu) - digital_price(kind, &vd)) / (2.0 * hv);
                    assert_close!(g.vega, v_fd, 1e-4, 1e-6);
                }
            }
        }
    }

    /// A digital price is bounded by its maximum discounted payout.
    #[test]
    fn digital_within_bounds() {
        let i = base();
        let cash = digital_price(DigitalKind::cash(OptionType::Call), &(&i).into());
        assert!(cash >= 0.0 && cash <= exp(-i.r_dom * i.t) + 1e-12);
    }

    /// Cross-asset enablement (independent oracle): a cash-or-nothing digital on a
    /// dividend-paying EQUITY priced through the agnostic carry seam
    /// (`Carry::CostOfCarry { r, b = r − q }`) must equal the hand-re-derived
    /// generalized-BSM digital `e^{−r·T}·Φ(d₂)`, with
    /// `d₂ = [ln(S/K) + (b − ½σ²)T]/(σ√T)`. The closed form is re-derived here from
    /// first principles — NOT read back from the engine — so the oracle is
    /// non-circular (the FRTB 0.75ρ lesson).
    #[test]
    fn equity_digital_matches_generalized_bsm() {
        use celnet_types::{Carry, Ccy, EquityRef, Symbol, Underlying};
        let (s, k, vol, t, r, q) = (100.0, 95.0, 0.22, 0.75, 0.04, 0.018);
        let e = ExoticInputs::new(
            s,
            k,
            vol,
            t,
            Underlying::Equity(EquityRef::new(
                Symbol::new("ACME", "XLON"),
                Ccy::parse("GBP").unwrap(),
            )),
            Carry::CostOfCarry { r, b: r - q },
        );
        let engine = digital_price(DigitalKind::cash(OptionType::Call), &e);

        // Independent hand-derived oracle.
        let b = r - q;
        let vsqt = vol * t.sqrt();
        let d2 = ((s / k).ln() + (b - 0.5 * vol * vol) * t) / vsqt;
        let oracle = (-r * t).exp() * norm_cdf(d2);
        assert_close!(engine, oracle, 1e-12, 1e-12);
    }
}
