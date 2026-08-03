//! Live FIX 4.4 acceptor edge — headless end-to-end over a REAL loopback socket.
//!
//! Boots a real [`Edge`] (the engine's calibrated EURUSD fixture) with the live FIX
//! acceptor attached on an ephemeral loopback port, then drives a **real**
//! `celnet-fix` 4.4 session as the external counterparty (logon → `QuoteRequest(R)`
//! → `Quote(S)` → lift `NewOrderSingle(D)` → `ExecutionReport(8)`) over a genuine
//! TCP socket. There is NO mock, NO faked FIX peer, NO lowered numeric tolerance,
//! and NO `#[ignore]`: the FIX engine is the real `celnet-fix` engine, the socket is
//! a real loopback `TcpStream`, and the asserted premium is reconciled to the
//! `celnet-vanilla` engine (itself `celnet-golden` / QuantLib-gated) **to 1e-12** via
//! the dialect's exact-premium provenance field.
//!
//! It proves:
//!   1. the returned `Quote` premium (exact field) == the engine/golden price for the
//!      instrument to 1e-12 — the FIX edge prices through the SAME surface_book /
//!      engine path the gRPC `QuoteService` uses;
//!   2. a lift books a fill `ExecutionReport(35=8, ExecType=F)` at exactly the quoted
//!      offer (1e-12) — the SAME keyed-MAC click-to-trade / last-look token path the
//!      RFS stream uses;
//!   3. a forged token, a replayed (already-consumed) token, and a stale (expired)
//!      token are each rejected with a reject `ExecutionReport(ExecType=8)` — exactly
//!      as the gRPC click-to-trade path declines.
//!
//! Every body is hard wall-clock bounded and every socket await is bounded, so a
//! regression surfaces as a fast failure, never an infinite hang.

mod common;

use std::net::SocketAddr;
use std::time::Duration;

use celnet_core::is_close;
use celnet_fix::MsgType;
use celnet_fix::dialect_rates::{
    self, BondQuoteRequestParams, RatesQuoteRequestParams, RatesSide, SubscriptionRequest,
};
use celnet_fix::framing::{FrameCursor, FrameEncoder};
use celnet_fix::messages::{
    self, EXEC_FILLED, EXEC_REJECTED, ExecReportView, Header, NewOrderParams, QuoteView,
};
use celnet_fix::session::{InMemoryStore, Role, Session, SessionConfig, SessionState};
use celnet_fix::transport::{FrameReader, write_frame};
use celnet_proto::{AccrualBasis, BondInstrument, BrokenDate, PaymentFrequency, Side};
use celnet_server::{Clock, SpreadModel};
use celnet_types::{OptionType, VanillaInputs};
use tokio::net::TcpStream;

use common::{live_market, start_edge_with};

/// Raised 10 s → 45 s: loaded-t2 M4 contention starves edge-boot past the old limit.
const DEADLINE: Duration = Duration::from_secs(45);
/// Raised 5 s → 20 s: matches celnet-client STEP_DEADLINE under loaded-t2 contention.
const STEP: Duration = Duration::from_secs(20);
const T: &[u8] = b"20260605-12:00:00.000";

/// The pre-agreed CompIDs for the e2e (the venue and its counterparty).
const VENUE: &[u8] = b"CELNET";
const CPTY: &[u8] = b"CELNET-CPTY";

/// The dialect tag carrying vol-time in years (mirrors `celnet_fix::dialect_fx::TAG_EXPIRY_YEARS`).
const TAG_EXPIRY_YEARS: u32 = 7001;

fn init_cfg() -> SessionConfig {
    SessionConfig {
        sender: CPTY.to_vec(),
        target: VENUE.to_vec(),
        heart_bt_int: 30,
        role: Role::Initiator,
    }
}

/// The first-principles reference two-way (offer) for the EURUSD 1Y 1.10 call at the
/// edge's live market under the default spread model — the SAME numbers the FIX edge
/// computes (absolute-strike vanilla priced at the live ATM vol).
fn reference_offer(strike: f64, expiry_years: f64) -> f64 {
    let m = live_market();
    let inputs = VanillaInputs::new(m.spot, strike, m.vol, expiry_years, m.r_dom(), m.r_for());
    let greeks = celnet_vanilla::greeks(OptionType::Call, &inputs);
    SpreadModel::default().two_way(greeks.price, &greeks).offer
}

