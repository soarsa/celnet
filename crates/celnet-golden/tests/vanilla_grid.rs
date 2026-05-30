//! Golden gate: Celnet vanilla pricing reproduces the frozen QuantLib oracle.
//!
//! Loads `data/vanilla_gk.csv` (Garman-Kohlhagen analytic price + Greeks from
//! QuantLib 1.42.1) and asserts that [`celnet_vanilla`] matches it across the
//! whole grid, within documented relative/absolute tolerances. The tolerances
//! are tight where the quantity is well-conditioned (price, delta, the rhos) and
//! loosened only via the *absolute* leg for quantities that legitimately collapse
//! toward zero in the deep-out-of-the-money short-dated corner of the grid
//! (gamma, vega, theta), where both libraries return sub-`1e-12` noise and a
//! relative test is meaningless. Closeness uses [`celnet_core::is_close`]: two
//! values agree if they are within *either* the relative or the absolute leg.

use celnet_core::is_close;
use celnet_golden::{VanillaRecord, load_vanilla};
use celnet_vanilla::{greeks, price};

/// A per-quantity tolerance pair `(relative, absolute)`.
struct Tol {
    name: &'static str,
    rel: f64,
    abs: f64,
}

/// Assert one quantity against the oracle, tracking the worst observed deviation.
fn check(
    tol: &Tol,
    celnet: f64,
    oracle: f64,
    rec: &VanillaRecord,
    worst_rel: &mut f64,
    worst_abs: &mut f64,
) {
    let abs_dev = (celnet - oracle).abs();
    let scale = celnet.abs().max(oracle.abs());
    // Only track the *relative* deviation where the quantity is meaningfully
    // non-zero. Near-zero quantities (e.g. gamma in the deep-OTM short-dated
    // corner, ~1e-150) carry a huge relative figure on pure sub-machine-epsilon
    // noise that is irrelevant to accuracy; those are governed by the absolute
    // leg instead, which `worst_abs` reports faithfully.
    if scale > 1e-6 {
        *worst_rel = worst_rel.max(abs_dev / scale);
    }
    *worst_abs = worst_abs.max(abs_dev);
    assert!(
        is_close(celnet, oracle, tol.rel, tol.abs),
        "{} mismatch vs QuantLib: celnet={celnet} oracle={oracle} |diff|={abs_dev} \
         (rel={}, abs={}) at {rec:?}",
        tol.name,
        tol.rel,
        tol.abs,
    );
}

#[test]
fn vanilla_price_and_greeks_match_quantlib() {
    let records = load_vanilla().expect("frozen vanilla table loads");
    assert_eq!(records.len(), 3200, "frozen grid size changed unexpectedly");

    // Tolerances. Price and delta are reproduced to ~1e-12 relative (these are
    // the same closed form evaluated through libm vs QuantLib's stdlib, so the
    // gap is pure last-bit rounding). The rhos are equally well-conditioned.
    // Gamma/vega/theta carry an absolute floor for the deep-OTM noise corner.
    let price_tol = Tol {
        name: "price",
        rel: 1e-11,
        abs: 1e-12,
    };
    let delta_tol = Tol {
        name: "delta_spot",
        rel: 1e-11,
        abs: 1e-12,
    };
    let gamma_tol = Tol {
        name: "gamma",
        rel: 1e-9,
        abs: 1e-12,
    };
    let vega_tol = Tol {
        name: "vega",
        rel: 1e-9,
        abs: 1e-10,
    };
    let theta_tol = Tol {
        name: "theta",
        rel: 1e-9,
        abs: 1e-10,
    };
    let rho_dom_tol = Tol {
        name: "rho_dom",
        rel: 1e-10,
        abs: 1e-12,
    };
    let rho_for_tol = Tol {
        name: "rho_for",
        rel: 1e-10,
        abs: 1e-12,
    };

    let mut worst_rel = 0.0f64;
    let mut worst_abs = 0.0f64;

    for rec in &records {
        let inputs = celnet_types::VanillaInputs::new(
            rec.spot, rec.strike, rec.vol, rec.t, rec.r_dom, rec.r_for,
        );

        let p = price(rec.option_type, &inputs);
        check(
            &price_tol,
            p,
            rec.price,
            rec,
            &mut worst_rel,
            &mut worst_abs,
        );

        let g = greeks(rec.option_type, &inputs);
        // Price must also be consistent through the one-pass greeks path.
        check(
            &price_tol,
            g.price,
            rec.price,
            rec,
            &mut worst_rel,
            &mut worst_abs,
        );
        check(
            &delta_tol,
            g.delta_spot,
            rec.delta_spot,
            rec,
            &mut worst_rel,
            &mut worst_abs,
        );
        check(
            &gamma_tol,
            g.gamma,
            rec.gamma,
            rec,
            &mut worst_rel,
            &mut worst_abs,
        );
        check(
            &vega_tol,
            g.vega,
            rec.vega,
            rec,
            &mut worst_rel,
            &mut worst_abs,
        );
        check(
            &theta_tol,
            g.theta,
            rec.theta,
            rec,
            &mut worst_rel,
            &mut worst_abs,
        );
        check(
            &rho_dom_tol,
            g.rho_dom,
            rec.rho_dom,
            rec,
            &mut worst_rel,
            &mut worst_abs,
        );
        check(
            &rho_for_tol,
            g.rho_for,
            rec.rho_for,
            rec,
            &mut worst_rel,
            &mut worst_abs,
        );
    }

    // Surfaced for the record (visible with `--nocapture`): the worst deviation
    // anywhere in the 3200-row grid across price + all six Greeks.
    println!(
        "golden vanilla grid: {} rows; worst relative deviation {worst_rel:.3e}, \
         worst absolute deviation {worst_abs:.3e}",
        records.len()
    );
}
