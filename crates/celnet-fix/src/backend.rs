//! The desk seam the FIX gateway maps the quoting lifecycle onto.
//!
//! A FIX counterparty's `QuoteRequest(R)` becomes an RFQ submitted into the
//! Celnet dealer desk (`RfqDeskService.SubmitDeskRequest`); the desk trader's
//! `RespondDeskRequest` (a maker quote) is observed and relayed back out as a
//! FIX `Quote(S)`; and the counterparty's lift (`NewOrderSingle(D)` referencing
//! the `QuoteID`) books the deal via `AcceptDeskQuote` and is reported as a FIX
//! `ExecutionReport(8)`.
//!
//! The [`DeskBackend`] trait is the one seam between the FIX lifecycle mapper
//! ([`crate::gateway`]) and the venue. The production implementation
//! ([`GrpcDeskBackend`]) speaks the one current gRPC contract — it authenticates
//! via `AuthService.Login` for a bearer session token and drives `RfqDeskService`
//! over `tonic`. The trait is also the single correct place for a *test* double:
//! a conformance test implements it to return canned desk responses so the
//! product lifecycle path is exercised end-to-end with no running edge (the
//! double stands in for the venue/desk, never for gateway behaviour).
//!
//! Domain types ([`DeskRfq`], [`ResponseOutcome`], [`DeskFill`]) sit at the seam
//! so the lifecycle mapper never speaks raw proto and a test double stays
//! trivial; [`GrpcDeskBackend`] converts domain ↔ wire.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use celnet_proto::auth_service_client::AuthServiceClient;
use celnet_proto::rfq_desk_service_client::RfqDeskServiceClient;
use celnet_proto::{
    AcceptDeskQuoteRequest, BrokenDate, CurveSet, Deal, DeskRequestKind, DeskRequestScope,
    DeskRequestState, EntitlementPrincipal, ListDeskRequestsRequest, LoginRequest, OisInstrument,
    OisPillar, RatesInstrument, Side, SubmitDeskRequestRequest, rates_instrument,
};
use tonic::transport::{Channel, Endpoint};

/// Whether an inbound request is a firm request-for-quote or a non-firm
/// indication-of-interest (the desk works the latter as an axe).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestKind {
    /// Request-for-quote — a counterparty wants a firm price.
    Rfq,
    /// Indication-of-interest — a non-firm axe for the desk to work.
    Ioi,
}

impl RequestKind {
    /// The proto [`DeskRequestKind`] discriminant.
    #[must_use]
    pub fn to_wire(self) -> i32 {
        match self {
            RequestKind::Rfq => DeskRequestKind::Rfq as i32,
            RequestKind::Ioi => DeskRequestKind::Ioi as i32,
        }
    }
}

/// The directional side of a rates request, from the counterparty's view. Maps
/// one-to-one onto the proto [`Side`] exactly as the SDK's OIS vocabulary does
/// (`SIDE_BUY` ⇒ the counterparty pays fixed).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeskSide {
    /// The counterparty pays the fixed leg (`SIDE_BUY`).
    PayFixed,
    /// The counterparty receives the fixed leg (`SIDE_SELL`).
    ReceiveFixed,
    /// No directional side — a two-way request (`SIDE_TWO_WAY`).
    TwoWay,
}

impl DeskSide {
    /// The proto [`Side`] discriminant.
    #[must_use]
    pub fn to_wire(self) -> i32 {
        match self {
            DeskSide::PayFixed => Side::Buy as i32,
            DeskSide::ReceiveFixed => Side::Sell as i32,
            DeskSide::TwoWay => Side::TwoWay as i32,
        }
    }

    /// Recover a [`DeskSide`] from a proto [`Side`] discriminant (unknown ⇒
    /// two-way, the neutral default).
    #[must_use]
    pub fn from_wire(side: i32) -> Self {
        match Side::try_from(side) {
            Ok(Side::Buy) => DeskSide::PayFixed,
            Ok(Side::Sell) => DeskSide::ReceiveFixed,
            _ => DeskSide::TwoWay,
        }
    }
}

/// One inbound rates RFQ/IOI the gateway injects into the desk.
#[derive(Debug, Clone, PartialEq)]
pub struct DeskRfq {
    /// RFQ (firm price wanted) or IOI (axe to work).
    pub kind: RequestKind,
    /// The counterparty label (display/attribution — the peer's `SenderCompID`).
    pub counterparty: String,
    /// The desk this request routes to (entitlement + notification scope).
    pub desk: String,
    /// The instrument family label echoed back on the quote (`Symbol(55)`).
    pub symbol: String,
    /// The OIS tenor in whole years from spot (`>= 1`).
    pub tenor_years: u32,
    /// The notional requested (curve currency, positive).
    pub notional: f64,
    /// The counterparty's directional side.
    pub side: DeskSide,
}

