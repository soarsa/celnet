//! Cross-client conformance gate — the **Rust SDK** half of the executable oracle.
//!
//! For every frozen golden vector (`celnet-golden/vectors/*.json`) this boots a
//! real in-process `celnet-server` edge, builds the SDK [`InstrumentSpec`] from the
//! vector's `{family, underlying, terms}`, prices it through the typed
//! [`celnet_client`] SDK **against the vector's own market context**, and asserts
//! the returned price (and any quoted Greeks) equals the vector's
//! independent-oracle `expected` — within the vector's tolerance for closed-form
//! families, or within `k · price_std_error` for the Monte-Carlo families.
//!
//! This turns "server == SDK == oracle" from a documentation claim into a gate: a
//! pricing regression on the wire/server path now fails here against an oracle that
//! never touched that path (the QuantLib CSVs / independent closed forms / a
//! code-disjoint Monte-Carlo). Every family is asserted reachable.
//!
//! Bodies are hard wall-clock bounded and every network await is bounded, so a
//! regression fails fast, never hangs.

mod common;

use std::collections::HashSet;
use std::time::Duration;

use celnet_client::{
    AccumulatorMonitoring, AccumulatorTerms, AmericanTerms, AsianMethod, AsianTerms, BarrierKind,
    BarrierSide, BarrierTerms, BasketKind, BasketLegTerms, BasketTerms, CliquetTerms, Conventions,
    DigitalTerms, DoubleBarrierTerms, FixingSource, ForwardSide, ForwardStartTerms, ForwardTerms,
    InstrumentSpec, Leg, LookbackStyle, MarketContext, NdfTerms, PricedLine, Quantity,
    QuantoPayoff, QuantoTerms, Side, StrategyKind, StrikeSpec, SwapTerms, TarfRedemption,
    TarfTerms, TouchTerms,
};
use celnet_golden::{FAMILIES, GoldenVector, load_vectors};
use celnet_types::{CcyPair, OptionType, Tenor};

/// The standard-error multiplier for the Monte-Carlo conformance band. With both
/// the frozen oracle stderr and the server's MC stderr at the corpus path budgets,
/// `k = 4` is a sound balance: tight enough to catch a wrong price, loose enough
/// that a correct independent MC virtually never spuriously fails.
const K_STDERR: f64 = 4.0;

/// A generous per-test deadline covering the heaviest MC family on the server.
const PRICE_DEADLINE: Duration = Duration::from_secs(60);

fn cp(token: &str) -> OptionType {
    match token {
        "CALL" => OptionType::Call,
        "PUT" => OptionType::Put,
        other => panic!("unknown option type `{other}`"),
    }
}

fn tenor_for(t: f64) -> Tenor {
    let months = (t * 12.0).round() as i64;
    if months % 12 == 0 && months > 0 {
        Tenor::Years((months / 12) as u16)
    } else {
        Tenor::Months(months.max(1) as u16)
    }
}

fn pair_for(underlying: &str) -> CcyPair {
    CcyPair::parse(underlying).unwrap_or_else(|| CcyPair::parse("EURUSD").unwrap())
}

fn market_of(v: &GoldenVector) -> MarketContext {
    MarketContext {
        spot: v.market.spot,
        vol: v.market.vol,
        r_dom: v.market.r_dom,
        r_for: v.market.r_for,
    }
}

