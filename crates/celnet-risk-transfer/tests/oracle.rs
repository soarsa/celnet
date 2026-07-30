//! Independent oracle for `celnet-risk-transfer` (guardrail 5).
//!
//! Every expected value below is computed **by hand / first principles** in the
//! comment beside it — NOT by calling the crate's own compute path. All inputs
//! are chosen to be exactly representable in binary IEEE-754 (integer notionals,
//! integer greeks, prices/marks and moved fractions that are multiples of a
//! negative power of two), so the expectations are asserted with **exact `==`
//! equality**, not a slackened tolerance.
//!
//! Model recap (see `lib.rs`):
//!   f            = 1                    for Full
//!                = q / |Σ signed_notional|  for Partial(q)
//!   moved_i      = signed_notional_i · f
//!   source leg   = −Σ moved_i     @ price
//!   target leg   = +Σ moved_i     @ price
//!   realized P&L = Σ moved_i · (price − mark_i)
//!   moved risk   = Σ (risk_i · f)

use celnet_risk_transfer::{
    BookRef, LegRole, MoveKind, PositionRef, PositionSlice, RiskTransfer, RiskVector,
    TransferContext, TransferError, TransferKind, TransferLeg, TransferPrice, TransferQuantity,
    TransferState, check_transfer, compute_legs, plan_position_moves,
};
use std::collections::HashMap;

/// Build a slice with a DV01-only risk vector (the other greeks zero) — enough
/// to exercise the scaling, and integer-exact.
fn slice(position_id: u64, signed_notional: f64, mark: f64, dv01: f64) -> PositionSlice {
    PositionSlice {
        position_id,
        signed_notional,
        mark,
        risk: RiskVector {
            dv01,
            ..RiskVector::ZERO
        },
    }
}

// ───────────────────────── numeric leg oracle (8 cases) ─────────────────────

#[test]
fn case1_full_long_price_above_mark() {
    // long +1_000_000 @ mark 1.5, transfer @ 1.75, Full.
    // f=1 ⇒ moved=+1_000_000; realized = 1_000_000·(1.75−1.5)=1_000_000·0.25=250_000.
    let p = PositionSlice {
        position_id: 1,
        signed_notional: 1_000_000.0,
        mark: 1.5,
        risk: RiskVector {
            dv01: 100.0,
            delta: 600_000.0,
            gamma: 8.0,
            vega: 2_000.0,
            theta: -40.0,
        },
    };
    let legs = compute_legs(&[p], 1.75, TransferQuantity::Full);
    assert_eq!(legs.source_offset.role, LegRole::SourceOffset);
    assert_eq!(legs.source_offset.signed_notional, -1_000_000.0);
    assert_eq!(legs.source_offset.price, 1.75);
    assert_eq!(legs.target_open.role, LegRole::TargetOpen);
    assert_eq!(legs.target_open.signed_notional, 1_000_000.0);
    assert_eq!(legs.target_open.price, 1.75);
    assert_eq!(legs.realized_pnl_source, 250_000.0);
    assert_eq!(legs.moved_risk.notional_base, 1_000_000.0);
    assert_eq!(legs.moved_risk.risk.dv01, 100.0);
    assert_eq!(legs.moved_risk.risk.delta, 600_000.0);
    assert_eq!(legs.moved_risk.risk.gamma, 8.0);
    assert_eq!(legs.moved_risk.risk.vega, 2_000.0);
    assert_eq!(legs.moved_risk.risk.theta, -40.0);
}

#[test]
fn case2_full_short_price_above_mark_is_loss() {
    // short −2_000_000 @ mark 1.25, transfer @ 1.5, Full.
    // moved=−2_000_000; source leg=+2_000_000 (buy to cover); target=−2_000_000.
    // realized = −2_000_000·(1.5−1.25)=−2_000_000·0.25=−500_000  (short, price up ⇒ loss).
    let legs = compute_legs(
        &[slice(2, -2_000_000.0, 1.25, -150.0)],
        1.5,
        TransferQuantity::Full,
    );
    assert_eq!(legs.source_offset.signed_notional, 2_000_000.0);
    assert_eq!(legs.target_open.signed_notional, -2_000_000.0);
    assert_eq!(legs.realized_pnl_source, -500_000.0);
    assert_eq!(legs.moved_risk.notional_base, -2_000_000.0);
    assert_eq!(legs.moved_risk.risk.dv01, -150.0);
}

