//! **End-to-end street-order routing over real loopback sockets.**
//!
//! Every test here stands up a genuine simulated counterparty — the same
//! [`celnet_lp_sim::OrderVenue`] / [`celnet_cme_sim`] venue the deployed simulators run,
//! with its own roster profile, depth ladder, whole-lot rules and reject vocabulary —
//! binds it on `127.0.0.1:0`, and drives the production hedge seam
//! ([`execute_external`]) through the production router ([`FixStreetRouter`]) at it.
//!
//! There are no stubs on the path. A fill in these tests exists because a counterparty
//! read a `NewOrderSingle(D)` off a socket, matched it against a book it had published,
//! and wrote back an `ExecutionReport(8)` — which is exactly the claim the street-side
//! blotter makes and which, before this seam existed, it could not support.
//!
//! Each connection is wrapped in a [`Tap`] that records the literal bytes crossing the
//! socket in both directions, so the assertions are made against the real frames and
//! `cargo test -- --nocapture` prints the wire.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use celnet_lp_sim::orders::{LiveMarkets, OrderVenue, QuotedMarket, serve_orders};
use celnet_lp_sim::roster::{OTC_ROSTER, profile_by_id};
use celnet_server::services::auto_hedge::{
    ExternalHedgeRequest, HEDGE_ORD_TYPE, HEDGE_TIME_IN_FORCE, HedgeVenue, LpFill, LpHedgeSource,
    RouteAnswer, RouteOutcome, StreetOrderIntent, StreetOrderRouter, execute_external,
};
use celnet_server::services::street_router::{FixStreetRouter, NO_ENDPOINT_REASON, OrderEndpoint};

// ---------------------------------------------------------------------------
// A recorded loopback venue
// ---------------------------------------------------------------------------

/// The bytes seen on one connection, in order, tagged by direction.
type WireLog = Arc<Mutex<Vec<(Direction, Vec<u8>)>>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Direction {
    /// CelNet → counterparty.
    Inbound,
    /// Counterparty → CelNet.
    Outbound,
}

/// A transparent stream wrapper that records every byte the venue reads and writes.
///
/// Wrapping the venue's own socket (rather than proxying) keeps the test on ONE TCP
/// connection: what is recorded is exactly what the router put on the wire.
struct Tap<S> {
    inner: S,
    log: WireLog,
}

impl<S: tokio::io::AsyncRead + Unpin> tokio::io::AsyncRead for Tap<S> {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let polled = std::pin::Pin::new(&mut self.inner).poll_read(cx, buf);
        if matches!(polled, std::task::Poll::Ready(Ok(()))) {
            let fresh = buf.filled()[before..].to_vec();
            if !fresh.is_empty() {
                self.log
                    .lock()
                    .expect("wire log")
                    .push((Direction::Inbound, fresh));
            }
        }
        polled
    }
}

impl<S: tokio::io::AsyncWrite + Unpin> tokio::io::AsyncWrite for Tap<S> {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        let polled = std::pin::Pin::new(&mut self.inner).poll_write(cx, buf);
        if let std::task::Poll::Ready(Ok(n)) = polled
            && n > 0
        {
            let written = buf[..n].to_vec();
            self.log
                .lock()
                .expect("wire log")
                .push((Direction::Outbound, written));
        }
        polled
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

/// A live simulated counterparty on loopback: its bound address, the markets it is
/// showing, and the bytes that have crossed its socket.
struct LiveVenue {
    addr: std::net::SocketAddr,
    log: WireLog,
    /// Kept alive for the venue's lifetime — dropping it stops the acceptor.
    _runtime: tokio::runtime::Runtime,
}

impl LiveVenue {
    /// Bind `venue` on an ephemeral loopback port, serving every connecting taker on a
    /// runtime of its own (so the venue and the router never share threads — exactly the
    /// deployed topology, where they are separate processes).
    fn bind(venue: OrderVenue) -> Self {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("venue runtime");
        let log: WireLog = Arc::new(Mutex::new(Vec::new()));
        let listener = runtime
            .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
            .expect("bind venue");
        let addr = listener.local_addr().expect("venue addr");
        let venue_log = Arc::clone(&log);
        runtime.spawn(async move {
            loop {
                let Ok((stream, _peer)) = listener.accept().await else {
                    return;
                };
                let v = venue.clone();
                let l = Arc::clone(&venue_log);
                tokio::spawn(async move {
                    let tapped = Tap {
                        inner: stream,
                        log: l,
                    };
                    let _ =
                        serve_orders(&v, tapped, fix_stamp(), celnet_lp_sim::orders::wall_nanos)
                            .await;
                });
            }
        });
        Self {
            addr,
            log,
            _runtime: runtime,
        }
    }