/// Build the SDK [`InstrumentSpec`] from a vector. The `quantity`/`side` are a
/// neutral unit base / two-way; the priced premium is per unit base (matching the
/// oracle), so the notional does not scale the per-unit price the SDK returns.
fn instrument_of(v: &GoldenVector) -> InstrumentSpec {
    let pair = pair_for(&v.underlying);
    let t = v.term_f64("expiry_years");
    let tenor = tenor_for(t);
    let qty = Quantity::base(1.0);
    let side = Side::TwoWay;
    match v.family.as_str() {
        "vanilla" => InstrumentSpec::vanilla(
            pair,
            tenor,
            t,
            qty,
            side,
            cp(v.term_str("option_type")),
            StrikeSpec::Absolute(v.term_f64("strike")),
        ),
        "strategy" => {
            let kind = match v.term_str("kind") {
                "RISK_REVERSAL" => StrategyKind::RiskReversal,
                "STRANGLE" => StrategyKind::Strangle,
                "STRADDLE" => StrategyKind::Straddle,
                "SEAGULL" => StrategyKind::Seagull,
                other => panic!("unknown strategy kind `{other}`"),
            };
            let legs: Vec<Leg> = v
                .terms
                .get("legs")
                .and_then(|l| l.as_array())
                .unwrap()
                .iter()
                .map(|leg| Leg {
                    option: cp(leg.get("option_type").unwrap().as_str().unwrap()),
                    strike: StrikeSpec::Absolute(leg.get("strike").unwrap().as_f64().unwrap()),
                    side: if leg.get("side").unwrap().as_str().unwrap() == "BUY" {
                        Side::Buy
                    } else {
                        Side::Sell
                    },
                    ratio: leg.get("ratio").unwrap().as_f64().unwrap(),
                })
                .collect();
            InstrumentSpec::strategy(pair, tenor, t, qty, side, kind, legs)
        }
        "single_barrier" => {
            let kind = match v.term_str("kind") {
                "KNOCK_OUT" => BarrierKind::KnockOut,
                "KNOCK_IN" => BarrierKind::KnockIn,
                _ => panic!("bad barrier kind"),
            };
            let bside = match v.term_str("side") {
                "UPPER" => BarrierSide::Up,
                "LOWER" => BarrierSide::Down,
                _ => panic!("bad barrier side"),
            };
            let terms = BarrierTerms::new(
                cp(v.term_str("option_type")),
                StrikeSpec::Absolute(v.term_f64("strike")),
                kind,
                bside,
                v.term_f64("barrier"),
            )
            .rebate(v.term_f64("rebate"));
            InstrumentSpec::single_barrier(pair, tenor, t, qty, side, terms)
        }
        "double_barrier" => {
            let kind = match v.term_str("kind") {
                "KNOCK_OUT" => BarrierKind::KnockOut,
                "KNOCK_IN" => BarrierKind::KnockIn,
                _ => panic!("bad double-barrier kind"),
            };
            let terms = DoubleBarrierTerms::new(
                cp(v.term_str("option_type")),
                StrikeSpec::Absolute(v.term_f64("strike")),
                kind,
                v.term_f64("lower_barrier"),
                v.term_f64("upper_barrier"),
            )
            .rebate(v.term_f64("rebate"));
            InstrumentSpec::double_barrier(pair, tenor, t, qty, side, terms)
        }
        "digital" => {
            let terms = DigitalTerms::cash_or_nothing(
                cp(v.term_str("option_type")),
                v.term_f64("strike"),
                v.term_f64("payout"),
            );
            InstrumentSpec::digital(pair, tenor, t, qty, side, terms)
        }
        "touch" => {
            let lower = v.term_f64("lower_barrier");
            let upper = v.term_f64("upper_barrier");
            let rebate = v.term_f64("rebate");
            let terms = match v.term_str("kind") {
                "ONE_TOUCH" => TouchTerms::one_touch(lower, rebate),
                "NO_TOUCH" => TouchTerms::no_touch(lower, rebate),
                "DOUBLE_NO_TOUCH" => TouchTerms::double_no_touch(lower, upper, rebate),
                "DOUBLE_ONE_TOUCH" => TouchTerms::double_one_touch(lower, upper, rebate),
                _ => panic!("bad touch kind"),
            };
            InstrumentSpec::touch(pair, tenor, t, qty, side, terms)
        }
        "variance_swap" => {
            InstrumentSpec::variance_swap(pair, tenor, t, qty, side, v.term_f64("strike_vol"))
        }
        "volatility_swap" => {
            InstrumentSpec::volatility_swap(pair, tenor, t, qty, side, v.term_f64("strike_vol"))
        }
        "asian_option" => {
            let obs = v.term_u64("observations") as u32;
            let method = match v.term_str("method") {
                "TURNBULL_WAKEMAN" => AsianMethod::TurnbullWakeman,
                _ => AsianMethod::Curran,
            };
            let terms = AsianTerms::fresh_discrete(
                cp(v.term_str("option_type")),
                v.term_f64("strike"),
                obs,
            )
            .method(method);
            InstrumentSpec::asian_option(pair, tenor, t, qty, side, terms)
        }
        "forward_start" => {
            let terms = ForwardStartTerms::new(
                cp(v.term_str("option_type")),
                v.term_f64("moneyness"),
                v.term_f64("reset"),
            );
            InstrumentSpec::forward_start(pair, tenor, t, qty, side, terms)
        }
        "cliquet" => {
            let mut terms = CliquetTerms::plain(
                cp(v.term_str("option_type")),
                v.term_f64("moneyness"),
                v.term_u64("periods") as u32,
            );
            let lf = v.term_opt_f64("local_floor");
            let lc = v.term_opt_f64("local_cap");
            let gf = v.term_opt_f64("global_floor");
            let gc = v.term_opt_f64("global_cap");
            if lf.is_some() || lc.is_some() {
                terms = terms.local(lf, lc);
            }
            if gf.is_some() || gc.is_some() {
                terms = terms.global(gf, gc);
            }
            InstrumentSpec::cliquet(pair, tenor, t, qty, side, terms)
        }
        "quanto" => {
            let payoff = match v.term_str("payoff") {
                "DIGITAL" => QuantoPayoff::Digital,
                _ => QuantoPayoff::Vanilla,
            };
            let terms = QuantoTerms::new(
                payoff,
                cp(v.term_str("option_type")),
                v.term_f64("strike"),
                v.term_f64("conversion_vol"),
                v.term_f64("correlation"),
            );
            InstrumentSpec::quanto(pair, tenor, t, qty, side, terms)
        }
        "tarf" => {
            let redemption = match v.term_str("redemption") {
                "CAPPED_GAIN" => TarfRedemption::CappedGain,
                _ => TarfRedemption::FullGain,
            };
            let terms = TarfTerms::new(
                cp(v.term_str("option_type")),
                v.term_f64("strike"),
                v.term_f64("target"),
                v.term_f64("leverage"),
                redemption,
                v.term_u64("fixings") as u32,
            )
            .fixing_notional(v.term_f64("fixing_notional"));
            InstrumentSpec::tarf(pair, tenor, t, qty, side, terms)
        }
        "accumulator" => {
            let monitoring = match v.term_str("monitoring") {
                "CONTINUOUS" => AccumulatorMonitoring::Continuous,
                _ => AccumulatorMonitoring::Discrete,
            };
            let terms = AccumulatorTerms::new(
                v.term_f64("pivot"),
                v.term_f64("barrier"),
                v.term_f64("leverage"),
                monitoring,
                v.term_u64("fixings") as u32,
            )
            .fixing_notional(v.term_f64("fixing_notional"));
            InstrumentSpec::accumulator(pair, tenor, t, qty, side, terms)
        }
        "lookback" => {
            let style = match v.term_str("style") {
                "FIXED" => LookbackStyle::Fixed,
                _ => LookbackStyle::Floating,
            };
            let option = cp(v.term_str("option_type"));
            let strike = v.term_f64("strike");
            let terms = match v.term_str("monitoring") {
                "CONTINUOUS" => celnet_client::LookbackTerms::continuous(style, option, strike),
                _ => celnet_client::LookbackTerms::discrete(
                    style,
                    option,
                    strike,
                    v.term_u64("observations") as u32,
                ),
            };
            InstrumentSpec::lookback(pair, tenor, t, qty, side, terms)
        }
        "window_barrier" => {
            let bside = match v.term_str("side") {
                "UPPER" => BarrierSide::Up,
                _ => BarrierSide::Down,
            };
            InstrumentSpec::window_barrier(
                pair,
                tenor,
                t,
                qty,
                side,
                cp(v.term_str("option_type")),
                v.term_f64("strike"),
                v.term_f64("barrier"),
                bside,
                v.term_f64("window_start"),
                v.term_f64("window_end"),
                v.term_u64("mc_pairs") as u32,
                v.term_u64("mc_steps") as u32,
                v.term_u64("mc_seed"),
            )
        }
        "american" => {
            let option = cp(v.term_str("option_type"));
            let strike = v.term_f64("strike");
            let terms = AmericanTerms::american(option, strike);
            InstrumentSpec::american(pair, tenor, t, qty, side, terms)
        }
        "basket" => {
            let legs: Vec<BasketLegTerms> = v
                .terms
                .get("legs")
                .and_then(|l| l.as_array())
                .unwrap()
                .iter()
                .map(|leg| {
                    BasketLegTerms::new(
                        CcyPair::parse(leg.get("pair").unwrap().as_str().unwrap()).unwrap(),
                        leg.get("weight").unwrap().as_f64().unwrap(),
                        leg.get("spot").unwrap().as_f64().unwrap(),
                        leg.get("vol").unwrap().as_f64().unwrap(),
                        leg.get("r_for").unwrap().as_f64().unwrap(),
                    )
                })
                .collect();
            let corr: Vec<f64> = v
                .terms
                .get("correlations")
                .unwrap()
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_f64().unwrap())
                .collect();
            let kind = match v.term_str("kind") {
                "BEST_OF" => BasketKind::BestOf,
                "WORST_OF" => BasketKind::WorstOf,
                _ => BasketKind::Basket,
            };
            let terms = BasketTerms::new(
                legs,
                corr,
                kind,
                cp(v.term_str("option_type")),
                v.term_f64("strike"),
            );
            InstrumentSpec::basket(pair, tenor, t, qty, side, terms)
        }
        "fx_forward" => {
            let terms = ForwardTerms::new(
                v.term_f64("contract_rate"),
                v.term_f64("notional"),
                forward_side(v.term_str("side")),
            );
            InstrumentSpec::fx_forward(pair, tenor, t, qty, terms)
        }
        "fx_swap" => {
            let near = ForwardTerms::new(
                v.term_f64("contract_rate"),
                v.term_f64("notional"),
                forward_side(v.term_str("near_side")),
            );
            InstrumentSpec::fx_swap(pair, tenor, t, qty, SwapTerms::new(near))
        }
        "ndf" => {
            let terms = NdfTerms::new(
                v.term_f64("contract_rate"),
                v.term_f64("notional"),
                forward_side(v.term_str("side")),
                fixing_source(v.term_str("fixing")),
                v.term_str("settlement_ccy"),
            );
            InstrumentSpec::ndf(pair, tenor, t, qty, terms)
        }
        other => panic!("conformance: unhandled family `{other}`"),
    }
}

