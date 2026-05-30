//! Golden gate: Celnet's double-barrier (corridor) knock-out and knock-in
//! reproduce a frozen, *independent* QuantLib oracle.
//!
//! Loads `data/double_barrier_gk.csv` (QuantLib `AnalyticDoubleBarrierEngine`
//! knock-out prices over the Garman-Kohlhagen process, plus the knock-in as
//! `vanilla_quantlib − ko_quantlib`) and asserts that
//! [`celnet_exotics::double_knock_out_price`] matches the KO rows directly, and
//! that the in/out parity `KI = vanilla − KO` reproduces the KI rows.
//!
//! Why this matters (audit finding, `celnet-exotics` minor): the double-KO had no
//! external reference at all — only a self-written, noisy Monte-Carlo oracle and
//! complementarity checks. QuantLib's analytic double-barrier engine is a fully
//! independent implementation, so an error in Celnet's Ikeda-Kunitomo image
//! series (a reflection-orientation or exponent bug) surfaces here as a failing
//! row.
//!
//! ## Tolerance
//!
//! Both sides evaluate the continuous-monitoring Ikeda-Kunitomo reflection series
//! through different CDF implementations and truncation lengths, so agreement is
//! near (not last-bit): a `1e-6` relative leg with a `1e-8` absolute floor (for
//! the rows where the KO collapses toward zero in a tight corridor) catches a real
//! formula error while admitting series-truncation differences.

use celnet_core::is_close;
use celnet_exotics::{DoubleBarrierKnockOut, double_knock_out_price};
use celnet_golden::{DoubleBarrierKind, DoubleBarrierRecord, load_double_barrier};
use celnet_types::VanillaInputs;
use celnet_vanilla::price as vanilla_price;

/// Price one double-barrier row with the matching Celnet entry point. Knock-out
/// is priced directly by the image series; knock-in is the in/out-parity
/// complement `vanilla − KO`.
fn celnet_double_barrier(rec: &DoubleBarrierRecord) -> f64 {
    let i = VanillaInputs::new(rec.spot, rec.strike, rec.vol, rec.t, rec.r_dom, rec.r_for);
    let ko = double_knock_out_price(
        &i,
        DoubleBarrierKnockOut::new(rec.option_type, rec.strike, rec.lower, rec.upper),
    );
    match rec.kind {
        DoubleBarrierKind::KnockOut => ko,
        DoubleBarrierKind::KnockIn => vanilla_price(rec.option_type, &i) - ko,
    }
}

#[test]
fn double_barrier_prices_match_quantlib() {
    let records = load_double_barrier().expect("frozen double-barrier table loads");
    assert_eq!(
        records.len(),
        432,
        "frozen double-barrier grid size changed unexpectedly"
    );

    const REL: f64 = 1e-6;
    const ABS: f64 = 1e-8;

    let mut worst_rel = 0.0f64;
    let mut worst_abs = 0.0f64;

    for rec in &records {
        let celnet = celnet_double_barrier(rec);
        let abs_dev = (celnet - rec.price).abs();
        let scale = celnet.abs().max(rec.price.abs());
        if scale > 1e-4 {
            worst_rel = worst_rel.max(abs_dev / scale);
        }
        worst_abs = worst_abs.max(abs_dev);

        assert!(
            is_close(celnet, rec.price, REL, ABS),
            "double-barrier mismatch vs QuantLib: kind={:?} {:?} celnet={celnet} \
             oracle={} |diff|={abs_dev} at {rec:?}",
            rec.kind,
            rec.option_type,
            rec.price,
        );
    }

    for k in [DoubleBarrierKind::KnockOut, DoubleBarrierKind::KnockIn] {
        assert!(
            records.iter().any(|r| r.kind == k),
            "double-barrier kind {k:?} missing from grid"
        );
    }

    println!(
        "golden double-barrier grid: {} rows; worst relative deviation \
         {worst_rel:.3e}, worst absolute {worst_abs:.3e}",
        records.len()
    );
}
