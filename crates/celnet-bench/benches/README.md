# celnet-bench — measured latency vs. budget

`divan` micro-benchmarks that turn the pricing-latency claims in
`docs/ARCHITECTURE.md` §1.2 into reproducible proof. Run with:

```bash
source "$HOME/.cargo/env" && cargo bench -p celnet-bench
```

`divan` reports `fastest` (min), `median`, `mean` and `slowest` per op. The
**median** is what we compare against the documented **p50** budget; the **min**
approximates the warm-cache floor.

## Budgets under test (`docs/ARCHITECTURE.md` §1.2)

| Workload | Budget | Bench |
|---|---|---|
| Vanilla price + full Greeks (cached surface), hot path | p50 ≤ 2 µs, p99 ≤ 10 µs | `vanilla::price_plus_full_greeks` |
| Streaming quote throughput | ≥ 1M updates/s/core | `batch::batch_price[_plus_greeks]` |
| Surface rebuild (single pair, all tenors) | p99 ≤ 150 µs | `batch::*` (per-slice total) |

## Reference run (Apple M4, `aarch64-apple-darwin`, `bench` profile, single core)

| Bench | median | min | per-option (median) | vs. budget |
|---|---|---|---|---|
| `vanilla::price_only` | 40.6 ns | 40.6 ns | 40.6 ns | — |
| `vanilla::price_plus_full_greeks` | 23.4 ns | 23.1 ns | 23.4 ns | **~85× inside** the 2 µs p50 |
| `vanilla::price_plus_full_greeks_put` | 23.2 ns | 22.8 ns | 23.2 ns | symmetric with call |
| `batch::batch_price` (64 strikes) | 3.33 µs | 3.29 µs | ~52 ns | 19.2 Mitem/s |
| `batch::batch_price_plus_greeks` (64 strikes) | 6.75 µs | 6.12 µs | ~105 ns | 9.48 Mitem/s → **~9.5×** the 1M/s target |

Notes:

- The single-option `price_plus_full_greeks` median benefits from divan running
  many iterations per sample on a register-resident input; the per-option figure
  in the batched bench (~105 ns) is the more conservative, cache-realistic
  amortized cost a surface/portfolio sweep pays — still **~19×** inside the 2 µs
  per-option p50.
- A full 64-strike price+Greeks slice completes in ~6.75 µs median, comfortably
  inside the 150 µs surface-rebuild p99 budget.
- Inputs are realistic `celnet_vanilla::VanillaInputs` from the shared fixtures
  in `src/lib.rs` (`representative_inputs` / `representative_batch`), guarded with
  `black_box` so the optimizer cannot elide the work. The same fixtures are
  asserted sane by the crate's unit tests, so the benchmarked workload is the
  tested workload.
- Absolute numbers are hardware-dependent; the **ratios to budget** are the
  durable claim. Re-run on the target host to re-baseline.

---

# In-core ABSOLUTE §1.2 truth-gate (`core_load`)

`divan`'s headline is a **median**. `docs/ARCHITECTURE.md` §1.2 commits the
hot-path budget as **absolute per-option percentile ceilings** — **p50 ≤ 2 µs,
p99 ≤ 10 µs, p99.9 ≤ 25 µs**. The `core_load` binary
(`src/bin/core_load.rs`, logic in `src/core_load.rs`) turns those ceilings into a
*measured, asserted* gate: it times **each individual** `celnet_vanilla::greeks`
call (price + full 13-Greek set — the actual quantity the budget governs) into a
coordinated-omission-aware `hdrhistogram::Histogram`, reports the full tail
(p50/p99/p99.9/p99.99/max) + single-core throughput, and **exits non-zero if any
measured percentile exceeds its §1.2 ceiling**.

```bash
# run the gate (exits non-zero on any §1.2 breach):
source "$HOME/.cargo/env" && cargo run --release -p celnet-bench --bin core_load
# refresh the committed §1.2 snapshot:
source "$HOME/.cargo/env" && cargo run --release -p celnet-bench --bin core_load -- \
  crates/celnet-bench/baselines/core_path.json
# fast smoke sizing (still asserts §1.2):
source "$HOME/.cargo/env" && cargo run --release -p celnet-bench --bin core_load -- --quick
```