/// The directional side a linear product's vector terms carry.
fn forward_side(token: &str) -> ForwardSide {
    match token {
        "BUY" => ForwardSide::Buy,
        "SELL" => ForwardSide::Sell,
        other => panic!("unknown forward side `{other}`"),
    }
}

/// The published fixing identity an NDF vector names. The vector strings match the
/// `celnet_types::FixingSource` variant names.
fn fixing_source(token: &str) -> FixingSource {
    match token {
        "KrwKftc18" => FixingSource::KrwKftc18,
        "TwdTaipei" => FixingSource::TwdTaipei,
        "InrRbiRef" => FixingSource::InrRbiRef,
        "BrlPtax" => FixingSource::BrlPtax,
        "ClpDolarObs" => FixingSource::ClpDolarObs,
        "CopTrm" => FixingSource::CopTrm,
        other => panic!("unknown fixing source `{other}`"),
    }
}

/// Assert one priced line against a vector's expectation. Closed-form vectors use
/// the `(rel, abs)` tolerance; Monte-Carlo vectors (any vector carrying a frozen
/// `price_std_error`) use the `k · stderr` band combining the frozen oracle stderr
/// with the server's own reported stderr where present.
fn assert_conforms(v: &GoldenVector, line: &PricedLine) {
    let got = line.greeks.price;
    let want = v.expected.price;

    // HONEST BOUNDARY (window_barrier): no closed form, LSV-only on the server, so
    // the corpus carries a flat-GBM *sanity reference* rather than the LSV price.
    // Gate STRUCTURAL invariants against an oracle that never ran LSV: the price is
    // finite, non-negative, strictly below the unbarriered vanilla (a knock-out can
    // only destroy value), and within the documented wide model band of the
    // flat-GBM reference (catching a gross regression without faking an LSV oracle).
    if v.family == "window_barrier" {
        let vanilla = v.term_f64("unbarriered_vanilla");
        assert!(got.is_finite(), "window_barrier {} price not finite", v.id);
        assert!(
            got >= -1e-9,
            "window_barrier {} price must be non-negative: {got}",
            v.id
        );
        assert!(
            got <= vanilla + 1e-9,
            "window_barrier {} ({got}) must be ≤ its unbarriered vanilla ({vanilla})",
            v.id
        );
        let scale = got.abs().max(want.abs());
        assert!(
            (got - want).abs() <= v.tolerance.abs + v.tolerance.rel * scale,
            "window_barrier {} : LSV price {got} outside the wide flat-GBM model band of {want} \
             (rel {}, abs {})",
            v.id,
            v.tolerance.rel,
            v.tolerance.abs
        );
        return;
    }

    if let Some(oracle_se) = v.expected.price_std_error {
        let server_se = line.price_std_error.unwrap_or(0.0);
        let band = K_STDERR * (oracle_se + server_se).max(1e-12);
        let diff = (got - want).abs();
        assert!(
            diff <= band,
            "MC vector {} : SDK price {got} vs oracle {want} |Δ|={diff:e} > band {band:e} \
             (oracle_se={oracle_se:e}, server_se={server_se:e})",
            v.id
        );
    } else {
        let scale = got.abs().max(want.abs());
        let ok = (got - want).abs() <= v.tolerance.abs + v.tolerance.rel * scale;
        assert!(
            ok,
            "vector {} : SDK price {got} vs oracle {want} (rel {}, abs {})",
            v.id, v.tolerance.rel, v.tolerance.abs
        );
        // Greeks where the oracle provides them.
        for (name, &expected) in &v.expected.greeks {
            let g = match name.as_str() {
                "delta_spot" => line.greeks.delta_spot,
                "gamma" => line.greeks.gamma,
                "vega" => line.greeks.vega,
                "theta" => line.greeks.theta,
                "rho_dom" => line.greeks.rho_dom,
                "rho_for" => line.greeks.rho_for,
                _ => continue,
            };
            let scale = g.abs().max(expected.abs());
            // Greeks ride a slightly looser absolute floor than price (FD/analytic
            // noise near-zero), but the same relative gate.
            let ok = (g - expected).abs() <= 1e-7 + v.tolerance.rel * scale.max(1.0);
            assert!(
                ok,
                "vector {} greek {name}: SDK {g} vs oracle {expected}",
                v.id
            );
        }
    }
}

