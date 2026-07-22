//! The **network feed mode** of the `lp-sim` binary (`--addr`): push the LP-SIM
//! panel's live two-ways to a running celnet server's `LiquidityFeedService.LpFeed`
//! ingest, so the consolidated composite surfaces to GUI subscribers of the target
//! aggregated book — rather than the composite being printed locally.
//!
//! The feed is a **client-streaming** gRPC call: the fleet's per-member,
//! per-instrument top-of-book is produced on a fixed interval and streamed as
//! [`LpQuote`]s until the process is stopped (or, in `--once` mode, one round then a
//! half-close to collect the [`LpFeedAck`]). The connect / reconnect lifecycle is
//! supervised in-process (a dropped connection re-dials after a short backoff),
//! mirroring how the FIX simulator supervises its initiator.
//!
//! The local in-process mode (the crate's default) stays fully synchronous; the
//! tokio runtime here is built only when `--addr` is passed.

use std::time::Duration;

use celnet_aggregation::VenueFeed;
use celnet_proto::LpQuote;
use celnet_proto::liquidity_feed_service_client::LiquidityFeedServiceClient;

use crate::lpsim::LpQuoteSnapshot;
use crate::universe::TreasuryBond;
use crate::{LpSimConfig, SimLp, build_fleet};

/// The reconnect backoff after a dropped / failed feed connection.
const RECONNECT_BACKOFF: Duration = Duration::from_secs(5);

/// Produce ONE round of wire [`LpQuote`]s: every member's current top-of-book for
/// every selected bond at `now_nanos`, stamped with the canonical `instrument_id`
/// (CUSIP). Pure and deterministic for a fixed `(fleet, bonds, now_nanos)` — the
/// unit test asserts its shape without a network.
#[must_use]
pub fn lp_quotes_round(fleet: &[SimLp], bonds: &[TreasuryBond], now_nanos: i64) -> Vec<LpQuote> {
    let mut out = Vec::with_capacity(fleet.len() * bonds.len());
    for member in fleet {
        for bond in bonds {
            if let Some(q) = member.top_of_book(&bond.engine_instrument(), now_nanos) {
                let snap = LpQuoteSnapshot::from_quote(&q, bond);
                out.push(LpQuote {
                    lp_name: snap.lp_name,
                    instrument_id: snap.instrument_id,
                    bid: snap.bid,
                    offer: snap.offer,
                    bid_size: snap.bid_size,
                    offer_size: snap.offer_size,
                    ts_nanos: snap.ts,
                });
            }
        }
    }
    out
}

/// Run the network feed to `addr` (e.g. `http://127.0.0.1:50051`): build the fleet,
/// then connect + stream rounds every `interval_secs`, supervising reconnects. In
/// `--once` mode a single round is streamed and the returned [`LpFeedAck`] accepted
/// count is reported. Blocks the calling thread on a private tokio runtime.
///
/// # Errors
/// Returns a message only for an unrecoverable setup failure (the runtime cannot be
/// built). Connection failures are logged and retried (continuous mode) or surfaced
/// after one attempt (`--once`).
pub fn run_network_feed(
    cfg: &LpSimConfig,
    selection: &[TreasuryBond],
    addr: &str,
    interval_secs: u64,
    once: bool,
) -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("build tokio runtime: {e}"))?;
    runtime.block_on(feed_loop(cfg, selection, addr, interval_secs, once))
}

/// The async connect → stream → (reconnect) supervision loop.
async fn feed_loop(
    cfg: &LpSimConfig,
    selection: &[TreasuryBond],
    addr: &str,
    interval_secs: u64,
    once: bool,
) -> Result<(), String> {
    loop {
        match connect_and_stream(cfg, selection, addr, interval_secs, once).await {
            Ok(accepted) => {
                if once {
                    tracing::info!(accepted, "lp-sim: server accepted quotes (once)");
                    println!("[lp-sim] server accepted {accepted} quote(s)");
                    return Ok(());
                }
                // A clean end of a continuous stream (server closed) — re-dial.
                tracing::warn!("lp-sim: feed stream ended; reconnecting");
            }
            Err(e) => {
                if once {
                    return Err(e);
                }
                tracing::warn!(error = %e, "lp-sim: feed connection error; reconnecting");
                eprintln!("[lp-sim] feed error: {e} — reconnecting in {RECONNECT_BACKOFF:?}");
            }
        }
        tokio::time::sleep(RECONNECT_BACKOFF).await;
    }
}

