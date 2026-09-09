//! Server Integration Test: Native Exchange Binary Protocol Gateways.
//!
//! Validates end-to-end processing of native binary exchange frames
//! (SBE MDP 3.0 market data and SBE iLink3 order entry) through Celnet transcoding pipelines.

use celnet_exchange_codecs::ilink::{ExecutionReport, ExecStatus, NewOrderSingle};
use celnet_exchange_codecs::mdp::{BookEntry, EntryType, IncrementalRefresh, UpdateAction};
use celnet_exchange_codecs::transcoder::ExchangeTranscoder;
use celnet_exchange_codecs::{ExchangeSide, ExchangeTimeInForce};

#[test]
fn test_exchange_gateway_market_data_ingest() {
    let mut buf = [0u8; 512];

    // Simulate exchange multicast market data packet
    let entry = BookEntry::from_price_and_size(
        UpdateAction::New,
        EntryType::Bid,
        8001,
        101,
        105.125,
        2_500_000,
        24,
    );

    let refresh = IncrementalRefresh {
        transact_time_nanos: 1788570000500,
        match_event_indicator: 0x01,
        entries: vec![entry],
    };

    let written = refresh.encode(&mut buf).expect("encode exchange market data");
    let decoded = IncrementalRefresh::decode(&buf[..written]).expect("decode exchange market data");

    // Transcode into Celnet SBE format for high-throughput zero-alloc core dispatch
    let tick = ExchangeTranscoder::mdp_entry_to_price_tick(&decoded.entries[0], 1788570000500).expect("transcode tick");

    assert_eq!(tick.pair_id, 8001);
    assert!((tick.bid - 105.125).abs() < 1e-6);
    assert_eq!(tick.flags, 1); // Tradable
}

#[test]
fn test_exchange_gateway_order_and_execution_lifecycle() {
    let mut order_buf = [0u8; 128];

    // 1. Inbound iLink3 NewOrderSingle
    let order = NewOrderSingle::new(
        "CL-EXCH-909",
        8001,
        ExchangeSide::Buy,
        50, // 50 contracts
        105.125,
        ExchangeTimeInForce::ImmediateOrCancel,
        false,
        "CELER",
    );

    let written = order.encode(&mut order_buf).expect("encode order");
    let decoded_order = NewOrderSingle::decode(&order_buf[..written]).expect("decode order");

    assert_eq!(decoded_order.order_qty, 50);

    // 2. Transcode into SBE ExecutionReport from engine fill
    let engine_exec = celnet_sbe::ExecutionReport {
        exec_id: 998877,
        quote_id: 12345,
        epoch_nanos: 1788570000900,
        exec_price: 105.125,
        exec_quantity: 50.0,
        side: celnet_sbe::Side::Buy,
        status: celnet_sbe::ExecutionStatus::Filled,
        lp_id: 1,
    };

    let ilink_exec = ExchangeTranscoder::celnet_exec_to_ilink(&engine_exec);
    assert_eq!(ilink_exec.status, ExecStatus::Filled);
    assert_eq!(ilink_exec.last_qty, 50);

    // Verify wire encoding of execution report back to client
    let mut exec_buf = [0u8; 128];
    let exec_written = ilink_exec.encode(&mut exec_buf).expect("encode ilink exec");
    let decoded_exec = ExecutionReport::decode(&exec_buf[..exec_written]).expect("decode ilink exec");

    assert_eq!(decoded_exec.exec_id, 998877);
    assert_eq!(decoded_exec.status, ExecStatus::Filled);
}
