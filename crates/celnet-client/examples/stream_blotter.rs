//! SDK quickstart — a streaming blotter over one multiplexed RFS session.
//!
//! Opens ONE [`celnet_client::StreamSession`] and subscribes three vanilla EURUSD
//! calls at distinct strikes over that single connection (a blotter uses one
//! session, not one stream per line). It prints each subscription's opening
//! snapshot, then drains a handful of live ticks per line, printing the moving
//! two-way + vol the SDK surfaces — the per-subscription sequence, gap-detection,
//! and resync are all handled inside the SDK.
//!
//! # Run it
//!
//! In one terminal, boot a seeded demo edge (binds gRPC on `127.0.0.1:50551`):
//! ```text
//! cargo run -p celnet-server --example demo_edge
//! ```
//! In another:
//! ```text
//! cargo run -p celnet-client --example stream_blotter
//! ```
//!
//! It exits non-zero unless every subscription delivered a finite-priced baseline
//! snapshot and at least one live tick — so a CI lane that boots the edge and runs
//! the example asserts a real, moving blotter, not just a clean compile.

use std::error::Error;
use std::process::ExitCode;
use std::time::Duration;

use celnet_client::{
    Client, Conventions, InstrumentSpec, Quantity, Side, StreamEvent, StrikeSpec, Subscription,
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

/// Bound every stream await so a never-arriving line fails fast rather than hanging.
const STEP: Duration = Duration::from_secs(5);

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("stream_blotter failed: {e}");
            eprintln!(
                "is a demo edge running? start one with: \
                 cargo run -p celnet-server --example demo_edge"
            );
            ExitCode::FAILURE
        }
    }
}

/// Await the next event on a subscription within the step deadline.
async fn next_event(sub: &mut Subscription) -> Result<StreamEvent, Box<dyn Error>> {
    let ev = tokio::time::timeout(STEP, sub.next_event())
        .await
        .map_err(|_| "timed out waiting for a stream event")?
        .ok_or("stream closed before an event arrived")??;
    Ok(ev)
}

async fn run() -> Result<(), Box<dyn Error>> {
    let endpoint = endpoint();
    println!("connecting to {endpoint}");
    let client = Client::connect(endpoint).await?;

    let pair = CcyPair::parse("EURUSD").expect("EURUSD parses");
    let strikes = [1.08_f64, 1.12, 1.16];

    // ONE session multiplexing all three subscriptions.
    let session = client.open_session().await?;
    let mut subs: Vec<Subscription> = Vec::new();
    for (i, &k) in strikes.iter().enumerate() {
        let instrument = InstrumentSpec::vanilla(
            pair,
            Tenor::Years(1),
            1.0,
            Quantity::base(1_000_000.0),
            Side::TwoWay,
            OptionType::Call,
            StrikeSpec::Absolute(k),
        );
        let sub = session
            .subscribe(
                instrument,
                Conventions::major_default(),
                Some(1000 + i as u64),
                None,
            )
            .await?;
        subs.push(sub);
    }
    println!("opened {} subscriptions over one session", subs.len());

    // Consume each subscription's baseline snapshot, then a few live ticks.
    let mut total_ticks = 0_usize;
    for (i, sub) in subs.iter_mut().enumerate() {
        let strike = strikes[i];

        // The first event is the baseline snapshot.
        let (mut last_seq, snap_offer) = match next_event(sub).await? {
            StreamEvent::Snapshot { line, .. } => {
                println!(
                    "K={strike:.4} snapshot seq {seq}: bid {bid:.6} offer {offer:.6} vol {vol:.4}",
                    seq = line.sequence,
                    bid = line.price.bid,
                    offer = line.price.offer,
                    vol = line.vol,
                );
                (line.sequence, line.price.offer)
            }
            other => return Err(format!("expected a Snapshot first, got {other:?}").into()),
        };
        if !snap_offer.is_finite() {
            return Err(format!("K={strike:.4}: non-finite snapshot offer").into());
        }

        // Drain up to three live ticks (skipping heartbeats).
        let mut ticks_here = 0;
        while ticks_here < 3 {
            match next_event(sub).await? {
                StreamEvent::Tick(line) => {
                    if line.sequence <= last_seq {
                        return Err(format!(
                            "K={strike:.4}: tick seq {} did not advance past {last_seq}",
                            line.sequence
                        )
                        .into());
                    }
                    last_seq = line.sequence;
                    println!(
                        "K={strike:.4} tick seq {seq}: bid {bid:.6} offer {offer:.6} vol {vol:.4}",
                        seq = line.sequence,
                        bid = line.price.bid,
                        offer = line.price.offer,
                        vol = line.vol,
                    );
                    ticks_here += 1;
                    total_ticks += 1;
                }
                StreamEvent::Heartbeat { .. } => {}
                other => return Err(format!("unexpected stream event: {other:?}").into()),
            }
        }
    }

    drop(subs);
    drop(session);

    if total_ticks == 0 {
        return Err("no live ticks streamed from the edge".into());
    }
    println!("done: streamed {total_ticks} live ticks across the blotter.");
    Ok(())
}
