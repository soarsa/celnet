//! Generator for the frozen golden-vector corpus.
//!
//! Run with `cargo run -p celnet-golden --bin gen_vectors`. Emits
//! `crates/celnet-golden/vectors/<family>.json` for all 21 product-oneof families,
//! every `expected.price` produced by an oracle **independent of the production
//! wire/server path** (see `celnet_golden::vectors` module docs and the per-family
//! comments below). The output is a frozen artifact, committed to disk; the
//! generator is reproducible (the Monte-Carlo families use fixed seeds).
//!
//! Anti-circular-oracle guarantee: this binary depends on `celnet_golden::oracle`
//! (independent closed forms + a code-disjoint `splitmix64` Monte-Carlo) and the
//! frozen QuantLib CSV tables — it does **not** call `celnet-exotics` or any
//! server pricer to produce an expected value.

use std::collections::BTreeMap;

use celnet_golden::oracle::{
    self, AccumulatorMonitoring, BasketKind, BasketLeg, Cp, McEstimate, TarfRedemption,
};
use celnet_golden::vectors::{Expected, GoldenVector, Market, Tolerance, vectors_file};
use celnet_golden::{
    BarrierType, DigitalSettlement, DoubleBarrierKind, TouchKind, load_barrier, load_digital,
    load_double_barrier, load_touch, load_vanilla,
};
use serde_json::json;

/// Standard Monte-Carlo path-pairs for the corpus oracle. Large enough that the
/// reported standard error is small relative to the price, so the `k·stderr`
/// conformance band is tight, while keeping generation fast.
const MC_PAIRS: usize = 400_000;
/// Basket Monte-Carlo paths (single replication block; the standard error is the
/// path stderr).
const MC_BASKET_PATHS: usize = 600_000;
/// The Monte-Carlo budget the SDK request asks the SERVER to use (encoded in the
/// MC vectors' `mc_*` terms). Kept modest so the conformance harness stays fast;
/// the `k·(oracle_se + server_se)` band auto-widens for the larger server stderr,
/// and the oracle's own expected value is computed at the much larger budgets
/// above (independent of what the server is asked to run).
const SERVER_MC_PAIRS: u64 = 20_000;
/// Loose tolerance pair for the Monte-Carlo families (the `k·stderr` band governs).
const MC_TOL: Tolerance = Tolerance {
    rel: 5e-2,
    abs: 5e-3,
};

fn cp_token(cp: Cp) -> &'static str {
    match cp {
        Cp::Call => "CALL",
        Cp::Put => "PUT",
    }
}

/// Map a QuantLib-CSV market row's fields into the corpus market.
fn market(spot: f64, vol: f64, r_dom: f64, r_for: f64) -> Market {
    Market {
        spot,
        vol,
        r_dom,
        r_for,
    }
}

fn write_family(family: &str, vectors: &[GoldenVector]) {
    let path = vectors_file(family);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).expect("create vectors dir");
    }
    // Pretty JSON, sorted by id within the file for a stable diff.
    let mut v = vectors.to_vec();
    v.sort_by(|a, b| a.id.cmp(&b.id));
    let text = serde_json::to_string_pretty(&v).expect("serialize vectors");
    std::fs::write(&path, format!("{text}\n")).expect("write vectors file");
    println!("wrote {} ({} vectors)", path.display(), v.len());
}

fn main() {
    gen_vanilla();
    gen_strategy();
    gen_single_barrier();
    gen_double_barrier();
    gen_digital();
    gen_touch();
    gen_variance_swap();
    gen_volatility_swap();
    gen_asian();
    gen_forward_start();
    gen_cliquet();
    gen_quanto();
    gen_tarf();
    gen_accumulator();
    gen_lookback();
    gen_window_barrier();
    gen_american();
    gen_basket();
    gen_fx_forward();
    gen_fx_swap();
    gen_ndf();
    println!("corpus generation complete.");
}

// ===========================================================================
// fx_forward — oracle: independent two-zero-coupon-bond discounted-cashflow form
//   PV = side·notional·(spot·e^{−r_for·t} − K·e^{−r_dom·t})  (per unit base)
// ===========================================================================

/// Map a `BUY`/`SELL` token to the signed multiplier the linear PV uses.
fn side_sign(side: &str) -> f64 {
    match side {
        "BUY" => 1.0,
        "SELL" => -1.0,
        other => panic!("unknown side `{other}`"),
    }
}

fn gen_fx_forward() {
    // Each case: (id, underlying, side, spot, contract_rate, r_dom, r_for, t).
    // `vol` is irrelevant to a linear forward but the market carries a positive
    // value (the corpus market shape requires vol > 0). notional is 1 (the corpus
    // price is per 1 unit of base notional).
    struct Case {
        id: &'static str,
        underlying: &'static str,
        side: &'static str,
        spot: f64,
        contract_rate: f64,
        r_dom: f64,
        r_for: f64,
        t: f64,
    }
    let cases = [
        Case {
            id: "fxforward-eurusd-1y-buy-fair",
            underlying: "EURUSD",
            side: "BUY",
            spot: 1.10,
            // Struck AT the fair forward F = 1.10·e^{(0.02−0.01)·1} ⇒ PV ≈ 0
            // (a structural anchor that a forward/discount slip cannot satisfy).
            contract_rate: 1.10 * (0.02f64 - 0.01).exp(),
            r_dom: 0.02,
            r_for: 0.01,
            t: 1.0,
        },
        Case {
            id: "fxforward-eurusd-1y-buy-itm",
            underlying: "EURUSD",
            side: "BUY",
            spot: 1.10,
            contract_rate: 1.08,
            r_dom: 0.02,
            r_for: 0.01,
            t: 1.0,
        },
        Case {
            id: "fxforward-eurusd-6m-sell-otm",
            underlying: "EURUSD",
            side: "SELL",
            spot: 1.30,
            contract_rate: 1.34,
            r_dom: 0.03,
            r_for: 0.01,
            t: 0.5,
        },
        Case {
            id: "fxforward-usdjpy-2y-buy",
            underlying: "USDJPY",
            side: "BUY",
            spot: 150.0,
            contract_rate: 145.0,
            r_dom: 0.04,
            r_for: 0.005,
            t: 2.0,
        },
    ];
    let mut out = Vec::new();
    for c in &cases {
        let price = oracle::fx_forward_pv(
            side_sign(c.side),
            c.spot,
            c.contract_rate,
            1.0,
            c.t,
            c.r_dom,
            c.r_for,
        );
        out.push(GoldenVector {
            id: c.id.to_owned(),
            family: "fx_forward".to_owned(),
            underlying: c.underlying.to_owned(),
            tenor: tenor_token(c.t),
            // vol = 0.10 placeholder (unused by a linear DCF; market requires > 0).
            market: market(c.spot, 0.10, c.r_dom, c.r_for),
            terms: json!({
                "contract_rate": c.contract_rate,
                "notional": 1.0,
                "side": c.side,
                "expiry_years": c.t,
            }),
            expected: Expected {
                price,
                greeks: BTreeMap::new(),
                price_std_error: None,
                oracle: "independent two-zero-coupon-bond DCF: \
                         side·N·(spot·e^{−r_for·t} − K·e^{−r_dom·t})"
                    .to_owned(),
            },
            // Closed-form to closed-form (the production path is the algebraically
            // identical df·(F−K) form): tight tolerance.
            tolerance: Tolerance {
                rel: 1e-9,
                abs: 1e-11,
            },
        });
    }
    write_family("fx_forward", &out);
}

// ===========================================================================
// fx_swap — oracle: independent sum of two outright forwards (near + opposite far)
// ===========================================================================