/// A thin real-`celnet-fix` initiator driver giving the test full control of the
/// session: logon, send a frame built by a closure, and pull the next delivered
/// application frame (driving any session-level admin replies in between).
struct Driver {
    session: Session<InMemoryStore>,
    reader: FrameReader<tokio::io::ReadHalf<TcpStream>>,
    write: tokio::io::WriteHalf<TcpStream>,
}

impl Driver {
    /// Connect to the FIX acceptor and complete the logon handshake.
    async fn connect(addr: SocketAddr) -> Self {
        let stream = tokio::time::timeout(STEP, TcpStream::connect(addr))
            .await
            .expect("connect in time")
            .expect("connect");
        let (read, mut write) = tokio::io::split(stream);
        let mut session = Session::new(init_cfg(), InMemoryStore::new());
        let mut reader = FrameReader::new(read);
        // Logon and wait for the acceptor's mirror (the session becomes Active).
        let logon = session.start_logon(T, false);
        write_frame(&mut write, &logon).await.expect("send logon");
        loop {
            let frame = tokio::time::timeout(STEP, reader.next_frame())
                .await
                .expect("logon mirror in time")
                .expect("read")
                .expect("acceptor stays open");
            let action = session.on_inbound(&frame, T).expect("session ok");
            for f in &action.outbound {
                write_frame(&mut write, f).await.expect("send admin");
            }
            if session.state() == SessionState::Active {
                break;
            }
        }
        Self {
            session,
            reader,
            write,
        }
    }

    /// Send one application frame built by `build`, returning the on-wire bytes.
    async fn send_app(&mut self, build: impl FnOnce(&Header<'_>, &mut FrameEncoder) -> Vec<u8>) {
        let frame = self.session.send_app(T, build);
        write_frame(&mut self.write, &frame)
            .await
            .expect("send app frame");
    }

    /// Pull the next delivered application frame of `want` (driving any admin frames
    /// the session must answer first), returning the raw bytes.
    async fn next_app(&mut self, want: MsgType) -> Vec<u8> {
        loop {
            let frame = tokio::time::timeout(STEP, self.reader.next_frame())
                .await
                .expect("an app frame before the deadline")
                .expect("read")
                .expect("acceptor stays open");
            let action = self.session.on_inbound(&frame, T).expect("session ok");
            for f in &action.outbound {
                write_frame(&mut self.write, f).await.expect("send admin");
            }
            if action.deliver == Some(want) {
                return frame;
            }
        }
    }
}

/// Build the EURUSD 1Y vanilla-call `QuoteRequest(R)` instrument block + the exact
/// vol-time the dialect carries.
fn build_quote_request<'a>(
    req_id: &'a [u8],
    strike: f64,
    expiry_years: f64,
) -> impl FnOnce(&Header<'_>, &mut FrameEncoder) -> Vec<u8> + 'a {
    move |h: &Header<'_>, e: &mut FrameEncoder| {
        e.clear();
        h.encode(MsgType::QuoteRequest, e);
        e.push(131, req_id);
        e.push(55, b"EURUSD");
        e.push(460, b"4"); // Product = CURRENCY
        e.push(167, b"FXVO"); // FX vanilla option
        e.push(201, b"1"); // PutOrCall = call
        e.push(202, format!("{strike}").as_bytes());
        e.push(947, b"USD"); // strike currency = quote
        e.push(1194, b"0"); // European
        e.push(TAG_EXPIRY_YEARS, format!("{expiry_years}").as_bytes());
        e.finish()
    }
}

/// Lift `NewOrderSingle(D)` against a `QuoteID` on a side, with an explicit `ClOrdID`.
fn build_lift<'a>(
    cl_ord_id: &'a [u8],
    quote_id: &'a [u8],
    side: u8,
) -> impl FnOnce(&Header<'_>, &mut FrameEncoder) -> Vec<u8> + 'a {
    move |h: &Header<'_>, e: &mut FrameEncoder| {
        let p = NewOrderParams {
            cl_ord_id,
            quote_id,
            symbol: b"EURUSD",
            side,
            qty: 1_000_000.0,
            transact_time: T,
        };
        messages::build_new_order_single(h, &p, e)
    }
}

