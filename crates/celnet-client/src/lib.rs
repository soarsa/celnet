//! Celnet typed async client SDK — the ergonomic GUI/API-user surface over the
//! gRPC + WebSocket edge: RFQ (request→quote→accept→execution), RFS streaming
//! (subscribe→snapshot+sequenced deltas+resync), surface reads/marks, and
//! risk/scenario, with reconnect and resync built in (work-stream WS-I/client).
//!
//! # What the SDK gives a caller
//!
//! A [`Client`] wraps the four `celnet-proto` gRPC services behind a typed,
//! futures-based API speaking the celnet domain vocabulary
//! ([`celnet_types`] + this crate's [`vocab`] / [`surface_vocab`]), never raw
//! proto. The caller works with [`InstrumentSpec`], [`Conventions`], [`Quote`],
//! [`Execution`], [`Smile`], [`ScenarioGrid`] and a typed [`StreamEvent`] stream —
//! the wire `Option`-wrapped messages, sequence numbers, idempotency keys, and the
//! bidirectional control protocol are all handled inside the SDK.
//!
//! ## RFQ — request → quote → accept
//!
//! [`Client::request_quote`] builds an [`Rfq`] handle carrying a stable
//! idempotency key (minted per client, see [`idempotency`]); a retried
//! [`Rfq::request`] returns the *same* [`Quote`], and a retried [`Rfq::accept`]
//! returns the *same* [`Execution`] — a network retry can never double-book.
//!
//! ## RFS — subscribe → snapshot + deltas, with auto-resync / reconnect
//!
//! [`Client::subscribe`] opens a [`Subscription`]: a typed async [`Stream`] of
//! [`StreamEvent`]s. The SDK tracks the per-subscription sequence, detects a gap
//! and auto-[`Resync`](celnet_proto::Resync)s, recovers a `LAGGED` drop, and
//! transparently re-dials + re-subscribes across a `DRAINING` blue-green cutover.
//!
//! ## Surface & risk
//!
//! [`Client::get_smile`] / [`Client::mark_surface`] read and (re)mark the vol
//! surface from broker ATM/RR/BF quotes with an arbitrage report; [`Client::price`]
//! one-shot-prices an instrument; [`Client::scenario`] runs a spot/vol/rate shock
//! grid — each returning typed results a quant branches on directly.
//!
//! # Transports
//!
//! The SDK speaks the gRPC transport of the contract (HTTP/2). The WebSocket
//! tagged-JSON mirror is the *same* contract on a different framing; the SDK's
//! typed surface is transport-agnostic by construction, and the gRPC path is the
//! primary, validated one here.
//!
//! # Determinism & test hygiene
//!
//! The SDK adds no pricing logic — every numeric result is the server's
//! `celnet-core`-deterministic computation surfaced verbatim. The crate's
//! integration tests drive a real in-process [`celnet-server`](https://docs)
//! edge on an ephemeral port and assert every client result equals the underlying
//! analytics crate's direct computation; every async body is hard wall-clock
//! bounded so a regression fails fast, never hangs.

#![forbid(unsafe_code)]

mod error;
mod idempotency;
pub mod rfs;
pub mod surface_vocab;
pub mod vocab;

pub use error::{ClientError, ClientResult};
pub use rfs::{StreamEvent, StreamLine, Subscription};
pub use surface_vocab::{
    ArbReport, BrokerQuoteSet, MarkedSurface, MarketContext, ScenarioGrid, ScenarioNode, ShockAxis,
    ShockFactor, Smile, SmilePoint,
};
pub use vocab::{
    BarrierKind, BarrierSide, Conventions, DigitalStyle, Execution, InstrumentSpec, Leg,
    PricedLine, Product, Quantity, Quote, RejectAck, Side, StrategyKind, StrikeSpec, TouchKind,
    TwoWay,
};

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use celnet_proto::pricing_service_client::PricingServiceClient;
use celnet_proto::quote_service_client::QuoteServiceClient;
use celnet_proto::surface_service_client::SurfaceServiceClient;
use celnet_proto::{
    GetSmileRequest, MarkSurfaceRequest, PriceRequest, QuoteAccept, QuoteReject, QuoteRequest,
    ScenarioRequest,
};
use celnet_types::CcyPair;
use tonic::transport::{Channel, Endpoint};

use idempotency::KeyMinter;

/// The typed async Celnet client: one connection multiplexing the pricing, quote,
/// surface, and stream services.
///
/// Cheap to clone — a clone shares the underlying HTTP/2 [`Channel`] (and so the
/// connection pool) and the idempotency-key minter, so concurrent calls from
/// cloned handles are safe and never collide. Construct with [`Client::connect`].
#[derive(Debug, Clone)]
pub struct Client {
    channel: Channel,
    keys: KeyMinter,
    /// Monotonic per-client subscription id source for RFS streams.
    next_sub_id: Arc<AtomicU64>,
}