/// Dial `addr` and stream the fleet's quotes; returns the server's accepted count
/// when the stream ends (only reached in `--once` mode, or on a server-side close).
async fn connect_and_stream(
    cfg: &LpSimConfig,
    selection: &[TreasuryBond],
    addr: &str,
    interval_secs: u64,
    once: bool,
) -> Result<u64, String> {
    let mut client = LiquidityFeedServiceClient::connect(addr.to_owned())
        .await
        .map_err(|e| format!("connect {addr}: {e}"))?;
    tracing::info!(
        addr,
        members = cfg.members,
        "lp-sim: connected; streaming LpFeed"
    );

    // The fleet is rebuilt per connection so a reconnect reproduces the same
    // deterministic feed from the seed.
    let fleet = build_fleet(cfg, selection);
    let bonds = selection.to_vec();
    let interval = Duration::from_secs(interval_secs.max(1));
    let start = std::time::Instant::now();

    // The client-streaming request: an unfold generator yields the next `LpQuote`,
    // sleeping `interval` between rounds; `--once` ends the stream after one round.
    let state = FeedState {
        fleet,
        bonds,
        cursor: 0,
        pending: Vec::new(),
        round_done_once: false,
        interval,
        once,
        start,
    };
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

/// The unfold generator state driving the client-streaming request.
struct FeedState {
    fleet: Vec<SimLp>,
    bonds: Vec<TreasuryBond>,
    /// The buffered quotes of the current round, drained front-to-back.
    pending: Vec<LpQuote>,
    /// Read cursor into `pending`.
    cursor: usize,
    /// In `--once` mode, whether the single round has already been emitted.
    round_done_once: bool,
    interval: Duration,
    once: bool,
    start: std::time::Instant,
}

impl FeedState {
    /// The next `LpQuote` to stream, or `None` to end the stream (only in `--once`
    /// mode, after one full round). Sleeps `interval` between rounds in continuous
    /// mode.
    async fn next_quote(&mut self) -> Option<LpQuote> {
        loop {
            if self.cursor < self.pending.len() {
                let q = self.pending[self.cursor].clone();
                self.cursor += 1;
                return Some(q);
            }
            // Current round drained. In `--once` mode, one round only.
            if self.once && self.round_done_once {
                return None;
            }
            if self.round_done_once {
                // Pace continuous rounds by the interval.
                tokio::time::sleep(self.interval).await;
            }
            let now_nanos = self.now_nanos();
            self.pending = lp_quotes_round(&self.fleet, &self.bonds, now_nanos);
            self.cursor = 0;
            self.round_done_once = true;
            if self.pending.is_empty() {
                // Nothing to quote (no modellable instruments) — end rather than spin.
                return None;
            }
        }
    }

    /// The valuation clock in epoch nanoseconds (real wall time so the server's
    /// staleness decay sees a monotonically advancing observation instant).
    fn now_nanos(&self) -> i64 {
        let since_epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        i64::try_from(since_epoch.as_nanos()).unwrap_or(i64::MAX)
            + i64::try_from(self.start.elapsed().as_nanos()).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_types::BrokenDate;

    #[test]
    fn one_round_produces_a_quote_per_member_per_bond() {
        let cfg = LpSimConfig {
            members: 3,
            settlement: BrokenDate::new(2026, 4, 16),
            ..LpSimConfig::default()
        };
        let universe = crate::load_coupon_universe();
        let mut selection: Vec<TreasuryBond> = universe
            .into_iter()
            .filter(|b| {
                b.yield_model(cfg.settlement, cfg.reversion_per_sec, cfg.perturbation)
                    .is_some()
            })
            .take(2)
            .collect();
        selection.truncate(2);
        assert_eq!(selection.len(), 2, "need two modellable bonds");
        let fleet = build_fleet(&cfg, &selection);
        let now = 1_700_000_000_000_000_000;
        let quotes = lp_quotes_round(&fleet, &selection, now);
        // 3 members × 2 bonds = 6 quotes, each well-formed and CUSIP-stamped.
        assert_eq!(quotes.len(), 6);
        for q in &quotes {
            assert!(!q.lp_name.is_empty());
            assert_eq!(
                q.instrument_id.len(),
                9,
                "instrument_id is the 9-char CUSIP"
            );
            assert!(q.bid.is_finite() && q.offer.is_finite());
            assert!(q.offer >= q.bid, "two-way is not crossed");
            assert_eq!(q.ts_nanos, now);
        }
        // Every member of the panel contributed.
        let venues: std::collections::HashSet<&str> =
            quotes.iter().map(|q| q.lp_name.as_str()).collect();
        assert_eq!(venues.len(), 3);
    }
}
