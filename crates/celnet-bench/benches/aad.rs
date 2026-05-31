//! Risk-path acceleration bench: **reverse-mode AAD vs bump-and-revalue** for the
//! per-position first-order Greek block (`docs/RISK-HIERARCHY.md` §3.3).
//!
//! The cube's additive leaf (`celnet-risk-normalize::canonicalize`) needs the full
//! Greek set per position. The two ways to get the first-order block:
//!
//! - **AAD** — one reverse sweep of the recorded Garman-Kohlhagen graph yields the
//!   whole gradient (`delta, vega, theta, rho_dom, rho_for`) at a small constant
//!   multiple of one price, *independent of the factor count*.
//! - **Bump-and-revalue** — central finite differences: **two repricings per risk
//!   factor**, i.e. O(n) prices for n factors.
//!
//! Both closures here compute the **same five first-order sensitivities** so the
//! comparison is like-for-like; `divan` reports the median of each. The ratio of
//! the two medians is the measured AAD speed-up on the first-order block, and it
//! grows linearly with the factor count (here n = 5 GK inputs). `black_box` guards
//! inputs and outputs so nothing is folded away.

use celnet_bench::representative_inputs;
use celnet_types::{OptionType, VanillaInputs};
use celnet_vanilla::{adjoint_greeks, price};
use divan::{Bencher, black_box};

fn main() {
    divan::main();
}

/// The five first-order sensitivities, the common output both engines produce.
#[derive(Clone, Copy)]
struct FirstOrder {
    delta: f64,
    vega: f64,
    theta: f64,
    rho_dom: f64,
    rho_for: f64,
}

/// Bump-and-revalue first-order block by **central differences**: two repricings
/// per factor over `{S, σ, t, r_dom, r_for}` — the O(n) baseline AAD replaces.
#[inline]
fn bump_first_order(opt: OptionType, i: &VanillaInputs) -> FirstOrder {
    let h_s = 1e-5 * i.spot;
    let h_v = 1e-5;
    let h_t = 1e-6;
    let h_r = 1e-6;
    let p = |spot, vol, t, rd, rf| price(opt, &VanillaInputs::new(spot, i.strike, vol, t, rd, rf));
    let delta = (p(i.spot + h_s, i.vol, i.t, i.r_dom, i.r_for)
        - p(i.spot - h_s, i.vol, i.t, i.r_dom, i.r_for))
        / (2.0 * h_s);
    let vega = (p(i.spot, i.vol + h_v, i.t, i.r_dom, i.r_for)
        - p(i.spot, i.vol - h_v, i.t, i.r_dom, i.r_for))
        / (2.0 * h_v);
    // Desk theta is −∂V/∂T.
    let theta = -(p(i.spot, i.vol, i.t + h_t, i.r_dom, i.r_for)
        - p(i.spot, i.vol, i.t - h_t, i.r_dom, i.r_for))
        / (2.0 * h_t);
    let rho_dom = (p(i.spot, i.vol, i.t, i.r_dom + h_r, i.r_for)
        - p(i.spot, i.vol, i.t, i.r_dom - h_r, i.r_for))
        / (2.0 * h_r);
    let rho_for = (p(i.spot, i.vol, i.t, i.r_dom, i.r_for + h_r)
        - p(i.spot, i.vol, i.t, i.r_dom, i.r_for - h_r))
        / (2.0 * h_r);
    FirstOrder {
        delta,
        vega,
        theta,
        rho_dom,
        rho_for,
    }
}

/// AAD first-order block: one reverse sweep yields the whole gradient at once.
#[divan::bench]
fn first_order_aad(bencher: Bencher) {
    let inputs: VanillaInputs = representative_inputs();
    bencher.bench_local(|| {
        let g = adjoint_greeks(OptionType::Call, black_box(&inputs));
        // Fold the five first-order outputs so none is elided.
        g.delta_spot + g.vega + g.theta + g.rho_dom + g.rho_for
    });
}

/// Bump-and-revalue first-order block: O(n) central-difference repricings.
#[divan::bench]
fn first_order_bump(bencher: Bencher) {
    let inputs: VanillaInputs = representative_inputs();
    bencher.bench_local(|| {
        let f = bump_first_order(OptionType::Call, black_box(&inputs));
        f.delta + f.vega + f.theta + f.rho_dom + f.rho_for
    });
}
