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
//! # The regression gate — this bench CAN fail
//!
//! Every benchmark carries a real regression configuration (see
//! [`instruction_gate`]), so this is a *gate*, not a report:
//!
//! * **Soft limits** (relative, vs the previous run's counts stored under
//!   `target/iai/…`): instructions (`Ir`) +5 %, estimated cycles +10 %. When a
//!   previous run exists and the new count exceeds the limit, the
//!   `iai-callgrind` runner reports the regression and **exits non-zero**, so
//!   `cargo bench` — and therefore the CI job — fails.
//! * **Hard limits** (absolute per-benchmark instruction ceilings): checked on
//!   *every* run, including the very first one on a fresh `target/` where the
//!   soft limits have nothing to compare against. They are the always-on
//!   backstop against a catastrophic blow-up (an accidental opt-level drop, a
//!   per-call allocation, an inlining collapse into the libm kernels).
//!
//! **How the baseline updates:** the comparison point is simply the previous
//! run's callgrind output in `target/iai/…` (iai-callgrind renames it to
//! `*.old` and diffs against it). On CI the `iai-instructions` job persists
//! that directory through a dedicated `actions/cache` entry that is only saved
//! when the job is green — so the baseline is always "the last green run", a
//! regressing run never advances it, and merging an *intended* count change
//! re-baselines automatically on its first green run. Locally, delete
//! `target/iai` (or just re-run twice) to re-baseline after an intended change.
//!
//! The workspace's `missing_docs` lint is allowed *for this bench file only*:
//! iai-callgrind's `#[library_benchmark]` attribute expands each benchmark fn
//! into a generated `pub mod` we cannot attach a doc comment to. A `cargo bench`
//! target is a build artifact, not part of the crate's public API surface, so
//! suppressing the doc lint here is correct scoping — it does not touch any
//! library/public item and disables no correctness lint.
#![allow(missing_docs)]

use std::hint::black_box;

use iai_callgrind::{
    Callgrind, EventKind, LibraryBenchmarkConfig, library_benchmark, library_benchmark_group, main,
};

use celnet_bench::{BATCH_STRIKES, representative_batch, representative_inputs};
use celnet_types::{Greeks, OptionType, VanillaInputs};
use celnet_vanilla::{greeks, price};

// ---------------------------------------------------------------------------
// Regression limits (the gate).
// ---------------------------------------------------------------------------

/// Soft (relative) instruction-count limit: fail when `Ir` regresses more than
/// this percentage vs the previous run. Callgrind's `Ir` is deterministic for
/// identical code + toolchain (measurement noise ≈ 0), so 5 % — the
/// iai-callgrind documentation's canonical threshold — is far above noise yet
/// well below any meaningful code-level regression on a hot path of O(10²–10³)
/// instructions (one lost inline of a libm kernel alone is worth more). It
/// also tolerates the small codegen drift a rustc/LLVM point release can
/// introduce; a toolchain jump that moves the count by >5 % is exactly the
/// kind of event a human should look at (and re-baseline by merging).
const SOFT_INSTRUCTION_REGRESSION_PCT: f64 = 5.0;

/// Soft (relative) estimated-cycles limit. `EstimatedCycles` is derived from
/// the *simulated* cache hits/misses (cache simulation is iai-callgrind's
/// default), so unlike `Ir` it moves with data layout and alignment — which a
/// relink can shift without any source change. 2× looser than the `Ir` limit
/// so it only fires on a real locality regression (an accidental allocation, a
/// working-set blow-up), never on benign relinking.
const SOFT_ESTIMATED_CYCLES_REGRESSION_PCT: f64 = 10.0;

/// Absolute `Ir` ceiling for `bench_price` (single Garman-Kohlhagen PV: two
/// normal CDFs + exp/ln/sqrt + a handful of multiplies — order 10²–10³
/// instructions under `-O3`). Sized roughly an order of magnitude above the
/// expected count so legitimate codegen drift can never trip it; its job is
/// the no-baseline first run and catastrophic (≳10×) blow-ups, which the soft
/// 5 % gate cannot see without history.
const PRICE_INSTRUCTION_CEILING: u64 = 10_000;

