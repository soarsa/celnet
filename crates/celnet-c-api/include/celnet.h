/*
 * Celnet C-ABI Interface (celnet.h)
 *
 * High-Performance Cross-Asset Valuation, Clearing Initial Margin,
 * Algorithmic Execution, Distributed Raft Cluster, and Enterprise Governance C API.
 *
 * Applicable for:
 *   - Native C / C++ quantitative trading systems
 *   - C# (.NET) via [DllImport("celnet_c_api")]
 *   - Python via ctypes or cffi
 *   - Excel Add-ins via C/C++ XLL (xloper/xloper12)
 */

#ifndef CELNET_H
#define CELNET_H

#include <stdint.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

/* -------------------------------------------------------------------------
 * Version & Telemetry
 * ------------------------------------------------------------------------- */

/** Return Celnet C-API semantic version code (e.g. 20260901). */
uint32_t celnet_c_api_version(void);

/* -------------------------------------------------------------------------
 * Valuation & Greeks (Analytic Garman-Kohlhagen / Black-Scholes)
 * ------------------------------------------------------------------------- */

/** Full 14-Greek sensitivity set calculated analytically. */
typedef struct {
    double price;
    double delta_spot;
    double delta_fwd;
    double gamma;
    double vega;
    double theta;
    double rho_dom;
    double rho_for;
    double vanna;
    double volga;
    double charm;
    double speed;
    double zomma;
    double color;
} CelnetGreeksC;

/**
 * Price European vanilla option with full 14-Greek sensitivity set.
 *
 * @param spot Underlier spot exchange rate
 * @param strike Contract strike price
 * @param expiry_years Time to expiry in fractional years (ACT/365F)
 * @param vol Annualized volatility
 * @param r_dom Domestic risk-free rate
 * @param r_for Foreign risk-free rate
 * @param is_call True for Call, False for Put
 */
CelnetGreeksC celnet_price_vanilla(
    double spot,
    double strike,
    double expiry_years,
    double vol,
    double r_dom,
    double r_for,
    bool is_call
);

/**
 * Price single-barrier knock-out option.
 *
 * @param spot Underlier spot price
 * @param strike Option strike
 * @param barrier Knock-out barrier level
 * @param expiry_years Time to maturity in years
 * @param vol Volatility
 * @param r_dom Domestic rate
 * @param r_for Foreign rate
 * @param is_call True for Call, False for Put
 * @param is_down True for Down-and-out, False for Up-and-out
 */
CelnetGreeksC celnet_price_barrier(
    double spot,
    double strike,
    double barrier,
    double expiry_years,
    double vol,
    double r_dom,
    double r_for,
    bool is_call,
    bool is_down
);

/**
 * Price European digital / binary cash-or-nothing option.
 *
 * @param spot Underlier spot price
 * @param strike Option strike price
 * @param expiry_years Time to expiry in fractional years
 * @param vol Annualized volatility
 * @param r_dom Domestic risk-free rate
 * @param r_for Foreign risk-free rate
 * @param payout Fixed cash payout if exercised in-the-money
 * @param is_call True for Digital Call, False for Digital Put
 */
CelnetGreeksC celnet_price_digital(
    double spot,
    double strike,
    double expiry_years,
    double vol,
    double r_dom,
    double r_for,
    double payout,
    bool is_call
);

/**
 * Path signature rough volatility implied surface point.
 */
typedef struct {
    double atm_vol;
    double atm_skew;
    double atm_curvature;
    double implied_vol;
} CelnetSignatureVolPointC;

/**
 * Price rough volatility smile using 2025/2026 path signature Markovian lift.
 *
 * @param spot Spot price
 * @param strike Strike price
 * @param tenor_years Tenor in years
 * @param hurst Hurst exponent H in (0.0, 0.5)
 * @param spot_vol Base spot volatility
 * @param vol_of_vol Volatility of volatility
 * @param rho Spot-vol correlation
 * @param theta Long-term mean reversion variance level
 */
CelnetSignatureVolPointC celnet_price_signature_vol(
    double spot,
    double strike,
    double tenor_years,
    double hurst,
    double spot_vol,
    double vol_of_vol,
    double rho,
    double theta
);

/* -------------------------------------------------------------------------
 * Rates & Fixed Income (SOFR OIS Swap, IRS, FRA, and Cash Bond Valuation)
 * ------------------------------------------------------------------------- */

typedef struct {
    double pv;
    double par_rate;
    double pv01;
    double dv01;
} CelnetRatesPricingC;

/**
 * Price a vanilla Overnight Index Swap (OIS) under SOFR discounting.
 */
CelnetRatesPricingC celnet_price_rates_ois(
    double fixed_rate,
    double tenor_years,
    double notional,
    double discount_rate,
    bool is_payer
);

/**
 * Price a standard fixed-for-floating Interest Rate Swap (IRS).
 */
CelnetRatesPricingC celnet_price_rates_irs(
    double fixed_rate,
    double floating_forward,
    double tenor_years,
    uint32_t payment_freq_per_year,
    double notional,
    double discount_rate,
    bool is_payer
);

/**
 * Price a Forward Rate Agreement (FRA).
 */
CelnetRatesPricingC celnet_price_rates_fra(
    double agreed_rate,
    double forward_rate,
    double start_time_years,
    double end_time_years,
    double notional,
    double discount_rate,
    bool is_payer
);

