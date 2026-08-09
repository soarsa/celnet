//! The dealer-side quoting desk — the typed SDK face of the `RfqDeskService`
//! contract (`SubmitDeskRequest` / `RespondDeskRequest` / `AcceptDeskQuote` /
//! `ListDeskRequests` / `ListDeals`).
//!
//! This is the MAKER side of the franchise: a counterparty submits an RFQ/IOI to a
//! desk, the responsible trader is notified (see [`crate::Client::stream_notifications`]),
//! prices and responds, and an accepted quote books a deal into the received-deals
//! blotter AND a rates position. An external client — an SDK user, the GUI, the
//! Excel add-in — drives the SAME desk contract through this handle: it never
//! reaches past the SDK into raw `celnet-proto` messages (the api-first parity rule).
//!
//! # Idempotency & correlation
//!
//! The desk lifecycle is keyed on the SERVER-assigned [`DeskRequest::request_id`]:
//! [`DeskClient::submit`] returns it, and [`DeskClient::respond_quote`] /
//! [`DeskClient::decline`] / [`DeskClient::accept`] all reference it — the request
//! id is the dedup handle the contract provides (a re-`accept` of an already-booked
//! request is refused server-side by the `QUOTED`-state precondition, never a
//! double-book). The contract carries no client idempotency key on these RPCs; a
//! caller's own join handle is the optional per-message `correlation_id`, echoed
//! through the lifecycle.
//!
//! # Authorization
//!
//! [`DeskClient::submit`] / [`DeskClient::list_requests`] / [`DeskClient::list_deals`]
//! are principal-gated (`ReadAny`): the SDK asserts the audited explicit grant-all
//! when a caller pins no principal, so they clear the production deny-by-default
//! edge exactly as the risk reads do. [`DeskClient::respond_quote`] /
//! [`DeskClient::decline`] (`RfqRespond`/`IoiRespond·FixedIncome`) and
//! [`DeskClient::accept`] (`Execute·FixedIncome`) are capability-gated: a capability
//! resolves ONLY from an authenticated session, so a deployment attaches a real
//! `AuthService.Login` bearer via [`crate::Client::with_session_token`] before the
//! desk trader responds or books.

use std::time::Duration;

use celnet_proto::convert::WireError;
use celnet_proto::rfq_desk_service_client::RfqDeskServiceClient;
use celnet_proto::{
    AcceptDeskQuoteRequest, Deal as WireDeal, DealScope, DeskQuote as WireDeskQuote, DeskReject,
    DeskRequest as WireDeskRequest, DeskRequestKind as WireDeskRequestKind, DeskRequestScope,
    DeskRequestState as WireDeskRequestState, ListDealsRequest, ListDeskRequestsRequest,
    RespondDeskRequestRequest, SubmitDeskRequestRequest,
    respond_desk_request_request::Response as RespondArm,
};
use tonic::transport::Channel;

use crate::error::{ClientError, ClientResult};
use crate::rates::{Ois, OisSide, UsdSofrCurve};
use crate::risk::{Entitlements, principal_or_grant_all};
use crate::vocab::Side;

/// What kind of inbound request this is — a firm-price request-for-quote, or a
/// non-firm indication-of-interest the desk works. The typed form of the wire
/// `DeskRequestKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeskRequestKind {
    /// A counterparty wants a firm price.
    Rfq,
    /// A non-firm axe for the desk to work.
    Ioi,
    /// A lift of a continuously-streamed executable price (the ESP / FixedIncomeStream venue).
    Esp,
}

impl DeskRequestKind {
    fn to_wire(self) -> WireDeskRequestKind {
        match self {
            DeskRequestKind::Rfq => WireDeskRequestKind::Rfq,
            DeskRequestKind::Ioi => WireDeskRequestKind::Ioi,
            DeskRequestKind::Esp => WireDeskRequestKind::Esp,
        }
    }

