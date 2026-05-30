//! Golden gate: Celnet single-barrier pricing reproduces the frozen QuantLib
//! oracle, independently for all eight flavours.
//!
//! Loads `data/barrier_gk.csv` (analytic single-barrier prices from QuantLib
//! 1.42.1's `AnalyticBarrierEngine` over the Garman-Kohlhagen process) and asserts
//! that [`celnet_exotics::single_barrier_price`] matches it across the whole grid.
//!
//! This is the *independent* oracle that the in-crate in/out parity test cannot
//! be: parity (`KI + KO = vanilla`) holds **by construction** in the Celnet code
//! (knock-out is defined as `vanilla − knock_in`), so it cannot detect a wrong
//! knock-in block selection. QuantLib computes every one of the eight flavours
//! {down,up} × {in,out} × {call,put} from a fully independent implementation, so a
//! mis-selected Reiner-Rubinstein block would surface here as a failing row.
//!
//! The grid uses zero rebate (so the rebate leg is exercised separately by the
//! exotics crate's own tests and is out of scope for this oracle); the barrier is
//! placed on the relevant side of spot (85 for down, 115 for up, spot = 100).
//!
//! ## Tolerance
//!
//! Both sides evaluate the same continuous-monitoring closed form (the
//! reflection-principle / Reiner-Rubinstein construction) through different
//! standard-normal-CDF implementations, so agreement is last-bit: a `1e-9`
//! relative leg with a `1e-11` absolute floor (for the rows where a knock-out is
//! driven to ~zero by a binding barrier) catches any real block-selection or
//! sign error while admitting only genuine rounding noise. Closeness is
//! [`celnet_core::is_close`] (agreement within *either* leg).

use celnet_core::is_close;
use celnet_exotics::{BarrierKind, BarrierStyle, SingleBarrier, single_barrier_price};
use celnet_golden::{BarrierRecord, BarrierType, load_barrier};
use celnet_types::VanillaInputs;

/// Map the frozen oracle barrier-kind string onto the exotics `BarrierKind`.
fn kind_of(rec: &BarrierRecord) -> BarrierKind {
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

#[test]
fn single_barrier_prices_match_quantlib_all_flavours() {
    let records = load_barrier().expect("frozen barrier table loads");
    assert_eq!(
        records.len(),
        288,
        "frozen barrier grid size changed unexpectedly"
    );

    // Relative leg tight enough to catch a real implementation error; absolute
    // floor for the binding-barrier rows where the price collapses toward zero and
    // a relative comparison on sub-machine-epsilon noise is meaningless.
    const REL: f64 = 1e-9;
    const ABS: f64 = 1e-11;

    // Per-flavour worst-case relative deviation, surfaced with `--nocapture`.
    // Index order: [DownOut, DownIn, UpOut, UpIn] × {Call, Put} flattened.
    let mut worst_rel = 0.0f64;
    let mut worst_abs = 0.0f64;
    let mut asserts = 0usize;

    for rec in &records {
        let inputs = VanillaInputs::new(rec.spot, rec.strike, rec.vol, rec.t, rec.r_dom, rec.r_for);
        let spec = SingleBarrier {
            kind: kind_of(rec),
            strike: rec.strike,
            barrier: rec.barrier,
            rebate: rec.rebate,
        };
        let celnet = single_barrier_price(&inputs, spec);

        let abs_dev = (celnet - rec.price).abs();
        let scale = celnet.abs().max(rec.price.abs());
        if scale > 1e-6 {
            worst_rel = worst_rel.max(abs_dev / scale);
        }
        worst_abs = worst_abs.max(abs_dev);
        asserts += 1;

        assert!(
            is_close(celnet, rec.price, REL, ABS),
            "single-barrier mismatch vs QuantLib: celnet={celnet} oracle={} \
             |diff|={abs_dev} (rel={REL}, abs={ABS}) at {rec:?}",
            rec.price,
        );
    }

    // Every flavour must actually be exercised by the grid (defence against a
    // silently-truncated oracle dropping a knock direction).
    for bt in [
        BarrierType::DownOut,
        BarrierType::DownIn,
        BarrierType::UpOut,
        BarrierType::UpIn,
    ] {
        for opt in [
            celnet_types::OptionType::Call,
            celnet_types::OptionType::Put,
        ] {
            let present = records
                .iter()
                .any(|r| r.barrier_type == bt && r.option_type == opt);
            assert!(present, "flavour {bt:?}/{opt:?} missing from barrier grid");
        }
    }

    println!(
        "golden barrier grid: {asserts} flavour×row assertions; \
         worst relative deviation {worst_rel:.3e}, worst absolute {worst_abs:.3e}"
    );
}
