//! Curve-query parity on the `SurfaceService` (`curve-surface-query`).
//!
//! Fixed income now rides the SAME market-data QUERY surface the FX vol surface has
//! (ADR-0021): `GetCurve` / `MarkCurve` / `CurveScenario` mirror the vol
//! `GetSmile` / `MarkSurface` / `Scenario` verbs on the discount-curve domain. These
//! tests exercise all three end to end over WS + gRPC and prove:
//!
//! 1. **codec round-trip** — a `GetCurve` / `MarkCurve` / `CurveScenario` JSON body
//!    decodes through the descriptor-driven generated codec field-for-field, and each
//!    reply re-encodes from the field tables (no FX-legacy quirks);
//! 2. **`GetCurve` == bootstrap** — the queried zero rates / discount factors equal
//!    an INDEPENDENTLY-bootstrapped `celnet-rates` curve (built here from the public
//!    `usd_sofr_ois_schedule` + `bootstrap_ois`, NOT by re-invoking the query) for the
//!    same market inputs, to `<= 1e-12`;
//! 3. **`MarkCurve` -> `GetCurve` pin round-trip** — a `GetCurve` pinned to a marked
//!    version reproduces the marked curve BIT-FOR-BIT (the FI analogue of the pinned
//!    surface), and equals the independent bootstrap;
//! 4. **`CurveScenario` shift** — a +Δbp parallel shift raises zero rates / lowers
//!    discount factors, and a repriced instrument's PV moves by `~ dv01 * Δbp`,
//!    sign-correct;
//! 5. **par-swap self-consistency** — the bootstrapped curve reprices each
//!    calibrating par swap to par (an independent mathematical anchor for the
//!    numbers, unrelated to the query surface).
//!
//! Every body is hard wall-clock bounded and every socket await is bounded.

mod common;

use std::time::Duration;

use celnet_proto::surface_service_client::SurfaceServiceClient;
use celnet_proto::{
    BrokenDate, CurveScenarioRequest, CurveSet, GetCurveRequest, MarkCurveRequest, OisInstrument,
    OisPillar, PillarTenor, RatesInstrument, RatesPriceRequest, Side, pillar_tenor,
    rates_instrument,
};
use celnet_rates::{OisQuote, bootstrap_ois, usd_sofr_ois_schedule};
use celnet_server::rates_pricing::price_rates;
use celnet_server::ws::generated_codec;
use celnet_types::{Rate, Time};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use time::{Date, Month};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message as WsMessage;

use common::{TEST_DEADLINE, start_ready_edge};

const STEP: Duration = Duration::from_secs(20);
const NOTIONAL: f64 = 100_000_000.0;

// ---------------------------------------------------------------------------
// shared USD-SOFR ladder — the same market built as the wire JSON, as the proto
// `CurveSet`, and (independently) as a directly-bootstrapped `celnet-rates` curve.
// ---------------------------------------------------------------------------

/// The calibrating pillars `(whole-year tenor, par rate)`.
const PILLARS: [(u32, f64); 4] = [(1, 0.0420), (2, 0.0410), (5, 0.0405), (10, 0.0415)];
const REF_YEAR: i32 = 2026;
const REF_MONTH: u32 = 6;
const REF_DAY: u32 = 25;

fn curve_json() -> Value {
    json!({
        "currency": "USD",
        "reference_date": { "year": REF_YEAR, "month": REF_MONTH, "day": REF_DAY },
        "ois_pillars": PILLARS.iter().map(|&(y, p)| json!({
            "tenor": { "years": y },
            "par_rate": p
        })).collect::<Vec<_>>()
    })
}

fn years_pillar(years: u32, par_rate: f64) -> OisPillar {
    OisPillar {
        tenor: Some(PillarTenor {
            point: Some(pillar_tenor::Point::Years(years)),
        }),
        par_rate,
    }
}

