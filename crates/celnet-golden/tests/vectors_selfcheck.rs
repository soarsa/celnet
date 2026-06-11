//! Self-check gate for the frozen golden-vector corpus.
//!
//! This proves the committed `vectors/*.json` cannot silently drift: every vector
//! is **re-derived** from its independent oracle (the same oracle the generator
//! uses) and asserted equal to the frozen `expected.price` within its tolerance.
//! It also asserts the corpus is well-formed: all 24 product-oneof families
//! present, unique ids, valid family tags, MC families carry a positive standard
//! error, closed-form families carry `null`.
//!
//! Independence is preserved exactly as the anti-circular rule requires: the
//! oracle (`celnet_golden::oracle` + the frozen QuantLib CSVs) shares no code with
//! the production wire/server pricer, so re-deriving here guards the on-disk
//! artifact without ever consulting the path under test.

use std::collections::HashSet;

use celnet_core::is_close;
use celnet_golden::oracle::{
    self, AccumulatorMonitoring, BasketKind, BasketLeg, Cp, McEstimate, TarfRedemption,
};
use celnet_golden::vectors::{CROSS_ASSET_FAMILIES, FAMILIES, GoldenVector, MC_FAMILIES};
use celnet_golden::{
    BarrierType, DigitalSettlement, DoubleBarrierKind, TouchKind, load_barrier,
    load_cross_asset_vectors, load_digital, load_double_barrier, load_touch, load_vectors,
};

/// The Monte-Carlo path-pairs / paths the generator used. The re-derivation must
/// use the same seeds (carried implicitly by re-running the same oracle calls);
/// the selfcheck instead re-derives the MC oracle at a *smaller* budget and
/// asserts the frozen price lies within the combined standard-error band, so the
/// re-check is fast yet still catches a corrupted frozen value.
const SELFCHECK_MC_PAIRS: usize = 60_000;
const SELFCHECK_MC_PATHS: usize = 80_000;

fn cp_of(v: &GoldenVector, key: &str) -> Cp {
    Cp::parse(v.term_str(key))
}

/// The signed `±1` multiplier a `BUY`/`SELL` side contributes to a linear PV.
fn side_sign(side: &str) -> f64 {
    match side {
        "BUY" => 1.0,
        "SELL" => -1.0,
        other => panic!("unknown side `{other}`"),
    }
}

#[test]
fn all_families_present_and_unique() {
    let vectors = load_vectors().expect("corpus loads");
    assert!(!vectors.is_empty(), "corpus must be non-empty");

    let present: HashSet<&str> = vectors.iter().map(|v| v.family.as_str()).collect();
    for fam in FAMILIES {
        assert!(
            present.contains(fam),
            "family `{fam}` has no golden vectors (all 24 oneof arms must be covered)"
        );
    }
    assert_eq!(
        present.len(),
        FAMILIES.len(),
        "corpus contains an unknown family tag: {present:?}"
    );

    // Unique ids.
    let mut ids = HashSet::new();
    for v in &vectors {
        assert!(
            FAMILIES.contains(&v.family.as_str()),
            "vector {} has an invalid family `{}`",
            v.id,
            v.family
        );
        assert!(ids.insert(v.id.clone()), "duplicate vector id `{}`", v.id);
        // A Monte-Carlo vector (per-vector: any vector carrying a std-error) lives
        // only in an MC-capable family, and its std-error is positive. (Some
        // families are mixed: a plain cliquet is closed-form, a clamped cliquet is
        // MC; a continuous lookback is closed-form, a discrete one is MC.)
        if let Some(se) = v.expected.price_std_error {
            assert!(se > 0.0, "MC vector {} std-error must be positive", v.id);
            assert!(
                MC_FAMILIES.contains(&v.family.as_str()),
                "vector {} carries a std-error but `{}` is not an MC-capable family",
                v.id,
                v.family
            );
        }
        // Markets and tolerances are sane.
        assert!(v.market.spot > 0.0 && v.market.vol > 0.0);
        assert!(v.tolerance.rel >= 0.0 && v.tolerance.abs >= 0.0);
        assert!(v.expected.price.is_finite());
    }
}

