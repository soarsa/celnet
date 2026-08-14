//! End-to-end check that the **Treasury futures complex actually reaches an
//! aggregated book, and can then be traded** — the two properties the futures venue
//! exists for.
//!
//! A DV01-ratio hedge on a corporate position is expressed in a benchmark Treasury
//! future. The server seeds those contracts into its tradeable reference registry, so
//! if no feed **quotes** them they never reach an aggregated book,
//! `AggregationHub::best_fill` finds no composite line, and every futures hedge
//! backstops to the synthetic COMPOSITE venue. Exactly that tradeable-but-unquotable
//! asymmetry already caused a production defect on the freshly-auctioned Treasuries.
//! And a venue that quotes but will not trade is only half a venue.
//!
//! So this walks the real path, with no mocks anywhere:
//!
//! 1. the venue's quotable set is built from the SAME `celnet-refdata` universe the
//!    server seeds its registry from, and must contain every listed contract;
//! 2. an operator-style aggregated book naming `cme-sim` resolves a streaming plan
//!    that includes every contract;
//! 3. one round produces well-formed, on-tick-grid, uncrossed, whole-contract markets
//!    that the REAL `celnet_aggregation::ConsolidatedBook` engine folds into an
//!    uncrossed composite — a crossed composite is rejected by the server's RFQ
//!    resolver, which would starve the hedge the venue exists to fill;
//! 4. the composite is usable as a hedge: the DV01-ratio contract count off the
//!    contract's derived DV01 is the ratio in the requirement;
//! 5. an order sent to the venue over a **real FIX session on a real loopback
//!    socket** comes back as a real `ExecutionReport(8)`, with FOK all-or-nothing,
//!    IOC partial-fill and whole-lot semantics all honoured.

use std::collections::{BTreeMap, BTreeSet};

use celnet_aggregation::{ConsolidatedBook, ConsolidationConfig, VenueFeed};
use celnet_cme_sim::{
    VENUE_ID, as_of, build_order_venue, build_venue_feed, contract_lot_size, futures_lines,
    load_futures_universe, publish_markets, resolve_contract, venue,
};
use celnet_lp_sim::execution::{
    Execution, OrderRequest, OrderType, RejectReason, Side, TimeInForce,
};
use celnet_lp_sim::orders::LiveMarkets;
use celnet_lp_sim::{QuotedLine, load_government_universe, resolve_from_descs};
use celnet_proto::{AggregatedBookDesc, AggregationScopeMode};
use celnet_types::BrokenDate;

const S: i64 = 1_000_000_000;
const NOW: i64 = 100 * S;
const SEED: u64 = 0xFEED_1234;

fn settlement() -> BrokenDate {
    BrokenDate::new(2026, 4, 16)
}

fn contracts() -> Vec<celnet_cme_sim::FuturesContract> {
    load_futures_universe(&load_government_universe(false), settlement())
}

fn lines(cs: &[celnet_cme_sim::FuturesContract]) -> Vec<QuotedLine> {
    futures_lines(
        cs,
        venue::BASE_HALF_SPREAD,
        venue::BASE_SKEW_STEP,
        0.02,
        3.0e-4,
    )
}

/// The listed contract codes, straight from reference data.
fn contract_codes() -> Vec<String> {
    celnet_refdata::treasury_futures_universe()
        .into_iter()
        .map(|s| s.instrument_id)
        .collect()
}

fn specs() -> BTreeMap<String, celnet_refdata::TreasuryFutureSpec> {
    celnet_refdata::treasury_futures_universe()
        .into_iter()
        .map(|s| (s.instrument_id.clone(), s))
        .collect()
}

#[test]
fn the_venue_quotes_every_tradeable_futures_contract() {
    let cs = contracts();
    let ls = lines(&cs);
    let quotable: BTreeSet<&str> = ls.iter().map(QuotedLine::instrument_id).collect();

    let codes = contract_codes();
    assert!(!codes.is_empty(), "reference data lists no futures");
    for code in &codes {
        assert!(
            quotable.contains(code.as_str()),
            "{code} is tradeable (the server seeds it) but NOT quotable — it would \
             never reach an aggregated book and every hedge on it would backstop to \
             the synthetic COMPOSITE venue"
        );
    }
    assert_eq!(
        quotable.len(),
        codes.len(),
        "the venue quotes only the listed complex"
    );
}

