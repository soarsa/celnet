//! The dealer-side **RFQ/IOI quoting desk**: inbound request capture, the desk's
//! firm response (quote or reject), quote acceptance (which books a [`Deal`] **and** a
//! [`RatesPosition`]), the desk inbox + received-deals reads, and the dedicated
//! server→client [`NotificationService`] push stream.
//!
//! # Where it sits
//!
//! This is the **human-quoted** complement to the synthetic-LP auto-pricer
//! ([`QuoteService::RequestQuote`](crate::services::quote)): a counterparty (the taker
//! SDK / simulator) submits an RFQ/IOI over the one current contract; the desk learns
//! of it instantly over the [`NotificationBroker`] push channel, responds with a firm
//! [`DeskQuote`] (or a [`DeskReject`]), and the counterparty lifts the price — booking
//! a received deal into the [`DealStore`] blotter and a rates position into the shared
//! [`RatesPositionStore`] the Book workspace reads.
//!
//! # Pricing reuse (CLAUDE.md rule 2/8: no re-implementation, vendor-neutral)
//!
//! A submitted request is **priced through the exact same entry**
//! ([`crate::rates_pricing::price_rates`]) `PricingService::PriceRates` and
//! `RiskService::AggregateRatesRisk` use — never a second copy of OIS math. Submission
//! validates the request is priceable against its `CurveSet` before it enters the
//! inbox, so a desk is never shown an unpriceable RFQ.
//!
//! # Auth (deny-by-default, mirrored from the rates reads)
//!
//! Every RPC threads `session_token` + `principal` through the same boundary the
//! `RiskService` rates RPCs use ([`resolve_caller`] + [`authorize_caller`] with
//! [`RequiredAuthority::ReadAny`]): an absent principal under
//! [`AccessMode::Enforce`](celnet_entitlements::AccessMode) is denied by default;
//! permissive dev-mode admits it (audited). The desk inbox / deals / notifications are
//! then **entitlement-pruned to the desks the caller may see** via the session's
//! [`DeskScope`] — exactly as [`FixAdminService`](crate::services::fix_admin) prunes
//! desk-owned connections.

// `tonic::Status` is the contract's typed error; its size is the wire library's
// choice (the same allowance every service module carries).
#![allow(clippy::result_large_err)]

pub mod notify;
pub mod store;

use std::sync::Arc;

use celnet_proto::rfq_desk_service_server::RfqDeskService;
use celnet_proto::{
    AcceptDeskQuoteRequest, AcceptDeskQuoteResponse, CurveSet, Deal, DeskQuote, DeskRequest,
    DeskRequestKind, DeskRequestState, ListDealsRequest, ListDealsResponse,
    ListDeskRequestsRequest, ListDeskRequestsResponse, Notification, NotificationKind,
    OisInstrument, RatesInstrument, RatesPosition, RespondDeskRequestRequest,
    RespondDeskRequestResponse, Side, SubmitDeskRequestRequest, SubmitDeskRequestResponse,
    rates_instrument, respond_desk_request_request::Response as RespondArm,
};
use tonic::{Request, Response, Status};

use crate::clock::Clock;
use crate::rates_pricing::{RatesPriceError, price_rates};
use crate::readiness::ReadinessGate;
use celnet_entitlements::{Action, AssetClass};

use crate::services::access::{DeskScope, RequiredAuthority, authorize_caller, resolve_caller};
use crate::services::rates_book::RatesPositionStore;
use crate::services::risk::store::PositionStore;
use crate::services::sessions::SessionRegistry;
use celnet_proto::RatesPricingResult;

use notify::{DeskFilter, NotificationBroker};
use store::{DealStore, DeskRequestStore};

use celnet_proto::RatesPriceRequest;

/// The default time-to-live (ms) granted to a submitted request when the caller
/// supplies `ttl_ms == 0`: two minutes, a generous desk-response window.
const DEFAULT_TTL_MS: u32 = 120_000;

/// The `RfqDeskService` edge over the shared desk inbox, received-deals blotter,
/// rates position book, and notification broker. Cheap to clone behind an [`Arc`].
#[derive(Debug)]
pub struct RfqDeskEdge {
    /// The FX position store — consulted **only** for its shared entitlements
    /// [`AccessMode`](celnet_entitlements::AccessMode) (one coherent policy across
    /// every edge), exactly as [`FixAdminService`](crate::services::fix_admin) does.
    access_store: Arc<PositionStore>,
    /// The edge-wide session registry (server-validated identities).
    sessions: Arc<SessionRegistry>,
    /// The readiness gate (`/readyz` + drain): every RPC enters it.
    gate: Arc<ReadinessGate>,
    /// The desk inbox.
    requests: Arc<DeskRequestStore>,
    /// The received-deals blotter.
    deals: Arc<DealStore>,
    /// The shared rates position book an accepted quote books into.
    rates: Arc<RatesPositionStore>,
    /// The notification push broker.
    notify: Arc<NotificationBroker>,
    /// The edge clock (receipt / execution timestamps), manual in tests.
    clock: Clock,
}

impl RfqDeskEdge {
    /// Construct the desk edge over its shared components.
    #[must_use]
    #[allow(clippy::too_many_arguments)] // the shared component set the edge is built from.
    pub fn new(
        access_store: Arc<PositionStore>,
        sessions: Arc<SessionRegistry>,
        gate: Arc<ReadinessGate>,
        requests: Arc<DeskRequestStore>,
        deals: Arc<DealStore>,
        rates: Arc<RatesPositionStore>,
        notify: Arc<NotificationBroker>,
        clock: Clock,
    ) -> Self {
        Self {
            access_store,
            sessions,
            gate,
            requests,
            deals,
            rates,
            notify,
            clock,
        }
    }

