//! Non-deliverable forward (NDF) — a cash-settled outright forward.
//!
//! An NDF references a restricted currency that cannot be delivered offshore; at
//! maturity the parties cash-settle the difference between the contract rate `K`
//! and the realized fixing, in the **convertible (settlement) currency**. The
//! risk-neutral present value is identical to a deliverable outright forward of
//! equal terms valued in the same numeraire — non-deliverability changes the
//! *settlement mechanics*, not the discounted-cashflow PV:
//!
//! ```text
//! PV = side · notional · df_settle · (F − K),   F = spot · forward_factor(t)
//! ```
//!
//! where `df_settle = discount_df(t)` discounts in the convertible settlement
//! currency. The contract's [`FixingSource`] (which published rate the fixing
//! references) is carried as **metadata only** — the realized fixing VALUE is an
//! estate-gated market-data feed, never sourced here (the honest boundary below).
//!
//! Asset-class-agnostic (ADR-0008): `F` and `df_settle` go through the carry
//! producer only; this engine prices an NDF on any underlying whose carry yields
//! a forward factor and a settlement discount factor.

use crate::forward::pv_at;
use crate::inputs::LinearInputs;
use celnet_types::FixingSource;

/// A non-deliverable forward: the linear contract terms plus the fixing identity
/// of the published rate the cash settlement references.
///
/// The [`FixingSource`] is metadata for booking, reconciliation and settlement-
/// convention reporting; it does **not** enter the PV (which is a deterministic
/// discounted cashflow). The realized fixing value is never sourced here.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ndf {
    /// The linear market state + contract terms (read through the carry seam).
    pub inputs: LinearInputs,
    /// The published settlement-rate option the NDF fixes against (e.g.
    /// `BrlPtax`). Metadata only — its live value is estate-gated.
    pub fixing: FixingSource,
}

impl Ndf {
    /// Construct an NDF from validated [`LinearInputs`] and a fixing identity.
    #[must_use]
    pub const fn new(inputs: LinearInputs, fixing: FixingSource) -> Self {
        Self { inputs, fixing }
    }

    /// The cash-settlement present value of the NDF, in the settlement
    /// (convertible) currency.
    ///
    /// `PV = side · notional · discount_df(t) · (forward(t) − contract_rate)`,
    /// settling at [`LinearInputs::near_settle_t`]. Identical risk-neutral PV to a
    /// deliverable forward of equal terms in the same numeraire.
    #[must_use]
    pub fn pv(&self) -> f64 {
        pv_at(&self.inputs, self.inputs.near_settle_t)
    }

