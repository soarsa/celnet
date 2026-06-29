//! Conformance + loopback tests for the desk-backed FIX↔gRPC gateway.
//!
//! The gateway's *product* path — the FIX codec + session FSM + the
//! [`DeskGateway`] lifecycle mapper — is exercised end-to-end with NO running
//! edge by driving it against a `FakeDesk` implementing
//! [`celnet_fix::backend::DeskBackend`]. The fake is the one correct place for a
//! test double: it stands in for the venue/desk (returning canned desk
//! responses), never for gateway behaviour, which is the real product code under
//! test. We assert BOTH halves of the bridge: the `RfqDeskService`-equivalent
//! calls the gateway makes (recorded by the fake) and the FIX messages it emits.

use std::sync::{Arc, Mutex};

use celnet_fix::backend::{
    BackendError, BackendFuture, DeskBackend, DeskFill, DeskQuoteOut, DeskRfq, DeskSide,
    RequestKind, ResponseOutcome,
};
use celnet_fix::dialect_fx::SIDE_BUY;
use celnet_fix::dialect_rates::{
    self, RatesQuoteRequestParams, RatesSide, SEC_TYPE_OIS, SubscriptionRequest,
};
use celnet_fix::dictionary::{MsgType, validate};
use celnet_fix::framing::FrameCursor;
use celnet_fix::gateway::{DeskGateway, GatewayConfig};
use celnet_fix::initiator::{Initiator, LiftPolicy};
use celnet_fix::messages::{
    self, ExecReportView, NewOrderParams, QuoteRequestRejectView, QuoteView,
};
use celnet_fix::session::{InMemoryStore, Role, Session, SessionConfig};

/// The desk request id the fake assigns (also the FIX `QuoteID(117)`).
const DESK_REQUEST_ID: &str = "DESK-REQ-1";
/// A fixed `SendingTime(52)` for the counterparty's frames (deterministic).
const SENDING_TIME: &[u8] = b"20260629-12:00:00.000";

/// The desk-side calls the gateway made, recorded for assertion.
#[derive(Default)]
struct Calls {
    submitted: Vec<DeskRfq>,
    accepted: Vec<String>,
}

/// An in-process desk stand-in: records the gateway's submit/accept calls and
/// returns canned responses. This is the test's stand-in for the venue/desk —
/// the gateway code path it drives is the real product.
struct FakeDesk {
    calls: Arc<Mutex<Calls>>,
    request_id: String,
    outcome: ResponseOutcome,
    fill: DeskFill,
}

impl DeskBackend for FakeDesk {
    fn submit<'a>(&'a self, rfq: DeskRfq) -> BackendFuture<'a, String> {
        let id = self.request_id.clone();
        let calls = Arc::clone(&self.calls);
        Box::pin(async move {
            calls.lock().expect("calls lock").submitted.push(rfq);
            Ok::<_, BackendError>(id)
        })
    }

    fn await_response<'a>(&'a self, _request_id: String) -> BackendFuture<'a, ResponseOutcome> {
        let outcome = self.outcome.clone();
        Box::pin(async move { Ok::<_, BackendError>(outcome) })
    }

    fn accept<'a>(&'a self, request_id: String) -> BackendFuture<'a, DeskFill> {
        let fill = self.fill.clone();
        let calls = Arc::clone(&self.calls);
        Box::pin(async move {
            calls.lock().expect("calls lock").accepted.push(request_id);
            Ok::<_, BackendError>(fill)
        })
    }
}

