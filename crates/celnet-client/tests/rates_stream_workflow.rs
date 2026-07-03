//! Live-loopback conformance for the linear-rates **streaming** SDK surface
//! (`Client::subscribe_rates` / `StreamSession::subscribe_rates`) — the client-side
//! parity of the landed `rates-stream-ws` server feed (`RatesSubscribe` →
//! `RatesStreamSnapshot` + sequenced `RatesStreamUpdate`s over the multiplexed
//! `StreamService`).
//!
//! Each test boots a real in-process `celnet-server` edge under the production
//! `Enforce` posture, authenticates as the seed admin (the `RatesSubscribe` frame is
//! capability-gated to `Stream·FixedIncome`), opens a live rates line, and asserts
//! two model-independent properties — never a re-run of the pricing engine as its own
//! oracle:
//!
//! 1. **Baseline agrees with the request/response price.** The opening
//!    `RatesStreamSnapshot` event's priced result equals the SAME client's
//!    `price_rates(instrument)` bit-for-bit — the stream's baseline is the faithful
//!    one-shot price (the server prices it through the identical landed path).
//! 2. **A live tick reprices consistently with its curve shift.** At least one
//!    `RatesStreamUpdate` arrives carrying a non-zero `curve_shift`, and its repriced
//!    PV moves in the direction the baseline `dv01` dictates for that shift, with the
//!    move matching the reported `dv01` to first order (`ΔPV ≈ dv01 · shift / 1bp`).
//!    This ties the streamed shift, the repriced PV, and the snapshot's reported risk
//!    together by a calculus identity — it never re-prices the instrument client-side.
//!
//! A third test proves the ADR-0021 generalization: an FX price line and an FI rates
//! line multiplexed on ONE `StreamSession` each receive their own baseline, demuxed
//! by `SubscriptionId` — the FI frame is never mis-routed onto the FX line, nor
//! dropped.
//!
//! Every body is hard wall-clock bounded and every stream await is itself bounded, so
//! a regression fails fast, never hangs.

mod common;

use std::time::Instant;

use celnet_client::{
    CivilDate, Ois, RatesStreamEvent, StreamEvent, UsdSofrCurve, rates::RatesPriced,
};

use common::{
    STEP_DEADLINE, TEST_DEADLINE, conventions, start_edge_and_authed_client, vanilla_call,
};

/// One basis point in decimal — the unit the reported `dv01` is expressed per.
const ONE_BP: f64 = 1.0e-4;

/// A calibrated USD-SOFR curve (whole-year par-OIS pillars) — the same known-good
/// market shape the request/response rates tests price against (reference
/// 2026-06-25; pillars 1y/2y/5y/10y), so the line calibrates cleanly.
fn curve() -> UsdSofrCurve {
    UsdSofrCurve::new(CivilDate::new(2026, 6, 25))
        .pillar(1, 0.0420)
        .pillar(2, 0.0410)
        .pillar(5, 0.0405)
        .pillar(10, 0.0415)
}

/// Bit-for-bit equality of two priced rates results — the stream baseline must be the
/// faithful one-shot price, not merely "close".
fn assert_priced_bit_eq(got: &RatesPriced, want: &RatesPriced, ctx: &str) {
    assert_eq!(got.pv.to_bits(), want.pv.to_bits(), "{ctx}: pv");
    assert_eq!(
        got.par_rate.to_bits(),
        want.par_rate.to_bits(),
        "{ctx}: par_rate"
    );
    assert_eq!(got.pv01.to_bits(), want.pv01.to_bits(), "{ctx}: pv01");
    assert_eq!(got.dv01.to_bits(), want.dv01.to_bits(), "{ctx}: dv01");
    assert_eq!(
        got.key_rate_ladder.len(),
        want.key_rate_ladder.len(),
        "{ctx}: ladder length"
    );
    for (i, (g, w)) in got
        .key_rate_ladder
        .iter()
        .zip(want.key_rate_ladder.iter())
        .enumerate()
    {
        assert_eq!(g.to_bits(), w.to_bits(), "{ctx}: ladder[{i}]");
    }
}

