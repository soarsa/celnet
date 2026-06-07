//! SDK quickstart — price an exotic via the typed vocabulary builders.
//!
//! One-shot-prices two of the catalogue products the SDK exposes through ergonomic
//! [`InstrumentSpec`] builders, against a running edge:
//!
//! * a fixed-strike **arithmetic-average-rate Asian** call (priced in closed form by
//!   the analytic Curran estimator — exact, so no Monte-Carlo standard error), and
//! * an **American** (continuous-exercise) put (priced by the finite-difference
//!   free-boundary engine), shown alongside the European reference so the early-
//!   exercise premium is visible.
//!
//! Both are built purely from the SDK's typed domain vocabulary — no raw proto — and
//! priced over the real edge, surfacing the maker's deterministic `celnet-core`
//! numbers verbatim.
//!
//! # Run it
//!
//! In one terminal, boot a seeded demo edge (binds gRPC on `127.0.0.1:50551`):
//! ```text
//! cargo run -p celnet-server --example demo_edge
//! ```
//! In another:
//! ```text
//! cargo run -p celnet-client --example price_exotic
//! ```
//!
//! It exits non-zero unless each product returns a finite, positive priced result —
//! so a CI lane that boots the edge and runs the example asserts a real priced
//! exotic, not just a clean compile.

use std::error::Error;
use std::process::ExitCode;

use celnet_client::{
    AmericanTerms, AsianTerms, Client, Conventions, InstrumentSpec, MarketContext, Quantity, Side,
    StrikeSpec,
};
use celnet_types::{CcyPair, OptionType, Tenor};

/// The gRPC endpoint, overridable. The default matches `demo_edge`.
fn endpoint() -> String {
    std::env::var("CELNET_GRPC_ADDR")
        .map(|a| {
            if a.starts_with("http") {
                a
            } else {
                format!("http://{a}")
            }
        })
        .unwrap_or_else(|_| "http://127.0.0.1:50551".to_owned())
}

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("price_exotic failed: {e}");
            eprintln!(
                "is a demo edge running? start one with: \
                 cargo run -p celnet-server --example demo_edge"
            );
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), Box<dyn Error>> {
    let endpoint = endpoint();
    println!("connecting to {endpoint}");
    let client = Client::connect(endpoint).await?;

    let pair = CcyPair::parse("EURUSD").expect("EURUSD parses");
    let conventions = Conventions::major_default();
    // A mild EURUSD market context — spot 1.10, 10.5 ATM vol, 2%/1% rates.
    let market = MarketContext {
        spot: 1.10,
        vol: 0.105,
        r_dom: 0.02,
        r_for: 0.01,
    };

    // ---- arithmetic-average-rate Asian call, 12 monthly fixings, K = 1.10 --------
    let asian = InstrumentSpec::asian_option(
        pair,
        Tenor::Years(1),
        1.0,
        Quantity::base(1_000_000.0),
        Side::TwoWay,
        AsianTerms::fresh_discrete(OptionType::Call, 1.10, 12),
    );
    let priced_asian = client.price(&asian, market, conventions).await?;
    println!(
        "Asian call (12 fixings, K=1.10): price {price:.8}  vega {vega:.6}  std_err {se:?}",
        price = priced_asian.greeks.price,
        vega = priced_asian.greeks.vega,
        se = priced_asian.price_std_error,
    );
    if !(priced_asian.greeks.price.is_finite() && priced_asian.greeks.price > 0.0) {
        return Err(format!(
            "Asian returned a non-positive / non-finite price: {}",
            priced_asian.greeks.price
        )
        .into());
    }

    // ---- American put, K = 1.10, vs the European reference ------------------------
    let american = InstrumentSpec::american(
        pair,
        Tenor::Years(1),
        1.0,
        Quantity::base(1_000_000.0),
        Side::TwoWay,
        AmericanTerms::american(OptionType::Put, 1.10),
    );
    let priced_american = client.price(&american, market, conventions).await?;

    let european = InstrumentSpec::vanilla(
        pair,
        Tenor::Years(1),
        1.0,
        Quantity::base(1_000_000.0),
        Side::TwoWay,
        OptionType::Put,
        StrikeSpec::Absolute(1.10),
    );
    let priced_european = client.price(&european, market, conventions).await?;

    let american_px = priced_american.greeks.price;
    let european_px = priced_european.greeks.price;
    println!(
        "American put (K=1.10): price {american_px:.8}   European put: {european_px:.8}   \
         early-exercise premium {prem:.8}",
        prem = american_px - european_px,
    );

    // Structural sanity: an American option is never worth less than the European of
    // the same strike (the holder can always choose to wait).
    if !(american_px.is_finite() && american_px >= european_px - 1e-9) {
        return Err(format!(
            "American put {american_px} priced below European {european_px} (early exercise \
             can only add value)"
        )
        .into());
    }

    println!("done: priced an Asian and an American via the SDK vocabulary builders.");
    Ok(())
}
