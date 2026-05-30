//! Batched many-strike latency/throughput benchmarks (divan).
//!
//! A surface rebuild or a portfolio repricing does not price one option — it
//! sweeps a whole strike ladder per pair/tenor. This bench times that sweep
//! over [`celnet_bench::representative_batch`] (a [`celnet_bench::BATCH_STRIKES`]
//! -strike slice) so the per-batch median, divided by the strike count, is the
//! amortized per-option cost that underwrites:
//!  * the **streaming quote throughput ≥ 1M updates/s/core** target, and
//!  * the **surface rebuild (single pair, all tenors) p99 ≤ 150 µs** target —
//!
//! both from `docs/ARCHITECTURE.md` §1.2.
//!
//! The batch vector is built **outside** the timed closure (in the `with_inputs`
//! setup) so the measurement is pure compute, not allocation. `black_box` guards
//! the per-option result inside the loop to defeat dead-code elimination.

use celnet_bench::{BATCH_STRIKES, representative_batch};
use celnet_types::{Greeks, OptionType, VanillaInputs};
use celnet_vanilla::{greeks, price};
use divan::{Bencher, black_box};

fn main() {
    divan::main();
}

/// Price a full many-strike slice (present value only).
///
/// Counted with `divan`'s item count so the harness also reports throughput
/// (items/s) directly — the basis for the ≥ 1M updates/s/core comparison.
#[divan::bench]
fn batch_price(bencher: Bencher) {
    bencher
        .with_inputs(representative_batch)
        .input_counter(|b: &Vec<VanillaInputs>| divan::counter::ItemsCount::new(b.len()))
        .bench_local_values(|batch: Vec<VanillaInputs>| {
            let mut acc = 0.0f64;
            for i in &batch {
                acc += price(OptionType::Call, black_box(i));
            }
            acc
        });
}

/// Price + full 14-Greek set across the whole many-strike slice.
///
/// This is the portfolio/surface analogue of the single-option hot-path budget:
/// the per-item amortized median should track the `p50 ≤ 2 µs` per-option target
/// in `docs/ARCHITECTURE.md` §1.2.
#[divan::bench]
fn batch_price_plus_greeks(bencher: Bencher) {
    bencher
        .with_inputs(representative_batch)
        .input_counter(|b: &Vec<VanillaInputs>| divan::counter::ItemsCount::new(b.len()))
        .bench_local_values(|batch: Vec<VanillaInputs>| {
            // Fold the price field of every Greek set so nothing is elided.
            let mut acc = 0.0f64;
            for i in &batch {
                let g: Greeks = greeks(OptionType::Call, black_box(i));
                acc += g.price + g.vega + g.gamma;
            }
            acc
        });
}

/// Sanity tie-in: the batch the bench prices is the documented size.
const _: () = assert!(BATCH_STRIKES == 64);