    /// Decode a wire `DeskRequestKind` tag; the `UNSPECIFIED` sentinel and any
    /// out-of-range tag are typed errors, never a silent default. Shared with the
    /// notification decode ([`crate::Notification`]).
    pub(crate) fn from_wire_tag(tag: i32) -> ClientResult<Self> {
        match WireDeskRequestKind::try_from(tag) {
            Ok(WireDeskRequestKind::Rfq) => Ok(DeskRequestKind::Rfq),
            Ok(WireDeskRequestKind::Ioi) => Ok(DeskRequestKind::Ioi),
            Ok(WireDeskRequestKind::Esp) => Ok(DeskRequestKind::Esp),
            _ => Err(ClientError::Wire(WireError::UnknownEnum {
                kind: "DeskRequestKind",
                tag,
            })),
        }
    }
}

/// The lifecycle state of an inbound desk request (one current fact per id). The
/// typed form of the wire `DeskRequestState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeskRequestState {
    /// Awaiting a desk response.
    Pending,
    /// The desk has responded with a firm price.
    Quoted,
    /// The counterparty lifted the quote → a deal booked.
    Accepted,
    /// The desk declined to quote.
    Rejected,
    /// The response deadline elapsed unquoted.
    Expired,
    /// The counterparty pulled the request.
    Withdrawn,
}

impl DeskRequestState {
    fn from_wire(tag: i32) -> ClientResult<Self> {
        match WireDeskRequestState::try_from(tag) {
            Ok(WireDeskRequestState::Pending) => Ok(DeskRequestState::Pending),
            Ok(WireDeskRequestState::Quoted) => Ok(DeskRequestState::Quoted),
            Ok(WireDeskRequestState::Accepted) => Ok(DeskRequestState::Accepted),
            Ok(WireDeskRequestState::Rejected) => Ok(DeskRequestState::Rejected),
            Ok(WireDeskRequestState::Expired) => Ok(DeskRequestState::Expired),
            Ok(WireDeskRequestState::Withdrawn) => Ok(DeskRequestState::Withdrawn),
            _ => Err(ClientError::Wire(WireError::UnknownEnum {
                kind: "DeskRequestState",
                tag,
            })),
        }
    }

    /// The state token as sent on a [`DeskRequestFilter`] states filter.
    fn to_wire(self) -> WireDeskRequestState {
        match self {
            DeskRequestState::Pending => WireDeskRequestState::Pending,
            DeskRequestState::Quoted => WireDeskRequestState::Quoted,
            DeskRequestState::Accepted => WireDeskRequestState::Accepted,
            DeskRequestState::Rejected => WireDeskRequestState::Rejected,
            DeskRequestState::Expired => WireDeskRequestState::Expired,
            DeskRequestState::Withdrawn => WireDeskRequestState::Withdrawn,
        }
    }
}

/// The OIS side an [`Ois`] instrument maps to on the desk request's top-level `side`
/// field (the counterparty's directional perspective): pay-fixed ⇒ [`Side::Buy`],
/// receive-fixed ⇒ [`Side::Sell`].
fn side_of(ois: OisSide) -> Side {
    match ois {
        OisSide::PayFixed => Side::Buy,
        OisSide::ReceiveFixed => Side::Sell,
    }
}

/// Decode a wire `Side` tag into the typed [`Side`].
fn side_from_wire(tag: i32) -> ClientResult<Side> {
    celnet_proto::Side::try_from(tag)
        .map(Side::from_wire)
        .map_err(|_| ClientError::Wire(WireError::UnknownEnum { kind: "Side", tag }))
}

/// The desk's firm response to an RFQ (or a firm level on an IOI): the quoted price,
/// the notional it stands for, how long it stands, and the quoting trader. Doubles
/// as the outbound builder ([`DeskClient::respond_quote`]) and the decoded form
/// carried on a [`DeskRequest::quote`]. The typed form of the wire `DeskQuote`.
///
/// For the OIS arm the `price` is the all-in fixed rate as a decimal (`0.041` =
/// 4.10%) the desk shows the counterparty.
#[derive(Debug, Clone, PartialEq)]
pub struct DeskQuote {
    /// The quoted price (the OIS all-in fixed rate as a decimal).
    pub price: f64,
    /// The notional the quote is good for (curve currency, positive).
    pub notional: f64,
    /// How long the quote stands from issue. `Duration::ZERO` ⇒ indicative only.
    pub valid_for: Duration,
    /// The quoting trader (attribution).
    pub trader: String,
}

