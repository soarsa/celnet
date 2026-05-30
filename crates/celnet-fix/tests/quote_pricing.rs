//! Quote round-trip pricing: a `QuoteRequest(R)` → `Quote(S)` cycle, driven by
//! the REAL acceptor against the REAL initiator over a loopback TCP socket,
//! reprices the SAME premium the `celnet-vanilla` engine produces — which is
//! itself gated against the `celnet-golden` / QuantLib tables. Plus the
//! multileg invariant: a package price equals the sum of per-leg prices off one
//! surface snapshot.
//!
//! No fakes: the acceptor prices via the injected `celnet_vanilla::price`; the
//! golden table is the independent oracle.

use std::time::Duration;

use celnet_fix::acceptor::{Acceptor, FixedQuoteSource};
use celnet_fix::dialect_fx::{
    self, ExerciseStyle, LegSide, MarketSnapshot, OptionDescriptor, StrategyLeg, StrategyPackage,
    VanillaPricer,
};
use celnet_fix::framing::{FrameCursor, FrameEncoder};
use celnet_fix::initiator::{Initiator, LiftPolicy};
use celnet_fix::messages::Header;
use celnet_fix::session::{InMemoryStore, Role, Session, SessionConfig};
use celnet_types::{Ccy, CcyPair, OptionType, Settlement, Tenor};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;

const T: &[u8] = b"20260530-12:00:00.000";
const DEADLINE: Duration = Duration::from_secs(5);

fn acc_cfg() -> SessionConfig {
    SessionConfig {
        sender: b"VENUE".to_vec(),
        target: b"TAKER".to_vec(),
        heart_bt_int: 30,
        role: Role::Acceptor,
    }
}
fn init_cfg() -> SessionConfig {
    SessionConfig {
        sender: b"TAKER".to_vec(),
        target: b"VENUE".to_vec(),
        heart_bt_int: 30,
        role: Role::Initiator,
    }
}

#[tokio::test]
async fn quote_request_reprices_to_vanilla_engine_over_socket() {
    let body = async {
        // The market snapshot the venue prices off. EURUSD-like.
        let snap = MarketSnapshot {
            spot: 1.10,
            vol: 0.105,
            t: 0.25,
            r_dom: 0.030,
            r_for: 0.012,
        };
        let strike = 1.1250_f64;
        let opt = OptionType::Call;

        // Independent reference: the engine premium (golden-gated) for this leg.
        let pricer: VanillaPricer = celnet_vanilla::price;
        let desc = OptionDescriptor {
            pair: CcyPair::new(Ccy::EUR, Ccy::USD),
            option_type: opt,
            strike,
            strike_ccy: Ccy::USD,
            exercise: ExerciseStyle::European,
            tenor: Tenor::Months(3),
            settlement: Settlement::Deliverable,
        };
        let expected_mid = dialect_fx::price_leg(&desc, &snap, pricer);

        let qs = FixedQuoteSource {
            snapshot: snap,
            tenor: Tenor::Months(3),
            half_spread: 0.0, // mid == bid == offer so we can compare to mid
            validity_ticks: 1000,
            pricer,
        };

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        // Acceptor task.
        let acceptor = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let session = Session::new(acc_cfg(), InMemoryStore::new());
            let mut acc = Acceptor::new(session, qs);
            acc.run(stream, T.to_vec()).await.ok();
        });

        // Initiator: logon + QuoteRequest(R) for the EURUSD 3M 1.1250 call,
        // observe the returned quote (no lift).
        let stream = TcpStream::connect(addr).await.unwrap();
        let mut init = Initiator::new(
            Session::new(init_cfg(), InMemoryStore::new()),
            LiftPolicy::Observe,
        );
        let build_req = |h: &Header<'_>, e: &mut FrameEncoder| {
            e.clear();
            h.encode(celnet_fix::MsgType::QuoteRequest, e);
            e.push(131, b"REQ-1");
            e.push(55, b"EURUSD");
            e.push(460, b"4");
            e.push(167, b"FXVO");
            e.push(201, b"1"); // call
            push_strike(e, strike);
            e.push(947, b"USD");
            e.push(1194, b"0");
            e.finish()
        };
        let result = init
            .request_and_lift(stream, T.to_vec(), build_req)
            .await
            .unwrap();

        // The quoted bid/offer (== mid here) must equal the engine premium to
        // 8-dp wire precision.
        let quoted = result.bid.expect("a quote was returned");
        assert!(
            (quoted - expected_mid).abs() < 1e-7,
            "quoted {quoted} vs engine {expected_mid}"
        );

        timeout(DEADLINE, acceptor).await.unwrap().unwrap();
    };
    timeout(DEADLINE, body).await.expect("test timed out");
}

