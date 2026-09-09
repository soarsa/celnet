//! ValuationService integration tests: testing the unified polymorphic calculation RPC
//! `ValuationService.Calculate(ValuationRequest) -> ValuationResponse` for options and linear rates.

mod common;

use celnet_proto::valuation_request::Payload as ReqPayload;
use celnet_proto::valuation_response::Payload as RespPayload;
use celnet_proto::valuation_service_client::ValuationServiceClient;
use celnet_proto::{PriceRequest, ValuationRequest};
use common::{
    STEP_DEADLINE, TEST_DEADLINE, live_market, start_ready_edge, vanilla_call, wire_conventions,
};

#[tokio::test]
async fn valuation_service_calculates_options_via_unified_rpc() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (_edge, addr, _data_dir) = start_ready_edge().await;
        let mut client = tokio::time::timeout(
            STEP_DEADLINE,
            ValuationServiceClient::connect(format!("http://{addr}")),
        )
        .await
        .expect("client connects in time")
        .expect("client connects");

        let strike = 1.10;
        let opts_req = PriceRequest {
            request_id: 1,
            instrument: Some(vanilla_call(strike)),
            market: Some(live_market()),
            conventions: Some(wire_conventions()),
            correlation_id: Some(101),
            surface_version: None,
        };

        let val_req = ValuationRequest {
            request_id: "val-1".to_owned(),
            correlation_id: Some(101),
            payload: Some(ReqPayload::Options(opts_req)),
        };

        let resp = tokio::time::timeout(STEP_DEADLINE, client.calculate(val_req))
            .await
            .expect("calculate returns in time")
            .expect("calculate succeeds")
            .into_inner();

        assert_eq!(resp.request_id, "val-1");
        assert_eq!(resp.correlation_id, Some(101));

        match resp.payload.expect("payload present") {
            RespPayload::Options(price_resp) => {
                let greeks = price_resp.greeks.expect("greeks present");
                assert!(greeks.price > 0.0, "option price is positive");
                assert!(
                    greeks.delta_spot > 0.0 && greeks.delta_spot < 1.0,
                    "call delta in (0, 1)"
                );
            }
            _ => panic!("expected options payload"),
        }
    })
    .await
    .expect("test completes within wall-clock ceiling");
}

#[tokio::test]
async fn valuation_service_calculates_rates_via_unified_rpc() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (_edge, addr, _data_dir) = start_ready_edge().await;
        let mut client = tokio::time::timeout(
            STEP_DEADLINE,
            ValuationServiceClient::connect(format!("http://{addr}")),
        )
        .await
        .expect("client connects in time")
        .expect("client connects");

        let rates_req = celnet_proto::RatesPriceRequest {
            request_id: 2,
            curve_set: Some(celnet_proto::CurveSet {
                currency: "USD".to_owned(),
                reference_date: Some(celnet_proto::BrokenDate {
                    year: 2026,
                    month: 6,
                    day: 25,
                }),
                ois_pillars: vec![
                    celnet_proto::OisPillar {
                        tenor: Some(celnet_proto::PillarTenor {
                            point: Some(celnet_proto::pillar_tenor::Point::Years(1)),
                        }),
                        par_rate: 0.0420,
                    },
                    celnet_proto::OisPillar {
                        tenor: Some(celnet_proto::PillarTenor {
                            point: Some(celnet_proto::pillar_tenor::Point::Years(5)),
                        }),
                        par_rate: 0.0405,
                    },
                ],
            }),
            instrument: Some(celnet_proto::RatesInstrument {
                instrument: Some(celnet_proto::rates_instrument::Instrument::Ois(
                    celnet_proto::OisInstrument {
                        tenor_years: 5,
                        fixed_rate: 0.04,
                        notional: 10_000_000.0,
                        side: celnet_proto::Side::Buy as i32,
                    },
                )),
            }),
            correlation_id: Some(202),
        };

        let val_req = ValuationRequest {
            request_id: "val-rates-1".to_owned(),
            correlation_id: Some(202),
            payload: Some(ReqPayload::Rates(rates_req)),
        };

        let resp = tokio::time::timeout(STEP_DEADLINE, client.calculate(val_req))
            .await
            .expect("calculate returns in time")
            .expect("calculate succeeds")
            .into_inner();

        assert_eq!(resp.request_id, "val-rates-1");
        assert_eq!(resp.correlation_id, Some(202));

        match resp.payload.expect("payload present") {
            RespPayload::Rates(rates_resp) => {
                let result = rates_resp.result.expect("result present");
                assert!(result.par_rate > 0.0, "par rate is positive");
                assert!(result.dv01 != 0.0, "dv01 is computed");
            }
            _ => panic!("expected rates payload"),
        }
    })
    .await
    .expect("test completes within wall-clock ceiling");
}