    /// The shared notification broker (so the WS connection layer registers
    /// subscribers on the same instance the publishers fan out over).
    #[must_use]
    pub fn broker(&self) -> &Arc<NotificationBroker> {
        &self.notify
    }

    /// The edge-wide session registry (so the WS notification subscribe path resolves
    /// the caller against the same sessions).
    #[must_use]
    pub fn sessions(&self) -> &Arc<SessionRegistry> {
        &self.sessions
    }

    /// The shared entitlements access mode (so the WS notification subscribe path
    /// gates with the same policy).
    #[must_use]
    pub fn access_mode(&self) -> celnet_entitlements::AccessMode {
        self.access_store.access_mode()
    }

    fn require_ready(&self) -> Result<(), Status> {
        if self.gate.is_ready() {
            Ok(())
        } else {
            Err(Status::unavailable(
                "edge is starting or draining — retry after readiness",
            ))
        }
    }

    /// Resolve + authorize a notification subscriber and register it on the broker,
    /// returning the bounded [`Subscription`](notify::Subscription). Shared by the
    /// gRPC `StreamNotifications` handler and the WS `subscribe_notifications` frame
    /// so both fronts apply the IDENTICAL deny-by-default boundary + desk-scope
    /// intersection over one broker.
    ///
    /// # Errors
    /// [`Status::unauthenticated`] / [`Status::permission_denied`] /
    /// [`Status::invalid_argument`] from the shared authorization boundary.
    pub fn subscribe_notifications(
        &self,
        req: &celnet_proto::StreamNotificationsRequest,
    ) -> Result<notify::Subscription, Status> {
        let caller = resolve_caller(
            &self.sessions,
            req.session_token.as_deref(),
            req.principal.clone(),
        )?;
        authorize_caller(
            self.access_store.access_mode(),
            &caller,
            "NotificationService/StreamNotifications",
            RequiredAuthority::ReadAny,
            None,
        )?;
        let requested = req
            .scope
            .as_ref()
            .map(|s| s.desks.clone())
            .unwrap_or_default();
        let filter = effective_desk_filter(&requested, &caller.desk_scope());
        Ok(self.notify.subscribe(filter))
    }
}

/// Price a desk request's instrument against its curve set through the shared
/// `price_rates` entry — the SAME math `PriceRates` runs. Pure (the market is the
/// request-supplied `CurveSet`).
///
/// # Errors
/// `invalid_argument` for a missing/malformed instrument or curve; `internal` for a
/// numeric bootstrap failure (mirrors `PricingService::PriceRates`).
pub fn price_desk_request(
    instrument: Option<&RatesInstrument>,
    curve_set: Option<&CurveSet>,
) -> Result<RatesPricingResult, Status> {
    let req = RatesPriceRequest {
        request_id: 0,
        curve_set: curve_set.cloned(),
        instrument: instrument.cloned(),
        correlation_id: None,
    };
    price_rates(&req).map_err(|e| match e {
        RatesPriceError::Bootstrap(_) => Status::internal(e.to_string()),
        _ => Status::invalid_argument(e.to_string()),
    })
}

/// The OIS arm of a rates instrument, if present.
fn ois_of(instrument: Option<&RatesInstrument>) -> Option<&OisInstrument> {
    match instrument.and_then(|i| i.instrument.as_ref()) {
        Some(rates_instrument::Instrument::Ois(ois)) => Some(ois),
        None => None,
    }
}

/// The desk's side of a deal: the opposite of the counterparty's submitted
/// instrument direction. `Buy` (pay fixed) ⇄ `Sell` (receive fixed). A two-way
/// instrument never reaches booking (`price_rates` rejects it at submit), so this is
/// total over the firm directions.
fn opposite_side(side: Side) -> Side {
    match side {
        Side::Buy => Side::Sell,
        Side::Sell => Side::Buy,
        // Defensive: a two-way instrument is rejected at submit, so booking only ever
        // sees a firm side. Treat any residual two-way as buy-fixed by convention.
        Side::TwoWay => Side::Buy,
    }
}

/// Map a caller's session [`DeskScope`] + a requested desk filter into the effective
/// per-subscriber [`DeskFilter`] for the notification stream (intersection).
#[must_use]
pub fn effective_desk_filter(requested: &[String], caller_scope: &DeskScope) -> DeskFilter {
    match caller_scope {
        DeskScope::All => {
            if requested.is_empty() {
                DeskFilter::All
            } else {
                DeskFilter::Desks(requested.iter().cloned().collect())
            }
        }
        DeskScope::Desk(d) => {
            // The caller may see only their desk; intersect with any requested set.
            let admit = requested.is_empty() || requested.iter().any(|r| r == d);
            if admit {
                DeskFilter::Desks([d.clone()].into_iter().collect())
            } else {
                DeskFilter::Desks(std::collections::HashSet::new())
            }
        }
        // A deskless trader: notifications always target a named desk, so none match.
        DeskScope::Deskless => DeskFilter::Desks(std::collections::HashSet::new()),
    }
}

/// Whether a desk-owned row (`row_desk`) is visible to a caller under `caller_scope`,
/// optionally narrowed to one requested `desk`.
fn desk_visible(row_desk: &str, caller_scope: &DeskScope, requested: Option<&str>) -> bool {
    caller_scope.allows(row_desk) && requested.is_none_or(|d| d == row_desk)
}