#[test]
fn a_book_naming_the_venue_plans_every_contract() {
    let cs = contracts();
    let ls = lines(&cs);
    let members: BTreeSet<String> = [VENUE_ID.to_owned()].into_iter().collect();
    let priceable: BTreeSet<String> = ls.iter().map(|l| l.instrument_id.clone()).collect();

    let books = vec![AggregatedBookDesc {
        id: "ust".to_string(),
        name: "UST composite".to_string(),
        member_connection_ids: vec![VENUE_ID.to_owned()],
        scope_mode: AggregationScopeMode::AllMembersQuote as i32,
        instrument_ids: Vec::new(),
        params: None,
        enabled: true,
    }];
    let plan = resolve_from_descs(&books, &members, &priceable);

    for line in &ls {
        assert!(
            plan.contains(VENUE_ID, &line.instrument_id),
            "{VENUE_ID} does not stream {}",
            line.instrument_id
        );
    }
    assert_eq!(plan.len(), ls.len());
}

#[test]
fn one_round_is_on_grid_uncrossed_and_in_whole_contracts() {
    let cs = contracts();
    let ls = lines(&cs);
    let feed = build_venue_feed(&ls, SEED);
    let markets = LiveMarkets::default();
    let ov = build_order_venue(LiveMarkets::clone(&markets), SEED);
    assert_eq!(publish_markets(&feed, &ov, &ls, &cs, NOW), ls.len());

    let specs = specs();
    let book = markets.read().expect("markets");
    for line in &ls {
        let m = book.get(&line.instrument_id).expect("published");
        let spec = &specs[&line.instrument_id];
        let tick = spec.terms.tick_size_points;

        assert!(m.is_tradeable(), "{}: not tradeable", line.instrument_id);
        assert!(
            m.offer > m.bid,
            "{}: crossed or locked market",
            line.instrument_id
        );
        for px in [m.bid, m.offer] {
            let ticks = px / tick;
            assert!(
                (ticks - ticks.round()).abs() < 1e-6,
                "{}: {px} is off the published tick grid ({tick})",
                line.instrument_id
            );
        }
        assert!(
            m.offer - m.bid <= 3.0 * tick + 1e-12,
            "{}: {} ticks wide is not a listed market",
            line.instrument_id,
            (m.offer - m.bid) / tick
        );
        // Whole contracts on BOTH sides.
        let lot = m.lot_size.expect("a listed market has a lot size");
        assert_eq!(lot, spec.terms.face_value);
        for size in [m.bid_size, m.offer_size] {
            let n = size / lot;
            assert!(
                (n - n.round()).abs() < 1e-9 && n >= 1.0,
                "{}: {size} is {n} contracts, not a whole number",
                line.instrument_id
            );
        }
    }
}

#[test]
fn the_real_engine_consolidates_every_contract_to_an_uncrossed_composite() {
    let cs = contracts();
    let ls = lines(&cs);
    let feeds: Vec<Box<dyn VenueFeed>> = vec![Box::new(build_venue_feed(&ls, SEED))];
    let ccfg = ConsolidationConfig {
        staleness_half_life_secs: 30.0,
        staleness_cutoff_secs: 60.0,
        divergence_tolerance: 0.50,
    };
    let specs = specs();

    for line in &ls {
        let spec = &specs[&line.instrument_id];
        let tick = spec.terms.tick_size_points;
        let book = ConsolidatedBook::consolidate(&feeds, &line.instrument, NOW, &ccfg)
            .unwrap_or_else(|e| panic!("{}: no composite: {e}", line.instrument_id));

        // THE property: a listed venue must not produce a crossed composite. The
        // server's `resolve_rfq_composite` rejects `best_bid > best_offer` outright,
        // so a crossed line is an unfillable hedge. A single central market makes
        // this structural rather than a budgeting exercise, which is exactly why the
        // venue was split out of the four-member OTC panel.
        assert!(
            book.best_bid <= book.best_offer,
            "{}: CROSSED composite — bid {} > offer {}",
            line.instrument_id,
            book.best_bid,
            book.best_offer
        );
        assert!(
            book.best_offer - book.best_bid <= 3.0 * tick + 1e-12,
            "{}: composite {} ticks wide",
            line.instrument_id,
            (book.best_offer - book.best_bid) / tick
        );
        assert_eq!(book.contributions.len(), 1, "one central market");
        assert!(book.contributions.iter().all(|c| c.excluded.is_none()));

        // The analytic ground truth over the venue's own top-of-book.
        let q = feeds[0]
            .top_of_book(&line.instrument, NOW)
            .expect("the venue quotes it");
        assert!(
            (book.best_bid - q.bid).abs() < 1e-9,
            "{}",
            line.instrument_id
        );
        assert!(
            (book.best_offer - q.offer).abs() < 1e-9,
            "{}",
            line.instrument_id
        );
    }
}