    /// The frames recorded in `direction`, decoded as SOH-delimited FIX text.
    fn frames(&self, direction: Direction) -> Vec<String> {
        let guard = self.log.lock().expect("wire log");
        let joined: Vec<u8> = guard
            .iter()
            .filter(|(d, _)| *d == direction)
            .flat_map(|(_, b)| b.clone())
            .collect();
        split_frames(&joined)
    }

    /// The one recorded frame of `msg_type` (`D`, `8`, …), panicking if there is not
    /// exactly one — an assertion that the conversation was the one we expected.
    fn only_frame(&self, direction: Direction, msg_type: &str) -> String {
        let mut hits: Vec<String> = self
            .frames(direction)
            .into_iter()
            .filter(|f| f.contains(&format!("|35={msg_type}|")))
            .collect();
        assert_eq!(
            hits.len(),
            1,
            "expected exactly one 35={msg_type} frame, saw {hits:#?}"
        );
        hits.pop().expect("checked above")
    }
}

/// Split a byte stream into complete FIX frames, rendering SOH as `|` for readability.
fn split_frames(bytes: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(bytes).replace('\x01', "|");
    let mut out = Vec::new();
    let mut rest = text.as_str();
    // A frame ends at its checksum field `10=nnn|`.
    while let Some(idx) = rest.find("|10=") {
        let end = rest[idx + 1..]
            .find('|')
            .map_or(rest.len(), |e| idx + 1 + e + 1);
        out.push(rest[..end].to_owned());
        rest = &rest[end..];
    }
    out
}

/// A FIX `SendingTime` stamp for the venue's own frames.
fn fix_stamp() -> Vec<u8> {
    b"20260814-09:00:00.000".to_vec()
}

/// The market every OTC test publishes: a firm 99.90 / 100.10 in 1mm a side.
fn market() -> QuotedMarket {
    QuotedMarket {
        bid: 99.90,
        offer: 100.10,
        bid_size: 1_000_000.0,
        offer_size: 1_000_000.0,
        ts_nanos: celnet_lp_sim::orders::wall_nanos(),
        lot_size: None,
    }
}

/// Stand up counterparty `id` showing `market` on `INSTRUMENT`.
fn counterparty(id: &str, market: QuotedMarket) -> LiveVenue {
    let profile = profile_by_id(OTC_ROSTER, id).expect("roster member");
    let markets: LiveMarkets = Arc::new(std::sync::RwLock::new(std::collections::BTreeMap::new()));
    let venue = OrderVenue::from_profile(profile, 2.0e-2, markets);
    venue.publish(INSTRUMENT, market);
    LiveVenue::bind(venue)
}

/// The security every test deals — a real US Treasury CUSIP from the bundled universe.
const INSTRUMENT: &str = "912810TZ1";

/// Point a router at `members`, each `(id, addr)`.
fn router_for(members: &[(&str, std::net::SocketAddr)], timeout: Duration) -> Arc<FixStreetRouter> {
    let router = FixStreetRouter::with_timeout(timeout);
    router.set_endpoints(
        members
            .iter()
            .map(|(id, addr)| OrderEndpoint {
                member_id: (*id).to_owned(),
                addr: addr.to_string(),
                sender_comp_id: "CELNET".to_owned(),
                target_comp_id: (*id).to_owned(),
            })
            .collect(),
    );
    router
}

/// A quote source that ranks a fixed panel — standing in for the aggregation hub's
/// inbound quotes, which is the ONLY thing being substituted here. Everything that
/// decides whether a hedge fills is the real counterparty.
struct Panel(Vec<LpFill>);
impl LpHedgeSource for Panel {
    fn rank(&self, _instrument: &str, _net_risk: f64, _size: f64) -> Vec<LpFill> {
        self.0.clone()
    }
}

/// A shed of `size`, reducing a long (so we SELL into the panel's bid).
fn shed(
    size: f64,
    mode: celnet_server::config::hedge_policy::HedgeExecutionMode,
) -> ExternalHedgeRequest<'static> {
    ExternalHedgeRequest {
        instrument: INSTRUMENT,
        net_risk: 50_000.0,
        size,
        mid: 100.0,
        bp_scale: 1e-2,
        mode,
        composite_spread_bp: 0.5,
    }
}

