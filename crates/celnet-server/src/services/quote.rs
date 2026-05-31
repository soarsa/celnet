//! The RFQ lifecycle service: `RequestQuote` → `Quote`, `AcceptQuote` →
//! `Execution`, `RejectQuote` → `RejectAck`.
//!
//! This is the request-for-quote half of the trader workflow. A client sends a
//! [`celnet_proto::QuoteRequest`] carrying an [`celnet_proto::Instrument`], a
//! [`celnet_proto::Conventions`], and a client-supplied **idempotency key**; the
//! service:
//!
//! 1. prices the instrument through [`crate::pricer`] against the instrument's
//!    market context (carried in the request via the edge's published market
//!    state — see below), producing a mid and the full Greek set;
//! 2. turns the mid into a tradable two-way bid/offer via the [`crate::spread`]
//!    model;
//! 3. assigns a stable, **unguessable** `quote_id` (a fresh monotonic counter
//!    folded through a per-process-keyed `splitmix64` bijection — unique but not
//!    enumerable), stamps a publication time and a `valid_until` last-look
//!    deadline, and stores the quote keyed by both `quote_id` and the client
//!    idempotency key.
//!
//! # Idempotency
//!
//! The idempotency key makes a retried request safe: a second `RequestQuote`
//! carrying a key already seen returns the **same** stored [`celnet_proto::Quote`]
//! (same `quote_id`, same prices), never re-pricing and never issuing a new id. A
//! retried `AcceptQuote` on an already-booked quote returns the **same**
//! [`celnet_proto::Execution`], so a network retry can never double-book.
//!
//! # Trust model (request-matched accept / unguessable id)
//!
//! `quote_id` is **not** an authority token. `AcceptQuote` is request-matched to
//! the originating idempotency key: an accept whose `idempotency_key` does not
//! equal the key the quote was minted under is refused with `invalid_argument`, so
//! a party that merely learns a `quote_id` cannot accept (or hijack the booking
//! of) another requester's live quote. An accept-retry returns the stored
//! execution only when both the key **and** the traded side match; a side flip is
//! a different intent and is refused with `failed_precondition`.
//!
//! `RejectQuote` carries no key on the wire (the contract has only `quote_id` +
//! free-text `reason`), so it cannot be request-matched the same way. Instead,
//! every `quote_id` is **unguessable** — minted by folding a strictly-fresh
//! monotonic counter through a `splitmix64` bijection keyed by a per-process
//! secret — so the id space is sparse and unenumerable: only a party that received
//! the quote (and thus holds its id) can decline it. A re-reject is idempotent.
//!
//! # Market context
//!
//! An RFQ does not carry a `MarketContext` on the wire (the request is a trader
//! intent, not a market snapshot); the maker prices it against its own live
//! market. The service reads the engine's live [`celnet_engine::MarketState`]
//! through the [`CoreLink`] and projects it into the
//! [`celnet_proto::MarketContext`] the pricer consumes, so a quote always reflects
//! the maker's current market.

#![allow(clippy::result_large_err)]

use std::collections::HashMap;
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::tick::TickSource;

use celnet_proto::quote_service_server::QuoteService;
use celnet_proto::{
    Execution, MarketContext, Quote, QuoteAccept, QuoteReject, QuoteRequest, RejectAck, Side,
    TwoWayPrice,
};
use tokio::sync::Mutex;
use tonic::{Request, Response, Status};

use crate::clock::Clock;
use crate::core_link::CoreLink;
use crate::pricer::{ConventionSet, price_instrument};
use crate::readiness::ReadinessGate;
use crate::services::pin::{PinnedVol, resolve_pinned_vol};
use crate::spread::SpreadModel;
use crate::surface_book::SurfaceBook;

/// How long an issued quote stays valid for a last-look accept, in nanoseconds
/// (5 seconds — the typical OTC last-look window).
const QUOTE_VALIDITY_NANOS: i64 = 5_000_000_000;

/// A booked, immutable quote record kept for idempotent re-request and accept.
#[derive(Debug, Clone)]
struct QuoteRecord {
    quote: Quote,
    /// The instrument the quote priced (echoed into the booking on accept).
    instrument: celnet_proto::Instrument,
    /// The execution booked from this quote, if it has been accepted. Storing it
    /// makes `AcceptQuote` idempotent: a retry returns the same booking.
    execution: Option<Execution>,
    /// Whether the quote has been declined (rejected). A rejected quote can no
    /// longer be accepted; a re-reject is idempotent.
    rejected: bool,
}