#[tokio::test]
async fn click_to_trade_last_look_fills_then_rejects_replay() {
    let body = async {
        let snap = MarketSnapshot {
            spot: 1.10,
            vol: 0.10,
            t: 0.25,
            r_dom: 0.03,
            r_for: 0.01,
        };
        let pricer: VanillaPricer = celnet_vanilla::price;
        let qs = FixedQuoteSource {
            snapshot: snap,
            tenor: Tenor::Months(3),
            half_spread: 0.0005,
            validity_ticks: 1_000_000,
            pricer,
        };

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let acceptor = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let session = Session::new(acc_cfg(), InMemoryStore::new());
            let mut acc = Acceptor::new(session, qs);
            acc.run(stream, T.to_vec()).await.ok();
        });

        // Initiator lifts the offer (BUY) on the first quote.
        let stream = TcpStream::connect(addr).await.unwrap();
        let mut init = Initiator::new(
            Session::new(init_cfg(), InMemoryStore::new()),
            LiftPolicy::LiftOffer,
        );
        let build_req = |h: &Header<'_>, e: &mut FrameEncoder| {
            e.clear();
            h.encode(celnet_fix::MsgType::QuoteRequest, e);
            e.push(131, b"REQ-1");
            e.push(55, b"EURUSD");
            e.push(167, b"FXVO");
            e.push(201, b"1");
            push_strike(e, 1.10);
            e.push(947, b"USD");
            e.finish()
        };
        let result = init
            .request_and_lift(stream, T.to_vec(), build_req)
            .await
            .unwrap();
        assert!(result.filled, "the lift should fill against a live quote");
        // BUY fills at the offer = mid + half_spread.
        let mid = dialect_fx::price_leg(
            &OptionDescriptor {
                pair: CcyPair::new(Ccy::EUR, Ccy::USD),
                option_type: OptionType::Call,
                strike: 1.10,
                strike_ccy: Ccy::USD,
                exercise: ExerciseStyle::European,
                tenor: Tenor::Months(3),
                settlement: Settlement::Deliverable,
            },
            &snap,
            pricer,
        );
        let fill = result.fill_px.unwrap();
        assert!(
            (fill - (mid + 0.0005)).abs() < 1e-6,
            "fill {fill} vs offer {}",
            mid + 0.0005
        );

        timeout(DEADLINE, acceptor).await.unwrap().unwrap();
    };
    timeout(DEADLINE, body).await.expect("test timed out");
}