#[test]
fn case3_full_short_price_below_mark_is_profit() {
    // short −2_000_000 @ mark 1.5, transfer @ 1.25, Full.
    // realized = −2_000_000·(1.25−1.5)=−2_000_000·(−0.25)=+500_000 (short, price down ⇒ profit).
    let legs = compute_legs(
        &[slice(3, -2_000_000.0, 1.5, -150.0)],
        1.25,
        TransferQuantity::Full,
    );
    assert_eq!(legs.source_offset.signed_notional, 2_000_000.0);
    assert_eq!(legs.target_open.signed_notional, -2_000_000.0);
    assert_eq!(legs.realized_pnl_source, 500_000.0);
}

#[test]
fn case4_partial_long_half() {
    // long +1_000_000 @ mark 1.5, transfer @ 1.75, Partial(500_000) ⇒ f=0.5.
    // moved=+500_000; realized=500_000·0.25=125_000; moved dv01 = 0.5·100 = 50.
    let legs = compute_legs(
        &[slice(4, 1_000_000.0, 1.5, 100.0)],
        1.75,
        TransferQuantity::Partial(500_000.0),
    );
    assert_eq!(legs.source_offset.signed_notional, -500_000.0);
    assert_eq!(legs.target_open.signed_notional, 500_000.0);
    assert_eq!(legs.realized_pnl_source, 125_000.0);
    assert_eq!(legs.moved_risk.notional_base, 500_000.0);
    assert_eq!(legs.moved_risk.risk.dv01, 50.0);
}

#[test]
fn case5_partial_identity_split_preserves_and_mints() {
    // Partial(250_000) of +1_000_000 ⇒ f=0.25.
    // remainder keeps id 42 with 750_000; moved slice mints new id 900 with 250_000.
    let mut next = 900u64;
    let moves = plan_position_moves(
        &[slice(42, 1_000_000.0, 1.5, 100.0)],
        TransferQuantity::Partial(250_000.0),
        || {
            let id = next;
            next += 1;
            id
        },
    );
    assert_eq!(moves.len(), 1);
    assert_eq!(moves[0].source_position_id, 42); // original preserved on remainder
    assert_eq!(
        moves[0].kind,
        MoveKind::Split {
            remainder_notional: 750_000.0,
            moved_notional: 250_000.0,
            moved_position_id: 900, // minted
        }
    );
}

#[test]
fn case6_multi_position_full_aggregate() {
    // id5 +1_000_000 @1.5 dv01 100 ; id6 +2_000_000 @1.25 dv01 200 ; transfer @2.0 Full.
    // moved = +3_000_000 ; realized = 1e6·0.5 + 2e6·0.75 = 500_000 + 1_500_000 = 2_000_000.
    // moved dv01 = 100 + 200 = 300.
    let ps = [
        slice(5, 1_000_000.0, 1.5, 100.0),
        slice(6, 2_000_000.0, 1.25, 200.0),
    ];
    let legs = compute_legs(&ps, 2.0, TransferQuantity::Full);
    assert_eq!(legs.source_offset.signed_notional, -3_000_000.0);
    assert_eq!(legs.target_open.signed_notional, 3_000_000.0);
    assert_eq!(legs.realized_pnl_source, 2_000_000.0);
    assert_eq!(legs.moved_risk.notional_base, 3_000_000.0);
    assert_eq!(legs.moved_risk.risk.dv01, 300.0);
}

#[test]
fn case7_multi_position_partial_half() {
    // same two positions, Partial(1_500_000) of |Σ|=3_000_000 ⇒ f=0.5.
    // moved=+1_500_000; realized = 0.5·2_000_000 = 1_000_000; moved dv01 = 0.5·300 = 150.
    let ps = [
        slice(5, 1_000_000.0, 1.5, 100.0),
        slice(6, 2_000_000.0, 1.25, 200.0),
    ];
    let legs = compute_legs(&ps, 2.0, TransferQuantity::Partial(1_500_000.0));
    assert_eq!(legs.source_offset.signed_notional, -1_500_000.0);
    assert_eq!(legs.target_open.signed_notional, 1_500_000.0);
    assert_eq!(legs.realized_pnl_source, 1_000_000.0);
    assert_eq!(legs.moved_risk.notional_base, 1_500_000.0);
    assert_eq!(legs.moved_risk.risk.dv01, 150.0);

    // and the per-position identity split: each keeps its id on the remainder,
    // each mints a new id for its moved half. f=0.5 ⇒ halves.
    let mut next = 900u64;
    let moves = plan_position_moves(&ps, TransferQuantity::Partial(1_500_000.0), || {
        let id = next;
        next += 1;
        id
    });
    assert_eq!(moves[0].source_position_id, 5);
    assert_eq!(
        moves[0].kind,
        MoveKind::Split {
            remainder_notional: 500_000.0,
            moved_notional: 500_000.0,
            moved_position_id: 900,
        }
    );
    assert_eq!(moves[1].source_position_id, 6);
    assert_eq!(
        moves[1].kind,
        MoveKind::Split {
            remainder_notional: 1_000_000.0,
            moved_notional: 1_000_000.0,
            moved_position_id: 901,
        }
    );
}