impl DeskQuote {
    /// A firm quote of `price` good for `notional` (curve currency), standing for
    /// `valid_for`, from `trader`.
    #[must_use]
    pub fn new(price: f64, notional: f64, valid_for: Duration, trader: impl Into<String>) -> Self {
        Self {
            price,
            notional,
            valid_for,
            trader: trader.into(),
        }
    }

    fn to_wire(&self) -> WireDeskQuote {
        WireDeskQuote {
            price: self.price,
            notional: self.notional,
            valid_for_ms: u32::try_from(self.valid_for.as_millis()).unwrap_or(u32::MAX),
            trader: self.trader.clone(),
        }
    }

    fn from_wire(w: &WireDeskQuote) -> Self {
        Self {
            price: w.price,
            notional: w.notional,
            valid_for: Duration::from_millis(u64::from(w.valid_for_ms)),
            trader: w.trader.clone(),
        }
    }
}

/// One inbound dealer-side request (RFQ or IOI) as the desk sees it — the typed form
/// of the wire `DeskRequest`. The instrument is decoded to the typed [`Ois`]; the
/// `side` is the COUNTERPARTY's directional perspective (`Buy` ⇒ they pay fixed).
#[derive(Debug, Clone, PartialEq)]
pub struct DeskRequest {
    /// Server-assigned, stable across the lifecycle — the dedup/reference handle.
    pub request_id: String,
    /// Whether this is an RFQ (firm price wanted) or an IOI (axe to work).
    pub kind: DeskRequestKind,
    /// The counterparty label (the taker). A display/attribution field, not auth.
    pub counterparty: String,
    /// The target desk this request routes to (the entitlement + notification scope).
    pub desk: String,
    /// The instrument the counterparty wants priced (the USD-SOFR OIS arm).
    pub instrument: Ois,
    /// The counterparty's directional side (`Buy` ⇒ they pay fixed).
    pub side: Side,
    /// The notional requested (curve currency, always positive).
    pub notional: f64,
    /// Server receipt time (epoch nanos, UTC).
    pub received_at_nanos: i64,
    /// Response deadline (epoch nanos, UTC); past this the request EXPIRES unquoted.
    pub expires_at_nanos: i64,
    /// The current lifecycle state (one current fact per `request_id`).
    pub state: DeskRequestState,
    /// The desk's response once quoted (absent while PENDING).
    pub quote: Option<DeskQuote>,
    /// Opaque caller correlation id, echoed through the lifecycle.
    pub correlation_id: Option<String>,
}

impl DeskRequest {
    fn from_wire(w: &WireDeskRequest) -> ClientResult<Self> {
        let instrument = w
            .instrument
            .as_ref()
            .ok_or(ClientError::MissingField("DeskRequest.instrument"))
            .and_then(Ois::from_wire)?;
        Ok(Self {
            request_id: w.request_id.clone(),
            kind: DeskRequestKind::from_wire_tag(w.kind)?,
            counterparty: w.counterparty.clone(),
            desk: w.desk.clone(),
            instrument,
            side: side_from_wire(w.side)?,
            notional: w.notional,
            received_at_nanos: w.received_at_nanos,
            expires_at_nanos: w.expires_at_nanos,
            state: DeskRequestState::from_wire(w.state)?,
            quote: w.quote.as_ref().map(DeskQuote::from_wire),
            correlation_id: w.correlation_id.clone(),
        })
    }
}