/// Price every vector whose family is in `families` through the SDK against a
/// fresh live edge, asserting conformance, and that each requested family was
/// actually exercised. Split across several `#[tokio::test]`s so the heavy
/// Monte-Carlo families run in parallel under nextest and each stays inside the
/// slow-test timeout.
async fn run_conformance(families: &[&str]) {
    let want: HashSet<&str> = families.iter().copied().collect();
    let vectors = load_vectors().expect("golden corpus loads");
    let subset: Vec<&GoldenVector> = vectors
        .iter()
        .filter(|v| want.contains(v.family.as_str()))
        .collect();
    assert!(!subset.is_empty(), "no vectors for families {families:?}");

    let (edge, client) = common::start_edge_and_client().await;
    let conv = Conventions::major_default();

    let mut exercised: HashSet<String> = HashSet::new();
    for v in &subset {
        let spec = instrument_of(v);
        let market = market_of(v);
        let line = tokio::time::timeout(PRICE_DEADLINE, client.price(&spec, market, conv))
            .await
            .unwrap_or_else(|_| panic!("pricing vector {} timed out", v.id))
            .unwrap_or_else(|e| panic!("pricing vector {} failed: {e:?}", v.id));
        assert_conforms(v, &line);
        exercised.insert(v.family.clone());
    }
    for fam in families {
        assert!(
            exercised.contains(*fam),
            "family `{fam}` was never exercised by the SDK conformance run"
        );
    }
    drop(client);
    drop(edge);
}

