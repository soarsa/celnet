//! The **price-tolerance / "are we making money" check** for the internalise decision
//! (`docs/hedging/AUTO-HEDGING-AND-INTERNALISATION-REQUIREMENTS.md` §6). Pure, off-core.
//!
//! When a dealer books a fill, the booking engine decides whether to **warehouse**
//! (internalise) the risk or shed it as an advisory external back-to-back. The first gate
//! of that decision is: *did the desk capture enough edge on THIS fill to justify holding
//! the risk?* A fill dealt at (or through) the engine's fair mid is a losing trade to
//! warehouse — the desk should hand it straight back to the street. A fill dealt with
//! comfortable dealer-favourable edge is worth internalising against opposing flow.
//!
//! This module owns only that **pure verdict**: the signed dealer-captured edge in basis
//! points, whether it is favourable ("making money"), and whether it clears a configurable
//! minimum-edge floor. The warehouse-cap / netting split is the pure
//! [`WarehouseThreshold`](celnet_hedge_routing::WarehouseThreshold) engine's job; the two
//! combine in [`crate::services::rates_book`].
//!
//! # Edge sign derivation (get pay/receive-fixed right)
//!
//! The desk deals at `dealt`; the engine's fair reference is `mid`. The dealer's captured
//! edge is the **dealer-favourable** distance of `dealt` from `mid` — which way is
//! favourable depends on the DESK's side (already the opposite of the counterparty's):
//!
//! | desk side          | favourable when      | native edge      |
//! |--------------------|----------------------|------------------|
//! | `Buy`  (pay fixed / long a bond)   | dealt **below** mid  | `mid − dealt` |
//! | `Sell` (receive fixed / short)     | dealt **above** mid  | `dealt − mid` |
//!
//! Paying a *lower* fixed rate than fair, or buying a bond *cheaper* than fair, is money in
//! the desk's pocket; receiving a *higher* fixed rate, or selling *richer*, likewise. Both
//! rows collapse to one rule with a direction sign `dir = −1` for `Buy`, `+1` for `Sell`:
//!
//! ```text
//! edge_native = dir · (dealt − mid)
//! ```
//!
//! The native edge is then scaled to **basis points of the quoted level**: for a rate
//! market one bp is `1e-4` in absolute rate terms; for a clean-price market (par = 100) one
//! bp of price is `0.01` price points. So `edge_bps = edge_native / bp_scale` with
//! `bp_scale = 1e-4` (rate) or `1e-2` (price). A positive `edge_bps` means the desk dealt
//! *better* than mid (made money on the spread); a non-positive one means it dealt at or
//! through mid.

use celnet_proto::Side;

/// The default minimum dealer-captured edge (basis points) a fill must clear to be
/// internalised rather than handed straight back to the street. Half a basis point — small
/// enough to internalise a normally-margined fill, large enough to refuse a trade dealt at
/// mid. Overridable per firm via [`crate::config::hedge_policy::HedgeConfigDef::min_edge_bps`].
pub const DEFAULT_MIN_EDGE_BPS: f64 = 0.5;

/// One basis point in absolute rate terms — the bp scale of a rate quote (swap/FRA fixed rate).
const RATE_BP: f64 = 1e-4;
/// One basis point of a clean-price quote (par = 100): `0.01` price points.
const PRICE_BP: f64 = 1e-2;

/// Whether the dealt price is quoted as a **rate** (swap/FRA fixed rate) or a **price**
/// (cash-bond clean price) — this selects the basis-point scale the native edge is divided
/// by so `edge_bps` is comparable across FI families.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuoteKind {
    /// A rate market (OIS / IRS / FRA): the dealt level and the reference mid are both
    /// absolute rates; one bp is `1e-4`.
    Rate,
    /// A clean-price market (cash bond): the dealt level and the reference mid are both
    /// clean prices (par = 100); one bp of price is `0.01` points.
    Price,
}

impl QuoteKind {
    /// The basis-point scale (native units per bp) for this quote kind.
    #[must_use]
    pub(crate) const fn bp_scale(self) -> f64 {
        match self {
            QuoteKind::Rate => RATE_BP,
            QuoteKind::Price => PRICE_BP,
        }
    }
}

/// The signed, dealer-favourable edge of a fill in **basis points** of the quoted level —
/// the module-level derivation. `desk_side` is the DESK's side (opposite the counterparty's),
/// `dealt` the executed level, `mid` the engine's fair reference. Positive ⇒ the desk dealt
/// better than mid (captured spread); non-positive ⇒ dealt at or through mid.
///
/// A non-finite input, or a two-way side that should never reach a firm booking, yields
/// `0.0` (no directional edge) rather than a spurious number.
#[must_use]
pub fn dealer_edge_bps(desk_side: Side, dealt: f64, mid: f64, kind: QuoteKind) -> f64 {
    if !dealt.is_finite() || !mid.is_finite() {
        return 0.0;
    }
    let dir = match desk_side {
        // Pay fixed / long: favourable to deal BELOW mid ⇒ edge = mid − dealt.
        Side::Buy => -1.0,
        // Receive fixed / short: favourable to deal ABOVE mid ⇒ edge = dealt − mid.
        Side::Sell => 1.0,
        // A two-way never reaches a firm booking; carry no directional edge defensively.
        Side::TwoWay => return 0.0,
    };
    (dir * (dealt - mid)) / kind.bp_scale()
}