    /// The fixing identity this NDF settles against (metadata accessor).
    #[must_use]
    pub const fn fixing(&self) -> FixingSource {
        self.fixing
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forward;
    use crate::inputs::Side;
    use celnet_types::{Carry, CcyPair, Underlying};

    // USD/BRL is a classic NDF pair (BRL is restricted); the convertible
    // settlement leg is USD. The underlying identity here is the CcyPair; the
    // PV is agnostic to deliverability (only the settlement mechanics differ).
    fn usdbrl() -> Underlying {
        Underlying::Fx(CcyPair::parse("USDBRL").unwrap())
    }

    fn mk(spot: f64, k: f64, n: f64, side: Side, r_dom: f64, r_for: f64, t: f64) -> LinearInputs {
        LinearInputs::outright(
            spot,
            usdbrl(),
            Carry::FxRates { r_dom, r_for },
            crate::inputs::LinearTerms::new(k, n, side),
            t,
        )
        .unwrap()
    }

    /// HAND-DERIVED absolute PV literal, computed externally (not from the impl).
    ///
    /// Terms: spot = 5.0000 (BRL per USD), K = 5.1000, r_dom = 0.1000 (BRL),
    ///        r_for = 0.0500 (USD), t = 0.5, notional = 1_000_000 USD, side = Buy.
    ///   F  = spot · e^{(r_dom−r_for)·t} = 5.0 · e^{0.05·0.5} = 5.0 · e^{0.025}
    ///      = 5.0 · 1.025315120524…  = 5.126575602622…
    ///   df = e^{−r_dom·t}            = e^{−0.05} = 0.951229424500…
    ///   PV = +1 · 1_000_000 · 0.951229424500714 · (5.126575602622144 − 5.1000)
    ///      = 1_000_000 · 0.951229424500714 · 0.026575602622144
    ///      = 25_279.495188… (BRL-equivalent in the discounting numeraire)
    /// (External recomputation pins 25_279.4951880221… — see the assert.)
    #[test]
    fn pv_hand_derived_literal() {
        let ndf = Ndf::new(
            mk(5.0, 5.1, 1_000_000.0, Side::Buy, 0.10, 0.05, 0.5),
            FixingSource::BrlPtax,
        );
        let expected = 25_279.495_188_022_105_f64;
        assert!(
            (ndf.pv() - expected).abs() <= 1e-6,
            "ndf pv {} vs pinned {expected}",
            ndf.pv()
        );
    }

    /// Structural identity (CAN DISAGREE with the literal): an NDF and a
    /// deliverable outright forward of equal terms have the SAME risk-neutral PV
    /// in the same numeraire — non-deliverability is a settlement-mechanics
    /// difference, not a valuation one. This is independent of the absolute
    /// literal above (one pins a value, the other an equality), so a shared slip
    /// cannot hide.
    #[test]
    fn ndf_pv_equals_deliverable_forward_pv() {
        let cases = [
            (5.0, 5.1, 1_000_000.0, Side::Buy, 0.10, 0.05, 0.5),
            (1300.0, 1280.0, 2_000_000.0, Side::Sell, 0.03, 0.045, 1.0),
            (83.0, 84.0, 5_000_000.0, Side::Buy, 0.066, 0.05, 0.25),
        ];
        for (s, k, n, side, rd, rf, t) in cases {
            let deliverable = mk(s, k, n, side, rd, rf, t);
            let ndf = Ndf::new(deliverable, FixingSource::BrlPtax);
            // Same numeraire, same terms ⇒ byte-identical PV (no separate route).
            assert_eq!(ndf.pv().to_bits(), forward::pv(&deliverable).to_bits());
        }
    }

    /// The fixing identity is carried as metadata and does NOT change the PV:
    /// the same terms under two different fixings price identically.
    #[test]
    fn fixing_identity_is_metadata_only() {
        let li = mk(5.0, 5.1, 1_000_000.0, Side::Buy, 0.10, 0.05, 0.5);
        let a = Ndf::new(li, FixingSource::BrlPtax);
        let b = Ndf::new(li, FixingSource::InrRbiRef);
        assert_eq!(a.pv().to_bits(), b.pv().to_bits());
        assert_eq!(a.fixing(), FixingSource::BrlPtax);
        assert_eq!(b.fixing(), FixingSource::InrRbiRef);
    }

    /// Settlement discounting uses the convertible-leg (discount) rate, not the
    /// restricted (base/foreign) rate: bumping only the discount rate moves the
    /// PV, and the move matches the analytic `∂PV/∂r_dom` of the forward.
    #[test]
    fn settlement_discounts_in_convertible_currency() {
        let base = Ndf::new(
            mk(5.0, 5.1, 1_000_000.0, Side::Buy, 0.10, 0.05, 0.5),
            FixingSource::BrlPtax,
        );
        let bumped = Ndf::new(
            mk(5.0, 5.1, 1_000_000.0, Side::Buy, 0.10 + 1e-6, 0.05, 0.5),
            FixingSource::BrlPtax,
        );
        let fd = (bumped.pv() - base.pv()) / 1e-6;
        let analytic = forward::greeks(&base.inputs).rho_dom;
        assert!(
            (fd - analytic).abs() <= 1e-2 * analytic.abs().max(1.0),
            "fd {fd} vs analytic rho_dom {analytic}"
        );
    }
}