/// One booked received deal (an accepted desk quote), as the blotter shows it — the
/// typed form of the wire `Deal`. The `side` is the DESK's side (opposite the
/// counterparty's submitted side).
#[derive(Debug, Clone, PartialEq)]
pub struct Deal {
    /// The server-assigned deal identity.
    pub deal_id: String,
    /// The originating [`DeskRequest::request_id`].
    pub request_id: String,
    /// Whether the originating request was an RFQ or an IOI.
    pub kind: DeskRequestKind,
    /// The counterparty label (display/attribution).
    pub counterparty: String,
    /// The desk the deal booked under.
    pub desk: String,
    /// The dealt instrument (the USD-SOFR OIS arm), from the desk's perspective.
    pub instrument: Ois,
    /// The DESK's side (opposite the counterparty's submitted side).
    pub side: Side,
    /// The dealt notional (curve currency, positive).
    pub notional: f64,
    /// The dealt level — the accepted [`DeskQuote::price`].
    pub price: f64,
    /// Execution time (epoch nanos, UTC).
    pub executed_at_nanos: i64,
    /// The booking trader (attribution).
    pub trader: String,
    /// The rates position id this deal booked into the rates store.
    pub position_id: Option<u64>,
    /// Opaque caller correlation id carried from the request.
    pub correlation_id: Option<String>,
}

impl Deal {
    fn from_wire(w: &WireDeal) -> ClientResult<Self> {
        let instrument = w
            .instrument
            .as_ref()
            .ok_or(ClientError::MissingField("Deal.instrument"))
            .and_then(Ois::from_wire)?;
        Ok(Self {
            deal_id: w.deal_id.clone(),
            request_id: w.request_id.clone(),
            kind: DeskRequestKind::from_wire_tag(w.kind)?,
            counterparty: w.counterparty.clone(),
            desk: w.desk.clone(),
            instrument,
            side: side_from_wire(w.side)?,
            notional: w.notional,
            price: w.price,
            executed_at_nanos: w.executed_at_nanos,
            trader: w.trader.clone(),
            position_id: w.position_id,
            correlation_id: w.correlation_id.clone(),
        })
    }
}

/// The result of lifting a quoted desk request ([`DeskClient::accept`]): the booked
/// [`Deal`] and the originating [`DeskRequest`] in its terminal `ACCEPTED` state.
#[derive(Debug, Clone, PartialEq)]
pub struct DeskAcceptance {
    /// The deal booked from the lifted quote.
    pub deal: Deal,
    /// The request in its terminal `ACCEPTED` state.
    pub request: DeskRequest,
}

/// A fluent builder for an inbound desk request (`SubmitDeskRequest`): the taker's
/// RFQ/IOI to a desk. The top-level `side` and `notional` the desk sees are taken
/// from the [`Ois`] instrument (its direction and notional), keeping the request
/// self-consistent. Build with [`DeskRfq::rfq`] / [`DeskRfq::ioi`] and submit with
/// [`DeskClient::submit`].
#[derive(Debug, Clone)]
pub struct DeskRfq {
    kind: DeskRequestKind,
    counterparty: String,
    desk: String,
    instrument: Ois,
    curve: UsdSofrCurve,
    ttl: Duration,
    correlation_id: Option<String>,
}

impl DeskRfq {
    /// A request-for-quote (firm price wanted) from `counterparty` to `desk` for
    /// `instrument`, priced against `curve`.
    #[must_use]
    pub fn rfq(
        counterparty: impl Into<String>,
        desk: impl Into<String>,
        instrument: Ois,
        curve: UsdSofrCurve,
    ) -> Self {
        Self::of(DeskRequestKind::Rfq, counterparty, desk, instrument, curve)
    }

    /// An indication-of-interest (a non-firm axe) from `counterparty` to `desk`.
    #[must_use]
    pub fn ioi(
        counterparty: impl Into<String>,
        desk: impl Into<String>,
        instrument: Ois,
        curve: UsdSofrCurve,
    ) -> Self {
        Self::of(DeskRequestKind::Ioi, counterparty, desk, instrument, curve)
    }

    fn of(
        kind: DeskRequestKind,
        counterparty: impl Into<String>,
        desk: impl Into<String>,
        instrument: Ois,
        curve: UsdSofrCurve,
    ) -> Self {
        Self {
            kind,
            counterparty: counterparty.into(),
            desk: desk.into(),
            instrument,
            curve,
            ttl: Duration::ZERO,
            correlation_id: None,
        }
    }