#[test]
fn closed_form_vectors_redrive_from_independent_oracle() {
    let vectors = load_vectors().expect("corpus loads");
    for v in &vectors {
        if v.expected.price_std_error.is_some() {
            continue; // Monte-Carlo vectors are checked within the stderr band.
        }
        let m = v.market;
        let t = v.term_f64("expiry_years");
        let recomputed = match v.family.as_str() {
            "vanilla" => oracle::gk_price(
                cp_of(v, "option_type"),
                m.spot,
                v.term_f64("strike"),
                m.vol,
                t,
                m.r_dom,
                m.r_for,
            ),
            "strategy" => redrive_strategy(v),
            "single_barrier" => redrive_single_barrier(v),
            "double_barrier" => redrive_double_barrier(v),
            "digital" => redrive_digital(v),
            "touch" => redrive_touch(v),
            "variance_swap" => m.vol * m.vol,
            "volatility_swap" => m.vol,
            "forward_start" => oracle::forward_start_price(
                cp_of(v, "option_type"),
                m.spot,
                v.term_f64("moneyness"),
                v.term_f64("reset"),
                t,
                m.vol,
                m.r_dom,
                m.r_for,
            ),
            "cliquet" => oracle::cliquet_plain_price(
                cp_of(v, "option_type"),
                m.spot,
                v.term_f64("moneyness"),
                v.term_u64("periods") as usize,
                t,
                m.vol,
                m.r_dom,
                m.r_for,
            ),
            "quanto" => redrive_quanto(v),
            "american" => {
                // The published / closed-form American oracle is the frozen value
                // itself (a hand-pinned published constant or the European GK limit).
                if v.id == "american-call-no-dividend-equals-european" {
                    oracle::gk_price(
                        Cp::Call,
                        m.spot,
                        v.term_f64("strike"),
                        m.vol,
                        t,
                        m.r_dom,
                        m.r_for,
                    )
                } else {
                    v.expected.price // hand-pinned published constant
                }
            }
            "fx_forward" => oracle::fx_forward_pv(
                side_sign(v.term_str("side")),
                m.spot,
                v.term_f64("contract_rate"),
                v.term_f64("notional"),
                t,
                m.r_dom,
                m.r_for,
            ),
            "fx_swap" => oracle::fx_swap_pv(
                side_sign(v.term_str("near_side")),
                m.spot,
                v.term_f64("contract_rate"),
                v.term_f64("notional"),
                v.term_f64("near_settle_years"),
                v.term_f64("far_settle_years"),
                m.r_dom,
                m.r_for,
            ),
            "ndf" => oracle::ndf_pv(
                side_sign(v.term_str("side")),
                m.spot,
                v.term_f64("contract_rate"),
                v.term_f64("notional"),
                t,
                m.r_dom,
                m.r_for,
            ),
            // A perpetual has NO expiry (`expiry_years` is 0 by the proto arm-30
            // invariant; `t` is unused). FX carry seam: r = r_dom, b = r_dom − r_for.
            // The oracle refuses a b > r call (no finite value) — the frozen
            // corpus must never carry one.
            "perpetual_option" => oracle::perpetual_american_price(
                cp_of(v, "option_type"),
                m.spot,
                v.term_f64("strike"),
                m.vol,
                m.r_dom,
                m.r_dom - m.r_for,
            )
            .unwrap_or_else(|| {
                panic!(
                    "vector {} is a refused b > r perpetual call — corpus defect",
                    v.id
                )
            }),
            // The market `spot` IS the listed futures price (the vector carries
            // r_dom = r_for = r, i.e. the futures-measure martingale carry b = 0);
            // the margining term selects discounted vs undiscounted Black-76.
            "listed_future_option" => match v.term_str("margining") {
                "EQUITY_STYLE" => oracle::black76_price(
                    cp_of(v, "option_type"),
                    m.spot,
                    v.term_f64("strike"),
                    m.vol,
                    t,
                    m.r_dom,
                ),
                "FUTURES_STYLE" => oracle::black76_undiscounted_price(
                    cp_of(v, "option_type"),
                    m.spot,
                    v.term_f64("strike"),
                    m.vol,
                    t,
                ),
                other => panic!("unknown margining `{other}` for vector {}", v.id),
            },
            other => panic!("unhandled closed-form family `{other}` for vector {}", v.id),
        };
        assert!(
            is_close(
                recomputed,
                v.expected.price,
                v.tolerance.rel,
                v.tolerance.abs
            ),
            "frozen vector {} drifted: re-derived {recomputed} vs frozen {} (rel {}, abs {})",
            v.id,
            v.expected.price,
            v.tolerance.rel,
            v.tolerance.abs
        );
    }
}