/// The opening event of a rates line MUST be its baseline snapshot; return the
/// baseline line and the echoed correlation id.
async fn expect_baseline(
    line: &mut celnet_client::RatesSubscription,
) -> (celnet_client::RatesLine, Option<u64>) {
    let event = tokio::time::timeout(STEP_DEADLINE, line.next_event())
        .await
        .expect("the baseline snapshot arrives within the step deadline")
        .expect("the rates line yields a first event")
        .expect("the first event decodes (no server status)");
    match event {
        RatesStreamEvent::Snapshot {
            line,
            correlation_id,
        } => (line, correlation_id),
        RatesStreamEvent::Update(l) => {
            panic!("the first rates event must be the baseline snapshot, got Update {l:?}")
        }
    }
}

/// **The baseline agrees with `price_rates`.** A live rates line's opening snapshot
/// carries exactly the priced result the request/response `price_rates` returns for
/// the same instrument + curve — the stream's baseline is the faithful one-shot
/// price, bit-for-bit, at `curve_shift == 0`.
#[tokio::test]
async fn rates_stream_baseline_equals_price_rates() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (_edge, client, _dir) = start_edge_and_authed_client().await;
        let curve = curve();
        let instrument = Ois::receive_fixed(5, 0.04).notional(100_000_000.0);

        // The request/response one-shot price — the independent baseline the stream
        // must reproduce.
        let priced = tokio::time::timeout(STEP_DEADLINE, client.price_rates(&curve, &instrument))
            .await
            .expect("price_rates resolves in time")
            .expect("price_rates succeeds");

        let mut line =
            tokio::time::timeout(STEP_DEADLINE, client.subscribe_rates(&curve, &instrument))
                .await
                .expect("the subscribe resolves in time")
                .expect("the rates line opens");

        let (baseline, _corr) = expect_baseline(&mut line).await;
        assert_eq!(baseline.sequence, 1, "the baseline is sequence 1");
        assert_eq!(
            baseline.curve_shift.to_bits(),
            0.0f64.to_bits(),
            "the baseline is unshifted"
        );
        assert_priced_bit_eq(&baseline.priced, &priced, "stream baseline vs price_rates");
    })
    .await
    .expect("the rates-stream baseline test completes within the deadline");
}

/// **A live tick reprices consistently with its shift.** As the pricing curve
/// deterministically evolves, at least one `RatesStreamUpdate` arrives carrying a
/// non-zero `curve_shift`, and its repriced PV moves in the direction the baseline
/// `dv01` dictates for that shift, matching `dv01` to first order — a calculus
/// identity between the streamed shift, the repriced PV, and the reported risk, never
/// a client-side re-price of the instrument.
#[tokio::test]
async fn rates_stream_ticks_reprice_consistently_with_the_shift() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (_edge, client, _dir) = start_edge_and_authed_client().await;
        let curve = curve();
        // A receiver OIS: `dv01 < 0` (long duration), so an upward shift lowers PV —
        // the check reads the direction off the reported `dv01`, valid for either side.
        let instrument = Ois::receive_fixed(5, 0.04).notional(100_000_000.0);

        let mut line = tokio::time::timeout(
            STEP_DEADLINE,
            client.subscribe_rates(&curve, &instrument),
        )
        .await
        .expect("the subscribe resolves in time")
        .expect("the rates line opens");

        let (baseline, _corr) = expect_baseline(&mut line).await;
        let pv0 = baseline.priced.pv;
        let dv01 = baseline.priced.dv01;
        assert!(
            dv01.abs() > 0.0,
            "a 5y OIS has a non-zero curve DV01 to steer the sign check"
        );

        // Consume updates until one carries a materially non-zero shift (the fan-out's
        // deterministic parallel shift is bounded by ±1bp; a near-zero draw is rare and
        // simply skipped so the sign check has signal). Bounded by the overall test
        // deadline; each await is itself bounded.
        let hard_stop = Instant::now() + STEP_DEADLINE;
        let mut checked = false;
        while Instant::now() < hard_stop {
            let event = tokio::time::timeout(STEP_DEADLINE, line.next_event())
                .await
                .expect("an update arrives within the step deadline")
                .expect("the rates line keeps yielding")
                .expect("the update decodes (no server status)");
            let RatesStreamEvent::Update(l) = event else {
                panic!("post-baseline events must be Updates, got a second Snapshot");
            };
            assert!(l.sequence >= 2, "an update advances past the baseline");
            let shift = l.curve_shift;
            if shift.abs() < 1.0e-6 {
                continue; // a near-zero shift carries no directional signal; skip.
            }

            let actual_dpv = l.priced.pv - pv0;
            // First-order prediction from the reported baseline DV01 (per +1bp).
            let predicted_dpv = dv01 * shift / ONE_BP;
            assert!(
                predicted_dpv != 0.0 && actual_dpv != 0.0,
                "a non-zero shift genuinely reprices the line (shift {shift}, ΔPV {actual_dpv})"
            );
            // Direction: the repriced PV moves the way the reported DV01 dictates — a
            // positive product means `ΔPV` and `dv01·shift` share a sign.
            assert!(
                actual_dpv * predicted_dpv > 0.0,
                "ΔPV {actual_dpv} must match the sign of dv01·shift {predicted_dpv} (shift {shift})"
            );
            // Magnitude: first-order agreement over a ≤1bp parallel shift (the residual
            // is second-order curve convexity, negligible at this scale).
            assert!(
                (actual_dpv - predicted_dpv).abs() <= predicted_dpv.abs() * 5.0e-2 + 1.0e-6,
                "ΔPV {actual_dpv} must match the DV01 prediction {predicted_dpv} to first order (shift {shift})"
            );
            checked = true;
            break;
        }
        assert!(
            checked,
            "at least one non-zero-shift update must arrive to validate repricing"
        );
    })
    .await
    .expect("the rates-stream tick test completes within the deadline");
}