/// Closed-form / analytic families (fast): vanilla, strategy, all barriers /
/// digital / touch, var/vol swaps, forward-start, plain cliquet, quanto,
/// continuous-form references — priced against the QuantLib CSVs / independent
/// closed forms.
#[tokio::test]
async fn sdk_conforms_closed_form_families() {
    run_conformance(&[
        "vanilla",
        "strategy",
        "single_barrier",
        "double_barrier",
        "digital",
        "touch",
        "variance_swap",
        "volatility_swap",
        "forward_start",
        "quanto",
        "american",
        "fx_forward",
        "fx_swap",
        "ndf",
    ])
    .await;
}

/// The Monte-Carlo families, each gated within `k · stderr` of a code-disjoint
/// Monte-Carlo oracle. Cliquet is mixed (plain closed-form + clamped MC); both
/// arms are covered here.
#[tokio::test]
async fn sdk_conforms_mc_asian() {
    run_conformance(&["asian_option"]).await;
}

#[tokio::test]
async fn sdk_conforms_mc_lookback() {
    run_conformance(&["lookback"]).await;
}

#[tokio::test]
async fn sdk_conforms_mc_tarf_accumulator() {
    run_conformance(&["tarf", "accumulator"]).await;
}

#[tokio::test]
async fn sdk_conforms_mc_cliquet_basket() {
    run_conformance(&["cliquet", "basket"]).await;
}

