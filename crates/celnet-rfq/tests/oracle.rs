//! Multi-dealer RFQ aggregation oracle (oracle class 4 + structural — the
//! injected ladder is the independently-computed ground truth, so it CAN
//! disagree with the engine; it is NOT a second copy of the ranking algebra).
//!
//! The panel boots **≥ 3 synthetic LP responders**: at least one **real**
//! [`FixLpAdapter`](celnet_rfq::FixLpAdapter) over a loopback `celnet-fix`
//! initiator/acceptor pair, plus deterministic in-process [`LadderSource`]s with
//! KNOWN injected bid/offer ladders. The gates:
//!
//! 1. **best-bid / best-offer** == the injected extremum (computed here from the
//!    ladders, not read from the engine);
//! 2. **tie-break determinism** — two equal-best LPs ⇒ the documented preference
//!    winner (earlier epoch, then smaller `lp_id`);
//! 3. **timeout** — one LP sleeps past the deadline ⇒ dropped, `lp_count`
//!    excludes it, next-best promoted;
//! 4. **last-look** — the winner's `valid_until_nanos` is in the past ⇒ rejected,
//!    next-best promoted;
//! 5. **`lp_count` / `lp_won` consistency invariant** — gated here over fixed
//!    cases and (in `proptest.rs`) over a random-ladder property sweep.
//!
//! Every test body is hard wall-clock bounded so a regression fails fast.
//!
//! # Honest boundary (verbatim)
//!
//! Live LP-panel connectivity (real bank sessions over WAN FIX) and the
//! regulated-venue / MAS-RMO status are ENV — designed, seamed and ADR'd
//! in-repo, validated at deploy, NEVER claimed in-repo. In-repo this suite proves
//! the aggregation / ranking / tie-break / last-look ALGORITHM plus the FIX
//! framing / dialect round-trip over a **loopback** socket only.

mod harness;

use std::time::Duration;

use celnet_fix::dialect_fx::MarketSnapshot;
use celnet_rfq::{MultiDealerEngine, RfqRequest};
use celnet_types::{Ccy, CcyPair, OptionType, Tenor};

use harness::{LadderSource, spawn_fix_lp};

const DEADLINE: Duration = Duration::from_secs(5);
const PANEL_DEADLINE: Duration = Duration::from_millis(500);

fn eurusd() -> CcyPair {
    CcyPair::new(Ccy::EUR, Ccy::USD)
}

fn sample_request() -> RfqRequest {
    RfqRequest::new(
        "REQ-1",
        eurusd(),
        OptionType::Call,
        1.1250,
        Tenor::Months(3),
    )
}

fn sample_snapshot() -> MarketSnapshot {
    MarketSnapshot {
        spot: 1.10,
        vol: 0.105,
        t: 0.25,
        r_dom: 0.030,
        r_for: 0.012,
    }
}

