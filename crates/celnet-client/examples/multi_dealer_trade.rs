//! SDK quickstart — request a multi-dealer (RFQ-to-many) panel, then book the
//! best LP line.
//!
//! The RFQ-to-many trader workflow over the typed [`celnet_client`] SDK: connect
//! to a running edge, fan one RFQ on a 1Y EURUSD vanilla call across the edge's
//! LP panel, print the ranked dealer ladder with the touch on each side, then
//! BUY the best-offer dealer's pinned line and print the booked execution.
//!
//! The `demo_edge` boots with a native maker + 3 deterministic **synthetic**
//! demo dealers (override the breadth with `CELNET_DEMO_LPS`) — live LP
//! connectivity is an environment concern, never claimed by this example.
//!
//! # Run it
//!
//! In one terminal, boot a seeded demo edge (binds gRPC on `127.0.0.1:50551`):
//! ```text
//! cargo run -p celnet-server --example demo_edge
//! ```
//! In another, run this example (override the address with `CELNET_GRPC_ADDR`):
//! ```text
//! cargo run -p celnet-client --example multi_dealer_trade
//! ```
//!
//! It exits non-zero if the edge returns no ranked panel or a degenerate touch —
//! so a CI lane that boots the edge and runs the example asserts a real,
//! non-empty ranked-and-booked result, not just a clean compile.

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
            eprintln!("multi_dealer_trade failed: {e}");
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

    // Fan the RFQ across the edge's LP panel. The handle owns a stable
    // idempotency key, so a retry re-ranks the same aggregate quote.
    let md = client.request_multi_dealer_quote(instrument, Conventions::major_default());
    let panel = md.request().await?;

    println!(
        "panel #{id}: {n} dealer(s) responded",
        id = panel.quote_id,
        n = panel.dealers.len(),
    );
    for d in &panel.dealers {
        println!(
            "  {lp:<20} bid {bid:.6}  offer {offer:.6}{native}",
            lp = d.lp_id,
            bid = d.price.bid,
            offer = d.price.offer,
            native = if d.greeks.is_some() {
                "  (native maker)"
            } else {
                ""
            },
        );
    }

    // Guard: a real edge must return a ranked panel with an uncrossed touch.
    let best_bid = panel
        .best_bid()
        .ok_or("edge returned a panel with no liftable bid")?;
    let best_offer = panel
        .best_offer()
        .ok_or("edge returned a panel with no liftable offer")?;
    println!(
        "touch: best bid {bid:.6} ({bl})  best offer {offer:.6} ({ol})",
        bid = best_bid.price.bid,
        bl = best_bid.lp_id,
        offer = best_offer.price.offer,
        ol = best_offer.lp_id,
    );
    if !(best_offer.price.offer.is_finite() && best_bid.price.bid <= best_offer.price.offer) {
        return Err(format!(
            "edge returned a degenerate touch: bid {} offer {}",
            best_bid.price.bid, best_offer.price.offer
        )
        .into());
    }

    // BUY the best-offer dealer's pinned line: the booking is exactly the price
    // the panel showed (never a re-price), attributed to that LP.
    let lp = best_offer.lp_id.clone();
    let offer = best_offer.price.offer;
    let execution = md.accept_dealer(&panel, Side::Buy, &*lp).await?;
    println!(
        "booked execution #{exec_id} on {side:?} vs {lp}: traded premium {prem:.6}",
        exec_id = execution.execution_id,
        side = execution.side,
        prem = execution.traded_premium,
    );

    if execution.execution_id == 0 {
        return Err("edge booked execution id 0 (no booking)".into());
    }
    if execution.traded_premium.to_bits() != offer.to_bits() {
        return Err(format!(
            "booked premium {} is not the pinned panel offer {}",
            execution.traded_premium, offer
        )
        .into());
    }
    println!("done: ranked a multi-dealer panel and booked the best LP line.");
    Ok(())
}