fn gen_fx_swap() {
    // Each case: (id, underlying, near_side, spot, contract_rate, r_dom, r_for,
    //             far_t).
    //
    // The swap's NEAR leg settles at the spot date (the valuation horizon, t = 0)
    // and the FAR leg at the instrument's forward tenor (`expiry_years` = far_t),
    // trading the opposite side — exactly the model the one wire contract carries
    // (`celnet_proto::FxSwap` has no per-leg settle time; the server fixes
    // near = 0 / far = expiry_years, see `pricer::SWAP_NEAR_SETTLE_YEARS`). The
    // economically meaningful swap quantity is the forward-points spread between
    // the two legs, carried entirely by that near = 0 / far = expiry separation.
    // The oracle prices the same two-leg sum by the independent two-bond route.
    const NEAR_T: f64 = 0.0;
    struct Case {
        id: &'static str,
        underlying: &'static str,
        near_side: &'static str,
        spot: f64,
        contract_rate: f64,
        r_dom: f64,
        r_for: f64,
        far_t: f64,
    }
    let cases = [
        Case {
            id: "fxswap-eurusd-spot9m-buynear",
            underlying: "EURUSD",
            near_side: "BUY",
            spot: 1.10,
            contract_rate: 1.10,
            r_dom: 0.02,
            r_for: 0.01,
            far_t: 0.75,
        },
        Case {
            id: "fxswap-eurusd-spot1y-sellnear",
            underlying: "EURUSD",
            near_side: "SELL",
            spot: 1.30,
            contract_rate: 1.30,
            r_dom: 0.03,
            r_for: 0.01,
            far_t: 1.0,
        },
        Case {
            id: "fxswap-usdjpy-spot18m-buynear",
            underlying: "USDJPY",
            near_side: "BUY",
            spot: 150.0,
            contract_rate: 150.0,
            r_dom: 0.04,
            r_for: 0.005,
            far_t: 1.5,
        },
    ];
    let mut out = Vec::new();
    for c in &cases {
        let price = oracle::fx_swap_pv(
            side_sign(c.near_side),
            c.spot,
            c.contract_rate,
            1.0,
            NEAR_T,
            c.far_t,
            c.r_dom,
            c.r_for,
        );
        out.push(GoldenVector {
            id: c.id.to_owned(),
            family: "fx_swap".to_owned(),
            underlying: c.underlying.to_owned(),
            tenor: tenor_token(c.far_t),
            market: market(c.spot, 0.10, c.r_dom, c.r_for),
            terms: json!({
                "contract_rate": c.contract_rate,
                "notional": 1.0,
                "near_side": c.near_side,
                "near_settle_years": NEAR_T,
                "far_settle_years": c.far_t,
                "expiry_years": c.far_t,
            }),
            expected: Expected {
                price,
                greeks: BTreeMap::new(),
                price_std_error: None,
                oracle: "independent sum of two outright forwards \
                         (near side + opposite-side far), each the two-bond DCF"
                    .to_owned(),
            },
            tolerance: Tolerance {
                rel: 1e-9,
                abs: 1e-11,
            },
        });
    }
    write_family("fx_swap", &out);
}

// ===========================================================================
// ndf — oracle: hand-derived side·N·df_settle·(F−K) == deliverable-forward PV in
//   the same numeraire (via the independent two-bond DCF). Fixing identity is
//   metadata only and does not enter the PV.
// ===========================================================================

fn gen_ndf() {
    // Each case: (id, underlying, side, spot, contract_rate, r_dom, r_for, t,
    //             fixing, settlement_ccy). The restricted-leg pairs are USD-quote
    //             NDFs (BRL/INR/COP); the convertible settlement leg is USD, so
    //             `r_dom` is the settlement (USD) discounting rate.
    struct Case {
        id: &'static str,
        underlying: &'static str,
        side: &'static str,
        spot: f64,
        contract_rate: f64,
        r_dom: f64,
        r_for: f64,
        t: f64,
        fixing: &'static str,
        settlement_ccy: &'static str,
    }
    let cases = [
        Case {
            id: "ndf-usdbrl-6m-buy",
            underlying: "USDBRL",
            side: "BUY",
            spot: 5.0,
            contract_rate: 5.1,
            // r_dom = BRL (restricted leg of the spot quote), r_for = USD. The PV
            // is the two-bond DCF in the BRL-equivalent numeraire; the structural
            // NDF==deliverable identity proves no separate route is used.
            r_dom: 0.10,
            r_for: 0.05,
            t: 0.5,
            fixing: "BrlPtax",
            settlement_ccy: "USD",
        },
        Case {
            id: "ndf-usdinr-1y-sell",
            underlying: "USDINR",
            side: "SELL",
            spot: 83.0,
            contract_rate: 84.0,
            r_dom: 0.066,
            r_for: 0.05,
            t: 1.0,
            fixing: "InrRbiRef",
            settlement_ccy: "USD",
        },
        Case {
            id: "ndf-usdcop-3m-buy",
            underlying: "USDCOP",
            side: "BUY",
            spot: 4000.0,
            contract_rate: 4050.0,
            r_dom: 0.095,
            r_for: 0.05,
            t: 0.25,
            fixing: "CopTrm",
            settlement_ccy: "USD",
        },
    ];
    let mut out = Vec::new();
    for c in &cases {
        let price = oracle::ndf_pv(
            side_sign(c.side),
            c.spot,
            c.contract_rate,
            1.0,
            c.t,
            c.r_dom,
            c.r_for,
        );
        out.push(GoldenVector {
            id: c.id.to_owned(),
            family: "ndf".to_owned(),
            underlying: c.underlying.to_owned(),
            tenor: tenor_token(c.t),
            market: market(c.spot, 0.10, c.r_dom, c.r_for),
            terms: json!({
                "contract_rate": c.contract_rate,
                "notional": 1.0,
                "side": c.side,
                "fixing": c.fixing,
                "settlement_ccy": c.settlement_ccy,
                "expiry_years": c.t,
            }),
            expected: Expected {
                price,
                greeks: BTreeMap::new(),
                price_std_error: None,
                oracle: "hand-derived NDF PV == deliverable-forward PV in same \
                         numeraire (independent two-bond DCF); fixing is metadata"
                    .to_owned(),
            },
            tolerance: Tolerance {
                rel: 1e-9,
                abs: 1e-11,
            },
        });
    }
    write_family("ndf", &out);
}

// ===========================================================================
// vanilla — oracle: frozen QuantLib vanilla_gk.csv (price + full Greek strip)
// ===========================================================================

fn gen_vanilla() {
    let recs = load_vanilla().expect("load vanilla golden");
    // Pick a spread of representative rows across call/put, moneyness, maturity.
    let picks = [120usize, 480, 905, 1330, 1755, 2180, 2605, 3050];
    let mut out = Vec::new();
    for &idx in &picks {
        let r = &recs[idx % recs.len()];
        let cp = match r.option_type {
            celnet_types::OptionType::Call => Cp::Call,
            celnet_types::OptionType::Put => Cp::Put,
        };
        let mut greeks = BTreeMap::new();
        greeks.insert("delta_spot".to_owned(), r.delta_spot);
        greeks.insert("gamma".to_owned(), r.gamma);
        greeks.insert("vega".to_owned(), r.vega);
        greeks.insert("theta".to_owned(), r.theta);
        greeks.insert("rho_dom".to_owned(), r.rho_dom);
        greeks.insert("rho_for".to_owned(), r.rho_for);
        out.push(GoldenVector {
            id: format!("vanilla-{}-k{}-t{}-{}", idx, r.strike, r.t, cp_token(cp)),
            family: "vanilla".to_owned(),
            underlying: "EURUSD".to_owned(),
            tenor: tenor_token(r.t),
            market: market(r.spot, r.vol, r.r_dom, r.r_for),
            terms: json!({
                "option_type": cp_token(cp),
                "strike": r.strike,
                "expiry_years": r.t,
            }),
            expected: Expected {
                price: r.price,
                greeks,
                price_std_error: None,
                oracle: format!("quantlib-1.42.1 vanilla_gk.csv row {idx}"),
            },
            // QuantLib analytic GK is reproduced by the production path to last-ULP;
            // the vanilla golden gate already proves ~1e-9 / 1e-11.
            tolerance: Tolerance {
                rel: 1e-7,
                abs: 1e-9,
            },
        });
    }
    write_family("vanilla", &out);
}

fn tenor_token(t: f64) -> String {
    // Best-effort label; the priced maturity is `expiry_years`, this is display.
    let months = (t * 12.0).round() as i64;
    if months % 12 == 0 && months > 0 {
        format!("{}Y", months / 12)
    } else {
        format!("{months}M")
    }
}

// ===========================================================================
// strategy — oracle: sum of independent vanilla-leg GK oracle values
// ===========================================================================