/// Gate 1 — best-bid / best-offer == the injected extremum, with a real FIX LP
/// in the panel. The oracle computes the expected winners from the injected
/// ladders + the FIX leg's independently-priced two-way, NOT from the engine.
#[tokio::test]
async fn best_bid_offer_match_injected_extremum_with_real_fix_lp() {
    tokio::time::timeout(DEADLINE, async {
        let req = sample_request();

        // A real FIX LP over a loopback socket: half_spread chosen so it is the
        // *widest* market (neither best bid nor best offer) — proving the engine
        // genuinely ranks across the FIX leg, not just the in-process sources.
        let (fix_lp, fix_tw) = spawn_fix_lp(
            "FIX-WIDE",
            sample_snapshot(),
            Tenor::Months(3),
            0.0020,
            100,
            10_000,
            &req,
        )
        .await;

        // Two in-process ladders with KNOWN, distinct two-ways straddling the FIX
        // leg: LADDER-A has the highest bid, LADDER-B the lowest offer.
        let a = LadderSource::firm("LADDER-A", fix_tw.bid + 0.0009, fix_tw.offer + 0.0005, 200);
        let b = LadderSource::firm("LADDER-B", fix_tw.bid - 0.0003, fix_tw.offer - 0.0008, 300);

        let engine = MultiDealerEngine::new(vec![
            Box::new(fix_lp),
            Box::new(a.clone()),
            Box::new(b.clone()),
        ]);

        let panel = engine.request(&req, PANEL_DEADLINE, 0).await.unwrap();

        assert_eq!(panel.lp_count, 3, "all three responded");
        // Independently-computed ground truth.
        let exp_best_bid = [a.bid, b.bid, fix_tw.bid]
            .into_iter()
            .fold(f64::MIN, f64::max);
        let exp_best_offer = [a.offer, b.offer, fix_tw.offer]
            .into_iter()
            .fold(f64::MAX, f64::min);
        assert!((panel.best_bid.unwrap() - exp_best_bid).abs() < 1e-12);
        assert!((panel.best_offer.unwrap() - exp_best_offer).abs() < 1e-12);
        assert_eq!(panel.lp_won_bid.as_deref(), Some("LADDER-A"));
        assert_eq!(panel.lp_won_offer.as_deref(), Some("LADDER-B"));

        // The real FIX LP's two-way is present and equals the independently
        // priced expectation to wire precision (the dialect repriced the same
        // premium the engine/golden produces).
        let fix_row = panel
            .rows
            .iter()
            .find(|r| r.lp_id == "FIX-WIDE")
            .expect("FIX LP responded over the loopback session");
        assert!((fix_row.price.bid - fix_tw.bid).abs() < 1e-7);
        assert!((fix_row.price.offer - fix_tw.offer).abs() < 1e-7);

        // Consistency invariant.
        assert!(
            panel
                .rows
                .iter()
                .any(|r| Some(&r.lp_id) == panel.lp_won_bid.as_ref())
        );
        assert!(
            panel
                .rows
                .iter()
                .any(|r| Some(&r.lp_id) == panel.lp_won_offer.as_ref())
        );
    })
    .await
    .expect("test timed out");
}

/// Gate 1b — the real FIX LP can itself WIN. With a tight FIX spread and wider
/// in-process ladders, the loopback-priced LP holds both best bid and best
/// offer, proving the FIX two-way participates fully in ranking.
#[tokio::test]
async fn real_fix_lp_can_win_both_sides() {
    tokio::time::timeout(DEADLINE, async {
        let req = sample_request();
        let (fix_lp, fix_tw) = spawn_fix_lp(
            "FIX-TIGHT",
            sample_snapshot(),
            Tenor::Months(3),
            0.0001,
            100,
            10_000,
            &req,
        )
        .await;

        // Wider in-process markets: lower bid, higher offer than the FIX leg.
        let a = LadderSource::firm("LADDER-A", fix_tw.bid - 0.0010, fix_tw.offer + 0.0010, 200);
        let b = LadderSource::firm("LADDER-B", fix_tw.bid - 0.0020, fix_tw.offer + 0.0020, 300);

        let engine = MultiDealerEngine::new(vec![Box::new(fix_lp), Box::new(a), Box::new(b)]);
        let panel = engine.request(&req, PANEL_DEADLINE, 0).await.unwrap();

        assert_eq!(panel.lp_count, 3);
        assert_eq!(panel.lp_won_bid.as_deref(), Some("FIX-TIGHT"));
        assert_eq!(panel.lp_won_offer.as_deref(), Some("FIX-TIGHT"));
        assert!((panel.best_bid.unwrap() - fix_tw.bid).abs() < 1e-7);
        assert!((panel.best_offer.unwrap() - fix_tw.offer).abs() < 1e-7);
    })
    .await
    .expect("test timed out");
}