impl RfqDeskEdge {
    /// Build + publish a notification for a desk lifecycle event (off the hot path).
    fn publish_notification(
        &self,
        kind: NotificationKind,
        request: &DeskRequest,
        headline: String,
        detail: Option<String>,
    ) {
        let notification = Notification {
            notification_id: self.notify_id(),
            kind: kind as i32,
            at_nanos: self.clock.now_nanos(),
            request_id: Some(request.request_id.clone()),
            desk: request.desk.clone(),
            counterparty: request.counterparty.clone(),
            request_kind: request.kind,
            headline,
            detail,
        };
        self.notify.publish(&notification);
    }

    /// Ingest an externally-originated (FIX venue) rates RFQ into the desk inbox
    /// **without** the RPC authorization boundary: a managed FIX acceptor has already
    /// authenticated the counterparty at the transport (CompID) layer, so this is the
    /// venue recording what it received — not an unauthenticated RPC caller.
    ///
    /// When `quote` is `Some` the venue auto-quoted the RFQ, so it is stored
    /// [`DeskRequestState::Quoted`] at that firm level (processed history the GUI shows
    /// alongside live work); otherwise it is stored [`DeskRequestState::Pending`] for a
    /// human trader to price. Either way the SAME notification the RPC submit path emits
    /// is published, so a subscribed desk sees the inbound RFQ instantly.
    ///
    /// Returns the stored [`DeskRequest`] (for logging/tests).
    #[allow(clippy::too_many_arguments)] // the request's identifying fields, no natural sub-struct.
    pub fn ingest_fix_rfq(
        &self,
        desk: &str,
        counterparty: &str,
        instrument: RatesInstrument,
        curve_set: CurveSet,
        side: Side,
        notional: f64,
        quote: Option<DeskQuote>,
    ) -> DeskRequest {
        let now = self.clock.now_nanos();
        let expires_at = now.saturating_add(i64::from(DEFAULT_TTL_MS).saturating_mul(1_000_000));
        let state = if quote.is_some() {
            DeskRequestState::Quoted
        } else {
            DeskRequestState::Pending
        };
        let stored = DeskRequest {
            request_id: self.requests.next_request_id(),
            kind: DeskRequestKind::Rfq as i32,
            counterparty: counterparty.to_owned(),
            desk: desk.to_owned(),
            instrument: Some(instrument),
            curve_set: Some(curve_set),
            side: side as i32,
            notional,
            received_at_nanos: now,
            expires_at_nanos: expires_at,
            state: state as i32,
            quote,
            correlation_id: None,
        };
        self.requests.insert(stored.clone());

        // Notify the desk: an auto-quoted RFQ is history (QUOTE_ACCEPTED reads as "the
        // venue showed a price"); a routed RFQ needs a human (RFQ_RECEIVED).
        let (kind, headline, detail) = if stored.quote.is_some() {
            (
                NotificationKind::QuoteAccepted,
                format!("Auto-quoted RFQ from {counterparty}"),
                Some(format!(
                    "{notional:.0} notional on desk {desk} — auto-quoted"
                )),
            )
        } else {
            (
                NotificationKind::RfqReceived,
                format!("New RFQ from {counterparty} — needs pricing"),
                Some(format!("{notional:.0} notional on desk {desk}")),
            )
        };
        self.publish_notification(kind, &stored, headline, detail);
        stored
    }