fn gen_strategy() {
    // Each entry: (id, underlying, spot/vol/rates, expiry, legs[(cp, strike, side, ratio)]).
    // `side`: BUY (+1) / SELL (−1); the strategy PV is Σ side·ratio·GK(leg).
    struct Leg {
        cp: Cp,
        strike: f64,
        buy: bool,
        ratio: f64,
    }
    struct Case {
        id: &'static str,
        kind: &'static str,
        spot: f64,
        vol: f64,
        r_dom: f64,
        r_for: f64,
        t: f64,
        legs: Vec<Leg>,
    }
    let l = |cp, strike, buy, ratio| Leg {
        cp,
        strike,
        buy,
        ratio,
    };
    let cases = vec![
        Case {
            id: "strategy-eurusd-1y-straddle",
            kind: "STRADDLE",
            spot: 1.10,
            vol: 0.105,
            r_dom: 0.02,
            r_for: 0.01,
            t: 1.0,
            legs: vec![l(Cp::Call, 1.11, true, 1.0), l(Cp::Put, 1.11, true, 1.0)],
        },
        Case {
            id: "strategy-eurusd-6m-strangle",
            kind: "STRANGLE",
            spot: 1.10,
            vol: 0.12,
            r_dom: 0.02,
            r_for: 0.01,
            t: 0.5,
            legs: vec![l(Cp::Call, 1.16, true, 1.0), l(Cp::Put, 1.04, true, 1.0)],
        },
        Case {
            id: "strategy-eurusd-1y-risk-reversal",
            kind: "RISK_REVERSAL",
            spot: 1.10,
            vol: 0.10,
            r_dom: 0.02,
            r_for: 0.015,
            t: 1.0,
            legs: vec![l(Cp::Call, 1.18, true, 1.0), l(Cp::Put, 1.02, false, 1.0)],
        },
        Case {
            id: "strategy-eurusd-2y-straddle",
            kind: "STRADDLE",
            spot: 1.10,
            vol: 0.11,
            r_dom: 0.025,
            r_for: 0.01,
            t: 2.0,
            legs: vec![l(Cp::Call, 1.13, true, 1.0), l(Cp::Put, 1.13, true, 1.0)],
        },
        Case {
            id: "strategy-eurusd-1y-seagull",
            kind: "SEAGULL",
            spot: 1.10,
            vol: 0.105,
            r_dom: 0.02,
            r_for: 0.01,
            t: 1.0,
            legs: vec![
                l(Cp::Call, 1.12, true, 1.0),
                l(Cp::Call, 1.22, false, 1.0),
                l(Cp::Put, 1.00, false, 1.0),
            ],
        },
    ];
    let mut out = Vec::new();
    for c in &cases {
        let mut price = 0.0;
        let mut legs_json = Vec::new();
        for leg in &c.legs {
            let v = oracle::gk_price(leg.cp, c.spot, leg.strike, c.vol, c.t, c.r_dom, c.r_for);
            let sign = if leg.buy { 1.0 } else { -1.0 };
            price += sign * leg.ratio * v;
            legs_json.push(json!({
                "option_type": cp_token(leg.cp),
                "strike": leg.strike,
                "side": if leg.buy { "BUY" } else { "SELL" },
                "ratio": leg.ratio,
            }));
        }
        out.push(GoldenVector {
            id: c.id.to_owned(),
            family: "strategy".to_owned(),
            underlying: "EURUSD".to_owned(),
            tenor: tenor_token(c.t),
            market: market(c.spot, c.vol, c.r_dom, c.r_for),
            terms: json!({ "kind": c.kind, "legs": legs_json, "expiry_years": c.t }),
            expected: Expected {
                price,
                greeks: BTreeMap::new(),
                price_std_error: None,
                oracle: "independent Σ side·ratio·GK(leg) of the vanilla closed form".to_owned(),
            },
            tolerance: Tolerance {
                rel: 1e-7,
                abs: 1e-9,
            },
        });
    }
    write_family("strategy", &out);
}

// ===========================================================================
// single_barrier — oracle: frozen QuantLib barrier_gk.csv
// ===========================================================================

fn gen_single_barrier() {
    let recs = load_barrier().expect("load barrier golden");
    // Pick representative rows covering all four knock kinds × call/put.
    let mut out = Vec::new();
    let mut taken_kinds: Vec<(BarrierType, celnet_types::OptionType)> = Vec::new();
    for (idx, r) in recs.iter().enumerate() {
        // Only knock-OUT barriers (the SDK single_barrier exposes KO/KI; KO is the
        // server's primal path). Cover down/up × call/put, a few each.
        let key = (r.barrier_type, r.option_type);
        let count = taken_kinds.iter().filter(|k| **k == key).count();
        if count >= 2 {
            continue;
        }
        // Skip rebated rows (rebate must be 0 for this simple oracle mapping).
        if r.rebate != 0.0 {
            continue;
        }
        taken_kinds.push(key);
        let cp = match r.option_type {
            celnet_types::OptionType::Call => Cp::Call,
            celnet_types::OptionType::Put => Cp::Put,
        };
        let (kind, side) = match r.barrier_type {
            BarrierType::DownOut => ("KNOCK_OUT", "LOWER"),
            BarrierType::DownIn => ("KNOCK_IN", "LOWER"),
            BarrierType::UpOut => ("KNOCK_OUT", "UPPER"),
            BarrierType::UpIn => ("KNOCK_IN", "UPPER"),
        };
        out.push(GoldenVector {
            id: format!("single-barrier-{idx}-{:?}-{}", r.barrier_type, cp_token(cp)),
            family: "single_barrier".to_owned(),
            underlying: "EURUSD".to_owned(),
            tenor: tenor_token(r.t),
            market: market(r.spot, r.vol, r.r_dom, r.r_for),
            terms: json!({
                "option_type": cp_token(cp),
                "strike": r.strike,
                "kind": kind,
                "side": side,
                "barrier": r.barrier,
                "rebate": r.rebate,
                "monitoring": "CONTINUOUS",
                "expiry_years": r.t,
            }),
            expected: Expected {
                price: r.price,
                greeks: BTreeMap::new(),
                price_std_error: None,
                oracle: format!("quantlib-1.42.1 barrier_gk.csv row {idx}"),
            },
            tolerance: Tolerance {
                rel: 1e-6,
                abs: 1e-8,
            },
        });
        if out.len() >= 8 {
            break;
        }
    }
    write_family("single_barrier", &out);
}

// ===========================================================================
// double_barrier — oracle: frozen QuantLib double_barrier_gk.csv
// ===========================================================================

fn gen_double_barrier() {
    let recs = load_double_barrier().expect("load double-barrier golden");
    let mut out = Vec::new();
    for (idx, r) in recs.iter().enumerate() {
        // Knock-out corridor (the server's primal double-barrier path).
        if r.kind != DoubleBarrierKind::KnockOut {
            continue;
        }
        let cp = match r.option_type {
            celnet_types::OptionType::Call => Cp::Call,
            celnet_types::OptionType::Put => Cp::Put,
        };
        out.push(GoldenVector {
            id: format!("double-barrier-{idx}-{}", cp_token(cp)),
            family: "double_barrier".to_owned(),
            underlying: "EURUSD".to_owned(),
            tenor: tenor_token(r.t),
            market: market(r.spot, r.vol, r.r_dom, r.r_for),
            terms: json!({
                "option_type": cp_token(cp),
                "strike": r.strike,
                "kind": "KNOCK_OUT",
                "lower_barrier": r.lower,
                "upper_barrier": r.upper,
                "rebate": 0.0,
                "monitoring": "CONTINUOUS",
                "expiry_years": r.t,
            }),
            expected: Expected {
                price: r.price,
                greeks: BTreeMap::new(),
                price_std_error: None,
                oracle: format!("quantlib-1.42.1 double_barrier_gk.csv row {idx}"),
            },
            tolerance: Tolerance {
                rel: 1e-5,
                abs: 1e-7,
            },
        });
        if out.len() >= 6 {
            break;
        }
    }
    write_family("double_barrier", &out);
}

// ===========================================================================
// digital — oracle: frozen QuantLib digital_gk.csv
// ===========================================================================