/// A quoting fake: quotes `DESK_REQUEST_ID` at `price` for `notional`, and books
/// the matching deal.
fn quoting_desk(calls: Arc<Mutex<Calls>>, price: f64, notional: f64) -> FakeDesk {
    FakeDesk {
        calls,
        request_id: DESK_REQUEST_ID.to_owned(),
        outcome: ResponseOutcome::Quoted(DeskQuoteOut {
            request_id: DESK_REQUEST_ID.to_owned(),
            price,
            notional,
            valid_for_ms: 5_000,
            trader: "alice".to_owned(),
        }),
        fill: DeskFill {
            deal_id: "DEAL-1".to_owned(),
            price,
            notional,
            // The desk takes the opposite side of a counterparty pay-fixed RFQ.
            desk_side: DeskSide::ReceiveFixed,
            trader: "alice".to_owned(),
        },
    }
}

/// A declining fake: declines every request with `reason`.
fn declining_desk(calls: Arc<Mutex<Calls>>, reason: &str) -> FakeDesk {
    FakeDesk {
        calls,
        request_id: DESK_REQUEST_ID.to_owned(),
        outcome: ResponseOutcome::Declined(reason.to_owned()),
        fill: DeskFill {
            deal_id: String::new(),
            price: 0.0,
            notional: 0.0,
            desk_side: DeskSide::TwoWay,
            trader: String::new(),
        },
    }
}

/// Build an acceptor-role gateway over `backend`.
fn gateway<B: DeskBackend>(backend: Arc<B>) -> DeskGateway<InMemoryStore, B> {
    let session = Session::new(
        SessionConfig {
            sender: b"CELNET-FIX".to_vec(),
            target: b"CPARTY".to_vec(),
            heart_bt_int: 30,
            role: Role::Acceptor,
        },
        InMemoryStore::new(),
    );
    DeskGateway::new(
        session,
        backend,
        GatewayConfig {
            desk: "RATES".to_owned(),
            counterparty: "CPARTY".to_owned(),
        },
    )
}

/// A counterparty-role session used purely as a correctly-framed-frame factory
/// (proper CompIDs + ascending `MsgSeqNum`).
fn counterparty() -> Session<InMemoryStore> {
    Session::new(
        SessionConfig {
            sender: b"CPARTY".to_vec(),
            target: b"CELNET-FIX".to_vec(),
            heart_bt_int: 30,
            role: Role::Initiator,
        },
        InMemoryStore::new(),
    )
}

/// Parse one emitted frame and return its validated [`MsgType`] + raw bytes.
fn one(frames: &[Vec<u8>]) -> (MsgType, &[u8]) {
    assert_eq!(frames.len(), 1, "expected exactly one emitted frame");
    let frame = FrameCursor::parse(&frames[0]).expect("emitted frame parses");
    let mt = validate(&frame).expect("emitted frame validates");
    (mt, &frames[0])
}