// ---------------------------------------------------------------------------
// The tests
// ---------------------------------------------------------------------------

/// **The headline claim.** A hedge decision produces a real `NewOrderSingle(D)` on a
/// real socket, a real counterparty matches it against the book it published, and the
/// booked fill is that counterparty's own `ExecutionReport(8)` — not an inference from
/// its standing quote.
#[test]
fn a_hedge_sends_a_real_order_and_books_the_counterpartys_own_report() {
    let venue = counterparty("citigroup-sim", market());
    let router = router_for(&[("citigroup-sim", venue.addr)], Duration::from_secs(2));
    let panel = Panel(vec![LpFill {
        lp_id: "citigroup-sim".to_owned(),
        price: 99.90,
    }]);

    let fill = execute_external(
        &shed(
            500_000.0,
            celnet_server::config::hedge_policy::HedgeExecutionMode::LpPanel,
        ),
        &panel,
        router.as_ref(),
    );

    assert_eq!(fill.venue, Some(HedgeVenue::LpPanel));
    assert_eq!(fill.lp_won.as_deref(), Some("citigroup-sim"));
    assert_eq!(fill.filled, 500_000.0);
    assert_eq!(fill.residual, 0.0);
    assert!(
        (fill.hedge_price - 99.90).abs() < 1e-9,
        "{}",
        fill.hedge_price
    );
    // Sold 10bp of clean price below the 100.00 reference ⇒ −10bp of slippage.
    assert!(
        (fill.slippage_bp - (-10.0)).abs() < 1e-9,
        "{}",
        fill.slippage_bp
    );

    let attempt = fill.attempts.first().expect("one routed order");
    assert_eq!(attempt.outcome, RouteOutcome::Filled);
    assert_eq!(attempt.order_type, HEDGE_ORD_TYPE);
    assert_eq!(attempt.time_in_force, HEDGE_TIME_IN_FORCE);
    let latency = attempt
        .response_latency_nanos
        .expect("a real round trip was measured");
    assert!(latency > 0, "a socket round trip cannot take zero time");

    // --- the wire itself -----------------------------------------------------
    let order = venue.only_frame(Direction::Inbound, "D");
    println!("NewOrderSingle sent:      {order}");
    assert!(order.contains("|49=CELNET|"), "SenderCompID: {order}");
    assert!(
        order.contains("|56=citigroup-sim|"),
        "TargetCompID: {order}"
    );
    assert!(
        order.contains(&format!("|55={INSTRUMENT}|")),
        "Symbol: {order}"
    );
    assert!(order.contains("|54=2|"), "Side=Sell: {order}");
    assert!(order.contains("|38=500000.00000000|"), "OrderQty: {order}");
    assert!(
        order.contains("|44=99.90000000|"),
        "Price at the LP's own bid: {order}"
    );
    assert!(order.contains("|40=2|"), "OrdType=Limit: {order}");
    assert!(order.contains("|59=3|"), "TimeInForce=IOC: {order}");

    let report = venue.only_frame(Direction::Outbound, "8");
    println!("ExecutionReport received: {report}");
    assert!(
        report.contains("|150=F|") && report.contains("|39=F|"),
        "{report}"
    );
    assert!(report.contains("|32=500000.00000000|"), "LastQty: {report}");
    assert!(report.contains("|31=99.90000000|"), "LastPx: {report}");
    assert!(
        !report.contains("|58="),
        "a clean fill carries no Text: {report}"
    );
}