**Why no workaround is needed.** The in-core op is ~tens of ns — ~400× under the
p99 ≤ 10 µs budget. The only thing that can inflate a *sampled* percentile is
measurement jitter (thread migration, frequency scaling, a preemption tick), so
the measurement removes it at the source: request the max scheduling QoS/priority
(`thread-priority`, safe API — no `unsafe`), pin to a core where the OS allows it
(`core_affinity`), discard a 2M-call warmup, then take **10M** timed samples, and
apply HdrHistogram's post-recording CO correction (`clone_correct`) so a stall can
only make the tail *more* pessimistic. The genuine OS-dependent deep tail (a stray
preemption) appears only at **p99.99+** — a *deeper* percentile than the §1.2 gate
— and is reported transparently, never used to weaken the gate.

## Reference run (Apple M4, `aarch64-apple-darwin`, `--release`, single core)

Workload `vanilla_price_plus_full_greeks`, 10M timed samples (committed snapshot:
`baselines/core_path.json`):

| Metric | Measured | §1.2 budget | Margin |
|---|---|---|---|
| throughput | **~13.0 M opt/s/core** | (≥ 1M/s) | ~13× |
| min | 1 ns | — | — |
| **p50** | **42 ns** | ≤ 2 000 ns | **~48×** inside |
| **p99** | **125 ns** | ≤ 10 000 ns | **~80×** inside |
| **p99.9** | **~1.0–1.4 µs** | ≤ 25 000 ns | **~18–25×** inside |
| p99.99 | ~7.8–8.1 µs | (deeper than the §1.2 gate; reported only) | — |
| max | ~16–18 µs | — | — |

Notes:

- **All three §1.2 budgets PASS, measured, with large margin** — the gate asserts
  the real committed numbers, no percentile relaxed.
- `pinned: false` on Apple Silicon is **honest**: macOS does not expose
  per-thread CPU affinity, so `core_affinity::set_for_current` is a no-op there
  (the binary reports this rather than faking success). Priority elevation does
  apply (`elevated-priority: true`). On Linux CI both pinning and priority apply.
  The budgets pass either way — pinning only tightens an already-passing tail.
- The min of `1 ns` is the `Instant` timer-resolution floor for an op faster than
  a few ns of overhead; the body-of-distribution figures (p50/p99) are the
  meaningful ones and are well-resolved by the 10M samples.

`bench_gate` runs this same in-core §1.2 absolute gate as **arm 1**; it also runs
the surface-rebuild §1.2 absolute gate (below) as **arm 1b**, the wire-path
relative regression gate as **arm 2**, and the fleet §11 loopback SLO gate as
**arm 3** — all four must pass.

---

# Surface-rebuild ABSOLUTE §1.2 truth-gate (`surface_rebuild`)

`docs/ARCHITECTURE.md` §1.2 commits a second absolute latency budget: **surface
rebuild on a market tick (single pair, all tenors) p99 ≤ 150 µs** — mechanism
*"VV/SSVI recompute over pre-allocated arenas; SIMD slice math."* The
`surface_rebuild` binary (`src/bin/surface_rebuild.rs`, logic in
`src/surface_rebuild.rs`) turns that ceiling into a *measured, asserted* gate with
the **same** low-jitter, coordinated-omission-aware discipline as `core_load`.

**What is gated.** FX surfaces are **sticky-delta** (`docs/ANALYTICS-SPEC.md` §3):
on a spot/forward tick the surface is *recomputed* in delta space — the exact word
§1.2 uses. The gated quantity is that per-tick recompute: re-derive the forwards
and re-evaluate the calibrated **all 11 standard tenors** (ON…2Y) across the mark
grid (11 strikes/tenor = **121 grid points**), for **both** the Vanna-Volga (VV)
and SSVI models §1.2 names. Each full recompute is timed into a CO-aware
`hdrhistogram::Histogram`; the gate exits non-zero if either model's measured p99
exceeds 150 µs.

**Honest companion — the cold calibration.** The one-off from-broker-quotes
calibration (the iterative market→smile strangle fixed point + convention-aware
delta→strike root-solves + the SSVI damped Gauss-Newton fit) runs on a **quotes**
change, not on every spot tick — it is a multi-millisecond operation by nature and
a different budget. The bench measures and **reports** it (~2.2 ms VV / ~3.4 ms
SSVI on the M4) but does **not** §1.2-gate it: surfaced, never hidden, never
mis-gated against a budget that does not govern it.

