//! At-the-money strike for the configured [`AtmConvention`].
//!
//! Two market ATM definitions (`docs/ANALYTICS-SPEC.md` §1.3,
//! `docs/CONVENTIONS.md`):
//!
//! - **ATM-forward (ATMF):** `K = F`. The delta-neutral point only under the
//!   unadjusted forward delta.
//! - **Delta-neutral straddle (DNS):** the strike where call delta + put delta
//!   `= 0`, the dominant interbank ATM for G10/EM. Its location depends on the
//!   premium adjustment:
//!   - unadjusted delta:        `K_DNS = F · e^{+½σ²T}`  (above the forward),
//!   - premium-adjusted delta:  `K_DNS = F · e^{−½σ²T}`  (below the forward —
//!     opposite sign; mislocating it corrupts the whole surface).
//!
//! The straddle root is convention-aware via [`DeltaConvention`] (only the
//! premium-adjusted flag matters; the spot vs forward factor cancels because the
//! straddle sets call + put delta to zero). Derivation: under the unadjusted
//! convention `Δ_C + Δ_P = factor·(2N(d1) − 1) = 0 ⇒ d1 = 0 ⇒ K = F·e^{½σ²T}`;
//! under the premium-adjusted convention `Δ_C + Δ_P = factor·(K/F)(2N(d2) − 1)`,
//! zero at `d2 = 0 ⇒ K = F·e^{−½σ²T}` (Reiswich & Wystup, 2010, §3.2).

use celnet_core::math::exp;
use celnet_types::{AtmConvention, DeltaConvention, VanillaInputs};

/// The ATM strike for the configured ATM and delta conventions.
///
/// `forward` is the outright forward `F = S·e^{(r_d−r_f)T}`; `vol` the ATM Black
/// vol; `t` the vol-time year fraction. For [`AtmConvention::AtmForward`] the
/// delta convention is irrelevant (`K = F`); for
/// [`AtmConvention::DeltaNeutralStraddle`] only the premium-adjusted flag of
/// `delta_conv` matters.
#[must_use]
pub fn atm_strike(
    atm: AtmConvention,
    delta_conv: DeltaConvention,
    forward: f64,
    vol: f64,
    t: f64,
) -> f64 {
    match atm {
        AtmConvention::AtmForward => forward,
        AtmConvention::DeltaNeutralStraddle => {
            let half_var = 0.5 * vol * vol * t;
            match delta_conv {
                DeltaConvention::SpotUnadjusted | DeltaConvention::ForwardUnadjusted => {
                    forward * exp(half_var)
                }
                DeltaConvention::SpotPremiumAdjusted | DeltaConvention::ForwardPremiumAdjusted => {
                    forward * exp(-half_var)
                }
            }
        }
    }
}

/// The ATM strike read directly off a [`VanillaInputs`] (uses its derived
/// forward and its `vol`/`t`).
///
/// Convenience wrapper over [`atm_strike`] for callers that already hold the
/// full input record.
#[must_use]
pub fn atm_strike_from_inputs(
    atm: AtmConvention,
    delta_conv: DeltaConvention,
    i: &VanillaInputs,
) -> f64 {
    atm_strike(atm, delta_conv, i.forward(), i.vol, i.t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convention_delta;
    use celnet_core::{assert_close, is_close};
    use celnet_types::{OptionType, VanillaInputs};

    fn inputs(strike: f64) -> VanillaInputs {
        VanillaInputs::new(1.10, strike, 0.12, 1.0, 0.02, 0.01)
    }

    /// The DNS strike equals the closed form `F·exp(±½σ²t)` exactly (unadjusted
    /// `+`, premium-adjusted `−`). Pins the `½σ²t` term directly so a mutant that
    /// perturbs the variance arithmetic (`σ·σ` → `σ/σ`, a dropped factor) — which
    /// the delta-neutrality test can absorb at this regime — is caught.
    #[test]
    fn dns_strike_matches_closed_form() {
        // Use vol≠1, t≠1, and vol≠t so every `*` in `½σ²t` is load-bearing: a
        // mutant that turns any product into a division (e.g. `σ²·t`→`σ²/t` or
        // `σ·σ`→`σ/σ`) changes the result and is caught (a t=1 / σ=1 regime would
        // make those mutants numerically equivalent).
        let f = 1.10_f64;
        let vol = 0.20_f64;
        let t = 2.5_f64;
        let half_var = 0.5 * vol * vol * t;
        for (dc, sign) in [
            (DeltaConvention::ForwardUnadjusted, 1.0_f64),
            (DeltaConvention::SpotUnadjusted, 1.0),
            (DeltaConvention::ForwardPremiumAdjusted, -1.0),
            (DeltaConvention::SpotPremiumAdjusted, -1.0),
        ] {
            let k = atm_strike(AtmConvention::DeltaNeutralStraddle, dc, f, vol, t);
            let expected = f * celnet_core::math::exp(sign * half_var);
            assert_close!(k, expected, 1e-13, 1e-14);
        }
    }

    /// ATMF is exactly the forward, independent of delta convention.
    #[test]
    fn atmf_is_forward() {
        let f = inputs(1.0).forward();
        for dc in [
            DeltaConvention::SpotUnadjusted,
            DeltaConvention::ForwardPremiumAdjusted,
        ] {
            assert_close!(atm_strike(AtmConvention::AtmForward, dc, f, 0.12, 1.0), f);
        }
    }

    /// DNS sign per convention: unadjusted straddle strike is *above* the
    /// forward (`F·e^{+½σ²T}`), premium-adjusted is *below* (`F·e^{−½σ²T}`).
    #[test]
    fn dns_sign_per_convention() {
        let f = inputs(1.0).forward();
        let vol = 0.12_f64;
        let t = 1.0_f64;
        let half = exp(0.5 * vol * vol * t);

        let k_unadj = atm_strike(
            AtmConvention::DeltaNeutralStraddle,
            DeltaConvention::ForwardUnadjusted,
            f,
            vol,
            t,
        );
        let k_padj = atm_strike(
            AtmConvention::DeltaNeutralStraddle,
            DeltaConvention::SpotPremiumAdjusted,
            f,
            vol,
            t,
        );
        assert!(k_unadj > f, "unadjusted DNS above forward");
        assert!(k_padj < f, "premium-adjusted DNS below forward");
        assert_close!(k_unadj, f * half, 1e-12, 1e-12);
        assert_close!(k_padj, f / half, 1e-12, 1e-12);
    }

    /// At the DNS strike the straddle is delta-neutral: call delta + put delta
    /// = 0 in the matching convention.
    #[test]
    fn dns_strike_is_delta_neutral() {
        for dc in [
            DeltaConvention::ForwardUnadjusted,
            DeltaConvention::SpotUnadjusted,
            DeltaConvention::ForwardPremiumAdjusted,
            DeltaConvention::SpotPremiumAdjusted,
        ] {
            let probe = inputs(1.0);
            let k = atm_strike(
                AtmConvention::DeltaNeutralStraddle,
                dc,
                probe.forward(),
                probe.vol,
                probe.t,
            );
            let i = inputs(k);
            let straddle = convention_delta(dc, OptionType::Call, &i)
                + convention_delta(dc, OptionType::Put, &i);
            assert!(
                is_close(straddle, 0.0, 1e-9, 1e-11),
                "{dc:?} straddle delta {straddle} at DNS strike {k}"
            );
        }
    }
}