fn gen_digital() {
    let recs = load_digital().expect("load digital golden");
    let mut out = Vec::new();
    let mut taken: Vec<(DigitalSettlement, celnet_types::OptionType)> = Vec::new();
    for (idx, r) in recs.iter().enumerate() {
        // The SDK/wire digital is cash-or-nothing with an explicit payout; cover
        // both call/put. (Asset-or-nothing is the asset leg; the wire digital is
        // cash-or-nothing, so restrict to CASH rows for an exact mapping.)
        if r.style != DigitalSettlement::CashOrNothing {
            continue;
        }
        let key = (r.style, r.option_type);
        if taken.iter().filter(|k| **k == key).count() >= 3 {
            continue;
        }
        taken.push(key);
        let cp = match r.option_type {
            celnet_types::OptionType::Call => Cp::Call,
            celnet_types::OptionType::Put => Cp::Put,
        };
        out.push(GoldenVector {
            id: format!("digital-{idx}-{}", cp_token(cp)),
            family: "digital".to_owned(),
            underlying: "EURUSD".to_owned(),
            tenor: tenor_token(r.t),
            market: market(r.spot, r.vol, r.r_dom, r.r_for),
            terms: json!({
                "option_type": cp_token(cp),
                "strike": r.strike,
                "style": "CASH_OR_NOTHING",
                "payout": r.payout,
                "expiry_years": r.t,
            }),
            expected: Expected {
                price: r.price,
                greeks: BTreeMap::new(),
                price_std_error: None,
                oracle: format!("quantlib-1.42.1 digital_gk.csv row {idx}"),
            },
            tolerance: Tolerance {
                rel: 1e-6,
                abs: 1e-8,
            },
        });
        if out.len() >= 6 {
            break;
        }
    }
    write_family("digital", &out);
}

// ===========================================================================
// touch — oracle: frozen QuantLib touch_gk.csv
// ===========================================================================

fn gen_touch() {
    let recs = load_touch().expect("load touch golden");
    let mut out = Vec::new();
    let mut taken: Vec<TouchKind> = Vec::new();
    for (idx, r) in recs.iter().enumerate() {
        if taken.iter().filter(|k| **k == r.kind).count() >= 2 {
            continue;
        }
        taken.push(r.kind);
        let (kind_token, lower, upper) = match r.kind {
            TouchKind::OneTouch => ("ONE_TOUCH", r.barrier.unwrap(), 0.0),
            TouchKind::NoTouch => ("NO_TOUCH", r.barrier.unwrap(), 0.0),
            TouchKind::Dnt => ("DOUBLE_NO_TOUCH", r.lower.unwrap(), r.upper.unwrap()),
            TouchKind::DoubleTouch => ("DOUBLE_ONE_TOUCH", r.lower.unwrap(), r.upper.unwrap()),
        };
        // The wire/server one-touch pays the rebate AT HIT (Reiner-Rubinstein),
        // whereas the QuantLib touch CSV is the AT-EXPIRY (deferred-rebate) product.
        // So the CSV is the correct oracle for NO_TOUCH / DOUBLE_NO_TOUCH /
        // DOUBLE_ONE_TOUCH (all at-expiry by construction), but for the single
        // ONE_TOUCH we use the independent at-hit closed form re-derived in `oracle`.
        let (price, oracle_str, tol) = match r.kind {
            TouchKind::OneTouch => (
                oracle::one_touch_at_hit_price(
                    r.spot,
                    r.barrier.unwrap(),
                    r.rebate,
                    r.vol,
                    r.t,
                    r.r_dom,
                    r.r_for,
                ),
                "Reiner-Rubinstein at-hit one-touch closed form (independent)".to_owned(),
                Tolerance {
                    rel: 1e-6,
                    abs: 1e-8,
                },
            ),
            _ => (
                r.price,
                format!("quantlib-1.42.1 touch_gk.csv row {idx} (at-expiry)"),
                Tolerance {
                    rel: 1e-5,
                    abs: 1e-7,
                },
            ),
        };
        out.push(GoldenVector {
            id: format!("touch-{idx}-{kind_token}"),
            family: "touch".to_owned(),
            underlying: "EURUSD".to_owned(),
            tenor: tenor_token(r.t),
            market: market(r.spot, r.vol, r.r_dom, r.r_for),
            terms: json!({
                "kind": kind_token,
                "lower_barrier": lower,
                "upper_barrier": upper,
                "rebate": r.rebate,
                "monitoring": "CONTINUOUS",
                "expiry_years": r.t,
            }),
            expected: Expected {
                price,
                greeks: BTreeMap::new(),
                price_std_error: None,
                oracle: oracle_str,
            },
            tolerance: tol,
        });
        if out.len() >= 8 {
            break;
        }
    }
    write_family("touch", &out);
}

// ===========================================================================
// variance_swap — oracle: flat-smile closed form K_var = σ²
// ===========================================================================

fn gen_variance_swap() {
    let cases = [
        ("varswap-eurusd-1y", 1.10, 0.105, 0.02, 0.01, 1.0),
        ("varswap-eurusd-6m", 1.30, 0.08, 0.03, 0.01, 0.5),
        ("varswap-usdjpy-2y", 150.0, 0.12, 0.04, 0.005, 2.0),
        ("varswap-eurusd-3m", 1.10, 0.15, 0.025, 0.02, 0.25),
        ("varswap-gbpusd-1y", 1.27, 0.09, 0.045, 0.04, 1.0),
    ];
    let mut out = Vec::new();
    for (id, spot, vol, r_dom, r_for, t) in cases {
        // The wire `price` for a var swap is the fair *variance* strike K_var; under
        // a flat smile this is exactly σ² (model-free, independent of any quadrature).
        let price = vol * vol;
        out.push(GoldenVector {
            id: id.to_owned(),
            family: "variance_swap".to_owned(),
            underlying: "EURUSD".to_owned(),
            tenor: tenor_token(t),
            market: market(spot, vol, r_dom, r_for),
            terms: json!({ "strike_vol": 0.0, "expiry_years": t }),
            expected: Expected {
                price,
                greeks: BTreeMap::new(),
                price_std_error: None,
                oracle: "flat-smile model-free fair variance K_var = σ²".to_owned(),
            },
            tolerance: Tolerance {
                rel: 1e-6,
                abs: 1e-9,
            },
        });
    }
    write_family("variance_swap", &out);
}

// ===========================================================================
// volatility_swap — oracle: flat-smile closed form K_vol = σ (zero convexity)
// ===========================================================================

fn gen_volatility_swap() {
    let cases = [
        ("volswap-eurusd-1y", 1.10, 0.105, 0.02, 0.01, 1.0),
        ("volswap-eurusd-6m", 1.30, 0.08, 0.03, 0.01, 0.5),
        ("volswap-usdjpy-2y", 150.0, 0.12, 0.04, 0.005, 2.0),
        ("volswap-eurusd-3m", 1.10, 0.15, 0.025, 0.02, 0.25),
        ("volswap-gbpusd-1y", 1.27, 0.09, 0.045, 0.04, 1.0),
    ];
    let mut out = Vec::new();
    for (id, spot, vol, r_dom, r_for, t) in cases {
        // Under a flat smile the convexity gap is zero, so the fair vol strike
        // K_vol = √K_var = σ exactly (independent of the Carr-Lee quadrature).
        let price = vol;
        out.push(GoldenVector {
            id: id.to_owned(),
            family: "volatility_swap".to_owned(),
            underlying: "EURUSD".to_owned(),
            tenor: tenor_token(t),
            market: market(spot, vol, r_dom, r_for),
            terms: json!({ "strike_vol": 0.0, "expiry_years": t }),
            expected: Expected {
                price,
                greeks: BTreeMap::new(),
                price_std_error: None,
                oracle: "flat-smile fair vol K_vol = σ (zero convexity gap)".to_owned(),
            },
            tolerance: Tolerance {
                rel: 1e-6,
                abs: 1e-9,
            },
        });
    }
    write_family("volatility_swap", &out);
}

// ===========================================================================
// asian_option — MC family: code-disjoint arithmetic-average GBM Monte-Carlo
// ===========================================================================

