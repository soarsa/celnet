//! The pure leg computation — the numerically-validated core.
//!
//! Given the selected position slices, a resolved transfer price, and the
//! quantity, [`compute_legs`] returns the two offsetting booking legs, the P&L
//! crystallised in the source, and the aggregate moved risk.
//! [`plan_position_moves`] returns the per-position identity plan (Full moves
//! carry the id; Partial moves keep the original id on the remainder and mint a
//! new id for the moved slice).
//!
//! Both are pure functions of their inputs (no clock, no rng, no I/O), so they
//! are deterministic and oracle-testable.

use crate::risk::{MovedRisk, RiskVector};
use crate::transfer::TransferQuantity;

/// A selected position, with the data needed to compute the economic legs.
///
/// Richer than [`crate::PositionRef`] (used for validation): it additionally
/// carries the `mark` used to crystallise P&L and the pass-through `risk`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PositionSlice {
    /// The position's stable id.
    pub position_id: u64,
    /// Signed base-currency notional (+ long, − short).
    pub signed_notional: f64,
    /// The price the slice is currently marked at (its cost basis for
    /// realising P&L).
    pub mark: f64,
    /// The pass-through risk the whole slice carries.
    pub risk: RiskVector,
}

/// Which side of the mirror pair a booked leg is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegRole {
    /// The offsetting leg booked into the **source** book (opposite side of the
    /// moved risk) so the source net-flattens and realises P&L.
    SourceOffset,
    /// The opening leg booked into the **target** book (same side as the
    /// original risk) so the target opens at the transfer price.
    TargetOpen,
}

/// One leg of the offsetting mirror pair.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BookedLeg {
    /// Which side of the pair.
    pub role: LegRole,
    /// The signed base-currency notional this leg books. The source and target
    /// legs are exact negations, so the pair nets to zero at the firm level.
    pub signed_notional: f64,
    /// The transfer price this leg books at.
    pub price: f64,
}

/// The full result of computing an economic transfer's legs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TransferLegs {
    /// The offsetting leg booked into the source book.
    pub source_offset: BookedLeg,
    /// The opening leg booked into the target book.
    pub target_open: BookedLeg,
    /// P&L crystallised in the source at the transfer price versus each slice's
    /// mark: `Σ moved_i · (price − mark_i)`.
    pub realized_pnl_source: f64,
    /// The aggregate risk moved from source to target.
    pub moved_risk: MovedRisk,
}

/// How a single selected position is split by the move.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MoveKind {
    /// The whole position moves; its id carries to the target unchanged.
    Whole,
    /// The position splits: the remainder keeps the original id (see
    /// [`PositionMove::source_position_id`]) with `remainder_notional`; a newly
    /// minted `moved_position_id` carries `moved_notional`.
    Split {
        /// Signed notional left behind in the source under the original id.
        remainder_notional: f64,
        /// Signed notional moved out under the freshly minted id.
        moved_notional: f64,
        /// The freshly minted id for the moved slice.
        moved_position_id: u64,
    },
}

/// The identity plan for one selected position.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PositionMove {
    /// The original position id — preserved on the source remainder (Split) or
    /// carried whole to the target (Whole).
    pub source_position_id: u64,
    /// How the position is split.
    pub kind: MoveKind,
}

/// The moved fraction of each slice: `1.0` for `Full`, `q / |Σ signed_notional|`
/// for `Partial(q)`. Assumes a validated transfer (a `Partial` with `0 < q ≤
/// |Σ|`); guards a zero aggregate to `0.0` to stay total and finite.
fn moved_fraction(positions: &[PositionSlice], quantity: TransferQuantity) -> f64 {
    match quantity {
        TransferQuantity::Full => 1.0,
        TransferQuantity::Partial(q) => {
            let agg_abs: f64 = positions
                .iter()
                .map(|p| p.signed_notional)
                .sum::<f64>()
                .abs();
            if agg_abs > 0.0 { q / agg_abs } else { 0.0 }
        }
    }
}

/// Compute the two offsetting legs, the realised source P&L, and the moved risk.
///
/// `price` is the already-resolved transfer price (the server resolves
/// Mid/MarkToMarket from the live composite; Agreed carries its own value).
/// Assumes a validated transfer (call [`crate::check_transfer`] first).
///
/// The source and target legs' signed notionals are exact negations of the same
/// computed `moved` aggregate, so [`TransferLegs::source_offset`] +
/// [`TransferLegs::target_open`] nets to zero at the firm level.
#[must_use]
pub fn compute_legs(
    positions: &[PositionSlice],
    price: f64,
    quantity: TransferQuantity,
) -> TransferLegs {
    let f = moved_fraction(positions, quantity);

    let mut moved_agg = 0.0f64;
    let mut realized = 0.0f64;
    let mut risk = RiskVector::ZERO;
    for p in positions {
        let moved_i = p.signed_notional * f;
        moved_agg += moved_i;
        realized += moved_i * (price - p.mark);
        risk = risk.add(&p.risk.scaled(f));
    }

    TransferLegs {
        source_offset: BookedLeg {
            role: LegRole::SourceOffset,
            signed_notional: -moved_agg,
            price,
        },
        target_open: BookedLeg {
            role: LegRole::TargetOpen,
            signed_notional: moved_agg,
            price,
        },
        realized_pnl_source: realized,
        moved_risk: MovedRisk {
            notional_base: moved_agg,
            risk,
        },
    }
}

/// Plan the per-position identity outcome of a move.
///
/// `Full` → each position moves whole (id carried to the target). `Partial` →
/// each position splits pro-rata: the remainder keeps the original id, and
/// `mint_id` is called once per position (in slice order) to allocate the moved
/// slice's new id. Passing the mint closure keeps this pure and deterministic
/// while letting the server supply its monotonic id source.
#[must_use]
pub fn plan_position_moves(
    positions: &[PositionSlice],
    quantity: TransferQuantity,
    mut mint_id: impl FnMut() -> u64,
) -> Vec<PositionMove> {
    match quantity {
        TransferQuantity::Full => positions
            .iter()
            .map(|p| PositionMove {
                source_position_id: p.position_id,
                kind: MoveKind::Whole,
            })
            .collect(),
        TransferQuantity::Partial(_) => {
            let f = moved_fraction(positions, quantity);
            positions
                .iter()
                .map(|p| PositionMove {
                    source_position_id: p.position_id,
                    kind: MoveKind::Split {
                        remainder_notional: p.signed_notional * (1.0 - f),
                        moved_notional: p.signed_notional * f,
                        moved_position_id: mint_id(),
                    },
                })
                .collect()
        }
    }
}
