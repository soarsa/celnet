//! Trader-workflow conformance tests for the multi-dealer (RFQ-to-many) panel
//! over the typed SDK — the client half of the `RequestMultiDealerQuote` →
//! `MultiDealerQuote` → `AcceptQuote(lp_id)` contract.
//!
//! Each test boots a real in-process `celnet-server` edge with an **explicit**
//! ≥3-LP panel (the explicit-panel boot path — no process-global env mutation)
//! and asserts, through the [`celnet_client`] SDK:
//!
//! 1. the ranked panel obeys the engine law **re-derived in-test from the raw
//!    rows** (best bid = max bid, best offer = min offer, lexicographic `lp_id`
//!    tie-break over liftable rows) — never trusted from the server's own
//!    ranking;
//! 2. booking the best-offer winner returns an [`celnet_client::Execution`]
//!    whose premium equals that row's offer **bit-for-bit** (the pinned line,
//!    never a re-price), attributed to the winning LP;
//! 3. the default accept (no `lp_id`) is the single-dealer path byte-identical:
//!    the native maker row equals a plain [`celnet_client::Rfq`] quote to the
//!    bit, and both accepts book the same premium bits;
//! 4. an accept after a panel row's last-look deadline is refused
//!    (`deadline_exceeded`), driven deterministically by a manual clock.
//!
//! Honest boundary: the panel beyond the native maker is the edge's labeled
//! deterministic **synthetic** demo/test dealers quoting around the same edge
//! mid — live LP connectivity is ENV, never claimed here. Every body is hard
//! wall-clock bounded and every network await is bounded, so a regression fails
//! fast, never hangs.

mod common;

use std::time::Duration;

use celnet_client::{ClientError, DealerQuote, Side};
use celnet_server::Clock;

use common::{
    STEP_DEADLINE, TEST_DEADLINE, conventions, start_panel_edge_and_client, vanilla_call,
};

/// The synthetic demo/test panel breadth every test boots (native maker + 3).
const SYNTHETIC_LPS: u32 = 3;

/// Independently re-derive one side's winner per the engine law over the raw
/// panel rows: only liftable rows compete (`valid_until_nanos >= now`), the best
/// bid is the **max** bid / the best offer the **min** offer, and an equal best
/// price falls to the earlier `epoch_nanos` then the lexicographically smallest
/// `lp_id` — every wire row shares the aggregate quote's epoch, so the epoch leg
/// always ties here and the `lp_id` leg decides.
fn law_winner(rows: &[DealerQuote], now_nanos: i64, bid_side: bool) -> Option<String> {
    let mut best: Option<&DealerQuote> = None;
    for d in rows.iter().filter(|d| d.valid_until_nanos >= now_nanos) {
        let better = match best {
            None => true,
            Some(b) => {
                let (dp, bp) = if bid_side {
                    (d.price.bid, b.price.bid)
                } else {
                    (d.price.offer, b.price.offer)
                };
                if dp == bp {
                    d.lp_id < b.lp_id
                } else if bid_side {
                    dp > bp
                } else {
                    dp < bp
                }
            }
        };
        if better {
            best = Some(d);
        }
    }
    best.map(|d| d.lp_id.clone())
}

/// The native maker's row: the one carrying the edge-priced greeks (an LP
/// discloses a price, not its greeks) — asserted unique.
fn native_row(rows: &[DealerQuote]) -> &DealerQuote {
    let mut native = rows.iter().filter(|d| d.greeks.is_some());
    let row = native.next().expect("the native maker row carries greeks");
    assert!(
        native.next().is_none(),
        "exactly one row (the native maker) carries greeks"
    );
    row
}

