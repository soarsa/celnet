//! [`FixLpAdapter`] — an external liquidity provider on the panel, reached over
//! a **real FIX 4.4 session on a TCP socket**.
//!
//! This is a genuine [`celnet_fix::initiator::Initiator`] driving a real
//! `Logon → QuoteRequest(R) → Quote(S)` cycle against the LP's acceptor over a
//! loopback (or, at deploy, WAN) socket — NOT a mock. Per request the adapter:
//! 1. connects a fresh TCP stream to the LP's configured dial address;
//! 2. logs on and sends a FIX `QuoteRequest(R)` whose instrument block is
//!    translated from the Celnet [`RfqRequest`] via the `celnet-fix` dialect
//!    tags (`Symbol(55)`, `Product(460)`, `SecurityType(167)`, `PutOrCall(201)`,
//!    `StrikePrice(202)`, `StrikeCurrency(947)`);
//! 3. observes the returned `Quote(S)` (indicative — `LiftPolicy::Observe`, no
//!    lift: the panel only needs the two-way; the actual lift is the
//!    coordinator's `AcceptQuote` against the panel winner);
//! 4. translates the FIX `BidPx(132)` / `OfferPx(133)` (preferring the
//!    round-trip-exact dialect tags when present) back into a Celnet
//!    [`TwoWay`](crate::panel::TwoWay).
//!
//! Any transport / protocol failure, or no `Quote` within the deadline, maps to
//! [`QuoteSourceReply::NoQuote`] — the adapter is *dropped* from the panel, never
//! an error (a flaky LP must not fail the whole RFQ). The hard per-request
//! deadline is also enforced by the engine, so a wedged LP can never hang the
//! panel.
//!
//! # Honest boundary (verbatim — deploy/ENV-gated)
//!
//! **Live LP-panel connectivity (real bank sessions over WAN FIX) and the
//! regulated-venue / MAS-RMO status are ENV — designed, seamed and ADR'd
//! in-repo, validated at deploy, NEVER claimed in-repo.** In-repo this adapter
//! proves the aggregation/ranking/tie-break/last-look ALGORITHM and the FIX
//! framing/dialect round-trip over a **loopback** socket only. Live endpoints are
//! selected by the `CELNET_LP_PANEL` environment configuration on the edge,
//! defaulting to the synthetic in-repo panel; this crate carries no live
//! endpoint and makes no WAN-latency or venue-status claim.

use std::time::Duration;

use celnet_fix::framing::FrameEncoder;
use celnet_fix::initiator::{Initiator, LiftPolicy};
use celnet_fix::messages::Header;
use celnet_fix::session::{InMemoryStore, Role, Session, SessionConfig};
use celnet_types::{Ccy, OptionType};
use tokio::net::TcpStream;

use crate::panel::{QuoteSource, QuoteSourceReply, RfqRequest, TwoWay};

/// Static configuration for one external FIX LP: how to address its session and
/// what audit identity / validity window to stamp on its quotes.
#[derive(Debug, Clone)]
pub struct FixLpConfig {
    /// Stable audit `lp_id` for this LP (the panel tie-break key).
    pub lp_id: String,
    /// The LP acceptor's dial address (`host:port`). On loopback this is the
    /// ephemeral `127.0.0.1:<port>` the synthetic LP bound; at deploy it is the
    /// bank's FIX endpoint (ENV-provided, see the honest boundary).
    pub dial_addr: String,
    /// Our `SenderCompID` (the taker identity).
    pub sender_comp_id: Vec<u8>,
    /// The LP acceptor's `TargetCompID` (the venue identity).
    pub target_comp_id: Vec<u8>,
    /// The `SendingTime(52)` bytes used for the session frames (the edge's clock
    /// timestamp).
    pub sending_time: Vec<u8>,
    /// The logical timestamp (engine nanos clock) stamped on the resulting
    /// [`QuoteSourceReply`] as `epoch_nanos` (the panel tie-break key 1). The FIX
    /// `Quote`'s own `ValidUntilTime(62)` string is in wall time; the panel's
    /// last-look operates in the engine's nanos clock, so the adapter declares
    /// the window in that clock here.
    pub epoch_nanos: u64,
    /// How long (engine nanos) the LP's quote stays liftable from `epoch_nanos`;
    /// `valid_until_nanos = epoch_nanos + valid_for_nanos`.
    pub valid_for_nanos: u64,
}

/// An external LP reached over a real FIX session. Cheap to clone (config only);
/// each [`QuoteSource::request`] opens a fresh session.
#[derive(Debug, Clone)]
pub struct FixLpAdapter {
    cfg: FixLpConfig,
}

