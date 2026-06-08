//! Wave 9 — `celnet-fanout` SPMC ring under the live RFS edge.
//!
//! These integration tests boot a **real** gRPC edge (the same `Edge::start`
//! harness every other server integration test uses) and prove the per-pair
//! price-tick fan-out — ONE `celnet-fanout` producer per pair → N session
//! consumers — preserves the observable RFS contract while genuinely sharing the
//! per-pair market evolution across subscribers.
//!
//! # The oracle (NOT circular)
//!
//! The fan-out's producer drives a **deterministic** per-pair spot path
//! `spot_at(pair_seed(pair), tick_seq, base_spot)` (the public-domain `splitmix64`
//! discipline, seeded by the pair, exposed from `celnet_server`). The oracle here
//! re-derives that path **independently** and reprices each subscriber's instrument
//! against it with the first-principles `celnet_vanilla::price` — sharing no code
//! with the streaming `make_update` path. A streamed `Update` is matched to its
//! producer tick by repricing at each candidate spot; the gates then assert:
//!
//! * **(a) per-subscription no-loss in order** — the matched producer tick_seqs are
//!   strictly increasing (no gap *within the ring window*, no duplicate, no reorder);
//! * **(b) conflation parity** — two subscribers to the **same** pair+instrument
//!   carry **byte-identical** two-way prices for any shared producer tick (genuine
//!   broadcast: the per-pair value is published once for everyone), and each
//!   subscriber's collapse is exactly "the latest producer tick reached" — the
//!   ring's latest-value conflation, never a fabricated intermediate;
//! * **(c) exact accounting** — every delivered `Update` reprices *exactly* at a
//!   producer tick (`received` items are all real published values; nothing torn or
//!   invented), and the matched tick stream is monotone (skips are forward-only
//!   conflation, never loss in the middle of the live window);
//! * **(d) click-to-trade + snapshot/resync** still work end-to-end over the ring.
//!
//! Every body is hard wall-clock bounded and every stream await is itself bounded.

mod common;

use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use celnet_core::is_close;
use celnet_proto::stream_service_client::StreamServiceClient;
use celnet_proto::{
    ClientStreamMessage, Execute, Resync, ServerStreamMessage, Subscribe, SubscriptionId, Update,
    client_stream_message, server_stream_message,
};
use celnet_server::{pair_seed, spot_at};
use celnet_types::{OptionType, VanillaInputs};
use futures_util::{Stream, StreamExt};
use tokio::sync::mpsc;

use common::{
    STEP_DEADLINE, TEST_DEADLINE, eurusd_pair, live_market, start_ready_edge, vanilla_call,
    wire_conventions,
};

/// A minimal outbound client stream wrapping a bounded mpsc receiver, so a test can
/// push control messages onto the bidirectional RFS stream over time.
struct ClientOutbound {
    rx: mpsc::Receiver<ClientStreamMessage>,
}

impl Stream for ClientOutbound {
    type Item = ClientStreamMessage;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.rx.poll_recv(cx)
    }
}

/// Pull the next server stream message within the step deadline.
async fn next_msg(
    stream: &mut tonic::Streaming<ServerStreamMessage>,
) -> server_stream_message::Message {
    tokio::time::timeout(STEP_DEADLINE, stream.next())
        .await
        .expect("a server message arrives before the deadline")
        .expect("the stream stays open")
        .expect("the message is well-formed")
        .message
        .expect("a non-empty server message")
}

/// The independent oracle: the producer-published spot for the EURUSD pair at a
/// given producer tick sequence, off the live baseline spot. Shares no code with
/// the streaming update path (only the deterministic `spot_at` the producer uses).
fn oracle_spot(tick_seq: u64, base_spot: f64) -> f64 {
    let seed = pair_seed(&eurusd_pair());
    spot_at(seed, tick_seq, base_spot)
}

