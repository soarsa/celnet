//! `codec_bench` — Micro-benchmark comparing Native memory copy, SBE (Simple Binary
//! Encoding) flyweights, Protobuf (prost), and JSON (serde_json) on quote data.
//!
//! Evaluates the empirical cost of serialization across architectural tiers:
//! - In-core / L1 cache (Native Rust `#[repr(C)]` copy)
//! - Low-latency IPC / Network (SBE zero-allocation direct-buffer flyweights)
//! - Edge / RPC (Protobuf varint / tag encoding)
//! - Web / Gateway (JSON text encoding)
#![forbid(unsafe_code)]
#![allow(missing_docs)]

use std::hint::black_box;
use std::time::Instant;

use prost::Message;
use serde::{Deserialize, Serialize};

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct NativeQuote {
    pub quote_id: u64,
    pub epoch_nanos: i64,
    pub valid_until_nanos: i64,
    pub bid_price: f64,
    pub ask_price: f64,
    pub resolved_strike: f64,
    pub surface_version: u64,
    pub greeks: celnet_types::Greeks,
}

impl NativeQuote {
    pub fn sample() -> Self {
        Self {
            quote_id: 123_456_789,
            epoch_nanos: 1_725_450_000_000_000_000,
            valid_until_nanos: 1_725_450_005_000_000_000,
            bid_price: 0.01234,
            ask_price: 0.01238,
            resolved_strike: 1.0850,
            surface_version: 101,
            greeks: celnet_types::Greeks {
                price: 0.012345,
                delta_spot: 0.4821,
                delta_forward: 0.4933,
                gamma: 2.118,
                vega: 0.305,
                theta: -0.018,
                rho_dom: 0.061,
                rho_for: -0.058,
                vanna: -0.072,
                volga: 0.144,
                charm: 0.0009,
                speed: -1.21,
                zomma: 0.33,
                color: 0.0004,
            },
        }
    }
}

fn to_sbe_quote(src: &NativeQuote) -> celnet_sbe::OptionQuote {
    celnet_sbe::OptionQuote {
        quote_id: src.quote_id,
        epoch_nanos: src.epoch_nanos,
        valid_until_nanos: src.valid_until_nanos,
        bid_price: src.bid_price,
        ask_price: src.ask_price,
        resolved_strike: src.resolved_strike,
        surface_version: src.surface_version,
        greeks: src.greeks,
    }
}