```bash
# run the gate (exits non-zero on any §1.2 breach):
source "$HOME/.cargo/env" && cargo run --release -p celnet-bench --bin surface_rebuild
# refresh the committed §1.2 snapshot:
source "$HOME/.cargo/env" && cargo run --release -p celnet-bench --bin surface_rebuild -- \
  crates/celnet-bench/baselines/surface_rebuild.json
# fast smoke sizing (still asserts §1.2):
source "$HOME/.cargo/env" && cargo run --release -p celnet-bench --bin surface_rebuild -- --quick
```

## Reference run (Apple M4, `aarch64-apple-darwin`, `--release`, single core)

Workload `per_pair_all_tenors_surface_rebuild` (EUR/USD, 11 tenors × 11 strikes),
200k timed recomputes (committed snapshot: `baselines/surface_rebuild.json`):

| Model | p50 | **p99** | p99.9 | §1.2 budget (p99) | Margin |
|---|---|---|---|---|---|
| **MarketHedge (VV)** | ~16.0 µs | **~19.6 µs** | ~23.5 µs | ≤ 150 µs | **~7.7×** inside |
| **ParametricSurface (SSVI)** | ~6.3 µs | **~7.8 µs** | ~16.3 µs | ≤ 150 µs | **~19×** inside |

Both models PASS the §1.2 p99 ≤ 150 µs ceiling, measured, with margin — no
percentile relaxed. (Cold calibration, reported un-gated: ~2.2 ms VV / ~3.4 ms
SSVI.) `pinned: false` on Apple Silicon is honest (macOS exposes no per-thread
affinity); priority elevation applies, and the budget passes either way.

---

# Instruction-count gate (`iai_instructions`, Linux + Valgrind)