/// Gate 1c — **negative control: the FIX leg is NOT vacuous.** A `FixLpAdapter`
/// pointed at a CLOSED loopback port cannot complete its session, so it must be
/// dropped (no quote) — proving the two-way the FIX LP contributes in the
/// passing tests genuinely came over a live socket, not from a synthetic stub.
/// The native dealer keeps a market.
#[tokio::test]
async fn fix_lp_with_no_acceptor_is_dropped() {
    use celnet_rfq::{FixLpAdapter, FixLpConfig, InternalPricerSource};
    tokio::time::timeout(DEADLINE, async {
        let req = sample_request();

        // Bind then immediately drop a listener to obtain a port that is
        // (almost certainly) closed — the connect must fail, dropping the LP.
        let dead_addr = {
            let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            l.local_addr().unwrap()
        };
        let dead_fix = FixLpAdapter::new(FixLpConfig {
            lp_id: "FIX-DEAD".to_owned(),
            dial_addr: format!("{dead_addr}"),
            sender_comp_id: b"TAKER".to_vec(),
            target_comp_id: b"VENUE".to_vec(),
            sending_time: b"20260608-12:00:00.000".to_vec(),
            epoch_nanos: 100,
            valid_for_nanos: 10_000,
        });

        let native = InternalPricerSource::new("CELNET-NATIVE", 0.0085, 0.0005, 200, 10_000);
        let other = LadderSource::firm("OTHER", 0.0082, 0.0095, 300);

        let engine =
            MultiDealerEngine::new(vec![Box::new(dead_fix), Box::new(native), Box::new(other)]);
        let panel = engine
            .request(&req, Duration::from_millis(300), 0)
            .await
            .unwrap();

        // The dead FIX LP is absent; only the two live in-process dealers count.
        assert_eq!(panel.lp_count, 2);
        assert!(panel.rows.iter().all(|r| r.lp_id != "FIX-DEAD"));
        assert!(panel.best_bid.is_some());
        assert!(panel.best_offer.is_some());
    })
    .await
    .expect("test timed out");
}

/// Gate 2 — deterministic tie-break: two LPs with the EQUAL best bid. The
/// documented preference is earlier `epoch_nanos`, then lexicographically
/// smallest `lp_id`. Here both quote the same best bid; the earlier-epoch one
/// must win regardless of fan-out order.
#[tokio::test]
async fn tie_break_prefers_earlier_epoch_then_smaller_lp_id() {
    tokio::time::timeout(DEADLINE, async {
        let req = sample_request();

        // EARLY and LATE have the identical best bid 0.0090; ZED also ties but is
        // lexicographically last and later. The native (non-best) source ensures
        // the panel has a third dealer.
        let early = LadderSource::firm("EARLY", 0.0090, 0.0100, 100);
        let late = LadderSource::firm("LATE", 0.0090, 0.0100, 500);
        let zed = LadderSource::firm("ZED", 0.0090, 0.0100, 500);
        // A genuinely worse bid so it cannot win — but a great offer so the
        // offer side has its own unambiguous winner.
        let other = LadderSource::firm("OTHER", 0.0050, 0.0095, 200);

        let engine = MultiDealerEngine::new(vec![
            Box::new(zed),
            Box::new(late),
            Box::new(early),
            Box::new(other),
        ]);
        let panel = engine.request(&req, PANEL_DEADLINE, 0).await.unwrap();

        assert_eq!(panel.lp_count, 4);
        // Earlier epoch (100) beats LATE/ZED (500) on the equal best bid.
        assert_eq!(panel.lp_won_bid.as_deref(), Some("EARLY"));
        assert!((panel.best_bid.unwrap() - 0.0090).abs() < 1e-12);
        // Offer winner is the unambiguous min offer (OTHER @ 0.0095).
        assert_eq!(panel.lp_won_offer.as_deref(), Some("OTHER"));
    })
    .await
    .expect("test timed out");
}

/// Gate 2b — tie-break falls through to the lexicographically smallest `lp_id`
/// when price AND epoch are equal.
#[tokio::test]
async fn tie_break_falls_through_to_smallest_lp_id() {
    tokio::time::timeout(DEADLINE, async {
        let req = sample_request();
        // Identical best bid AND identical epoch ⇒ smallest lp_id wins ("AAA").
        let aaa = LadderSource::firm("AAA", 0.0090, 0.0100, 100);
        let bbb = LadderSource::firm("BBB", 0.0090, 0.0100, 100);
        let ccc = LadderSource::firm("CCC", 0.0090, 0.0100, 100);

        let engine = MultiDealerEngine::new(vec![Box::new(ccc), Box::new(bbb), Box::new(aaa)]);
        let panel = engine.request(&req, PANEL_DEADLINE, 0).await.unwrap();

        assert_eq!(panel.lp_count, 3);
        assert_eq!(panel.lp_won_bid.as_deref(), Some("AAA"));
        assert_eq!(panel.lp_won_offer.as_deref(), Some("AAA"));
    })
    .await
    .expect("test timed out");
}