    /// A deterministic, broker-scoped notification id.
    fn notify_id(&self) -> String {
        // The broker owns no id space (it is a pure fan-out); deals/requests own
        // theirs. Notifications are transient — id them off a per-edge monotonic
        // derived from the broker's subscriber-independent counter via the deal/req
        // stores would couple unrelated spaces, so mint from the clock-independent
        // request store ordinal namespace is wrong too. Use a dedicated counter.
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(1);
        format!("notif-{}", NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

#[tonic::async_trait]
impl RfqDeskService for RfqDeskEdge {
    async fn submit_desk_request(
        &self,
        request: Request<SubmitDeskRequestRequest>,
    ) -> Result<Response<SubmitDeskRequestResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        let caller = resolve_caller(&self.sessions, req.session_token.as_deref(), req.principal)?;
        authorize_caller(
            self.access_store.access_mode(),
            &caller,
            "RfqDeskService/SubmitDeskRequest",
            RequiredAuthority::ReadAny,
            None,
        )?;

        // Validate the request is priceable BEFORE it enters the inbox (the desk is
        // never shown an unpriceable RFQ) — through the shared `price_rates` entry.
        price_desk_request(req.instrument.as_ref(), req.curve_set.as_ref())?;
        if req.notional <= 0.0 {
            return Err(Status::invalid_argument("notional must be > 0"));
        }
        if req.desk.trim().is_empty() {
            return Err(Status::invalid_argument(
                "a request must name a target desk",
            ));
        }

        let kind = DeskRequestKind::try_from(req.kind)
            .map_err(|_| Status::invalid_argument("unknown DeskRequestKind"))?;
        if kind == DeskRequestKind::Unspecified {
            return Err(Status::invalid_argument("request kind must be RFQ or IOI"));
        }
        let side =
            Side::try_from(req.side).map_err(|_| Status::invalid_argument("unknown Side"))?;

        let now = self.clock.now_nanos();
        let ttl_ms = if req.ttl_ms == 0 {
            DEFAULT_TTL_MS
        } else {
            req.ttl_ms
        };
        let expires_at = now.saturating_add(i64::from(ttl_ms).saturating_mul(1_000_000));

        let request_id = self.requests.next_request_id();
        let stored = DeskRequest {
            request_id: request_id.clone(),
            kind: kind as i32,
            counterparty: req.counterparty,
            desk: req.desk,
            instrument: req.instrument,
            curve_set: req.curve_set,
            side: side as i32,
            notional: req.notional,
            received_at_nanos: now,
            expires_at_nanos: expires_at,
            state: DeskRequestState::Pending as i32,
            quote: None,
            correlation_id: req.correlation_id,
        };
        self.requests.insert(stored.clone());

        // Tell the desk an RFQ/IOI needs pricing — instantly, over the push channel.
        let (nkind, head) = match kind {
            DeskRequestKind::Ioi => (
                NotificationKind::IoiReceived,
                format!("New IOI from {}", stored.counterparty),
            ),
            _ => (
                NotificationKind::RfqReceived,
                format!("New RFQ from {} — needs pricing", stored.counterparty),
            ),
        };
        self.publish_notification(
            nkind,
            &stored,
            head,
            Some(format!(
                "{:.0} notional on desk {}",
                stored.notional, stored.desk
            )),
        );

        Ok(Response::new(SubmitDeskRequestResponse {
            request: Some(stored),
        }))
    }

    async fn respond_desk_request(
        &self,
        request: Request<RespondDeskRequestRequest>,
    ) -> Result<Response<RespondDeskRequestResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        let caller = resolve_caller(&self.sessions, req.session_token.as_deref(), req.principal)?;
        authorize_caller(
            self.access_store.access_mode(),
            &caller,
            "RfqDeskService/RespondDeskRequest",
            RequiredAuthority::ReadAny,
            None,
        )?;

        let mut current = self
            .requests
            .get(&req.request_id)
            .ok_or_else(|| Status::not_found(format!("no desk request `{}`", req.request_id)))?;
        if current.state != DeskRequestState::Pending as i32 {
            return Err(Status::failed_precondition(format!(
                "request `{}` is not PENDING (state {})",
                req.request_id, current.state
            )));
        }

        // Action-capability gate: responding to an RFQ vs an IOI is a distinct
        // capability and the kind is known only after the request resolves, so the
        // pre-lookup `ReadAny` gate above authenticates the caller (no anonymous
        // enumeration of request ids) and this gate enforces the action right.
        let respond_action = match DeskRequestKind::try_from(current.kind) {
            Ok(DeskRequestKind::Ioi) => Action::IoiRespond,
            _ => Action::RfqRespond,
        };
        authorize_caller(
            self.access_store.access_mode(),
            &caller,
            "RfqDeskService/RespondDeskRequest",
            RequiredAuthority::Capability(respond_action, AssetClass::FixedIncome),
            None,
        )?;

        let arm = req.response.ok_or_else(|| {
            Status::invalid_argument("respond: a `quote` or `reject` is required")
        })?;
        match arm {
            RespondArm::Quote(quote) => {
                if quote.notional <= 0.0 {
                    return Err(Status::invalid_argument("quote notional must be > 0"));
                }
                current.quote = Some(quote);
                current.state = DeskRequestState::Quoted as i32;
                let stored = self
                    .requests
                    .replace(current)
                    .ok_or_else(|| Status::internal("request vanished mid-response"))?;
                Ok(Response::new(RespondDeskRequestResponse {
                    request: Some(stored),
                }))
            }
            RespondArm::Reject(reject) => {
                current.state = DeskRequestState::Rejected as i32;
                let stored = self
                    .requests
                    .replace(current)
                    .ok_or_else(|| Status::internal("request vanished mid-response"))?;
                self.publish_notification(
                    NotificationKind::QuoteRejected,
                    &stored,
                    format!("RFQ {} declined", stored.request_id),
                    Some(reject.reason),
                );
                Ok(Response::new(RespondDeskRequestResponse {
                    request: Some(stored),
                }))
            }
        }
    }

    async fn accept_desk_quote(
        &self,
        request: Request<AcceptDeskQuoteRequest>,
    ) -> Result<Response<AcceptDeskQuoteResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        let caller = resolve_caller(&self.sessions, req.session_token.as_deref(), req.principal)?;
        authorize_caller(
            self.access_store.access_mode(),
            &caller,
            "RfqDeskService/AcceptDeskQuote",
            // Accepting a desk quote books a deal — the Execute (deal) capability.
            RequiredAuthority::Capability(Action::Execute, AssetClass::FixedIncome),
            None,
        )?;

        let mut current = self
            .requests
            .get(&req.request_id)
            .ok_or_else(|| Status::not_found(format!("no desk request `{}`", req.request_id)))?;
        if current.state != DeskRequestState::Quoted as i32 {
            return Err(Status::failed_precondition(format!(
                "request `{}` is not QUOTED (state {}) — cannot accept",
                req.request_id, current.state
            )));
        }
        let quote: DeskQuote = current
            .quote
            .clone()
            .ok_or_else(|| Status::internal("QUOTED request carries no quote"))?;

        // The desk's traded direction is the opposite of the counterparty's firm
        // instrument side (validated priceable at submit, so always Buy/Sell).
        let ois = ois_of(current.instrument.as_ref())
            .ok_or_else(|| Status::invalid_argument("request carries no OIS instrument to book"))?;
        let traded_side = Side::try_from(ois.side)
            .map_err(|_| Status::invalid_argument("unknown instrument Side"))?;
        let desk_side = opposite_side(traded_side);

        // Book the dealt rates position (desk perspective) at the lifted level.
        let booked_instrument = RatesInstrument {
            instrument: Some(rates_instrument::Instrument::Ois(OisInstrument {
                tenor_years: ois.tenor_years,
                fixed_rate: quote.price,
                notional: quote.notional,
                side: desk_side as i32,
            })),
        };
        let booked = self.rates.book(RatesPosition {
            position_id: 0,
            entity: 0,
            book: 0,
            instrument: Some(booked_instrument),
        })?;

        let now = self.clock.now_nanos();
        let deal = Deal {
            deal_id: self.deals.next_deal_id(),
            request_id: current.request_id.clone(),
            kind: current.kind,
            counterparty: current.counterparty.clone(),
            desk: current.desk.clone(),
            instrument: Some(booked_instrument),
            curve_set: current.curve_set.clone(),
            side: desk_side as i32,
            notional: quote.notional,
            price: quote.price,
            executed_at_nanos: now,
            trader: quote.trader.clone(),
            position_id: Some(booked.position_id),
            correlation_id: current.correlation_id.clone(),
        };
        self.deals.insert(deal.clone());

        current.state = DeskRequestState::Accepted as i32;
        let stored = self
            .requests
            .replace(current)
            .ok_or_else(|| Status::internal("request vanished mid-accept"))?;

        self.publish_notification(
            NotificationKind::QuoteAccepted,
            &stored,
            format!("Quote lifted — deal {} booked", deal.deal_id),
            Some(format!(
                "{} {:.4} on {:.0} notional",
                stored.desk, deal.price, deal.notional
            )),
        );

        Ok(Response::new(AcceptDeskQuoteResponse {
            deal: Some(deal),
            request: Some(stored),
        }))
    }