    /// Set the time-to-live for the desk to respond. `Duration::ZERO` (the default)
    /// ⇒ the server's default TTL.
    #[must_use]
    pub fn ttl(mut self, ttl: Duration) -> Self {
        self.ttl = ttl;
        self
    }

    /// Attach a caller correlation id, echoed through the lifecycle.
    #[must_use]
    pub fn correlation_id(mut self, id: impl Into<String>) -> Self {
        self.correlation_id = Some(id.into());
        self
    }

    fn to_wire(
        &self,
        session_token: Option<String>,
        principal: Option<&Entitlements>,
    ) -> SubmitDeskRequestRequest {
        SubmitDeskRequestRequest {
            session_token,
            kind: self.kind.to_wire() as i32,
            counterparty: self.counterparty.clone(),
            desk: self.desk.clone(),
            instrument: Some(self.instrument.to_wire()),
            curve_set: Some(self.curve.to_wire()),
            side: side_of(self.instrument.side()).to_wire() as i32,
            notional: self.instrument.notional_amount(),
            ttl_ms: u32::try_from(self.ttl.as_millis()).unwrap_or(u32::MAX),
            principal: Some(principal_or_grant_all(principal)),
            correlation_id: self.correlation_id.clone(),
        }
    }
}

/// A scope filter for the desk inbox read ([`DeskClient::list_requests`]): restrict
/// to a set of lifecycle states and/or one desk. The default admits every state
/// across every desk the caller is entitled to.
#[derive(Debug, Clone, Default)]
pub struct DeskRequestFilter {
    states: Vec<DeskRequestState>,
    desk: Option<String>,
}

impl DeskRequestFilter {
    /// The unfiltered inbox (all states, all entitled desks).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Restrict to these lifecycle `states` (additive across calls).
    #[must_use]
    pub fn states(mut self, states: impl IntoIterator<Item = DeskRequestState>) -> Self {
        self.states.extend(states);
        self
    }

    /// Restrict to one desk.
    #[must_use]
    pub fn desk(mut self, desk: impl Into<String>) -> Self {
        self.desk = Some(desk.into());
        self
    }

    fn to_wire(&self) -> Option<DeskRequestScope> {
        if self.states.is_empty() && self.desk.is_none() {
            return None;
        }
        Some(DeskRequestScope {
            states: self.states.iter().map(|s| s.to_wire() as i32).collect(),
            desk: self.desk.clone(),
        })
    }
}

/// A scope filter for the received-deals blotter read ([`DeskClient::list_deals`]):
/// restrict to one desk (default: every desk the caller is entitled to).
#[derive(Debug, Clone, Default)]
pub struct DealFilter {
    desk: Option<String>,
}

impl DealFilter {
    /// The unfiltered blotter (all entitled desks).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Restrict to one desk.
    #[must_use]
    pub fn desk(mut self, desk: impl Into<String>) -> Self {
        self.desk = Some(desk.into());
        self
    }

    fn to_wire(&self) -> Option<DealScope> {
        self.desk.as_ref().map(|d| DealScope {
            desk: Some(d.clone()),
        })
    }
}

/// The typed dealer-desk handle over the `RfqDeskService` contract. Cheap to clone —
/// shares the underlying HTTP/2 [`Channel`], bearer token, and asserted principal.
/// Construct with [`crate::Client::desk`].
#[derive(Debug, Clone)]
pub struct DeskClient {
    channel: Channel,
    session_token: Option<String>,
    principal: Option<Entitlements>,
}

impl DeskClient {
    /// Build a desk handle over a channel + the client's bearer token / principal.
    pub(crate) fn new(
        channel: Channel,
        session_token: Option<String>,
        principal: Option<Entitlements>,
    ) -> Self {
        Self {
            channel,
            session_token,
            principal,
        }
    }

