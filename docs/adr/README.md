# Architecture Decision Records — index

The numbered registry of Celnet's architectural decisions. **The number is the identity**:
never reuse one, never renumber a published record, and add new records by taking the next
free number in this table.

> This index exists because the registry drifted: `ADR-0013` was issued twice (2026-07-01)
> and `ADR-0017` was skipped. Reconciled 2026-08-17 — the acceleration record kept `0013`
> because it is cited from code (`celnet-risk-cube/src/nonadditive.rs`,
> `celnet-risk-accel/src/lib.rs`); the front-end record moved into the vacant `0017`.

| # | Decision | Status |
|---|---|---|
| [0007](ADR-0007-one-unversioned-contract.md) | One unversioned wire contract — evolve in place, zero legacy | Accepted |
| [0008](ADR-0008-multi-asset-carry-architecture.md) | Multi-asset: identity / carry-producing market / agnostic payoff | Accepted |
| [0009](ADR-0009-edge-wire-codec.md) | Network price-edge codec — fixed-offset zero-copy `EdgeFrame` | Accepted (Active target) |
| [0010](ADR-0010-converge-fi-rates-onto-carry-seam.md) | Converge FI rates onto the carry seam as a term structure | Superseded by ADR-0018 & ADR-0020 |
| [0011](ADR-0011-celer-estate-ingress.md) | Celer-estate ingress + vendor FX-options market-data ingestion | Accepted |
| [0012](ADR-0012-unified-gbsm-kernel.md) | One unified generalized-BSM carry kernel | Accepted |
| [0013](ADR-0013-acceleration-extensibility-wiring.md) | Acceleration & extensibility wiring (GPU + plugin surface) | Accepted / Active (`celnet-risk-accel` live) |
| [0014](ADR-0014-generated-wire-contract.md) | Generated wire contract & client parity | Accepted (Target architecture) |
| [0015](ADR-0015-replicated-state-elastic-fleet.md) | Replicated state & elastic fleet | Accepted (Target architecture) |
| [0016](ADR-0016-governance-risk-and-latency-gate.md) | Governance & pre-trade risk wiring + latency SLO gate | Accepted (Active target) |
| [0017](ADR-0017-single-front-end-market-making-loop.md) | One sell-side front-end — persona lenses over the market-making loop | Accepted |
| [0018](ADR-0018-fixed-income-as-a-new-asset-class-leaf.md) | Fixed income (cash bonds + credit) as a new asset-class leaf | Accepted & Implemented (`celnet-bond` live) |
| [0019](ADR-0019-credit-pricing-leaf.md) | Credit (survival curves + CDS + credit-risky bonds) as an FI leaf | Accepted (Target leaf, non-blocking) |
| [0020](ADR-0020-central-cross-asset-pricing-risk-contract.md) | One central cross-asset pricing/risk contract | Accepted & Implemented (`celnet-core::contract` live) |
| [0021](ADR-0021-uniform-asset-class-architecture.md) | Uniform architecture for every asset class, on every surface | Accepted |
| [0022](ADR-0022-fi-aggregated-book.md) | FI aggregated book — admin-defined multi-LP consolidation | Accepted |

**Next free number: 0023.**

`ATTESTATION.md` records who verified which decision and when; it is not itself an ADR.

## Numbers 0001–0006

Not present in this repository. The design corpus that preceded the ADR series lives in
`docs/ARCHITECTURE.md` and the topic documents beside it; the numbered series begins at 0007.