    async fn list_desk_requests(
        &self,
        request: Request<ListDeskRequestsRequest>,
    ) -> Result<Response<ListDeskRequestsResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        let caller = resolve_caller(&self.sessions, req.session_token.as_deref(), req.principal)?;
        authorize_caller(
            self.access_store.access_mode(),
            &caller,
            "RfqDeskService/ListDeskRequests",
            RequiredAuthority::ReadAny,
            None,
        )?;
        let scope = caller.desk_scope();
        let (states, desk): (Vec<i32>, Option<String>) = match req.scope {
            Some(s) => (s.states, s.desk),
            None => (Vec::new(), None),
        };
        let requests = self
            .requests
            .snapshot()
            .into_iter()
            .filter(|r| desk_visible(&r.desk, &scope, desk.as_deref()))
            .filter(|r| states.is_empty() || states.contains(&r.state))
            .collect();
        Ok(Response::new(ListDeskRequestsResponse { requests }))
    }

    async fn list_deals(
        &self,
        request: Request<ListDealsRequest>,
    ) -> Result<Response<ListDealsResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        let caller = resolve_caller(&self.sessions, req.session_token.as_deref(), req.principal)?;
        authorize_caller(
            self.access_store.access_mode(),
            &caller,
            "RfqDeskService/ListDeals",
            RequiredAuthority::ReadAny,
            None,
        )?;
        let scope = caller.desk_scope();
        let desk = req.scope.and_then(|s| s.desk);
        let deals = self
            .deals
            .snapshot()
            .into_iter()
            .filter(|d| desk_visible(&d.desk, &scope, desk.as_deref()))
            .collect();
        Ok(Response::new(ListDealsResponse { deals }))
    }
}

#[tonic::async_trait]
impl celnet_proto::notification_service_server::NotificationService for RfqDeskEdge {
    type StreamNotificationsStream =
        crate::services::stream_rx::ReceiverStream<Result<Notification, Status>>;

