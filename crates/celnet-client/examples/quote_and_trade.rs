//! SDK quickstart — request a quote, then accept (book) it.
//!
//! The canonical RFQ trader workflow over the typed [`celnet_client`] SDK:
//! connect to a running edge, request a two-way quote on a 1Y EURUSD vanilla call,
//! print the bid/offer + the headline Greeks the maker returned, then BUY (lift the
//! offer) and print the booked execution.
//!
//! # Run it
//!
//! In one terminal, boot a seeded demo edge (binds gRPC on `127.0.0.1:50551`):
//! ```text
//! cargo run -p celnet-server --example demo_edge
//! ```
//! In another, run this example (override the address with `CELNET_GRPC_ADDR`):
//! ```text
//! cargo run -p celnet-client --example quote_and_trade
//! ```
//!
//! It exits non-zero if the edge returns no priced quote — so a CI lane that boots
//! the edge and runs the example asserts a real, non-empty priced result, not just
//! a clean compile.

use std::error::Error;
use std::process::ExitCode;

use celnet_client::{Client, Conventions, InstrumentSpec, Quantity, Side, StrikeSpec};
use celnet_types::{CcyPair, OptionType, Tenor};

/// The gRPC endpoint, overridable so the same example serves any local layout. The
/// default matches `celnet-server`'s `demo_edge` gRPC bind (`127.0.0.1:50551`).
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
            eprintln!("quote_and_trade failed: {e}");
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

    // A 1Y EURUSD vanilla call struck at 1.12, 1mm EUR notional, two-way request.
    let instrument = InstrumentSpec::vanilla(
        CcyPair::parse("EURUSD").expect("EURUSD parses"),
        Tenor::Years(1),
        1.0,
        Quantity::base(1_000_000.0),
        Side::TwoWay,
        OptionType::Call,
        StrikeSpec::Absolute(1.12),
    );

    // Request the two-way quote. The handle owns a stable idempotency key, so a
    // retry would return the *same* quote rather than re-pricing.
    let rfq = client.request_quote(instrument, Conventions::major_default());
    let quote = rfq.request().await?;

    println!(
        "quote #{id}: bid {bid:.6}  offer {offer:.6}  (mid {mid:.6})  K={strike:.4}",
        id = quote.quote_id,
        bid = quote.price.bid,
        offer = quote.price.offer,
        mid = quote.price.mid(),
        strike = quote.resolved_strike,
    );
    println!(
        "  greeks: delta {:.6}  vega {:.6}  gamma {:.6}  theta {:.6}",
        quote.greeks.delta_spot, quote.greeks.vega, quote.greeks.gamma, quote.greeks.theta
    );

    // Guard: a real edge must return a finite priced two-way. Empty / non-finite ⇒
    // the example fails loudly (the CI assertion the doc-comment promises).
    if !(quote.price.offer.is_finite() && quote.price.offer >= quote.price.bid) {
        return Err(format!(
            "edge returned a degenerate quote: bid {} offer {}",
            quote.price.bid, quote.price.offer
        )
        .into());
    }

    // BUY lifts the offer and books. The accept is idempotent under the same handle.
    let execution = rfq.accept(&quote, Side::Buy).await?;
    println!(
        "booked execution #{exec_id} on {side:?}: traded premium {prem:.6}",
        exec_id = execution.execution_id,
        side = execution.side,
        prem = execution.traded_premium,
    );

    if execution.execution_id == 0 {
        return Err("edge booked execution id 0 (no booking)".into());
    }
    println!("done: quoted and booked a vanilla call.");
    Ok(())
}