/// The in-memory RFQ store: idempotency-key → quote_id, and quote_id → record.
///
/// Bounded only by the RFQ rate over a deployment lifetime (a production edge
/// expires old quotes; the records here are small and the test workload is
/// finite). Guarded by an async [`Mutex`] held only for the brief map operations —
/// never across a price call.
#[derive(Debug, Default)]
struct QuoteStore {
    by_key: HashMap<String, u64>,
    by_id: HashMap<u64, QuoteRecord>,
}

/// The RFQ service over the [`CoreLink`] and the readiness gate.
#[derive(Debug)]
pub struct QuoteEdge {
    link: Arc<CoreLink>,
    gate: Arc<ReadinessGate>,
    spread: SpreadModel,
    clock: Clock,
    next_quote_id: AtomicU64,
    next_execution_id: AtomicU64,
    /// A per-process secret folded into every minted `quote_id` so ids are
    /// **unguessable** (not a dense monotonic sequence a third party can
    /// enumerate). Combined with the idempotency-key match on accept, this is the
    /// in-band trust model: `quote_id` alone is not an authority token — accepting
    /// requires the originating key, and rejecting requires possession of the
    /// (unguessable) id. Seeded from the OS-random [`RandomState`], never the
    /// pricing path, so it does not touch the deterministic core.
    quote_id_secret: u64,
    store: Mutex<QuoteStore>,
    /// The shared versioned marked-surface registry: a `QuoteRequest` carrying a
    /// pinned `surface_version` prices against the marked surface (and echoes it).
    surface_book: Arc<SurfaceBook>,
}

impl QuoteEdge {
    /// Construct the RFQ service.
    #[must_use]
    pub fn new(
        link: Arc<CoreLink>,
        gate: Arc<ReadinessGate>,
        spread: SpreadModel,
        clock: Clock,
        surface_book: Arc<SurfaceBook>,
    ) -> Self {
        // Mint a process-unique unguessable secret from the OS-seeded hasher. This
        // is a control-plane identity concern, *not* a pricing path, so OS entropy
        // is appropriate (the determinism / counter-RNG guardrail governs pricing,
        // which never consumes this value).
        let quote_id_secret = RandomState::new().build_hasher().finish();
        Self {
            link,
            gate,
            spread,
            clock,
            next_quote_id: AtomicU64::new(1),
            next_execution_id: AtomicU64::new(1),
            quote_id_secret,
            store: Mutex::new(QuoteStore::default()),
            surface_book,
        }
    }

    /// Mint the next **unguessable** `quote_id`: a strictly-fresh monotonic counter
    /// folded through the public-domain `splitmix64` bijection keyed by the
    /// per-process secret. `splitmix64` is a bijection over `u64`, so distinct
    /// counters yield distinct ids (no collision) while the secret makes the id
    /// space sparse and unenumerable. The lone counter that would map to `0` is
    /// skipped so every id is `>= 1` (a valid, non-sentinel quote id).
    fn mint_quote_id(&self) -> u64 {
        loop {
            let n = self.next_quote_id.fetch_add(1, Ordering::Relaxed);
            let id = TickSource::splitmix64(self.quote_id_secret ^ n);
            if id != 0 {
                return id;
            }
        }
    }

    /// Reject the call if the edge is not accepting traffic.
    fn require_ready(&self) -> Result<(), Status> {
        if self.gate.is_ready() {
            Ok(())
        } else {
            Err(Status::unavailable(
                "edge not ready (starting or draining); steer to the active instance",
            ))
        }
    }

    /// Project the live engine market state into the wire market context the
    /// pricer consumes.
    async fn live_market(&self) -> Result<MarketContext, Status> {
        let snap = self
            .link
            .market_snapshot()
            .await
            .map_err(|e| Status::unavailable(e.to_string()))?;
        Ok(MarketContext {
            spot: snap.spot,
            vol: snap.atm_vol,
            r_dom: snap.r_dom,
            r_for: snap.r_for,
        })
    }
}

/// Does an incoming request match the request the stored idempotency record was
/// minted for? An idempotency key is only honoured when replayed with the *same*
/// instrument and conventions; a key reused for a different payload is a collision.
fn idempotency_record_matches(
    rec: &QuoteRecord,
    instrument: Option<&celnet_proto::Instrument>,
    conventions: Option<&celnet_proto::Conventions>,
) -> bool {
    instrument == Some(&rec.instrument) && conventions == rec.quote.conventions.as_ref()
}

/// The status returned when an idempotency key is reused for a different request
/// payload (a key-collision) — `invalid_argument`, so the client learns the key
/// is ambiguous rather than silently receiving a stale, mismatched quote.
fn idempotency_conflict(key: &str) -> Status {
    Status::invalid_argument(format!(
        "idempotency key {key:?} was already used for a different request \
         (instrument/conventions mismatch); reuse a key only for an identical retry"
    ))
}