/// A ≥3-LP panel through the SDK: native + synthetic rows, each an uncrossed
/// two-way on the same resolved line with a forward last-look window, ranked per
/// the engine law re-derived in-test from the raw rows — never trusted from the
/// server's own `best_*_lp_id`.
#[tokio::test]
async fn panel_ranks_per_engine_law_rechecked_from_raw_rows() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let clock = Clock::manual(1_000_000_000);
        let (edge, client) = start_panel_edge_and_client(clock.clone(), SYNTHETIC_LPS).await;

        let md = client.request_multi_dealer_quote(vanilla_call(1.12), conventions());
        let panel = tokio::time::timeout(STEP_DEADLINE, md.request())
            .await
            .expect("panel request returns in time")
            .expect("panel request succeeds");

        assert!(panel.quote_id >= 1, "an aggregate quote id is assigned");
        assert_eq!(
            panel.idempotency_key,
            md.idempotency_key(),
            "the panel echoes the handle's key"
        );
        assert_eq!(
            panel.dealers.len(),
            1 + SYNTHETIC_LPS as usize,
            "native maker + {SYNTHETIC_LPS} synthetic demo dealers"
        );

        let now = clock.now_nanos();
        let native = native_row(&panel.dealers);
        for d in &panel.dealers {
            assert!(
                d.price.bid < d.price.offer,
                "{}: uncrossed two-way {:?}",
                d.lp_id,
                d.price
            );
            assert_eq!(
                d.resolved_strike.to_bits(),
                native.resolved_strike.to_bits(),
                "{}: every dealer quotes the same resolved line",
                d.lp_id
            );
            assert!(
                d.valid_until_nanos > panel.epoch_nanos,
                "{}: forward last-look deadline",
                d.lp_id
            );
            assert!(
                d.last_look_remaining(now).is_some(),
                "{}: the last-look window is open at issue time",
                d.lp_id
            );
            // The documented panel law: a synthetic dealer never quotes a
            // tighter spread than the maker's own — re-checked from raw rows.
            if d.greeks.is_none() {
                assert!(
                    d.price.offer - d.price.bid > native.price.offer - native.price.bid,
                    "{}: a synthetic dealer is never tighter than the maker",
                    d.lp_id
                );
            }
        }

        // The engine law, re-derived IN-TEST from the raw rows: max bid / min
        // offer / lexicographic lp_id tie-break over liftable rows. The server's
        // own winner fields must match the independent recomputation.
        assert_eq!(
            panel.best_bid_lp_id,
            law_winner(&panel.dealers, now, true),
            "best bid winner re-derived from raw rows"
        );
        assert_eq!(
            panel.best_offer_lp_id,
            law_winner(&panel.dealers, now, false),
            "best offer winner re-derived from raw rows"
        );
        // The ranked touch is never crossed.
        let best_bid = panel.best_bid().expect("a best bid exists");
        let best_offer = panel.best_offer().expect("a best offer exists");
        assert!(
            best_bid.price.bid <= best_offer.price.offer,
            "panel touch is uncrossed: {} / {}",
            best_bid.price.bid,
            best_offer.price.offer
        );

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

/// Booking the best-offer winner books exactly that pinned row: the execution's
/// premium equals the row's offer bit-for-bit (never a re-price), attributed to
/// the winning LP, and a retry returns the same execution.
#[tokio::test]
async fn booking_best_offer_winner_books_that_rows_offer_bit_for_bit() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let clock = Clock::manual(1_000_000_000);
        let (edge, client) = start_panel_edge_and_client(clock.clone(), SYNTHETIC_LPS).await;

        let md = client.request_multi_dealer_quote(vanilla_call(1.12), conventions());
        let panel = tokio::time::timeout(STEP_DEADLINE, md.request())
            .await
            .expect("panel request returns in time")
            .expect("panel request succeeds");

        // The winner per the in-test law (cross-checked against the server in the
        // ranking test); BUY lifts its offer.
        let lp = law_winner(&panel.dealers, clock.now_nanos(), false)
            .expect("a best offer exists on a ≥3-LP panel");
        let row = panel
            .dealer(&lp)
            .expect("the winner is a panel row")
            .clone();

        let exec = tokio::time::timeout(STEP_DEADLINE, md.accept_dealer(&panel, Side::Buy, &*lp))
            .await
            .expect("accept returns in time")
            .expect("accept succeeds");

        assert_eq!(
            exec.quote_id, panel.quote_id,
            "booked under the aggregate id"
        );
        assert_eq!(exec.side, Side::Buy);
        assert_eq!(
            exec.traded_premium.to_bits(),
            row.price.offer.to_bits(),
            "BUY books the pinned row's offer bit-for-bit: {} vs {}",
            exec.traded_premium,
            row.price.offer
        );
        // The fill is attributed to the winning LP, never anonymous.
        let quoted_by = &exec
            .attribution
            .as_ref()
            .expect("a dealer-line fill is attributed")
            .quoted_by;
        assert_eq!(quoted_by.book, lp, "the booking names the winning LP");

        // An accept retry (same side, same line) returns the SAME execution.
        let retry = tokio::time::timeout(STEP_DEADLINE, md.accept_dealer(&panel, Side::Buy, &*lp))
            .await
            .expect("retry returns in time")
            .expect("retry succeeds");
        assert_eq!(
            retry.execution_id, exec.execution_id,
            "an accept retry returns the same execution (no double book)"
        );
        assert_eq!(
            retry.traded_premium.to_bits(),
            exec.traded_premium.to_bits()
        );

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

