---
name: scale-and-performance
description: "Celnet must scale to IB-sized portfolios and stream to HFT counterparties, using latest academic research."
metadata: 
  node_type: memory
  type: feedback
  originSessionId: b315eccc-f521-4987-b5b5-1a21d5710edb
---

Scale and performance are **first-class requirements** for every Celnet component: it must handle **investment-banking-sized portfolios** and **stream prices to high-performance counterparties** within the latency/throughput budgets in `docs/ARCHITECTURE.md` §1.2 (e.g. p99 ≤ 10µs vanilla+Greeks, ≥1M updates/s/core).

**Why:** User directive (30 May 2026).

**How to apply:** Design for horizontal scale-out and large many-instrument/many-tenor batch from the start; choose algorithms/data structures for scale (vectorized/SIMD, GPU batch, lock-free streaming, zero-alloc hot path). **Leverage the latest academic research** to optimize and scale wherever it helps, and cite the paper/method in code comments and `docs/`. Constrained by [[no-commercial-products]] (free/OSS methods only) and [[no-mocks-policy]] (complete, validated). See [[celnet-mission]].