fn create_proto_quote(src: &NativeQuote) -> celnet_proto::Quote {
    celnet_proto::Quote {
        quote_id: src.quote_id,
        idempotency_key: "IDEMP-BENCHMARK-001".to_string(),
        price: Some(celnet_proto::TwoWayPrice {
            bid: src.bid_price,
            offer: src.ask_price,
        }),
        greeks: Some(celnet_proto::Greeks {
            price: src.greeks.price,
            delta_spot: src.greeks.delta_spot,
            delta_forward: src.greeks.delta_forward,
            gamma: src.greeks.gamma,
            vega: src.greeks.vega,
            theta: src.greeks.theta,
            rate_sensitivities: Some(celnet_proto::RateSensitivities::fx(
                src.greeks.rho_dom,
                src.greeks.rho_for,
            )),
            vanna: src.greeks.vanna,
            volga: src.greeks.volga,
            charm: src.greeks.charm,
            speed: src.greeks.speed,
            zomma: src.greeks.zomma,
            color: src.greeks.color,
        }),
        conventions: Some(celnet_proto::Conventions {
            delta_convention: celnet_proto::DeltaConvention::SpotPremiumAdjusted as i32,
            atm_convention: celnet_proto::AtmConvention::DeltaNeutralStraddle as i32,
            premium_style: celnet_proto::PremiumStyle::PercentForeign as i32,
            cut: celnet_proto::Cut::NewYork1000 as i32,
            day_count: celnet_proto::DayCount::Act365Fixed as i32,
            settlement: celnet_proto::Settlement::Deliverable as i32,
        }),
        resolved_strike: src.resolved_strike,
        epoch_nanos: src.epoch_nanos,
        valid_until_nanos: src.valid_until_nanos,
        correlation_id: Some(42),
        surface_version: Some(src.surface_version),
        attribution: None,
        price_std_error: None,
        pricing_provenance: None,
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CodecResult {
    pub codec: String,
    pub iterations: u64,
    pub payload_bytes: usize,
    pub encode_ns_per_op: f64,
    pub decode_ns_per_op: f64,
    pub round_trip_ns_per_op: f64,
    pub throughput_million_ops_sec: f64,
    pub zero_alloc: bool,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let iters = 2_000_000u64;
    let warmup = 100_000u64;

    println!("================================================================================");
    println!("  Celnet Codec Benchmark: Native vs SBE vs Protobuf vs JSON");
    println!("  Iterations: {} (warmup: {})", iters, warmup);
    println!("================================================================================\n");

    let native_src = NativeQuote::sample();
    let proto_src = create_proto_quote(&native_src);

    // -------------------------------------------------------------------------
    // 1. Native Struct In-Memory Copy (Baseline)
    // -------------------------------------------------------------------------
    let mut native_dest = native_src;
    for _ in 0..warmup {
        native_dest = black_box(black_box(native_src));
    }
    let start = Instant::now();
    for _ in 0..iters {
        native_dest = black_box(black_box(native_src));
    }
    let native_elapsed = start.elapsed();
    let native_ns = native_elapsed.as_nanos() as f64 / iters as f64;
    let native_mops = (iters as f64 / native_elapsed.as_secs_f64()) / 1_000_000.0;
    black_box(native_dest);

    let native_result = CodecResult {
        codec: "Native Rust #[repr(C)] Copy".to_string(),
        iterations: iters,
        payload_bytes: std::mem::size_of::<NativeQuote>(),
        encode_ns_per_op: native_ns / 2.0,
        decode_ns_per_op: native_ns / 2.0,
        round_trip_ns_per_op: native_ns,
        throughput_million_ops_sec: native_mops,
        zero_alloc: true,
    };

    // -------------------------------------------------------------------------
    // 2. SBE Direct Buffer Flyweight (celnet-sbe crate)
    // -------------------------------------------------------------------------
    let sbe_src = to_sbe_quote(&native_src);
    let mut sbe_buffer = [0u8; celnet_sbe::OPTION_QUOTE_TOTAL_SIZE];
    for _ in 0..warmup {
        celnet_sbe::encode_option_quote(&sbe_src, &mut sbe_buffer).unwrap();
        let flyweight = celnet_sbe::OptionQuoteFlyweight::wrap(&sbe_buffer).unwrap();
        black_box(flyweight.quote_id());
        black_box(flyweight.bid_price());
        black_box(flyweight.greeks());
    }

    // Measure SBE encode
    let start = Instant::now();
    for _ in 0..iters {
        celnet_sbe::encode_option_quote(black_box(&sbe_src), black_box(&mut sbe_buffer)).unwrap();
    }
    let sbe_encode_elapsed = start.elapsed();
    let sbe_encode_ns = sbe_encode_elapsed.as_nanos() as f64 / iters as f64;

    // Measure SBE decode (flyweight access)
    let start = Instant::now();
    for _ in 0..iters {
        let fw = celnet_sbe::OptionQuoteFlyweight::wrap(black_box(&sbe_buffer)).unwrap();
        black_box(fw.quote_id());
        black_box(fw.bid_price());
        black_box(fw.ask_price());
        black_box(fw.resolved_strike());
        black_box(fw.greeks());
    }
    let sbe_decode_elapsed = start.elapsed();
    let sbe_decode_ns = sbe_decode_elapsed.as_nanos() as f64 / iters as f64;
    let sbe_total_ns = sbe_encode_ns + sbe_decode_ns;
    let sbe_mops = 1_000.0 / sbe_total_ns;

    let sbe_result = CodecResult {
        codec: "SBE Direct-Buffer Flyweight".to_string(),
        iterations: iters,
        payload_bytes: celnet_sbe::OPTION_QUOTE_TOTAL_SIZE,
        encode_ns_per_op: sbe_encode_ns,
        decode_ns_per_op: sbe_decode_ns,
        round_trip_ns_per_op: sbe_total_ns,
        throughput_million_ops_sec: sbe_mops,
        zero_alloc: true,
    };

    // -------------------------------------------------------------------------
    // 3. Protobuf (prost)
    // -------------------------------------------------------------------------
    let mut proto_buf = Vec::with_capacity(512);
    proto_src.encode(&mut proto_buf).unwrap();
    let proto_len = proto_buf.len();

    // Warmup
    for _ in 0..warmup {
        proto_buf.clear();
        proto_src.encode(&mut proto_buf).unwrap();
        let decoded = celnet_proto::Quote::decode(&proto_buf[..]).unwrap();
        black_box(decoded);
    }

    // Measure Protobuf encode
    let start = Instant::now();
    for _ in 0..iters {
        proto_buf.clear();
        black_box(&proto_src).encode(&mut proto_buf).unwrap();
    }
    let proto_encode_elapsed = start.elapsed();
    let proto_encode_ns = proto_encode_elapsed.as_nanos() as f64 / iters as f64;

    // Measure Protobuf decode
    let start = Instant::now();
    for _ in 0..iters {
        let decoded = celnet_proto::Quote::decode(black_box(&proto_buf[..])).unwrap();
        black_box(decoded);
    }
    let proto_decode_elapsed = start.elapsed();
    let proto_decode_ns = proto_decode_elapsed.as_nanos() as f64 / iters as f64;
    let proto_total_ns = proto_encode_ns + proto_decode_ns;
    let proto_mops = 1_000.0 / proto_total_ns;

    let proto_result = CodecResult {
        codec: "Protobuf (prost v3)".to_string(),
        iterations: iters,
        payload_bytes: proto_len,
        encode_ns_per_op: proto_encode_ns,
        decode_ns_per_op: proto_decode_ns,
        round_trip_ns_per_op: proto_total_ns,
        throughput_million_ops_sec: proto_mops,
        zero_alloc: false,
    };

    // -------------------------------------------------------------------------
    // 4. JSON (serde_json)
    // -------------------------------------------------------------------------
    let json_bytes = serde_json::to_vec(&native_src).unwrap();
    let json_len = json_bytes.len();
    let json_iters = iters / 10; // 200,000 to keep run time reasonable

    // Warmup
    for _ in 0..warmup.min(20_000) {
        let bytes = serde_json::to_vec(&native_src).unwrap();
        let decoded: NativeQuote = serde_json::from_slice(&bytes).unwrap();
        black_box(decoded);
    }

    // Measure JSON encode
    let start = Instant::now();
    for _ in 0..json_iters {
        let bytes = serde_json::to_vec(black_box(&native_src)).unwrap();
        black_box(bytes);
    }
    let json_encode_elapsed = start.elapsed();
    let json_encode_ns = json_encode_elapsed.as_nanos() as f64 / json_iters as f64;

    // Measure JSON decode
    let start = Instant::now();
    for _ in 0..json_iters {
        let decoded: NativeQuote = serde_json::from_slice(black_box(&json_bytes)).unwrap();
        black_box(decoded);
    }
    let json_decode_elapsed = start.elapsed();
    let json_decode_ns = json_decode_elapsed.as_nanos() as f64 / json_iters as f64;
    let json_total_ns = json_encode_ns + json_decode_ns;
    let json_mops = 1_000.0 / json_total_ns;

    let json_result = CodecResult {
        codec: "JSON (serde_json)".to_string(),
        iterations: json_iters,
        payload_bytes: json_len,
        encode_ns_per_op: json_encode_ns,
        decode_ns_per_op: json_decode_ns,
        round_trip_ns_per_op: json_total_ns,
        throughput_million_ops_sec: json_mops,
        zero_alloc: false,
    };

    let results = vec![native_result, sbe_result, proto_result, json_result];

    // Print ASCII comparison table
    println!(
        "{:<30} | {:>10} | {:>12} | {:>12} | {:>14} | {:>12} | {:>10}",
        "Codec / Memory Pattern",
        "Size (B)",
        "Encode (ns)",
        "Decode (ns)",
        "RoundTrip (ns)",
        "M ops/sec",
        "Zero Alloc"
    );
    println!("{:-<30}-+-{:-<10}-+-{:-<12}-+-{:-<12}-+-{:-<14}-+-{:-<12}-+-{:-<10}", "", "", "", "", "", "", "");

    for r in &results {
        println!(
            "{:<30} | {:>10} | {:>12.2} | {:>12.2} | {:>14.2} | {:>12.2} | {:>10}",
            r.codec,
            r.payload_bytes,
            r.encode_ns_per_op,
            r.decode_ns_per_op,
            r.round_trip_ns_per_op,
            r.throughput_million_ops_sec,
            if r.zero_alloc { "YES (0 B)" } else { "NO (heap)" }
        );
    }

    println!("\nRelative Speedup vs Protobuf:");
    let proto_rt = results[2].round_trip_ns_per_op;
    println!("  - Native struct copy: {:>6.1}x faster", proto_rt / results[0].round_trip_ns_per_op);
    println!("  - SBE flyweight:      {:>6.1}x faster", proto_rt / results[1].round_trip_ns_per_op);
    println!("  - Protobuf:             1.0x (baseline)");
    println!("  - JSON:               {:>6.1}x slower", results[3].round_trip_ns_per_op / proto_rt);

    println!("\nRelative Speedup vs JSON:");
    let json_rt = results[3].round_trip_ns_per_op;
    println!("  - Native struct copy: {:>6.1}x faster", json_rt / results[0].round_trip_ns_per_op);
    println!("  - SBE flyweight:      {:>6.1}x faster", json_rt / results[1].round_trip_ns_per_op);
    println!("  - Protobuf:           {:>6.1}x faster", json_rt / proto_rt);

    // Serialize to JSON for CI / documentation embedding
    let json_out = serde_json::to_string_pretty(&results)?;
    let out_path = "crates/celnet-bench/baselines/codec_comparison.json";
    std::fs::write(out_path, format!("{json_out}\n"))?;
    println!("\nWrote benchmark results to {}", out_path);

    Ok(())
}