#[test]
fn a_composite_futures_line_sizes_a_dv01_hedge() {
    // The requirement's worked example: a $25,000/bp corporate book hedged in the
    // front 10-Year contract. The contract count must be the DV01 ratio computed off
    // the SAME derived DV01 the reference data publishes, at the SAME cash-curve
    // yield the feed prices the contract at.
    let cs = contracts();
    let front = cs
        .iter()
        .find(|c| c.spec.terms.symbol == "ZN")
        .expect("a 10-Year contract in the universe");

    let contract_dv01 = front.dv01_per_contract().expect("derives");
    assert!(
        (40.0..120.0).contains(&contract_dv01),
        "implausible 10-Year contract DV01 {contract_dv01}"
    );

    let portfolio_dv01: f64 = 25_000.0;
    let want = (portfolio_dv01 / contract_dv01).round() as i64;
    let got = front
        .spec
        .hedge_contracts(portfolio_dv01, front.reference_yield)
        .expect("sizes");
    assert_eq!(got, want);
    assert!(got > 0, "a long-duration book sells futures");

    let px = front.reference_price().expect("prices");
    let rendered = front
        .spec
        .format_price_32nds(front.spec.round_bid_to_tick(px));
    assert!(
        rendered.contains('\''),
        "a futures price renders in points and 32nds, got {rendered}"
    );
}

#[test]
fn a_product_symbol_rolls_but_a_named_month_never_moves() {
    let cs = contracts();
    for product in ["ZT", "ZF", "ZN", "TN", "ZB", "UB"] {
        let (c, how) = resolve_contract(&cs, product, as_of(settlement()))
            .unwrap_or_else(|| panic!("{product} did not resolve to a live contract"));
        assert_eq!(how, celnet_cme_sim::SymbolResolution::RolledToFrontMonth);
        let front =
            celnet_refdata::front_contract_id(product, as_of(settlement())).expect("front month");
        assert_eq!(c.instrument_id(), front);

        // The resolved contract is one the venue actually quotes and can trade.
        let (again, how) = resolve_contract(&cs, c.instrument_id(), as_of(settlement()))
            .expect("the rolled code resolves explicitly");
        assert_eq!(how, celnet_cme_sim::SymbolResolution::ExplicitContract);
        assert_eq!(again.instrument_id(), front);
        assert!(contract_lot_size(again).is_some_and(|l| l > 0.0));
    }
}

// ===========================================================================
// The ORDER path — a real FIX session over a real loopback socket.
// ===========================================================================

