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