/// The status returned when an `AcceptQuote` carries an idempotency key that does
/// not match the key the quote was minted under — `invalid_argument`. The accept
/// must be request-matched to the originating key; a `quote_id` alone is not an
/// authority token, so a party that merely learns the id cannot accept another
/// requester's quote. The key value is never echoed (avoid leaking the secret).
fn accept_key_mismatch(quote_id: u64) -> Status {
    Status::invalid_argument(format!(
        "accept for quote {quote_id} did not carry the originating idempotency key; \
         an accept must be request-matched to the key the quote was issued under"
    ))
}

#[tonic::async_trait]
impl QuoteService for QuoteEdge {
    async fn request_quote(
        &self,
        request: Request<QuoteRequest>,
    ) -> Result<Response<Quote>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();

        // Idempotency: a key already seen returns the stored quote verbatim — but
        // only when the request payload (instrument + conventions) matches the one
        // the key was minted for. Reusing a key for a *different* instrument is a
        // client error (a key-collision), answered with `invalid_argument` rather
        // than the mismatched stale quote.
        if !req.idempotency_key.is_empty() {
            let store = self.store.lock().await;
            if let Some(&qid) = store.by_key.get(&req.idempotency_key)
                && let Some(rec) = store.by_id.get(&qid)
            {
                if idempotency_record_matches(
                    rec,
                    req.instrument.as_ref(),
                    req.conventions.as_ref(),
                ) {
                    return Ok(Response::new(rec.quote.clone()));
                }
                return Err(idempotency_conflict(&req.idempotency_key));
            }
        }

        let instrument = req
            .instrument
            .ok_or_else(|| Status::invalid_argument("missing `instrument`"))?;
        let wire_conv = req
            .conventions
            .ok_or_else(|| Status::invalid_argument("missing `conventions`"))?;
        let conv = ConventionSet::decode(&wire_conv)
            .map_err(|e| Status::invalid_argument(e.to_string()))?;

        let market = self.live_market().await?;
        // Resolve the optional pinned `surface_version`: an honoured pin prices the
        // quote against the marked surface and is echoed on the `Quote`; an unknown
        // pinned version is refused (the pin cannot be honoured).
        let PinnedVol {
            market: effective_market,
            echo_version,
        } = resolve_pinned_vol(
            &self.surface_book,
            req.surface_version,
            &instrument,
            &market,
        )?;
        let priced = price_instrument(&instrument, &effective_market, &conv)
            .map_err(|e| Status::invalid_argument(e.to_string()))?;

        let two_way = self.spread.two_way(priced.greeks.price, &priced.greeks);

        let now = self.clock.now_nanos();
        let quote_id = self.mint_quote_id();
        let quote = Quote {
            quote_id,
            idempotency_key: req.idempotency_key.clone(),
            price: Some(two_way),
            greeks: Some(priced.greeks.into()),
            conventions: Some(wire_conv),
            resolved_strike: priced.resolved_strike,
            epoch_nanos: now,
            valid_until_nanos: now + QUOTE_VALIDITY_NANOS,
            correlation_id: req.correlation_id,
            surface_version: echo_version,
            // Stamp the who's-trading chain: the maker auto-pricer is the
            // `quoted_by` (the engine priced and showed this line); the client's
            // requesting seat, when supplied, becomes the `held_by`. So the flow is
            // attributable request→quote→booking and never anonymous.
            attribution: Some(super::attribution::resolve(req.attribution.as_ref())),
        };

        // Store under both keys (id always; idempotency key when present).
        {
            let mut store = self.store.lock().await;
            // Re-check the key under the write lock to win a concurrent first-quote
            // race: if another caller stored the same key meanwhile, return theirs
            // (or reject a colliding key reused for a different request payload).
            if !req.idempotency_key.is_empty()
                && let Some(&qid) = store.by_key.get(&req.idempotency_key)
                && let Some(rec) = store.by_id.get(&qid)
            {
                if idempotency_record_matches(rec, Some(&instrument), Some(&wire_conv)) {
                    return Ok(Response::new(rec.quote.clone()));
                }
                return Err(idempotency_conflict(&req.idempotency_key));
            }
            store.by_id.insert(
                quote_id,
                QuoteRecord {
                    quote: quote.clone(),
                    instrument,
                    execution: None,
                    rejected: false,
                },
            );
            if !req.idempotency_key.is_empty() {
                store.by_key.insert(req.idempotency_key.clone(), quote_id);
            }
        }