/// Send one `NewOrderSingle(D)` to a live `cme-sim` order acceptor and return the
/// `ExecutionReport(8)` it answers with. A genuine `Logon(A)` → order → report cycle
/// over TCP; nothing is stubbed.
async fn trade(
    addr: std::net::SocketAddr,
    symbol: &str,
    side: u8,
    qty: f64,
    ord_type: u8,
    price: f64,
    tif: Option<u8>,
) -> (u8, u8, f64, f64, String) {
    use celnet_fix::framing::{FrameCursor, FrameEncoder};
    use celnet_fix::messages;
    use celnet_fix::session::{InMemoryStore, Role, Session, SessionConfig};
    use celnet_fix::transport::{FrameReader, write_frame};

    const T: &[u8] = b"20260814-09:00:00.000";
    let stream = tokio::net::TcpStream::connect(addr).await.expect("dial");
    let (rh, mut wh) = tokio::io::split(stream);
    let mut reader = FrameReader::new(rh);
    let mut session = Session::new(
        SessionConfig {
            sender: b"CELNET".to_vec(),
            target: VENUE_ID.as_bytes().to_vec(),
            heart_bt_int: 30,
            role: Role::Initiator,
        },
        InMemoryStore::new(),
    );

    let logon = session.start_logon(T, false);
    write_frame(&mut wh, &logon).await.expect("logon");
    let mirror = reader
        .next_frame()
        .await
        .expect("io")
        .expect("logon mirror");
    session.on_inbound(&mirror, T).expect("logon accepted");

    let mut enc = FrameEncoder::new();
    let order = session.send_app(T, |h, e| {
        let p = messages::MarketOrderParams {
            cl_ord_id: b"IT-1",
            symbol: symbol.as_bytes(),
            quote_id: b"",
            security_type: b"",
            side,
            qty,
            price,
            ord_type,
            tif,
            transact_time: T,
        };
        messages::build_new_order_by_symbol(h, &p, e)
    });
    let _ = &mut enc;
    write_frame(&mut wh, &order).await.expect("order");

    let raw = reader
        .next_frame()
        .await
        .expect("io")
        .expect("execution report");
    let frame = FrameCursor::parse(&raw).expect("report parses");
    assert_eq!(
        celnet_fix::dictionary::validate(&frame),
        Ok(celnet_fix::dictionary::MsgType::ExecutionReport),
        "the venue answered with something that is not an ExecutionReport"
    );
    let byte = |t: u32| frame.get(t).and_then(|v| v.first().copied()).unwrap_or(0);
    let num = |t: u32| {
        frame
            .get(t)
            .and_then(celnet_fix::dialect_fx::parse_float)
            .unwrap_or_default()
    };
    let text = frame
        .get(58)
        .map(|v| String::from_utf8_lossy(v).into_owned())
        .unwrap_or_default();
    assert_eq!(frame.get(11), Some(&b"IT-1"[..]), "ClOrdID not echoed");
    (byte(150), byte(39), num(32), num(31), text)
}

/// Stand up a live `cme-sim` order acceptor over loopback with its markets already
/// published, and return `(addr, contracts, lines)`.
async fn live_venue() -> (
    std::net::SocketAddr,
    Vec<celnet_cme_sim::FuturesContract>,
    Vec<QuotedLine>,
) {
    let cs = contracts();
    let ls = lines(&cs);
    let feed = build_venue_feed(&ls, SEED);
    let markets = LiveMarkets::default();
    let ov = build_order_venue(LiveMarkets::clone(&markets), SEED);
    // Publish at the CURRENT wall clock, since the live acceptor ages its
    // tradeability against wall time.
    publish_markets(&feed, &ov, &ls, &cs, celnet_lp_sim::orders::wall_nanos());
    let (addr, _handle) = celnet_lp_sim::orders::run_order_acceptor(ov, "127.0.0.1:0")
        .await
        .expect("bind loopback acceptor");
    (addr, cs, ls)
}

#[tokio::test]
async fn the_venue_fills_a_whole_lot_order_over_a_real_fix_session() {
    use celnet_fix::messages::{ord_type, time_in_force};
    let (addr, cs, _ls) = live_venue().await;
    let front = cs
        .iter()
        .find(|c| c.spec.terms.symbol == "ZN")
        .expect("10-Year contract");
    let lot = contract_lot_size(front).expect("lot size");

    // Two contracts, fill-or-kill, market — comfortably inside the venue's touch.
    let (exec_type, ord_status, last_qty, last_px, text) = trade(
        addr,
        front.instrument_id(),
        b'1',
        2.0 * lot,
        ord_type::MARKET,
        0.0,
        Some(time_in_force::FILL_OR_KILL),
    )
    .await;
    assert_eq!(exec_type, celnet_fix::messages::EXEC_FILLED, "text: {text}");
    assert_eq!(ord_status, celnet_fix::messages::EXEC_FILLED);
    assert!(
        (last_qty - 2.0 * lot).abs() < 1e-6,
        "filled {last_qty}, wanted {}",
        2.0 * lot
    );
    assert!(last_px > 0.0, "no fill price");
    assert!(text.is_empty(), "a clean fill carried text: {text}");
}

