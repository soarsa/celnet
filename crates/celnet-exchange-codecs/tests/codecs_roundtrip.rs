//! Unit and roundtrip tests for native exchange binary codecs.

use celnet_exchange_codecs::eti::FramedOrderRequest;
use celnet_exchange_codecs::ilink::{ExecutionReport, ExecStatus, NewOrderSingle};
use celnet_exchange_codecs::mdp::{BookEntry, EntryType, IncrementalRefresh, PacketHeader, UpdateAction};
use celnet_exchange_codecs::ouch::OuchEnterOrder;
use celnet_exchange_codecs::transcoder::ExchangeTranscoder;
use celnet_exchange_codecs::{ExchangeSide, ExchangeTimeInForce};

#[test]
fn test_mdp_packet_header_roundtrip() {
    let mut buf = [0u8; 64];
    let header = PacketHeader {
        sequence_number: 104523,
        sending_time_nanos: 1788570000000000000,
    };
    let written = header.encode(&mut buf).expect("encode packet header");
    assert_eq!(written, PacketHeader::SIZE);

    let decoded = PacketHeader::decode(&buf[..written]).expect("decode packet header");
    assert_eq!(header, decoded);
}

#[test]
fn test_mdp_incremental_refresh_roundtrip() {
    let mut buf = [0u8; 256];
    let entry1 = BookEntry::from_price_and_size(
        UpdateAction::New,
        EntryType::Bid,
        1001,
        1,
        105.250,
        500,
        12,
    );
    let entry2 = BookEntry::from_price_and_size(
        UpdateAction::Change,
        EntryType::Offer,
        1001,
        2,
        105.255,
        750,
        18,
    );

    let refresh = IncrementalRefresh {
        transact_time_nanos: 1788570000123456789,
        match_event_indicator: 0x01,
        entries: vec![entry1, entry2],
    };

    let written = refresh.encode(&mut buf).expect("encode refresh");
    let decoded = IncrementalRefresh::decode(&buf[..written]).expect("decode refresh");

    assert_eq!(refresh.transact_time_nanos, decoded.transact_time_nanos);
    assert_eq!(refresh.match_event_indicator, decoded.match_event_indicator);
    assert_eq!(refresh.entries.len(), decoded.entries.len());
    assert_eq!(refresh.entries[0].action, decoded.entries[0].action);
    assert_eq!(refresh.entries[0].entry_type, decoded.entries[0].entry_type);
    assert_eq!(refresh.entries[0].security_id, decoded.entries[0].security_id);
    assert!((refresh.entries[0].price_f64() - decoded.entries[0].price_f64()).abs() < 1e-6);
    assert_eq!(refresh.entries[0].size, decoded.entries[0].size);
}

#[test]
fn test_ilink_new_order_single_roundtrip() {
    let mut buf = [0u8; 128];
    let order = NewOrderSingle::new(
        "ORD-2026-X99",
        205,
        ExchangeSide::Buy,
        100,
        98.4375,
        ExchangeTimeInForce::ImmediateOrCancel,
        false,
        "CELNET",
    );

    let written = order.encode(&mut buf).expect("encode new order");
    assert_eq!(written, NewOrderSingle::WIRE_SIZE);

    let decoded = NewOrderSingle::decode(&buf[..written]).expect("decode new order");
    assert_eq!(order.security_id, decoded.security_id);
    assert_eq!(order.side, decoded.side);
    assert_eq!(order.order_qty, decoded.order_qty);
    assert!((order.price_f64() - decoded.price_f64()).abs() < 1e-6);
    assert_eq!(order.time_in_force, decoded.time_in_force);
}