#[test]
fn case8_partial_short_quarter_price_below_mark_is_profit() {
    // short −4_000_000 @ mark 1.5, transfer @ 1.25, Partial(1_000_000) of |Σ|=4e6 ⇒ f=0.25.
    // moved=−1_000_000; source leg=+1_000_000; target=−1_000_000.
    // realized = −1_000_000·(1.25−1.5) = −1_000_000·(−0.25) = +250_000.
    // moved dv01 = 0.25·(−400) = −100.
    let legs = compute_legs(
        &[slice(7, -4_000_000.0, 1.5, -400.0)],
        1.25,
        TransferQuantity::Partial(1_000_000.0),
    );
    assert_eq!(legs.source_offset.signed_notional, 1_000_000.0);
    assert_eq!(legs.target_open.signed_notional, -1_000_000.0);
    assert_eq!(legs.realized_pnl_source, 250_000.0);
    assert_eq!(legs.moved_risk.notional_base, -1_000_000.0);
    assert_eq!(legs.moved_risk.risk.dv01, -100.0);
}

// ───────────────────────── validation rejection oracle ──────────────────────

/// Two enabled books "SRC" (desk D1) and "TGT" (desk D2), and two positions
/// (10, 11) both in "SRC". A knob lets a test disable the target.
fn ctx(target_enabled: bool) -> TransferContext {
    let mut positions = HashMap::new();
    positions.insert(
        10u64,
        PositionRef {
            risk_book_id: "SRC".into(),
            signed_notional: 1_000_000.0,
        },
    );
    positions.insert(
        11u64,
        PositionRef {
            risk_book_id: "SRC".into(),
            signed_notional: 500_000.0,
        },
    );
    let mut books = HashMap::new();
    books.insert(
        "SRC".into(),
        BookRef {
            desk_id: "D1".into(),
            enabled: true,
        },
    );
    books.insert(
        "TGT".into(),
        BookRef {
            desk_id: "D2".into(),
            enabled: target_enabled,
        },
    );
    TransferContext::new(positions, books)
}

/// A well-formed DeskToDesk transfer of position 10 (SRC/D1 → TGT/D2) that
/// passes validation — the baseline the rejection tests each perturb.
fn base_transfer() -> RiskTransfer {
    RiskTransfer {
        id: "t-1".into(),
        kind: TransferKind::DeskToDesk,
        source: TransferLeg {
            risk_book_id: "SRC".into(),
            desk_id: "D1".into(),
            trader: "alice".into(),
            position_ids: vec![10],
        },
        target: TransferLeg {
            risk_book_id: "TGT".into(),
            desk_id: "D2".into(),
            trader: "bob".into(),
            position_ids: vec![],
        },
        quantity: TransferQuantity::Full,
        price: TransferPrice::Mid,
        reason: String::new(),
        initiated_by: "alice".into(),
        initiated_at: 0,
        state: TransferState::Pending,
        approver: None,
        decided_at: None,
        provenance: None,
    }
}

#[test]
fn valid_baseline_passes() {
    assert_eq!(check_transfer(&base_transfer(), &ctx(true)), Ok(()));
}

#[test]
fn reject_agreed_price_without_reason() {
    let mut t = base_transfer();
    t.price = TransferPrice::Agreed(1.5);
    // reason left empty ⇒ rejected.
    assert_eq!(
        check_transfer(&t, &ctx(true)),
        Err(TransferError::AgreedPriceRequiresReason)
    );
    // with a reason it passes.
    t.reason = "manual cross at yesterday's close".into();
    assert_eq!(check_transfer(&t, &ctx(true)), Ok(()));
}

#[test]
fn reject_partial_over_notional() {
    let mut t = base_transfer();
    // position 10 carries 1_000_000; ask to move 1_500_000 ⇒ exceeds.
    t.quantity = TransferQuantity::Partial(1_500_000.0);
    assert_eq!(
        check_transfer(&t, &ctx(true)),
        Err(TransferError::PartialExceedsNotional {
            requested: 1_500_000.0,
            available: 1_000_000.0,
        })
    );
}