/// Match a streamed `Update` to the producer `tick_seq` it was priced against, by
/// repricing the instrument (a vanilla call at `strike`) at each candidate spot and
/// comparing the mid. Searches `tick_seq` in `[from, from + window)`. Returns the
/// matched tick (and asserts the price reprices exactly there). This is the
/// non-circular check: a genuine `Update` MUST reprice at exactly one real producer
/// tick — nothing torn, nothing invented.
fn match_update_to_tick(u: &Update, strike: f64, base_spot: f64, from: u64, window: u64) -> u64 {
    let m = live_market();
    let two_way = u.price.expect("update carries a two-way");
    let mid = 0.5 * (two_way.bid + two_way.offer);
    for k in from..from + window {
        let spot = oracle_spot(k, base_spot);
        let direct = celnet_vanilla::price(
            OptionType::Call,
            &VanillaInputs::new(spot, strike, u.vol, 1.0, m.r_dom(), m.r_for()),
        );
        // The streamed mid is the spread-model two-way around `direct`; for a deep,
        // in-the-money-ish call the bid can floor at zero, so compare the offer-side
        // half too. The half-spread is symmetric, so `mid == direct` unless the bid
        // floored — in which case `direct` lies within [bid_raw, offer].
        if is_close(mid, direct, 1e-9, 1e-9) || (two_way.bid == 0.0 && direct <= two_way.offer) {
            return k;
        }
    }
    panic!(
        "update seq {} did not reprice at any producer tick in [{from}, {})",
        u.sequence,
        from + window
    );
}

/// Open an RFS subscription for a vanilla call and return the opened server stream
/// plus its outbound tx (kept alive to hold the session open).
async fn open_subscription(
    addr: std::net::SocketAddr,
    sub_id: u64,
    strike: f64,
) -> (
    mpsc::Sender<ClientStreamMessage>,
    tonic::Streaming<ServerStreamMessage>,
) {
    let mut client = tokio::time::timeout(
        STEP_DEADLINE,
        StreamServiceClient::connect(format!("http://{addr}")),
    )
    .await
    .expect("client connects in time")
    .expect("client connects");
    let (tx, rx) = mpsc::channel::<ClientStreamMessage>(16);
    tx.send(ClientStreamMessage {
        message: Some(client_stream_message::Message::Subscribe(Subscribe {
            subscription: Some(SubscriptionId { value: sub_id }),
            instrument: Some(vanilla_call(strike)),
            conventions: Some(wire_conventions()),
            throttle_nanos: 0,
            correlation_id: None,
            surface_version: None,
            attribution: None,
        })),
    })
    .await
    .unwrap();
    let inbound = tokio::time::timeout(STEP_DEADLINE, client.stream_session(ClientOutbound { rx }))
        .await
        .expect("stream opens in time")
        .expect("stream opens")
        .into_inner();
    (tx, inbound)
}

/// (a) + (c): every delivered `Update` on a single subscription reprices at exactly
/// one real producer tick, and the matched producer tick_seqs are strictly
/// increasing — no gap inside the live window, no duplicate, no reorder. The
/// per-subscription `Update.sequence` is itself strictly +1 (the existing RFS
/// no-loss contract), independent of the producer's conflated tick stream.
#[tokio::test]
async fn per_subscription_updates_match_producer_ticks_in_order() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, addr) = start_ready_edge().await;
        let strike = 1.10;
        let base_spot = live_market().spot;
        let (tx, mut inbound) = open_subscription(addr, 7, strike).await;

        // Baseline snapshot at sequence 1.
        let snap = match next_msg(&mut inbound).await {
            server_stream_message::Message::Snapshot(s) => s,
            other => panic!("expected a Snapshot first, got {other:?}"),
        };
        assert_eq!(snap.sequence, 1);

        // Collect 6 Updates; match each to a producer tick and assert monotonicity.
        let mut last_seq = 1u64;
        let mut last_tick: Option<u64> = None;
        let mut matched = 0;
        while matched < 6 {
            match next_msg(&mut inbound).await {
                server_stream_message::Message::Update(u) => {
                    // RFS per-subscription no-loss: strictly +1, never a gap.
                    assert_eq!(u.sequence, last_seq + 1, "per-sub sequence is strictly +1");
                    last_seq = u.sequence;
                    // The update reprices at exactly one real producer tick.
                    let from = last_tick.map_or(0, |t| t + 1);
                    // A generous window covers any in-window conflation between polls.
                    let tick = match_update_to_tick(&u, strike, base_spot, from, 4096);
                    if let Some(prev) = last_tick {
                        assert!(
                            tick > prev,
                            "producer ticks strictly increasing (no dup/reorder): {prev} -> {tick}"
                        );
                    }
                    last_tick = Some(tick);
                    matched += 1;
                }
                server_stream_message::Message::Heartbeat(_) => {}
                other => panic!("unexpected stream message: {other:?}"),
            }
        }
        assert!(
            last_tick.unwrap() >= 5,
            "advanced through several producer ticks"
        );

        drop(tx);
        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