fn gen_asian() {
    struct Case {
        id: &'static str,
        cp: Cp,
        spot: f64,
        strike: f64,
        vol: f64,
        r_dom: f64,
        r_for: f64,
        t: f64,
        obs: usize,
    }
    let cases = [
        Case {
            id: "asian-eurusd-1y-call-atm-12obs",
            cp: Cp::Call,
            spot: 1.10,
            strike: 1.10,
            vol: 0.105,
            r_dom: 0.02,
            r_for: 0.01,
            t: 1.0,
            obs: 12,
        },
        Case {
            id: "asian-eurusd-1y-put-atm-12obs",
            cp: Cp::Put,
            spot: 1.10,
            strike: 1.10,
            vol: 0.105,
            r_dom: 0.02,
            r_for: 0.01,
            t: 1.0,
            obs: 12,
        },
        Case {
            id: "asian-eurusd-6m-call-otm-6obs",
            cp: Cp::Call,
            spot: 1.10,
            strike: 1.16,
            vol: 0.12,
            r_dom: 0.03,
            r_for: 0.01,
            t: 0.5,
            obs: 6,
        },
        Case {
            id: "asian-usdjpy-1y-call-atm-4obs",
            cp: Cp::Call,
            spot: 150.0,
            strike: 150.0,
            vol: 0.10,
            r_dom: 0.04,
            r_for: 0.005,
            t: 1.0,
            obs: 4,
        },
    ];
    let mut out = Vec::new();
    for (n, c) in cases.iter().enumerate() {
        let seed = 0xA51A_0000 + n as u64;
        let est = oracle::asian_arithmetic_mc(
            c.cp, c.spot, c.strike, c.vol, c.t, c.r_dom, c.r_for, c.obs, MC_PAIRS, seed,
        );
        out.push(mc_vector(
            c.id,
            "asian_option",
            &underlying_for(c.spot),
            c.t,
            market(c.spot, c.vol, c.r_dom, c.r_for),
            json!({
                "option_type": cp_token(c.cp),
                "strike": c.strike,
                "averaging": "DISCRETE",
                "observations": c.obs,
                "method": "CURRAN",
                "elapsed_avg": 0.0,
                "elapsed_weight": 0.0,
                "expiry_years": c.t,
            }),
            est,
            "code-disjoint splitmix64 arithmetic-average GBM Monte-Carlo",
        ));
    }
    write_family("asian_option", &out);
}

// ===========================================================================
// forward_start — oracle: Rubinstein closed form, re-derived independently
// ===========================================================================

fn gen_forward_start() {
    struct Case {
        id: &'static str,
        cp: Cp,
        spot: f64,
        moneyness: f64,
        reset: f64,
        vol: f64,
        r_dom: f64,
        r_for: f64,
        t: f64,
    }
    let cases = [
        Case {
            id: "forwardstart-eurusd-1y-atm-reset3m",
            cp: Cp::Call,
            spot: 1.10,
            moneyness: 1.0,
            reset: 0.25,
            vol: 0.105,
            r_dom: 0.02,
            r_for: 0.01,
            t: 1.0,
        },
        Case {
            id: "forwardstart-eurusd-1y-put-atm-reset6m",
            cp: Cp::Put,
            spot: 1.10,
            moneyness: 1.0,
            reset: 0.5,
            vol: 0.11,
            r_dom: 0.025,
            r_for: 0.01,
            t: 1.0,
        },
        Case {
            id: "forwardstart-eurusd-2y-otm-reset1y",
            cp: Cp::Call,
            spot: 1.10,
            moneyness: 1.05,
            reset: 1.0,
            vol: 0.10,
            r_dom: 0.02,
            r_for: 0.015,
            t: 2.0,
        },
        Case {
            id: "forwardstart-usdjpy-1y-call-reset3m",
            cp: Cp::Call,
            spot: 150.0,
            moneyness: 0.98,
            reset: 0.25,
            vol: 0.12,
            r_dom: 0.04,
            r_for: 0.005,
            t: 1.0,
        },
    ];
    let mut out = Vec::new();
    for c in &cases {
        let price = oracle::forward_start_price(
            c.cp,
            c.spot,
            c.moneyness,
            c.reset,
            c.t,
            c.vol,
            c.r_dom,
            c.r_for,
        );
        out.push(GoldenVector {
            id: c.id.to_owned(),
            family: "forward_start".to_owned(),
            underlying: underlying_for(c.spot),
            tenor: tenor_token(c.t),
            market: market(c.spot, c.vol, c.r_dom, c.r_for),
            terms: json!({
                "option_type": cp_token(c.cp),
                "moneyness": c.moneyness,
                "reset": c.reset,
                "expiry_years": c.t,
            }),
            expected: Expected {
                price,
                greeks: BTreeMap::new(),
                price_std_error: None,
                oracle: "Rubinstein 1990 FX dual-carry forward-start closed form".to_owned(),
            },
            tolerance: Tolerance {
                rel: 1e-7,
                abs: 1e-9,
            },
        });
    }
    write_family("forward_start", &out);
}

// ===========================================================================
// cliquet — plain ratchet: Σ forward-start legs (closed form, independent);
//           clamped: code-disjoint Monte-Carlo
// ===========================================================================

fn gen_cliquet() {
    let mut out = Vec::new();
    // Plain (closed-form) ratchets.
    struct Plain {
        id: &'static str,
        cp: Cp,
        spot: f64,
        moneyness: f64,
        periods: usize,
        vol: f64,
        r_dom: f64,
        r_for: f64,
        t: f64,
    }
    let plains = [
        Plain {
            id: "cliquet-eurusd-1y-plain-4p",
            cp: Cp::Call,
            spot: 1.10,
            moneyness: 1.0,
            periods: 4,
            vol: 0.105,
            r_dom: 0.02,
            r_for: 0.01,
            t: 1.0,
        },
        Plain {
            id: "cliquet-eurusd-2y-plain-8p",
            cp: Cp::Call,
            spot: 1.10,
            moneyness: 1.0,
            periods: 8,
            vol: 0.11,
            r_dom: 0.025,
            r_for: 0.01,
            t: 2.0,
        },
        Plain {
            id: "cliquet-eurusd-1y-plain-put-4p",
            cp: Cp::Put,
            spot: 1.10,
            moneyness: 1.0,
            periods: 4,
            vol: 0.10,
            r_dom: 0.02,
            r_for: 0.015,
            t: 1.0,
        },
    ];
    for p in &plains {
        let price = oracle::cliquet_plain_price(
            p.cp,
            p.spot,
            p.moneyness,
            p.periods,
            p.t,
            p.vol,
            p.r_dom,
            p.r_for,
        );
        out.push(GoldenVector {
            id: p.id.to_owned(),
            family: "cliquet".to_owned(),
            underlying: underlying_for(p.spot),
            tenor: tenor_token(p.t),
            market: market(p.spot, p.vol, p.r_dom, p.r_for),
            terms: json!({
                "option_type": cp_token(p.cp),
                "moneyness": p.moneyness,
                "periods": p.periods,
                "local_floor": serde_json::Value::Null,
                "local_cap": serde_json::Value::Null,
                "global_floor": serde_json::Value::Null,
                "global_cap": serde_json::Value::Null,
                "mc_pairs": 0,
                "mc_seed": 0,
                "expiry_years": p.t,
            }),
            expected: Expected {
                price,
                greeks: BTreeMap::new(),
                price_std_error: None,
                oracle: "plain ratchet = Σ forward-start legs (closed form)".to_owned(),
            },
            tolerance: Tolerance {
                rel: 1e-7,
                abs: 1e-9,
            },
        });
    }
    // Clamped (Monte-Carlo) cliquet.
    let est = oracle::cliquet_clamped_mc(
        Cp::Call,
        1.10,
        1.0,
        4,
        None,
        Some(0.04),
        None,
        None,
        0.105,
        1.0,
        0.02,
        0.01,
        MC_PAIRS,
        0xC119_0001,
    );
    out.push(mc_vector(
        "cliquet-eurusd-1y-capped-4p",
        "cliquet",
        "EURUSD",
        1.0,
        market(1.10, 0.105, 0.02, 0.01),
        json!({
            "option_type": "CALL",
            "moneyness": 1.0,
            "periods": 4,
            "local_floor": serde_json::Value::Null,
            "local_cap": 0.04,
            "global_floor": serde_json::Value::Null,
            "global_cap": serde_json::Value::Null,
            "mc_pairs": SERVER_MC_PAIRS,
            "mc_seed": 99,
            "expiry_years": 1.0,
        }),
        est,
        "code-disjoint splitmix64 clamped-cliquet Monte-Carlo",
    ));
    write_family("cliquet", &out);
}