/// The desk's response to a submitted request, as the gateway needs it to form
/// the outbound FIX message.
#[derive(Debug, Clone, PartialEq)]
pub enum ResponseOutcome {
    /// The desk quoted a firm level → emit a FIX `Quote(S)`.
    Quoted(DeskQuoteOut),
    /// The desk declined / the request expired or was withdrawn → emit a FIX
    /// `QuoteRequestReject(AG)` carrying the reason.
    Declined(String),
}

/// A firm desk quote relayed to the counterparty.
#[derive(Debug, Clone, PartialEq)]
pub struct DeskQuoteOut {
    /// The originating desk `request_id` (also the FIX `QuoteID(117)`, so the
    /// lift maps straight back to the desk request).
    pub request_id: String,
    /// The quoted all-in fixed rate as a decimal (`0.041` = 4.10%).
    pub price: f64,
    /// The notional the quote is good for (curve currency, positive).
    pub notional: f64,
    /// How long the quote stands from issue, milliseconds (0 ⇒ indicative).
    pub valid_for_ms: u32,
    /// The quoting trader (attribution).
    pub trader: String,
}

/// A booked deal (an accepted desk quote), as the gateway needs it to form the
/// `ExecutionReport(8)` fill.
#[derive(Debug, Clone, PartialEq)]
pub struct DeskFill {
    /// The server-assigned deal identity (FIX `OrderID(37)` / `ExecID(17)`).
    pub deal_id: String,
    /// The dealt level — the accepted quote price (all-in fixed rate, decimal).
    pub price: f64,
    /// The dealt notional (curve currency, positive).
    pub notional: f64,
    /// The desk's side (opposite the counterparty's submitted side).
    pub desk_side: DeskSide,
    /// The booking trader (attribution).
    pub trader: String,
}

/// A desk-backend failure surfaced to the gateway lifecycle mapper.
#[derive(Debug)]
pub enum BackendError {
    /// The transport channel could not be established or dialled.
    Transport(String),
    /// Authentication (`AuthService.Login`) failed.
    Auth(String),
    /// A gRPC call returned a non-OK status.
    Status(String),
    /// A submitted request could not be found in the desk inbox read-back.
    NotFound(String),
    /// The desk did not respond within the polling deadline.
    Timeout,
    /// A response was missing a required field.
    Malformed(&'static str),
}

impl core::fmt::Display for BackendError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            BackendError::Transport(e) => write!(f, "desk transport error: {e}"),
            BackendError::Auth(e) => write!(f, "desk authentication failed: {e}"),
            BackendError::Status(e) => write!(f, "desk service error: {e}"),
            BackendError::NotFound(id) => write!(f, "desk request {id} not found"),
            BackendError::Timeout => write!(f, "timed out awaiting the desk response"),
            BackendError::Malformed(field) => write!(f, "desk response missing {field}"),
        }
    }
}

impl std::error::Error for BackendError {}

/// A boxed, `Send` future result — the dyn-compatible shape the gateway holds the
/// backend behind (`Arc<dyn DeskBackend>`), so one acceptor can be spawned per
/// connection without leaking the concrete backend type.
pub type BackendFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, BackendError>> + Send + 'a>>;

/// The seam between the FIX quoting lifecycle and the Celnet dealer desk.
///
/// Three operations cover the full lifecycle: submit an RFQ/IOI, await the desk's
/// response (quote or decline), and accept (lift) a quoted request into a booked
/// deal. The production [`GrpcDeskBackend`] implements these over the one current
/// gRPC contract; a conformance test implements them with canned responses.
pub trait DeskBackend: Send + Sync {
    /// Submit an RFQ/IOI into the desk, returning the server-assigned
    /// `request_id` of the captured (PENDING) request.
    fn submit<'a>(&'a self, rfq: DeskRfq) -> BackendFuture<'a, String>;

    /// Await the desk's response to `request_id` (the trader quotes or declines).
    fn await_response<'a>(&'a self, request_id: String) -> BackendFuture<'a, ResponseOutcome>;

    /// Accept (lift) a quoted `request_id`, booking the deal.
    fn accept<'a>(&'a self, request_id: String) -> BackendFuture<'a, DeskFill>;
}

/// The production desk backend: speaks the one current gRPC contract.
///
/// Constructed via [`GrpcDeskBackend::login`], which dials the edge and exchanges
/// credentials for a bearer session token through `AuthService.Login`. Every
/// subsequent `RfqDeskService` call carries that token plus an entitlement
/// principal (grant-all by default — the same audited default the SDK asserts);
/// a deployment runs the gateway as a desk-entitled user holding the
/// `RfqRespond`/`Execute·FixedIncome` capabilities the booking path requires.
#[derive(Debug, Clone)]
pub struct GrpcDeskBackend {
    channel: Channel,
    session_token: String,
    principal: EntitlementPrincipal,
    curve_set: CurveSet,
    desk: String,
    poll_interval: Duration,
    poll_deadline: Duration,
}