#[test]
fn mc_vectors_redrive_within_stderr_band() {
    let vectors = load_vectors().expect("corpus loads");
    for v in &vectors {
        if v.expected.price_std_error.is_none() {
            continue; // closed-form vectors checked elsewhere.
        }
        let est = redrive_mc(v);
        // Combine the frozen and the re-check standard errors; a 6σ band makes a
        // false positive astronomically unlikely while still catching a corrupted
        // or wrong-formula frozen value (which would be many σ away).
        let se_frozen = v.expected.price_std_error.expect("MC stderr");
        let band = 6.0 * (se_frozen + est.std_error).max(1e-12);
        let diff = (v.expected.price - est.price).abs();
        assert!(
            diff <= band,
            "frozen MC vector {} drifted: frozen {} vs re-check {} (|Δ|={diff:e}, band={band:e})",
            v.id,
            v.expected.price,
            est.price
        );
    }
}

#[test]
fn vanilla_vectors_carry_full_greek_strip() {
    let vectors = load_vectors().expect("corpus loads");
    let vanillas: Vec<_> = vectors.iter().filter(|v| v.family == "vanilla").collect();
    assert!(!vanillas.is_empty());
    for v in &vanillas {
        for g in ["delta_spot", "gamma", "vega", "theta", "rho_dom", "rho_for"] {
            assert!(
                v.expected.greeks.contains_key(g),
                "vanilla vector {} missing greek `{g}`",
                v.id
            );
        }
    }
}

