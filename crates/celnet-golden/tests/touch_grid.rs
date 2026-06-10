//! Golden gate: Celnet's touch family (one-touch / no-touch / double-no-touch /
//! double-touch) reproduces a frozen, *independent* QuantLib oracle.
//!
//! Loads `data/touch_gk.csv` and asserts that
//! [`celnet_exotics::one_touch_price`], [`celnet_exotics::no_touch_price`],
//! [`celnet_exotics::double_no_touch_price`] and
//! [`celnet_exotics::double_touch_price`] match it across the whole grid.
//!
//! Why this matters (audit finding, `celnet-exotics` major): the in-crate touch
//! tests only assert *self-consistency* — `no_touch = df − one_touch` and
//! `double_touch = df − dnt` hold **by construction** in the Celnet code, so a
//! common-mode sign/exponent error shared by both legs of a complementary pair is
//! invisible to them. QuantLib's `AnalyticDoubleBarrierBinaryEngine` is a *fully
//! independent* implementation (its own reflection series for the corridor
//! survival probability). The single-barrier touches are recovered as the
//! wide-corridor limit of that same independent engine (one wall pushed ≳ 12σ√T
//! out), so every row here is an external cross-check, not a tautology.
//!
//! The frozen grid validates the *at-expiry* (deferred) rebate timing only. It
//! does **not** transitively validate the at-hit form: the at-hit λ-power /
//! CDF-argument pairing is machinery the at-expiry form never touches (the P0
//! at-hit pairing defect passed this entire grid). The at-hit form is gated
//! separately — by the discounted-first-passage-density quadrature oracle behind
//! the `touch.json` ONE_TOUCH vectors plus the in-crate law tests (T→∞ Laplace
//! limit, the discounting sandwich, the r_d = 0 collapse).
//!
//! ## Tolerance
//!
//! Both sides evaluate the continuous-monitoring reflection series through
//! different standard-normal-CDF and series implementations. The double-corridor
//! survival series differs in truncation length and summation order between
//! QuantLib and Celnet, so agreement is *near* but not last-bit: a `1e-6` relative
//! leg with a `1e-8` absolute floor catches any real formula error (a sign,
//! exponent or reflection-orientation bug shifts the price by percent, not ppm)
//! while admitting the genuine series-truncation / summation-order difference.

use celnet_core::is_close;
use celnet_exotics::{
    DoubleNoTouch, RebateTiming, double_no_touch_price, double_touch_price, no_touch_price,
    one_touch_price,
};
use celnet_golden::{TouchKind, TouchRecord, load_touch};
use celnet_types::VanillaInputs;

/// Price one touch row with the matching Celnet entry point.
fn celnet_touch(rec: &TouchRecord) -> f64 {
    let v = VanillaInputs::new(rec.spot, rec.spot, rec.vol, rec.t, rec.r_dom, rec.r_for);
    let i: celnet_exotics::ExoticInputs = (&v).into();
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
                rec.lower.expect("dt lower"),
                rec.upper.expect("dt upper"),
                rec.rebate,
            );
            double_touch_price(&i, dnt)
        }
    }
}

#[test]
fn touch_prices_match_quantlib_all_kinds() {
    let records = load_touch().expect("frozen touch table loads");
    assert_eq!(
        records.len(),
        1224,
        "frozen touch grid size changed unexpectedly"
    );

    const REL: f64 = 1e-6;
    const ABS: f64 = 1e-8;

    let mut worst_rel = 0.0f64;
    let mut worst_abs = 0.0f64;

    for rec in &records {
        let celnet = celnet_touch(rec);
        let abs_dev = (celnet - rec.price).abs();
        let scale = celnet.abs().max(rec.price.abs());
        if scale > 1e-4 {
            worst_rel = worst_rel.max(abs_dev / scale);
        }
        worst_abs = worst_abs.max(abs_dev);

        assert!(
            is_close(celnet, rec.price, REL, ABS),
            "touch mismatch vs QuantLib: kind={:?} celnet={celnet} oracle={} \
             |diff|={abs_dev} at {rec:?}",
            rec.kind,
            rec.price,
        );
    }

    // Every kind must be exercised (defence against a silently-truncated oracle).
    for k in [
        TouchKind::OneTouch,
        TouchKind::NoTouch,
        TouchKind::Dnt,
        TouchKind::DoubleTouch,
    ] {
        assert!(
            records.iter().any(|r| r.kind == k),
            "touch kind {k:?} missing from grid"
        );
    }

    println!(
        "golden touch grid: {} rows; worst relative deviation {worst_rel:.3e}, \
         worst absolute {worst_abs:.3e}",
        records.len()
    );
}
