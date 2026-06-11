//! Parity rows — the **pivot Target-Redemption Accumulator** (proto arm 32,
//! `Instrument.product.pivot`) priced by [`celnet_exotics::pivot_tra_price`] /
//! [`celnet_exotics::pivot_tra_price_cv`] on the agnostic carry seam reproduces
//! genuinely independent oracles.
//!
//! Three gates, none of which re-runs the engine as its own check (the
//! FRTB-0.75ρ circular-oracle lesson):
//!
//!   (i)   **Code-disjoint Monte-Carlo oracle**
//!         ([`celnet_golden::oracle::pivot_tra_bank_pv_mc`]): a `splitmix64`
//!         RNG + Box–Muller normals + the indicator-form piecewise-linear
//!         payoff — sharing neither the production counter RNG (Philox), nor
//!         its Acklam inverse-CDF normal, nor its branch-on-`d_pivot` payoff
//!         coding, nor its antithetic+control-variate estimator. `celnet-golden`
//!         does not depend on `celnet-exotics`, so the routes share no code.
//!         The oracle is free to disagree; these rows assert it does not,
//!         within Monte-Carlo confidence bands, across sides × redemption
//!         styles × {dead-band `P > K`, overlap `P < K`} geometries.
//!   (ii)  **The degeneracy law** (`pivot == strike` ⇒ the exact plain TARF):
//!         asserted `to_bits` on the engine pair (the production pivot pricer
//!         vs the production TARF pricer — same RNG coordinates by design, the
//!         engine's frozen contract), AND statistically across the two
//!         **disjoint golden oracles** (the pivot-coding splitmix64 MC vs the
//!         TARF-coding splitmix64 MC at distinct seeds) — the same financial
//!         law encoded twice on each side of the engine/oracle divide.
//!   (iii) **Model-free monotonicity, cross-checked on both routes**: raising
//!         the adverse-leg gearing raises the bank's PV — asserted
//!         independently on the engine and on the golden oracle (agreeing
//!         monotonicity on disjoint implementations is evidence about the
//!         product, not about either code path).

use celnet_exotics::pivot::{PivotTra, PivotTraMcConfig, pivot_tra_price, pivot_tra_price_cv};
use celnet_exotics::tarf::{RedemptionStyle, Tarf, TarfMcConfig, tarf_price};
use celnet_golden::oracle::{self, Cp, TarfRedemption};
use celnet_types::{OptionType, VanillaInputs};

fn cp(opt: OptionType) -> Cp {
    match opt {
        OptionType::Call => Cp::Call,
        OptionType::Put => Cp::Put,
    }
}

fn style(redemption: RedemptionStyle) -> TarfRedemption {
    match redemption {
        RedemptionStyle::FullGain => TarfRedemption::FullGain,
        RedemptionStyle::CappedGain => TarfRedemption::CappedGain,
    }
}

/// The shared 1Y EURUSD-scale market the rows price against.
fn market() -> VanillaInputs {
    VanillaInputs::new(1.30, 1.30, 0.12, 1.0, 0.03, 0.01)
}

/// (i) Engine (variance-reduced antithetic + control variate) vs the
/// code-disjoint splitmix64 indicator-form oracle, across the full
/// side × redemption × geometry grid. The band is the combined Monte-Carlo
/// confidence interval — tight enough that a payoff/sign/gearing/redemption
/// slip on either route fails by many σ.
#[test]
fn pivot_engine_matches_code_disjoint_golden_oracle() {
    let i = market();
    for side in [OptionType::Call, OptionType::Put] {
        for redemption in [RedemptionStyle::FullGain, RedemptionStyle::CappedGain] {
            // Dead band (P on the favourable side of K) and overlap (P on the
            // adverse side), both relative to spot 1.30.
            for &(strike, pivot) in &[(1.28, 1.33), (1.33, 1.28)] {
                let spec = PivotTra {
                    strike,
                    pivot,
                    fixings: 12,
                    target: 0.08,
                    leverage: 2.0,
                    favourable_side: side,
                    notional: 1.0,
                    redemption,
                };
                let engine = pivot_tra_price_cv(
                    &(&i).into(),
                    spec,
                    PivotTraMcConfig {
                        pairs: 150_000,
                        seed: 0x9170_77AA,
                    },
                );
                let reference = oracle::pivot_tra_bank_pv_mc(
                    cp(side),
                    i.spot,
                    strike,
                    pivot,
                    spec.target,
                    spec.leverage,
                    spec.notional,
                    style(redemption),
                    spec.fixings,
                    i.vol,
                    i.t,
                    i.r_dom,
                    i.r_for,
                    300_000,
                    0xA11CE_1707,
                );
                let band = 4.0 * (engine.std_error + reference.std_error) + 1e-9;
                assert!(
                    (engine.price - reference.price).abs() < band,
                    "pivot disagreement side={side:?} redemption={redemption:?} K={strike} \
                     P={pivot}: engine {} vs golden oracle {} (band {band})",
                    engine.price,
                    reference.price
                );
            }
        }
    }
}

