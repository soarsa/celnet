//! The venue's **network mode**: publish the listed two-ways into a running celnet
//! server's `LiquidityFeedService.LpFeed` ingest while accepting orders against them
//! over FIX.
//!
//! The two halves run concurrently on one runtime and share one book of published
//! markets (see [`celnet_lp_sim::orders::LiveMarkets`]), which is the invariant that
//! keeps the venue honest: a taker trades against exactly the level the composite
//! was built from, because both come from the same `top_of_book` call.
//!
//! The publish path is deliberately the *same* client-streaming RPC the OTC
//! simulator uses, so a listed contract reaches an aggregated book by exactly the
//! route a cash bond does and needs no server-side special case.

use std::time::Duration;

use celnet_lp_sim::orders::{LiveMarkets, OrderVenue, run_order_acceptor};
use celnet_lp_sim::quoted::QuotedLine;
use celnet_proto::LpQuote;
use celnet_proto::liquidity_feed_service_client::LiquidityFeedServiceClient;

use crate::contract::FuturesContract;
use crate::venue::{VENUE_ID, build_order_venue, build_venue_feed, publish_markets};

/// The reconnect backoff after a dropped / failed feed connection — the same
/// five-second cadence the OTC simulator supervises its feed on.
const RECONNECT_BACKOFF: Duration = Duration::from_secs(5);

/// The tunables of one venue run.
#[derive(Debug, Clone)]
pub struct FuturesFeedOptions {
    /// How often a fresh round of two-ways is published.
    pub quote_interval: Duration,
    /// The `host:port` the FIX order acceptor binds, or `None` to publish prices
    /// only. A venue that quotes without accepting orders is a price display, not a
    /// venue, so the deployed configuration always sets this.
    pub order_bind: Option<String>,
    /// The root seed — the same seed reproduces the venue byte-for-byte.
    pub seed: u64,
    /// Publish a single round and exit.
    pub once: bool,
}

impl Default for FuturesFeedOptions {
    fn default() -> Self {
        Self {
            quote_interval: Duration::from_secs(2),
            order_bind: None,
            seed: 0x1234_5678,
            once: false,
        }
    }
}

/// Run the venue against `addr` (e.g. `http://127.0.0.1:50051`): bind the order
/// acceptor, then connect and publish rounds, supervising reconnects. Blocks the
/// calling thread on a private tokio runtime.
///
/// # Errors
/// Returns a message for an unrecoverable setup failure (the runtime cannot be
/// built, or the order port cannot be bound) or, in `--once` mode, the first
/// connection error. A dropped connection in continuous mode is logged and retried.
pub fn run_futures_feed(
    lines: &[QuotedLine],
    contracts: &[FuturesContract],
    addr: &str,
    opts: &FuturesFeedOptions,
) -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("build tokio runtime: {e}"))?;
    runtime.block_on(supervise(lines, contracts, addr, opts))
}

/// Bind the order acceptor once, then loop the publish session.
async fn supervise(
    lines: &[QuotedLine],
    contracts: &[FuturesContract],
    addr: &str,
    opts: &FuturesFeedOptions,
) -> Result<(), String> {
    let markets: LiveMarkets = LiveMarkets::default();
    let order_venue = build_order_venue(LiveMarkets::clone(&markets), opts.seed);

    // The order acceptor is bound BEFORE the first publish, so there is never a
    // window in which the venue is showing prices it cannot be hit on.
    let _acceptor = match &opts.order_bind {
        Some(bind) => {
            let (local, handle) = run_order_acceptor(order_venue.clone(), bind)
                .await
                .map_err(|e| format!("bind order acceptor on {bind}: {e}"))?;
            eprintln!("[cme-sim] order acceptor listening on {local}");
            Some(handle)
        }
        None => {
            eprintln!(
                "[cme-sim] WARNING: no --order-port; publishing prices only. \
                 Orders sent to this venue will not be answered."
            );
            None
        }
    };

    loop {
        match publish_session(lines, contracts, addr, opts, &order_venue).await {
            Ok(accepted) => {
                if opts.once {
                    println!("[cme-sim] server accepted {accepted} quote(s)");
                    return Ok(());
                }
                tracing::warn!("cme-sim: feed stream ended; reconnecting");
            }
            Err(e) => {
                if opts.once {
                    return Err(e);
                }
                tracing::warn!(error = %e, "cme-sim: feed connection error; reconnecting");
                eprintln!("[cme-sim] feed error: {e} — reconnecting in {RECONNECT_BACKOFF:?}");
            }
        }
        tokio::time::sleep(RECONNECT_BACKOFF).await;
    }
}

