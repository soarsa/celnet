//! Single-option vanilla latency benchmarks (divan).
//!
//! These benchmarks prove the per-option hot-path budget from
//! `docs/ARCHITECTURE.md` §1.2: **vanilla price + full Greeks p50 ≤ 2 µs**
//! (p99 ≤ 10 µs) on a cached surface. `divan` reports the median and the min
//! per operation; the median is what we compare against the p50 ≤ 2 µs target.
//!
//! Two cases are timed:
//!  * [`price_only`] — a single Garman-Kohlhagen present value.
//!  * [`price_plus_full_greeks`] — present value **and** the full 14-Greek set
//!    in one pass, which is the actual quantity the budget governs.
//!
//! Inputs come from [`celnet_bench::representative_inputs`] so the timed work is
//! a realistic at-the-money option, and `black_box` guards both the input and
//! the result so the optimizer cannot fold the call away.

use celnet_bench::representative_inputs;
use celnet_types::{OptionType, VanillaInputs};
use celnet_vanilla::{greeks, price};
use divan::{Bencher, black_box};

fn main() {
    divan::main();
}

/// Single Garman-Kohlhagen present value (no Greeks).
#[divan::bench]
fn price_only(bencher: Bencher) {
    let inputs: VanillaInputs = representative_inputs();
    bencher.bench_local(|| price(OptionType::Call, black_box(&inputs)));
}

/// Present value **plus** the full 14-Greek set in a single pass.
///
/// This is the quantity the `p50 ≤ 2 µs` hot-path budget in
/// `docs/ARCHITECTURE.md` §1.2 governs.
#[divan::bench]
fn price_plus_full_greeks(bencher: Bencher) {
    let inputs: VanillaInputs = representative_inputs();
    bencher.bench_local(|| greeks(OptionType::Call, black_box(&inputs)));
}

/// Put-side full Greek pass, to confirm the call/put branches cost the same.
#[divan::bench]
fn price_plus_full_greeks_put(bencher: Bencher) {
    let inputs: VanillaInputs = representative_inputs();
    bencher.bench_local(|| greeks(OptionType::Put, black_box(&inputs)));
}