#[test]
fn reject_nonpositive_partial() {
    let mut t = base_transfer();
    t.quantity = TransferQuantity::Partial(0.0);
    assert_eq!(
        check_transfer(&t, &ctx(true)),
        Err(TransferError::NonPositivePartial(0.0))
    );
}

#[test]
fn reject_position_not_in_source_book() {
    let mut t = base_transfer();
    // claim position 10 is in a book it is not in.
    t.source.risk_book_id = "TGT".into();
    t.source.desk_id = "D2".into(); // keep source desk consistent w/ TGT's owner
    t.target.risk_book_id = "SRC".into();
    t.target.desk_id = "D1".into();
    // position 10 is actually in SRC, but source now claims TGT.
    assert_eq!(
        check_transfer(&t, &ctx(true)),
        Err(TransferError::PositionNotInSourceBook {
            position_id: 10,
            actual_book: "SRC".into(),
            source_book: "TGT".into(),
        })
    );
}

#[test]
fn reject_position_not_found() {
    let mut t = base_transfer();
    t.source.position_ids = vec![999];
    assert_eq!(
        check_transfer(&t, &ctx(true)),
        Err(TransferError::PositionNotFound(999))
    );
}

#[test]
fn reject_disabled_target_book() {
    let t = base_transfer();
    assert_eq!(
        check_transfer(&t, &ctx(false)),
        Err(TransferError::TargetBookDisabled("TGT".into()))
    );
}

#[test]
fn reject_reattribute_across_desks() {
    let mut t = base_transfer();
    t.kind = TransferKind::ReAttribute; // but source D1 ≠ target D2.
    assert_eq!(
        check_transfer(&t, &ctx(true)),
        Err(TransferError::ReAttributeCrossDesk {
            source_desk: "D1".into(),
            target_desk: "D2".into(),
        })
    );
}

#[test]
fn reject_empty_selection() {
    let mut t = base_transfer();
    t.source.position_ids = vec![];
    assert_eq!(
        check_transfer(&t, &ctx(true)),
        Err(TransferError::EmptyPositionSet)
    );
}

// ───────────────────────── property-based invariants ────────────────────────

use proptest::prelude::*;

proptest! {
    /// §3.2 firm-level net-flat: the source offsetting leg and the target
    /// opening leg net to exactly zero notional, for any slices/price/quantity.
    #[test]
    fn prop_legs_net_to_zero(
        notionals in proptest::collection::vec(-5_000_000.0f64..5_000_000.0, 1..6),
        marks in proptest::collection::vec(0.1f64..3.0, 1..6),
        price in 0.1f64..3.0,
        full in any::<bool>(),
        frac in 0.0f64..1.0,
    ) {
        let n = notionals.len().min(marks.len());
        let ps: Vec<PositionSlice> = (0..n)
            .map(|i| slice(i as u64, notionals[i], marks[i], 0.0))
            .collect();
        let agg_abs: f64 = ps.iter().map(|p| p.signed_notional).sum::<f64>().abs();
        let quantity = if full {
            TransferQuantity::Full
        } else {
            // a valid partial: strictly inside (0, |Σ|]; skip the degenerate |Σ|≈0.
            prop_assume!(agg_abs > 1.0);
            TransferQuantity::Partial((frac * agg_abs).max(f64::MIN_POSITIVE))
        };
        let legs = compute_legs(&ps, price, quantity);
        // exact negation ⇒ sum is exactly 0.0.
        prop_assert_eq!(legs.source_offset.signed_notional + legs.target_open.signed_notional, 0.0);
        prop_assert_eq!(legs.moved_risk.notional_base, legs.target_open.signed_notional);
    }

    /// Realised P&L is antisymmetric in (price − mark): with a common mark of
    /// 0.0, transferring at +d vs −d yields exactly opposite P&L.
    #[test]
    fn prop_realized_pnl_antisymmetric(
        notionals in proptest::collection::vec(-5_000_000.0f64..5_000_000.0, 1..6),
        d in 0.0f64..2.0,
    ) {
        // mark = 0 makes (price − mark) exactly ±d (no cancellation error).
        let ps: Vec<PositionSlice> = notionals
            .iter()
            .enumerate()
            .map(|(i, &nn)| slice(i as u64, nn, 0.0, 0.0))
            .collect();
        let plus = compute_legs(&ps, d, TransferQuantity::Full).realized_pnl_source;
        let minus = compute_legs(&ps, -d, TransferQuantity::Full).realized_pnl_source;
        prop_assert_eq!(plus, -minus);
    }
}