/// One connected publish session: stream rounds until the server closes the stream.
async fn publish_session(
    lines: &[QuotedLine],
    contracts: &[FuturesContract],
    addr: &str,
    opts: &FuturesFeedOptions,
    order_venue: &OrderVenue,
) -> Result<u64, String> {
    let mut client = LiquidityFeedServiceClient::connect(addr.to_owned())
        .await
        .map_err(|e| format!("connect {addr}: {e}"))?;
    tracing::info!(
        addr,
        venue = VENUE_ID,
        lines = lines.len(),
        "cme-sim: connected"
    );

    let state = RoundState {
        feed: build_venue_feed(lines, opts.seed),
        order_venue: order_venue.clone(),
        lines: lines.to_vec(),
        contracts: contracts.to_vec(),
        interval: opts.quote_interval.max(Duration::from_millis(1)),
        once: opts.once,
        pending: Vec::new(),
        cursor: 0,
        started: false,
    };
    // The SAME client-streaming generator shape the OTC simulator's feed uses.
    let request = futures_util::stream::unfold(state, |mut st| async move {
        st.next_quote().await.map(|q| (q, st))
    });
    let ack = client
        .lp_feed(request)
        .await
        .map_err(|e| format!("lp_feed stream: {e}"))?
        .into_inner();
    Ok(ack.accepted)
}

/// The unfold generator state driving the client-streaming publish.
struct RoundState {
    feed: celnet_lp_sim::SimLp,
    order_venue: OrderVenue,
    lines: Vec<QuotedLine>,
    contracts: Vec<FuturesContract>,
    interval: Duration,
    once: bool,
    pending: Vec<LpQuote>,
    cursor: usize,
    started: bool,
}

impl RoundState {
    /// The next `LpQuote` to stream, or `None` to end the stream (`--once` only).
    ///
    /// Each round publishes the venue's tradeable book from the SAME `top_of_book`
    /// call that produces the wire quotes, so the two can never diverge.
    async fn next_quote(&mut self) -> Option<LpQuote> {
        loop {
            if self.cursor < self.pending.len() {
                let q = self.pending[self.cursor].clone();
                self.cursor += 1;
                return Some(q);
            }
            if self.once && self.started {
                return None;
            }
            if self.started {
                tokio::time::sleep(self.interval).await;
            }
            let now = wall_nanos();
            publish_markets(
                &self.feed,
                &self.order_venue,
                &self.lines,
                &self.contracts,
                now,
            );
            self.pending = self.round(now);
            self.cursor = 0;
            self.started = true;
            if self.pending.is_empty() {
                // Nothing quotable (every contract expired) — end rather than spin.
                return None;
            }
        }
    }

    /// One round of wire quotes, read back out of the book just published so the
    /// wire and the tradeable market are literally the same numbers.
    fn round(&self, now: i64) -> Vec<LpQuote> {
        let book = match self.order_venue.markets().read() {
            Ok(b) => b,
            // A poisoned lock means the publisher panicked; emitting nothing is the
            // honest answer, and the consolidator will age this venue out.
            Err(_) => return Vec::new(),
        };
        self.lines
            .iter()
            .filter_map(|line| {
                let m = book.get(&line.instrument_id)?;
                m.is_tradeable().then(|| LpQuote {
                    lp_name: VENUE_ID.to_owned(),
                    instrument_id: line.instrument_id.clone(),
                    bid: m.bid,
                    offer: m.offer,
                    bid_size: m.bid_size,
                    offer_size: m.offer_size,
                    ts_nanos: m.ts_nanos.min(now),
                })
            })
            .collect()
    }
}

/// The venue's valuation clock in epoch nanoseconds: wall time and nothing else.
fn wall_nanos() -> i64 {
    celnet_lp_sim::orders::wall_nanos()
}
