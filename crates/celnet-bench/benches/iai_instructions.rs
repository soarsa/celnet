//! Jitter-free **instruction-count** gate for the vanilla pricing hot path.
//!
//! The wall-clock benchmarks (`benches/vanilla.rs` via `divan`, and the absolute
//! §1.2 truth-gate in `src/bin/core_load.rs`) measure *time* — the quantity the
//! architecture's microsecond budget is stated in. This bench measures the
//! complementary, **deterministic** quantity: the number of CPU instructions
//! retired (plus cache accesses and an estimated cycle count) per pricing call,
//! counted under Valgrind/Callgrind via [`iai_callgrind`].
//!
//! Why both? Wall-clock time is the contract, but it floats with host load,
//! frequency scaling and the scheduler — so a *wall-clock* CI gate must carry a
//! generous tolerance to avoid flapping. Instruction count is **machine- and
//! noise-independent**: the same code retires the same instructions every run, on
//! any core, under any load. That makes it the right primitive for catching a
//! *code-level* regression (an extra branch, a lost inlining, an accidental
//! allocation) the moment it lands — with a tight baseline and no flake.
//!
//! # Where this runs
//!
//! Callgrind is part of Valgrind, which is Linux/Unix-only (no Windows, no macOS
//! Apple-Silicon support). This bench therefore runs on the **Linux CI lane**
//! (`.github/workflows/ci.yml` → `iai-instructions` job, which installs Valgrind).
//! It is a `cargo bench` target, and benches are **not** nextest targets, so it
//! does not run — and is not required to run — under `just check` / nextest on the
//! M4 dev host. The crate still *compiles* this file on macOS (the dev-dep builds
//! fine cross-platform); only *executing* it needs Valgrind, which CI provides.
//! No `cfg`/feature kludge is needed or used.
//!
//! # What is counted
//!
//! Three representative hot-path quantities, each over the shared
//! `celnet_bench` fixtures so the instruction baseline tracks exactly the work
//! the latency budgets govern:
//!
//! * `price` — a single Garman-Kohlhagen present value.
//! * `greeks` — present value **plus** the full 13-Greek set in one pass (the
//!   quantity the §1.2 p50 ≤ 2 µs / p99 ≤ 10 µs budget is stated for).
//! * `batch_greeks` — the full Greek pass over the 64-strike representative
//!   surface slice (the per-tenor strike ladder a surface rebuild iterates over).
//!
//! The workspace's `missing_docs` lint is allowed *for this bench file only*:
//! iai-callgrind's `#[library_benchmark]` attribute expands each benchmark fn
//! into a generated `pub mod` we cannot attach a doc comment to. A `cargo bench`
//! target is a build artifact, not part of the crate's public API surface, so
//! suppressing the doc lint here is correct scoping — it does not touch any
//! library/public item and disables no correctness lint.
#![allow(missing_docs)]

use std::hint::black_box;

use iai_callgrind::{library_benchmark, library_benchmark_group, main};

use celnet_bench::{BATCH_STRIKES, representative_batch, representative_inputs};
use celnet_types::{Greeks, OptionType, VanillaInputs};
use celnet_vanilla::{greeks, price};

// Single Garman-Kohlhagen present value — the cheapest hot-path quantity.
//
// The `setup` builds the input *outside* the counted region (so allocation /
// fixture construction is not attributed to the call), and the input is
// `black_box`-ed across the boundary so the optimizer cannot fold the priced
// value to a constant. (iai-callgrind's `#[library_benchmark]` rejects any
// non-`bench` attribute between itself and the fn, including `///` doc
// comments, so these explanatory notes are plain `//` comments.)
#[library_benchmark]
#[bench::atm(setup = representative_inputs)]
fn bench_price(inputs: VanillaInputs) -> f64 {
    black_box(price(OptionType::Call, black_box(&inputs)))
}

// Present value **plus** the full 13-Greek set in one pass — the quantity the
// §1.2 hot-path latency budget governs. This is the headline instruction count.
#[library_benchmark]
#[bench::atm(setup = representative_inputs)]
fn bench_greeks(inputs: VanillaInputs) -> Greeks {
    black_box(greeks(OptionType::Call, black_box(&inputs)))
}

// The full Greek pass over the representative 64-strike surface slice — the unit
// of work a surface rebuild / portfolio repricing iterates over. The batch is
// built in `setup` (un-counted); only the priced loop is attributed.
#[library_benchmark]
#[bench::liquid_ladder(setup = representative_batch)]
fn bench_batch_greeks(batch: Vec<VanillaInputs>) -> f64 {
    debug_assert_eq!(batch.len(), BATCH_STRIKES);
    let mut acc = 0.0_f64;
    for inputs in black_box(&batch) {
        let g = greeks(OptionType::Call, black_box(inputs));
        acc += g.price + g.vega + g.gamma;
    }
    black_box(acc)
}

library_benchmark_group!(
    name = vanilla_hot_path;
    benchmarks = bench_price, bench_greeks, bench_batch_greeks
);

main!(library_benchmark_groups = vanilla_hot_path);