/// The price-tolerance verdict for a fill (§6): the captured edge, whether it is
/// dealer-favourable ("making money"), and whether it clears the minimum-edge floor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InternaliseVerdict {
    /// Whether the fill was dealt at a dealer-favourable level (`edge_bps > 0`).
    pub making_money: bool,
    /// The signed dealer-captured edge in basis points ([`dealer_edge_bps`]).
    pub edge_bps: f64,
    /// Whether `edge_bps` cleared the configured `min_edge_bps` floor — the gate that
    /// decides warehouse (internalise) vs advisory external back-to-back.
    pub within_tolerance: bool,
}

/// Assess a fill's price tolerance: compute its dealer-captured [`dealer_edge_bps`] and
/// gate it against `min_edge_bps`. A `min_edge_bps` below zero is clamped to zero (a floor
/// is never negative), so `within_tolerance` always implies a non-loss.
#[must_use]
pub fn verdict(
    desk_side: Side,
    dealt: f64,
    mid: f64,
    kind: QuoteKind,
    min_edge_bps: f64,
) -> InternaliseVerdict {
    let edge_bps = dealer_edge_bps(desk_side, dealt, mid, kind);
    let floor = min_edge_bps.max(0.0);
    InternaliseVerdict {
        making_money: edge_bps > 0.0,
        edge_bps,
        within_tolerance: edge_bps >= floor,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pay_fixed_edge_is_favourable_below_mid() {
        // Desk pays fixed (Buy) a swap at 4.00% vs a 4.05% fair mid → paying 5bp under mid
        // is +5bp of dealer edge.
        let e = dealer_edge_bps(Side::Buy, 0.0400, 0.0405, QuoteKind::Rate);
        assert!((e - 5.0).abs() < 1e-9, "got {e}");
        // Paying ABOVE mid is a negative edge (a losing fill to warehouse).
        let e2 = dealer_edge_bps(Side::Buy, 0.0410, 0.0405, QuoteKind::Rate);
        assert!((e2 + 5.0).abs() < 1e-9, "got {e2}");
    }

    #[test]
    fn receive_fixed_edge_is_favourable_above_mid() {
        // Desk receives fixed (Sell) at 4.10% vs a 4.05% fair mid → receiving 5bp over mid
        // is +5bp of dealer edge.
        let e = dealer_edge_bps(Side::Sell, 0.0410, 0.0405, QuoteKind::Rate);
        assert!((e - 5.0).abs() < 1e-9, "got {e}");
        let e2 = dealer_edge_bps(Side::Sell, 0.0400, 0.0405, QuoteKind::Rate);
        assert!((e2 + 5.0).abs() < 1e-9, "got {e2}");
    }

    #[test]
    fn bond_price_edge_uses_price_bp_scale() {
        // Desk buys a bond (Buy / long) at 99.50 clean vs a 99.75 fair mid → 0.25 points
        // cheaper than fair = 25 bp of price edge (0.25 / 0.01).
        let e = dealer_edge_bps(Side::Buy, 99.50, 99.75, QuoteKind::Price);
        assert!((e - 25.0).abs() < 1e-9, "got {e}");
        // Desk sells (Sell / short) at 100.10 vs 99.75 → 0.35 points richer = +35 bp.
        let e2 = dealer_edge_bps(Side::Sell, 100.10, 99.75, QuoteKind::Price);
        assert!((e2 - 35.0).abs() < 1e-9, "got {e2}");
    }

    #[test]
    fn verdict_gates_on_min_edge_floor() {
        // 5bp edge clears a 0.5bp floor → within tolerance, making money.
        let v = verdict(
            Side::Buy,
            0.0400,
            0.0405,
            QuoteKind::Rate,
            DEFAULT_MIN_EDGE_BPS,
        );
        assert!(v.making_money && v.within_tolerance);
        assert!((v.edge_bps - 5.0).abs() < 1e-9);

        // A thin 0.2bp edge is favourable but below a 0.5bp floor → NOT within tolerance.
        let thin = verdict(Side::Buy, 0.04048, 0.0405, QuoteKind::Rate, 0.5);
        assert!(thin.making_money, "0.2bp is still favourable");
        assert!(!thin.within_tolerance, "0.2bp is below the 0.5bp floor");

        // Dealing at mid: zero edge, not making money, not within tolerance.
        let flat = verdict(Side::Sell, 0.0405, 0.0405, QuoteKind::Rate, 0.5);
        assert!(!flat.making_money && !flat.within_tolerance);
        assert_eq!(flat.edge_bps, 0.0);

        // An adverse fill (dealt through mid): negative edge, refused.
        let adverse = verdict(Side::Buy, 0.0410, 0.0405, QuoteKind::Rate, 0.5);
        assert!(!adverse.making_money && !adverse.within_tolerance);
        assert!(adverse.edge_bps < 0.0);
    }

    #[test]
    fn non_finite_and_two_way_are_zero_edge() {
        assert_eq!(
            dealer_edge_bps(Side::Buy, f64::NAN, 0.04, QuoteKind::Rate),
            0.0
        );
        assert_eq!(
            dealer_edge_bps(Side::TwoWay, 0.04, 0.0405, QuoteKind::Rate),
            0.0
        );
    }
}