#[tokio::test]
async fn conformance_rfq_quote_lift_fill() {
    let calls = Arc::new(Mutex::new(Calls::default()));
    let backend = Arc::new(quoting_desk(Arc::clone(&calls), 0.0405, 100_000_000.0));
    let mut gw = gateway(backend);
    let mut cpty = counterparty();

    // 1. Logon → the gateway mirrors a Logon.
    let logon = cpty.start_logon(SENDING_TIME, false);
    let out = gw.handle_frame(&logon).await;
    let (mt, _) = one(&out);
    assert_eq!(mt, MsgType::Logon, "the acceptor mirrors the Logon");

    // 2. QuoteRequest (firm RFQ, pay-fixed 5Y 100mm) → submit + Quote(S).
    let qr = cpty.send_app(SENDING_TIME, |h, e| {
        dialect_rates::build_rates_quote_request(
            h,
            &RatesQuoteRequestParams {
                quote_req_id: b"RFQ-1",
                symbol: b"USDSOFR",
                tenor_years: 5,
                notional: 100_000_000.0,
                side: RatesSide::PayFixed,
                subscription: SubscriptionRequest::Snapshot,
            },
            e,
        )
    });
    let out = gw.handle_frame(&qr).await;
    let (mt, raw) = one(&out);
    assert_eq!(mt, MsgType::Quote, "an RFQ is answered with a Quote(S)");
    let quote = QuoteView::new(FrameCursor::parse(raw).unwrap());
    assert_eq!(
        quote.quote_id(),
        Some(DESK_REQUEST_ID.as_bytes()),
        "the QuoteID is the desk request id (so the lift maps straight back)"
    );
    assert_eq!(quote.symbol(), Some(&b"USDSOFR"[..]));
    assert!((quote.bid().unwrap() - 0.0405).abs() < 1e-9);
    assert!((quote.offer().unwrap() - 0.0405).abs() < 1e-9);

    // The desk-side submit call carries the mapped economics.
    {
        let c = calls.lock().unwrap();
        assert_eq!(c.submitted.len(), 1, "exactly one SubmitDeskRequest");
        let s = &c.submitted[0];
        assert_eq!(s.kind, RequestKind::Rfq);
        assert_eq!(s.counterparty, "CPARTY");
        assert_eq!(s.desk, "RATES");
        assert_eq!(s.symbol, "USDSOFR");
        assert_eq!(s.tenor_years, 5);
        assert_eq!(s.notional, 100_000_000.0);
        assert_eq!(s.side, DeskSide::PayFixed);
    }

    // 3. Lift the quote with a NewOrderSingle referencing the QuoteID → fill.
    let order = cpty.send_app(SENDING_TIME, |h, e| {
        messages::build_new_order_single(
            h,
            &NewOrderParams {
                cl_ord_id: b"ORD-1",
                quote_id: DESK_REQUEST_ID.as_bytes(),
                symbol: b"USDSOFR",
                side: SIDE_BUY,
                qty: 100_000_000.0,
                transact_time: SENDING_TIME,
            },
            e,
        )
    });
    let out = gw.handle_frame(&order).await;
    let (mt, raw) = one(&out);
    assert_eq!(
        mt,
        MsgType::ExecutionReport,
        "the lift books an ExecutionReport"
    );
    let exec = ExecReportView::new(FrameCursor::parse(raw).unwrap());
    assert_eq!(exec.exec_type(), Some(messages::EXEC_FILLED), "filled");
    assert_eq!(exec.cl_ord_id(), Some(&b"ORD-1"[..]));
    assert!((exec.last_px().unwrap() - 0.0405).abs() < 1e-9);

    // The desk-side accept call lifted the right request.
    {
        let c = calls.lock().unwrap();
        assert_eq!(c.accepted, vec![DESK_REQUEST_ID.to_owned()]);
    }
}

#[tokio::test]
async fn conformance_ioi_maps_to_desk_ioi() {
    let calls = Arc::new(Mutex::new(Calls::default()));
    let backend = Arc::new(quoting_desk(Arc::clone(&calls), 0.0410, 50_000_000.0));
    let mut gw = gateway(backend);
    let mut cpty = counterparty();

    let logon = cpty.start_logon(SENDING_TIME, false);
    let _ = gw.handle_frame(&logon).await;

    // An indicative QuoteRequest (QuoteType(537)=0) → a desk IOI. Hand-built so
    // the QuoteType field is present (the standard rates builder omits it).
    let ioi = cpty.send_app(SENDING_TIME, |h, e| {
        e.clear();
        h.encode(MsgType::QuoteRequest, e);
        e.push(131, b"IOI-1");
        e.push(55, b"USDSOFR");
        e.push(167, SEC_TYPE_OIS);
        e.push(537, b"0"); // QuoteType = Indicative → IOI
        e.push_int(38, 50_000_000);
        e.push_int(dialect_rates::TAG_TENOR_YEARS, 7);
        e.push(54, &[SIDE_BUY]);
        e.finish()
    });
    let out = gw.handle_frame(&ioi).await;
    let (mt, _) = one(&out);
    assert_eq!(mt, MsgType::Quote, "the desk firms the IOI into a Quote");

    let c = calls.lock().unwrap();
    assert_eq!(c.submitted.len(), 1);
    assert_eq!(
        c.submitted[0].kind,
        RequestKind::Ioi,
        "537=0 maps to a desk IOI"
    );
    assert_eq!(c.submitted[0].tenor_years, 7);
}