impl Client {
    /// Connect to a Celnet edge at `endpoint` (e.g. `"http://127.0.0.1:50051"`).
    ///
    /// Establishes the HTTP/2 channel eagerly so a misconfigured endpoint or an
    /// unreachable edge fails here rather than on the first RPC.
    ///
    /// # Errors
    ///
    /// [`ClientError::InvalidEndpoint`] if the URI is malformed, or
    /// [`ClientError::Transport`] if the connection cannot be established.
    pub async fn connect(endpoint: impl Into<String>) -> ClientResult<Self> {
        let endpoint = endpoint.into();
        let ep = Endpoint::from_shared(endpoint.clone())
            .map_err(|_| ClientError::InvalidEndpoint(endpoint))?;
        let channel = ep.connect().await?;
        Ok(Self::with_channel(channel))
    }

    /// Build a client over an already-connected [`Channel`] (e.g. one dialed with
    /// custom transport settings). Shares the channel's connection pool.
    #[must_use]
    pub fn with_channel(channel: Channel) -> Self {
        Self {
            channel,
            keys: KeyMinter::new(),
            next_sub_id: Arc::new(AtomicU64::new(1)),
        }
    }

    // ---- RFQ --------------------------------------------------------------

    /// Begin a request-for-quote for `instrument` under `conventions`, returning an
    /// [`Rfq`] handle that owns a stable idempotency key for the whole
    /// request→accept lifecycle. Building the handle does not yet hit the wire;
    /// call [`Rfq::request`].
    #[must_use]
    pub fn request_quote(&self, instrument: InstrumentSpec, conventions: Conventions) -> Rfq {
        Rfq {
            client: self.clone(),
            idempotency_key: self.keys.next_key(),
            instrument,
            conventions,
        }
    }

    // ---- one-shot pricing -------------------------------------------------

    /// Price one instrument against a market context, returning the full Greek set.
    ///
    /// # Errors
    ///
    /// [`ClientError`] on a transport / server failure or a malformed response.
    pub async fn price(
        &self,
        instrument: &InstrumentSpec,
        market: MarketContext,
        conventions: Conventions,
    ) -> ClientResult<PricedLine> {
        let mut svc = PricingServiceClient::new(self.channel.clone());
        let request = PriceRequest {
            request_id: 0,
            instrument: Some(instrument.to_wire()),
            market: Some(market.to_wire()),
            conventions: Some(conventions.to_wire()),
        };
        let resp = svc.price(request).await?.into_inner();
        let greeks = resp
            .greeks
            .as_ref()
            .map(vocab::greeks_from_wire)
            .ok_or(ClientError::MissingField("PriceResponse.greeks"))?;
        let conventions = resp
            .conventions
            .as_ref()
            .ok_or(ClientError::MissingField("PriceResponse.conventions"))
            .and_then(Conventions::from_wire)?;
        Ok(PricedLine {
            greeks,
            resolved_strike: resp.resolved_strike,
            conventions,
        })
    }

    // ---- surface ----------------------------------------------------------

    /// Read the calibrated delta-axis smile for one `(pair, tenor)` from the
    /// maker's live market.
    ///
    /// # Errors
    ///
    /// [`ClientError`] on a transport / server failure or a malformed response.
    pub async fn get_smile(
        &self,
        pair: CcyPair,
        tenor_years: f64,
        conventions: Conventions,
    ) -> ClientResult<Smile> {
        let mut svc = SurfaceServiceClient::new(self.channel.clone());
        let request = GetSmileRequest {
            pair: Some(celnet_proto::CcyPair::from(pair)),
            tenor_years,
            conventions: Some(conventions.to_wire()),
        };
        let resp = svc.get_smile(request).await?.into_inner();
        Smile::from_wire(resp)
    }

    /// Mark / recalibrate the surface for a pair from a set of per-tenor broker
    /// quote sets (ATM + 25Δ/10Δ RR/BF), returning the calibrated smiles + an
    /// arbitrage report and a surface version.
    ///
    /// # Errors
    ///
    /// [`ClientError`] on a transport / server failure or a malformed response.
    pub async fn mark_surface(
        &self,
        pair: CcyPair,
        broker_quotes: &[BrokerQuoteSet],
        conventions: Conventions,
    ) -> ClientResult<MarkedSurface> {
        let mut svc = SurfaceServiceClient::new(self.channel.clone());
        let request = MarkSurfaceRequest {
            pair: Some(celnet_proto::CcyPair::from(pair)),
            broker_quotes: broker_quotes.iter().map(|b| b.to_wire()).collect(),
            conventions: Some(conventions.to_wire()),
        };
        let resp = svc.mark_surface(request).await?.into_inner();
        MarkedSurface::from_wire(resp)
    }