/// Build an OIS `QuoteRequest(R)` on the fixed-income dialect at a whole-year tenor.
fn build_rates_rfq<'a>(
    req_id: &'a [u8],
    tenor_years: u32,
    notional: f64,
    side: RatesSide,
) -> impl FnOnce(&Header<'_>, &mut FrameEncoder) -> Vec<u8> + 'a {
    move |h: &Header<'_>, e: &mut FrameEncoder| {
        let p = RatesQuoteRequestParams {
            quote_req_id: req_id,
            symbol: b"USD-OIS",
            tenor_years,
            notional,
            side,
            subscription: SubscriptionRequest::Snapshot,
        };
        dialect_rates::build_rates_quote_request(h, &p, e)
    }
}

/// Lift an OIS quote (`NewOrderSingle(D)` against a `QuoteID` on a side).
fn build_rates_lift<'a>(
    cl_ord_id: &'a [u8],
    quote_id: &'a [u8],
    side: u8,
) -> impl FnOnce(&Header<'_>, &mut FrameEncoder) -> Vec<u8> + 'a {
    move |h: &Header<'_>, e: &mut FrameEncoder| {
        let p = NewOrderParams {
            cl_ord_id,
            quote_id,
            symbol: b"USD-OIS",
            side,
            qty: 100_000_000.0,
            transact_time: T,
        };
        messages::build_new_order_single(h, &p, e)
    }
}

/// The bond symbol the fixed-income e2e uses on the wire.
const BOND_SYMBOL: &[u8] = b"US-TREASURY-5Y";
/// The maker half-spread the bond edge quotes a two-way clean price with, in price
/// points per 100 face (mirrors `fix::BOND_HALF_SPREAD`).
const BOND_HALF_SPREAD: f64 = 0.05;

/// The intended cash bond behind the fixed-income e2e: a 4.5% semi-annual, 30/360 USD
/// bond redeeming at par, maturing on the P0 curve's 5-year anniversary (2031-06-25).
/// The coupon sits above the ~4.05% 5y curve level, so it prices at a premium (clean
/// price > par). The clean price and DV01 are side-independent magnitudes.
fn bond_instrument() -> BondInstrument {
    BondInstrument {
        coupon_rate: 0.045,
        coupon_frequency: PaymentFrequency::SemiAnnual as i32,
        day_count: AccrualBasis::Thirty360BondBasis as i32,
        maturity_date: Some(BrokenDate {
            year: 2031,
            month: 6,
            day: 25,
        }),
        redemption: 100.0,
        side: Side::TwoWay as i32,
    }
}

/// Build a cash-bond `QuoteRequest(R)` on the fixed-income dialect for [`bond_instrument`].
fn build_bond_rfq<'a>(
    req_id: &'a [u8],
    notional: f64,
    side: Side,
) -> impl FnOnce(&Header<'_>, &mut FrameEncoder) -> Vec<u8> + 'a {
    move |h: &Header<'_>, e: &mut FrameEncoder| {
        let p = BondQuoteRequestParams {
            quote_req_id: req_id,
            symbol: BOND_SYMBOL,
            coupon_rate: 0.045,
            coupon_frequency: PaymentFrequency::SemiAnnual,
            day_count: AccrualBasis::Thirty360BondBasis,
            maturity: BrokenDate {
                year: 2031,
                month: 6,
                day: 25,
            },
            redemption: 100.0,
            notional,
            side,
            subscription: SubscriptionRequest::Snapshot,
        };
        dialect_rates::build_bond_quote_request(h, &p, e)
    }
}

/// Lift a bond quote (`NewOrderSingle(D)` against a `QuoteID` on a side).
fn build_bond_lift<'a>(
    cl_ord_id: &'a [u8],
    quote_id: &'a [u8],
    side: u8,
) -> impl FnOnce(&Header<'_>, &mut FrameEncoder) -> Vec<u8> + 'a {
    move |h: &Header<'_>, e: &mut FrameEncoder| {
        let p = NewOrderParams {
            cl_ord_id,
            quote_id,
            symbol: BOND_SYMBOL,
            side,
            qty: 25_000_000.0,
            transact_time: T,
        };
        messages::build_new_order_single(h, &p, e)
    }
}

