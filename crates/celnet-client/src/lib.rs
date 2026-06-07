//! Celnet typed async client SDK — the ergonomic GUI/API-user surface over the
//! gRPC + WebSocket edge: RFQ (request→quote→accept→execution), a multiplexed RFS
//! streaming session (one connection → many subscriptions + click-to-trade),
//! surface reads/marks, and risk/scenario (point Greeks + book-shaped risk), with
//! reconnect and resync built in (work-stream WS-I/client).
//!
//! # What the SDK gives a caller
//!
//! A [`Client`] wraps the four `celnet-proto` gRPC services behind a typed,
//! futures-based API speaking the celnet domain vocabulary
//! ([`celnet_types`] + this crate's [`vocab`] / [`surface_vocab`]), never raw
//! proto. The caller works with [`InstrumentSpec`], [`Conventions`], [`Quote`],
//! [`Execution`], [`Smile`], [`ScenarioGrid`], [`BucketedRisk`] and a typed
//! [`StreamEvent`] stream — the wire `Option`-wrapped messages, sequence numbers,
//! idempotency keys, click-to-trade tokens, and the bidirectional control protocol
//! are all handled inside the SDK.
//!
//! ## RFQ — request → quote → accept
//!
//! [`Client::request_quote`] builds an [`Rfq`] handle carrying a stable
//! idempotency key (minted per client, see [`idempotency`]); a retried
//! [`Rfq::request`] returns the *same* [`Quote`], and a retried [`Rfq::accept`]
//! returns the *same* [`Execution`] — a network retry can never double-book.
//!
//! ## RFS — one multiplexed session, many subscriptions, click-to-trade
//!
//! [`Client::open_session`] opens a [`StreamSession`]: ONE bidirectional connection
//! over which [`StreamSession::subscribe`] opens any number of [`Subscription`]s —
//! a blotter watching hundreds of structures uses a single connection, not one
//! stream per line. Each subscription is a typed async [`Stream`] of
//! [`StreamEvent`]s; the SDK tracks per-subscription sequence, detects a gap and
//! auto-resyncs, recovers a `LAGGED` drop, and transparently re-dials +
//! re-subscribes every live subscription across a `DRAINING` blue-green cutover.
//! [`Subscription::execute`] click-trades *exactly* a streamed [`StreamLine`] on a
//! side — the SDK presents the maker's `tradable_token` for the caller and returns
//! a typed [`ExecuteOutcome`] (booked, or rejected by last-look).
//!
//! ## Surface & risk
//!
//! [`Client::get_smile`] / [`Client::mark_surface`] read and (re)mark the vol
//! surface from broker ATM/RR/BF quotes with an arbitrage report;
//! [`Client::mark_surface_with`] selects the calibration model ([`Calibration`] —
//! market-hedge, stochastic-vol, or a parametric family), pinned by the returned
//! `surface_version`, with the model used echoed in the smile's
//! [`ArbReport::note`](crate::ArbReport). [`Client::price`] one-shot-prices an
//! instrument; [`Client::scenario`] runs a spot/vol/rate shock grid (under a chosen
//! model via [`Client::scenario_with_model`]); [`Client::scenario_with_risk`]
//! additionally returns the book-shaped risk decomposition ([`BucketedRisk`]:
//! bucketed vega + cross-gamma + theta roll) in one round-trip — each returning
//! typed results a quant branches on directly.
//!
//! ## Market-series (TrendMode) feed
//!
//! [`StreamSession::subscribe_series`] opens a [`MarketSeries`] over the *same*
//! multiplexed session: a typed stream of one labelled [`Observable`] (ATM vol,
//! spot, a delta-wing risk reversal / butterfly, or the forward) seeded with an
//! opening [`SeriesEvent::Snapshot`] then live [`SeriesEvent::Point`]s — the trend
//! tile and its blotter share one connection.
//!
//! ## Who's-trading attribution
//!
//! A caller declares the requesting [`Seat`] / [`BookId`] via
//! [`Rfq::with_attribution`] (RFQ) or [`StreamSession::subscribe_attributed`] (RFS);
//! the server resolves the [`Attribution`] chain (the maker that quoted, the holder,
//! the won/LP-count) and echoes it on the [`Quote`] / [`Execution`] / streamed
//! snapshot so the blotter/Book stop being anonymous.
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
pub mod risk;
pub mod series;
pub mod surface_vocab;
pub mod vocab;

