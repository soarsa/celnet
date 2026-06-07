//! Shared in-process test harness for the Celnet gRPC edge.
//!
//! Every test starts an [`Edge`] on an ephemeral port over the engine's calibrated
//! EURUSD fixture, dials it with the generated `celnet-proto` clients, and asserts
//! against first-principles `celnet-vanilla` / `celnet-exotics` references. Every
//! body is wrapped in a hard wall-clock deadline and every network await is itself
//! bounded, so a regression surfaces as a fast failure, never an infinite hang.

// Each integration-test binary `mod common;`s this file and uses only a subset of
// the helpers, so per-binary some are unused (`dead_code`) and their `pub` is not
// reachable outside the binary (`unreachable_pub`). Both are expected for a shared
// test-support module and are allowed here.
#![allow(dead_code, unreachable_pub)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use celnet_engine::testing::make_state;
use celnet_server::{Clock, CoreLink, Edge, SpreadModel};
use celnet_types::{CcyPair, Tenor};

/// The hard wall-clock ceiling for any single edge integration test. A correctness
/// failure must surface as a *fast* failure, never an infinite hang.
pub const TEST_DEADLINE: Duration = Duration::from_secs(10);

/// Bound a single network / response await so a never-arriving reply fails fast.
pub const STEP_DEADLINE: Duration = Duration::from_secs(5);

/// The resolved EURUSD 1Y convention record the fixture is built on.
#[must_use]
pub fn eurusd_conv() -> celnet_conventions::ConventionRecord {
    celnet_conventions::resolve(CcyPair::parse("EURUSD").unwrap(), Tenor::Years(1)).record
}

/// Start a ready edge on an ephemeral port over the EURUSD fixture with a system
/// clock, returning the edge and its bound gRPC address.
pub async fn start_ready_edge() -> (Edge, SocketAddr) {
    start_edge_with(true, Clock::system()).await
}

/// Start an edge with an explicit ready flag and clock (so a test can drive
/// last-look expiry with a manual clock or assert not-ready rejection).
pub async fn start_edge_with(ready: bool, clock: Clock) -> (Edge, SocketAddr) {
    let initial = make_state(1.10, eurusd_conv());
    let link = CoreLink::start(initial, None);
    let grpc: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let edge = Edge::start(grpc, Arc::clone(&link), SpreadModel::default(), clock)
        .await
        .expect("edge binds on an ephemeral port");
    if ready {
        edge.gate().mark_ready();
    }
    let addr = edge.grpc_addr();
    (edge, addr)
}

/// The wire conventions used across the tests (EURUSD spot-premium-adjusted /
/// delta-neutral-straddle / foreign-percent).
#[must_use]
pub fn wire_conventions() -> celnet_proto::Conventions {
    celnet_proto::Conventions {
        delta_convention: celnet_proto::DeltaConvention::SpotUnadjusted as i32,
        atm_convention: celnet_proto::AtmConvention::AtmForward as i32,
        premium_style: celnet_proto::PremiumStyle::DomesticPips as i32,
        cut: celnet_proto::Cut::NewYork1000 as i32,
        day_count: celnet_proto::DayCount::Act365Fixed as i32,
        settlement: celnet_proto::Settlement::Deliverable as i32,
    }
}

/// A EUR/USD currency pair on the wire.
#[must_use]
pub fn eurusd_pair() -> celnet_proto::CcyPair {
    celnet_proto::CcyPair {
        base: "EUR".to_owned(),
        quote: "USD".to_owned(),
    }
}

/// A vanilla-call instrument at an absolute strike with a 1Y expiry.
#[must_use]
pub fn vanilla_call(strike: f64) -> celnet_proto::Instrument {
    celnet_proto::Instrument {
        pair: Some(eurusd_pair()),
        tenor: Some(celnet_proto::Tenor {
            unit: celnet_proto::tenor::Unit::Years as i32,
            count: 1,
            broken_date: None,
        }),
        expiry_years: 1.0,
        quantity: Some(celnet_proto::Quantity {
            notional: 1_000_000.0,
            base_ccy: true,
        }),
        side: celnet_proto::Side::TwoWay as i32,
        solve: None,
        pricing_model: celnet_proto::PricingModel::Default as i32,
        product: Some(celnet_proto::instrument::Product::Vanilla(
            celnet_proto::Vanilla {
                option_type: celnet_proto::OptionType::Call as i32,
                strike: Some(celnet_proto::StrikeOrDelta {
                    spec: Some(celnet_proto::strike_or_delta::Spec::Strike(strike)),
                }),
            },
        )),
    }
}

/// The live market context the edge prices an RFQ / RFS against: the EURUSD
/// fixture's spot, ATM vol (at the forward), and rates. Tests mirror this to form
/// a first-principles reference.
#[must_use]
pub fn live_market() -> celnet_proto::MarketContext {
    let state = make_state(1.10, eurusd_conv());
    let forward = state.forward();
    let atm_vol = celnet_core::Smile::implied_vol(&state.smile, forward, forward, state.t).0;
    celnet_proto::MarketContext {
        spot: state.spot,
        vol: atm_vol,
        r_dom: state.r_dom,
        r_for: state.r_for,
    }
}