The wall-clock gates above measure *time* (the contract's unit), but wall-clock
floats with host load and so a CI gate on it must carry a generous tolerance.
The complementary **instruction-count** gate (`benches/iai_instructions.rs`, via
`iai-callgrind`/Callgrind) measures a **deterministic, machine-independent**
quantity: retired instructions / cache accesses / estimated cycles per pricing
call. That makes it the right primitive for catching a *code-level* regression
(an extra branch, a lost inlining, an accidental allocation) the moment it lands,
with a tight baseline and no flake. It counts three quantities over the shared
fixtures: `price` (single PV), `greeks` (PV + full 13-Greek set — the §1.2
quantity), and `batch_greeks` (the 64-strike slice).

Callgrind is part of **Valgrind (Linux/Unix-only — no Windows, no macOS Apple
Silicon)**, so this is a dedicated **Linux CI lane** (`.github/workflows/ci.yml`
→ `iai-instructions`). Benches are **not** nextest targets, so adding the
`iai-callgrind` dev-dep does **not** affect `just check` / nextest on the M4 dev
host — the bench file only *compiles* there (which is fine; only *running* needs
Valgrind). No `cfg`/feature kludge is used.

```bash
# Linux + Valgrind only:
source "$HOME/.cargo/env" && cargo bench -p celnet-bench --bench iai_instructions
```

---

# Wire-path latency under load (end-to-end proof)

The micro-benchmarks above measure the **pinned hot core in isolation** (no
wire): price + full Greeks in ~23 ns. That is the *compute floor*. The figure a
real counterparty observes — and the figure the GA gating caveat is about — is
the **wire-path round-trip**: dialing the running service edge over the network
and pricing an option *while the edge is under sustained streaming + RFQ load*.

The `wire_load` binary (`src/bin/wire_load.rs`, harness in `src/wire.rs`) is that
proof. It spins the **real** `celnet_server::Edge` in-process on an ephemeral
`127.0.0.1:0` port (the same construction the server's own integration tests
use — celnet-bench reads the public `celnet-server` API, it does not modify the
edge), then:

1. opens N concurrent RFS streaming subscriptions, each draining
   snapshot/delta/heartbeat messages continuously (the **background load** that
   keeps the async runtime, the SPSC core rings, and the pricing core busy); and
2. fires a fixed budget of `RequestQuote` round-trips from C concurrent client
   tasks sharing one gRPC channel, timing **each** round-trip client-side with a
   monotonic clock into an `hdrhistogram::Histogram` (coordinated-omission aware).

Run it (always under a shell timeout — the harness self-terminates at a request
budget *or* a hard wall-clock cap, whichever comes first):

```bash
source "$HOME/.cargo/env" && timeout 200 cargo run --release -p celnet-bench --bin wire_load
```

## In-core vs wire-path (the two numbers, kept distinct)

| Path | What it includes | Apple M4 figure |
|---|---|---|
| **In-core** (`vanilla::price_plus_full_greeks`) | pure pricing compute, register-resident input | **~23 ns** p50 |
| **Wire-path** (`wire_load`, loopback gRPC under load) | full tonic/gRPC/HTTP-2 client+server stack + codec + async⇄core SPSC hop + pricing | see below |

The wire-path is ~4 orders of magnitude above the in-core floor because it is
dominated by the **gRPC/HTTP-2 framing + tonic codec + the closed-loop client
concurrency**, not by the pricing math. This is the honest, expected shape: the
core is not the bottleneck on the wire.

## Reference run (Apple M4, `aarch64-apple-darwin`, `--release`, loopback)

Workload `rfq_under_rfs_load`, **published-proof** sizing (64 concurrent RFS
subscriptions + 32 concurrent RFQ client tasks, 100 000 timed RFQ round-trips):

| Metric | RFQ wire-path round-trip |
|---|---|
| throughput | **~43.8 k RFQ/s** (sustained, alongside ~33 k streamed RFS msgs) |
| min | 100.7 µs |
| **p50** | **715.3 µs** |
| **p99** | **1 212.4 µs** |
| **p99.9** | **1 784.8 µs** |
| p99.99 | 7 139.3 µs |
| max | 7 299.1 µs |

Workload `rfq_under_rfs_load`, **CI-gate** sizing (32 RFS subs + 16 RFQ tasks,
20 000 timed round-trips — this is what `bench_gate` re-measures and the
committed baseline `baselines/wire_path.json` records):

| Metric | RFQ wire-path round-trip |
|---|---|
| throughput | ~40.1 k RFQ/s |
| min | 90.2 µs |
| **p50** | **388.4 µs** |
| **p99** | **685.1 µs** |
| **p99.9** | **1 188.9 µs** |
| p99.99 | 2 105.3 µs |

### Honesty / scope of the claim

- These are **loopback** numbers on the dev M4: they include the full
  gRPC/HTTP-2 client+server stack, tonic codec, the async⇄core SPSC hop, and the
  pricing compute — **everything except the physical NIC and the network**. They
  are therefore an **upper bound on the compute+framing cost** and a **lower
  bound on real cross-host wire latency** (which adds NIC + switch + propagation,
  typically tens to hundreds of µs more on a tuned LAN).
- The p50/p99 are inflated by the **closed-loop concurrency** (C client tasks
  each blocking on their own round-trip): they measure latency *at the offered
  throughput*, not an unloaded single-request floor (the `min` ~90–100 µs is the
  closer proxy for that floor). The durable claims are the **tail shape** and the
  **ratio of in-core to wire-path** (the core is ~4 orders of magnitude below the
  framing cost — i.e. pricing is never the bottleneck), not an absolute
  cross-host headline.
- Absolute numbers float with the host's core count and concurrent load. The
  committed baseline is re-baselined per host; the CI gate (below) is tuned to a
  generous tolerance so it catches a *structural* regression, not host jitter.

## CI bench-regression gate

`bench_gate` (`src/bin/bench_gate.rs`) re-runs the CI-sized wire-path load
against a fresh in-process edge and compares the measured RFQ percentiles to the
committed baseline `baselines/wire_path.json` at a relative tolerance (default
`+100%`, i.e. fail if a gated percentile exceeds `2× baseline`). It exits
non-zero — failing CI — on any breach. It is bounded the same way as the load
binary, so it can never hang the pipeline.

```bash
# refresh the committed baseline on this host:
source "$HOME/.cargo/env" && timeout 200 cargo run --release -p celnet-bench \
  --bin wire_load -- --ci crates/celnet-bench/baselines/wire_path.json

# run the gate (CI does this on every PR, see .github/workflows/ci.yml):
source "$HOME/.cargo/env" && timeout 200 cargo run --release -p celnet-bench --bin bench_gate
# or: just bench-gate
```

The gate is intentionally a **slowdown** detector only (it does not gate
throughput, which is core-count- and CI-load-sensitive in a way the latency tail
is not). Tune the tolerance via `bench_gate <baseline.json> <tolerance>`.