/// **IOC partial.** A clip deeper than the counterparty's eligible depth fills what is
/// there and leaves an honest residual — carrying the venue's own reason for why it did
/// not complete.
#[test]
fn an_ioc_beyond_the_depth_partially_fills_and_leaves_a_real_residual() {
    let venue = counterparty("marketaccess-sim", market());
    let router = router_for(&[("marketaccess-sim", venue.addr)], Duration::from_secs(2));
    let panel = Panel(vec![LpFill {
        lp_id: "marketaccess-sim".to_owned(),
        price: 99.90,
    }]);

    let clip = 9_000_000.0;
    let fill = execute_external(
        &shed(
            clip,
            celnet_server::config::hedge_policy::HedgeExecutionMode::LpPanel,
        ),
        &panel,
        router.as_ref(),
    );

    assert_eq!(fill.venue, Some(HedgeVenue::LpPanel));
    assert!(
        fill.filled > 0.0 && fill.filled < clip,
        "filled {}",
        fill.filled
    );
    assert!(
        (fill.filled + fill.residual - clip).abs() < 1e-6,
        "filled {} + residual {} must reconstruct the clip",
        fill.filled,
        fill.residual
    );
    let attempt = &fill.attempts[0];
    assert_eq!(attempt.outcome, RouteOutcome::PartiallyFilled);
    let reason = attempt.reason.as_deref().expect("a partial states why");
    assert!(
        reason.starts_with("IOC_"),
        "the venue's own partial-fill code, got {reason}"
    );

    let report = venue.only_frame(Direction::Outbound, "8");
    println!("ExecutionReport received: {report}");
    assert!(
        report.contains("|39=1|"),
        "OrdStatus=PartiallyFilled: {report}"
    );
}

/// **FOK all-or-nothing.** The same clip against the same book, sent fill-or-kill, is
/// killed in full rather than half-done — the counterparty, not us, enforces that.
#[test]
fn a_fill_or_kill_beyond_the_depth_is_killed_in_full() {
    let venue = counterparty("marketaccess-sim", market());
    let router = router_for(&[("marketaccess-sim", venue.addr)], Duration::from_secs(2));

    let answer = router.route(&StreetOrderIntent {
        lp_id: "marketaccess-sim",
        instrument: INSTRUMENT,
        side: celnet_analytics::StreetSide::Sell,
        quantity: 9_000_000.0,
        limit_price: 99.90,
        ord_type: HEDGE_ORD_TYPE,
        time_in_force: celnet_fix::messages::time_in_force::FILL_OR_KILL,
    });

    match answer {
        RouteAnswer::Cancelled { reason, .. } => assert_eq!(reason, "FOK_UNFILLABLE"),
        other => panic!("a killed FOK must cancel with its reason, got {other:?}"),
    }
    let order = venue.only_frame(Direction::Inbound, "D");
    assert!(
        order.contains("|59=4|"),
        "TimeInForce=FOK on the wire: {order}"
    );
    let report = venue.only_frame(Direction::Outbound, "8");
    println!("ExecutionReport received: {report}");
    assert!(
        report.contains("|39=4|"),
        "a killed FOK is CANCELLED, not rejected: {report}"
    );
    assert!(
        report.contains("|32=0.00000000|"),
        "nothing traded: {report}"
    );
}