#[test]
fn test_ilink_execution_report_roundtrip() {
    let mut buf = [0u8; 128];
    let mut cl_ord_id = [b' '; 20];
    cl_ord_id[..12].copy_from_slice(b"ORD-2026-X99");

    let report = ExecutionReport {
        cl_ord_id,
        order_id: 888999,
        exec_id: 1788570000999,
        status: ExecStatus::Filled,
        cum_qty: 100,
        leaves_qty: 0,
        last_px_mantissa: 984375000,
        last_px_exponent: -7,
        last_qty: 100,
        transact_time_nanos: 1788570000000,
    };

    let written = report.encode(&mut buf).expect("encode exec report");
    let decoded = ExecutionReport::decode(&buf[..written]).expect("decode exec report");

    assert_eq!(report.order_id, decoded.order_id);
    assert_eq!(report.status, decoded.status);
    assert_eq!(report.cum_qty, decoded.cum_qty);
    assert!((report.last_px_f64() - decoded.last_px_f64()).abs() < 1e-6);
}

#[test]
fn test_eti_framed_order_roundtrip() {
    let mut buf = [0u8; 128];
    let order = FramedOrderRequest {
        cl_ord_id: 5544332211,
        security_id: 100200,
        side: ExchangeSide::Sell,
        time_in_force: ExchangeTimeInForce::Day,
        order_qty: 250,
        price_scaled: 10525000000, // 105.25000000
    };

    let written = order.encode(&mut buf).expect("encode framed order");
    assert_eq!(written, FramedOrderRequest::WIRE_SIZE);

    let decoded = FramedOrderRequest::decode(&buf[..written]).expect("decode framed order");
    assert_eq!(order.cl_ord_id, decoded.cl_ord_id);
    assert_eq!(order.security_id, decoded.security_id);
    assert_eq!(order.side, decoded.side);
    assert_eq!(order.order_qty, decoded.order_qty);
    assert!((order.price_f64() - decoded.price_f64()).abs() < 1e-6);
}

#[test]
fn test_ouch_order_roundtrip() {
    let mut buf = [0u8; 64];
    let order = OuchEnterOrder::new(
        "TOKEN12345",
        ExchangeSide::Buy,
        500,
        "US10Y",
        99.1250,
        ExchangeTimeInForce::ImmediateOrCancel,
        "CELR",
    );

    let written = order.encode(&mut buf).expect("encode OUCH order");
    assert_eq!(written, OuchEnterOrder::SIZE);

    let decoded = OuchEnterOrder::decode(&buf[..written]).expect("decode OUCH order");
    assert_eq!(order.side, decoded.side);
    assert_eq!(order.shares, decoded.shares);
    assert!((order.price_f64() - decoded.price_f64()).abs() < 1e-4);
    assert_eq!(order.time_in_force, decoded.time_in_force);
}

#[test]
fn test_exchange_transcoder_flow() {
    let entry = BookEntry::from_price_and_size(
        UpdateAction::New,
        EntryType::Bid,
        5501,
        1,
        1.0850,
        1_000_000,
        5,
    );
    let tick = ExchangeTranscoder::mdp_entry_to_price_tick(&entry, 1788570000).expect("transcode");
    assert_eq!(tick.pair_id, 5501);
    assert!((tick.bid - 1.0850).abs() < 1e-6);

    let celnet_exec = celnet_sbe::ExecutionReport {
        exec_id: 1001,
        quote_id: 2002,
        epoch_nanos: 1788570000,
        exec_price: 1.0855,
        exec_quantity: 500_000.0,
        side: celnet_sbe::Side::Buy,
        status: celnet_sbe::ExecutionStatus::Filled,
        lp_id: 9,
    };
    let ilink_rep = ExchangeTranscoder::celnet_exec_to_ilink(&celnet_exec);
    assert_eq!(ilink_rep.order_id, 2002);
    assert_eq!(ilink_rep.last_qty, 500_000);
    assert_eq!(ilink_rep.status, ExecStatus::Filled);

    let ouch_exec = ExchangeTranscoder::ilink_to_ouch_executed(&ilink_rep);
    assert_eq!(ouch_exec.executed_shares, 500_000);
    assert_eq!(ouch_exec.execution_price, 10855); // 1.0855 * 10000
}