fn curve_proto() -> CurveSet {
    CurveSet {
        currency: "USD".to_owned(),
        reference_date: Some(BrokenDate {
            year: REF_YEAR,
            month: REF_MONTH,
            day: REF_DAY,
        }),
        ois_pillars: PILLARS.iter().map(|&(y, p)| years_pillar(y, p)).collect(),
    }
}

fn ois_proto(side: Side) -> OisInstrument {
    OisInstrument {
        tenor_years: 5,
        fixed_rate: 0.04,
        notional: NOTIONAL,
        side: side as i32,
    }
}

fn ois_json() -> Value {
    json!({ "ois": { "tenor_years": 5, "fixed_rate": 0.04, "notional": NOTIONAL, "side": 0 } })
}

/// The INDEPENDENT reference curve: bootstrapped directly from the public
/// `celnet-rates` primitives (`usd_sofr_ois_schedule` + `bootstrap_ois`) — NOT by
/// invoking the `GetCurve` query. The `SurfaceService` bootstraps from the SAME
/// schedule builder, so a faithful query reproduces this curve exactly. (The
/// bootstrap MATH itself is anchored to QuantLib in `celnet-rates`'s own oracle
/// suite; this is the query-faithfulness anchor.)
fn oracle_curve(pillars: &[(u32, f64)]) -> celnet_rates::Curve {
    let reference = Date::from_calendar_date(REF_YEAR, Month::June, REF_DAY as u8)
        .expect("valid reference date");
    let quotes: Vec<OisQuote> = pillars
        .iter()
        .map(|&(y, p)| OisQuote {
            schedule: usd_sofr_ois_schedule(reference, y).expect("valid schedule"),
            par_rate: Rate(p),
        })
        .collect();
    bootstrap_ois(&quotes).expect("bootstraps")
}

async fn send_json<S>(ws: &mut S, v: Value)
where
    S: SinkExt<WsMessage, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
{
    tokio::time::timeout(STEP, ws.send(WsMessage::Text(v.to_string())))
        .await
        .expect("the send completes in time")
        .expect("the send succeeds");
}

async fn next_json<S>(ws: &mut S) -> Value
where
    S: StreamExt<Item = Result<WsMessage, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    loop {
        let frame = tokio::time::timeout(STEP, ws.next())
            .await
            .expect("a WS frame arrives before the deadline")
            .expect("the socket stays open")
            .expect("the frame is well-formed");
        match frame {
            WsMessage::Text(t) => {
                let v: Value = serde_json::from_str(&t).expect("frame is valid JSON");
                // Skip the unsolicited connect-time firm-wide pricing-control frame.
                if v.get("type").and_then(Value::as_str) == Some("pricing_control") {
                    continue;
                }
                return v;
            }
            WsMessage::Ping(_) | WsMessage::Pong(_) => continue,
            other => panic!("unexpected non-text WS frame: {other:?}"),
        }
    }
}

// ---------------------------------------------------------------------------
// (1) codec conformance — decode requests + encode replies via the generated codec
// ---------------------------------------------------------------------------

/// A `GetCurve` wire body decodes through the generated codec field-for-field.
#[test]
fn get_curve_request_decodes_field_for_field() {
    let body = json!({
        "type": "get_curve",
        "curve_set": curve_json(),
        "query_tenor_years": [1.0, 2.0, 5.0, 10.0],
        "curve_version": 7
    });
    let o = body.as_object().expect("request object");
    let req = generated_codec::decode_get_curve_request(o).expect("decodes");

    assert_eq!(req.curve_set.as_ref(), Some(&curve_proto()));
    assert_eq!(req.query_tenor_years, vec![1.0, 2.0, 5.0, 10.0]);
    assert_eq!(req.curve_version, Some(7));
}

/// A `GetCurve` body WITHOUT a pin decodes with `curve_version = None` (the live
/// inline-bootstrap case) — proto3 presence, not a defaulted zero.
#[test]
fn get_curve_request_without_pin_has_no_version() {
    let body =
        json!({ "type": "get_curve", "curve_set": curve_json(), "query_tenor_years": [1.0] });
    let o = body.as_object().expect("request object");
    let req = generated_codec::decode_get_curve_request(o).expect("decodes");
    assert_eq!(req.curve_version, None);
    assert!(req.curve_set.is_some());
}