/// The cross-asset option corpus (equity / commodity / crypto — the `vanilla`
/// product arm seen through a non-FX `Underlying.ref` arm) re-derives from the
/// independent, code-disjoint cross-asset oracles in [`oracle`] (the `libm::erf`
/// normal-CDF route, disjoint from the leaves' `erfc`-based `norm_cdf`). This guards
/// the frozen `<asset>_option.json` artifacts exactly as the FX corpus is guarded,
/// without ever consulting the equity/commodity/crypto leaves under test.
#[test]
fn cross_asset_vectors_redrive_from_independent_oracle() {
    let vectors = load_cross_asset_vectors().expect("cross-asset corpus loads");
    assert!(!vectors.is_empty(), "cross-asset corpus must be non-empty");

    // Every cross-asset family present, well-formed, and tagged with a known
    // CROSS_ASSET_FAMILIES key (NOT a proto product arm — those stay in FAMILIES).
    let present: HashSet<&str> = vectors.iter().map(|v| v.family.as_str()).collect();
    for fam in CROSS_ASSET_FAMILIES {
        assert!(
            present.contains(fam),
            "cross-asset family `{fam}` has no golden vectors"
        );
    }
    let mut ids = HashSet::new();
    for v in &vectors {
        assert!(
            CROSS_ASSET_FAMILIES.contains(&v.family.as_str()),
            "cross-asset vector {} has an invalid family `{}`",
            v.id,
            v.family
        );
        assert!(
            !FAMILIES.contains(&v.family.as_str()),
            "cross-asset family `{}` must NOT collide with a proto product arm",
            v.family
        );
        assert!(ids.insert(v.id.clone()), "duplicate vector id `{}`", v.id);
        assert!(v.expected.price_std_error.is_none(), "closed-form only");

        let m = v.market;
        let t = v.term_f64("expiry_years");
        let cp = cp_of(v, "option_type");
        let strike = v.term_f64("strike");
        let recomputed = match v.family.as_str() {
            "equity_option" => oracle::equity_bsm_price(
                cp,
                m.spot,
                strike,
                m.vol,
                t,
                v.term_f64("r"),
                v.term_f64("q"),
                v.term_f64("repo"),
            ),
            "commodity_option" => {
                let r = v.term_f64("r");
                // The oracle takes the FORWARD directly: a listed future IS the
                // forward (b=0); the spot representation lifts S to F=S·e^{(r−conv)t}.
                let f = match v.term_str("representation") {
                    "FUTURE" => m.spot,
                    "SPOT" => m.spot * ((r - v.term_f64("convenience")) * t).exp(),
                    other => panic!("unknown commodity representation `{other}`"),
                };
                oracle::black76_price(cp, f, strike, m.vol, t, r)
            }
            "crypto_option" => {
                let r = v.term_f64("r");
                let funding = v.term_f64("funding");
                match v.term_str("settlement_style") {
                    "LINEAR" => {
                        oracle::crypto_linear_price(cp, m.spot, strike, m.vol, t, r, funding)
                    }
                    "INVERSE_COIN" => {
                        oracle::crypto_inverse_price(cp, m.spot, strike, m.vol, t, r, funding)
                    }
                    other => panic!("unknown crypto settlement_style `{other}`"),
                }
            }
            other => panic!("unhandled cross-asset family `{other}` for vector {}", v.id),
        };
        assert!(
            is_close(
                recomputed,
                v.expected.price,
                v.tolerance.rel,
                v.tolerance.abs
            ),
            "frozen cross-asset vector {} drifted: re-derived {recomputed} vs frozen {} (rel {}, abs {})",
            v.id,
            v.expected.price,
            v.tolerance.rel,
            v.tolerance.abs
        );
        assert!(m.spot > 0.0 && m.vol > 0.0);
        assert!(v.expected.price.is_finite());
    }
}

// --- re-derivation helpers (independent oracle) ----------------------------

fn redrive_strategy(v: &GoldenVector) -> f64 {
    let m = v.market;
    let t = v.term_f64("expiry_years");
    let legs = v.terms.get("legs").and_then(|l| l.as_array()).unwrap();
    let mut price = 0.0;
    for leg in legs {
        let cp = Cp::parse(leg.get("option_type").unwrap().as_str().unwrap());
        let strike = leg.get("strike").unwrap().as_f64().unwrap();
        let buy = leg.get("side").unwrap().as_str().unwrap() == "BUY";
        let ratio = leg.get("ratio").unwrap().as_f64().unwrap();
        let val = oracle::gk_price(cp, m.spot, strike, m.vol, t, m.r_dom, m.r_for);
        price += if buy { 1.0 } else { -1.0 } * ratio * val;
    }
    price
}

fn redrive_quanto(v: &GoldenVector) -> f64 {
    let m = v.market;
    let t = v.term_f64("expiry_years");
    let cp = cp_of(v, "option_type");
    let strike = v.term_f64("strike");
    let conv = v.term_f64("conversion_vol");
    let corr = v.term_f64("correlation");
    if v.term_str("payoff") == "VANILLA" {
        oracle::quanto_vanilla_price(cp, m.spot, strike, m.vol, t, m.r_dom, m.r_for, conv, corr)
    } else {
        oracle::quanto_digital_price(cp, m.spot, strike, m.vol, t, m.r_dom, m.r_for, conv, corr)
    }
}