/// **Limit away from the market.** A counterparty that will not trade at the price it is
/// being asked to honour is recorded as a last-look pull — a fact about that
/// counterparty, and never a fabricated fill at the level it was showing.
#[test]
fn a_limit_the_venue_will_not_honour_is_a_last_look_pull() {
    let venue = counterparty("jpm-sim", market());
    let router = router_for(&[("jpm-sim", venue.addr)], Duration::from_secs(2));

    // We sell, but demand a price ABOVE the venue's bid — away from its market.
    let answer = router.route(&StreetOrderIntent {
        lp_id: "jpm-sim",
        instrument: INSTRUMENT,
        side: celnet_analytics::StreetSide::Sell,
        quantity: 100_000.0,
        limit_price: 100.50,
        ord_type: HEDGE_ORD_TYPE,
        time_in_force: HEDGE_TIME_IN_FORCE,
    });
    match answer {
        RouteAnswer::LastLookPulled { reason, .. } => assert_eq!(reason, "NOT_MARKETABLE"),
        other => panic!("expected a last-look pull, got {other:?}"),
    }
    let report = venue.only_frame(Direction::Outbound, "8");
    println!("ExecutionReport received: {report}");
    assert!(report.contains("|39=4|"), "{report}");
}

/// **Whole-lot rejection.** A listed venue refusing a fractional-contract clip is an
/// `OrdStatus=8` REJECT — the venue would not accept the order at all, which the record
/// must never conflate with a cancel.
#[test]
fn a_fractional_lot_on_a_listed_venue_is_rejected_not_cancelled() {
    let markets: LiveMarkets = Arc::new(std::sync::RwLock::new(std::collections::BTreeMap::new()));
    let listed = celnet_cme_sim::build_order_venue(Arc::clone(&markets), 7);
    let contract = "ZFZ26";
    listed.publish(
        contract,
        QuotedMarket {
            bid: 108.50,
            offer: 108.515_625,
            bid_size: 500.0,
            offer_size: 500.0,
            ts_nanos: celnet_lp_sim::orders::wall_nanos(),
            // Futures trade in whole contracts — the rule the venue enforces below.
            lot_size: Some(1.0),
        },
    );
    let venue_id = listed.id();
    let venue = LiveVenue::bind(listed);
    let router = router_for(&[(venue_id, venue.addr)], Duration::from_secs(2));

    let answer = router.route(&StreetOrderIntent {
        lp_id: venue_id,
        instrument: contract,
        side: celnet_analytics::StreetSide::Sell,
        quantity: 12.5,
        limit_price: 108.50,
        ord_type: HEDGE_ORD_TYPE,
        time_in_force: HEDGE_TIME_IN_FORCE,
    });
    match answer {
        RouteAnswer::Rejected { reason, .. } => assert_eq!(reason, "NOT_A_WHOLE_LOT"),
        other => panic!("expected a venue rejection, got {other:?}"),
    }
    let report = venue.only_frame(Direction::Outbound, "8");
    println!("ExecutionReport received: {report}");
    assert!(
        report.contains("|39=8|"),
        "a refusal is OrdStatus=8, distinct from a cancel: {report}"
    );

    // …and a WHOLE number of contracts on the same venue trades.
    let ok = router.route(&StreetOrderIntent {
        lp_id: venue_id,
        instrument: contract,
        side: celnet_analytics::StreetSide::Sell,
        quantity: 12.0,
        limit_price: 108.50,
        ord_type: HEDGE_ORD_TYPE,
        time_in_force: HEDGE_TIME_IN_FORCE,
    });
    match ok {
        RouteAnswer::Traded(f) => assert_eq!(f.filled, 12.0),
        other => panic!("a whole-lot clip should trade, got {other:?}"),
    }
}