impl FixLpAdapter {
    /// Build an adapter from its config.
    #[must_use]
    pub fn new(cfg: FixLpConfig) -> Self {
        Self { cfg }
    }

    /// Run one real FIX `QuoteRequest → Quote` cycle and translate the result.
    /// `Ok(None)` means a clean no-quote (declined / no `Quote` returned);
    /// `Err` is a transport/protocol failure — both map to
    /// [`QuoteSourceReply::NoQuote`] at the call site (an LP is dropped, never an
    /// error).
    async fn run_cycle(&self, request: &RfqRequest) -> std::io::Result<Option<TwoWay>> {
        let stream = TcpStream::connect(&self.cfg.dial_addr).await?;
        let cfg = SessionConfig {
            sender: self.cfg.sender_comp_id.clone(),
            target: self.cfg.target_comp_id.clone(),
            heart_bt_int: 30,
            role: Role::Initiator,
        };
        let mut init = Initiator::new(Session::new(cfg, InMemoryStore::new()), LiftPolicy::Observe);

        // Translate the Celnet RFQ into the FIX instrument block. The strike
        // currency is the pair's quote currency (the dialect's convention guard
        // rejects anything else).
        let symbol = fix_symbol(request);
        let strike = request.strike;
        let put_or_call: &[u8] = match request.option_type {
            OptionType::Call => b"1",
            OptionType::Put => b"0",
        };
        let strike_ccy = ccy_bytes(request.pair.quote);
        let req_id = request.request_id.clone().into_bytes();

        let build_req = move |h: &Header<'_>, e: &mut FrameEncoder| {
            e.clear();
            h.encode(celnet_fix::MsgType::QuoteRequest, e);
            e.push(131, &req_id);
            e.push(55, &symbol);
            e.push(460, b"4"); // Product = CURRENCY
            e.push(167, b"FXVO"); // deliverable FX vanilla option
            e.push(201, put_or_call);
            push_strike(e, strike);
            e.push(947, &strike_ccy);
            e.push(1194, b"0"); // European exercise
            e.finish()
        };

        let result = init
            .request_and_lift(stream, self.cfg.sending_time.clone(), build_req)
            .await?;

        // Prefer the round-trip-exact dialect tags (full f64) over the 8-dp wire
        // price fields when the maker stamped them — but `InitiatorResult` only
        // surfaces the standard parsed bid/offer, which is exact to wire
        // precision and what the panel ranks on.
        match (result.bid, result.offer) {
            (Some(bid), Some(offer)) => Ok(Some(TwoWay { bid, offer })),
            _ => Ok(None),
        }
    }
}

impl QuoteSource for FixLpAdapter {
    fn lp_id(&self) -> &str {
        &self.cfg.lp_id
    }

    fn request<'a>(
        &'a self,
        request: &'a RfqRequest,
        _deadline: Duration,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = QuoteSourceReply> + Send + 'a>> {
        Box::pin(async move {
            match self.run_cycle(request).await {
                Ok(Some(price)) => QuoteSourceReply::Quote {
                    price,
                    epoch_nanos: self.cfg.epoch_nanos,
                    valid_until_nanos: self
                        .cfg
                        .epoch_nanos
                        .saturating_add(self.cfg.valid_for_nanos),
                },
                // A clean no-quote OR any transport/protocol error: the LP is
                // simply absent from the panel (dropped, not errored).
                Ok(None) | Err(_) => QuoteSourceReply::NoQuote,
            }
        })
    }
}

/// The 6-byte `Symbol(55)` for an RFQ's pair (e.g. `b"EURUSD"`).
fn fix_symbol(request: &RfqRequest) -> Vec<u8> {
    let mut s = Vec::with_capacity(6);
    s.extend_from_slice(request.pair.base.as_str().as_bytes());
    s.extend_from_slice(request.pair.quote.as_str().as_bytes());
    s
}

/// A currency's 3 ASCII bytes.
fn ccy_bytes(ccy: Ccy) -> Vec<u8> {
    ccy.as_str().as_bytes().to_vec()
}

/// Push a `StrikePrice(202)` with up to 8-dp fixed precision (mirrors the
/// `celnet-fix` test framing so the acceptor decodes the identical strike).
fn push_strike(e: &mut FrameEncoder, strike: f64) {
    let scaled = (strike * 1e8).round() as u128;
    let int = scaled / 100_000_000;
    let frac = scaled % 100_000_000;
    let s = format!("{int}.{frac:08}");
    e.push(202, s.as_bytes());
}