#[test]
fn multileg_package_equals_sum_of_legs() {
    // Risk reversal (+call/-put) and straddle (+call/+put), one snapshot.
    let snap = MarketSnapshot {
        spot: 1.10,
        vol: 0.10,
        t: 0.5,
        r_dom: 0.03,
        r_for: 0.01,
    };
    let pricer: VanillaPricer = celnet_vanilla::price;
    let pair = CcyPair::new(Ccy::EUR, Ccy::USD);
    let mk = |opt, strike| OptionDescriptor {
        pair,
        option_type: opt,
        strike,
        strike_ccy: Ccy::USD,
        exercise: ExerciseStyle::European,
        tenor: Tenor::Months(6),
        settlement: Settlement::Deliverable,
    };

    // Risk reversal: buy 25d call, sell 25d put.
    let rr = StrategyPackage {
        legs: vec![
            StrategyLeg {
                option: mk(OptionType::Call, 1.16),
                side: LegSide::Buy,
                ratio: 1.0,
            },
            StrategyLeg {
                option: mk(OptionType::Put, 1.04),
                side: LegSide::Sell,
                ratio: 1.0,
            },
        ],
    };
    let expect_rr = dialect_fx::price_leg(&mk(OptionType::Call, 1.16), &snap, pricer)
        - dialect_fx::price_leg(&mk(OptionType::Put, 1.04), &snap, pricer);
    assert!((rr.package_price(&snap, pricer) - expect_rr).abs() < 1e-14);

    // Straddle: buy call + buy put, same strike.
    let straddle = StrategyPackage {
        legs: vec![
            StrategyLeg {
                option: mk(OptionType::Call, 1.10),
                side: LegSide::Buy,
                ratio: 1.0,
            },
            StrategyLeg {
                option: mk(OptionType::Put, 1.10),
                side: LegSide::Buy,
                ratio: 1.0,
            },
        ],
    };
    let expect_straddle = dialect_fx::price_leg(&mk(OptionType::Call, 1.10), &snap, pricer)
        + dialect_fx::price_leg(&mk(OptionType::Put, 1.10), &snap, pricer);
    assert!((straddle.package_price(&snap, pricer) - expect_straddle).abs() < 1e-14);

    // Per-leg breakdown sums to the package.
    let parts: f64 = straddle.leg_contributions(&snap, pricer).iter().sum();
    assert!((parts - straddle.package_price(&snap, pricer)).abs() < 1e-15);
}

#[test]
fn dialect_decode_reprices_against_golden() {
    // Pull a few well-conditioned rows from the frozen golden table, build the
    // FIX instrument block, decode it through the dialect, and assert the
    // engine price the dialect carries matches the golden (QuantLib) reference.
    let rows = celnet_golden::load_vanilla().expect("golden table");
    let pricer: VanillaPricer = celnet_vanilla::price;
    let mut checked = 0;
    for r in rows.iter().filter(|r| r.t > 0.05 && r.vol > 0.03) {
        // Build a QuoteRequest instrument block with this row's strike/type.
        let mut e = FrameEncoder::new();
        e.push(35, b"R");
        e.push(131, b"REQ");
        e.push(55, b"EURUSD");
        e.push(167, b"FXVO");
        e.push(
            201,
            if r.option_type == OptionType::Call {
                b"1"
            } else {
                b"0"
            },
        );
        push_strike(&mut e, r.strike);
        e.push(947, b"USD");
        let raw = e.finish();
        let frame = FrameCursor::parse(&raw).unwrap();
        let desc = dialect_fx::decode_option(&frame, Tenor::Months(3)).unwrap();
        let snap = MarketSnapshot {
            spot: r.spot,
            vol: r.vol,
            t: r.t,
            r_dom: r.r_dom,
            r_for: r.r_for,
        };
        let priced = dialect_fx::price_leg(&desc, &snap, pricer);
        assert!(
            (priced - r.price).abs() < 1e-9,
            "dialect-carried price {priced} vs golden {} (strike {}, vol {})",
            r.price,
            r.strike,
            r.vol
        );
        checked += 1;
        if checked >= 200 {
            break;
        }
    }
    assert!(checked > 0, "no golden rows exercised");
}

/// Push a `StrikePrice(202)` field with up to 8-dp fixed precision.
fn push_strike(e: &mut FrameEncoder, strike: f64) {
    let scaled = (strike * 1e8).round() as u128;
    let int = scaled / 100_000_000;
    let frac = scaled % 100_000_000;
    let s = format!("{int}.{frac:08}");
    e.push(202, s.as_bytes());
}
