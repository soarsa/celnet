//! Fuzz target: arbitrary bytes -> prost decode of the wire contract ->
//! `celnet_proto::convert` domain mapping.
//!
//! The wire contract is decoded straight off a gRPC/WS socket by `prost`, then
//! the resulting (proto3-permissive) wire structs are mapped into the strict
//! `celnet_types` domain by the fallible `TryFrom` converters in `convert.rs`.
//! Both halves are untrusted-input parsers:
//!   * `prost::Message::decode` consumes arbitrary attacker bytes (varints,
//!     length-delimited fields, unknown tags), and
//!   * the `convert::TryFrom` mappers must reject every out-of-domain value with a
//!     typed [`celnet_proto::convert::WireError`] rather than panic — a free-text
//!     currency code, an enum integer with no matching variant, an out-of-range
//!     tenor count, a `BrokenDate` month/day that does not fit a `u8`.
//!
//! This drives the **exact** paths the server uses (see
//! `celnet-server/src/services/{surface,stream,forward}.rs`,
//! `ws/codec.rs`): decode a top-level request message, then run the domain
//! converters on its fields.
//!
//! Contract under any input bytes:
//!   * `prost` decode never panics — it returns `Ok(msg)` or a `DecodeError`, and
//!   * every domain `TryFrom`/enum `try_from` either yields a value or a typed
//!     error; none panics.
//!
//! Run (Linux nightly):
//!   cargo +nightly fuzz run proto_convert -- -max_total_time=120

#![no_main]

use libfuzzer_sys::fuzz_target;

use prost::Message as _;

use celnet_proto::{
    BrokenDate as WireBrokenDate, CcyPair as WireCcyPair, GetSmileRequest, QuoteRequest,
    SmileModel as WireSmileModel, Tenor as WireTenor,
};
use celnet_types::{BrokenDate, CcyPair, SmileModel, Tenor};

fuzz_target!(|data: &[u8]| {
    // 1) Top-level request messages the server decodes off the wire. prost decode
    //    must never panic; on success, run the domain converters its fields feed.
    if let Ok(req) = GetSmileRequest::decode(data) {
        if let Some(pair) = req.pair {
            // The fallible currency-code mapper (free text -> domain CcyPair).
            let _ = CcyPair::try_from(pair);
        }
        // tenor_years is a raw f64 off the wire — exercise it without trusting it.
        let _ = req.tenor_years.is_finite();
    }
    // QuoteRequest carries the unified Instrument oneof + idempotency key — a much
    // larger nested decode surface; just decoding it adversarially is the point.
    let _ = QuoteRequest::decode(data);

    // 2) The standalone fallible field converters, fed wire structs decoded from
    //    arbitrary bytes (each is a hand-written `TryFrom` in convert.rs).
    if let Ok(wire) = WireCcyPair::decode(data) {
        let _ = CcyPair::try_from(wire);
    }
    if let Ok(wire) = WireTenor::decode(data) {
        // Covers the enum-tag, out-of-range count, and missing-broken-date paths.
        let _ = Tenor::try_from(wire);
    }
    if let Ok(wire) = WireBrokenDate::decode(data) {
        // Covers the month/day `u8::try_from` out-of-range rejections.
        let _ = BrokenDate::try_from(wire);
    }

    // 3) The proto3 enum -> domain enum path the server uses
    //    (`SmileModel::try_from(i32)` then `convert::TryFrom<WireSmileModel>`).
    //    Drive it across the i32 space the fuzzer reaches via the first 4 bytes.
    if data.len() >= 4 {
        let tag = i32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        if let Ok(wire_enum) = WireSmileModel::try_from(tag) {
            let _ = SmileModel::try_from(wire_enum);
        }
    }
});