/// A `CurveScenario` wire body decodes its shift axes + instrument field-for-field.
#[test]
fn curve_scenario_request_decodes_field_for_field() {
    let body = json!({
        "type": "curve_scenario",
        "curve_set": curve_json(),
        "parallel_shift_bp": 25.0,
        "key_rate_shift_bp": [1.0, 2.0, 3.0, 4.0],
        "query_tenor_years": [2.0, 5.0],
        "instrument": ois_json()
    });
    let o = body.as_object().expect("request object");
    let req = generated_codec::decode_curve_scenario_request(o).expect("decodes");

    assert_eq!(req.parallel_shift_bp, 25.0);
    assert_eq!(req.key_rate_shift_bp, vec![1.0, 2.0, 3.0, 4.0]);
    assert_eq!(req.query_tenor_years, vec![2.0, 5.0]);
    assert_eq!(
        req.instrument,
        Some(RatesInstrument {
            instrument: Some(rates_instrument::Instrument::Ois(ois_proto(Side::Buy))),
        })
    );
}

/// The `GetCurveResponse` re-encodes from the field tables to the expected WS JSON
/// shape (currency, reference date, points, par pillars) — descriptor-driven.
#[test]
fn get_curve_response_encodes_field_tables() {
    let resp = celnet_proto::GetCurveResponse {
        currency: "USD".to_owned(),
        reference_date: Some(BrokenDate {
            year: REF_YEAR,
            month: REF_MONTH,
            day: REF_DAY,
        }),
        points: vec![celnet_proto::CurvePoint {
            tenor_years: 1.0,
            zero_rate: 0.041,
            discount_factor: 0.96,
        }],
        par_pillars: vec![celnet_proto::CurveParPillar {
            tenor_years: 1.0,
            par_rate: 0.042,
        }],
        curve_version: Some(3),
        epoch_nanos: 123,
    };
    let j = generated_codec::encode_get_curve_response(&resp);
    assert_eq!(j["currency"], json!("USD"));
    assert_eq!(j["reference_date"]["year"], json!(REF_YEAR));
    assert_eq!(j["points"][0]["tenor_years"], json!(1.0));
    assert_eq!(j["points"][0]["discount_factor"], json!(0.96));
    assert_eq!(j["par_pillars"][0]["par_rate"], json!(0.042));
    assert_eq!(j["curve_version"], json!(3));
    assert_eq!(j["epoch_nanos"], json!(123));
}

// ---------------------------------------------------------------------------
// (2) GetCurve == independent bootstrap, over WS
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ws_get_curve_matches_independent_bootstrap() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, _grpc, _dir) = start_ready_edge().await;
        let url = format!("ws://{}", edge.ws_addr());
        let (mut ws, _resp) = tokio::time::timeout(STEP, connect_async(url))
            .await
            .expect("WS connects in time")
            .expect("WS connects");

        let tenors = [0.5, 1.0, 2.0, 5.0, 10.0];
        send_json(
            &mut ws,
            json!({
                "type": "get_curve",
                "curve_set": curve_json(),
                "query_tenor_years": tenors
            }),
        )
        .await;
        let resp = next_json(&mut ws).await;
        assert_eq!(resp["type"], json!("get_curve_response"));
        assert_eq!(resp["currency"], json!("USD"));
        // No pin on a live inline read.
        assert!(resp.get("curve_version").is_none_or(Value::is_null));

        let curve = oracle_curve(&PILLARS);
        let points = resp["points"].as_array().expect("points array");
        assert_eq!(points.len(), tenors.len());
        for (point, &t) in points.iter().zip(tenors.iter()) {
            assert_eq!(point["tenor_years"].as_f64().unwrap(), t);
            let df = point["discount_factor"].as_f64().unwrap();
            let z = point["zero_rate"].as_f64().unwrap();
            let df_ref = curve.discount_factor(Time(t)).0;
            let z_ref = curve.zero_rate(Time(t)).0;
            assert!(
                (df - df_ref).abs() <= 1e-12,
                "GetCurve DF({t}) {df} vs independent bootstrap {df_ref}"
            );
            assert!(
                (z - z_ref).abs() <= 1e-12,
                "GetCurve zero({t}) {z} vs independent bootstrap {z_ref}"
            );
        }

        // The echoed par pillars carry the calibrating par rates.
        let par = resp["par_pillars"].as_array().expect("par pillars");
        assert_eq!(par.len(), PILLARS.len());
        for (echoed, &(_, rate)) in par.iter().zip(PILLARS.iter()) {
            assert!((echoed["par_rate"].as_f64().unwrap() - rate).abs() <= 1e-12);
        }

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

