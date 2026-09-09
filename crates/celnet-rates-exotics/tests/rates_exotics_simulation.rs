//! Unit and validation tests for Cheyette rates, SABR-LMM, and Credit Copulas.

use celnet_rates_exotics::cheyette::{Cheyette1FParams, Cheyette2FParams, CheyettePricer, SwaptionSpec};
use celnet_rates_exotics::credit_copula::{CdoTranche, CreditCopulaEngine, CreditObligor};
use celnet_rates_exotics::sabr_lmm::{SabrModel, SabrParams};

#[test]
fn test_cheyette_1f_swaption_pricing() {
    let spec = SwaptionSpec {
        expiry_years: 1.0,      // 1Y
        swap_tenor_years: 5.0,  // 5Y
        strike_rate: 0.035,     // 3.5%
        notional: 10_000_000.0, // $10M
        is_payer: true,
    };

    let params = Cheyette1FParams::default();
    let forward_swap_rate = 0.035; // ATM
    let annuity = 4.65;            // 5Y swap annuity

    let pv = CheyettePricer::price_swaption_1f(&spec, &params, forward_swap_rate, annuity)
        .expect("price 1f swaption");

    assert!(pv > 0.0);
    // At 120 bps vol on $10M x 4.65 annuity, ATM swaption is ~$218,000
    assert!(pv > 150_000.0 && pv < 300_000.0);
    println!("Cheyette 1F Swaption PV: ${:.2}", pv);
}

#[test]
fn test_cheyette_2f_swaption_pricing() {
    let spec = SwaptionSpec {
        expiry_years: 2.0,      // 2Y
        swap_tenor_years: 10.0, // 10Y
        strike_rate: 0.040,
        notional: 25_000_000.0, // $25M
        is_payer: true,
    };

    let params = Cheyette2FParams::default();
    let forward_swap_rate = 0.042; // In the money
    let annuity = 8.25;

    let pv = CheyettePricer::price_swaption_2f(&spec, &params, forward_swap_rate, annuity)
        .expect("price 2f swaption");

    assert!(pv > 0.0);
    println!("Cheyette 2F Swaption PV: ${:.2}", pv);
}

#[test]
fn test_sabr_volatility_smile_and_correlation() {
    let params = SabrParams::default();
    let forward = 0.035;
    let expiry = 1.0;

    let atm_vol = SabrModel::implied_volatility(forward, forward, expiry, &params).unwrap();
    let otm_put_vol = SabrModel::implied_volatility(forward, 0.025, expiry, &params).unwrap();
    let otm_call_vol = SabrModel::implied_volatility(forward, 0.045, expiry, &params).unwrap();

    // With negative rho (-0.25), OTM put has higher vol than OTM call (skew)
    assert!(otm_put_vol > otm_call_vol);
    assert!(atm_vol > 0.0);
    println!(
        "SABR Vol Smile: OTM Put={:.4}, ATM={:.4}, OTM Call={:.4}",
        otm_put_vol, atm_vol, otm_call_vol
    );

    // Cross-tenor correlation
    let corr_2y_5y = SabrModel::cross_tenor_correlation(2.0, 5.0, 0.08);
    let corr_2y_10y = SabrModel::cross_tenor_correlation(2.0, 10.0, 0.08);
    assert!(corr_2y_5y > corr_2y_10y);
    assert!(corr_2y_10y > 0.0);
}

#[test]
fn test_cdo_tranche_pricing_equity_vs_senior() {
    // 5-name synthetic portfolio
    let obligors = vec![
        CreditObligor {
            name: "NAME_A".into(),
            recovery_rate: 0.40,
            hazard_rate: 0.015,
            notional: 20_000_000.0,
            factor_loading: 0.50,
        },
        CreditObligor {
            name: "NAME_B".into(),
            recovery_rate: 0.40,
            hazard_rate: 0.020,
            notional: 20_000_000.0,
            factor_loading: 0.50,
        },
        CreditObligor {
            name: "NAME_C".into(),
            recovery_rate: 0.40,
            hazard_rate: 0.012,
            notional: 20_000_000.0,
            factor_loading: 0.50,
        },
        CreditObligor {
            name: "NAME_D".into(),
            recovery_rate: 0.40,
            hazard_rate: 0.018,
            notional: 20_000_000.0,
            factor_loading: 0.50,
        },
        CreditObligor {
            name: "NAME_E".into(),
            recovery_rate: 0.40,
            hazard_rate: 0.025,
            notional: 20_000_000.0,
            factor_loading: 0.50,
        },
    ];

    let total_notional = 100_000_000.0; // $100M

    // Equity tranche [0%, 3%]
    let equity_tranche = CdoTranche {
        attachment: 0.00,
        detachment: 0.03,
        maturity_years: 5.0,
        portfolio_notional: total_notional,
    };

    // Mezzanine tranche [3%, 7%]
    let mezz_tranche = CdoTranche {
        attachment: 0.03,
        detachment: 0.07,
        maturity_years: 5.0,
        portfolio_notional: total_notional,
    };

    let equity_res = CreditCopulaEngine::price_cdo_tranche(&equity_tranche, &obligors, 0.03).unwrap();
    let mezz_res = CreditCopulaEngine::price_cdo_tranche(&mezz_tranche, &obligors, 0.03).unwrap();

    // Equity tranche absorbs first loss, so its fair spread is much higher than Mezzanine
    assert!(equity_res.fair_spread_bps > mezz_res.fair_spread_bps);
    assert!(equity_res.expected_loss > mezz_res.expected_loss);
    println!(
        "CDO Tranche Spreads: Equity [0-3%]: {:.1} bps, Mezz [3-7%]: {:.1} bps",
        equity_res.fair_spread_bps, mezz_res.fair_spread_bps
    );
}

#[test]
fn test_first_to_default_basket() {
    let obligors = vec![
        CreditObligor {
            name: "AAA".into(),
            recovery_rate: 0.40,
            hazard_rate: 0.008,
            notional: 10_000_000.0,
            factor_loading: 0.40,
        },
        CreditObligor {
            name: "BBB".into(),
            recovery_rate: 0.40,
            hazard_rate: 0.015,
            notional: 10_000_000.0,
            factor_loading: 0.40,
        },
        CreditObligor {
            name: "CCC".into(),
            recovery_rate: 0.40,
            hazard_rate: 0.022,
            notional: 10_000_000.0,
            factor_loading: 0.40,
        },
    ];

    let ftd_pv = CreditCopulaEngine::price_first_to_default_basket(&obligors, 3.0, 0.025).unwrap();
    assert!(ftd_pv > 0.0);
    println!("First-to-Default Basket PV: ${:.2}", ftd_pv);
}