/// (ii) The degeneracy law `pivot == strike` ⇒ the exact plain TARF, on both
/// sides of the engine/oracle divide:
///  * engine: `pivot_tra_price` at `P = K` is `to_bits`-identical to
///    `tarf_price` on the same terms/seed (price AND std-error AND the
///    redemption diagnostics) — the engine's frozen bit-contract;
///  * golden: the pivot-coding splitmix64 oracle at `P = K` agrees with the
///    TARF-coding splitmix64 oracle at a DISTINCT seed within the combined
///    Monte-Carlo band — two disjoint payoff codings of one product.
#[test]
fn pivot_at_strike_collapses_to_tarf_on_both_routes() {
    let i = market();
    for redemption in [RedemptionStyle::FullGain, RedemptionStyle::CappedGain] {
        // Engine route: bitwise.
        let piv = PivotTra {
            strike: 1.32,
            pivot: 1.32,
            fixings: 12,
            target: 0.06,
            leverage: 2.0,
            favourable_side: OptionType::Put,
            notional: 1.0,
            redemption,
        };
        let tarf = Tarf {
            strike: 1.32,
            fixings: 12,
            target: 0.06,
            leverage: 2.0,
            favourable_side: OptionType::Put,
            notional: 1.0,
            redemption,
        };
        let p = pivot_tra_price(
            &(&i).into(),
            piv,
            PivotTraMcConfig {
                pairs: 100_000,
                seed: 0x7A4F,
            },
        );
        let t = tarf_price(
            &(&i).into(),
            tarf,
            TarfMcConfig {
                pairs: 100_000,
                seed: 0x7A4F,
            },
        );
        assert_eq!(
            p.price.to_bits(),
            t.price.to_bits(),
            "engine pivot(P=K) {} must equal engine TARF {} bit-for-bit",
            p.price,
            t.price
        );
        assert_eq!(p.std_error.to_bits(), t.std_error.to_bits());
        assert_eq!(
            p.expected_redemption_fixing.to_bits(),
            t.expected_redemption_fixing.to_bits()
        );
        assert_eq!(
            p.expected_overshoot.to_bits(),
            t.expected_overshoot.to_bits()
        );

        // Golden route: two disjoint payoff codings, distinct seeds.
        let po = oracle::pivot_tra_bank_pv_mc(
            Cp::Put,
            i.spot,
            1.32,
            1.32,
            0.06,
            2.0,
            1.0,
            style(redemption),
            12,
            i.vol,
            i.t,
            i.r_dom,
            i.r_for,
            200_000,
            0x9170_7001,
        );
        let to = oracle::tarf_bank_pv_mc(
            Cp::Put,
            i.spot,
            1.32,
            0.06,
            2.0,
            1.0,
            style(redemption),
            12,
            i.vol,
            i.t,
            i.r_dom,
            i.r_for,
            200_000,
            0x7A1F_7001,
        );
        let band = 6.0 * (po.std_error + to.std_error);
        assert!(
            (po.price - to.price).abs() < band,
            "golden degeneracy law violated ({redemption:?}): pivot-coding {} vs \
             tarf-coding {} (band {band})",
            po.price,
            to.price
        );
    }
}

/// (iii) Model-free gearing monotonicity, independently on both routes: with
/// common random numbers per route, raising the adverse-leg leverage strictly
/// raises the bank's PV. Each route is compared only against itself across the
/// leverage axis (no engine↔oracle value reuse), so agreement of the two
/// monotonicity verdicts is a genuinely two-witness law check.
#[test]
fn higher_gearing_raises_bank_pv_on_both_routes() {
    let i = market();
    let spec = |leverage: f64| PivotTra {
        strike: 1.28,
        pivot: 1.33,
        fixings: 12,
        target: 0.08,
        leverage,
        favourable_side: OptionType::Call,
        notional: 1.0,
        redemption: RedemptionStyle::FullGain,
    };
    let cfg = PivotTraMcConfig {
        pairs: 150_000,
        seed: 0x6EA2,
    };
    let e1 = pivot_tra_price_cv(&(&i).into(), spec(1.0), cfg);
    let e3 = pivot_tra_price_cv(&(&i).into(), spec(3.0), cfg);
    assert!(
        e3.price > e1.price,
        "engine: more gearing must raise bank PV: {} !> {}",
        e3.price,
        e1.price
    );

    let oracle_at = |leverage: f64| {
        oracle::pivot_tra_bank_pv_mc(
            Cp::Call,
            i.spot,
            1.28,
            1.33,
            0.08,
            leverage,
            1.0,
            TarfRedemption::FullGain,
            12,
            i.vol,
            i.t,
            i.r_dom,
            i.r_for,
            200_000,
            0xC0FF_EE,
        )
    };
    let o1 = oracle_at(1.0);
    let o3 = oracle_at(3.0);
    assert!(
        o3.price > o1.price,
        "oracle: more gearing must raise bank PV: {} !> {}",
        o3.price,
        o1.price
    );
}