        Ok(Response::new(quote))
    }

    async fn accept_quote(
        &self,
        request: Request<QuoteAccept>,
    ) -> Result<Response<Execution>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let acc = request.into_inner();

        let mut store = self.store.lock().await;
        let rec = store
            .by_id
            .get(&acc.quote_id)
            .ok_or_else(|| Status::not_found(format!("unknown quote_id {}", acc.quote_id)))?
            .clone();

        // Request-matched idempotency: an accept must carry the *same* idempotency
        // key the quote was minted under. The `quote_id` alone is not an authority
        // token; binding the accept to the originating key stops any party that
        // merely learns a `quote_id` from accepting another requester's live quote,
        // and makes an accept-retry safe only for the genuine originator. (Mirrors
        // the request-matched contract `RequestQuote` already enforces.)
        if acc.idempotency_key != rec.quote.idempotency_key {
            return Err(accept_key_mismatch(acc.quote_id));
        }

        // Idempotent accept: a retry returns the already-booked execution — but
        // only when the retry's traded side matches the booked side. A second
        // accept that flips the side is a different trade intent, not a retry, so
        // it is refused (`failed_precondition`) rather than handed a booking it did
        // not ask for. The key already matched above.
        if let Some(exec) = &rec.execution {
            let retry_side = Side::try_from(acc.side).unwrap_or(Side::Buy);
            let booked_side = Side::try_from(exec.side).unwrap_or(Side::Buy);
            let effective_retry_side = if retry_side == Side::Sell {
                Side::Sell
            } else {
                Side::Buy
            };
            if effective_retry_side != booked_side {
                return Err(Status::failed_precondition(format!(
                    "quote {} already booked on side {booked_side:?}; \
                     accept-retry requested side {effective_retry_side:?}",
                    acc.quote_id
                )));
            }
            return Ok(Response::new(exec.clone()));
        }

        // A declined (rejected) quote can no longer be accepted.
        if rec.rejected {
            return Err(Status::failed_precondition(format!(
                "quote {} was rejected; cannot accept",
                acc.quote_id
            )));
        }

        // Last-look: an accept after the validity deadline is rejected as expired.
        let now = self.clock.now_nanos();
        if now > rec.quote.valid_until_nanos {
            return Err(Status::deadline_exceeded(format!(
                "quote {} expired at {} (now {now})",
                acc.quote_id, rec.quote.valid_until_nanos
            )));
        }

        // Resolve the traded side and the lifted/hit premium of the two-way.
        let side = Side::try_from(acc.side).unwrap_or(Side::Buy);
        let price = rec.quote.price.as_ref().copied().unwrap_or(TwoWayPrice {
            bid: 0.0,
            offer: 0.0,
        });
        // BUY (or two-way default) lifts the offer; SELL hits the bid.
        let (traded_side, traded_premium) = match side {
            Side::Sell => (Side::Sell, price.bid),
            _ => (Side::Buy, price.offer),
        };

        let execution_id = self.next_execution_id.fetch_add(1, Ordering::Relaxed);
        let execution = Execution {
            execution_id,
            quote_id: acc.quote_id,
            side: traded_side as i32,
            traded_premium,
            instrument: Some(rec.instrument.clone()),
            epoch_nanos: now,
            // Carry the quote's attribution chain onto the booking.
            attribution: rec.quote.attribution.clone(),
        };

        // Book it back into the record so a retry is idempotent.
        if let Some(stored) = store.by_id.get_mut(&acc.quote_id) {
            stored.execution = Some(execution.clone());
        }

        Ok(Response::new(execution))
    }

    async fn reject_quote(
        &self,
        request: Request<QuoteReject>,
    ) -> Result<Response<RejectAck>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let rej = request.into_inner();

        let mut store = self.store.lock().await;
        // The reject is authorised by possession of the (unguessable) quote_id —
        // see the module "Trust model" doc: the id space is sparse/unenumerable, so
        // only a party that received the quote can decline it. A reject on an
        // unknown quote is a client error; a reject on a booked quote is refused (it
        // already traded).
        let rec = store
            .by_id
            .get_mut(&rej.quote_id)
            .ok_or_else(|| Status::not_found(format!("unknown quote_id {}", rej.quote_id)))?;
        if rec.execution.is_some() {
            return Err(Status::failed_precondition(format!(
                "quote {} already booked; cannot reject",
                rej.quote_id
            )));
        }
        // Mark the quote declined so it can no longer be accepted. A re-reject is
        // idempotent (the flag is already set). A reject never books a trade, so
        // the contract returns a purpose-typed acknowledgement, not an execution.
        rec.rejected = true;
        Ok(Response::new(RejectAck {
            quote_id: rej.quote_id,
            epoch_nanos: self.clock.now_nanos(),
        }))
    }
}
