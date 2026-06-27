//! Trader-workflow integration tests for the multiplexed RFS streaming SDK.
//!
//! These drive a real in-process `celnet-server` edge through the typed
//! [`celnet_client`] SDK over **one** [`celnet_client::StreamSession`] and assert the
//! client results equal the underlying analytics crate's direct computation — the
//! same EURUSD fixture and the same first-principles references the server's own
//! integration tests use, so the SDK is validated end-to-end against the analytics,
//! never merely "looks plausible".
//!
//! * **Market-maker streams many instruments over ONE session.** A maker streams a
//!   1Y 25Δ risk reversal *and* three vanilla calls at distinct strikes over a
//!   single multiplexed session; the client tracks all four subscriptions, each with
//!   its own strictly-sequenced stream (no gap ever surfaced), and each streamed
//!   line reprices consistently against `celnet-vanilla`.
//! * **Taker click-trades off a streamed line within the validity window.** A taker
//!   grabs a fresh streamed line, clicks to BUY, and books — the SDK handles the
//!   `tradable_token` transparently and the booked premium equals the streamed
//!   offer (which a first-principles GK price brackets). A click on a line whose
//!   token has aged past its validity window is *rejected* (last-look), never booked.
//! * **Risk manager pulls book-shaped risk.** A risk manager requests bucketed vega,
//!   cross-gamma, and a theta roll; the client's typed `BucketedRisk` equals a
//!   first-principles finite-difference decomposition computed directly with
//!   `celnet-vanilla`.
//!
//! Every body is hard wall-clock bounded and every stream await is bounded, so a
//! regression fails fast, never hangs.

mod common;

use std::time::Duration;

use celnet_client::{
    Client, ExecuteOutcome, InstrumentSpec, Leg, Quantity, RiskRequest, Side, StrategyKind,
    StreamEvent, StreamLine, StreamSession, StrikeSpec, Subscription,
};
use celnet_core::is_close;
use celnet_server::Clock;
use celnet_types::{OptionType, Tenor, VanillaInputs};

use common::{
    FIXTURE_SPOT, STEP_DEADLINE, TEST_DEADLINE, conventions, eurusd, live_market, login_seed_admin,
    start_edge_and_client_with, start_ready_edge, vanilla_call,
};

/// A 1Y EURUSD 25-delta risk reversal: long the 25Δ call, short the 25Δ put.
fn rr_25() -> InstrumentSpec {
    InstrumentSpec::strategy(
        eurusd(),
        Tenor::Years(1),
        1.0,
        Quantity::base(1_000_000.0),
        Side::TwoWay,
        StrategyKind::RiskReversal,
        vec![
            Leg::unit(OptionType::Call, StrikeSpec::Delta(0.25), Side::Buy),
            Leg::unit(OptionType::Put, StrikeSpec::Delta(-0.25), Side::Sell),
        ],
    )
}

/// Await the next stream event on a subscription within the step deadline, panicking
/// on a hang or a closed stream.
async fn next_event(sub: &mut Subscription) -> StreamEvent {
    tokio::time::timeout(STEP_DEADLINE, sub.next_event())
        .await
        .expect("a stream event arrives before the deadline")
        .expect("the stream stays open")
        .expect("the event is well-formed")
}

/// Consume the baseline snapshot of a subscription, returning its line + resolved
/// strike; panics if the first event is not a snapshot.
async fn baseline(sub: &mut Subscription) -> (StreamLine, f64) {
    match next_event(sub).await {
        StreamEvent::Snapshot {
            line,
            resolved_strike,
            ..
        } => {
            assert_eq!(line.sequence, 1, "baseline snapshot is sequence 1");
            (line, resolved_strike)
        }
        other => panic!("expected a Snapshot first, got {other:?}"),
    }
}

/// Drive a subscription forward `n` ticks (skipping heartbeats), returning the last
/// tick line; asserts strictly-sequenced advance with no gap surfaced.
async fn drive_ticks(sub: &mut Subscription, n: usize, mut last_seq: u64) -> StreamLine {
    let mut seen = 0;
    let mut last_line = None;
    while seen < n {
        match next_event(sub).await {
            StreamEvent::Tick(line) => {
                assert_eq!(
                    line.sequence,
                    last_seq + 1,
                    "ticks are strictly sequenced (SDK surfaces no gap)"
                );
                last_seq = line.sequence;
                last_line = Some(line);
                seen += 1;
            }
            StreamEvent::Heartbeat { sequence, .. } => assert!(sequence >= last_seq),
            StreamEvent::GapDetected { .. } => {
                panic!("the SDK must never surface a raw gap on a healthy stream")
            }
            other => panic!("unexpected stream event: {other:?}"),
        }
    }
    last_line.expect("at least one tick consumed")
}

