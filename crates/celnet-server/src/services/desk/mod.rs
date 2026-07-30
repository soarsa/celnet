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
    ListDeskRequestsRequest, ListDeskRequestsResponse, ManualInterventionReason, Notification,
    NotificationKind, RatesInstrument, RatesPosition, RespondDeskRequestRequest,
    RespondDeskRequestResponse, Side, SubmitDeskRequestRequest, SubmitDeskRequestResponse,
    rates_instrument, respond_desk_request_request::Response as RespondArm,
};
use tonic::{Request, Response, Status};

use crate::clock::Clock;
use crate::rates_pricing::{RatesPriceError, price_rates};
use crate::readiness::ReadinessGate;
use celnet_entitlements::{Action, AssetClass};

use crate::services::access::{DeskScope, RequiredAuthority, authorize_caller, resolve_caller};
use crate::services::analytics::{ClientFlowSource, in_window};
use crate::services::rates_book::{RatesPositionStore, RatesRoutingAttribution};
use crate::services::risk::store::PositionStore;
use crate::services::sessions::SessionRegistry;
use celnet_proto::RatesPricingResult;

use notify::{DeskFilter, NotificationBroker};
use store::{DealStore, DeskRequestStore};

use celnet_proto::RatesPriceRequest;

/// The default time-to-live (ms) granted to a submitted request when the caller
/// supplies `ttl_ms == 0`: two minutes, a generous desk-response window.
const DEFAULT_TTL_MS: u32 = 120_000;

/// The outcome of ingesting an inbound (FIX-venue) RFQ into the desk inbox — it decides
/// the stored [`DeskRequestState`], whether a firm [`DeskQuote`] rides the row, and which
/// notification the desk sees (and crucially whether it is **alert-worthy**):
///
/// * [`AutoQuoted`](RfqIngestOutcome::AutoQuoted) — the venue priced it; QUOTED history +
///   a quiet `QUOTE_ACCEPTED`.
/// * [`RoutedToDesk`](RfqIngestOutcome::RoutedToDesk) — a routine RFQ a human prices in
///   the normal flow (e.g. a large on-the-run clip); PENDING + a quiet `RFQ_RECEIVED`.
/// * [`ManualIntervention`](RfqIngestOutcome::ManualIntervention) — it CANNOT be
///   auto-priced and needs a human now; PENDING + an **alert** `MANUAL_INTERVENTION_REQUIRED`
///   carrying the machine-readable [`ManualInterventionReason`].
#[derive(Debug, Clone)]
pub enum RfqIngestOutcome {
    /// Auto-quoted at a firm level — QUOTED history, quiet `QUOTE_ACCEPTED`.
    AutoQuoted(DeskQuote),
    /// Routed to a human desk as a routine RFQ — PENDING, quiet `RFQ_RECEIVED`.
    RoutedToDesk,
    /// Cannot be auto-priced — PENDING, ALERT `MANUAL_INTERVENTION_REQUIRED` + reason.
    ManualIntervention(ManualInterventionReason),
}

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
    /// The risk-transfer service backing the transfer-inbox push stream (§9.2). Set
    /// once at boot (via [`RfqDeskEdge::set_transfer_service`]) AFTER this edge is
    /// already shared behind an `Arc` — the identity store the transfer service needs
    /// is constructed later than this edge — so it is a [`OnceLock`] rather than a
    /// pre-`Arc` builder field. Unset in an isolated desk test (the inbox stream then
    /// reports `unavailable`).
    ///
    /// [`OnceLock`]: std::sync::OnceLock
    transfer_service: std::sync::OnceLock<Arc<crate::services::risk_transfer::RiskTransferService>>,
    /// The edge clock (receipt / execution timestamps), manual in tests.
    clock: Clock,
}

/// The FI dealer-quoting desk as a **client-flow analytics source** (Analytics
/// phase 2): it maps its retained desk-inbox [`DeskRequest`]s into asset-tagged
/// [`FlowRecord`](celnet_analytics::FlowRecord)s the rollup folds. Read-only,
/// on-query, off the hot path.
///
/// Honest scope (guardrail 2): the desk-quoting path is single-dealer and is NOT
/// priced through the `celnet-tiering` feature pipeline, so a desk quote/deal
/// carries **no** per-feature provenance and **no** competing panel. `margin`,
/// `quoted_spread`, and `cover_distance` are therefore legitimately `0`/`None`
/// here — a real upstream product gap (FI-desk provenance is a later phase), not a
/// fabricated value. Volume, counts, hit-rate and quote-fishing ARE real from the
/// inbox lifecycle (quoted vs accepted).
#[tonic::async_trait]
impl ClientFlowSource for RfqDeskEdge {
    async fn flow_records(
        &self,
        from: Option<i64>,
        to: Option<i64>,
    ) -> Vec<celnet_analytics::FlowRecord> {
        self.requests
            .snapshot()
            .iter()
            .filter_map(|req| desk_request_to_flow(req, from, to))
            .collect()
    }
}

/// A coarse (arm-level) instrument key for an FI desk request — the linear-rates
/// product family. Fine-grained per-ISIN/tenor keying is a later refinement; the
/// primary FI lenses this phase are client / counterparty / asset.
fn rates_instrument_label(instrument: Option<&RatesInstrument>) -> String {
    match instrument.and_then(|i| i.instrument.as_ref()) {
        Some(rates_instrument::Instrument::Ois(_)) => "OIS",
        Some(rates_instrument::Instrument::Irs(_)) => "IRS",
        Some(rates_instrument::Instrument::Fra(_)) => "FRA",
        Some(rates_instrument::Instrument::Bond(_)) => "BOND",
        None => "rates",
    }
    .to_owned()
}

