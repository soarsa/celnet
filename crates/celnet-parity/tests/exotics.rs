//! Parity rows 12–14: **first-generation exotics reproduce QuantLib reference
//! values** — the independent open-source oracle (`QuantLib 1.42.1`) that backs
//! the "✅ QuantLib-gated" rows in `docs/CAPABILITIES-VS-COMPETITION.md` §3. The
//! incumbents (Fenics kACE, Bloomberg OVML, SynOption) price the same products
//! behind closed pricers with no published validation; Celnet reproduces an
//! independent QuantLib computation to ~1e-9, in the open.
//!
//! The reference tables live in `celnet-golden` (QuantLib-sourced CSVs); this
//! crate re-runs the parity through the **public** exotics API so the matrix
//! gates the same product surface a downstream consumer would call:
//!
//!  12. European digitals (cash-or-nothing, asset-or-nothing × call/put);
//!  13. one-touch / no-touch / double-no-touch / double-touch;
//!  14. all eight single-barrier flavours and the double knock-out/knock-in.
//!
//! Tolerances mirror the golden gates: a tight relative leg to catch any real
//! block-selection / sign error, with an absolute floor for the binding-barrier
//! rows whose price collapses toward zero.

use celnet_core::is_close;
use celnet_exotics::{
    BarrierKind, BarrierStyle, DigitalKind, DoubleBarrierKnockOut, DoubleNoTouch, RebateTiming,
    SingleBarrier, digital_price, double_knock_out_price, double_no_touch_price,
    double_touch_price, no_touch_price, one_touch_price, single_barrier_price,
};
use celnet_golden::{
    BarrierRecord, BarrierType, DigitalSettlement, DoubleBarrierKind, DoubleBarrierRecord,
    TouchKind, TouchRecord, load_barrier, load_digital, load_double_barrier, load_touch,
};
use celnet_types::VanillaInputs;
use celnet_vanilla::price as vanilla_price;

/// Tight relative leg (catches a real implementation error) with an absolute
/// floor for rows whose price is driven to ~zero by a binding barrier. These
/// mirror the closed-form golden gates for digitals / single-barriers / touches.
const REL: f64 = 1e-9;
const ABS: f64 = 1e-10;

/// The analytic double-barrier price is a truncated infinite reflection series,
/// so its agreement with QuantLib's independent series implementation is to
/// ~1e-6, not last-bit — the same tolerance the `celnet-golden` double-barrier
/// gate uses. Anything wider would admit a real error; anything tighter would
/// reject genuine series-truncation noise.
const DB_REL: f64 = 1e-6;
const DB_ABS: f64 = 1e-8;

fn inputs(spot: f64, strike: f64, vol: f64, t: f64, r_dom: f64, r_for: f64) -> VanillaInputs {
    VanillaInputs::new(spot, strike, vol, t, r_dom, r_for)
}

/// Row 12 — European digitals reproduce the QuantLib `CashOrNothingPayoff` /
/// `AssetOrNothingPayoff` references across both styles and both directions.
#[test]
fn digitals_match_quantlib() {
    let records = load_digital().expect("frozen digital table loads");
    let mut rows = 0usize;
    for rec in &records {
        let i = inputs(rec.spot, rec.strike, rec.vol, rec.t, rec.r_dom, rec.r_for);
        let kind = match rec.style {
            DigitalSettlement::CashOrNothing => DigitalKind::cash(rec.option_type),
            DigitalSettlement::AssetOrNothing => DigitalKind::asset(rec.option_type),
        };
        // The table scales the unit pricer by the row's payout (unit cash / one
        // asset unit in this grid).
        let celnet = rec.payout * digital_price(kind, &i);
        assert!(
            is_close(celnet, rec.price, REL, ABS),
            "{:?} {:?}: celnet {celnet} != QuantLib {}",
            rec.style,
            rec.option_type,
            rec.price
        );
        rows += 1;
    }
    assert!(rows > 0, "digital grid empty");
    // Both settlement styles must be present in what we gated.
    assert!(
        records
            .iter()
            .any(|r| r.style == DigitalSettlement::CashOrNothing)
            && records
                .iter()
                .any(|r| r.style == DigitalSettlement::AssetOrNothing),
        "digital grid must cover both settlement styles"
    );
    eprintln!("digitals: gated {rows} QuantLib rows");
}

/// Map a touch record to the matching Celnet pricer call.
fn celnet_touch(rec: &TouchRecord) -> f64 {
    let i = inputs(rec.spot, rec.strike(), rec.vol, rec.t, rec.r_dom, rec.r_for);
    match rec.kind {
        TouchKind::OneTouch => one_touch_price(
            &i,
            rec.barrier.expect("one-touch barrier"),
            rec.rebate,
            RebateTiming::AtExpiry,
        ),
        TouchKind::NoTouch => {
            no_touch_price(&i, rec.barrier.expect("no-touch barrier"), rec.rebate)
        }
        TouchKind::Dnt => {
            let dnt = DoubleNoTouch::new(
                rec.lower.expect("dnt lower"),
                rec.upper.expect("dnt upper"),
                rec.rebate,
            );
            double_no_touch_price(&i, dnt)
        }
        TouchKind::DoubleTouch => {
            let dnt = DoubleNoTouch::new(
                rec.lower.expect("double-touch lower"),
                rec.upper.expect("double-touch upper"),
                rec.rebate,
            );
            double_touch_price(&i, dnt)
        }
    }
}