    /// Run a scenario / what-if grid: reprice `instrument` across the Cartesian
    /// product of `axes` applied to `base_market`.
    ///
    /// # Errors
    ///
    /// [`ClientError`] on a transport / server failure or a malformed response.
    pub async fn scenario(
        &self,
        instrument: &InstrumentSpec,
        base_market: MarketContext,
        axes: &[ShockAxis],
        conventions: Conventions,
    ) -> ClientResult<ScenarioGrid> {
        let mut svc = SurfaceServiceClient::new(self.channel.clone());
        let request = ScenarioRequest {
            instrument: Some(instrument.to_wire()),
            base_market: Some(base_market.to_wire()),
            conventions: Some(conventions.to_wire()),
            axes: axes.iter().map(|a| a.to_wire()).collect(),
        };
        let resp = svc.scenario(request).await?.into_inner();
        ScenarioGrid::from_wire(resp)
    }

    // ---- RFS streaming ----------------------------------------------------

    /// Open a request-for-stream subscription for `instrument`, returning a typed
    /// [`Subscription`] stream of [`StreamEvent`]s with gap-detection, resync, and
    /// reconnect handled by the SDK. The subscription id is minted per client.
    ///
    /// # Errors
    ///
    /// [`ClientError`] if the bidirectional stream cannot be opened.
    pub async fn subscribe(
        &self,
        instrument: InstrumentSpec,
        conventions: Conventions,
    ) -> ClientResult<Subscription> {
        let sub_id = self.next_sub_id.fetch_add(1, Ordering::Relaxed);
        Subscription::open(self.channel.clone(), sub_id, instrument, conventions).await
    }
}

/// A request-for-quote handle: owns a stable idempotency key so the whole
/// request→accept lifecycle is safe to retry.
///
/// A retried [`Rfq::request`] returns the same [`Quote`] (id + prices), and a
/// retried [`Rfq::accept`] returns the same [`Execution`] — the SDK relays the
/// caller's single key on every call, so the maker deduplicates transparently.
#[derive(Debug, Clone)]
pub struct Rfq {
    client: Client,
    idempotency_key: String,
    instrument: InstrumentSpec,
    conventions: Conventions,
}

impl Rfq {
    /// The stable idempotency key this RFQ relays on every request / accept.
    #[must_use]
    pub fn idempotency_key(&self) -> &str {
        &self.idempotency_key
    }

    /// Request the tradable two-way quote. Idempotent: a retry under this handle
    /// returns the same [`Quote`], never re-pricing.
    ///
    /// # Errors
    ///
    /// [`ClientError`] on a transport / server failure or a malformed quote.
    pub async fn request(&self) -> ClientResult<Quote> {
        let mut svc = QuoteServiceClient::new(self.client.channel.clone());
        let request = QuoteRequest {
            idempotency_key: self.idempotency_key.clone(),
            instrument: Some(self.instrument.to_wire()),
            conventions: Some(self.conventions.to_wire()),
        };
        let resp = svc.request_quote(request).await?.into_inner();
        Quote::from_wire(resp)
    }

    /// Accept a previously issued [`Quote`] on `side` (`Buy` lifts the offer,
    /// `Sell` hits the bid) and book the execution. Idempotent: a retry returns the
    /// same [`Execution`], never double-booking.
    ///
    /// # Errors
    ///
    /// [`ClientError::Status`] (`deadline_exceeded`) if the quote's last-look
    /// window has expired, `not_found` for an unknown id, plus transport failures.
    pub async fn accept(&self, quote: &Quote, side: Side) -> ClientResult<Execution> {
        let mut svc = QuoteServiceClient::new(self.client.channel.clone());
        let request = QuoteAccept {
            quote_id: quote.quote_id,
            idempotency_key: self.idempotency_key.clone(),
            side: side.to_wire() as i32,
        };
        let resp = svc.accept_quote(request).await?.into_inner();
        Execution::from_wire(resp)
    }

    /// Reject a previously issued [`Quote`] (decline to trade), with a free-text
    /// `reason` for the audit trail. Returns a typed [`RejectAck`]; after it, the
    /// quote can no longer be accepted.
    ///
    /// # Errors
    ///
    /// [`ClientError`] on a transport / server failure or if the quote already
    /// booked.
    pub async fn reject(
        &self,
        quote: &Quote,
        reason: impl Into<String>,
    ) -> ClientResult<RejectAck> {
        let mut svc = QuoteServiceClient::new(self.client.channel.clone());
        let request = QuoteReject {
            quote_id: quote.quote_id,
            reason: reason.into(),
        };
        let resp = svc.reject_quote(request).await?.into_inner();
        Ok(RejectAck::from_wire(resp))
    }
}