// ---------------------------------------------------------------------------
// (3) MarkCurve -> GetCurve pin round-trip, bit-for-bit
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ws_mark_then_get_pinned_curve_round_trips_bit_for_bit() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, _grpc, _dir) = start_ready_edge().await;
        let url = format!("ws://{}", edge.ws_addr());
        let (mut ws, _resp) = tokio::time::timeout(STEP, connect_async(url))
            .await
            .expect("WS connects in time")
            .expect("WS connects");

        // Mark the curve; capture the assigned version + the marked points.
        send_json(
            &mut ws,
            json!({ "type": "mark_curve", "curve_set": curve_json() }),
        )
        .await;
        let marked = next_json(&mut ws).await;
        assert_eq!(marked["type"], json!("mark_curve_response"));
        let version = marked["curve_version"].as_u64().expect("version");
        assert!(
            version >= 1,
            "version is the monotonic mark authority (>= 1)"
        );
        let marked_points = marked["points"].as_array().expect("marked points").clone();
        assert_eq!(marked_points.len(), PILLARS.len());

        // Query the pinned version at the EXACT marked pillar tenors.
        let tenors: Vec<f64> = marked_points
            .iter()
            .map(|p| p["tenor_years"].as_f64().unwrap())
            .collect();
        send_json(
            &mut ws,
            json!({
                "type": "get_curve",
                "curve_version": version,
                "query_tenor_years": tenors
            }),
        )
        .await;
        let pinned = next_json(&mut ws).await;
        assert_eq!(pinned["type"], json!("get_curve_response"));
        assert_eq!(pinned["curve_version"].as_u64(), Some(version));

        // Bit-for-bit: the pinned read reproduces the marked curve exactly, and both
        // equal the independent bootstrap.
        let curve = oracle_curve(&PILLARS);
        let pinned_points = pinned["points"].as_array().expect("pinned points");
        assert_eq!(pinned_points.len(), marked_points.len());
        for (pinned_pt, marked_pt) in pinned_points.iter().zip(marked_points.iter()) {
            let (pz, pdf) = (
                pinned_pt["zero_rate"].as_f64().unwrap(),
                pinned_pt["discount_factor"].as_f64().unwrap(),
            );
            let (mz, mdf) = (
                marked_pt["zero_rate"].as_f64().unwrap(),
                marked_pt["discount_factor"].as_f64().unwrap(),
            );
            assert_eq!(
                pz.to_bits(),
                mz.to_bits(),
                "pinned zero {pz} must be bit-identical to marked {mz}"
            );
            assert_eq!(
                pdf.to_bits(),
                mdf.to_bits(),
                "pinned DF {pdf} must be bit-identical to marked {mdf}"
            );
            let t = marked_pt["tenor_years"].as_f64().unwrap();
            assert!((pdf - curve.discount_factor(Time(t)).0).abs() <= 1e-12);
        }

        // An unknown pin cannot be honoured.
        send_json(
            &mut ws,
            json!({ "type": "get_curve", "curve_version": 999_999, "query_tenor_years": [1.0] }),
        )
        .await;
        let err = next_json(&mut ws).await;
        assert_eq!(err["type"], json!("error"), "unknown pin is an error frame");

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