/// **A venue that never answers.** A silent counterparty produces a recorded `Expired`
/// with the measured wait — never a hang, and never a composite backstop that would read
/// as "the street showed nothing".
#[test]
fn a_venue_that_never_answers_expires_with_a_measured_wait() {
    // A listener that completes the TCP handshake and then says nothing at all: the
    // worst realistic failure, because the socket looks perfectly healthy.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("mute venue runtime");
    let listener = runtime
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .expect("bind mute venue");
    let addr = listener.local_addr().expect("addr");
    let held = Arc::new(Mutex::new(Vec::new()));
    let keep = Arc::clone(&held);
    runtime.spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            // Hold the socket open, read nothing, write nothing.
            keep.lock().expect("held").push(stream);
        }
    });

    let timeout = Duration::from_millis(300);
    let router = FixStreetRouter::with_timeout(timeout);
    router.set_endpoints(vec![OrderEndpoint {
        member_id: "mute-sim".to_owned(),
        addr: addr.to_string(),
        sender_comp_id: "CELNET".to_owned(),
        target_comp_id: "mute-sim".to_owned(),
    }]);

    let started = std::time::Instant::now();
    let panel = Panel(vec![LpFill {
        lp_id: "mute-sim".to_owned(),
        price: 99.90,
    }]);
    let fill = execute_external(
        &shed(
            250_000.0,
            celnet_server::config::hedge_policy::HedgeExecutionMode::LpPanelThenComposite,
        ),
        &panel,
        router.as_ref(),
    );
    let elapsed = started.elapsed();

    // It returned, and it returned promptly — the deadline is honoured, not merely
    // documented.
    assert!(
        elapsed < Duration::from_secs(5),
        "a silent venue must not hold the booking thread: waited {elapsed:?}"
    );
    let attempt = &fill.attempts[0];
    assert!(
        matches!(
            attempt.outcome,
            RouteOutcome::Expired | RouteOutcome::Unroutable
        ),
        "a silent venue is expired or unroutable, got {:?}",
        attempt.outcome
    );
    assert!(attempt.reason.is_some(), "the outcome states why");
    // The shed still gets hedged — on the composite, which credits nobody.
    assert_eq!(fill.venue, Some(HedgeVenue::Composite));
    assert_eq!(fill.lp_won.as_deref(), Some(HedgeVenue::COMPOSITE_LABEL));
}

/// **A member with no order endpoint.** It still ranks on the panel (it is genuinely
/// quoting) but an order to it is answered with a stated configuration reason, and the
/// shed backstops to the composite crediting nobody — it is never filled from the quote.
#[test]
fn a_member_with_no_endpoint_degrades_to_a_stated_reason_and_a_credited_backstop() {
    let router = FixStreetRouter::new();
    let panel = Panel(vec![LpFill {
        lp_id: "unconfigured-sim".to_owned(),
        price: 99.90,
    }]);
    let fill = execute_external(
        &shed(
            250_000.0,
            celnet_server::config::hedge_policy::HedgeExecutionMode::LpPanelThenComposite,
        ),
        &panel,
        router.as_ref(),
    );

    assert_eq!(
        fill.attempts.len(),
        1,
        "the member was addressed, not skipped"
    );
    assert_eq!(fill.attempts[0].outcome, RouteOutcome::Unroutable);
    assert_eq!(fill.attempts[0].reason.as_deref(), Some(NO_ENDPOINT_REASON));
    assert!(
        fill.attempts[0].response_latency_nanos.is_none(),
        "nothing was sent, so nothing was timed"
    );
    assert_eq!(fill.venue, Some(HedgeVenue::Composite));
    assert_eq!(
        fill.lp_won.as_deref(),
        Some(HedgeVenue::COMPOSITE_LABEL),
        "a backstop credits no counterparty"
    );
    assert!(
        fill.panel.is_empty(),
        "a backstop competed against nothing it could deal on"
    );
}