    async fn stream_notifications(
        &self,
        request: Request<celnet_proto::StreamNotificationsRequest>,
    ) -> Result<Response<Self::StreamNotificationsStream>, Status> {
        // The readiness gate gates *new* streams: a draining instance refuses to open
        // one so new traffic steers to the warm replacement. The guard is held for the
        // stream lifetime (the drain barrier), exactly as `StreamService` does.
        let guard = self.gate.enter();
        if !self.gate.is_ready() {
            return Err(Status::unavailable(
                "edge not ready (starting or draining); steer to the active instance",
            ));
        }
        let req = request.into_inner();
        let subscription = self.subscribe_notifications(&req)?;
        let sub_id = subscription.id;
        let mut broker_rx = subscription.rx;
        let broker = Arc::clone(&self.notify);
        let (out_tx, out_rx) =
            tokio::sync::mpsc::channel::<Result<Notification, Status>>(notify::NOTIFY_QUEUE_DEPTH);
        tokio::spawn(async move {
            let _guard = guard; // held for the stream lifetime (drain barrier).
            while let Some(n) = broker_rx.recv().await {
                if out_tx.send(Ok(n)).await.is_err() {
                    break; // the client dropped the stream.
                }
            }
            // The broker side closed (edge shutdown) or the client went away — either
            // way, deregister this subscriber so it never leaks.
            broker.unsubscribe(sub_id);
        });
        Ok(Response::new(
            crate::services::stream_rx::ReceiverStream::new(out_rx),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::rates_book::RatesPositionStore;
    use celnet_limits::{LimitScope, LimitSpec};
    use celnet_proto::{CurveSet, RatesInstrument};

    fn curve() -> CurveSet {
        crate::rates_pricing::default_usd_sofr_curve_set()
    }

    fn ois_instrument(side: Side) -> RatesInstrument {
        RatesInstrument {
            instrument: Some(rates_instrument::Instrument::Ois(OisInstrument {
                tenor_years: 5,
                fixed_rate: 0.0405,
                notional: 25_000_000.0,
                side: side as i32,
            })),
        }
    }

    fn edge() -> RfqDeskEdge {
        let gate = Arc::new(ReadinessGate::new());
        gate.mark_ready();
        RfqDeskEdge::new(
            Arc::new(PositionStore::new()),
            Arc::new(SessionRegistry::new(Clock::manual(0))),
            gate,
            Arc::new(DeskRequestStore::new()),
            Arc::new(DealStore::new()),
            Arc::new(RatesPositionStore::new()),
            Arc::new(NotificationBroker::new()),
            Clock::manual(1_000),
        )
    }

    fn submit_req(kind: DeskRequestKind, side: Side) -> SubmitDeskRequestRequest {
        SubmitDeskRequestRequest {
            session_token: None,
            kind: kind as i32,
            counterparty: "cp-bank".to_owned(),
            desk: "g10".to_owned(),
            instrument: Some(ois_instrument(side)),
            curve_set: Some(curve()),
            side: side as i32,
            notional: 25_000_000.0,
            ttl_ms: 0,
            // Grant-all so the deny-by-default boundary admits the call (no session).
            principal: Some(celnet_proto::EntitlementPrincipal {
                grant_all: true,
                grants: vec![],
                denies: vec![],
            }),
            correlation_id: Some("corr-1".to_owned()),
        }
    }

    /// Pricing a desk request goes through the SAME `price_rates` entry as
    /// `PriceRates` — identical PV/par for the same instrument + curve.
    #[test]
    fn desk_pricing_matches_price_rates() {
        let instrument = ois_instrument(Side::Buy);
        let cset = curve();
        let via_desk = price_desk_request(Some(&instrument), Some(&cset)).expect("prices");
        let direct = price_rates(&RatesPriceRequest {
            request_id: 7,
            curve_set: Some(cset),
            instrument: Some(instrument),
            correlation_id: None,
        })
        .expect("prices");
        assert_eq!(via_desk.pv.to_bits(), direct.pv.to_bits());
        assert_eq!(via_desk.par_rate.to_bits(), direct.par_rate.to_bits());
        assert_eq!(via_desk.dv01.to_bits(), direct.dv01.to_bits());
    }

    /// Issue a live `Trader` session on the edge's registry and return its bearer
    /// token. Responding to / accepting a desk quote now requires the action
    /// capability the session carries (an asserted body principal cannot
    /// self-grant it), so the dealing tests authenticate a real desk trader.
    fn trader_token(edge: &RfqDeskEdge) -> String {
        edge.sessions()
            .issue(crate::services::sessions::AuthenticatedUser {
                user_id: "t-1".to_owned(),
                email: "trader@celnet.com".to_owned(),
                display_name: "Desk Trader".to_owned(),
                role: crate::config::identity::Role::Trader,
                desk_id: Some("g10".to_owned()),
                role_caps: crate::config::identity::default_trader_bundle(),
                cap_grants: Vec::new(),
                cap_denies: Vec::new(),
            })
            .expect("issue trader session")
            .token
    }

    /// submit → respond(quote) → accept books a deal AND a rates position, and the
    /// request ends ACCEPTED with the desk side opposite the counterparty's.
    #[tokio::test]
    async fn lifecycle_quote_then_accept_books_deal_and_position() {
        let edge = edge();
        let token = trader_token(&edge);
        let submitted = edge
            .submit_desk_request(Request::new(submit_req(DeskRequestKind::Rfq, Side::Buy)))
            .await
            .expect("submit")
            .into_inner()
            .request
            .expect("request");
        assert_eq!(submitted.state, DeskRequestState::Pending as i32);
        let id = submitted.request_id.clone();

        let quoted = edge
            .respond_desk_request(Request::new(RespondDeskRequestRequest {
                session_token: Some(token.clone()),
                request_id: id.clone(),
                principal: Some(celnet_proto::EntitlementPrincipal {
                    grant_all: true,
                    grants: vec![],
                    denies: vec![],
                }),
                correlation_id: None,
                response: Some(RespondArm::Quote(DeskQuote {
                    price: 0.0411,
                    notional: 25_000_000.0,
                    valid_for_ms: 30_000,
                    trader: "alice".to_owned(),
                })),
            }))
            .await
            .expect("respond")
            .into_inner()
            .request
            .expect("request");
        assert_eq!(quoted.state, DeskRequestState::Quoted as i32);

        let accept = edge
            .accept_desk_quote(Request::new(AcceptDeskQuoteRequest {
                session_token: Some(token.clone()),
                request_id: id.clone(),
                principal: Some(celnet_proto::EntitlementPrincipal {
                    grant_all: true,
                    grants: vec![],
                    denies: vec![],
                }),
                correlation_id: None,
            }))
            .await
            .expect("accept")
            .into_inner();
        let deal = accept.deal.expect("deal");
        let final_req = accept.request.expect("request");
        assert_eq!(final_req.state, DeskRequestState::Accepted as i32);
        // Counterparty bought (paid fixed) ⇒ desk sells (receives fixed).
        assert_eq!(deal.side, Side::Sell as i32);
        assert_eq!(deal.price.to_bits(), 0.0411_f64.to_bits());
        assert!(deal.position_id.is_some());
        assert_eq!(edge.deals.len(), 1);
        assert_eq!(edge.rates.len(), 1);
        // The booked position carries the dealt level + desk side.
        let pos = &edge.rates.snapshot()[0];
        let booked_ois = ois_of(pos.instrument.as_ref()).unwrap();
        assert_eq!(booked_ois.side, Side::Sell as i32);
        assert_eq!(booked_ois.fixed_rate.to_bits(), 0.0411_f64.to_bits());
    }

    /// FRONT-END 4 (`RfqDeskService::AcceptDeskQuote`, ADR-0016 A1): accepting a desk
    /// quote whose booking would blow a hard firm-wide limit is **rejected** with a
    /// `failed_precondition` `LimitBreached` status at the rates position sink, and
    /// nothing is booked — neither a rates position nor a deal.
    #[tokio::test]
    async fn accept_desk_quote_rejects_a_hard_limit_blown_booking() {
        // A desk edge over a rates book with a hard firm Delta cap of 1 base unit — a
        // 25mm 5y OIS charges `25mm · 5 · 1bp = 12500` against it, a hard breach.
        let rates = Arc::new(RatesPositionStore::new());
        rates.set_limit(
            LimitScope::Firm,
            LimitSpec::hard(celnet_limits::LimitMetric::Delta, 1.0),
        );
        let gate = Arc::new(ReadinessGate::new());
        gate.mark_ready();
        let edge = RfqDeskEdge::new(
            Arc::new(PositionStore::new()),
            Arc::new(SessionRegistry::new(Clock::manual(0))),
            gate,
            Arc::new(DeskRequestStore::new()),
            Arc::new(DealStore::new()),
            Arc::clone(&rates),
            Arc::new(NotificationBroker::new()),
            Clock::manual(1_000),
        );
        let token = trader_token(&edge);

        let id = edge
            .submit_desk_request(Request::new(submit_req(DeskRequestKind::Rfq, Side::Buy)))
            .await
            .expect("submit")
            .into_inner()
            .request
            .expect("request")
            .request_id;
        edge.respond_desk_request(Request::new(RespondDeskRequestRequest {
            session_token: Some(token.clone()),
            request_id: id.clone(),
            principal: Some(celnet_proto::EntitlementPrincipal {
                grant_all: true,
                grants: vec![],
                denies: vec![],
            }),
            correlation_id: None,
            response: Some(RespondArm::Quote(DeskQuote {
                price: 0.0411,
                notional: 25_000_000.0,
                valid_for_ms: 30_000,
                trader: "alice".to_owned(),
            })),
        }))
        .await
        .expect("respond");

        let err = edge
            .accept_desk_quote(Request::new(AcceptDeskQuoteRequest {
                session_token: Some(token),
                request_id: id,
                principal: Some(celnet_proto::EntitlementPrincipal {
                    grant_all: true,
                    grants: vec![],
                    denies: vec![],
                }),
                correlation_id: None,
            }))
            .await
            .expect_err("a hard-limit-blown desk accept must be rejected");
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);
        assert!(
            err.message().contains("limit breached"),
            "the reject carries the uniform LimitBreached reason, got {:?}",
            err.message()
        );
        assert_eq!(
            rates.len(),
            0,
            "a rejected desk accept books no rates position"
        );
        assert_eq!(edge.deals.len(), 0, "a rejected desk accept books no deal");
    }

    /// submit → respond(reject) sets REJECTED and never books anything.
    #[tokio::test]
    async fn lifecycle_reject() {
        let edge = edge();
        let token = trader_token(&edge);
        let id = edge
            .submit_desk_request(Request::new(submit_req(DeskRequestKind::Rfq, Side::Sell)))
            .await
            .expect("submit")
            .into_inner()
            .request
            .unwrap()
            .request_id;
        let rejected = edge
            .respond_desk_request(Request::new(RespondDeskRequestRequest {
                session_token: Some(token.clone()),
                request_id: id.clone(),
                principal: Some(celnet_proto::EntitlementPrincipal {
                    grant_all: true,
                    grants: vec![],
                    denies: vec![],
                }),
                correlation_id: None,
                response: Some(RespondArm::Reject(celnet_proto::DeskReject {
                    reason: "off-market".to_owned(),
                })),
            }))
            .await
            .expect("respond")
            .into_inner()
            .request
            .unwrap();
        assert_eq!(rejected.state, DeskRequestState::Rejected as i32);
        assert_eq!(edge.deals.len(), 0);
        assert_eq!(edge.rates.len(), 0);
    }

    /// Accepting a non-QUOTED request fails precondition (cannot lift an unquoted RFQ).
    #[tokio::test]
    async fn accept_unquoted_is_failed_precondition() {
        let edge = edge();
        let token = trader_token(&edge);
        let id = edge
            .submit_desk_request(Request::new(submit_req(DeskRequestKind::Rfq, Side::Buy)))
            .await
            .expect("submit")
            .into_inner()
            .request
            .unwrap()
            .request_id;
        let err = edge
            .accept_desk_quote(Request::new(AcceptDeskQuoteRequest {
                session_token: Some(token.clone()),
                request_id: id,
                principal: Some(celnet_proto::EntitlementPrincipal {
                    grant_all: true,
                    grants: vec![],
                    denies: vec![],
                }),
                correlation_id: None,
            }))
            .await
            .expect_err("cannot accept a PENDING request");
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);
    }

    /// The action-capability gate: an asserted grant-all body principal with NO
    /// authenticated session cannot accept (deal) under enforce — a body principal
    /// can never self-grant the Execute capability (security finding #3 guard, on
    /// the real RPC). Contrast `accept_unquoted_is_failed_precondition`, which now
    /// authenticates and so reaches the precondition check.
    #[tokio::test]
    async fn accept_without_session_is_denied_under_enforce() {
        let edge = edge();
        let id = edge
            .submit_desk_request(Request::new(submit_req(DeskRequestKind::Rfq, Side::Buy)))
            .await
            .expect("submit")
            .into_inner()
            .request
            .unwrap()
            .request_id;
        let err = edge
            .accept_desk_quote(Request::new(AcceptDeskQuoteRequest {
                session_token: None,
                request_id: id,
                principal: Some(celnet_proto::EntitlementPrincipal {
                    grant_all: true,
                    grants: vec![],
                    denies: vec![],
                }),
                correlation_id: None,
            }))
            .await
            .expect_err("a body principal cannot self-grant Execute");
        assert_eq!(err.code(), tonic::Code::Unauthenticated);
    }

    /// Deny-by-default: an absent principal under enforce is denied at submit.
    #[tokio::test]
    async fn submit_denies_absent_principal_under_enforce() {
        let edge = edge();
        let mut req = submit_req(DeskRequestKind::Rfq, Side::Buy);
        req.principal = None; // absent ⇒ deny-by-default under enforce (the construction default)
        let err = edge
            .submit_desk_request(Request::new(req))
            .await
            .expect_err("absent principal must be denied");
        assert_eq!(err.code(), tonic::Code::Unauthenticated);
    }

    /// A submit publishes an RFQ_RECEIVED notification to a matching subscriber.
    #[tokio::test]
    async fn submit_publishes_notification() {
        let edge = edge();
        let mut sub = edge.notify.subscribe(DeskFilter::All);
        edge.submit_desk_request(Request::new(submit_req(DeskRequestKind::Rfq, Side::Buy)))
            .await
            .expect("submit");
        let n = sub.rx.try_recv().expect("a notification was published");
        assert_eq!(n.kind, NotificationKind::RfqReceived as i32);
        assert_eq!(n.desk, "g10");
    }

    /// The shared `subscribe_notifications` path (gRPC + WS) authorizes the caller and
    /// registers a subscriber that then receives a published notification; an absent
    /// principal under enforce is denied (deny-by-default).
    #[tokio::test]
    async fn subscribe_notifications_authorizes_and_receives() {
        let edge = edge();
        // Absent principal under enforce ⇒ denied.
        let denied = edge.subscribe_notifications(&celnet_proto::StreamNotificationsRequest {
            session_token: None,
            scope: None,
            principal: None,
            correlation_id: None,
        });
        assert_eq!(denied.unwrap_err().code(), tonic::Code::Unauthenticated);

        // Grant-all ⇒ subscribed; a submit then fans a notification to it.
        let mut sub = edge
            .subscribe_notifications(&celnet_proto::StreamNotificationsRequest {
                session_token: None,
                scope: Some(celnet_proto::NotificationScope {
                    desks: vec!["g10".to_owned()],
                }),
                principal: Some(celnet_proto::EntitlementPrincipal {
                    grant_all: true,
                    grants: vec![],
                    denies: vec![],
                }),
                correlation_id: None,
            })
            .expect("subscribes");
        edge.submit_desk_request(Request::new(submit_req(DeskRequestKind::Rfq, Side::Buy)))
            .await
            .expect("submit");
        let n = sub
            .rx
            .try_recv()
            .expect("the g10 subscriber receives the RFQ");
        assert_eq!(n.desk, "g10");
    }

    /// A FIX venue ingesting an inbound RFQ records it into the inbox WITHOUT the RPC
    /// auth boundary: an auto-quoted RFQ lands QUOTED (with the firm quote) as processed
    /// history, a routed one lands PENDING for a human — and both fan a notification and
    /// appear in the snapshot the GUI reads.
    #[tokio::test]
    async fn ingest_fix_rfq_records_auto_and_manual() {
        let edge = edge();
        let mut sub = edge.notify.subscribe(DeskFilter::All);

        // Auto-quoted: carries a firm quote ⇒ QUOTED history.
        let auto = edge.ingest_fix_rfq(
            "g10-rates",
            "CELER_RATES",
            ois_instrument(Side::Buy),
            curve(),
            Side::Buy,
            10_000_000.0,
            Some(DeskQuote {
                price: 0.0405,
                notional: 10_000_000.0,
                valid_for_ms: 30_000,
                trader: "auto".to_owned(),
            }),
        );
        assert_eq!(auto.state, DeskRequestState::Quoted as i32);
        assert_eq!(auto.desk, "g10-rates");
        assert_eq!(auto.counterparty, "CELER_RATES");
        assert!(auto.quote.is_some());
        let n = sub.rx.try_recv().expect("auto-quote fans a notification");
        assert_eq!(n.kind, NotificationKind::QuoteAccepted as i32);

        // Routed: no quote ⇒ PENDING for a human.
        let manual = edge.ingest_fix_rfq(
            "g10-rates",
            "CELER_RATES",
            ois_instrument(Side::Sell),
            curve(),
            Side::Sell,
            50_000_000.0,
            None,
        );
        assert_eq!(manual.state, DeskRequestState::Pending as i32);
        assert!(manual.quote.is_none());
        let n = sub.rx.try_recv().expect("a routed RFQ fans a notification");
        assert_eq!(n.kind, NotificationKind::RfqReceived as i32);

        // Both are in the snapshot the GUI reads (history + live), newest-first.
        let all = edge.requests.snapshot();
        assert_eq!(all.len(), 2);
        assert!(
            all.iter()
                .any(|r| r.state == DeskRequestState::Quoted as i32)
        );
        assert!(
            all.iter()
                .any(|r| r.state == DeskRequestState::Pending as i32)
        );
    }

    /// `effective_desk_filter` intersects requested desks with the caller's scope.
    #[test]
    fn desk_filter_intersects_scope() {
        assert_eq!(effective_desk_filter(&[], &DeskScope::All), DeskFilter::All);
        assert_eq!(
            effective_desk_filter(&[], &DeskScope::Desk("g10".to_owned())),
            DeskFilter::Desks(["g10".to_owned()].into_iter().collect())
        );
        // Requesting a desk the caller cannot see yields an empty filter.
        assert_eq!(
            effective_desk_filter(&["em".to_owned()], &DeskScope::Desk("g10".to_owned())),
            DeskFilter::Desks(std::collections::HashSet::new())
        );
        assert_eq!(
            effective_desk_filter(&["em".to_owned()], &DeskScope::Deskless),
            DeskFilter::Desks(std::collections::HashSet::new())
        );
    }
}