    /// Submit an inbound RFQ/IOI to a desk (the taker side). The request is validated
    /// priceable before it enters the desk inbox, and a `RFQ_RECEIVED` / `IOI_RECEIVED`
    /// notification is fanned to the desk's subscribers. Returns the captured
    /// [`DeskRequest`] in its `PENDING` state, carrying its server-assigned id.
    ///
    /// # Errors
    ///
    /// [`ClientError`] on a transport / server failure (e.g. `invalid_argument` for an
    /// unpriceable instrument or a non-positive notional) or a malformed response.
    pub async fn submit(&self, request: DeskRfq) -> ClientResult<DeskRequest> {
        let mut svc = RfqDeskServiceClient::new(self.channel.clone());
        let wire = request.to_wire(self.session_token.clone(), self.principal.as_ref());
        let resp = svc.submit_desk_request(wire).await?.into_inner();
        let request = resp.request.ok_or(ClientError::MissingField(
            "SubmitDeskRequestResponse.request",
        ))?;
        DeskRequest::from_wire(&request)
    }

    /// Quote a pending desk request at a firm level (`RfqRespond`/`IoiRespond`
    /// capability-gated). Advances the request to `QUOTED`; returns it carrying the
    /// desk's [`DeskQuote`].
    ///
    /// # Errors
    ///
    /// [`ClientError::Status`] — `not_found` for an unknown id, `failed_precondition`
    /// for a request that is not `PENDING`, `permission_denied`/`unauthenticated`
    /// without the respond capability — plus transport failures.
    pub async fn respond_quote(
        &self,
        request_id: impl Into<String>,
        quote: DeskQuote,
    ) -> ClientResult<DeskRequest> {
        self.respond(request_id.into(), RespondArm::Quote(quote.to_wire()))
            .await
    }

    /// Decline to quote a pending desk request, with a short human `reason` shown to
    /// the counterparty (`RfqRespond`/`IoiRespond` capability-gated). Advances the
    /// request to `REJECTED`.
    ///
    /// # Errors
    ///
    /// [`ClientError`] as [`DeskClient::respond_quote`].
    pub async fn decline(
        &self,
        request_id: impl Into<String>,
        reason: impl Into<String>,
    ) -> ClientResult<DeskRequest> {
        self.respond(
            request_id.into(),
            RespondArm::Reject(DeskReject {
                reason: reason.into(),
            }),
        )
        .await
    }

    async fn respond(&self, request_id: String, arm: RespondArm) -> ClientResult<DeskRequest> {
        let mut svc = RfqDeskServiceClient::new(self.channel.clone());
        let wire = RespondDeskRequestRequest {
            session_token: self.session_token.clone(),
            request_id,
            response: Some(arm),
            principal: Some(principal_or_grant_all(self.principal.as_ref())),
            correlation_id: None,
        };
        let resp = svc.respond_desk_request(wire).await?.into_inner();
        let request = resp.request.ok_or(ClientError::MissingField(
            "RespondDeskRequestResponse.request",
        ))?;
        DeskRequest::from_wire(&request)
    }

    /// Lift a quoted desk request → book a [`Deal`] and a rates position
    /// (`Execute·FixedIncome` capability-gated). Returns the booked deal and the
    /// request in its terminal `ACCEPTED` state.
    ///
    /// # Errors
    ///
    /// [`ClientError::Status`] — `not_found` for an unknown id, `failed_precondition`
    /// for a request that is not `QUOTED`, `permission_denied`/`unauthenticated`
    /// without the execute capability — plus transport failures.
    pub async fn accept(&self, request_id: impl Into<String>) -> ClientResult<DeskAcceptance> {
        let mut svc = RfqDeskServiceClient::new(self.channel.clone());
        let wire = AcceptDeskQuoteRequest {
            session_token: self.session_token.clone(),
            request_id: request_id.into(),
            principal: Some(principal_or_grant_all(self.principal.as_ref())),
            correlation_id: None,
        };
        let resp = svc.accept_desk_quote(wire).await?.into_inner();
        let deal = resp
            .deal
            .as_ref()
            .ok_or(ClientError::MissingField("AcceptDeskQuoteResponse.deal"))
            .and_then(Deal::from_wire)?;
        let request = resp
            .request
            .as_ref()
            .ok_or(ClientError::MissingField("AcceptDeskQuoteResponse.request"))
            .and_then(DeskRequest::from_wire)?;
        Ok(DeskAcceptance { deal, request })
    }

