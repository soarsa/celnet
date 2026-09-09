//! Server Integration Test: Nonlinear Rates Exotics and Credit Copulas.
//!
//! Validates pricing of Cheyette swaptions and CDO synthetic credit tranches
//! through server valuation pipelines.

use celnet_rates_exotics::cheyette::{Cheyette1FParams, Cheyette2FParams, CheyettePricer, SwaptionSpec};
use celnet_rates_exotics::credit_copula::{CdoTranche, CreditCopulaEngine, CreditObligor};

#[test]
fn test_server_cheyette_swaption_valuation() {
    let spec = SwaptionSpec {
        expiry_years: 1.0,
        swap_tenor_years: 5.0,
        strike_rate: 0.038,
        notional: 50_000_000.0, // $50M
        is_payer: true,
    };

    let params_1f = Cheyette1FParams::default();
    let pv_1f = CheyettePricer::price_swaption_1f(&spec, &params_1f, 0.040, 4.60).expect("price 1f");
    assert!(pv_1f > 0.0);

    let params_2f = Cheyette2FParams::default();
    let pv_2f = CheyettePricer::price_swaption_2f(&spec, &params_2f, 0.040, 4.60).expect("price 2f");
    assert!(pv_2f > 0.0);

    println!("Server Valuation: 1F Swaption PV=${:.2}, 2F Swaption PV=${:.2}", pv_1f, pv_2f);
}

#[test]
fn test_server_credit_portfolio_tranche_valuation() {
    let obligors = vec![
        CreditObligor {
            name: "US-IG-1".into(),
            recovery_rate: 0.40,
            hazard_rate: 0.010, // 100 bps spread
            notional: 25_000_000.0,
            factor_loading: 0.45,
        },
        CreditObligor {
            name: "US-IG-2".into(),
            recovery_rate: 0.40,
            hazard_rate: 0.012,
            notional: 25_000_000.0,
            factor_loading: 0.45,
        },
        CreditObligor {
            name: "US-IG-3".into(),
            recovery_rate: 0.40,
            hazard_rate: 0.018,
            notional: 25_000_000.0,
            factor_loading: 0.45,
        },
        CreditObligor {
            name: "US-IG-4".into(),
            recovery_rate: 0.40,
            hazard_rate: 0.022,
            notional: 25_000_000.0,
            factor_loading: 0.45,
        },
    ];

    let tranche_equity = CdoTranche {
        attachment: 0.00,
        detachment: 0.03, // 0-3% equity
        maturity_years: 5.0,
        portfolio_notional: 100_000_000.0,
    };

    let tranche_senior = CdoTranche {
        attachment: 0.07,
        detachment: 0.15, // 7-15% senior
        maturity_years: 5.0,
        portfolio_notional: 100_000_000.0,
    };

    let res_eq = CreditCopulaEngine::price_cdo_tranche(&tranche_equity, &obligors, 0.035).unwrap();
    let res_sr = CreditCopulaEngine::price_cdo_tranche(&tranche_senior, &obligors, 0.035).unwrap();

    assert!(res_eq.fair_spread_bps > res_sr.fair_spread_bps);
    println!(
        "Server Credit Portfolio Valuation: Equity Tranche={:.1} bps, Senior Tranche={:.1} bps",
        res_eq.fair_spread_bps, res_sr.fair_spread_bps
    );
}