#[tokio::test]
async fn a_fractional_contract_order_is_refused_by_name() {
    use celnet_fix::messages::{EXEC_REJECTED, ord_type, time_in_force};
    let (addr, cs, _ls) = live_venue().await;
    let front = cs
        .iter()
        .find(|c| c.spec.terms.symbol == "ZN")
        .expect("10-Year contract");
    let lot = contract_lot_size(front).expect("lot size");

    let (exec_type, ord_status, last_qty, _px, text) = trade(
        addr,
        front.instrument_id(),
        b'1',
        1.5 * lot,
        ord_type::MARKET,
        0.0,
        Some(time_in_force::FILL_OR_KILL),
    )
    .await;
    assert_eq!(exec_type, EXEC_REJECTED);
    assert_eq!(ord_status, EXEC_REJECTED);
    assert_eq!(last_qty, 0.0, "a refused order must not trade");
    assert!(
        text.starts_with("NOT_A_WHOLE_LOT"),
        "a fractional listed order must be refused by name, got {text:?}"
    );
}

#[tokio::test]
async fn an_oversized_fok_is_killed_but_an_ioc_partially_fills() {
    use celnet_fix::messages::{
        EXEC_CANCELED, EXEC_FILLED, ORD_STATUS_CANCELED, ORD_STATUS_PARTIALLY_FILLED, ord_type,
        time_in_force,
    };
    let (addr, cs, _ls) = live_venue().await;
    let front = cs
        .iter()
        .find(|c| c.spec.terms.symbol == "ZN")
        .expect("10-Year contract");
    let lot = contract_lot_size(front).expect("lot size");
    // Far beyond anything the venue shows, including every depth level.
    let huge = 100_000.0 * lot;

    // FOK: all or nothing. Killed in FULL — never a partial.
    let (exec_type, ord_status, last_qty, _px, text) = trade(
        addr,
        front.instrument_id(),
        b'1',
        huge,
        ord_type::MARKET,
        0.0,
        Some(time_in_force::FILL_OR_KILL),
    )
    .await;
    assert_eq!(exec_type, EXEC_CANCELED, "text: {text}");
    assert_eq!(ord_status, ORD_STATUS_CANCELED);
    assert_eq!(last_qty, 0.0, "FOK must never partially fill");
    assert!(text.starts_with("FOK_UNFILLABLE"), "got {text:?}");

    // IOC: take what is there, cancel the rest — a PARTIAL fill is the normal answer.
    let (exec_type, ord_status, last_qty, last_px, text) = trade(
        addr,
        front.instrument_id(),
        b'1',
        huge,
        ord_type::MARKET,
        0.0,
        Some(time_in_force::IMMEDIATE_OR_CANCEL),
    )
    .await;
    assert_eq!(exec_type, EXEC_FILLED, "text: {text}");
    assert_eq!(ord_status, ORD_STATUS_PARTIALLY_FILLED);
    assert!(last_qty > 0.0 && last_qty < huge, "IOC filled {last_qty}");
    assert!(
        (last_qty / lot - (last_qty / lot).round()).abs() < 1e-9,
        "IOC partial {last_qty} is not whole contracts"
    );
    assert!(last_px > 0.0);
    assert!(text.starts_with("IOC_"), "got {text:?}");
}