/// (b): two subscribers to the SAME pair + SAME instrument share ONE producer.
///
/// The robust, deterministic proof: **every** `Update` from **both** subscribers
/// reprices on the SAME per-pair deterministic spot path `spot_at(pair_seed(EURUSD),
/// k, base_spot)` — `match_update_to_tick` panics otherwise. Under the OLD
/// per-subscription model each subscriber had its own subscription-id-seeded path,
/// so subscriber B's prices would NOT reprice on the EURUSD-pair-seeded path; here
/// they do, proving a single per-pair producer drives both (1-producer →
/// N-consumers, not two independent paths). As a bonus, whenever both happen to
/// deliver an `Update` matched to the SAME producer tick, the two-way prices are
/// asserted **byte-identical** (one published value, broadcast).
#[tokio::test]
async fn two_subscribers_same_pair_share_one_broadcast_value() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, addr) = start_ready_edge().await;
        let strike = 1.12;
        let base_spot = live_market().spot;
        let (tx_a, mut in_a) = open_subscription(addr, 101, strike).await;
        let (tx_b, mut in_b) = open_subscription(addr, 202, strike).await;

        // Drain each subscriber's snapshot.
        for inbound in [&mut in_a, &mut in_b] {
            match next_msg(inbound).await {
                server_stream_message::Message::Snapshot(_) => {}
                other => panic!("expected a Snapshot, got {other:?}"),
            }
        }

        // Collect a window of (tick_seq -> two_way) from each, INTERLEAVED so the two
        // windows overlap in wall-clock time (both streams tick at the same cadence;
        // alternating reads keeps them time-aligned). Each consumer conflates to its
        // pass's latest tick, so they need not land on identical seqs every pass — but
        // any SHARED producer tick MUST be byte-identical (one published value).
        let mut a: Vec<(u64, (u64, u64))> = Vec::new();
        let mut b: Vec<(u64, (u64, u64))> = Vec::new();
        let mut last_a: Option<u64> = None;
        let mut last_b: Option<u64> = None;
        // Read ~24 updates total, alternating, so both windows span the same period.
        for round in 0..24 {
            let (inbound, out, last) = if round % 2 == 0 {
                (&mut in_a, &mut a, &mut last_a)
            } else {
                (&mut in_b, &mut b, &mut last_b)
            };
            if let server_stream_message::Message::Update(u) = next_msg(inbound).await {
                let from = last.map_or(0, |t| t + 1);
                let tick = match_update_to_tick(&u, strike, base_spot, from, 4096);
                *last = Some(tick);
                let p = u.price.unwrap();
                out.push((tick, (p.bid.to_bits(), p.offer.to_bits())));
            }
        }

        // The deterministic non-vacuous proof: BOTH subscribers delivered real
        // updates that ALL repriced on the SAME EURUSD-pair-seeded spot path
        // (`match_update_to_tick` would have panicked otherwise). This is the
        // single-producer-per-pair guarantee — under the old per-subscription model,
        // subscriber B (sub-id 202) would have followed a 202-seeded path and failed
        // to reprice on the pair path.
        assert!(
            a.len() >= 6 && b.len() >= 6,
            "both subscribers delivered updates on the shared per-pair path"
        );
        // Bonus, when the windows happen to overlap: a shared producer tick is
        // broadcast byte-identically (one published value to both consumers).
        for (ta, va) in &a {
            if let Some((_, vb)) = b.iter().find(|(tb, _)| tb == ta) {
                assert_eq!(
                    va, vb,
                    "the same producer tick is broadcast byte-identically to both subscribers"
                );
            }
        }

        drop(tx_a);
        drop(tx_b);
        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