/// Scenario: a market-maker streams a 25Δ risk reversal AND three vanilla calls over
/// ONE multiplexed session; the client tracks all four subscriptions independently,
/// each strictly sequenced, and each streamed line reprices consistently.
#[tokio::test]
async fn market_maker_streams_many_instruments_over_one_session() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, addr) = start_ready_edge(Clock::system()).await;
        let client = tokio::time::timeout(STEP_DEADLINE, Client::connect(format!("http://{addr}")))
            .await
            .expect("client connects in time")
            .expect("client connects");

        // ONE session multiplexing four subscriptions.
        let session: StreamSession = tokio::time::timeout(STEP_DEADLINE, client.open_session())
            .await
            .expect("session opens in time")
            .expect("session opens");

        let strikes = [1.08_f64, 1.12, 1.16];
        // Open the risk reversal and three vanilla calls over the same connection,
        // each with a distinct correlation id the snapshot echoes back.
        let mut rr = session
            .subscribe(rr_25(), conventions(), Some(1000), None)
            .await
            .expect("rr subscribes");
        let mut calls: Vec<Subscription> = Vec::new();
        for (i, &k) in strikes.iter().enumerate() {
            let corr = 2000 + i as u64;
            let sub = session
                .subscribe(vanilla_call(k), conventions(), Some(corr), None)
                .await
                .expect("call subscribes");
            calls.push(sub);
        }

        // Distinct per-session subscription ids (one connection, many lines).
        let mut ids = vec![rr.id()];
        ids.extend(calls.iter().map(Subscription::id));
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 4, "four distinct subscriptions on one session");

        // The risk reversal baseline: a headline strike resolved, correlation echoed.
        match next_event(&mut rr).await {
            StreamEvent::Snapshot {
                line,
                resolved_strike,
                correlation_id,
                ..
            } => {
                assert_eq!(line.sequence, 1);
                assert!(resolved_strike > 0.0, "a headline strike resolved");
                assert_eq!(correlation_id, Some(1000), "snapshot echoes correlation id");
                assert!(line.price.bid.is_finite() && line.price.offer.is_finite());
            }
            other => panic!("expected RR Snapshot, got {other:?}"),
        }
        let mut rr_seq = 1;

        // Each vanilla call baseline reprices against a first-principles GK price at
        // its streamed (spot, vol): the SDK carries the maker's real numbers.
        let m = live_market();
        for (i, sub) in calls.iter_mut().enumerate() {
            let (line, _strike) = baseline(sub).await;
            let direct = celnet_vanilla::price(
                OptionType::Call,
                &VanillaInputs::new(m.spot, strikes[i], line.vol, 1.0, m.r_dom, m.r_for),
            );
            let mid = line.price.mid();
            assert!(
                is_close(mid, direct, 1e-6, 1e-6) || line.price.bid == 0.0,
                "call {i} snapshot mid {mid} vs direct {direct}"
            );
        }

        // The session keeps every subscription advancing independently with no gap.
        rr_seq = drive_ticks(&mut rr, 3, rr_seq).await.sequence;
        assert!(rr_seq >= 4, "rr saw snapshot + 3 ticks: seq {rr_seq}");
        for sub in &mut calls {
            let last = drive_ticks(sub, 2, 1).await;
            assert!(last.sequence >= 3, "each call advanced past its baseline");
            assert!(last.greeks.price.is_finite() && last.vol > 0.0);
        }

        drop(rr);
        drop(calls);
        drop(session);
        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