// ---------------------------------------------------------------------------
// (4) CurveScenario shift + reprice, over WS
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ws_curve_scenario_parallel_shift_and_reprice() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, _grpc, _dir) = start_ready_edge().await;
        let url = format!("ws://{}", edge.ws_addr());
        let (mut ws, _resp) = tokio::time::timeout(STEP, connect_async(url))
            .await
            .expect("WS connects in time")
            .expect("WS connects");

        let shift_bp = 25.0;
        let tenors = [1.0, 2.0, 5.0, 10.0];
        send_json(
            &mut ws,
            json!({
                "type": "curve_scenario",
                "curve_set": curve_json(),
                "parallel_shift_bp": shift_bp,
                "query_tenor_years": tenors,
                "instrument": ois_json()
            }),
        )
        .await;
        let resp = next_json(&mut ws).await;
        assert_eq!(resp["type"], json!("curve_scenario_response"));

        // A +25bp parallel shift raises zero rates (~ +0.0025) and lowers discount
        // factors at every queried tenor, relative to the base bootstrap.
        let base = oracle_curve(&PILLARS);
        let points = resp["points"].as_array().expect("points");
        for (point, &t) in points.iter().zip(tenors.iter()) {
            let z_shift = point["zero_rate"].as_f64().unwrap();
            let df_shift = point["discount_factor"].as_f64().unwrap();
            let z_base = base.zero_rate(Time(t)).0;
            let df_base = base.discount_factor(Time(t)).0;
            assert!(
                z_shift > z_base,
                "shifted zero {z_shift} must exceed base {z_base} at {t}"
            );
            assert!(
                df_shift < df_base,
                "shifted DF {df_shift} must be below base {df_base} at {t}"
            );
            // The parallel par shift maps to ~ a parallel zero shift; within 3bp.
            assert!(
                ((z_shift - z_base) - shift_bp * 1e-4).abs() < 3e-4,
                "zero shift {} ~ {}bp at {t}",
                z_shift - z_base,
                shift_bp
            );
        }

        // The repriced OIS PV moves by ~ dv01 * shift_bp, sign-correct.
        let reprice = &resp["reprice"];
        let base_pv = reprice["base_pv"].as_f64().unwrap();
        let shifted_pv = reprice["shifted_pv"].as_f64().unwrap();
        let pv_change = reprice["pv_change"].as_f64().unwrap();
        let dv01 = reprice["dv01"].as_f64().unwrap();
        assert!((pv_change - (shifted_pv - base_pv)).abs() <= 1e-6);
        assert!(dv01.abs() > 0.0, "a 5Y OIS has a non-zero dv01");
        let predicted = dv01 * shift_bp;
        assert!(
            pv_change.signum() == predicted.signum(),
            "pv_change {pv_change} sign must match dv01*shift {predicted}"
        );
        assert!(
            (pv_change - predicted).abs() <= 0.05 * predicted.abs(),
            "pv_change {pv_change} ~ dv01*shift {predicted} to first order"
        );

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

// ---------------------------------------------------------------------------
// (5) gRPC parity + an independent par-swap self-consistency anchor
// ---------------------------------------------------------------------------