/// **ADR-0021 generalization.** An FX price line and an FI rates line multiplexed on
/// ONE `StreamSession` each receive their own baseline, demuxed by `SubscriptionId`:
/// the FI snapshot equals `price_rates` (so the FI frame routed to the rates line,
/// never onto the FX line), and the FX line receives an FX `Snapshot` (so the FX
/// frame is not dropped or mis-routed onto the rates line). One session, one
/// connection, both asset classes.
#[tokio::test]
async fn fx_and_rates_lines_multiplex_on_one_session() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (_edge, client, _dir) = start_edge_and_authed_client().await;
        let curve = curve();
        let instrument = Ois::receive_fixed(5, 0.04).notional(100_000_000.0);

        let priced = tokio::time::timeout(STEP_DEADLINE, client.price_rates(&curve, &instrument))
            .await
            .expect("price_rates resolves in time")
            .expect("price_rates succeeds");

        // One session carrying BOTH asset classes — the generalized streaming seam.
        let session = tokio::time::timeout(STEP_DEADLINE, client.open_session())
            .await
            .expect("the session opens in time")
            .expect("the session opens");

        // An FX price line on the same session.
        let mut fx = tokio::time::timeout(
            STEP_DEADLINE,
            session.subscribe(vanilla_call(1.10), conventions(), None, None),
        )
        .await
        .expect("the FX subscribe resolves in time")
        .expect("the FX line opens");

        // An FI rates line on the same session, with an echoed correlation id.
        let mut rates = tokio::time::timeout(
            STEP_DEADLINE,
            session.subscribe_rates(&curve, &instrument, Some(7), 0),
        )
        .await
        .expect("the rates subscribe resolves in time")
        .expect("the rates line opens");

        // The rates baseline routed to the rates line (not the FX line) and equals the
        // faithful one-shot price; the correlation id is echoed.
        let (baseline, corr) = expect_baseline(&mut rates).await;
        assert_eq!(corr, Some(7), "the opening correlation id is echoed");
        assert_priced_bit_eq(&baseline.priced, &priced, "multiplexed rates baseline");

        // The FX baseline routed to the FX line (not the rates line, not dropped).
        let fx_event = tokio::time::timeout(STEP_DEADLINE, fx.next_event())
            .await
            .expect("the FX snapshot arrives within the step deadline")
            .expect("the FX line yields a first event")
            .expect("the FX event decodes (no server status)");
        assert!(
            matches!(fx_event, StreamEvent::Snapshot { .. }),
            "the FX line receives its own Snapshot, demuxed by SubscriptionId, got {fx_event:?}"
        );
    })
    .await
    .expect("the multiplexed FX+rates test completes within the deadline");
}