/// Scenario: a taker click-trades off a streamed line within its validity window.
/// The SDK presents the line's BUY token transparently; the maker books at the
/// stamped offer, which a first-principles GK price brackets. The taker also tracks
/// a second subscription on the same session, proving execute and stream coexist.
#[tokio::test]
async fn taker_click_trades_a_streamed_line_within_the_window() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, addr) = start_ready_edge(Clock::system()).await;
        let client = tokio::time::timeout(STEP_DEADLINE, Client::connect(format!("http://{addr}")))
            .await
            .expect("client connects in time")
            .expect("client connects");

        let session = tokio::time::timeout(STEP_DEADLINE, client.open_session())
            .await
            .expect("session opens in time")
            .expect("session opens");

        let strike = 1.12;
        let mut traded = session
            .subscribe(vanilla_call(strike), conventions(), None, None)
            .await
            .expect("subscribe opens");
        // A second subscription multiplexed on the same session, to prove execute and
        // independent streaming coexist.
        let mut watched = session
            .subscribe(vanilla_call(1.05), conventions(), None, None)
            .await
            .expect("second subscribe opens");

        // Grab a *fresh* live tick line (a snapshot's tokens are retired by the next
        // sequence; a just-arrived tick carries currently-live tokens).
        let _ = baseline(&mut traded).await;
        let line = loop {
            match next_event(&mut traded).await {
                StreamEvent::Tick(line) if line.token_for(Side::Buy).is_some() => break line,
                StreamEvent::Tick(_) | StreamEvent::Heartbeat { .. } => {}
                other => panic!("unexpected event while seeking a tradable tick: {other:?}"),
            }
        };

        // The SDK handles the tradable token: click to BUY (lift the offer).
        let buy_token = line.token_for(Side::Buy).expect("a BUY token");
        let outcome = tokio::time::timeout(STEP_DEADLINE, traded.execute(&line, Side::Buy))
            .await
            .expect("execute resolves in time")
            .expect("execute sends");

        let booked = match outcome {
            ExecuteOutcome::Booked(e) => e,
            ExecuteOutcome::Rejected { reason } => {
                panic!("a click within the window must book, got reject {reason:?}")
            }
        };
        assert_eq!(booked.side, Side::Buy, "BUY lifted the offer");
        // The booked premium is exactly the line's stamped BUY (offer) premium — the
        // SDK booked the clicked price, no re-pricing.
        assert_eq!(
            booked.traded_premium.to_bits(),
            buy_token.premium.to_bits(),
            "booked at the stamped offer premium"
        );
        assert_eq!(
            booked.traded_premium.to_bits(),
            line.price.offer.to_bits(),
            "the BUY token premium is the line offer"
        );
        // The streamed line reprices against a first-principles GK price. The maker
        // bumps spot deterministically each tick (a per-tick fractional move bounded
        // by `STREAM_BUMP`), so the clicked tick's mid sits between the GK price at
        // the lower and upper ends of that spot band — proving the streamed line
        // carries the maker's real GK numbers, not a stub.
        const STREAM_BUMP: f64 = 0.0005;
        let m = live_market();
        let gk = |spot: f64| {
            celnet_vanilla::price(
                OptionType::Call,
                &VanillaInputs::new(spot, strike, line.vol, 1.0, m.r_dom, m.r_for),
            )
        };
        let lo = gk(m.spot * (1.0 - STREAM_BUMP)).min(gk(m.spot * (1.0 + STREAM_BUMP)));
        let hi = gk(m.spot * (1.0 - STREAM_BUMP)).max(gk(m.spot * (1.0 + STREAM_BUMP)));
        let mid = line.price.mid();
        assert!(
            mid >= lo - 1e-6 && mid <= hi + 1e-6,
            "streamed mid {mid} not bracketed by GK over the tick spot band [{lo}, {hi}]"
        );

        // The other subscription is unaffected by the click — still streaming.
        let _ = baseline(&mut watched).await;
        let _ = drive_ticks(&mut watched, 1, 1).await;

        drop(traded);
        drop(watched);
        drop(session);
        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

