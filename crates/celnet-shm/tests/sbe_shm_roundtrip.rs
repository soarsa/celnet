//! SBE over Shared-Memory IPC integration test.
//!
//! Verifies end-to-end zero-allocation transmission of SBE OptionQuotes
//! across shared-memory ring buffer boundaries.
use std::hint::black_box;
use std::time::Instant;

use celnet_sbe::{
    OPTION_QUOTE_TOTAL_SIZE, OptionQuote, OptionQuoteFlyweight, encode_option_quote,
};
use celnet_shm::{ShmConsumer, ShmProducer};
use tempfile::NamedTempFile;

#[test]
fn sbe_shm_sub_microsecond_round_trip() {
    let temp = NamedTempFile::new().expect("create tempfile");
    let path = temp.path();

    let capacity = 1024;
    let slot_size = OPTION_QUOTE_TOTAL_SIZE;

    let mut producer = ShmProducer::create(path, capacity, slot_size).expect("producer creates shm");
    let mut consumer = ShmConsumer::open_replay(path).expect("consumer opens shm");

    let num_quotes = 10_000u64;
    let mut sbe_buf = [0u8; OPTION_QUOTE_TOTAL_SIZE];
    let mut recv_buf = [0u8; OPTION_QUOTE_TOTAL_SIZE];

    let start = Instant::now();
    for i in 0..num_quotes {
        let quote = OptionQuote {
            quote_id: 1_000_000 + i,
            epoch_nanos: 1_725_450_000_000_000_000 + i as i64,
            valid_until_nanos: 1_725_450_005_000_000_000 + i as i64,
            bid_price: 1.0850 + (i as f64 * 0.00001),
            ask_price: 1.0852 + (i as f64 * 0.00001),
            resolved_strike: 1.0850,
            surface_version: 42,
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
        };

        // 1. Encode SBE quote into frame
        encode_option_quote(&quote, &mut sbe_buf).expect("encode succeeds");

        // 2. Publish SBE frame into shared memory
        producer.publish(&sbe_buf).expect("publish succeeds");

        // 3. Consume SBE frame from shared memory
        let len = consumer.try_recv(&mut recv_buf).expect("recv succeeds");
        assert_eq!(len, OPTION_QUOTE_TOTAL_SIZE);

        // 4. Decode via SBE flyweight and assert exact field identity
        let fw = OptionQuoteFlyweight::wrap(&recv_buf[..len]).expect("flyweight wrap succeeds");
        assert_eq!(fw.quote_id(), quote.quote_id);
        assert_eq!(fw.epoch_nanos(), quote.epoch_nanos);
        assert_eq!(fw.bid_price(), quote.bid_price);
        assert_eq!(fw.greeks(), quote.greeks);

        black_box(fw.greeks());
    }

    let elapsed = start.elapsed();
    let ns_per_round_trip = elapsed.as_nanos() as f64 / num_quotes as f64;
    let mops = (num_quotes as f64 / elapsed.as_secs_f64()) / 1_000_000.0;

    println!(
        "\n[celnet-shm + celnet-sbe] Transmitted {} SBE OptionQuotes over Shared Memory:\n  Total Elapsed: {:?}\n  Latency per IPC Round-Trip: {:.2} ns ({:.3} µs)\n  Throughput: {:.2} Million quotes/sec\n",
        num_quotes, elapsed, ns_per_round_trip, ns_per_round_trip / 1000.0, mops
    );

    // Assert sub-microsecond latency (< 1,000 ns)
    assert!(
        ns_per_round_trip < 1000.0,
        "IPC round-trip latency must be sub-microsecond, was {:.2} ns",
        ns_per_round_trip
    );
}

#[test]
fn sbe_shm_zero_copy_in_place_view_round_trip() {
    let temp = NamedTempFile::new().expect("create tempfile");
    let path = temp.path();

    let capacity = 1024;
    let slot_size = OPTION_QUOTE_TOTAL_SIZE;

    let mut producer = ShmProducer::create(path, capacity, slot_size).expect("producer creates shm");
    let mut consumer = ShmConsumer::open_replay(path).expect("consumer opens shm");

    let num_quotes = 10_000u64;

    let start = Instant::now();
    for i in 0..num_quotes {
        let quote = OptionQuote {
            quote_id: 2_000_000 + i,
            epoch_nanos: 1_725_450_000_000_000_000 + i as i64,
            valid_until_nanos: 1_725_450_005_000_000_000 + i as i64,
            bid_price: 1.0850 + (i as f64 * 0.00001),
            ask_price: 1.0852 + (i as f64 * 0.00001),
            resolved_strike: 1.0850,
            surface_version: 42,
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
        };

        // Publish using high-level SBE publisher
        producer.publish_sbe_quote(&quote).expect("publish succeeds");

        // Zero-copy in-place consumption directly inside mapped buffer
        let read_delta = consumer
            .try_recv_sbe_quote_view(|fw| {
                assert_eq!(fw.quote_id(), quote.quote_id);
                fw.greeks().delta_spot
            })
            .expect("view succeeds");

        black_box(read_delta);
    }

    let elapsed = start.elapsed();
    let ns_per_round_trip = elapsed.as_nanos() as f64 / num_quotes as f64;
    let mops = (num_quotes as f64 / elapsed.as_secs_f64()) / 1_000_000.0;

    println!(
        "\n[celnet-shm + SOTA Zero-Copy View] Transmitted {} SBE OptionQuotes In-Place:\n  Total Elapsed: {:?}\n  Latency per IPC In-Place Round-Trip: {:.2} ns ({:.3} µs)\n  Throughput: {:.2} Million quotes/sec\n",
        num_quotes, elapsed, ns_per_round_trip, ns_per_round_trip / 1000.0, mops
    );

    assert!(
        ns_per_round_trip < 1000.0,
        "Zero-copy IPC round-trip latency must be sub-microsecond, was {:.2} ns",
        ns_per_round_trip
    );
}
