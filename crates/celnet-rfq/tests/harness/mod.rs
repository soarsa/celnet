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
use celnet_fix::dialect_rates;
use celnet_fix::dictionary::MsgType;
use celnet_fix::framing::FrameCursor;
use celnet_fix::messages::{self, QuoteParams};
use celnet_fix::session::{InMemoryStore, Role, Session, SessionConfig};
use celnet_fix::transport::{FrameReader, write_frame};
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
    let fx = request
        .fx()
        .expect("spawn_fix_lp expects an FX-option RFQ leg");

    let qs = FixedQuoteSource {
        snapshot,
        tenor,
        half_spread,
        validity_ticks: 1_000_000,
        pricer,
        securities: vec![dialect_fx::SecurityDef::new(
            format!("{}{}", fx.pair.base.as_str(), fx.pair.quote.as_str()).as_bytes(),
            dialect_fx::SEC_TYPE_FXVO,
            fx.pair.quote.as_str().as_bytes(),
        )],
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
            let mut acc = Acceptor::new(session, qs.clone());
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
        pair: fx.pair,
        option_type: fx.option_type,
        strike: fx.strike,
        strike_ccy: fx.pair.quote,
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

/// Boot a **real** loopback FIX 4.4 rates LP: a synthetic fixed-income liquidity
/// provider on an ephemeral `127.0.0.1` port that answers a rates `QuoteRequest(R)`
/// — decoding it through the `celnet-fix` rates dialect (`SecurityType(167)=OIS` /
/// `BOND`), so the [`FixLpAdapter`]'s rates request is proven a valid dialect frame
/// — with a `Quote(S)` carrying the **injected** two-way `market` (the oracle's
/// KNOWN ground truth, exactly like [`LadderSource`], but over a genuine FIX
/// session — no mock). Returns a real [`FixLpAdapter`] wired to dial it.
///
/// The injected two-way must be representable at the 8-dp wire precision the
/// `Quote(S)` price fields carry (choose clean values, e.g. `0.0404` / `98.50`), so
/// it round-trips to the adapter bit-for-bit and the oracle can assert the winner.
pub async fn spawn_fix_rates_lp(
    lp_id: &str,
    market: TwoWay,
    epoch_nanos: u64,
    valid_for_nanos: u64,
) -> FixLpAdapter {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let bid = market.bid;
    let offer = market.offer;

    tokio::spawn(async move {
        // Serve sequential sessions (each RFQ opens a fresh one), mirroring the FX
        // acceptor loop but replying a rates two-way for a rates QuoteRequest.
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let mut session = Session::new(acc_cfg(), InMemoryStore::new());
            let (read_half, mut write_half) = tokio::io::split(stream);
            let mut reader = FrameReader::new(read_half);
            while let Ok(Some(frame)) = reader.next_frame().await {
                let Ok(action) = session.on_inbound(&frame, SENDING_TIME) else {
                    break; // a session/dialect fault: end this connection.
                };
                let mut fault = false;
                for f in &action.outbound {
                    if write_frame(&mut write_half, f).await.is_err() {
                        fault = true;
                        break;
                    }
                }
                if fault {
                    break;
                }
                if action.deliver == Some(MsgType::QuoteRequest) {
                    let Ok(cursor) = FrameCursor::parse(&frame) else {
                        continue;
                    };
                    // Prove the adapter sent a valid rates-dialect QuoteRequest: it
                    // must decode as an OIS or a cash-bond RFQ. A frame that is
                    // neither is not quoted (the LP shows no price).
                    let is_rates = dialect_rates::decode_rates_rfq(&cursor).is_ok()
                        || dialect_rates::decode_bond_rfq(&cursor).is_ok();
                    if !is_rates {
                        continue;
                    }
                    let req_id = cursor.get(131).map(<[u8]>::to_vec).unwrap_or_default();
                    let symbol = cursor.get(55).map(<[u8]>::to_vec).unwrap_or_default();
                    let out = session.send_app(SENDING_TIME, |h, e| {
                        let p = QuoteParams {
                            quote_req_id: &req_id,
                            quote_id: b"RQ-RATES-1",
                            symbol: &symbol,
                            bid_px: bid,
                            offer_px: offer,
                            size: 1_000_000.0,
                            valid_until: b"20260608-12:00:05.000",
                        };
                        messages::build_quote(h, &p, e)
                    });
                    if write_frame(&mut write_half, &out).await.is_err() {
                        break;
                    }
                }
            }
        }
    });

    FixLpAdapter::new(FixLpConfig {
        lp_id: lp_id.to_owned(),
        dial_addr: format!("{addr}"),
        sender_comp_id: b"TAKER".to_vec(),
        target_comp_id: b"VENUE".to_vec(),
        sending_time: SENDING_TIME.to_vec(),
        epoch_nanos,
        valid_for_nanos,
    })
}