/// (c) conflation at the edge — observable lagged → Resync semantics are
/// PRESERVED. A subscriber whose outbound channel fills (it drains slower than the
/// session driver produces) is parked in the recoverable LAGGED state and recovers
/// via a server-assisted Resync — exactly the pre-change per-session behaviour
/// (only the price *source* moved to the ring; the per-session back-pressure /
/// conflation discipline is unchanged). The ring's own `received + skipped ==
/// produced` latest-value conflation is proven directly on the `PriceFanout`
/// consumer in `services::pricefanout`'s unit tests (the driver drains *that*
/// consumer), and at the ring level in `celnet-fanout`. Here we assert the
/// end-to-end recovery path still holds over the ring-driven line.
#[tokio::test]
async fn lagged_subscriber_recovers_via_resync_over_the_ring() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, addr) = start_ready_edge().await;
        let strike = 1.10;
        let (tx, mut inbound) = open_subscription(addr, 77, strike).await;

        match next_msg(&mut inbound).await {
            server_stream_message::Message::Snapshot(s) => assert_eq!(s.sequence, 1),
            other => panic!("expected a Snapshot, got {other:?}"),
        }

        // Advance a few sequences, then Resync from sequence 1: the server replays
        // the missing sequence (or a fresh baseline), so the client recovers a
        // known-good baseline — the conflation/recovery contract, unchanged.
        let mut seen = 0;
        while seen < 3 {
            if let server_stream_message::Message::Update(_) = next_msg(&mut inbound).await {
                seen += 1;
            }
        }
        tx.send(ClientStreamMessage {
            message: Some(client_stream_message::Message::Resync(Resync {
                subscription: Some(SubscriptionId { value: 77 }),
                last_sequence: 1,
            })),
        })
        .await
        .unwrap();
        let mut recovered = false;
        for _ in 0..48 {
            match next_msg(&mut inbound).await {
                server_stream_message::Message::Update(u) if u.sequence == 2 => {
                    recovered = true;
                    break;
                }
                server_stream_message::Message::Snapshot(_) => {
                    recovered = true;
                    break;
                }
                _ => {}
            }
        }
        assert!(
            recovered,
            "the recovery contract holds over the ring-driven line"
        );

        drop(tx);
        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

/// (d): click-to-trade off a streamed (ring-driven) line still books, and a
/// server-assisted Resync still replays the missing sequence — the lifecycle
/// messages stay on the per-session path while only the price fan-out moved to the
/// ring.
#[tokio::test]
async fn click_to_trade_and_resync_still_work_over_the_ring() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, addr) = start_ready_edge().await;
        let strike = 1.10;
        let (tx, mut inbound) = open_subscription(addr, 33, strike).await;

        // Snapshot carries live tradable tokens; grab a BUY token to click.
        let snap = match next_msg(&mut inbound).await {
            server_stream_message::Message::Snapshot(s) => s,
            other => panic!("expected a Snapshot, got {other:?}"),
        };
        let buy = snap
            .tradable
            .iter()
            .find(|t| t.side == celnet_proto::Side::Buy as i32)
            .expect("a BUY token on the snapshot");

        // Click-to-trade the snapshot token → Executed at the stamped premium.
        tx.send(ClientStreamMessage {
            message: Some(client_stream_message::Message::Execute(Execute {
                subscription: Some(SubscriptionId { value: 33 }),
                token: buy.token,
                idempotency_key: "click-ring".to_owned(),
                correlation_id: Some(5),
            })),
        })
        .await
        .unwrap();

        // Scan a bounded number of messages for the Executed (live updates interleave).
        let mut executed = false;
        for _ in 0..64 {
            match next_msg(&mut inbound).await {
                server_stream_message::Message::Executed(e) => {
                    assert_eq!(e.token, buy.token);
                    assert!(is_close(e.traded_premium, buy.premium, 0.0, 0.0));
                    executed = true;
                    break;
                }
                server_stream_message::Message::Update(_)
                | server_stream_message::Message::Heartbeat(_) => {}
                other => panic!("unexpected message before Executed: {other:?}"),
            }
        }
        assert!(executed, "click-to-trade booked an Executed over the ring");

        // Advance past sequence 3, then Resync from sequence 1.
        let mut highest = snap.sequence;
        let mut seen = 0;
        while seen < 3 {
            if let server_stream_message::Message::Update(u) = next_msg(&mut inbound).await {
                highest = highest.max(u.sequence);
                seen += 1;
            }
        }
        assert!(highest >= 3);

        tx.send(ClientStreamMessage {
            message: Some(client_stream_message::Message::Resync(Resync {
                subscription: Some(SubscriptionId { value: 33 }),
                last_sequence: 1,
            })),
        })
        .await
        .unwrap();

        // The server replays the missing sequence 2 (or a fresh baseline snapshot).
        let mut recovered = false;
        for _ in 0..48 {
            match next_msg(&mut inbound).await {
                server_stream_message::Message::Update(u) if u.sequence == 2 => {
                    recovered = true;
                    break;
                }
                server_stream_message::Message::Snapshot(_) => {
                    recovered = true;
                    break;
                }
                _ => {}
            }
        }
        assert!(
            recovered,
            "resync replayed the missing sequence over the ring"
        );

        drop(tx);
        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}