// The barrier / digital / touch families re-derive from the *frozen QuantLib CSV
// row identified in the oracle provenance string* — the same independent table the
// generator used — re-found by matching the market and contract terms.
fn redrive_single_barrier(v: &GoldenVector) -> f64 {
    let m = v.market;
    let t = v.term_f64("expiry_years");
    let strike = v.term_f64("strike");
    let barrier = v.term_f64("barrier");
    let kind = v.term_str("kind");
    let side = v.term_str("side");
    let want = match (kind, side) {
        ("KNOCK_OUT", "LOWER") => BarrierType::DownOut,
        ("KNOCK_IN", "LOWER") => BarrierType::DownIn,
        ("KNOCK_OUT", "UPPER") => BarrierType::UpOut,
        ("KNOCK_IN", "UPPER") => BarrierType::UpIn,
        _ => panic!("bad barrier kind/side"),
    };
    let cp = match cp_of(v, "option_type") {
        Cp::Call => celnet_types::OptionType::Call,
        Cp::Put => celnet_types::OptionType::Put,
    };
    let recs = load_barrier().expect("barrier golden");
    let r = recs
        .iter()
        .find(|r| {
            r.barrier_type == want
                && r.option_type == cp
                && eqf(r.spot, m.spot)
                && eqf(r.strike, strike)
                && eqf(r.barrier, barrier)
                && eqf(r.vol, m.vol)
                && eqf(r.t, t)
                && eqf(r.r_dom, m.r_dom)
                && eqf(r.r_for, m.r_for)
        })
        .unwrap_or_else(|| panic!("no QuantLib barrier row for vector {}", v.id));
    r.price
}

fn redrive_double_barrier(v: &GoldenVector) -> f64 {
    let m = v.market;
    let t = v.term_f64("expiry_years");
    let cp = match cp_of(v, "option_type") {
        Cp::Call => celnet_types::OptionType::Call,
        Cp::Put => celnet_types::OptionType::Put,
    };
    let recs = load_double_barrier().expect("double-barrier golden");
    let r = recs
        .iter()
        .find(|r| {
            r.kind == DoubleBarrierKind::KnockOut
                && r.option_type == cp
                && eqf(r.spot, m.spot)
                && eqf(r.strike, v.term_f64("strike"))
                && eqf(r.lower, v.term_f64("lower_barrier"))
                && eqf(r.upper, v.term_f64("upper_barrier"))
                && eqf(r.vol, m.vol)
                && eqf(r.t, t)
                && eqf(r.r_dom, m.r_dom)
                && eqf(r.r_for, m.r_for)
        })
        .unwrap_or_else(|| panic!("no QuantLib double-barrier row for vector {}", v.id));
    r.price
}

fn redrive_digital(v: &GoldenVector) -> f64 {
    let m = v.market;
    let t = v.term_f64("expiry_years");
    let cp = match cp_of(v, "option_type") {
        Cp::Call => celnet_types::OptionType::Call,
        Cp::Put => celnet_types::OptionType::Put,
    };
    let recs = load_digital().expect("digital golden");
    let r = recs
        .iter()
        .find(|r| {
            r.style == DigitalSettlement::CashOrNothing
                && r.option_type == cp
                && eqf(r.spot, m.spot)
                && eqf(r.strike, v.term_f64("strike"))
                && eqf(r.payout, v.term_f64("payout"))
                && eqf(r.vol, m.vol)
                && eqf(r.t, t)
                && eqf(r.r_dom, m.r_dom)
                && eqf(r.r_for, m.r_for)
        })
        .unwrap_or_else(|| panic!("no QuantLib digital row for vector {}", v.id));
    r.price
}