/// The default desk-response polling cadence (how often the inbox is re-read for
/// the trader's quote).
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(200);
/// The default ceiling on awaiting a desk response before the gateway declines
/// the request back to the counterparty.
pub const DEFAULT_POLL_DEADLINE: Duration = Duration::from_secs(30);

impl GrpcDeskBackend {
    /// Dial the edge at `endpoint`, authenticate `email`/`password` via
    /// `AuthService.Login`, and bind the backend to `desk` (the routing/entitlement
    /// scope) pricing against `curve_set` (the desk's calibrated curve).
    ///
    /// # Errors
    /// [`BackendError::Transport`] if the endpoint is malformed/unreachable, or
    /// [`BackendError::Auth`] if the credentials are rejected.
    pub async fn login(
        endpoint: impl Into<String>,
        email: impl Into<String>,
        password: impl Into<String>,
        desk: impl Into<String>,
        curve_set: CurveSet,
    ) -> Result<Self, BackendError> {
        let endpoint = endpoint.into();
        let ep = Endpoint::from_shared(endpoint.clone())
            .map_err(|e| BackendError::Transport(format!("invalid endpoint {endpoint}: {e}")))?;
        let channel = ep
            .connect()
            .await
            .map_err(|e| BackendError::Transport(e.to_string()))?;

        let mut auth = AuthServiceClient::new(channel.clone());
        let resp = auth
            .login(LoginRequest {
                email: email.into(),
                password: password.into(),
                correlation_id: None,
            })
            .await
            .map_err(|s| BackendError::Auth(s.message().to_string()))?
            .into_inner();

        Ok(Self {
            channel,
            session_token: resp.session_token,
            // Grant-all is the audited default principal the whole client estate
            // asserts; a scoped deployment can tighten this later.
            principal: EntitlementPrincipal {
                grant_all: true,
                grants: Vec::new(),
                denies: Vec::new(),
            },
            curve_set,
            desk: desk.into(),
            poll_interval: DEFAULT_POLL_INTERVAL,
            poll_deadline: DEFAULT_POLL_DEADLINE,
        })
    }

    /// Override the desk-response polling cadence and deadline (chainable).
    #[must_use]
    pub fn with_polling(mut self, interval: Duration, deadline: Duration) -> Self {
        self.poll_interval = interval;
        self.poll_deadline = deadline;
        self
    }

    /// Build the wire OIS instrument for `rfq` against the configured curve.
    fn instrument_for(rfq: &DeskRfq) -> RatesInstrument {
        RatesInstrument {
            instrument: Some(rates_instrument::Instrument::Ois(OisInstrument {
                tenor_years: rfq.tenor_years,
                // An RFQ asks the desk to show a level; the dealt rate is the
                // desk's quoted price, so the submitted leg coupon is the
                // at-market 0.0 (the request carries economics, not a coupon).
                fixed_rate: 0.0,
                notional: rfq.notional,
                side: rfq.side.to_wire(),
            })),
        }
    }
}

