//! Celnet executable competitive-parity matrix — the claims in
//! `docs/CAPABILITIES-VS-COMPETITION.md` rendered as gated tests, so "meets or
//! beats SynOption / Fenics / Bloomberg" is continuously *proven*, not asserted.
//!
//! Each capability row maps to a test that reprices a reference input, checks a
//! convention/arbitrage invariant, or demonstrates a feature the incumbents lack
//! or treat as opaque internal machinery. Competitor names appear **only** in
//! test-data and doc-comment context here (the binding-rule carve-out for
//! parity), never in any product API identifier — this crate exports no public
//! pricing surface, it is a verification harness.
//!
//! The tests live in `tests/` (integration targets) so they exercise the crates
//! exactly as a downstream consumer would, through their public APIs.
//!
//! # Capability → gated-test map
//!
//! | # | Capability claim (vs incumbents) | Gated test |
//! |---|----------------------------------|------------|
//! | 1 | Vanilla price = closed form / textbook (vs OVML/kACE closed pricers) | `conventions::vanilla_price_matches_closed_form` |
//! | 2 | Convention transparency: four delta conventions + ATM/DNS, strike↔delta round-trip (vs convention-opaque terminals) | `conventions::strike_delta_roundtrip_all_conventions`, `conventions::atm_dns_strike_is_delta_neutral` |
//! | 3 | Premium-adjusted call-delta guard (the non-monotone primitive competitors hide) | `conventions::premium_adjusted_call_delta_is_guarded` |
//! | 4 | Full 13-Greek finite-difference agreement (vs FD-unvalidated closed Greeks) | `greeks::full_greek_set_matches_finite_difference`, `greeks::second_order_wing_greeks_match_fd` |
//! | 5 | Put-call parity holds across regimes (correctness floor) | `greeks::put_call_parity_across_regimes` |
//! | 6 | Arbitrage-free surface: butterfly density ≥ 0 (vs ML-fill / heuristic surfaces with no guarantee) | `surface::butterfly_density_nonnegative` |
//! | 7 | Arbitrage-free surface: vertical (call) monotonicity in strike | `surface::vertical_monotone_in_strike` |
//! | 8 | Arbitrage-free surface: monotone total variance in business time (calendar) | `surface::calendar_total_variance_monotone` |
//! | 9 | VV vs SSVI wing agreement + each of VV/SABR/SVI/SSVI independently selectable & usable (user-selectable smile family) | `surface::vannavolga_and_ssvi_agree_in_wings`, `surface::each_smile_family_is_selectable` |
//! | 10 | Broker→smile reprices the market strangle exactly (the documented #1 production bug) | `broker_smile::smile_reprices_broker_strangle` |
//! | 11 | Broker→smile on a high-RR EM case (where the naive arithmetic butterfly mismarks) | `broker_smile::high_rr_em_case_reprices` |
//! | 12 | First-gen exotics: European digitals match QuantLib (vs closed kACE/OVML) | `exotics::digitals_match_quantlib` |
//! | 13 | First-gen exotics: one-touch / no-touch / DNT / double-touch match QuantLib | `exotics::touches_and_dnt_match_quantlib` |
//! | 14 | First-gen exotics: all eight single-barrier flavours + double-KO match QuantLib | `exotics::barriers_match_quantlib`, `exotics::double_barriers_match_quantlib` |
//! | 15 | Determinism: identical inputs → bit-identical price and full Greek set | `determinism::price_and_greeks_are_bit_identical`, `determinism::smile_and_exotics_are_bit_identical` |
//! | 16 | Second-gen exotics: **quanto** vanilla + digital closed forms cross-validated by MC, zero-correlation collapse to plain price (the product class SynOption/Fenics price opaquely) | `structured::quanto_closed_form_matches_mc_and_collapses_at_zero_correlation` |
//! | 17 | Second-gen exotics: **lookback** (floating/fixed) closed forms cross-validated by MC + lookback-dominates-vanilla invariant | `structured::lookback_closed_form_matches_mc_and_dominates_vanilla` |
//! | 18 | Structured: **TARF** gap-risk decomposition — FullGain (overshoot kept) strictly costlier to the bank than CappedGain; explicit, signed gap premium (the structured book SynOption monetises) | `structured::tarf_gap_risk_premium_is_priced_and_signed` |
//! | 19 | Structured: **accumulator** continuous (Brownian-bridge) monitoring knocks out more than discrete — model-free knock-out correctness | `structured::accumulator_continuous_monitoring_knocks_out_more_than_discrete` |
//!
//! Every row above is gated: a regression makes `cargo nextest run -p
//! celnet-parity` fail, so the matrix cannot silently rot out of sync with the
//! capabilities document (zero-legacy invariant). Rows 16–19 promote the
//! second-generation / structured-product book (quanto, lookback, TARF,
//! accumulator) from "deferred" to **delivered & gated** — each validated against
//! an independent route (closed form ⇄ Monte-Carlo) or a model-free financial
//! invariant, in the open.
//!
//! ## Property-based coverage
//!
//! The three static arbitrage laws (butterfly, vertical, calendar) are gated as
//! **property tests** (`proptest`) sweeping randomized arbitrage-free smile
//! parameters, not single fixtures — so the guarantee is proven over a domain,
//! the way an IPV/FRTB auditor would demand, not asserted on a happy path.

#![forbid(unsafe_code)]