/// Gate 3 — timeout: one LP responds AFTER the panel deadline. It must be
/// dropped (not errored), excluded from `lp_count`, and the next-best promoted.
#[tokio::test]
async fn slow_lp_is_dropped_and_next_best_promoted() {
    tokio::time::timeout(DEADLINE, async {
        let req = sample_request();

        // SLOW would have the best bid (0.0099) but sleeps past the 200ms panel
        // deadline ⇒ it must be dropped and NEXT (0.0095) promoted.
        let slow =
            LadderSource::firm("SLOW", 0.0099, 0.0098, 100).with_delay(Duration::from_secs(2));
        let next = LadderSource::firm("NEXT", 0.0095, 0.0099, 200);
        let third = LadderSource::firm("THIRD", 0.0080, 0.0101, 300);

        let engine = MultiDealerEngine::new(vec![Box::new(slow), Box::new(next), Box::new(third)]);
        let panel = engine
            .request(&req, Duration::from_millis(200), 0)
            .await
            .unwrap();

        // SLOW dropped: only two responders.
        assert_eq!(panel.lp_count, 2);
        assert!(panel.rows.iter().all(|r| r.lp_id != "SLOW"));
        // NEXT promoted to best bid (SLOW's 0.0099 is gone).
        assert_eq!(panel.lp_won_bid.as_deref(), Some("NEXT"));
        assert!((panel.best_bid.unwrap() - 0.0095).abs() < 1e-12);
        // Offer winner is NEXT (0.0099 < 0.0101).
        assert_eq!(panel.lp_won_offer.as_deref(), Some("NEXT"));
    })
    .await
    .expect("test timed out");
}

/// Gate 4 — last-look: the would-be winner's `valid_until_nanos` is already in
/// the past at ranking time ⇒ rejected, next-best promoted. The stale LP still
/// counts as a responder (it answered in time), but it cannot WIN.
#[tokio::test]
async fn stale_winner_rejected_by_last_look_and_next_best_promoted() {
    tokio::time::timeout(DEADLINE, async {
        let req = sample_request();
        let now: u64 = 1_000;

        // STALE has the best bid 0.0099 but expired at t=500 < now=1000 ⇒
        // last-look rejects it from winning. FRESH (0.0095, valid far future)
        // is promoted.
        let stale = LadderSource::firm("STALE", 0.0099, 0.0090, 100).valid_until(500);
        let fresh = LadderSource::firm("FRESH", 0.0095, 0.0099, 200).valid_until(u64::MAX);
        let third = LadderSource::firm("THIRD", 0.0080, 0.0101, 300).valid_until(u64::MAX);

        let engine =
            MultiDealerEngine::new(vec![Box::new(stale), Box::new(fresh), Box::new(third)]);
        let panel = engine.request(&req, PANEL_DEADLINE, now).await.unwrap();

        // STALE is a RESPONDER (answered) — lp_count includes it (3 rows) — but
        // it cannot WIN (last-look).
        assert_eq!(panel.lp_count, 3);
        assert!(panel.rows.iter().any(|r| r.lp_id == "STALE"));
        assert_eq!(panel.lp_won_bid.as_deref(), Some("FRESH"));
        assert!((panel.best_bid.unwrap() - 0.0095).abs() < 1e-12);
        // STALE's offer 0.0090 would be best, but it is last-look-rejected too ⇒
        // FRESH's 0.0099 wins the offer over THIRD's 0.0101.
        assert_eq!(panel.lp_won_offer.as_deref(), Some("FRESH"));
    })
    .await
    .expect("test timed out");
}