/// The default accept (no `lp_id`) is the single-dealer path byte-identical: on
/// the same frozen-clock edge, the panel's native maker row equals a plain
/// single-dealer [`celnet_client::Rfq`] quote to the bit, and both accepts book
/// the same premium bits.
#[tokio::test]
async fn default_accept_is_single_dealer_path_byte_identical() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let clock = Clock::manual(1_000_000_000);
        let (edge, client) = start_panel_edge_and_client(clock.clone(), SYNTHETIC_LPS).await;

        // The single-dealer reference: a plain RFQ on the same instrument at the
        // same frozen instant — the maker's deterministic pricing makes its line
        // reproducible to the bit.
        let rfq = client.request_quote(vanilla_call(1.12), conventions());
        let single = tokio::time::timeout(STEP_DEADLINE, rfq.request())
            .await
            .expect("single-dealer quote in time")
            .expect("single-dealer quote ok");

        let md = client.request_multi_dealer_quote(vanilla_call(1.12), conventions());
        let panel = tokio::time::timeout(STEP_DEADLINE, md.request())
            .await
            .expect("panel request in time")
            .expect("panel request ok");

        // The native maker row IS the single-dealer line: same two-way, bit for
        // bit, and the same resolved strike.
        let native = native_row(&panel.dealers).clone();
        assert_eq!(
            native.price.bid.to_bits(),
            single.price.bid.to_bits(),
            "native panel bid == single-dealer bid bit-for-bit"
        );
        assert_eq!(
            native.price.offer.to_bits(),
            single.price.offer.to_bits(),
            "native panel offer == single-dealer offer bit-for-bit"
        );
        assert_eq!(
            native.resolved_strike.to_bits(),
            single.resolved_strike.to_bits()
        );

        // The default accept (empty lp_id on the wire) books the maker line —
        // byte-identical to the single-dealer accept of the plain quote.
        let exec_panel = tokio::time::timeout(STEP_DEADLINE, md.accept(&panel, Side::Buy))
            .await
            .expect("panel default accept in time")
            .expect("panel default accept ok");
        let exec_single = tokio::time::timeout(STEP_DEADLINE, rfq.accept(&single, Side::Buy))
            .await
            .expect("single-dealer accept in time")
            .expect("single-dealer accept ok");

        assert_eq!(
            exec_panel.traded_premium.to_bits(),
            native.price.offer.to_bits(),
            "the default accept books the native maker offer bit-for-bit"
        );
        assert_eq!(
            exec_panel.traded_premium.to_bits(),
            exec_single.traded_premium.to_bits(),
            "panel default accept == single-dealer accept, byte-identical premium"
        );
        assert_eq!(exec_panel.side, exec_single.side);

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

/// An accept after a panel row's last-look deadline is refused
/// (`deadline_exceeded`) — the engine law applied to the exact line being booked
/// — driven deterministically past the window by a manual clock, with the SDK's
/// per-row [`DealerQuote::last_look_remaining`] flipping `Some` → `None` at the
/// same instant.
#[tokio::test]
async fn accept_after_panel_row_last_look_is_refused() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let clock = Clock::manual(1_000_000_000);
        let (edge, client) = start_panel_edge_and_client(clock.clone(), SYNTHETIC_LPS).await;

        let md = client.request_multi_dealer_quote(vanilla_call(1.12), conventions());
        let panel = tokio::time::timeout(STEP_DEADLINE, md.request())
            .await
            .expect("panel request in time")
            .expect("panel request ok");

        let lp = panel
            .best_offer_lp_id
            .clone()
            .expect("a best offer exists on a ≥3-LP panel");
        let row = panel.dealer(&lp).expect("the winner is a panel row");
        assert!(
            row.last_look_remaining(clock.now_nanos()).is_some(),
            "the line is liftable before the window lapses"
        );

        // Jump the clock past the 5-second validity window: every line expires.
        clock.advance(6_000_000_000);
        assert!(
            row.last_look_remaining(clock.now_nanos()).is_none(),
            "the SDK reports the lapsed window as no remaining last-look"
        );

        let err = tokio::time::timeout(STEP_DEADLINE, md.accept_dealer(&panel, Side::Buy, &*lp))
            .await
            .expect("accept returns in time")
            .expect_err("an accept past the row's last-look deadline must be refused");
        match err {
            ClientError::Status(s) => {
                assert_eq!(s.code(), tonic::Code::DeadlineExceeded);
            }
            other => panic!("expected a status error, got {other:?}"),
        }

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}