// ===========================================================================
// quanto — oracle: closed-form quanto-drift-adjusted GK, re-derived independently
// ===========================================================================

fn gen_quanto() {
    struct Case {
        id: &'static str,
        payoff: &'static str,
        cp: Cp,
        spot: f64,
        strike: f64,
        vol: f64,
        r_dom: f64,
        r_for: f64,
        t: f64,
        conv_vol: f64,
        corr: f64,
    }
    let cases = [
        Case {
            id: "quanto-eurusd-1y-vanilla-call-rho0",
            payoff: "VANILLA",
            cp: Cp::Call,
            spot: 1.10,
            strike: 1.10,
            vol: 0.105,
            r_dom: 0.02,
            r_for: 0.01,
            t: 1.0,
            conv_vol: 0.08,
            corr: 0.0,
        },
        Case {
            id: "quanto-eurusd-1y-vanilla-call-rhopos",
            payoff: "VANILLA",
            cp: Cp::Call,
            spot: 1.10,
            strike: 1.12,
            vol: 0.105,
            r_dom: 0.02,
            r_for: 0.01,
            t: 1.0,
            conv_vol: 0.09,
            corr: 0.4,
        },
        Case {
            id: "quanto-eurusd-1y-vanilla-put-rhoneg",
            payoff: "VANILLA",
            cp: Cp::Put,
            spot: 1.10,
            strike: 1.08,
            vol: 0.11,
            r_dom: 0.025,
            r_for: 0.01,
            t: 1.0,
            conv_vol: 0.10,
            corr: -0.3,
        },
        Case {
            id: "quanto-eurusd-1y-digital-call",
            payoff: "DIGITAL",
            cp: Cp::Call,
            spot: 1.10,
            strike: 1.12,
            vol: 0.105,
            r_dom: 0.02,
            r_for: 0.01,
            t: 1.0,
            conv_vol: 0.09,
            corr: 0.3,
        },
    ];
    let mut out = Vec::new();
    for c in &cases {
        let price = if c.payoff == "VANILLA" {
            oracle::quanto_vanilla_price(
                c.cp, c.spot, c.strike, c.vol, c.t, c.r_dom, c.r_for, c.conv_vol, c.corr,
            )
        } else {
            oracle::quanto_digital_price(
                c.cp, c.spot, c.strike, c.vol, c.t, c.r_dom, c.r_for, c.conv_vol, c.corr,
            )
        };
        out.push(GoldenVector {
            id: c.id.to_owned(),
            family: "quanto".to_owned(),
            underlying: underlying_for(c.spot),
            tenor: tenor_token(c.t),
            market: market(c.spot, c.vol, c.r_dom, c.r_for),
            terms: json!({
                "payoff": c.payoff,
                "option_type": cp_token(c.cp),
                "strike": c.strike,
                "conversion_vol": c.conv_vol,
                "correlation": c.corr,
                "expiry_years": c.t,
            }),
            expected: Expected {
                price,
                greeks: BTreeMap::new(),
                price_std_error: None,
                oracle: "closed-form quanto-drift-adjusted GK (independent re-derivation)"
                    .to_owned(),
            },
            tolerance: Tolerance {
                rel: 1e-7,
                abs: 1e-9,
            },
        });
    }
    write_family("quanto", &out);
}

// ===========================================================================
// tarf — MC family: code-disjoint TARF bank-PV Monte-Carlo
// ===========================================================================

fn gen_tarf() {
    struct Case {
        id: &'static str,
        cp: Cp,
        spot: f64,
        strike: f64,
        target: f64,
        leverage: f64,
        redemption: TarfRedemption,
        fixings: usize,
        vol: f64,
        r_dom: f64,
        r_for: f64,
        t: f64,
    }
    let cases = [
        Case {
            id: "tarf-eurusd-1y-put-fullgain",
            cp: Cp::Put,
            spot: 1.30,
            strike: 1.30,
            target: 0.10,
            leverage: 2.0,
            redemption: TarfRedemption::FullGain,
            fixings: 12,
            vol: 0.10,
            r_dom: 0.03,
            r_for: 0.01,
            t: 1.0,
        },
        Case {
            id: "tarf-eurusd-1y-put-cappedgain",
            cp: Cp::Put,
            spot: 1.30,
            strike: 1.30,
            target: 0.10,
            leverage: 2.0,
            redemption: TarfRedemption::CappedGain,
            fixings: 12,
            vol: 0.10,
            r_dom: 0.03,
            r_for: 0.01,
            t: 1.0,
        },
        Case {
            id: "tarf-eurusd-1y-call-fullgain",
            cp: Cp::Call,
            spot: 1.10,
            strike: 1.10,
            target: 0.08,
            leverage: 1.5,
            redemption: TarfRedemption::FullGain,
            fixings: 6,
            vol: 0.105,
            r_dom: 0.02,
            r_for: 0.01,
            t: 1.0,
        },
    ];
    let mut out = Vec::new();
    for (n, c) in cases.iter().enumerate() {
        let est = oracle::tarf_bank_pv_mc(
            c.cp,
            c.spot,
            c.strike,
            c.target,
            c.leverage,
            1.0,
            c.redemption,
            c.fixings,
            c.vol,
            c.t,
            c.r_dom,
            c.r_for,
            MC_PAIRS,
            0x7A1F_0000 + n as u64,
        );
        let redemption = match c.redemption {
            TarfRedemption::FullGain => "FULL_GAIN",
            TarfRedemption::CappedGain => "CAPPED_GAIN",
        };
        out.push(mc_vector(
            c.id,
            "tarf",
            &underlying_for(c.spot),
            c.t,
            market(c.spot, c.vol, c.r_dom, c.r_for),
            json!({
                "option_type": cp_token(c.cp),
                "strike": c.strike,
                "target": c.target,
                "leverage": c.leverage,
                "redemption": redemption,
                "fixings": c.fixings,
                "fixing_notional": 1.0,
                "mc_pairs": SERVER_MC_PAIRS,
                "mc_seed": 99,
                "expiry_years": c.t,
            }),
            est,
            "code-disjoint splitmix64 TARF bank-PV Monte-Carlo",
        ));
    }
    write_family("tarf", &out);
}

// ===========================================================================
// accumulator — MC family: code-disjoint accumulator client-PV Monte-Carlo
// (discrete monitoring only — unambiguous payoff)
// ===========================================================================

fn gen_accumulator() {
    struct Case {
        id: &'static str,
        spot: f64,
        pivot: f64,
        barrier: f64,
        leverage: f64,
        fixings: usize,
        vol: f64,
        r_dom: f64,
        r_for: f64,
        t: f64,
    }
    let cases = [
        Case {
            id: "accumulator-eurusd-1y-disc-12f",
            spot: 1.10,
            pivot: 1.08,
            barrier: 1.18,
            leverage: 2.0,
            fixings: 12,
            vol: 0.105,
            r_dom: 0.02,
            r_for: 0.01,
            t: 1.0,
        },
        Case {
            id: "accumulator-eurusd-6m-disc-6f",
            spot: 1.10,
            pivot: 1.09,
            barrier: 1.15,
            leverage: 1.5,
            fixings: 6,
            vol: 0.12,
            r_dom: 0.03,
            r_for: 0.01,
            t: 0.5,
        },
        Case {
            id: "accumulator-usdjpy-1y-disc-12f",
            spot: 150.0,
            pivot: 148.0,
            barrier: 160.0,
            leverage: 2.0,
            fixings: 12,
            vol: 0.10,
            r_dom: 0.04,
            r_for: 0.005,
            t: 1.0,
        },
    ];
    let mut out = Vec::new();
    for (n, c) in cases.iter().enumerate() {
        let est = oracle::accumulator_client_pv_mc(
            c.spot,
            c.pivot,
            c.barrier,
            c.leverage,
            1.0,
            AccumulatorMonitoring::Discrete,
            c.fixings,
            c.vol,
            c.t,
            c.r_dom,
            c.r_for,
            MC_PAIRS,
            0xACC0_0000 + n as u64,
        );
        out.push(mc_vector(
            c.id,
            "accumulator",
            &underlying_for(c.spot),
            c.t,
            market(c.spot, c.vol, c.r_dom, c.r_for),
            json!({
                "pivot": c.pivot,
                "barrier": c.barrier,
                "leverage": c.leverage,
                "monitoring": "DISCRETE",
                "fixings": c.fixings,
                "fixing_notional": 1.0,
                "mc_pairs": SERVER_MC_PAIRS,
                "mc_seed": 99,
                "expiry_years": c.t,
            }),
            est,
            "code-disjoint splitmix64 accumulator client-PV Monte-Carlo (discrete)",
        ));
    }
    write_family("accumulator", &out);
}