/// Scenario: a click on a line whose token has aged past its validity window is
/// rejected (last-look), never booked. Driven on a manual clock advanced past the
/// 1-second token window so the staleness is deterministic, not timing-dependent.
#[tokio::test]
async fn stale_click_token_is_rejected_not_booked() {
    tokio::time::timeout(TEST_DEADLINE, async {
        // A manual clock we keep a handle to, so we can age the token past its window.
        let clock = Clock::manual(1_000_000_000);
        let (edge, client) = start_edge_and_client_with(clock.clone()).await;

        let session = tokio::time::timeout(STEP_DEADLINE, client.open_session())
            .await
            .expect("session opens in time")
            .expect("session opens");

        let mut sub = session
            .subscribe(vanilla_call(1.12), conventions(), None, None)
            .await
            .expect("subscribe opens");

        // Capture a tradable line.
        let (snap, _) = baseline(&mut sub).await;
        let line = if snap.token_for(Side::Buy).is_some() {
            snap
        } else {
            // Fall back to the first tradable tick if the snapshot was indicative.
            loop {
                match next_event(&mut sub).await {
                    StreamEvent::Tick(l) if l.token_for(Side::Buy).is_some() => break l,
                    StreamEvent::Tick(_) | StreamEvent::Heartbeat { .. } => {}
                    other => panic!("unexpected event: {other:?}"),
                }
            }
        };

        // Age every token well past its 1-second validity window (the clock never
        // advances on its own), so the click is stale by the time the maker sees it.
        clock.advance(5_000_000_000);

        let outcome = tokio::time::timeout(STEP_DEADLINE, sub.execute(&line, Side::Buy))
            .await
            .expect("execute resolves in time")
            .expect("execute sends");

        // A stale click never books — it is declined (expired window, or the token
        // was already retired by a newer sequence: both are a rejection, not a book).
        match outcome {
            ExecuteOutcome::Rejected { .. } => {}
            ExecuteOutcome::Booked(e) => panic!("a stale click must not book, got {e:?}"),
        }

        drop(sub);
        drop(session);
        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

/// Scenario: a risk manager pulls book-shaped risk (bucketed vega + cross-gamma +
/// theta roll) for a vanilla, and the client's typed `BucketedRisk` equals a
/// first-principles finite-difference decomposition computed directly with
/// `celnet-vanilla`.
#[tokio::test]
async fn risk_manager_pulls_book_shaped_risk_equals_direct_fd() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, client) = start_edge_and_client_with(Clock::system()).await;

        let strike = 1.12;
        let expiry = 1.0;
        let instrument = vanilla_call(strike);
        let m = live_market();
        let base = celnet_client::MarketContext {
            spot: m.spot,
            vol: m.vol,
            r_dom: m.r_dom,
            r_for: m.r_for,
        };

        // The risk manager asks for ATM-pillar vega at the instrument's own tenor, a
        // spot×vol cross-gamma, and an overnight theta roll.
        let request = RiskRequest::vega(vec![(expiry, 0.50)])
            .with_cross_gamma(vec![(
                celnet_client::ShockFactor::Spot,
                celnet_client::ShockFactor::Vol,
            )])
            .with_roll_horizons(vec![1.0 / 365.0]);

        // A trivial single-node grid (no shock) accompanies the risk in one round-trip.
        let no_shock =
            celnet_client::ShockAxis::absolute(celnet_client::ShockFactor::Spot, vec![0.0]);
        let result = tokio::time::timeout(
            STEP_DEADLINE,
            client.scenario_with_risk(&instrument, base, &[no_shock], &request, conventions()),
        )
        .await
        .expect("scenario_with_risk in time")
        .expect("scenario_with_risk ok");

        // ---- bucketed vega: central difference of the GK price in vol -----------
        let h = 1e-4;
        let v_up = celnet_vanilla::price(
            OptionType::Call,
            &VanillaInputs::new(m.spot, strike, m.vol + h, expiry, m.r_dom, m.r_for),
        );
        let v_dn = celnet_vanilla::price(
            OptionType::Call,
            &VanillaInputs::new(m.spot, strike, m.vol - h, expiry, m.r_dom, m.r_for),
        );
        let direct_vega = (v_up - v_dn) / (2.0 * h);
        let client_vega = result
            .risk
            .vega_at(expiry, 0.50)
            .expect("the ATM-pillar vega bucket is present");
        assert!(
            is_close(client_vega, direct_vega, 1e-6, 1e-8),
            "client bucketed vega {client_vega} vs direct FD {direct_vega}"
        );

        // ---- cross-gamma spot×vol: 2-D central stencil of the GK price ----------
        let hs = m.spot * 1e-4;
        let hv = 1e-4;
        let p = |spot: f64, vol: f64| {
            celnet_vanilla::price(
                OptionType::Call,
                &VanillaInputs::new(spot, strike, vol, expiry, m.r_dom, m.r_for),
            )
        };
        let v_pp = p(m.spot + hs, m.vol + hv);
        let v_pm = p(m.spot + hs, m.vol - hv);
        let v_mp = p(m.spot - hs, m.vol + hv);
        let v_mm = p(m.spot - hs, m.vol - hv);
        let direct_cross = (v_pp - v_pm - v_mp + v_mm) / (4.0 * hs * hv);
        let client_cross = result
            .risk
            .cross_gamma(
                celnet_client::ShockFactor::Spot,
                celnet_client::ShockFactor::Vol,
            )
            .expect("the spot×vol cross-gamma term is present");
        assert!(
            is_close(client_cross, direct_cross, 1e-5, 1e-6),
            "client cross-gamma {client_cross} vs direct FD {direct_cross}"
        );

        // ---- theta roll: GK price at the rolled-forward expiry ------------------
        let rolled = expiry - 1.0 / 365.0;
        let direct_roll = celnet_vanilla::price(
            OptionType::Call,
            &VanillaInputs::new(m.spot, strike, m.vol, rolled, m.r_dom, m.r_for),
        );
        assert_eq!(result.risk.theta_roll.len(), 1, "one rolled horizon");
        assert!(
            is_close(result.risk.theta_roll[0], direct_roll, 1e-6, 1e-8),
            "client theta roll {} vs direct {direct_roll}",
            result.risk.theta_roll[0]
        );

        // The accompanying base grid node reprices the unshocked instrument too.
        let base_node = result
            .grid
            .node_with_shocks(&[0.0])
            .expect("the unshocked base node");
        let direct_base = celnet_vanilla::price(
            OptionType::Call,
            &VanillaInputs::new(m.spot, strike, m.vol, expiry, m.r_dom, m.r_for),
        );
        assert!(
            is_close(base_node.greeks.price, direct_base, 1e-6, 1e-8),
            "base grid node price {} vs direct {direct_base}",
            base_node.greeks.price
        );

        let _ = FIXTURE_SPOT;
        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

/// **Streaming-path authentication, end-to-end.** The server now rejects an
/// unauthenticated `StreamSession` under the production deny-by-default posture
/// (`AccessMode::Enforce`, set by the test harness). This proves the SDK closes that
/// seam: it sends an `Authenticate` frame as the FIRST control frame whenever it
/// opens a session, so the subscribe that follows is admitted.
///
/// * **Real Login token.** The taker logs in via the edge's real `AuthService.Login`
///   RPC (the seed admin), attaches the issued bearer with
///   [`Client::with_session_token`], and the SDK authenticates the stream as that
///   user — the token validated against the edge's OWN session registry, the same
///   one `StreamAuth` is checked against.
/// * **Grant-all default.** A client that attaches no token still authenticates: the
///   SDK sends the audited explicit grant-all `Authenticate` frame (parity with the
///   gated risk requests), which `Enforce` admits. Both subscribe and stream.
#[tokio::test]
async fn stream_authenticates_under_enforce_with_login_token_and_grant_all_default() {
    tokio::time::timeout(TEST_DEADLINE, async {
        // Force a fresh seed of the default admin (`admin@celnet.com` / `password`)
        // into the gitignored CWD `identity.json`, so the login below works with the
        // default credential regardless of any prior run's rotated password. Safe
        // under the gate's serial (`--test-threads 1`) execution.
        let _ = std::fs::remove_file("identity.json");
        let (edge, addr) = start_ready_edge(Clock::system()).await;

        // ---- (a) a real Login-issued session token authenticates the stream -------
        let token = login_seed_admin(addr).await;
        let authed = tokio::time::timeout(STEP_DEADLINE, Client::connect(format!("http://{addr}")))
            .await
            .expect("client connects in time")
            .expect("client connects")
            .with_session_token(token);

        let session = tokio::time::timeout(STEP_DEADLINE, authed.open_session())
            .await
            .expect("session opens in time")
            .expect("session opens");
        let mut sub = tokio::time::timeout(
            STEP_DEADLINE,
            session.subscribe(vanilla_call(1.12), conventions(), None, None),
        )
        .await
        .expect("subscribe resolves in time")
        .expect("an authenticated subscribe is admitted under Enforce");
        // A baseline snapshot proves the Authenticate frame led and the subscribe
        // was admitted — an anonymous subscribe under Enforce would have errored here.
        match next_event(&mut sub).await {
            StreamEvent::Snapshot { line, .. } => assert_eq!(line.sequence, 1),
            other => panic!("expected an admitted Snapshot, got {other:?}"),
        }
        drop(sub);
        drop(session);

        // ---- (b) the grant-all default (no token) is also admitted under Enforce --
        let default_client =
            tokio::time::timeout(STEP_DEADLINE, Client::connect(format!("http://{addr}")))
                .await
                .expect("client connects in time")
                .expect("client connects");
        let session2 = tokio::time::timeout(STEP_DEADLINE, default_client.open_session())
            .await
            .expect("session opens in time")
            .expect("session opens");
        let mut sub2 = tokio::time::timeout(
            STEP_DEADLINE,
            session2.subscribe(vanilla_call(1.10), conventions(), None, None),
        )
        .await
        .expect("subscribe resolves in time")
        .expect("the grant-all default Authenticate admits the subscribe under Enforce");
        match next_event(&mut sub2).await {
            StreamEvent::Snapshot { line, .. } => assert_eq!(line.sequence, 1),
            other => panic!("expected an admitted Snapshot, got {other:?}"),
        }

        drop(sub2);
        drop(session2);
        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}