/// gRPC `GetCurve` pinned to a `MarkCurve`d version reproduces the marked curve, over
/// the tonic transport (the canonical API, second encoding).
#[tokio::test]
async fn grpc_mark_then_get_pinned_curve() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, addr, _dir) = start_ready_edge().await;
        let mut client = tokio::time::timeout(
            STEP,
            SurfaceServiceClient::connect(format!("http://{addr}")),
        )
        .await
        .expect("client connects in time")
        .expect("client connects");

        let marked = tokio::time::timeout(
            STEP,
            client.mark_curve(MarkCurveRequest {
                curve_set: Some(curve_proto()),
            }),
        )
        .await
        .expect("mark returns in time")
        .expect("mark succeeds")
        .into_inner();
        assert!(marked.curve_version >= 1);
        assert_eq!(marked.points.len(), PILLARS.len());

        let tenors: Vec<f64> = marked.points.iter().map(|p| p.tenor_years).collect();
        let pinned = tokio::time::timeout(
            STEP,
            client.get_curve(GetCurveRequest {
                curve_set: None,
                query_tenor_years: tenors,
                curve_version: Some(marked.curve_version),
                curve_id: None,
            }),
        )
        .await
        .expect("get returns in time")
        .expect("get succeeds")
        .into_inner();
        assert_eq!(pinned.curve_version, Some(marked.curve_version));

        for (p, m) in pinned.points.iter().zip(marked.points.iter()) {
            assert_eq!(p.zero_rate.to_bits(), m.zero_rate.to_bits());
            assert_eq!(p.discount_factor.to_bits(), m.discount_factor.to_bits());
        }

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

/// gRPC `CurveScenario` with an empty shift reprices the instrument to the base PV,
/// and the bootstrapped curve reprices each calibrating par swap to par — an
/// independent mathematical anchor for the curve numbers (a par swap struck at the
/// pillar's par rate has ~ zero PV), unrelated to the query surface.
#[tokio::test]
async fn grpc_curve_bootstrap_reprices_pillars_to_par() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, addr, _dir) = start_ready_edge().await;
        let mut client = tokio::time::timeout(
            STEP,
            SurfaceServiceClient::connect(format!("http://{addr}")),
        )
        .await
        .expect("client connects in time")
        .expect("client connects");

        // A CurveScenario with no shift returns the base curve; its repriced OIS PV
        // equals a direct base price_rates (the reprice rides the one FI path).
        let scenario = tokio::time::timeout(
            STEP,
            client.curve_scenario(CurveScenarioRequest {
                curve_set: Some(curve_proto()),
                parallel_shift_bp: 0.0,
                key_rate_shift_bp: vec![],
                query_tenor_years: vec![5.0],
                instrument: Some(RatesInstrument {
                    instrument: Some(rates_instrument::Instrument::Ois(ois_proto(Side::Sell))),
                }),
            }),
        )
        .await
        .expect("scenario returns in time")
        .expect("scenario succeeds")
        .into_inner();
        let reprice = scenario.reprice.expect("carries a repriced leg");
        assert!(
            (reprice.pv_change).abs() <= 1e-9,
            "no shift => no PV change"
        );
        let direct = price_rates(&RatesPriceRequest {
            request_id: 0,
            curve_set: Some(curve_proto()),
            instrument: Some(RatesInstrument {
                instrument: Some(rates_instrument::Instrument::Ois(ois_proto(Side::Sell))),
            }),
            correlation_id: None,
        })
        .expect("direct prices")
        .pv;
        assert!(
            (reprice.base_pv - direct).abs() <= 1e-9,
            "scenario base_pv {} vs direct price_rates {direct}",
            reprice.base_pv
        );

        // Independent anchor: for each pillar, an OIS struck at the pillar's par rate
        // prices to ~ par (zero PV) on the bootstrapped curve.
        for &(tenor, par) in &PILLARS {
            let pv = price_rates(&RatesPriceRequest {
                request_id: 0,
                curve_set: Some(curve_proto()),
                instrument: Some(RatesInstrument {
                    instrument: Some(rates_instrument::Instrument::Ois(OisInstrument {
                        tenor_years: tenor,
                        fixed_rate: par,
                        notional: NOTIONAL,
                        side: Side::Sell as i32,
                    })),
                }),
                correlation_id: None,
            })
            .expect("prices")
            .pv;
            assert!(
                pv.abs() <= 1e-6 * NOTIONAL,
                "par OIS at {tenor}Y ({par}) must reprice to ~par, got PV {pv}"
            );
        }

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}