impl DeskBackend for GrpcDeskBackend {
    fn submit<'a>(&'a self, rfq: DeskRfq) -> BackendFuture<'a, String> {
        Box::pin(async move {
            let mut client = RfqDeskServiceClient::new(self.channel.clone());
            let req = SubmitDeskRequestRequest {
                session_token: Some(self.session_token.clone()),
                kind: rfq.kind.to_wire(),
                counterparty: rfq.counterparty.clone(),
                desk: rfq.desk.clone(),
                instrument: Some(Self::instrument_for(&rfq)),
                curve_set: Some(self.curve_set.clone()),
                side: rfq.side.to_wire(),
                notional: rfq.notional,
                ttl_ms: 0,
                principal: Some(self.principal.clone()),
                correlation_id: None,
            };
            let resp = client
                .submit_desk_request(req)
                .await
                .map_err(|s| BackendError::Status(s.message().to_string()))?
                .into_inner();
            let request = resp
                .request
                .ok_or(BackendError::Malformed("SubmitDeskRequestResponse.request"))?;
            Ok(request.request_id)
        })
    }

    fn await_response<'a>(&'a self, request_id: String) -> BackendFuture<'a, ResponseOutcome> {
        Box::pin(async move {
            let started = tokio::time::Instant::now();
            loop {
                let mut client = RfqDeskServiceClient::new(self.channel.clone());
                let resp = client
                    .list_desk_requests(ListDeskRequestsRequest {
                        session_token: Some(self.session_token.clone()),
                        scope: Some(DeskRequestScope {
                            states: Vec::new(),
                            desk: Some(self.desk.clone()),
                        }),
                        principal: Some(self.principal.clone()),
                        correlation_id: None,
                    })
                    .await
                    .map_err(|s| BackendError::Status(s.message().to_string()))?
                    .into_inner();

                if let Some(req) = resp
                    .requests
                    .into_iter()
                    .find(|r| r.request_id == request_id)
                {
                    match DeskRequestState::try_from(req.state) {
                        Ok(DeskRequestState::Quoted) => {
                            let q = req
                                .quote
                                .ok_or(BackendError::Malformed("DeskRequest.quote"))?;
                            return Ok(ResponseOutcome::Quoted(DeskQuoteOut {
                                request_id,
                                price: q.price,
                                notional: q.notional,
                                valid_for_ms: q.valid_for_ms,
                                trader: q.trader,
                            }));
                        }
                        Ok(DeskRequestState::Rejected) => {
                            return Ok(ResponseOutcome::Declined("desk declined".to_owned()));
                        }
                        Ok(DeskRequestState::Expired) => {
                            return Ok(ResponseOutcome::Declined("request expired".to_owned()));
                        }
                        Ok(DeskRequestState::Withdrawn) => {
                            return Ok(ResponseOutcome::Declined("request withdrawn".to_owned()));
                        }
                        // PENDING / unspecified: keep polling.
                        _ => {}
                    }
                }

                if started.elapsed() >= self.poll_deadline {
                    return Err(BackendError::Timeout);
                }
                tokio::time::sleep(self.poll_interval).await;
            }
        })
    }

    fn accept<'a>(&'a self, request_id: String) -> BackendFuture<'a, DeskFill> {
        Box::pin(async move {
            let mut client = RfqDeskServiceClient::new(self.channel.clone());
            let resp = client
                .accept_desk_quote(AcceptDeskQuoteRequest {
                    session_token: Some(self.session_token.clone()),
                    request_id: request_id.clone(),
                    principal: Some(self.principal.clone()),
                    correlation_id: None,
                })
                .await
                .map_err(|s| BackendError::Status(s.message().to_string()))?
                .into_inner();
            let deal: Deal = resp
                .deal
                .ok_or(BackendError::Malformed("AcceptDeskQuoteResponse.deal"))?;
            Ok(DeskFill {
                deal_id: deal.deal_id,
                price: deal.price,
                notional: deal.notional,
                desk_side: DeskSide::from_wire(deal.side),
                trader: deal.trader,
            })
        })
    }
}

/// Build a one-pillar-per-tenor USD-SOFR [`CurveSet`] from `(tenor_years,
/// par_rate)` pairs anchored at `(year, month, day)` — the desk's calibrated
/// curve the gateway prices submitted requests against. A convenience for the
/// gateway binary's configuration surface (and tests); mirrors the SDK's
/// `UsdSofrCurve` builder shape without depending on its private wire mapping.
#[must_use]
pub fn usd_sofr_curve(reference: (i32, u32, u32), pillars: &[(u32, f64)]) -> CurveSet {
    let (year, month, day) = reference;
    CurveSet {
        currency: "USD".to_owned(),
        reference_date: Some(BrokenDate { year, month, day }),
        ois_pillars: pillars
            .iter()
            .map(|&(tenor_years, par_rate)| OisPillar {
                tenor_years,
                par_rate,
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_kind_maps_to_wire() {
        assert_eq!(RequestKind::Rfq.to_wire(), DeskRequestKind::Rfq as i32);
        assert_eq!(RequestKind::Ioi.to_wire(), DeskRequestKind::Ioi as i32);
    }

    #[test]
    fn desk_side_round_trips_through_wire() {
        for s in [DeskSide::PayFixed, DeskSide::ReceiveFixed, DeskSide::TwoWay] {
            assert_eq!(DeskSide::from_wire(s.to_wire()), s);
        }
    }

    #[test]
    fn pay_fixed_is_buy_receive_fixed_is_sell() {
        assert_eq!(DeskSide::PayFixed.to_wire(), Side::Buy as i32);
        assert_eq!(DeskSide::ReceiveFixed.to_wire(), Side::Sell as i32);
    }

    #[test]
    fn curve_builder_lays_pillars_in_order() {
        let cs = usd_sofr_curve((2026, 6, 25), &[(1, 0.0432), (5, 0.0405)]);
        assert_eq!(cs.currency, "USD");
        assert_eq!(cs.reference_date.unwrap().day, 25);
        assert_eq!(cs.ois_pillars.len(), 2);
        assert_eq!(cs.ois_pillars[1].tenor_years, 5);
        assert_eq!(cs.ois_pillars[1].par_rate, 0.0405);
    }
}