// ===========================================================================
// lookback — discrete: MC family; the continuous closed form is the server's
// primal but is re-derived here as an independent closed-form vector too.
// ===========================================================================

fn gen_lookback() {
    let mut out = Vec::new();
    // Discrete-monitoring lookbacks (the server prices these by Monte-Carlo).
    struct Disc {
        id: &'static str,
        floating: bool,
        cp: Cp,
        spot: f64,
        strike: f64,
        obs: usize,
        vol: f64,
        r_dom: f64,
        r_for: f64,
        t: f64,
    }
    let discs = [
        Disc {
            id: "lookback-eurusd-1y-floating-call-disc",
            floating: true,
            cp: Cp::Call,
            spot: 1.10,
            strike: 1.10,
            obs: 16,
            vol: 0.105,
            r_dom: 0.02,
            r_for: 0.01,
            t: 1.0,
        },
        Disc {
            id: "lookback-eurusd-1y-floating-put-disc",
            floating: true,
            cp: Cp::Put,
            spot: 1.10,
            strike: 1.10,
            obs: 16,
            vol: 0.105,
            r_dom: 0.02,
            r_for: 0.01,
            t: 1.0,
        },
        Disc {
            id: "lookback-eurusd-1y-fixed-call-disc",
            floating: false,
            cp: Cp::Call,
            spot: 1.10,
            strike: 1.10,
            obs: 16,
            vol: 0.11,
            r_dom: 0.025,
            r_for: 0.01,
            t: 1.0,
        },
    ];
    for (n, d) in discs.iter().enumerate() {
        let est = oracle::lookback_discrete_mc(
            d.floating,
            d.cp,
            d.spot,
            d.strike,
            d.vol,
            d.t,
            d.r_dom,
            d.r_for,
            d.obs,
            MC_PAIRS,
            0x100B_0000 + n as u64,
        );
        out.push(mc_vector(
            d.id,
            "lookback",
            &underlying_for(d.spot),
            d.t,
            market(d.spot, d.vol, d.r_dom, d.r_for),
            json!({
                "style": if d.floating { "FLOATING" } else { "FIXED" },
                "option_type": cp_token(d.cp),
                "monitoring": "DISCRETE",
                "strike": d.strike,
                "observations": d.obs,
                "mc_pairs": SERVER_MC_PAIRS,
                "mc_seed": 99,
                "expiry_years": d.t,
            }),
            est,
            "code-disjoint splitmix64 discrete-monitoring lookback Monte-Carlo",
        ));
    }
    write_family("lookback", &out);
}

// ===========================================================================
// window_barrier — MC family: code-disjoint window knock-out Monte-Carlo
// ===========================================================================

fn gen_window_barrier() {
    struct Case {
        id: &'static str,
        cp: Cp,
        up: bool,
        spot: f64,
        strike: f64,
        barrier: f64,
        window_start: f64,
        window_end: f64,
        vol: f64,
        r_dom: f64,
        r_for: f64,
        t: f64,
    }
    let cases = [
        Case {
            id: "window-barrier-eurusd-1y-up-out-call",
            cp: Cp::Call,
            up: true,
            spot: 1.10,
            strike: 1.10,
            barrier: 1.25,
            window_start: 0.25,
            window_end: 0.75,
            vol: 0.105,
            r_dom: 0.02,
            r_for: 0.01,
            t: 1.0,
        },
        Case {
            id: "window-barrier-eurusd-1y-down-out-put",
            cp: Cp::Put,
            up: false,
            spot: 1.10,
            strike: 1.10,
            barrier: 0.98,
            window_start: 0.0,
            window_end: 0.5,
            vol: 0.11,
            r_dom: 0.025,
            r_for: 0.01,
            t: 1.0,
        },
        Case {
            id: "window-barrier-eurusd-1y-up-out-call-backwindow",
            cp: Cp::Call,
            up: true,
            spot: 1.10,
            strike: 1.08,
            barrier: 1.22,
            window_start: 0.5,
            window_end: 1.0,
            vol: 0.105,
            r_dom: 0.02,
            r_for: 0.01,
            t: 1.0,
        },
    ];
    let mut out = Vec::new();
    for (n, c) in cases.iter().enumerate() {
        // 128 monitoring steps: the discrete-grid bias is folded into the band
        // because the server's LSV MC engine also monitors discretely; the
        // conformance gate is k·stderr. Use the MC (LSV-MC) engine on the server
        // side (mc_pairs > 0) so both sides are discrete-monitored Monte-Carlo.
        let est = oracle::window_barrier_mc(
            c.cp,
            c.up,
            c.spot,
            c.strike,
            c.barrier,
            c.window_start,
            c.window_end,
            c.vol,
            c.t,
            c.r_dom,
            c.r_for,
            128,
            MC_PAIRS,
            0x1D00_0000 + n as u64,
        );
        // HONEST BOUNDARY: a window barrier has no closed form and is priced ONLY
        // by the production LSV engine (a particle-calibrated stochastic-variance
        // model, ξ = 0.18, ρ = −0.30). The only oracle independent of that path
        // would be *another* LSV implementation — i.e. the production path itself,
        // which is circular. So `expected.price` here is a **flat-GBM (no
        // stochastic-vol) sanity reference**, NOT the LSV price: the LSV value
        // differs by the genuine vol-of-variance barrier effect (~5% on these
        // grids). The conformance gate therefore checks STRUCTURAL invariants
        // (finite, non-negative, strictly below the unbarriered vanilla, and inside
        // a wide documented model band of this reference) rather than a tight match
        // — a real regression gate that does not fake an independent LSV oracle.
        let unbarriered = oracle::gk_price(c.cp, c.spot, c.strike, c.vol, c.t, c.r_dom, c.r_for);
        out.push(GoldenVector {
            id: c.id.to_owned(),
            family: "window_barrier".to_owned(),
            underlying: underlying_for(c.spot),
            tenor: tenor_token(c.t),
            market: market(c.spot, c.vol, c.r_dom, c.r_for),
            terms: json!({
                "option_type": cp_token(c.cp),
                "strike": c.strike,
                "barrier": c.barrier,
                "side": if c.up { "UPPER" } else { "LOWER" },
                "window_start": c.window_start,
                "window_end": c.window_end,
                "unbarriered_vanilla": unbarriered,
                "mc_pairs": SERVER_MC_PAIRS,
                "mc_steps": 128,
                "mc_seed": 99,
                "expiry_years": c.t,
            }),
            expected: Expected {
                price: est.price,
                greeks: BTreeMap::new(),
                price_std_error: Some(est.std_error),
                oracle: "flat-GBM (no-stochastic-vol) sanity reference — NOT the LSV price; \
                         window_barrier is LSV-only, so the gate is structural (see conformance)"
                    .to_owned(),
            },
            // A wide model band: the LSV value sits within ~25% of the flat-GBM
            // reference on these grids (the vol-of-vol barrier effect). This is a
            // sanity bound, not a precision oracle (documented in `oracle`).
            tolerance: Tolerance {
                rel: 0.25,
                abs: 5e-3,
            },
        });
    }
    write_family("window_barrier", &out);
}

// ===========================================================================
// american — oracle: hand-pinned published Longstaff-Schwartz (2001) Table 1
// ===========================================================================