/**
 * Sovereign/Corporate fixed coupon bond pricing metrics.
 */
typedef struct {
    double clean_price;
    double dirty_price;
    double accrued_interest;
    double modified_duration;
    double macaulay_duration;
    double convexity;
    double dv01;
} CelnetBondPricingC;

/**
 * Price a fixed-rate sovereign/corporate coupon bond.
 */
CelnetBondPricingC celnet_price_bond(
    double coupon_rate,
    double yield_to_maturity,
    double tenor_years,
    uint32_t frequency,
    double fraction_elapsed
);

/* -------------------------------------------------------------------------
 * Clearing Initial Margin & Pre-Trade What-If
 * ------------------------------------------------------------------------- */

typedef struct {
    double total_initial_margin;
    double expected_shortfall;
    double value_at_risk;
    double stress_component;
    uint64_t calculated_epoch_nanos;
} CelnetMarginC;

/**
 * Calculate clearing initial margin under ISDA SIMM 2.7 / Expected Shortfall 97.5%.
 */
CelnetMarginC celnet_calculate_margin(
    double net_notional,
    double confidence_level,
    uint32_t lookback_days
);

typedef struct {
    bool is_approved;
    double initial_margin_before;
    double initial_margin_after;
    double delta_margin;
    double collateral_headroom;
} CelnetPreTradeMarginC;

/**
 * Simulate incremental pre-trade margin impact and evaluate account collateral headroom.
 */
CelnetPreTradeMarginC celnet_simulate_pre_trade_margin(
    double trade_notional,
    double available_collateral,
    double existing_initial_margin
);

/* -------------------------------------------------------------------------
 * Algorithmic Execution (TWAP & Optimal Liquidation)
 * ------------------------------------------------------------------------- */

typedef struct {
    double total_quantity;
    double executed_quantity;
    double arrival_price;
    double avg_exec_price;
    double implementation_shortfall_bps;
    uint32_t slice_count;
    bool is_active;
} CelnetAlgoScheduleC;

/**
 * Plan TWAP algorithmic execution schedule.
 */
CelnetAlgoScheduleC celnet_plan_twap_algo(
    double total_quantity,
    double arrival_price,
    uint32_t duration_seconds,
    uint32_t slice_count,
    bool is_buy
);

/**
 * Plan transient propagator execution schedule with real-time order book imbalance conditioning (August 2026 literature).
 *
 * @param total_quantity Total parent order size
 * @param arrival_price Initial market arrival mid-price
 * @param horizon_seconds Total execution window
 * @param slice_count Discretization intervals
 * @param observed_obi Real-time order book imbalance in [-1, 1]
 * @param decay_half_life_seconds Half-life of transient market impact
 * @param impact_scale_eta Linear impact scaling factor
 */
CelnetAlgoScheduleC celnet_plan_propagator_algo(
    double total_quantity,
    double arrival_price,
    uint32_t horizon_seconds,
    uint32_t slice_count,
    double observed_obi,
    double decay_half_life_seconds,
    double impact_scale_eta
);

/* -------------------------------------------------------------------------
 * Distributed Raft Cluster & Zero-Downtime Rolling Upgrade
 * ------------------------------------------------------------------------- */

typedef struct {
    uint64_t active_generation;
    uint32_t active_nodes;
    uint32_t leader_node_id;
    bool is_consensus_healthy;
} CelnetClusterHealthC;

/**
 * Inspect Raft cluster topology health.
 */
CelnetClusterHealthC celnet_check_cluster_health(
    uint32_t node_count,
    uint64_t active_generation
);

typedef struct {
    uint64_t max_ulp_divergence;
    bool is_bit_exact_pass;
    uint64_t evaluated_trades;
} CelnetUpgradeVerificationC;

/**
 * Verify shadow twin numerical equivalence (0 ULP divergence threshold).
 */
CelnetUpgradeVerificationC celnet_verify_shadow_twin_ulp(
    uint64_t shadow_ulp_max,
    uint64_t evaluated_trades
);

/* -------------------------------------------------------------------------
 * Enterprise Governance (Hardware Attestation & Dynamic Capability Licensing)
 * ------------------------------------------------------------------------- */

typedef struct {
    bool is_valid;
    bool hardware_pcr_match;
    uint64_t timestamp_epoch_nanos;
} CelnetAttestationResultC;

/**
 * Verify TPM 2.0 PCR hardware enclave cryptographic quote.
 */
CelnetAttestationResultC celnet_verify_hardware_attestation(
    uint64_t pcr_register_val,
    uint64_t expected_pcr
);

typedef struct {
    bool is_valid;
    uint32_t tier_code;
    uint64_t expiry_epoch_secs;
    uint64_t capability_mask;
} CelnetLicenseStatusC;

/**
 * Inspect dynamic institutional capability token license.
 */
CelnetLicenseStatusC celnet_verify_capability_token_license(
    uint32_t tier_code,
    uint64_t expiry_epoch_secs
);

/**
 * Legacy export alias for dynamic capability token license inspection.
 */
CelnetLicenseStatusC celnet_verify_biscuit_license_caps(
    uint32_t tier_code,
    uint64_t expiry_epoch_secs
);

#ifdef __cplusplus
}
#endif

#endif /* CELNET_H */