    /// Read the desk inbox (the pending/terminal RFQ & IOI requests), newest first,
    /// entitlement-pruned. `ReadAny`-gated.
    ///
    /// # Errors
    ///
    /// [`ClientError`] on a transport / server failure or a malformed response.
    pub async fn list_requests(
        &self,
        filter: &DeskRequestFilter,
    ) -> ClientResult<Vec<DeskRequest>> {
        let mut svc = RfqDeskServiceClient::new(self.channel.clone());
        let wire = ListDeskRequestsRequest {
            session_token: self.session_token.clone(),
            scope: filter.to_wire(),
            principal: Some(principal_or_grant_all(self.principal.as_ref())),
            correlation_id: None,
        };
        let resp = svc.list_desk_requests(wire).await?.into_inner();
        resp.requests.iter().map(DeskRequest::from_wire).collect()
    }

    /// Read the received-deals blotter, newest first, entitlement-pruned.
    /// `ReadAny`-gated.
    ///
    /// # Errors
    ///
    /// [`ClientError`] on a transport / server failure or a malformed response.
    pub async fn list_deals(&self, filter: &DealFilter) -> ClientResult<Vec<Deal>> {
        let mut svc = RfqDeskServiceClient::new(self.channel.clone());
        let wire = ListDealsRequest {
            session_token: self.session_token.clone(),
            scope: filter.to_wire(),
            principal: Some(principal_or_grant_all(self.principal.as_ref())),
            correlation_id: None,
        };
        let resp = svc.list_deals(wire).await?.into_inner();
        resp.deals.iter().map(Deal::from_wire).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn curve() -> UsdSofrCurve {
        use crate::rates::CivilDate;
        UsdSofrCurve::new(CivilDate::new(2026, 6, 25)).pillar(5, 0.0405)
    }

    #[test]
    fn submit_maps_side_and_notional_from_the_instrument() {
        let rfq = DeskRfq::rfq(
            "ACME",
            "g10",
            Ois::pay_fixed(5, 0.0405).notional(25_000_000.0),
            curve(),
        )
        .ttl(Duration::from_millis(1500))
        .correlation_id("cid-1");
        let wire = rfq.to_wire(Some("tok".to_owned()), None);
        assert_eq!(wire.desk, "g10");
        assert_eq!(wire.counterparty, "ACME");
        assert_eq!(wire.side, celnet_proto::Side::Buy as i32, "pay-fixed ⇒ Buy");
        assert_eq!(wire.notional, 25_000_000.0);
        assert_eq!(wire.ttl_ms, 1500);
        assert_eq!(wire.correlation_id.as_deref(), Some("cid-1"));
        assert_eq!(wire.session_token.as_deref(), Some("tok"));
        assert!(wire.principal.expect("grant-all default").grant_all);
    }

    #[test]
    fn desk_quote_round_trips_through_the_wire() {
        let q = DeskQuote::new(0.0412, 25_000_000.0, Duration::from_secs(2), "alice");
        let wire = q.to_wire();
        assert_eq!(wire.valid_for_ms, 2000);
        assert_eq!(DeskQuote::from_wire(&wire), q);
    }

    #[test]
    fn request_filter_states_and_desk() {
        assert!(DeskRequestFilter::new().to_wire().is_none());
        let scope = DeskRequestFilter::new()
            .states([DeskRequestState::Pending, DeskRequestState::Quoted])
            .desk("g10")
            .to_wire()
            .expect("scope present");
        assert_eq!(scope.states.len(), 2);
        assert_eq!(scope.desk.as_deref(), Some("g10"));
    }

    #[test]
    fn kind_and_state_reject_unknown_tags() {
        assert!(DeskRequestKind::from_wire_tag(999).is_err());
        assert!(DeskRequestState::from_wire(999).is_err());
        // The UNSPECIFIED sentinel is never a valid decoded kind/state.
        assert!(DeskRequestKind::from_wire_tag(WireDeskRequestKind::Unspecified as i32).is_err());
        assert!(DeskRequestState::from_wire(WireDeskRequestState::Unspecified as i32).is_err());
    }
}