#[tokio::test]
async fn conformance_desk_decline_emits_quote_request_reject() {
    let calls = Arc::new(Mutex::new(Calls::default()));
    let backend = Arc::new(declining_desk(Arc::clone(&calls), "no axe"));
    let mut gw = gateway(backend);
    let mut cpty = counterparty();

    let logon = cpty.start_logon(SENDING_TIME, false);
    let _ = gw.handle_frame(&logon).await;

    let qr = cpty.send_app(SENDING_TIME, |h, e| {
        dialect_rates::build_rates_quote_request(
            h,
            &RatesQuoteRequestParams {
                quote_req_id: b"RFQ-9",
                symbol: b"USDSOFR",
                tenor_years: 10,
                notional: 25_000_000.0,
                side: RatesSide::ReceiveFixed,
                subscription: SubscriptionRequest::Snapshot,
            },
            e,
        )
    });
    let out = gw.handle_frame(&qr).await;
    let (mt, raw) = one(&out);
    assert_eq!(
        mt,
        MsgType::QuoteRequestReject,
        "a desk decline is a QuoteRequestReject"
    );
    let rej = QuoteRequestRejectView::new(FrameCursor::parse(raw).unwrap());
    assert_eq!(rej.quote_req_id(), Some(&b"RFQ-9"[..]), "addresses the RFQ");
    assert_eq!(rej.text(), Some(&b"no axe"[..]), "carries the desk reason");
}

#[tokio::test]
async fn loopback_initiator_lifts_desk_quote_over_a_socket() {
    let calls = Arc::new(Mutex::new(Calls::default()));
    let backend = Arc::new(quoting_desk(Arc::clone(&calls), 0.0405, 100_000_000.0));

    let (server_io, client_io) = tokio::io::duplex(16 * 1024);

    // Run the gateway on the server half.
    let mut gw = gateway(backend);
    let gw_task = tokio::spawn(async move { gw.run(server_io).await });

    // Drive the real Initiator (price-taker) on the client half: logon, send a
    // rates RFQ, lift the offer, collect the ExecutionReport.
    let init_session = Session::new(
        SessionConfig {
            sender: b"CPARTY".to_vec(),
            target: b"CELNET-FIX".to_vec(),
            heart_bt_int: 30,
            role: Role::Initiator,
        },
        InMemoryStore::new(),
    );
    let mut initiator = Initiator::new(init_session, LiftPolicy::LiftOffer);
    let result = initiator
        .request_and_lift(client_io, SENDING_TIME.to_vec(), |h, e| {
            dialect_rates::build_rates_quote_request(
                h,
                &RatesQuoteRequestParams {
                    quote_req_id: b"RFQ-1",
                    symbol: b"USDSOFR",
                    tenor_years: 5,
                    notional: 100_000_000.0,
                    side: RatesSide::PayFixed,
                    subscription: SubscriptionRequest::Snapshot,
                },
                e,
            )
        })
        .await
        .expect("initiator cycle completes");

    assert!(
        result.filled,
        "the lift booked a fill end-to-end over the socket"
    );
    assert!((result.fill_px.unwrap() - 0.0405).abs() < 1e-9);
    assert_eq!(result.quote_id.as_deref(), Some(DESK_REQUEST_ID.as_bytes()));

    // The client half closed on return → the gateway loop drains cleanly.
    gw_task
        .await
        .expect("gateway task joins")
        .expect("gateway run ok");

    let c = calls.lock().unwrap();
    assert_eq!(c.submitted.len(), 1, "one RFQ submitted into the desk");
    assert_eq!(
        c.accepted,
        vec![DESK_REQUEST_ID.to_owned()],
        "one deal accepted"
    );
}