#[tokio::test]
async fn a_limit_away_from_the_market_and_a_resting_tif_are_both_declined() {
    use celnet_fix::messages::{
        EXEC_CANCELED, EXEC_REJECTED, ORD_STATUS_CANCELED, ord_type, time_in_force,
    };
    let (addr, cs, _ls) = live_venue().await;
    let front = cs
        .iter()
        .find(|c| c.spec.terms.symbol == "ZN")
        .expect("10-Year contract");
    let lot = contract_lot_size(front).expect("lot size");

    // A buy limit far below the market is an honest miss — CANCELLED, not rejected:
    // the order was fine, the market was simply away from it.
    let (exec_type, ord_status, last_qty, _px, text) = trade(
        addr,
        front.instrument_id(),
        b'1',
        lot,
        ord_type::LIMIT,
        1.0,
        Some(time_in_force::IMMEDIATE_OR_CANCEL),
    )
    .await;
    assert_eq!(exec_type, EXEC_CANCELED);
    assert_eq!(ord_status, ORD_STATUS_CANCELED);
    assert_eq!(last_qty, 0.0);
    assert!(text.starts_with("NOT_MARKETABLE"), "got {text:?}");

    // A resting TIF is REJECTED with the reason — never silently downgraded to an
    // IOC, which would fabricate a cancel the taker did not ask for.
    let (exec_type, ord_status, last_qty, _px, text) = trade(
        addr,
        front.instrument_id(),
        b'1',
        lot,
        ord_type::MARKET,
        0.0,
        Some(time_in_force::DAY),
    )
    .await;
    assert_eq!(exec_type, EXEC_REJECTED);
    assert_eq!(ord_status, EXEC_REJECTED);
    assert_eq!(last_qty, 0.0);
    assert!(text.starts_with("RESTING_TIF_UNSUPPORTED"), "got {text:?}");
}

#[tokio::test]
async fn an_instrument_the_venue_does_not_quote_is_declined_by_name() {
    use celnet_fix::messages::{EXEC_REJECTED, ord_type, time_in_force};
    let (addr, _cs, _ls) = live_venue().await;
    // A cash CUSIP: tradeable somewhere in the estate, but not on this venue.
    let (exec_type, _ord_status, last_qty, _px, text) = trade(
        addr,
        "912810TZ1",
        b'1',
        100_000.0,
        ord_type::MARKET,
        0.0,
        Some(time_in_force::FILL_OR_KILL),
    )
    .await;
    assert_eq!(exec_type, EXEC_REJECTED);
    assert_eq!(last_qty, 0.0);
    assert!(text.starts_with("INSTRUMENT_NOT_QUOTED"), "got {text:?}");
}

/// The in-process equivalent of the socket tests, asserted against the venue's own
/// published book so the economics (not just the wire) are pinned: a fill never
/// happens at a price the venue did not show, and the fill price is on the grid.
#[test]
fn a_fill_never_prints_away_from_the_published_market() {
    let cs = contracts();
    let ls = lines(&cs);
    let feed = build_venue_feed(&ls, SEED);
    let markets = LiveMarkets::default();
    let ov = build_order_venue(LiveMarkets::clone(&markets), SEED);
    publish_markets(&feed, &ov, &ls, &cs, NOW);

    let specs = specs();
    for line in &ls {
        let m = *markets
            .read()
            .expect("markets")
            .get(&line.instrument_id)
            .expect("published");
        let lot = m.lot_size.expect("lot");
        let tick = specs[&line.instrument_id].terms.tick_size_points;

        for side in [Side::Buy, Side::Sell] {
            let order = OrderRequest {
                cl_ord_id: "X".into(),
                instrument_id: line.instrument_id.clone(),
                side,
                quantity: lot,
                ord_type: OrderType::Market,
                tif: TimeInForce::FillOrKill,
            };
            let e = ov.handle(&order, NOW);
            let Execution::Filled(f) = &e else {
                panic!("{}: one contract did not fill: {e:?}", line.instrument_id);
            };
            let touch = m.touch(side).0;
            assert!(
                (f.average_price - touch).abs() < 1e-12,
                "{}: filled at {} but the venue showed {touch}",
                line.instrument_id,
                f.average_price
            );
            let ticks = f.average_price / tick;
            assert!(
                (ticks - ticks.round()).abs() < 1e-6,
                "{}: fill price {} is off the tick grid",
                line.instrument_id,
                f.average_price
            );
        }

        // A zero-quantity order is refused, not treated as a no-op.
        let bad = OrderRequest {
            cl_ord_id: "X".into(),
            instrument_id: line.instrument_id.clone(),
            side: Side::Buy,
            quantity: 0.0,
            ord_type: OrderType::Market,
            tif: TimeInForce::FillOrKill,
        };
        assert_eq!(
            ov.handle(&bad, NOW),
            Execution::Rejected(RejectReason::InvalidQuantity)
        );
    }
}