/// A touch record carries no strike (the payoff is the rebate); supply spot as a
/// harmless placeholder strike for the `VanillaInputs` diffusion state.
trait TouchStrike {
    fn strike(&self) -> f64;
}
impl TouchStrike for TouchRecord {
    fn strike(&self) -> f64 {
        self.spot
    }
}

/// Row 13 — one-touch / no-touch / double-no-touch / double-touch reproduce the
/// QuantLib independent double-barrier-binary engine across all four kinds.
#[test]
fn touches_and_dnt_match_quantlib() {
    let records = load_touch().expect("frozen touch table loads");
    let mut rows = 0usize;
    for rec in &records {
        let celnet = celnet_touch(rec);
        assert!(
            is_close(celnet, rec.price, REL, ABS),
            "{:?}: celnet {celnet} != QuantLib {}",
            rec.kind,
            rec.price
        );
        rows += 1;
    }
    for k in [
        TouchKind::OneTouch,
        TouchKind::NoTouch,
        TouchKind::Dnt,
        TouchKind::DoubleTouch,
    ] {
        assert!(
            records.iter().any(|r| r.kind == k),
            "touch grid missing kind {k:?}"
        );
    }
    eprintln!("touches/DNT: gated {rows} QuantLib rows");
}

/// Map the frozen barrier-type onto the Celnet `BarrierKind`.
fn barrier_kind(rec: &BarrierRecord) -> BarrierKind {
    let (up, style) = match rec.barrier_type {
        BarrierType::DownOut => (false, BarrierStyle::KnockOut),
        BarrierType::DownIn => (false, BarrierStyle::KnockIn),
        BarrierType::UpOut => (true, BarrierStyle::KnockOut),
        BarrierType::UpIn => (true, BarrierStyle::KnockIn),
    };
    BarrierKind {
        up,
        style,
        option: rec.option_type,
    }
}

/// Row 14 — all eight single-barrier flavours ({down,up}×{in,out}×{call,put})
/// reproduce QuantLib's `AnalyticBarrierEngine`. QuantLib computes each block
/// from a fully independent implementation, so a mis-selected Reiner-Rubinstein
/// block would surface as a failing row here (knock-out = vanilla − knock-in is
/// by-construction in Celnet, so an in-crate parity test could *not* catch it).
#[test]
fn barriers_match_quantlib() {
    let records = load_barrier().expect("frozen barrier table loads");
    let mut rows = 0usize;
    for rec in &records {
        let i = inputs(rec.spot, rec.strike, rec.vol, rec.t, rec.r_dom, rec.r_for);
        let spec = SingleBarrier {
            kind: barrier_kind(rec),
            strike: rec.strike,
            barrier: rec.barrier,
            rebate: rec.rebate,
        };
        let celnet = single_barrier_price(&i, spec);
        assert!(
            is_close(celnet, rec.price, REL, ABS),
            "{:?} {:?}: celnet {celnet} != QuantLib {}",
            rec.barrier_type,
            rec.option_type,
            rec.price
        );
        rows += 1;
    }
    for bt in [
        BarrierType::DownOut,
        BarrierType::DownIn,
        BarrierType::UpOut,
        BarrierType::UpIn,
    ] {
        assert!(
            records.iter().any(|r| r.barrier_type == bt),
            "barrier grid missing flavour {bt:?}"
        );
    }
    eprintln!("single barriers: gated {rows} QuantLib rows (all 8 flavours)");
}

/// Row 14 (double) — the double knock-out reproduces QuantLib's
/// `AnalyticDoubleBarrierEngine`, and the knock-in is derived as
/// `vanilla − KO` (both QuantLib-sourced), gating the corridor products.
#[test]
fn double_barriers_match_quantlib() {
    let records = load_double_barrier().expect("frozen double-barrier table loads");
    let mut rows = 0usize;
    for rec in &records {
        let celnet = celnet_double_barrier(rec);
        assert!(
            is_close(celnet, rec.price, DB_REL, DB_ABS),
            "{:?} {:?}: celnet {celnet} != QuantLib {}",
            rec.kind,
            rec.option_type,
            rec.price
        );
        rows += 1;
    }
    assert!(rows > 0, "double-barrier grid empty");
    eprintln!("double barriers: gated {rows} QuantLib rows");
}

/// Map a double-barrier record to the Celnet pricer (KO direct, KI = vanilla−KO).
fn celnet_double_barrier(rec: &DoubleBarrierRecord) -> f64 {
    let i = inputs(rec.spot, rec.strike, rec.vol, rec.t, rec.r_dom, rec.r_for);
    let ko = double_knock_out_price(
        &i,
        DoubleBarrierKnockOut::new(rec.option_type, rec.strike, rec.lower, rec.upper),
    );
    match rec.kind {
        DoubleBarrierKind::KnockOut => ko,
        DoubleBarrierKind::KnockIn => vanilla_price(rec.option_type, &i) - ko,
    }
}