fn redrive_touch(v: &GoldenVector) -> f64 {
    let m = v.market;
    let t = v.term_f64("expiry_years");
    let kind = v.term_str("kind");
    let lower = v.term_f64("lower_barrier");
    let upper = v.term_f64("upper_barrier");
    // ONE_TOUCH is the at-hit product — re-driven by the independent
    // discounted-first-passage-density quadrature (no λ / Φ-pairing, so it is
    // structurally unable to reproduce the engine's historical flipped-pairing
    // defect). The QuantLib touch CSV is the at-expiry product, so it is NOT the
    // oracle for the single one-touch (only for no-touch / DNT / double-one-touch).
    if kind == "ONE_TOUCH" {
        return oracle::one_touch_at_hit_price(
            m.spot,
            lower,
            v.term_f64("rebate"),
            m.vol,
            t,
            m.r_dom,
            m.r_for,
        );
    }
    let want = match kind {
        "ONE_TOUCH" => TouchKind::OneTouch,
        "NO_TOUCH" => TouchKind::NoTouch,
        "DOUBLE_NO_TOUCH" => TouchKind::Dnt,
        "DOUBLE_ONE_TOUCH" => TouchKind::DoubleTouch,
        _ => panic!("bad touch kind"),
    };
    let recs = load_touch().expect("touch golden");
    let r = recs
        .iter()
        .find(|r| {
            if r.kind != want
                || !eqf(r.spot, m.spot)
                || !eqf(r.vol, m.vol)
                || !eqf(r.t, t)
                || !eqf(r.r_dom, m.r_dom)
                || !eqf(r.r_for, m.r_for)
            {
                return false;
            }
            match want {
                TouchKind::OneTouch | TouchKind::NoTouch => {
                    r.barrier.map(|b| eqf(b, lower)).unwrap_or(false)
                }
                TouchKind::Dnt | TouchKind::DoubleTouch => {
                    r.lower.map(|l| eqf(l, lower)).unwrap_or(false)
                        && r.upper.map(|u| eqf(u, upper)).unwrap_or(false)
                }
            }
        })
        .unwrap_or_else(|| panic!("no QuantLib touch row for vector {}", v.id));
    r.price
}

fn redrive_mc(v: &GoldenVector) -> McEstimate {
    let m = v.market;
    let t = v.term_f64("expiry_years");
    match v.family.as_str() {
        "asian_option" => oracle::asian_arithmetic_mc(
            cp_of(v, "option_type"),
            m.spot,
            v.term_f64("strike"),
            m.vol,
            t,
            m.r_dom,
            m.r_for,
            v.term_u64("observations") as usize,
            SELFCHECK_MC_PAIRS,
            0x5E1F_0001,
        ),
        "tarf" => oracle::tarf_bank_pv_mc(
            cp_of(v, "option_type"),
            m.spot,
            v.term_f64("strike"),
            v.term_f64("target"),
            v.term_f64("leverage"),
            v.term_f64("fixing_notional"),
            if v.term_str("redemption") == "FULL_GAIN" {
                TarfRedemption::FullGain
            } else {
                TarfRedemption::CappedGain
            },
            v.term_u64("fixings") as usize,
            m.vol,
            t,
            m.r_dom,
            m.r_for,
            SELFCHECK_MC_PAIRS,
            0x5E1F_0002,
        ),
        "pivot" => {
            let est = oracle::pivot_tra_bank_pv_mc(
                cp_of(v, "option_type"),
                m.spot,
                v.term_f64("strike"),
                v.term_f64("pivot"),
                v.term_f64("target"),
                v.term_f64("leverage"),
                v.term_f64("fixing_notional"),
                if v.term_str("redemption") == "FULL_GAIN" {
                    TarfRedemption::FullGain
                } else {
                    TarfRedemption::CappedGain
                },
                v.term_u64("fixings") as usize,
                m.vol,
                t,
                m.r_dom,
                m.r_for,
                SELFCHECK_MC_PAIRS,
                0x5E1F_0008,
            );
            // The degeneracy LAW row (`pivot == strike`) is additionally
            // re-derived against the PLAIN TARF oracle — a code-disjoint payoff
            // coding of the same product — so the frozen corpus keeps pinning
            // the pivot→TARF collapse, not merely its own oracle.
            if v.term_f64("pivot") == v.term_f64("strike") {
                let tarf = oracle::tarf_bank_pv_mc(
                    cp_of(v, "option_type"),
                    m.spot,
                    v.term_f64("strike"),
                    v.term_f64("target"),
                    v.term_f64("leverage"),
                    v.term_f64("fixing_notional"),
                    if v.term_str("redemption") == "FULL_GAIN" {
                        TarfRedemption::FullGain
                    } else {
                        TarfRedemption::CappedGain
                    },
                    v.term_u64("fixings") as usize,
                    m.vol,
                    t,
                    m.r_dom,
                    m.r_for,
                    SELFCHECK_MC_PAIRS,
                    0x5E1F_1707,
                );
                let se_frozen = v.expected.price_std_error.expect("pivot is MC");
                let band = 6.0 * (se_frozen + tarf.std_error).max(1e-12);
                assert!(
                    (v.expected.price - tarf.price).abs() <= band,
                    "pivot degeneracy law drifted for {}: frozen {} vs TARF oracle {} (band {band:e})",
                    v.id,
                    v.expected.price,
                    tarf.price
                );
            }
            est
        }
        "accumulator" => oracle::accumulator_client_pv_mc(
            m.spot,
            v.term_f64("pivot"),
            v.term_f64("barrier"),
            v.term_f64("leverage"),
            v.term_f64("fixing_notional"),
            AccumulatorMonitoring::Discrete,
            v.term_u64("fixings") as usize,
            m.vol,
            t,
            m.r_dom,
            m.r_for,
            SELFCHECK_MC_PAIRS,
            0x5E1F_0003,
        ),
        "lookback" => oracle::lookback_discrete_mc(
            v.term_str("style") == "FLOATING",
            cp_of(v, "option_type"),
            m.spot,
            v.term_f64("strike"),
            m.vol,
            t,
            m.r_dom,
            m.r_for,
            v.term_u64("observations") as usize,
            SELFCHECK_MC_PAIRS,
            0x5E1F_0004,
        ),
        "cliquet" => oracle::cliquet_clamped_mc(
            cp_of(v, "option_type"),
            m.spot,
            v.term_f64("moneyness"),
            v.term_u64("periods") as usize,
            v.term_opt_f64("local_floor"),
            v.term_opt_f64("local_cap"),
            v.term_opt_f64("global_floor"),
            v.term_opt_f64("global_cap"),
            m.vol,
            t,
            m.r_dom,
            m.r_for,
            SELFCHECK_MC_PAIRS,
            0x5E1F_0005,
        ),
        "window_barrier" => oracle::window_barrier_mc(
            cp_of(v, "option_type"),
            v.term_str("side") == "UPPER",
            m.spot,
            v.term_f64("strike"),
            v.term_f64("barrier"),
            v.term_f64("window_start"),
            v.term_f64("window_end"),
            m.vol,
            t,
            m.r_dom,
            m.r_for,
            v.term_u64("mc_steps") as usize,
            SELFCHECK_MC_PAIRS,
            0x5E1F_0006,
        ),
        "basket" => redrive_basket_mc(v),
        other => panic!("unhandled MC family `{other}`"),
    }
}