/// Project one desk-inbox [`DeskRequest`] onto a neutral [`FlowRecord`], or `None`
/// when we never showed a price (no quote) or its receipt time is outside the
/// `[from, to)` window. A quoted request counts toward `quote_count`; an accepted
/// one additionally counts as a trade.
fn desk_request_to_flow(
    req: &DeskRequest,
    from: Option<i64>,
    to: Option<i64>,
) -> Option<celnet_analytics::FlowRecord> {
    use celnet_analytics::{FlowRecord, Side as FlowSide};

    // Only requests we actually quoted contribute (a bare PENDING/EXPIRED/declined
    // request is not flow we priced). `Accepted` always carries a prior quote.
    req.quote.as_ref()?;
    if !in_window(req.received_at_nanos, from, to) {
        return None;
    }
    let was_traded = req.state == DeskRequestState::Accepted as i32;
    let side = match Side::try_from(req.side) {
        Ok(Side::Sell) => FlowSide::Sell,
        _ => FlowSide::Buy,
    };
    Some(FlowRecord {
        client: req.counterparty.clone(),
        counterparty: req.desk.clone(),
        instrument: rates_instrument_label(req.instrument.as_ref()),
        asset: "fi".to_owned(),
        notional: req.notional.abs(),
        side,
        was_quoted: true,
        was_traded,
        // The FI desk path is not tiered ⇒ no provenance margin / no shown spread /
        // no competing panel. Honest zeros/None (not fabricated).
        margin: 0.0,
        quoted_spread: 0.0,
        cover_distance: None,
        markout: None,
        hedge_cost: None,
    })
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
            transfer_service: std::sync::OnceLock::new(),
            clock,
        }
    }

    /// Wire the shared risk-transfer service so the transfer-inbox push stream is live
    /// (the boot path shares the SAME service the `AuthService` transfer RPCs use). Set
    /// once, after this edge is already `Arc`-shared; a second call is a no-op.
    pub fn set_transfer_service(
        &self,
        service: Arc<crate::services::risk_transfer::RiskTransferService>,
    ) {
        let _ = self.transfer_service.set(service);
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

    /// Resolve + authorize a risk-transfer-inbox subscriber and register it on the
    /// transfer broker under its entitlement-resolved desk filter (all the caller's
    /// desks), then prime it with the current `Pending` roster. Shared by the gRPC
    /// `StreamRiskTransferInbox` handler and the WS subscribe frame.
    ///
    /// # Errors
    /// [`Status::unavailable`] when no transfer service is wired; the shared
    /// authorization boundary's statuses otherwise.
    pub fn subscribe_transfer_inbox(
        &self,
        req: &celnet_proto::StreamRiskTransferInboxRequest,
    ) -> Result<crate::services::risk_transfer::InboxSubscription, Status> {
        let service = self.transfer_service.get().ok_or_else(|| {
            Status::unavailable("risk transfer service is not wired on this edge")
        })?;
        let caller = resolve_caller(
            &self.sessions,
            req.session_token.as_deref(),
            req.principal.clone(),
        )?;
        authorize_caller(
            self.access_store.access_mode(),
            &caller,
            "NotificationService/StreamRiskTransferInbox",
            RequiredAuthority::ReadAny,
            None,
        )?;
        let filter = effective_desk_filter(&[], &caller.desk_scope());
        let subscription = service.broker().subscribe(filter);
        // Prime the fresh subscriber with the current pending roster (its desk-filtered
        // slice arrives on the next publish tick).
        service.publish_inbox();
        Ok(subscription)
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

/// The counterparty's submitted direction, read from whichever FI arm the request
/// carries (OIS / IRS / FRA / cash bond). `None` for a missing arm or an
/// unrecognised `side` enum — the caller treats that as an unbookable request.
fn traded_side_of(instrument: Option<&RatesInstrument>) -> Option<Side> {
    let arm = instrument.and_then(|i| i.instrument.as_ref())?;
    let side = match arm {
        rates_instrument::Instrument::Ois(o) => o.side,
        rates_instrument::Instrument::Irs(i) => i.side,
        rates_instrument::Instrument::Fra(f) => f.side,
        rates_instrument::Instrument::Bond(b) => b.side,
    };
    Side::try_from(side).ok()
}

/// Rebuild the traded instrument at the **dealt level** on the desk's side, ready to
/// book as a firm position — generalised across every streamed FI family (the desk
/// executes a risk trade on whichever line the counterparty lifted, not OIS alone).
///
/// The arm is preserved (all schedule/economics carry through — it is a scalar
/// `Copy` POD) and only the executed terms are overwritten:
/// - **OIS / IRS / FRA** (rate markets): the fixed rate is struck at the dealt
///   `level` (the quoted par rate), the `notional` set to the dealt size, `side` to
///   the desk's side. Priced/booked through the swap engines in `celnet-rates`.
/// - **cash bond** (a clean-price market): the coupon and maturity are the security's
///   own and never change; the dealt `level` is the executed **clean price**, recorded
///   on the [`Deal`] (not on the instrument), so only the `redemption` (face/size) and
///   `side` are set here. Priced/booked through `celnet-bond`.
///
/// `None` for a missing / unrecognised arm (the request is then rejected as
/// unbookable), so the caller never fabricates a position.
fn rebook_at_dealt_level(
    instrument: Option<&RatesInstrument>,
    level: f64,
    notional: f64,
    desk_side: Side,
) -> Option<RatesInstrument> {
    let arm = instrument.and_then(|i| i.instrument.as_ref())?;
    let side = desk_side as i32;
    let booked = match arm {
        rates_instrument::Instrument::Ois(o) => {
            let mut o = *o;
            o.fixed_rate = level;
            o.notional = notional;
            o.side = side;
            rates_instrument::Instrument::Ois(o)
        }
        rates_instrument::Instrument::Irs(i) => {
            let mut i = *i;
            i.fixed_rate = level;
            i.notional = notional;
            i.side = side;
            rates_instrument::Instrument::Irs(i)
        }
        rates_instrument::Instrument::Fra(f) => {
            let mut f = *f;
            f.fixed_rate = level;
            f.notional = notional;
            f.side = side;
            rates_instrument::Instrument::Fra(f)
        }
        rates_instrument::Instrument::Bond(b) => {
            // A cash bond's coupon and maturity are the security's own and never change
            // with the trade; the executed clean price is recorded on the Deal. The
            // booked position size is the traded face (a `RatesPosition` carries no
            // separate quantity, so the redemption face IS the position size), set from
            // the dealt notional, plus the desk's direction. (celnet-bond's YTM solve
            // is price-scale-relative, so a large face books correctly.)
            let mut b = *b;
            b.redemption = notional;
            b.side = side;
            rates_instrument::Instrument::Bond(b)
        }
    };
    Some(RatesInstrument {
        instrument: Some(booked),
    })
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

/// A short human label for a manual-intervention reason (headline/detail text only — the
/// machine-readable signal the GUI gates on is the enum on the `Notification.reason`
/// field, never this string).
fn manual_reason_label(reason: ManualInterventionReason) -> &'static str {
    match reason {
        ManualInterventionReason::Unspecified => "manual pricing",
        ManualInterventionReason::UnconfiguredTenor => "unconfigured tenor",
        ManualInterventionReason::CreditRiskBreak => "credit risk break",
        ManualInterventionReason::UnknownSecurity => "unknown security",
        ManualInterventionReason::PricingFailure => "pricing failure",
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
        DeskScope::Desks(owned) => {
            // The caller may see only their desks; intersect with any requested set.
            // No request ⇒ the caller's full desk set; a request narrows to the
            // intersection (a notification for desk D reaches the caller iff D is in
            // their set and, when a scope was requested, in the requested set too).
            let effective: std::collections::HashSet<String> = if requested.is_empty() {
                owned.clone()
            } else {
                requested
                    .iter()
                    .filter(|r| owned.contains(*r))
                    .cloned()
                    .collect()
            };
            DeskFilter::Desks(effective)
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
    /// Build + publish a **quiet** (not alert-worthy) notification for a desk lifecycle
    /// event (received / auto-quoted / booked / rejected) — off the hot path. These land
    /// in the blotter/inbox without popping a toast (`alert_worthy = false`, no `reason`).
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
            alert_worthy: false,
            reason: None,
        };
        self.notify.publish(&notification);
    }

    /// Build + publish an **alert-worthy** `MANUAL_INTERVENTION_REQUIRED` notification for
    /// an inbound RFQ that cannot be auto-priced and needs a human. Carries the
    /// machine-readable [`ManualInterventionReason`] the GUI renders and gates the popup
    /// on (`alert_worthy = true`). The ONLY path that sets the alert flag.
    fn publish_manual_intervention(
        &self,
        request: &DeskRequest,
        reason: ManualInterventionReason,
        headline: String,
        detail: Option<String>,
    ) {
        let notification = Notification {
            notification_id: self.notify_id(),
            kind: NotificationKind::ManualInterventionRequired as i32,
            at_nanos: self.clock.now_nanos(),
            request_id: Some(request.request_id.clone()),
            desk: request.desk.clone(),
            counterparty: request.counterparty.clone(),
            request_kind: request.kind,
            headline,
            detail,
            alert_worthy: true,
            reason: Some(reason as i32),
        };
        self.notify.publish(&notification);
    }

    /// Ingest an externally-originated (FIX venue) rates RFQ into the desk inbox
    /// **without** the RPC authorization boundary: a managed FIX acceptor has already
    /// authenticated the counterparty at the transport (CompID) layer, so this is the
    /// venue recording what it received — not an unauthenticated RPC caller.
    ///
    /// The `outcome` decides the stored state + notification (see [`RfqIngestOutcome`]):
    /// an [`AutoQuoted`](RfqIngestOutcome::AutoQuoted) RFQ lands
    /// [`DeskRequestState::Quoted`] at the firm level (processed history the GUI shows
    /// alongside live work) with a quiet `QUOTE_ACCEPTED`; a
    /// [`RoutedToDesk`](RfqIngestOutcome::RoutedToDesk) RFQ lands PENDING with a quiet
    /// `RFQ_RECEIVED`; a [`ManualIntervention`](RfqIngestOutcome::ManualIntervention) RFQ
    /// lands PENDING with an **alert-worthy** `MANUAL_INTERVENTION_REQUIRED` carrying the
    /// machine-readable reason. Every case fans a notification so a subscribed desk sees
    /// the inbound RFQ instantly; only the manual-intervention case gates a popup.
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
        outcome: RfqIngestOutcome,
    ) -> DeskRequest {
        let now = self.clock.now_nanos();
        let expires_at = now.saturating_add(i64::from(DEFAULT_TTL_MS).saturating_mul(1_000_000));
        let (state, quote) = match &outcome {
            RfqIngestOutcome::AutoQuoted(q) => (DeskRequestState::Quoted, Some(q.clone())),
            RfqIngestOutcome::RoutedToDesk | RfqIngestOutcome::ManualIntervention(_) => {
                (DeskRequestState::Pending, None)
            }
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

        // Notify the desk. Auto-quoted / routed RFQs are quiet (no popup); only a
        // manual-intervention RFQ is alert-worthy and carries the machine-readable reason.
        match outcome {
            RfqIngestOutcome::AutoQuoted(_) => self.publish_notification(
                NotificationKind::QuoteAccepted,
                &stored,
                format!("Auto-quoted RFQ from {counterparty}"),
                Some(format!(
                    "{notional:.0} notional on desk {desk} — auto-quoted"
                )),
            ),
            RfqIngestOutcome::RoutedToDesk => self.publish_notification(
                NotificationKind::RfqReceived,
                &stored,
                format!("New RFQ from {counterparty} — needs pricing"),
                Some(format!("{notional:.0} notional on desk {desk}")),
            ),
            RfqIngestOutcome::ManualIntervention(reason) => self.publish_manual_intervention(
                &stored,
                reason,
                format!(
                    "Manual pricing needed — RFQ from {counterparty} ({})",
                    manual_reason_label(reason)
                ),
                Some(format!(
                    "{notional:.0} notional on desk {desk} — {}",
                    manual_reason_label(reason)
                )),
            ),
        }
        stored
    }

    /// Book a FIX-venue auto-quote **lift**: the taker sent a `NewOrderSingle(D)` against
    /// a live auto-quote on the FIX edge, so the QUOTED desk request that auto-quote
    /// created transitions to ACCEPTED and books the dealt rates position + [`Deal`] at
    /// the quoted level — the SAME booking body as the gRPC [`Self::accept_desk_quote`],
    /// minus the RPC auth/gate (the FIX transport already authenticated the pre-agreed
    /// counterparty). Returns the booked [`Deal`], or `None` when the request is unknown /
    /// not QUOTED / carries no OIS leaf / breaches a hard limit (the position book refuses
    /// the booking) — in which case the request is left untouched so nothing half-books.
    /// This is what makes a FIX auto-quote that the taker executes appear as a **booked
    /// deal** in the blotter and a live position in the rates Book, not merely a shown
    /// price.
    #[must_use]
    pub fn book_fix_lift(&self, request_id: &str) -> Option<Deal> {
        let mut current = self.requests.get(request_id)?;
        if current.state != DeskRequestState::Quoted as i32 {
            return None;
        }
        let quote: DeskQuote = current.quote.clone()?;

        // The desk's traded direction is the opposite of the counterparty's firm
        // instrument side (validated priceable at ingest, so always Buy/Sell) — read
        // from whichever FI family (OIS / IRS / FRA / bond) the request carries.
        let traded_side = traded_side_of(current.instrument.as_ref())?;
        let desk_side = opposite_side(traded_side);

        // Book the dealt rates position (desk perspective) at the lifted level, on the
        // ACTUAL traded family — the swap/FRA fixed rate (or the bond face) struck at
        // the dealt terms, priced/booked through celnet-rates / celnet-bond.
        let booked_instrument = rebook_at_dealt_level(
            current.instrument.as_ref(),
            quote.price,
            quote.notional,
            desk_side,
        )?;
        // The position book enforces the SAME pre-trade limit tree the gRPC accept path
        // consults; a hard breach refuses the booking (Err) → leave the request QUOTED.
        let booked = self
            .rates
            .book_with_routing(
                RatesPosition {
                    position_id: 0,
                    entity: 0,
                    book: 0,
                    instrument: Some(booked_instrument),
                },
                RatesRoutingAttribution {
                    counterparty: current.counterparty.clone(),
                    ccy: current
                        .curve_set
                        .as_ref()
                        .map(|c| c.currency.clone())
                        .unwrap_or_default(),
                },
            )
            .ok()?;

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
            // The rates dealer-quoting desk path is not priced through a pricing-group
            // feature pipeline, so a desk deal carries no per-feature provenance
            // (design §7 — absent, never a fabricated waterfall).
            pricing_provenance: None,
            // The Risk Portfolio the fill's risk routed into, read from the booked
            // position's stamp: `None` when the firm installed no routing graph (or a
            // routing fall-back left it unrouted) — surfaced, never fabricated.
            risk_book_id: self.rates.risk_book_of(booked.position_id),
        };
        self.deals.insert(deal.clone());

        current.state = DeskRequestState::Accepted as i32;
        let stored = self.requests.replace(current)?;
        // A FIX-venue lift is a taker's firm NewOrderSingle that fills atomically: the
        // ORDER arrived and the deal booked (a FILL). Emit both — distinct trader event
        // families (the GUI configures/sounds them separately) — while the explicit
        // platform quote-lift (`accept_desk_quote`) keeps QUOTE_ACCEPTED. Both quiet.
        self.publish_notification(
            NotificationKind::OrderReceived,
            &stored,
            format!("Order in — FIX lift from {}", stored.counterparty),
            Some(format!(
                "{} {:.4} on {:.0} notional — firm order on the FIX venue",
                stored.desk, deal.price, deal.notional
            )),
        );
        self.publish_notification(
            NotificationKind::Fill,
            &stored,
            format!("Fill — deal {} booked", deal.deal_id),
            Some(format!(
                "{} {:.4} on {:.0} notional — executed on the FIX venue",
                stored.desk, deal.price, deal.notional
            )),
        );
        Some(deal)
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
        // instrument side (validated priceable at submit, so always Buy/Sell) — read
        // from whichever FI family (OIS / IRS / FRA / bond) the request carries.
        let traded_side = traded_side_of(current.instrument.as_ref())
            .ok_or_else(|| Status::invalid_argument("request carries no bookable FI instrument"))?;
        let desk_side = opposite_side(traded_side);

        // Book the dealt rates position (desk perspective) at the lifted level, on the
        // ACTUAL traded family — the swap/FRA fixed rate (or bond face) struck at the
        // dealt terms, priced/booked through celnet-rates / celnet-bond.
        let booked_instrument = rebook_at_dealt_level(
            current.instrument.as_ref(),
            quote.price,
            quote.notional,
            desk_side,
        )
        .ok_or_else(|| Status::invalid_argument("request carries no bookable FI instrument"))?;
        let booked = self.rates.book_with_routing(
            RatesPosition {
                position_id: 0,
                entity: 0,
                book: 0,
                instrument: Some(booked_instrument),
            },
            RatesRoutingAttribution {
                counterparty: current.counterparty.clone(),
                ccy: current
                    .curve_set
                    .as_ref()
                    .map(|c| c.currency.clone())
                    .unwrap_or_default(),
            },
        )?;

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
            // The rates dealer-quoting desk path is not priced through a pricing-group
            // feature pipeline, so a desk deal carries no per-feature provenance
            // (design §7 — absent, never a fabricated waterfall).
            pricing_provenance: None,
            // The Risk Portfolio the fill's risk routed into, read from the booked
            // position's stamp: `None` when the firm installed no routing graph (or a
            // routing fall-back left it unrouted) — surfaced, never fabricated.
            risk_book_id: self.rates.risk_book_of(booked.position_id),
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

    type StreamRiskTransferInboxStream =
        crate::services::stream_rx::ReceiverStream<Result<celnet_proto::RiskTransferInbox, Status>>;

    async fn stream_risk_transfer_inbox(
        &self,
        request: Request<celnet_proto::StreamRiskTransferInboxRequest>,
    ) -> Result<Response<Self::StreamRiskTransferInboxStream>, Status> {
        // A draining instance refuses new streams; the guard is held for the stream
        // lifetime (the drain barrier), exactly as `stream_notifications` does.
        let guard = self.gate.enter();
        if !self.gate.is_ready() {
            return Err(Status::unavailable(
                "edge not ready (starting or draining); steer to the active instance",
            ));
        }
        let req = request.into_inner();
        let service = Arc::clone(self.transfer_service.get().ok_or_else(|| {
            Status::unavailable("risk transfer service is not wired on this edge")
        })?);
        let subscription = self.subscribe_transfer_inbox(&req)?;
        let sub_id = subscription.id;
        let mut broker_rx = subscription.rx;
        let broker = Arc::clone(service.broker());
        let (out_tx, out_rx) = tokio::sync::mpsc::channel::<
            Result<celnet_proto::RiskTransferInbox, Status>,
        >(crate::services::risk_transfer::INBOX_QUEUE_DEPTH);
        tokio::spawn(async move {
            let _guard = guard; // held for the stream lifetime (drain barrier).
            while let Some(frame) = broker_rx.recv().await {
                if out_tx.send(Ok(frame)).await.is_err() {
                    break; // the client dropped the stream.
                }
            }
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
    use celnet_proto::{CurveSet, OisInstrument, RatesInstrument};

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
        edge_with_rates(Arc::new(RatesPositionStore::new()))
    }

    /// One desk-inbox request in a given lifecycle state, always carrying a shown
    /// quote (so it counts as flow we priced) — the raw material of the FI
    /// client-flow source.
    fn desk_request(client: &str, state: DeskRequestState) -> DeskRequest {
        DeskRequest {
            request_id: format!("desk-req-{client}-{}", state as i32),
            kind: DeskRequestKind::Rfq as i32,
            counterparty: client.to_owned(),
            desk: "rates".to_owned(),
            instrument: Some(ois_instrument(Side::Buy)),
            curve_set: None,
            side: Side::Buy as i32,
            notional: 10_000_000.0,
            received_at_nanos: 1_000,
            expires_at_nanos: 2_000,
            state: state as i32,
            quote: Some(DeskQuote {
                price: 0.0405,
                notional: 10_000_000.0,
                valid_for_ms: 500,
                trader: "t".to_owned(),
            }),
            correlation_id: None,
        }
    }

    /// The FI source maps desk requests into FlowRecords whose fold reproduces the
    /// fisher-vs-converter split: a client that only ever quotes (never lifts)
    /// scores a maximal `fishing_score`; a client that converts every quote scores
    /// zero. Validated against the crate's own oracle-tested rollup (guardrail 5).
    #[test]
    fn fi_source_folds_fisher_high_and_converter_low() {
        use celnet_analytics::group_by_client;

        // GOOD converts every quote (3 quoted → 3 accepted); FISH never lifts
        // (5 quoted, 0 accepted).
        let mut requests: Vec<DeskRequest> = (0..3)
            .map(|i| {
                let mut r = desk_request("GOOD", DeskRequestState::Accepted);
                r.request_id = format!("good-{i}");
                r
            })
            .collect();
        for i in 0..5 {
            let mut r = desk_request("FISH", DeskRequestState::Quoted);
            r.request_id = format!("fish-{i}");
            requests.push(r);
        }

        let records: Vec<_> = requests
            .iter()
            .filter_map(|r| desk_request_to_flow(r, None, None))
            .collect();
        let by_client = group_by_client(&records);

        // Converter: full hit-rate, zero fishing, all traded notional captured.
        let good = &by_client["GOOD"];
        assert_eq!(good.quote_count, 3);
        assert_eq!(good.traded_count, 3);
        assert_eq!(good.hit_rate, Some(1.0));
        assert_eq!(good.fishing_score, 0.0);
        assert_eq!(good.traded_notional, 30_000_000.0);
        // The FI desk path is untiered ⇒ no provenance margin (honest zero/None).
        assert_eq!(good.gross_pnl, 0.0);
        assert_eq!(good.dpm_gross, Some(0.0));

        // Fisher: zero hit-rate, maximal fishing score, no trades.
        let fish = &by_client["FISH"];
        assert_eq!(fish.quote_count, 5);
        assert_eq!(fish.traded_count, 0);
        assert_eq!(fish.hit_rate, Some(0.0));
        assert_eq!(fish.fishing_score, 1.0);
        assert_eq!(fish.dpm_net, None);

        // Window filter excludes out-of-range receipts (received_at_nanos = 1_000).
        let windowed: Vec<_> = requests
            .iter()
            .filter_map(|r| desk_request_to_flow(r, Some(5_000), None))
            .collect();
        assert!(windowed.is_empty());

        // A request we never quoted contributes nothing (not priced flow).
        let mut unquoted = desk_request("X", DeskRequestState::Pending);
        unquoted.quote = None;
        assert!(desk_request_to_flow(&unquoted, None, None).is_none());
    }

    /// An edge over a caller-supplied rates store — so a test can install a firm-wide
    /// risk-routing graph on the store before booking and assert the routed
    /// `Deal.risk_book_id` the desk path stamps.
    fn edge_with_rates(rates: Arc<RatesPositionStore>) -> RfqDeskEdge {
        let gate = Arc::new(ReadinessGate::new());
        gate.mark_ready();
        RfqDeskEdge::new(
            Arc::new(PositionStore::new()),
            Arc::new(SessionRegistry::new(Clock::manual(0))),
            gate,
            Arc::new(DeskRequestStore::new()),
            Arc::new(DealStore::new()),
            rates,
            Arc::new(NotificationBroker::new()),
            Clock::manual(1_000),
        )
    }

    fn submit_req(kind: DeskRequestKind, side: Side) -> SubmitDeskRequestRequest {
        submit_req_for(kind, side, ois_instrument(side))
    }

    /// A submit request carrying an arbitrary FI instrument (any of OIS / IRS / FRA /
    /// bond) — so the execute/booking path is exercised across every streamed family.
    fn submit_req_for(
        kind: DeskRequestKind,
        side: Side,
        instrument: RatesInstrument,
    ) -> SubmitDeskRequestRequest {
        SubmitDeskRequestRequest {
            session_token: None,
            kind: kind as i32,
            counterparty: "cp-bank".to_owned(),
            desk: "g10".to_owned(),
            instrument: Some(instrument),
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

    /// A par-ish 5y vanilla IRS on the `curve()` pillars (side-parametrised). Wire
    /// enums match the canonical `rates_pricing` IRS fixture exactly.
    fn irs_instrument(side: Side) -> RatesInstrument {
        RatesInstrument {
            instrument: Some(rates_instrument::Instrument::Irs(
                celnet_proto::VanillaIrsInstrument {
                    tenor_years: 5,
                    fixed_rate: 0.041,
                    notional: 25_000_000.0,
                    side: side as i32,
                    fixed_frequency: celnet_proto::PaymentFrequency::SemiAnnual as i32,
                    fixed_day_count: celnet_proto::AccrualBasis::Act360 as i32,
                    float_frequency: celnet_proto::PaymentFrequency::Quarterly as i32,
                    float_day_count: celnet_proto::AccrualBasis::Act360 as i32,
                },
            )),
        }
    }

    /// A 3x6 FRA on the `curve()` pillars (side-parametrised).
    fn fra_instrument(side: Side) -> RatesInstrument {
        RatesInstrument {
            instrument: Some(rates_instrument::Instrument::Fra(
                celnet_proto::FraInstrument {
                    start_months: 3,
                    end_months: 6,
                    fixed_rate: 0.042,
                    notional: 25_000_000.0,
                    side: side as i32,
                    accrual_basis: celnet_proto::AccrualBasis::Act360 as i32,
                },
            )),
        }
    }

    /// A 5y 4% semi-annual cash bond redeeming at par (side-parametrised).
    fn bond_instrument(side: Side) -> RatesInstrument {
        RatesInstrument {
            instrument: Some(rates_instrument::Instrument::Bond(
                celnet_proto::BondInstrument {
                    coupon_rate: 0.04,
                    coupon_frequency: celnet_proto::PaymentFrequency::SemiAnnual as i32,
                    day_count: celnet_proto::AccrualBasis::Thirty360BondBasis as i32,
                    maturity_date: Some(celnet_proto::BrokenDate {
                        year: 2031,
                        month: 6,
                        day: 1,
                    }),
                    redemption: 100.0,
                    side: side as i32,
                },
            )),
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
                desk_ids: vec!["g10".to_owned()],
                all_desks: false,
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
        // The booked position carries the dealt level + desk side, on the OIS arm.
        let pos = &edge.rates.snapshot()[0];
        let Some(rates_instrument::Instrument::Ois(booked_ois)) =
            pos.instrument.as_ref().and_then(|i| i.instrument.as_ref())
        else {
            panic!("booked position must carry the OIS arm");
        };
        assert_eq!(booked_ois.side, Side::Sell as i32);
        assert_eq!(booked_ois.fixed_rate.to_bits(), 0.0411_f64.to_bits());
    }

    /// Drive submit → respond(quote@`dealt`) → accept for an arbitrary FI family and
    /// return the booked rates position. Asserts the deal books (desk side = opposite
    /// of the counterparty's) and exactly one position lands.
    async fn book_family_and_return_position(
        instrument: RatesInstrument,
        dealt: f64,
    ) -> (RfqDeskEdge, RatesPosition) {
        let edge = edge();
        let token = trader_token(&edge);
        let submitted = edge
            .submit_desk_request(Request::new(submit_req_for(
                DeskRequestKind::Rfq,
                Side::Buy,
                instrument,
            )))
            .await
            .expect("submit")
            .into_inner()
            .request
            .expect("request");
        let id = submitted.request_id.clone();

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
                price: dealt,
                notional: 25_000_000.0,
                valid_for_ms: 30_000,
                trader: "alice".to_owned(),
            })),
        }))
        .await
        .expect("respond");

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
        // Counterparty bought ⇒ desk sells.
        assert_eq!(deal.side, Side::Sell as i32);
        assert!(deal.position_id.is_some());
        assert_eq!(edge.rates.len(), 1);
        let pos = edge.rates.snapshot()[0];
        (edge, pos)
    }

    /// The booked position prices to a finite, non-trivial PV and DV01 through the
    /// shared pricer — proving it is a *real* booked risk trade, not a fabricated fill.
    fn assert_position_has_live_risk(pos: &RatesPosition) {
        let priced = price_rates(&RatesPriceRequest {
            request_id: 0,
            curve_set: Some(curve()),
            instrument: pos.instrument,
            correlation_id: None,
        })
        .expect("booked position prices");
        assert!(priced.pv.is_finite(), "booked PV must be finite");
        assert!(priced.dv01.is_finite(), "booked DV01 must be finite");
        assert!(
            priced.dv01.abs() > 0.0,
            "a booked rate/price position carries non-zero DV01"
        );
    }

    /// PART B: an **IRS** desk request executes and books a correct IRS position on the
    /// desk's side at the dealt fixed rate — priced/booked through `celnet-rates`.
    #[tokio::test]
    async fn irs_desk_request_books_a_correct_position() {
        let (_edge, pos) = book_family_and_return_position(irs_instrument(Side::Buy), 0.0415).await;
        let Some(rates_instrument::Instrument::Irs(booked)) =
            pos.instrument.as_ref().and_then(|i| i.instrument.as_ref())
        else {
            panic!("booked position must carry the IRS arm");
        };
        assert_eq!(booked.side, Side::Sell as i32, "desk receives fixed");
        assert_eq!(booked.fixed_rate.to_bits(), 0.0415_f64.to_bits());
        assert_eq!(booked.tenor_years, 5, "the traded schedule is preserved");
        assert_position_has_live_risk(&pos);
    }

    /// PART B: an **FRA** desk request executes and books a correct FRA position on the
    /// desk's side at the dealt fixed rate — priced/booked through `celnet-rates`.
    #[tokio::test]
    async fn fra_desk_request_books_a_correct_position() {
        let (_edge, pos) = book_family_and_return_position(fra_instrument(Side::Buy), 0.0435).await;
        let Some(rates_instrument::Instrument::Fra(booked)) =
            pos.instrument.as_ref().and_then(|i| i.instrument.as_ref())
        else {
            panic!("booked position must carry the FRA arm");
        };
        assert_eq!(booked.side, Side::Sell as i32);
        assert_eq!(booked.fixed_rate.to_bits(), 0.0435_f64.to_bits());
        assert_eq!(booked.start_months, 3, "the traded window is preserved");
        assert_eq!(booked.end_months, 6);
        assert_position_has_live_risk(&pos);
    }

    /// PART B: a **cash bond** desk request executes and books a correct bond position
    /// on the desk's side at the dealt clean price — priced/booked through `celnet-bond`.
    /// The bond's coupon/maturity are the security's own (unchanged); the dealt clean
    /// price is recorded on the deal, and only face + side are set on the position.
    #[tokio::test]
    async fn bond_desk_request_books_a_correct_position() {
        let dealt_clean = 99.25;
        let (edge, pos) =
            book_family_and_return_position(bond_instrument(Side::Buy), dealt_clean).await;
        let Some(rates_instrument::Instrument::Bond(booked)) =
            pos.instrument.as_ref().and_then(|i| i.instrument.as_ref())
        else {
            panic!("booked position must carry the bond arm");
        };
        assert_eq!(booked.side, Side::Sell as i32);
        // The security's coupon is the traded bond's, unchanged by the dealt price; the
        // booked face is the traded notional (the position's size), and the executed
        // clean price lives on the deal.
        assert_eq!(booked.coupon_rate.to_bits(), 0.04_f64.to_bits());
        assert_eq!(booked.redemption.to_bits(), 25_000_000.0_f64.to_bits());
        // The executed clean price lives on the deal, not the instrument.
        assert_eq!(
            edge.deals.snapshot()[0].price.to_bits(),
            dealt_clean.to_bits()
        );
        assert_position_has_live_risk(&pos);
    }

    /// PART B: OIS still executes and books correctly after the generalisation (the
    /// legacy path is preserved, not regressed).
    #[tokio::test]
    async fn ois_desk_request_still_books_after_generalisation() {
        let (_edge, pos) = book_family_and_return_position(ois_instrument(Side::Buy), 0.0409).await;
        let Some(rates_instrument::Instrument::Ois(booked)) =
            pos.instrument.as_ref().and_then(|i| i.instrument.as_ref())
        else {
            panic!("booked position must carry the OIS arm");
        };
        assert_eq!(booked.side, Side::Sell as i32);
        assert_eq!(booked.fixed_rate.to_bits(), 0.0409_f64.to_bits());
        assert_position_has_live_risk(&pos);
    }

    /// PART B: a desk request carrying no instrument arm is rejected cleanly at submit
    /// (unpriceable), never reaching the booking path.
    #[tokio::test]
    async fn desk_request_with_no_instrument_is_rejected() {
        let edge = edge();
        let mut req = submit_req(DeskRequestKind::Rfq, Side::Buy);
        req.instrument = Some(RatesInstrument { instrument: None });
        let err = edge
            .submit_desk_request(Request::new(req))
            .await
            .expect_err("an armless instrument must be rejected");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert_eq!(
            edge.rates.len(),
            0,
            "nothing is booked on a rejected request"
        );
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

    /// A FIX-venue lift (a taker's firm NewOrderSingle that fills atomically) publishes
    /// ORDER_RECEIVED then FILL — the two phase-5 arms — distinct from the platform
    /// `accept_desk_quote` path, which stays QUOTE_ACCEPTED. Both are quiet.
    #[tokio::test]
    async fn book_fix_lift_publishes_order_received_then_fill() {
        let edge = edge();
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

        // Subscribe AFTER quoting so the channel carries only the lift's notifications.
        let mut sub = edge.notify.subscribe(DeskFilter::All);
        edge.book_fix_lift(&id).expect("fix lift books a deal");
        assert_eq!(edge.deals.len(), 1);

        let first = sub.rx.try_recv().expect("order-received published");
        assert_eq!(first.kind, NotificationKind::OrderReceived as i32);
        assert!(
            !first.alert_worthy,
            "an inbound order is quiet, not alert-worthy"
        );
        let second = sub.rx.try_recv().expect("fill published");
        assert_eq!(second.kind, NotificationKind::Fill as i32);
        assert!(!second.alert_worthy, "a fill is quiet, not alert-worthy");
        assert_eq!(second.request_id.as_deref(), Some(id.as_str()));
    }

    /// Drive a QUOTED desk request to a FIX-lift booking and return the booked [`Deal`].
    /// Shared by the routed / unrouted `risk_book_id` assertions below.
    async fn lift_a_deal(edge: &RfqDeskEdge) -> Deal {
        let token = trader_token(edge);
        let id = edge
            .submit_desk_request(Request::new(submit_req(DeskRequestKind::Rfq, Side::Buy)))
            .await
            .expect("submit")
            .into_inner()
            .request
            .expect("request")
            .request_id;
        edge.respond_desk_request(Request::new(RespondDeskRequestRequest {
            session_token: Some(token),
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
        edge.book_fix_lift(&id).expect("fix lift books a deal")
    }

    /// A firm-wide graph routing on the booking-time counterparty attribution the desk
    /// supplies: `counterparty == "cp-bank"` → BOOK-FI, else DEFAULT.
    fn counterparty_graph() -> celnet_risk_routing::RiskRoutingGraph {
        use celnet_risk_routing::{RouteField, RouteOp, RouteValue, RoutingNode};
        use std::collections::BTreeMap;
        let mut nodes = BTreeMap::new();
        nodes.insert(
            0u32,
            RoutingNode::Condition {
                field: RouteField::Counterparty,
                op: RouteOp::Eq,
                value: RouteValue::Text("cp-bank".to_owned()),
                on_true: 1,
                on_false: 2,
            },
        );
        nodes.insert(
            1u32,
            RoutingNode::Book {
                risk_book_id: "BOOK-FI".to_owned(),
            },
        );
        nodes.insert(
            2u32,
            RoutingNode::Book {
                risk_book_id: "DEFAULT".to_owned(),
            },
        );
        celnet_risk_routing::RiskRoutingGraph { entry: 0, nodes }
    }

    /// With a firm-wide risk-routing graph installed, a FIX-lift booking stamps the
    /// resolved Risk Portfolio onto the [`Deal`] (read from the booked position's
    /// `risk_book_of` stamp) — the counterparty attribution the desk supplies routes
    /// `cp-bank` into BOOK-FI.
    #[tokio::test]
    async fn book_fix_lift_deal_carries_routed_risk_book_id() {
        let rates = Arc::new(RatesPositionStore::new());
        rates.set_routing(Some(counterparty_graph()));
        let edge = edge_with_rates(rates);
        let deal = lift_a_deal(&edge).await;
        assert_eq!(deal.risk_book_id.as_deref(), Some("BOOK-FI"));
        // And it matches the store's own stamp for the booked position.
        assert_eq!(
            deal.risk_book_id,
            edge.rates
                .risk_book_of(deal.position_id.expect("booked position id"))
        );
    }

    /// With NO routing graph installed, a FIX-lift booking leaves `risk_book_id` absent —
    /// an unrouted fill is never assigned a fabricated portfolio.
    #[tokio::test]
    async fn book_fix_lift_deal_is_unrouted_without_graph() {
        let edge = edge();
        assert!(!edge.rates.has_routing());
        let deal = lift_a_deal(&edge).await;
        assert_eq!(deal.risk_book_id, None);
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

        // Auto-quoted: carries a firm quote ⇒ QUOTED history, quiet QUOTE_ACCEPTED.
        let auto = edge.ingest_fix_rfq(
            "g10-rates",
            "CELER_RATES",
            ois_instrument(Side::Buy),
            curve(),
            Side::Buy,
            10_000_000.0,
            RfqIngestOutcome::AutoQuoted(DeskQuote {
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
        assert!(!n.alert_worthy, "an auto-quote is quiet (no popup)");
        assert_eq!(n.reason, None);

        // Routed to a human as a routine RFQ ⇒ PENDING, quiet RFQ_RECEIVED.
        let manual = edge.ingest_fix_rfq(
            "g10-rates",
            "CELER_RATES",
            ois_instrument(Side::Sell),
            curve(),
            Side::Sell,
            50_000_000.0,
            RfqIngestOutcome::RoutedToDesk,
        );
        assert_eq!(manual.state, DeskRequestState::Pending as i32);
        assert!(manual.quote.is_none());
        let n = sub.rx.try_recv().expect("a routed RFQ fans a notification");
        assert_eq!(n.kind, NotificationKind::RfqReceived as i32);
        assert!(!n.alert_worthy, "a routine routed RFQ is quiet (no popup)");
        assert_eq!(n.reason, None);

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

    /// A manual-intervention ingest lands PENDING and fans an ALERT-worthy
    /// `MANUAL_INTERVENTION_REQUIRED` notification carrying the machine-readable reason —
    /// the ONLY path that gates a GUI popup.
    #[tokio::test]
    async fn ingest_fix_rfq_manual_intervention_is_alert_worthy_with_reason() {
        let edge = edge();
        let mut sub = edge.notify.subscribe(DeskFilter::All);
        let manual = edge.ingest_fix_rfq(
            "g10-rates",
            "CELER_RATES",
            ois_instrument(Side::Buy),
            curve(),
            Side::Buy,
            10_000_000.0,
            RfqIngestOutcome::ManualIntervention(ManualInterventionReason::UnconfiguredTenor),
        );
        assert_eq!(manual.state, DeskRequestState::Pending as i32);
        assert!(manual.quote.is_none(), "no auto quote on a manual RFQ");
        let n = sub
            .rx
            .try_recv()
            .expect("a manual-intervention RFQ fans a notification");
        assert_eq!(
            n.kind,
            NotificationKind::ManualInterventionRequired as i32,
            "manual-intervention carries the dedicated kind"
        );
        assert!(
            n.alert_worthy,
            "manual-intervention is alert-worthy (popup)"
        );
        assert_eq!(
            n.reason,
            Some(ManualInterventionReason::UnconfiguredTenor as i32),
            "the machine-readable reason rides the notification"
        );
    }

    /// `effective_desk_filter` intersects requested desks with the caller's scope.
    #[test]
    fn desk_filter_intersects_scope() {
        let g10 = || DeskScope::Desks(["g10".to_owned()].into_iter().collect());
        assert_eq!(effective_desk_filter(&[], &DeskScope::All), DeskFilter::All);
        assert_eq!(
            effective_desk_filter(&[], &g10()),
            DeskFilter::Desks(["g10".to_owned()].into_iter().collect())
        );
        // Requesting a desk the caller cannot see yields an empty filter.
        assert_eq!(
            effective_desk_filter(&["em".to_owned()], &g10()),
            DeskFilter::Desks(std::collections::HashSet::new())
        );
        assert_eq!(
            effective_desk_filter(&["em".to_owned()], &DeskScope::Deskless),
            DeskFilter::Desks(std::collections::HashSet::new())
        );
    }

    /// A multi-desk caller (desks {g10, em}) receives notifications for BOTH desks
    /// when no scope is requested, and the intersection when a scope is requested.
    #[test]
    fn desk_filter_multi_desk_membership() {
        let both = DeskScope::Desks(["g10".to_owned(), "em".to_owned()].into_iter().collect());
        // No request ⇒ the caller's full set.
        assert_eq!(
            effective_desk_filter(&[], &both),
            DeskFilter::Desks(["g10".to_owned(), "em".to_owned()].into_iter().collect())
        );
        // A request narrows to the intersection (em kept, ny dropped).
        assert_eq!(
            effective_desk_filter(&["em".to_owned(), "ny".to_owned()], &both),
            DeskFilter::Desks(["em".to_owned()].into_iter().collect())
        );
        // The resulting filter admits both owned desks, not a third.
        let f = effective_desk_filter(&[], &both);
        assert!(f.allows("g10") && f.allows("em") && !f.allows("ny"));
    }
}