pub use error::{ClientError, ClientResult};
pub use rfs::{
    ClickExecution, ExecuteOutcome, RejectReason, StreamEvent, StreamLine, StreamSession,
    Subscription, TradableLine,
};
pub use risk::{
    AdditiveRisk, AggregateQuery, CcyExposure, DrillQuery, Enforcement, EntitlementScope,
    Entitlements, LimitMetric, LimitQuery, LimitStatus, LimitUtilization, NonAdditiveRisk,
    Numeraire, OrgDimension, OrgKey, PositionList, PositionQuery, Rag, RiskAggregate, RiskDrill,
    RiskNode, RiskPillar, RiskPosition, Scope, VegaLadderBucket,
};
pub use series::{MarketSeries, Observable, SeriesEvent, SeriesPoint};
pub use surface_vocab::{
    ArbReport, BrokerQuoteSet, BucketedRisk, CrossGammaTerm, MarkedSurface, MarketContext,
    RiskRequest, ScenarioGrid, ScenarioNode, ScenarioRisk, ShockAxis, ShockFactor, Smile,
    SmilePoint, VegaPillar,
};
pub use vocab::{
    AccumulatorMonitoring, AccumulatorTerms, AsianMethod, AsianTerms, Attribution, AveragingStyle,
    BarrierKind, BarrierSide, BookId, Calibration, CliquetTerms, Conventions, DigitalStyle,
    Execution, ForwardStartTerms, InstrumentSpec, Leg, LookbackMonitoring, LookbackStyle,
    LookbackTerms, PricedLine, Product, Quantity, QuantoPayoff, QuantoTerms, Quote, RejectAck,
    Seat, Side, StrategyKind, StrikeSpec, TarfRedemption, TarfTerms, TouchKind, TwoWay,
};