/// Absolute `Ir` ceiling for `bench_greeks` (PV **plus** the full 13-Greek set
/// in one pass — shares the transcendental evaluations with the PV, adding
/// mostly plain arithmetic: order 10³ instructions). Same ~10× headroom
/// rationale as [`PRICE_INSTRUCTION_CEILING`].
const GREEKS_INSTRUCTION_CEILING: u64 = 25_000;

/// Absolute `Ir` ceiling for `bench_batch_greeks`: the 64-strike ladder is
/// exactly [`BATCH_STRIKES`] full-Greek passes plus trivial loop/accumulate
/// overhead, so the ceiling is derived, not re-estimated.
const BATCH_GREEKS_INSTRUCTION_CEILING: u64 = BATCH_STRIKES as u64 * GREEKS_INSTRUCTION_CEILING;

/// The soft (relative) regression limits shared by every benchmark: `Ir` +5 %,
/// `EstimatedCycles` +10 % vs the previous run (see the constants above for
/// the threshold rationale). Split out of [`instruction_gate`] so the group
/// default and the per-benchmark configs provably carry identical soft limits.
fn soft_regression_limits() -> Callgrind {
    Callgrind::default()
        .soft_limits([
            (EventKind::Ir, SOFT_INSTRUCTION_REGRESSION_PCT),
            (EventKind::EstimatedCycles, SOFT_ESTIMATED_CYCLES_REGRESSION_PCT),
        ])
        .clone()
}

/// The full per-benchmark gate: the shared soft limits **plus** the
/// benchmark's absolute `Ir` ceiling.
///
/// The complete configuration (soft + hard) is attached to *each* benchmark
/// because iai-callgrind resolves a more specific `Callgrind` regression
/// config by **replacement, not merge** (`Tool::update` → `update_option`):
/// splitting soft limits onto the group and hard limits onto the benchmark
/// would silently drop the soft gate. The group-level config (soft limits
/// only) exists purely as the default for any future benchmark added without a
/// per-benchmark config.
fn instruction_gate(hard_instruction_ceiling: u64) -> LibraryBenchmarkConfig {
    let mut gated = soft_regression_limits();
    gated.hard_limits([(EventKind::Ir, hard_instruction_ceiling)]);
    LibraryBenchmarkConfig::default().tool(gated).clone()
}

// Single Garman-Kohlhagen present value — the cheapest hot-path quantity.
//
// The `setup` builds the input *outside* the counted region (so allocation /
// fixture construction is not attributed to the call), and the input is
// `black_box`-ed across the boundary so the optimizer cannot fold the priced
// value to a constant. (iai-callgrind's `#[library_benchmark]` rejects any
// non-`bench` attribute between itself and the fn, including `///` doc
// comments, so these explanatory notes are plain `//` comments.)
#[library_benchmark(config = instruction_gate(PRICE_INSTRUCTION_CEILING))]
#[bench::atm(setup = representative_inputs)]
fn bench_price(inputs: VanillaInputs) -> f64 {
    black_box(price(OptionType::Call, black_box(&inputs)))
}

// Present value **plus** the full 13-Greek set in one pass — the quantity the
// §1.2 hot-path latency budget governs. This is the headline instruction count.
#[library_benchmark(config = instruction_gate(GREEKS_INSTRUCTION_CEILING))]
#[bench::atm(setup = representative_inputs)]
fn bench_greeks(inputs: VanillaInputs) -> Greeks {
    black_box(greeks(OptionType::Call, black_box(&inputs)))
}

// The full Greek pass over the representative 64-strike surface slice — the unit
// of work a surface rebuild / portfolio repricing iterates over. The batch is
// built in `setup` (un-counted); only the priced loop is attributed.
#[library_benchmark(config = instruction_gate(BATCH_GREEKS_INSTRUCTION_CEILING))]
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

// Group default: the soft limits alone. Every current benchmark overrides this
// with its full `instruction_gate(…)` (soft + hard) — the default exists so a
// future benchmark added to this group without a per-benchmark config is still
// soft-gated rather than silently ungated.
library_benchmark_group!(
    name = vanilla_hot_path;
    config = LibraryBenchmarkConfig::default().tool(soft_regression_limits());
    benchmarks = bench_price, bench_greeks, bench_batch_greeks
);

main!(library_benchmark_groups = vanilla_hot_path);