/// Gate 4b — when EVERY responder is last-look-stale there is no liftable
/// winner: `best_bid` / `best_offer` are `None`, the winners `None`, yet the
/// stale responders still count (the consistency invariant holds vacuously).
#[tokio::test]
async fn all_stale_yields_no_winner_but_counts_responders() {
    tokio::time::timeout(DEADLINE, async {
        let req = sample_request();
        let now: u64 = 1_000;
        let s1 = LadderSource::firm("S1", 0.0099, 0.0090, 100).valid_until(100);
        let s2 = LadderSource::firm("S2", 0.0095, 0.0092, 200).valid_until(200);
        let s3 = LadderSource::firm("S3", 0.0093, 0.0094, 300).valid_until(300);

        let engine = MultiDealerEngine::new(vec![Box::new(s1), Box::new(s2), Box::new(s3)]);
        let panel = engine.request(&req, PANEL_DEADLINE, now).await.unwrap();

        assert_eq!(panel.lp_count, 3);
        assert_eq!(panel.best_bid, None);
        assert_eq!(panel.best_offer, None);
        assert_eq!(panel.lp_won_bid, None);
        assert_eq!(panel.lp_won_offer, None);
    })
    .await
    .expect("test timed out");
}

/// Gate 2c — adversarial tie × last-look interaction: two LPs tie on BOTH price
/// and epoch, and the tie-break-preferred one (lexicographically smallest
/// `lp_id`) is last-look-stale. The promotion must go to the **tied sibling**
/// (same best price, next-smallest `lp_id`), never to a lower-priced fresh LP —
/// the last-look filter applies BEFORE the tie-break, not after it. (The
/// property sweep draws continuous prices, so an exact tie has measure zero
/// there; this pins the case deterministically.)
#[tokio::test]
async fn stale_tie_break_preferred_winner_promotes_tied_sibling() {
    tokio::time::timeout(DEADLINE, async {
        let req = sample_request();
        let now: u64 = 1_000;

        // AAA and BBB tie exactly (bid 0.0090, epoch 100); AAA would win the
        // tie-break but expired at t=500 < now=1000. CCC is fresh with an
        // EARLIER epoch but a worse bid — it must not be promoted over BBB.
        let aaa = LadderSource::firm("AAA", 0.0090, 0.0100, 100).valid_until(500);
        let bbb = LadderSource::firm("BBB", 0.0090, 0.0100, 100).valid_until(u64::MAX);
        let ccc = LadderSource::firm("CCC", 0.0089, 0.0101, 50).valid_until(u64::MAX);

        let engine = MultiDealerEngine::new(vec![Box::new(aaa), Box::new(bbb), Box::new(ccc)]);
        let panel = engine.request(&req, PANEL_DEADLINE, now).await.unwrap();

        // AAA responded ⇒ it counts and its row is present — it just cannot WIN.
        assert_eq!(panel.lp_count, 3);
        assert!(panel.rows.iter().any(|r| r.lp_id == "AAA"));
        // The tied sibling is promoted at the SAME best price.
        assert_eq!(panel.lp_won_bid.as_deref(), Some("BBB"));
        assert!((panel.best_bid.unwrap() - 0.0090).abs() < 1e-12);
        // Offer side promotes identically (AAA's 0.0100 is stale; BBB ties it).
        assert_eq!(panel.lp_won_offer.as_deref(), Some("BBB"));
        assert!((panel.best_offer.unwrap() - 0.0100).abs() < 1e-12);
    })
    .await
    .expect("test timed out");
}