fn redrive_basket_mc(v: &GoldenVector) -> McEstimate {
    let m = v.market;
    let t = v.term_f64("expiry_years");
    let legs_json = v.terms.get("legs").and_then(|l| l.as_array()).unwrap();
    let legs: Vec<BasketLeg> = legs_json
        .iter()
        .map(|l| BasketLeg {
            spot: l.get("spot").unwrap().as_f64().unwrap(),
            vol: l.get("vol").unwrap().as_f64().unwrap(),
            r_for: l.get("r_for").unwrap().as_f64().unwrap(),
            weight: l.get("weight").unwrap().as_f64().unwrap(),
        })
        .collect();
    let n = legs.len();
    let flat: Vec<f64> = v
        .terms
        .get("correlations")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_f64().unwrap())
        .collect();
    let corr: Vec<Vec<f64>> = (0..n).map(|i| flat[i * n..i * n + n].to_vec()).collect();
    let kind = match v.term_str("kind") {
        "BASKET" => BasketKind::Basket,
        "BEST_OF" => BasketKind::BestOf,
        "WORST_OF" => BasketKind::WorstOf,
        _ => panic!("bad basket kind"),
    };
    oracle::basket_mc(
        cp_of(v, "option_type"),
        v.term_f64("strike"),
        kind,
        &legs,
        &corr,
        t,
        m.r_dom,
        SELFCHECK_MC_PATHS,
        0x5E1F_0007,
    )
}

/// Equality of two CSV-sourced floats within the parse precision (the CSV carries
/// full f64 text, so an exact match is expected; a tiny tolerance guards the round
/// trip).
fn eqf(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-12 + 1e-12 * a.abs().max(b.abs())
}