use celnet_proto::pricing_service_client::PricingServiceClient;
use celnet_proto::quote_service_client::QuoteServiceClient;
use celnet_proto::risk_service_client::RiskServiceClient;
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
        }
    }

    /// The underlying HTTP/2 [`Channel`] (a cheap clone — shares the connection
    /// pool). Exposed so a caller that needs a generated `celnet-proto` service
    /// client not surfaced as an ergonomic method (e.g. a server federating the
    /// `RiskService` across backend edges) can build it directly off this channel,
    /// rather than re-dialling.
    #[must_use]
    pub fn channel(&self) -> Channel {
        self.channel.clone()
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
            attribution: None,
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
            correlation_id: None,
            surface_version: None,
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
            price_std_error: resp.price_std_error,
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
    /// arbitrage report and a surface version. Calibrates under the server's default
    /// model ([`Calibration::MarketHedge`]); use [`Client::mark_surface_with`] to
    /// select a different calibration.
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
        self.mark_surface_impl(pair, broker_quotes, conventions, None)
            .await
    }

    /// Mark / recalibrate the surface for a pair (as [`Client::mark_surface`]) under
    /// an explicit calibration `model` — the market-hedge baseline, a
    /// stochastic-vol fit, or a single/surface parametric family. The returned
    /// [`MarkedSurface`] carries a fresh `surface_version` pinning the model-tagged
    /// calibration; pin a later [`Client::price`] / [`Rfq`] / stream subscription to
    /// that version to price reproducibly against this exact marked model, and read
    /// the model the server used from each smile's [`crate::ArbReport::note`] (the
    /// frozen contract has no echo field, so the note is the provenance channel).
    ///
    /// # Errors
    ///
    /// [`ClientError`] on a transport / server failure or a malformed response.
    pub async fn mark_surface_with(
        &self,
        pair: CcyPair,
        broker_quotes: &[BrokerQuoteSet],
        conventions: Conventions,
        model: Calibration,
    ) -> ClientResult<MarkedSurface> {
        self.mark_surface_impl(pair, broker_quotes, conventions, Some(model))
            .await
    }

    async fn mark_surface_impl(
        &self,
        pair: CcyPair,
        broker_quotes: &[BrokerQuoteSet],
        conventions: Conventions,
        model: Option<Calibration>,
    ) -> ClientResult<MarkedSurface> {
        let mut svc = SurfaceServiceClient::new(self.channel.clone());
        let request = MarkSurfaceRequest {
            pair: Some(celnet_proto::CcyPair::from(pair)),
            broker_quotes: broker_quotes.iter().map(|b| b.to_wire()).collect(),
            conventions: Some(conventions.to_wire()),
            // Absent ⇒ the server default (market-hedge), preserving prior behaviour.
            smile_model: model.map(vocab::calibration_to_wire),
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
        self.scenario_model(instrument, base_market, axes, conventions, None)
            .await
    }

    /// Run a scenario / what-if grid (see [`Client::scenario`]) under an explicit
    /// calibration `model`. The model is validated by the server; the grid is
    /// repriced consistently under it.
    ///
    /// # Errors
    ///
    /// [`ClientError`] on a transport / server failure or a malformed response.
    pub async fn scenario_with_model(
        &self,
        instrument: &InstrumentSpec,
        base_market: MarketContext,
        axes: &[ShockAxis],
        conventions: Conventions,
        model: Calibration,
    ) -> ClientResult<ScenarioGrid> {
        self.scenario_model(instrument, base_market, axes, conventions, Some(model))
            .await
    }

    async fn scenario_model(
        &self,
        instrument: &InstrumentSpec,
        base_market: MarketContext,
        axes: &[ShockAxis],
        conventions: Conventions,
        model: Option<Calibration>,
    ) -> ClientResult<ScenarioGrid> {
        let mut svc = SurfaceServiceClient::new(self.channel.clone());
        let request = ScenarioRequest {
            instrument: Some(instrument.to_wire()),
            base_market: Some(base_market.to_wire()),
            conventions: Some(conventions.to_wire()),
            axes: axes.iter().map(|a| a.to_wire()).collect(),
            expiry_years: instrument.expiry_years,
            risk_buckets: None,
            smile_model: model.map(vocab::calibration_to_wire),
        };
        let resp = svc.scenario(request).await?.into_inner();
        ScenarioGrid::from_wire(resp)
    }

    // ---- RFS streaming ----------------------------------------------------

    /// Open a multiplexed [`StreamSession`] over this client's connection: one
    /// bidirectional gRPC channel over which the caller opens many
    /// [`Subscription`]s and executes click-to-trade off the streamed lines. Many
    /// instruments stream over ONE connection (a blotter uses one session, not one
    /// stream per line), with gap-detection, resync, and reconnect handled by the
    /// SDK.
    ///
    /// # Errors
    ///
    /// [`ClientError`] if the bidirectional session stream cannot be opened.
    pub async fn open_session(&self) -> ClientResult<StreamSession> {
        StreamSession::open(self.channel.clone(), self.keys.clone()).await
    }

    /// Run a scenario / what-if grid (see [`Client::scenario`]) **and** the
    /// book-shaped risk decomposition (bucketed vega per `(tenor, delta)` pillar,
    /// cross-gamma per factor pair, theta roll over horizons) in one round-trip,
    /// returning both the typed [`ScenarioGrid`] and the typed [`ScenarioRisk`].
    ///
    /// # Errors
    ///
    /// [`ClientError`] on a transport / server failure or a malformed response.
    pub async fn scenario_with_risk(
        &self,
        instrument: &InstrumentSpec,
        base_market: MarketContext,
        axes: &[ShockAxis],
        risk: &RiskRequest,
        conventions: Conventions,
    ) -> ClientResult<ScenarioRisk> {
        self.scenario_with_risk_model(instrument, base_market, axes, risk, conventions, None)
            .await
    }

    /// Run a scenario grid + book-shaped risk (see [`Client::scenario_with_risk`])
    /// under an explicit calibration `model`.
    ///
    /// # Errors
    ///
    /// [`ClientError`] on a transport / server failure or a malformed response.
    pub async fn scenario_with_risk_and_model(
        &self,
        instrument: &InstrumentSpec,
        base_market: MarketContext,
        axes: &[ShockAxis],
        risk: &RiskRequest,
        conventions: Conventions,
        model: Calibration,
    ) -> ClientResult<ScenarioRisk> {
        self.scenario_with_risk_model(
            instrument,
            base_market,
            axes,
            risk,
            conventions,
            Some(model),
        )
        .await
    }

    async fn scenario_with_risk_model(
        &self,
        instrument: &InstrumentSpec,
        base_market: MarketContext,
        axes: &[ShockAxis],
        risk: &RiskRequest,
        conventions: Conventions,
        model: Option<Calibration>,
    ) -> ClientResult<ScenarioRisk> {
        let mut svc = SurfaceServiceClient::new(self.channel.clone());
        let request = ScenarioRequest {
            instrument: Some(instrument.to_wire()),
            base_market: Some(base_market.to_wire()),
            conventions: Some(conventions.to_wire()),
            axes: axes.iter().map(|a| a.to_wire()).collect(),
            expiry_years: instrument.expiry_years,
            risk_buckets: Some(risk.to_wire()),
            smile_model: model.map(vocab::calibration_to_wire),
        };
        let resp = svc.scenario(request).await?.into_inner();
        ScenarioRisk::from_wire(resp)
    }

    // ---- firm-scale hierarchical risk -------------------------------------

    /// List the open positions the server's cube aggregates — the entitled open
    /// book, each [`risk::RiskPosition`] carrying its [`risk::OrgKey`] placement and
    /// attribution. Build the [`risk::PositionQuery`] to scope the listing and/or
    /// apply an entitlement principal (omitting the principal is the grant-all
    /// show-all-now default).
    ///
    /// # Errors
    ///
    /// [`ClientError`] on a transport / server failure or a malformed response.
    pub async fn list_positions(
        &self,
        query: &risk::PositionQuery,
    ) -> ClientResult<risk::PositionList> {
        let mut svc = RiskServiceClient::new(self.channel.clone());
        let resp = svc
            .list_positions(risk::list_positions_request(query))
            .await?
            .into_inner();
        risk::position_list_from_wire(resp)
    }

    /// Request a hierarchical risk aggregate, rolled up SERVER-SIDE over an org
    /// dimension into a [`risk::RiskAggregate`] node tree of netted additive +
    /// re-derived non-additive measures in one reporting [`risk::Numeraire`]. The
    /// server prunes by the [`risk::Entitlements`] principal BEFORE the roll-up (no
    /// aggregate leakage). This is the SAME call the GUI Book view makes — the
    /// client never loops positions and sums.
    ///
    /// # Errors
    ///
    /// [`ClientError`] on a transport / server failure (e.g. `failed_precondition`
    /// for a numeraire missing a rate it needs) or a malformed response.
    pub async fn aggregate_risk(
        &self,
        query: &risk::AggregateQuery,
    ) -> ClientResult<risk::RiskAggregate> {
        let mut svc = RiskServiceClient::new(self.channel.clone());
        let resp = svc
            .aggregate_risk(risk::aggregate_request(query))
            .await?
            .into_inner();
        risk::aggregate_from_wire(resp)
    }

    /// Drill one node into its child sub-nodes (at a finer dimension) and/or its
    /// contributing positions — the Book → Risk drill — returning a
    /// [`risk::RiskDrill`]. Address the node with a [`risk::Scope`] (e.g. from
    /// [`risk::RiskNode::scope`]); opt into children / positions on the
    /// [`risk::DrillQuery`]. The drill is entitlement-pruned identically to the
    /// aggregate.
    ///
    /// # Errors
    ///
    /// [`ClientError`] on a transport / server failure or a malformed response.
    pub async fn drill_risk(&self, query: &risk::DrillQuery) -> ClientResult<risk::RiskDrill> {
        let mut svc = RiskServiceClient::new(self.channel.clone());
        let resp = svc
            .drill_risk(risk::drill_request(query))
            .await?
            .into_inner();
        risk::drill_from_wire(resp)
    }

    /// Read the limit-tree utilization + RAG for one scope node into a
    /// [`risk::LimitStatus`]: every limit configured at the [`risk::Scope`] with its
    /// cap / exposure / ratio / [`risk::Rag`], the worst status across them, and the
    /// hard-breach escalation flag. Build the [`risk::LimitQuery`] to add VaR/ES
    /// shocks for the non-additive limits.
    ///
    /// # Errors
    ///
    /// [`ClientError`] on a transport / server failure or a malformed response.
    pub async fn limit_status(&self, query: &risk::LimitQuery) -> ClientResult<risk::LimitStatus> {
        let mut svc = RiskServiceClient::new(self.channel.clone());
        let resp = svc
            .limit_status(risk::limit_request(query))
            .await?
            .into_inner();
        risk::limit_status_from_wire(resp)
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
    attribution: Option<Attribution>,
}

impl Rfq {
    /// The stable idempotency key this RFQ relays on every request / accept.
    #[must_use]
    pub fn idempotency_key(&self) -> &str {
        &self.idempotency_key
    }

    /// Declare the requesting book/seat this RFQ is sent on behalf of. The server
    /// resolves the full who's-trading chain (the maker that prices the line, the
    /// `held_by` once a trade books, the `won` flag, LP count) and echoes it on the
    /// resulting [`Quote`] and [`Execution`]. Without this, the line is sent
    /// unattributed and the server may still resolve the maker side.
    #[must_use]
    pub fn with_attribution(mut self, attribution: Attribution) -> Self {
        self.attribution = Some(attribution);
        self
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
            correlation_id: None,
            surface_version: None,
            // The requesting book/seat declared via `Rfq::with_attribution`; absent
            // ⇒ unattributed. The server resolves the full chain it echoes back.
            attribution: self.attribution.as_ref().map(Attribution::to_wire),
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