/// Gate 5 — combined `lp_count`-vs-responders accounting in ONE panel: a
/// timed-out source and a declining source are dropped (absent from `rows`,
/// excluded from `lp_count`), a last-look-stale source IS a responder (counted,
/// row present) yet cannot win, and the winners come from the liftable rows per
/// the law — all interacting in the same fan-out.
#[tokio::test]
async fn mixed_timeout_decline_stale_accounting_in_one_panel() {
    tokio::time::timeout(DEADLINE, async {
        let req = sample_request();
        let now: u64 = 1_000;

        // TIMEOUT holds the best bid but sleeps past the 200ms panel deadline.
        let timeout =
            LadderSource::firm("TIMEOUT", 0.0099, 0.0098, 100).with_delay(Duration::from_secs(2));
        // DECLINE holds the next-best bid but returns NoQuote.
        let mut decline = LadderSource::firm("DECLINE", 0.0098, 0.0097, 100);
        decline.decline = true;
        // STALE responds with the best surviving bid AND offer, but expired.
        let stale = LadderSource::firm("STALE", 0.0097, 0.0090, 100).valid_until(500);
        // The liftable market.
        let fresh_a = LadderSource::firm("FRESH-A", 0.0095, 0.0099, 200).valid_until(u64::MAX);
        let fresh_b = LadderSource::firm("FRESH-B", 0.0080, 0.0101, 300).valid_until(u64::MAX);

        let engine = MultiDealerEngine::new(vec![
            Box::new(timeout),
            Box::new(decline),
            Box::new(stale),
            Box::new(fresh_a),
            Box::new(fresh_b),
        ]);
        let panel = engine
            .request(&req, Duration::from_millis(200), now)
            .await
            .unwrap();

        // Responders = STALE + FRESH-A + FRESH-B; dropped = TIMEOUT + DECLINE.
        assert_eq!(panel.lp_count, 3);
        assert_eq!(panel.rows.len(), 3);
        assert!(
            panel
                .rows
                .iter()
                .all(|r| r.lp_id != "TIMEOUT" && r.lp_id != "DECLINE")
        );
        assert!(panel.rows.iter().any(|r| r.lp_id == "STALE"));

        // Winners per the law over the LIFTABLE rows only: STALE's better bid
        // (0.0097) and offer (0.0090) cannot win; FRESH-A takes both sides.
        assert_eq!(panel.lp_won_bid.as_deref(), Some("FRESH-A"));
        assert!((panel.best_bid.unwrap() - 0.0095).abs() < 1e-12);
        assert_eq!(panel.lp_won_offer.as_deref(), Some("FRESH-A"));
        assert!((panel.best_offer.unwrap() - 0.0099).abs() < 1e-12);

        // Consistency invariant: both winners are real responder rows.
        for won in [&panel.lp_won_bid, &panel.lp_won_offer] {
            assert!(panel.rows.iter().any(|r| Some(&r.lp_id) == won.as_ref()));
        }
    })
    .await
    .expect("test timed out");
}

/// Gate 3b — a declining LP (`NoQuote`) is dropped exactly like a timeout, and
/// the native in-process dealer guarantees a market still exists.
#[tokio::test]
async fn declining_lp_dropped_native_dealer_keeps_market() {
    tokio::time::timeout(DEADLINE, async {
        use celnet_rfq::InternalPricerSource;
        let req = sample_request();

        let mut decliner = LadderSource::firm("DECLINE", 0.0099, 0.0090, 100);
        decliner.decline = true;

        // Native dealer: mid 0.0085 ± 0.0005 ⇒ bid 0.0080 / offer 0.0090.
        let native = InternalPricerSource::new("CELNET-NATIVE", 0.0085, 0.0005, 200, 10_000);
        let other = LadderSource::firm("OTHER", 0.0082, 0.0095, 300);

        let engine =
            MultiDealerEngine::new(vec![Box::new(decliner), Box::new(native), Box::new(other)]);
        let panel = engine.request(&req, PANEL_DEADLINE, 0).await.unwrap();

        // DECLINE dropped ⇒ 2 responders; market still exists via the native.
        assert_eq!(panel.lp_count, 2);
        assert!(panel.rows.iter().all(|r| r.lp_id != "DECLINE"));
        assert!(panel.best_bid.is_some());
        assert!(panel.best_offer.is_some());
        // Best bid is OTHER 0.0082; best offer is native/OTHER min(0.0090,0.0095)=0.0090.
        assert_eq!(panel.lp_won_bid.as_deref(), Some("OTHER"));
        assert_eq!(panel.lp_won_offer.as_deref(), Some("CELNET-NATIVE"));
    })
    .await
    .expect("test timed out");
}