/// **Walking the panel.** A counterparty that refuses does not end the hedge: the order
/// goes to the next-ranked member, and both orders are on the record because both really
/// went out.
#[test]
fn a_refusal_walks_to_the_next_member_and_both_orders_are_recorded() {
    // `refuser` shows a market it cannot trade (crossed ⇒ never tradeable), so its
    // decline is the venue's own, not a scripted one.
    let refuser = counterparty(
        "jpm-sim",
        QuotedMarket {
            bid: 100.20,
            ..market()
        },
    );
    let filler = counterparty("citigroup-sim", market());
    let router = router_for(
        &[("jpm-sim", refuser.addr), ("citigroup-sim", filler.addr)],
        Duration::from_secs(2),
    );
    let panel = Panel(vec![
        LpFill {
            lp_id: "jpm-sim".to_owned(),
            price: 99.92,
        },
        LpFill {
            lp_id: "citigroup-sim".to_owned(),
            price: 99.90,
        },
    ]);

    let fill = execute_external(
        &shed(
            400_000.0,
            celnet_server::config::hedge_policy::HedgeExecutionMode::LpPanelThenComposite,
        ),
        &panel,
        router.as_ref(),
    );

    assert_eq!(
        fill.attempts.len(),
        2,
        "two orders genuinely left the building"
    );
    assert_eq!(fill.attempts[0].lp_id, "jpm-sim");
    assert!(
        !fill.attempts[0].outcome.is_fill(),
        "the refuser did not trade"
    );
    assert_eq!(fill.attempts[1].lp_id, "citigroup-sim");
    assert_eq!(fill.attempts[1].outcome, RouteOutcome::Filled);
    assert_eq!(fill.venue, Some(HedgeVenue::LpPanel));
    assert_eq!(fill.lp_won.as_deref(), Some("citigroup-sim"));
    // Dealt at the price we actually got, not the better one we could not have.
    assert!(
        (fill.hedge_price - 99.90).abs() < 1e-9,
        "{}",
        fill.hedge_price
    );

    println!(
        "refuser saw:  {}",
        refuser.only_frame(Direction::Inbound, "D")
    );
    println!(
        "filler saw:   {}",
        filler.only_frame(Direction::Inbound, "D")
    );
    // Both counterparties were genuinely addressed.
    assert!(!refuser.frames(Direction::Inbound).is_empty());
    assert!(!filler.frames(Direction::Inbound).is_empty());
}

/// **One session, many orders.** The router logs on ONCE per counterparty and reuses the
/// session — a hedge desk does not churn a logon per order, and the sequence numbers
/// prove it did not.
#[test]
fn many_orders_share_one_logged_on_session() {
    let venue = counterparty("traderweb-sim", market());
    let router = router_for(&[("traderweb-sim", venue.addr)], Duration::from_secs(2));

    for _ in 0..3 {
        let answer = router.route(&StreetOrderIntent {
            lp_id: "traderweb-sim",
            instrument: INSTRUMENT,
            side: celnet_analytics::StreetSide::Sell,
            quantity: 100_000.0,
            limit_price: 99.90,
            ord_type: HEDGE_ORD_TYPE,
            time_in_force: HEDGE_TIME_IN_FORCE,
        });
        assert!(
            matches!(answer, RouteAnswer::Traded(_)),
            "each order should trade, got {answer:?}"
        );
    }

    let inbound = venue.frames(Direction::Inbound);
    let logons = inbound.iter().filter(|f| f.contains("|35=A|")).count();
    assert_eq!(logons, 1, "one logon for three orders: {inbound:#?}");
    let orders: Vec<&String> = inbound.iter().filter(|f| f.contains("|35=D|")).collect();
    assert_eq!(orders.len(), 3);
    // Each order carries a distinct ClOrdID and an advancing MsgSeqNum on ONE session.
    assert!(
        orders[0].contains("|11=CN-traderweb-sim-1|"),
        "{:?}",
        orders[0]
    );
    assert!(
        orders[2].contains("|11=CN-traderweb-sim-3|"),
        "{:?}",
        orders[2]
    );
    assert!(orders[0].contains("|34=2|"), "{:?}", orders[0]);
    assert!(orders[2].contains("|34=4|"), "{:?}", orders[2]);
}
