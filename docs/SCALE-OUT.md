Wrote `/Users/adrian/code/celeroption/docs/SCALE-OUT.md` — Celnet's distributed/horizontal scale-out architecture.

Contents:
- **§1 Governing principle** — scale *around* the hot path, never through it; latency table (kernel ~20–50 µs, DPDK ~7 µs, Raft quorum) vs the §1.2 p50 ≤ 2 µs budget; LMAX ~6M events/s single-thread ceiling.
- **§2 Partitioning** — shard = one pinned thread-per-core node; primary key = ccy-pair (all tenors co-resident), sub-shard by book/tenant; **HRW (rendezvous) hashing** with versioned gossiped partition map; co-residency-vs-netting tension.
- **§3 Stateless pricing replicas** — stateless HRW router tier + single-owner-shard authority + optional read-only reflectors for indicative fan-out.
- **§4 Surface/curve state distribution** — event-sourced replicated log → per-shard node-local `arc-swap<SurfaceSnapshot>`; cache coherency within node, eventual across shards; surface-epoch tagging on every quote/tick.
- **§5 MD fan-out & streaming** — in-proc LMAX-Disruptor SPMC (`celnet-distributor` / `disruptor-rs` / `jonhoo/bus`); networking tiers (`SO_REUSEPORT`+eBPF default → `io_uring`/XDP → DPDK as ADR); instrument-dedup subscription registry; gRPC+WS one-contract-two-transports.
- **§6 Backpressure** — conflation not buffering; separate audit (never-drop) vs telemetry (drop) rings.
- **§7 Blue-green across a fleet** — router-coordinated rolling drain wrapping per-shard `SO_REUSEPORT`+`rkyv`+arc-swap+SHADOW; no mixed-version window (single current contract).
- **§8 Failover/replay** — thin Raft/Aeron-Cluster log, hot standbys, ~2× election-timeout failover, Resync-from-last_seq, f64 CPU oracle determinism.
- **§9 Decision matrix** per workload (fat pinned node vs cluster) against §1.2 budgets + **decisive recommendation**.
- **§10 ASCII deployment topology**, **§11 fleet SLOs**, **§12 build-now-vs-defer list**, **§13 risks**.

All crate/method/competitor specifics from the bundle preserved; vendor-neutral purpose-named identifiers, single unversioned contract, OSS-only deps. Note: the doc references a fleet-SLO addition to `ARCHITECTURE.md §1.2` and aligns with `INTERFACES.md` — neither was modified.