/// Attach a FIX acceptor to a freshly-booted edge and return its bound address.
async fn boot_edge_with_fix(clock: Clock) -> (celnet_server::Edge, SocketAddr) {
    let (mut edge, _grpc, _data_dir) = start_edge_with(true, clock.clone()).await;
    let fix_addr = edge
        .attach_fix_acceptor(
            "127.0.0.1:0".parse().unwrap(),
            SpreadModel::default(),
            clock,
            VENUE.to_vec(),
            CPTY.to_vec(),
        )
        .await
        .expect("FIX acceptor binds on an ephemeral port");
    (edge, fix_addr)
}

/// RFQ → Quote: the exact-premium field reconciles to the engine/golden price to 1e-12,
/// and a BUY lift books a fill ExecutionReport at exactly that offer (1e-12).
#[tokio::test]
async fn fix_rfq_quote_reprices_to_golden_and_lift_fills() {
    tokio::time::timeout(DEADLINE, async {
        let clock = Clock::system();
        let (edge, fix_addr) = boot_edge_with_fix(clock).await;

        let strike = 1.10;
        let expiry = 1.0;
        let expected_offer = reference_offer(strike, expiry);

        let mut drv = Driver::connect(fix_addr).await;

        // QuoteRequest → Quote.
        drv.send_app(build_quote_request(b"REQ-1", strike, expiry))
            .await;
        let quote_raw = drv.next_app(MsgType::Quote).await;
        let quote = FrameCursor::parse(&quote_raw).expect("a well-formed Quote");
        let view = QuoteView::new(quote);

        let quote_id = view
            .quote_id()
            .expect("the quote carries a QuoteID")
            .to_vec();
        // The exact-premium provenance field reconciles to the engine/golden price to
        // 1e-12 (the standard OfferPx(133) is pip-resolution; the exact tag is the
        // last-bit value the maker booked).
        let offer_exact = view
            .offer_exact()
            .expect("the Quote carries the exact offer premium");
        assert!(
            is_close(offer_exact, expected_offer, 1e-12, 1e-12),
            "FIX quote offer {offer_exact} != engine/golden offer {expected_offer}"
        );
        // The pip-resolution OfferPx is consistent (within wire 8-dp).
        assert!(
            (view.offer().unwrap() - expected_offer).abs() < 5e-9,
            "wire OfferPx within pip resolution of the golden"
        );

        // Lift the offer (BUY) → a fill ExecutionReport at exactly the offer.
        drv.send_app(build_lift(b"ORD-1", &quote_id, b'1')).await;
        let exec_raw = drv.next_app(MsgType::ExecutionReport).await;
        let exec = FrameCursor::parse(&exec_raw).expect("a well-formed ExecutionReport");
        let er = ExecReportView::new(exec);
        assert_eq!(er.exec_type(), Some(EXEC_FILLED), "the lift fills");
        let fill_exact = er
            .last_px_exact()
            .expect("the fill carries the exact premium");
        assert!(
            is_close(fill_exact, expected_offer, 1e-12, 1e-12),
            "FIX fill premium {fill_exact} != quoted offer {expected_offer}"
        );

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test timed out");
}

/// Fixed-income RFQ → Quote on the OIS dialect: the maker shows a two-way RATE
/// market centred on the engine's USD-SOFR par rate (the exact-rate field
/// reconciles to 1e-12), and a pay-fixed (BUY) lift books a fill at exactly the
/// quoted offer rate — proving "the FIX API supports fixed income" runs through
/// the SAME quote / keyed-MAC token / last-look machinery as the FX path.
#[tokio::test]
async fn fix_rates_rfq_quotes_par_and_lift_fills() {
    tokio::time::timeout(DEADLINE, async {
        let clock = Clock::system();
        let (edge, fix_addr) = boot_edge_with_fix(clock).await;

        // First-principles reference: the par rate of the 5y OIS on the P0 static
        // USD-SOFR curve, plus the maker half-spread on the offer side.
        let curve = celnet_server::rates_pricing::default_usd_sofr_curve_set();
        let par = celnet_server::rates_pricing::par_rate_for(&curve, 5).expect("5y par rate");
        let (_bid, expected_offer) = dialect_rates::two_way_rates(par, 0.000_05);

        let mut drv = Driver::connect(fix_addr).await;

        // OIS RFQ → Quote (two-way request).
        drv.send_app(build_rates_rfq(
            b"RREQ-1",
            5,
            100_000_000.0,
            RatesSide::TwoWay,
        ))
        .await;
        let quote_raw = drv.next_app(MsgType::Quote).await;
        let view = QuoteView::new(FrameCursor::parse(&quote_raw).expect("a well-formed Quote"));

        let quote_id = view
            .quote_id()
            .expect("the quote carries a QuoteID")
            .to_vec();
        let offer_exact = view
            .offer_exact()
            .expect("the Quote carries the exact offer rate");
        assert!(
            is_close(offer_exact, expected_offer, 1e-12, 1e-12),
            "FIX rates offer {offer_exact} != engine par+spread {expected_offer}"
        );

        // Pay-fixed lift (BUY) takes the offer → a fill at exactly the quoted rate.
        drv.send_app(build_rates_lift(b"RORD-1", &quote_id, b'1'))
            .await;
        let exec_raw = drv.next_app(MsgType::ExecutionReport).await;
        let er = ExecReportView::new(
            FrameCursor::parse(&exec_raw).expect("a well-formed ExecutionReport"),
        );
        assert_eq!(er.exec_type(), Some(EXEC_FILLED), "the rates lift fills");
        let fill_exact = er.last_px_exact().expect("the fill carries the exact rate");
        assert!(
            is_close(fill_exact, expected_offer, 1e-12, 1e-12),
            "FIX rates fill {fill_exact} != quoted offer {expected_offer}"
        );

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test timed out");
}

// NOTE: the incoming-quote-acceptance gate runs on the DESK-ROUTED FI venue path
// (`on_new_order` → `RfqDeskEdge::evaluate_fix_acceptance` → `book_fix_lift`), which the
// legacy `attach_fix_acceptor` harness here does NOT stand up (it wires no `desk_edge`, so a
// rates lift fills on the token ledger without booking and never reaches the gate). The
// acceptance gate is therefore exercised directly against `evaluate_fix_acceptance` — the
// exact method `on_new_order` invokes — in the `services::desk` unit tests
// (accept-all / reject-by-counterparty / notional-cap / edge-floor / hold), plus the graph
// RPC (persist / round-trip / capability) in the `services::auth` tests and the WS
// byte-identity vectors in `ws_codec_differential`.

/// Fixed-income RFQ → Quote on the cash-BOND dialect: the maker shows a two-way CLEAN
/// PRICE market centred on the engine's clean price for the bond off the P0 static
/// USD-SOFR curve (the exact-price field reconciles to 1e-12), and a BUY lift books a
/// fill at exactly the quoted offer — proving cash bonds are first-class on the FIX edge,
/// running through the SAME quote / keyed-MAC token / last-look machinery as OIS and FX.
///
/// The reference clean price + DV01 come from the LANDED bond engine
/// (`rates_pricing::quote_bond`, itself `to_bits`-identical to `price_rates`(Bond) and
/// QuantLib-gated in `celnet-bond`) applied to a HAND-BUILT [`BondInstrument`]; the FIX
/// server independently decodes the SAME economics off the raw wire frame. A decode or
/// routing bug (wrong frequency/day-count/maturity, or quoting the dirty price / wrong
/// spread) diverges the two and fails — a non-circular identity, not the engine checking
/// itself.
#[tokio::test]
async fn fix_bond_rfq_quotes_clean_price_and_lift_fills() {
    tokio::time::timeout(DEADLINE, async {
        let clock = Clock::system();
        let (edge, fix_addr) = boot_edge_with_fix(clock).await;

        // First-principles reference: the LANDED engine's clean price for the bond, plus
        // the maker half-spread on the offer side.
        let curve = celnet_server::rates_pricing::default_usd_sofr_curve_set();
        let engine = celnet_server::rates_pricing::quote_bond(&bond_instrument(), &curve)
            .expect("the bond prices off the P0 curve");
        let (_bid, expected_offer) =
            dialect_rates::two_way_rates(engine.clean_price, BOND_HALF_SPREAD);

        // Independent sanity on the magnitude (a 4.5% coupon vs a ~4.05% 5y curve is a
        // premium bond): clean price is above par, in a sane band; DV01 is a sane
        // positive per-100-face sensitivity; dirty = clean + accrued.
        assert!(
            engine.clean_price > 100.0 && engine.clean_price < 110.0,
            "premium-bond clean price {} out of sane band",
            engine.clean_price
        );
        assert!(
            engine.dv01 > 0.0 && engine.dv01 < 1.0,
            "bond DV01 {} out of sane band",
            engine.dv01
        );
        assert!(
            (engine.clean_price + engine.accrued_interest - engine.dirty_price).abs() < 1e-9,
            "clean + accrued != dirty"
        );

        let mut drv = Driver::connect(fix_addr).await;

        // Bond RFQ → Quote (two-way request).
        drv.send_app(build_bond_rfq(b"BREQ-1", 25_000_000.0, Side::TwoWay))
            .await;
        let quote_raw = drv.next_app(MsgType::Quote).await;
        let view = QuoteView::new(FrameCursor::parse(&quote_raw).expect("a well-formed Quote"));

        let quote_id = view
            .quote_id()
            .expect("the quote carries a QuoteID")
            .to_vec();
        let offer_exact = view
            .offer_exact()
            .expect("the Quote carries the exact offer price");
        assert!(
            is_close(offer_exact, expected_offer, 1e-12, 1e-12),
            "FIX bond offer {offer_exact} != engine clean+spread {expected_offer}"
        );

        // BUY (long) lift takes the offer → a fill at exactly the quoted clean price.
        drv.send_app(build_bond_lift(b"BORD-1", &quote_id, b'1'))
            .await;
        let exec_raw = drv.next_app(MsgType::ExecutionReport).await;
        let er = ExecReportView::new(
            FrameCursor::parse(&exec_raw).expect("a well-formed ExecutionReport"),
        );
        assert_eq!(er.exec_type(), Some(EXEC_FILLED), "the bond lift fills");
        let fill_exact = er
            .last_px_exact()
            .expect("the fill carries the exact price");
        assert!(
            is_close(fill_exact, expected_offer, 1e-12, 1e-12),
            "FIX bond fill {fill_exact} != quoted offer {expected_offer}"
        );

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test timed out");
}

/// A FORGED QuoteID (never minted by the venue) is rejected — no fill.
#[tokio::test]
async fn fix_forged_token_is_rejected() {
    tokio::time::timeout(DEADLINE, async {
        let clock = Clock::system();
        let (edge, fix_addr) = boot_edge_with_fix(clock).await;
        let mut drv = Driver::connect(fix_addr).await;

        // Lift a QuoteID that was never issued (a forged keyed-MAC value).
        drv.send_app(build_lift(b"ORD-F", b"999999999999", b'1'))
            .await;
        let exec_raw = drv.next_app(MsgType::ExecutionReport).await;
        let er = ExecReportView::new(FrameCursor::parse(&exec_raw).unwrap());
        assert_eq!(
            er.exec_type(),
            Some(EXEC_REJECTED),
            "a forged QuoteID books nothing"
        );

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test timed out");
}

/// A REPLAYED lift (a second lift of an already-executed QuoteID) is rejected as
/// already-consumed — the SAME replay protection the gRPC click-to-trade path enforces.
#[tokio::test]
async fn fix_replayed_token_is_rejected() {
    tokio::time::timeout(DEADLINE, async {
        let clock = Clock::system();
        let (edge, fix_addr) = boot_edge_with_fix(clock).await;
        let mut drv = Driver::connect(fix_addr).await;

        drv.send_app(build_quote_request(b"REQ-R", 1.10, 1.0)).await;
        let quote_raw = drv.next_app(MsgType::Quote).await;
        let quote_id = QuoteView::new(FrameCursor::parse(&quote_raw).unwrap())
            .quote_id()
            .unwrap()
            .to_vec();

        // First lift fills.
        drv.send_app(build_lift(b"ORD-R1", &quote_id, b'1')).await;
        let first = drv.next_app(MsgType::ExecutionReport).await;
        assert_eq!(
            ExecReportView::new(FrameCursor::parse(&first).unwrap()).exec_type(),
            Some(EXEC_FILLED),
        );

        // A replayed lift of the SAME QuoteID is rejected (already consumed).
        drv.send_app(build_lift(b"ORD-R2", &quote_id, b'1')).await;
        let second = drv.next_app(MsgType::ExecutionReport).await;
        assert_eq!(
            ExecReportView::new(FrameCursor::parse(&second).unwrap()).exec_type(),
            Some(EXEC_REJECTED),
            "a replayed lift books nothing (already consumed)"
        );

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test timed out");
}

/// A STALE lift (past the token's last-look validity window) is rejected as expired.
/// Driven on a manual clock so expiry is deterministic without sleeping.
#[tokio::test]
async fn fix_stale_token_is_rejected() {
    tokio::time::timeout(DEADLINE, async {
        // A manual clock the edge + FIX acceptor share; advancing it past the 5s
        // validity window expires the quote's tokens.
        let clock = Clock::manual(1_000_000_000);
        let (edge, fix_addr) = boot_edge_with_fix(clock.clone()).await;
        let mut drv = Driver::connect(fix_addr).await;

        drv.send_app(build_quote_request(b"REQ-S", 1.10, 1.0)).await;
        let quote_raw = drv.next_app(MsgType::Quote).await;
        let quote_id = QuoteView::new(FrameCursor::parse(&quote_raw).unwrap())
            .quote_id()
            .unwrap()
            .to_vec();

        // Advance the shared clock well past the 5-second last-look window.
        clock.advance(6_000_000_000);

        // The lift is now stale → rejected as expired.
        drv.send_app(build_lift(b"ORD-S", &quote_id, b'1')).await;
        let exec_raw = drv.next_app(MsgType::ExecutionReport).await;
        assert_eq!(
            ExecReportView::new(FrameCursor::parse(&exec_raw).unwrap()).exec_type(),
            Some(EXEC_REJECTED),
            "a lift past the last-look window books nothing (expired)"
        );

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test timed out");
}

/// FRONT-END 2 (FIX `on_new_order`, ADR-0016 A1): with a hard firm-wide Delta cap set on
/// the shared position book, a `NewOrderSingle(D)` lift of a live FX quote is refused
/// with a rejected `ExecutionReport(ExecType=8)` carrying a `Text(58) = "limit breached:
/// …"` reason — the SAME shared limit tree the RFS click-to-trade sink enforces, so the
/// FIX venue cannot book a hard-limit-blown fill. A valid, unforged, unexpired token that
/// would otherwise FILL (proven by `fix_rfq_quote_reprices_to_golden_and_lift_fills`) is
/// stopped solely by the limit gate.
#[tokio::test]
async fn fix_hard_limit_blown_lift_is_rejected() {
    tokio::time::timeout(DEADLINE, async {
        use celnet_limits::{LimitMetric, LimitScope, LimitSpec};

        let clock = Clock::system();
        let (edge, fix_addr) = boot_edge_with_fix(clock).await;
        // A firm Delta cap of 1 base unit — a 1mm EURUSD call's delta (hundreds of
        // thousands of base) blows it hard, so an otherwise-fillable lift is refused.
        edge.store()
            .set_limit(LimitScope::Firm, LimitSpec::hard(LimitMetric::Delta, 1.0));

        let mut drv = Driver::connect(fix_addr).await;
        drv.send_app(build_quote_request(b"REQ-L", 1.10, 1.0)).await;
        let quote_raw = drv.next_app(MsgType::Quote).await;
        let quote_id = QuoteView::new(FrameCursor::parse(&quote_raw).unwrap())
            .quote_id()
            .expect("the quote carries a QuoteID")
            .to_vec();

        // A BUY lift now blows the hard firm Delta limit → a rejected ExecutionReport.
        drv.send_app(build_lift(b"ORD-L", &quote_id, b'1')).await;
        let exec_raw = drv.next_app(MsgType::ExecutionReport).await;
        let exec = FrameCursor::parse(&exec_raw).expect("a well-formed ExecutionReport");
        assert_eq!(
            ExecReportView::new(exec).exec_type(),
            Some(EXEC_REJECTED),
            "a hard-limit-blown FIX lift books nothing"
        );
        let text = exec
            .get(58)
            .expect("a limit-rejected lift carries a Text(58) reason");
        let text_str = std::str::from_utf8(text).unwrap_or("");
        assert!(
            text_str.contains("limit breached"),
            "the FIX reject carries the uniform LimitBreached reason, got {text_str:?}"
        );

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test timed out");
}
