//! Shared test harness for the multi-dealer RFQ oracle.
//!
//! Provides:
//! * [`LadderSource`] — a deterministic in-process [`QuoteSource`] with a KNOWN
//!   injected `(bid, offer, epoch, valid_until)` ladder and an optional response
//!   delay (to drive the timeout case). This is NOT the production
//!   `InternalPricerSource`; it is the oracle's ground-truth injector — the test
//!   KNOWS, by construction, which LP holds the best bid/offer.
//! * [`spawn_fix_lp`] — boots a **real** `celnet-fix` acceptor on an ephemeral
//!   `127.0.0.1` port that prices off an injected market snapshot via the
//!   golden-gated `celnet-vanilla` engine, and returns a real
//!   [`FixLpAdapter`](celnet_rfq::FixLpAdapter) wired to dial it. The adapter
//!   runs a genuine FIX 4.4 `QuoteRequest → Quote` session over the loopback
//!   socket — no mock.

// A shared integration-test harness module included via `mod harness;` from
// multiple test binaries. Its items are `pub` so each binary can reach them, but
// the compiler analyses each binary in isolation and so cannot see the
// cross-binary use — hence the two allows below (idiomatic for shared test
// support; no production code is affected).
#![allow(dead_code, unreachable_pub)]

use std::time::Duration;

use celnet_fix::acceptor::{Acceptor, FixedQuoteSource};
use celnet_fix::dialect_fx::{self, MarketSnapshot, VanillaPricer};
use celnet_fix::session::{InMemoryStore, Role, Session, SessionConfig};
use celnet_rfq::panel::{QuoteSource, QuoteSourceReply, RfqRequest, TwoWay};
use celnet_rfq::{FixLpAdapter, FixLpConfig};
use celnet_types::Tenor;
use tokio::net::TcpListener;

/// The FIX session sending-time bytes used throughout the harness.
pub const SENDING_TIME: &[u8] = b"20260608-12:00:00.000";

/// A deterministic in-process LP with a known injected two-way ladder. The
/// `epoch_nanos` / `valid_until_nanos` drive the panel tie-break and last-look,
/// and `delay` lets a test make the source respond *after* the panel deadline
/// (the timeout case). The oracle reads these fields directly to compute the
/// expected winner — it is the ground truth, not a second copy of the ranking
/// algebra.
#[derive(Debug, Clone)]
pub struct LadderSource {
    pub lp_id: String,
    pub bid: f64,
    pub offer: f64,
    pub epoch_nanos: u64,
    pub valid_until_nanos: u64,
    pub delay: Duration,
    /// When true the source declines (returns `NoQuote`) — the explicit
    /// no-market case, dropped exactly like a timeout.
    pub decline: bool,
}

impl LadderSource {
    /// A firm-quoting ladder source with no delay and a far-future validity.
    #[must_use]
    pub fn firm(lp_id: &str, bid: f64, offer: f64, epoch_nanos: u64) -> Self {
        Self {
            lp_id: lp_id.to_owned(),
            bid,
            offer,
            epoch_nanos,
            valid_until_nanos: u64::MAX,
            delay: Duration::ZERO,
            decline: false,
        }
    }

    /// Set an explicit last-look validity horizon.
    #[must_use]
    pub fn valid_until(mut self, valid_until_nanos: u64) -> Self {
        self.valid_until_nanos = valid_until_nanos;
        self
    }

    /// Set a response delay (used to exceed the panel deadline → timeout).
    #[must_use]
    pub fn with_delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }
}

impl QuoteSource for LadderSource {
    fn lp_id(&self) -> &str {
        &self.lp_id
    }

    fn request<'a>(
        &'a self,
        _request: &'a RfqRequest,
        _deadline: Duration,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = QuoteSourceReply> + Send + 'a>> {
        Box::pin(async move {
            if self.delay > Duration::ZERO {
                tokio::time::sleep(self.delay).await;
            }
            if self.decline {
                return QuoteSourceReply::NoQuote;
            }
            QuoteSourceReply::Quote {
                price: TwoWay {
                    bid: self.bid,
                    offer: self.offer,
                },
                epoch_nanos: self.epoch_nanos,
                valid_until_nanos: self.valid_until_nanos,
            }
        })
    }
}

/// Boot a **real** `celnet-fix` acceptor on an ephemeral loopback port that
/// quotes the option off `snapshot` using the golden-gated `celnet-vanilla`
/// engine with the given `half_spread`, and return a real [`FixLpAdapter`] wired
/// to dial it. The returned adapter, when fanned by the engine, opens a genuine
/// FIX 4.4 session over the socket. `epoch_nanos` / `valid_for_nanos` are the
/// engine-clock window the adapter stamps on the resulting reply.
///
/// The acceptor task runs until the test process ends (every test body is
/// deadline-bounded). Returns `(adapter, expected_two_way)` where the expected
/// two-way is the **independently** computed mid ± half_spread off the injected
/// snapshot (the FIX leg's ground truth).
pub async fn spawn_fix_lp(
    lp_id: &str,
    snapshot: MarketSnapshot,
    tenor: Tenor,
    half_spread: f64,
    epoch_nanos: u64,
    valid_for_nanos: u64,
    request: &RfqRequest,
) -> (FixLpAdapter, TwoWay) {
    let pricer: VanillaPricer = celnet_vanilla::price;

    let qs = FixedQuoteSource {
        snapshot,
        tenor,
        half_spread,
        validity_ticks: 1_000_000,
        pricer,
    };

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        // Serve as many sequential sessions as connect (each RFQ opens a fresh
        // one); loop so repeated fan-outs in a single test each get answered.
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let session = Session::new(acc_cfg(), InMemoryStore::new());
            let mut acc = Acceptor::new(session, qs);
            acc.run(stream, SENDING_TIME.to_vec()).await.ok();
        }
    });

    let adapter = FixLpAdapter::new(FixLpConfig {
        lp_id: lp_id.to_owned(),
        dial_addr: format!("{addr}"),
        sender_comp_id: b"TAKER".to_vec(),
        target_comp_id: b"VENUE".to_vec(),
        sending_time: SENDING_TIME.to_vec(),
        epoch_nanos,
        valid_for_nanos,
    });

    // Independent ground truth for the FIX leg: the mid the dialect prices off
    // the injected snapshot, ± the half-spread. Computed here, NOT read back
    // from the adapter.
    let desc = dialect_fx::OptionDescriptor {
        pair: request.pair,
        option_type: request.option_type,
        strike: request.strike,
        strike_ccy: request.pair.quote,
        exercise: dialect_fx::ExerciseStyle::European,
        tenor,
        settlement: celnet_types::Settlement::Deliverable,
    };
    let mid = dialect_fx::price_leg(&desc, &snapshot, pricer);
    let expected = TwoWay {
        bid: mid - half_spread,
        offer: mid + half_spread,
    };

    (adapter, expected)
}

fn acc_cfg() -> SessionConfig {
    SessionConfig {
        sender: b"VENUE".to_vec(),
        target: b"TAKER".to_vec(),
        heart_bt_int: 30,
        role: Role::Acceptor,
    }
}