#[tokio::test]
async fn sdk_conforms_window_barrier() {
    run_conformance(&["window_barrier"]).await;
}

/// Cross-asset vanilla conformance: price an **equity**, a **commodity** and a
/// **digital-asset (crypto)** vanilla through the SDK against a live edge and
/// reconcile SDK == server == oracle.
///
/// The underlying is contract identity for the option payoff (ADR-0008's
/// asset-class-agnostic payoff over the carry-producing market): under the linear
/// (quote-margined) settlement style, a vanilla on any asset class prices by the
/// SAME generalized-BSM / Garman-Kohlhagen closed form against the request market
/// context. The oracle here is therefore the **independent** `celnet-vanilla`
/// closed form computed in-test (a code path the wire/server never touched) — not
/// a tautology against the server. We additionally assert each cross-asset price
/// matches the equivalent FX vanilla on the same market to a tight tolerance —
/// bit-identical for the equity leaf (which shares the GK arithmetic), and to 1e-10
/// for the commodity (Black-76) and crypto (funded) leaves, which compute the same
/// value via a different float path — proving the asset class travels as identity
/// only and does not perturb the linear payoff. (The FX path's own byte-identity is
/// gated separately by the W2 `to_bits` golden.)
#[tokio::test]
async fn sdk_conforms_cross_asset_vanilla() {
    use celnet_client::{Ccy, Underlying};
    use celnet_types::VanillaInputs;

    // A non-degenerate 1Y market (spot/vol/rates) shared by every asset class.
    let (spot, strike, vol, t, r_dom, r_for) = (100.0, 105.0, 0.22, 1.0, 0.03, 0.012);
    let market = MarketContext {
        spot,
        vol,
        r_dom,
        r_for,
    };
    let tenor = Tenor::Years(1);
    let qty = Quantity::base(1.0);
    let conv = Conventions::major_default();

    // The independent oracle: the generalized-BSM / GK closed form for this market.
    // (`r_for` is the asset's carry yield — dividend yield for an equity, the
    // cost-of-carry for a commodity, the funding/quote rate for a crypto pair.)
    let oracle = celnet_vanilla::greeks(
        OptionType::Call,
        &VanillaInputs::new(spot, strike, vol, t, r_dom, r_for),
    );

    let (edge, client) = common::start_edge_and_client().await;

    // The three cross-asset underlyings, each a CALL struck at 105 on the same market.
    let specs: Vec<(&str, InstrumentSpec)> = vec![
        (
            "equity",
            InstrumentSpec::equity_vanilla(
                "AAPL",
                "XNAS",
                Ccy::USD,
                tenor,
                t,
                qty,
                Side::TwoWay,
                OptionType::Call,
                StrikeSpec::Absolute(strike),
            ),
        ),
        (
            "commodity",
            InstrumentSpec::commodity_vanilla(
                "BRENT",
                "",
                Ccy::USD,
                tenor,
                t,
                qty,
                Side::TwoWay,
                OptionType::Call,
                StrikeSpec::Absolute(strike),
            ),
        ),
        (
            "crypto",
            InstrumentSpec::crypto_vanilla(
                "BTC",
                "USDT",
                tenor,
                t,
                qty,
                Side::TwoWay,
                OptionType::Call,
                StrikeSpec::Absolute(strike),
            ),
        ),
    ];

    // The FX baseline on the identical market — the byte-identity reference.
    let fx_spec = InstrumentSpec::vanilla_on(
        Underlying::Fx(CcyPair::parse("EURUSD").unwrap()),
        tenor,
        t,
        qty,
        Side::TwoWay,
        OptionType::Call,
        StrikeSpec::Absolute(strike),
    );
    let fx_line = tokio::time::timeout(PRICE_DEADLINE, client.price(&fx_spec, market, conv))
        .await
        .expect("fx vanilla price timed out")
        .expect("fx vanilla price");

    for (label, spec) in &specs {
        let line = tokio::time::timeout(PRICE_DEADLINE, client.price(spec, market, conv))
            .await
            .unwrap_or_else(|_| panic!("{label} vanilla price timed out"))
            .unwrap_or_else(|e| panic!("{label} vanilla price failed: {e:?}"));

        // SDK == oracle: the asset-class-agnostic generalized-BSM / GK closed form on
        // the shared carry-producing market. ADR-0008 — the underlying is identity for
        // the payoff, so every asset class reprices the SAME closed form to the request
        // market (`r_for` is the asset's carry yield: dividend / convenience / funding).
        assert!(
            (line.greeks.price - oracle.price).abs() <= 1e-10,
            "{label} vanilla SDK price {} vs independent oracle {}",
            line.greeks.price,
            oracle.price
        );
        // ...and therefore matches the FX baseline priced on the identical market. The
        // equity leaf shares the GK arithmetic so it is bit-identical; the commodity
        // (Black-76) and crypto (funded) leaves reach the mathematically-identical value
        // via a different float path, so they agree to a tight tolerance, not bit-for-bit.
        // (The FX path's OWN byte-identity is gated separately by the W2 `to_bits` golden.)
        assert!(
            (line.greeks.price - fx_line.greeks.price).abs() <= 1e-10,
            "{label} vanilla price {} vs FX baseline {} (asset-class-agnostic payoff)",
            line.greeks.price,
            fx_line.greeks.price
        );
        assert!(
            (line.greeks.vega - fx_line.greeks.vega).abs() <= 1e-8,
            "{label} vanilla vega {} vs FX baseline {}",
            line.greeks.vega,
            fx_line.greeks.vega
        );
    }

    drop(client);
    drop(edge);
}

/// Reachability backstop: every one of the 21 product-oneof families appears in
/// the corpus (the conformance tests above collectively price them all).
#[tokio::test]
async fn corpus_covers_all_families() {
    let vectors = load_vectors().expect("golden corpus loads");
    let present: HashSet<&str> = vectors.iter().map(|v| v.family.as_str()).collect();
    for fam in FAMILIES {
        assert!(
            present.contains(fam),
            "family `{fam}` missing from the corpus"
        );
    }
}