fn gen_american() {
    // Longstaff & Schwartz (2001), Table 1, first row: American PUT with
    // S₀ = K = 40, r = 0.06, σ = 0.20, T = 1, no dividend (r_f = 0). Published
    // finite-difference reference price 2.314 (their LSM column 2.313, s.e. 0.009).
    // This is the canonical published independent oracle. The server FD engine on
    // its default grid must land within FD-grid tolerance of the published value.
    let mut out = Vec::new();
    out.push(GoldenVector {
        id: "american-ls2001-table1-put".to_owned(),
        family: "american".to_owned(),
        underlying: "EURUSD".to_owned(),
        tenor: "1Y".to_owned(),
        market: market(40.0, 0.20, 0.06, 0.0),
        terms: json!({
            "option_type": "PUT",
            "strike": 40.0,
            "style": "AMERICAN",
            "lsm_paths": 0,
            "lsm_exercise_dates": 0,
            "lsm_seed": 0,
            "expiry_years": 1.0,
        }),
        expected: Expected {
            price: 2.314,
            greeks: BTreeMap::new(),
            price_std_error: None,
            oracle: "hand-pinned: Longstaff-Schwartz (2001) Table 1, American put, FD ref 2.314"
                .to_owned(),
        },
        // FD-grid convergence tolerance (the published value is at a fine grid; the
        // server default grid converges to within ~1e-2 of it).
        tolerance: Tolerance {
            rel: 1e-2,
            abs: 2e-2,
        },
    });
    // A second published anchor: an American call with no dividend equals its
    // European value (never optimal to exercise early), an exact independent GK
    // closed-form check that the early-exercise engine collapses correctly.
    let eur_call = oracle::gk_price(Cp::Call, 100.0, 100.0, 0.20, 1.0, 0.05, 0.0);
    out.push(GoldenVector {
        id: "american-call-no-dividend-equals-european".to_owned(),
        family: "american".to_owned(),
        underlying: "EURUSD".to_owned(),
        tenor: "1Y".to_owned(),
        market: market(100.0, 0.20, 0.05, 0.0),
        terms: json!({
            "option_type": "CALL",
            "strike": 100.0,
            "style": "AMERICAN",
            "lsm_paths": 0,
            "lsm_exercise_dates": 0,
            "lsm_seed": 0,
            "expiry_years": 1.0,
        }),
        expected: Expected {
            price: eur_call,
            greeks: BTreeMap::new(),
            price_std_error: None,
            oracle: "American call, r_f=0 ⇒ never exercise early ⇒ = European GK (closed form)"
                .to_owned(),
        },
        tolerance: Tolerance {
            rel: 5e-3,
            abs: 5e-3,
        },
    });
    write_family("american", &out);
}

// ===========================================================================
// basket — MC family: code-disjoint Cholesky-correlated GBM Monte-Carlo
// ===========================================================================

fn gen_basket() {
    let mut out = Vec::new();

    // Two-leg equal-weight basket call, ρ = 0.5.
    let legs2 = vec![
        BasketLeg {
            spot: 1.10,
            vol: 0.105,
            r_for: 0.01,
            weight: 0.5,
        },
        BasketLeg {
            spot: 1.30,
            vol: 0.12,
            r_for: 0.008,
            weight: 0.5,
        },
    ];
    let corr2 = vec![vec![1.0, 0.5], vec![0.5, 1.0]];
    let strike2 = 1.20; // ≈ weighted spot
    let est = oracle::basket_mc(
        Cp::Call,
        strike2,
        BasketKind::Basket,
        &legs2,
        &corr2,
        1.0,
        0.02,
        MC_BASKET_PATHS,
        0xBA53_0001,
    );
    out.push(basket_vector(
        "basket-2leg-call-rho0.5",
        BasketKind::Basket,
        Cp::Call,
        strike2,
        &legs2,
        &corr2,
        market(1.20, 0.10, 0.02, 0.0),
        est,
    ));

    // Two-leg worst-of put, ρ = 0.3.
    let corr2b = vec![vec![1.0, 0.3], vec![0.3, 1.0]];
    let est = oracle::basket_mc(
        Cp::Put,
        1.10,
        BasketKind::WorstOf,
        &legs2,
        &corr2b,
        1.0,
        0.02,
        MC_BASKET_PATHS,
        0xBA53_0002,
    );
    out.push(basket_vector(
        "basket-2leg-worstof-put-rho0.3",
        BasketKind::WorstOf,
        Cp::Put,
        1.10,
        &legs2,
        &corr2b,
        market(1.20, 0.10, 0.02, 0.0),
        est,
    ));

    // Three-leg best-of call.
    let legs3 = vec![
        BasketLeg {
            spot: 1.10,
            vol: 0.105,
            r_for: 0.01,
            weight: 1.0 / 3.0,
        },
        BasketLeg {
            spot: 1.30,
            vol: 0.12,
            r_for: 0.008,
            weight: 1.0 / 3.0,
        },
        BasketLeg {
            spot: 1.25,
            vol: 0.11,
            r_for: 0.012,
            weight: 1.0 / 3.0,
        },
    ];
    let corr3 = vec![
        vec![1.0, 0.4, 0.3],
        vec![0.4, 1.0, 0.35],
        vec![0.3, 0.35, 1.0],
    ];
    let est = oracle::basket_mc(
        Cp::Call,
        0.42,
        BasketKind::BestOf,
        &legs3,
        &corr3,
        1.0,
        0.02,
        MC_BASKET_PATHS,
        0xBA53_0003,
    );
    out.push(basket_vector(
        "basket-3leg-bestof-call",
        BasketKind::BestOf,
        Cp::Call,
        0.42,
        &legs3,
        &corr3,
        market(1.20, 0.10, 0.02, 0.0),
        est,
    ));

    write_family("basket", &out);
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a Monte-Carlo vector (loose tolerance; `k·stderr` conformance band).
#[allow(clippy::too_many_arguments)] // a generator-local builder taking the full vector payload
fn mc_vector(
    id: &str,
    family: &str,
    underlying: &str,
    t: f64,
    market: Market,
    terms: serde_json::Value,
    est: McEstimate,
    oracle_desc: &str,
) -> GoldenVector {
    GoldenVector {
        id: id.to_owned(),
        family: family.to_owned(),
        underlying: underlying.to_owned(),
        tenor: tenor_token(t),
        market,
        terms,
        expected: Expected {
            price: est.price,
            greeks: BTreeMap::new(),
            price_std_error: Some(est.std_error),
            oracle: oracle_desc.to_owned(),
        },
        tolerance: MC_TOL,
    }
}

#[allow(clippy::too_many_arguments)] // a generator-local builder taking the full basket-vector payload
fn basket_vector(
    id: &str,
    kind: BasketKind,
    cp: Cp,
    strike: f64,
    legs: &[BasketLeg],
    corr: &[Vec<f64>],
    market: Market,
    est: McEstimate,
) -> GoldenVector {
    let kind_token = match kind {
        BasketKind::Basket => "BASKET",
        BasketKind::BestOf => "BEST_OF",
        BasketKind::WorstOf => "WORST_OF",
    };
    let legs_json: Vec<_> = legs
        .iter()
        .map(|l| {
            json!({
                "pair": "EURUSD",
                "spot": l.spot,
                "vol": l.vol,
                "r_for": l.r_for,
                "weight": l.weight,
            })
        })
        .collect();
    let corr_flat: Vec<f64> = corr.iter().flatten().copied().collect();
    GoldenVector {
        id: id.to_owned(),
        family: "basket".to_owned(),
        underlying: "EURUSD".to_owned(),
        tenor: "1Y".to_owned(),
        market,
        terms: json!({
            "legs": legs_json,
            "correlations": corr_flat,
            "option_type": cp_token(cp),
            "strike": strike,
            "kind": kind_token,
            "mc_paths": 0,
            "mc_replications": 0,
            "mc_steps": 0,
            "mc_seed": 0,
            "expiry_years": 1.0,
        }),
        expected: Expected {
            price: est.price,
            greeks: BTreeMap::new(),
            price_std_error: Some(est.std_error),
            oracle: "code-disjoint splitmix64 Cholesky-correlated GBM basket Monte-Carlo"
                .to_owned(),
        },
        tolerance: MC_TOL,
    }
}

/// A display underlying token from the spot scale (EURUSD-like vs USDJPY-like).
fn underlying_for(spot: f64) -> String {
    if spot > 10.0 {
        "USDJPY".to_owned()
    } else {
        "EURUSD".to_owned()
    }
}
