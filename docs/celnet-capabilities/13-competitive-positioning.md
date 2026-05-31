<sub>**[Celnet Capabilities](../CELNET-CAPABILITIES.md)** › Competitive Positioning</sub>

# 13. Competitive Positioning

Celnet does not ask a desk to take its capability claims on faith. Every claim in this brochure is wired to a gated test, so "meets or beats" is something the platform proves on every build rather than something a sales deck asserts. The result is a posture that is easy to state and hard to argue with: Celnet **out-functions, out-intuits, and out-performs** across the FX-options stack, and it does so as a supply-chain-clean, cross-platform-deterministic system that drops into the estate you already run.

![Celnet versus the vendor-neutral capability landscape](../assets/celnet-capabilities/fig-12-capability-landscape.png)
*Figure 13.1 — The capability landscape, mapped against vendor-neutral archetypes. Celnet's footprint spans pricing depth, scenario and firm-wide risk, streaming edge, and client parity in one coherent product.*

### 13.1 An executable parity matrix — claims you can run

The cornerstone of Celnet's positioning is an **executable parity matrix**: capability claims rendered as gated integration tests inside a large multi-crate Rust workspace, backed by an extensive automated test suite. Each row is a capability — a delta convention, a smile model, a barrier method, a Greek, a risk roll-up, a streaming guarantee — expressed as code that must pass. Because the matrix runs in the same gates that govern every change, parity is *continuously demonstrated*, never merely claimed. Numerical results are validated to machine precision against an independent reference library, smile and surface models are checked against their arbitrage gates, and exotics are cross-validated across analytic, PDE, and Monte-Carlo methods so the answers agree before they ship.

This turns competitive evaluation into a reproducible exercise. A desk evaluating Celnet runs the matrix; the matrix is the evidence.

### 13.2 Out-functions

Celnet carries the full working surface of an options desk in a single product, end to end:

| Dimension | What Celnet delivers |
|---|---|
| Vanilla & Greeks | Garman-Kohlhagen pricing off the outright forward with separate domestic and foreign discount factors; the full FX desk Greek set computed in one pass, each finite-difference cross-validated |
| Conventions | All four delta conventions, a branch-aware strike-delta solver that handles the premium-adjusted call-delta maximum, and sign-correct ATMF / delta-neutral-straddle rules |
| Smile & surface | Vanna-Volga, SABR, SVI and SSVI with broker-strangle calibration, butterfly / calendar / vertical arbitrage gates, an arbitrage-free term structure in total variance, and a smile-model selector |
| Exotics | Digitals, one-touch / no-touch, double-no-touch / double-touch, and single & double barriers via analytic, Crank-Nicolson/Rannacher PDE, Philox Monte-Carlo, and a survival-weighted market overlay |
| Risk | Book-shaped scenario risk and a firm-wide OLAP position-fact cube with a cascading limit tree and entitlement-aware pre-aggregation |
| Edge & clients | One unversioned contract feeding a GUI, a typed Rust client SDK, an admin CLI, an Excel add-in, a WebSocket mirror, and a FIX engine — every surface bit-identical |
| Extensibility | An Open Quant SDK and tiered plugin host so bespoke and structured products extend coverage without forking the platform |

Where an incumbent archetype is strong in one band — deep pricing, or firm-wide risk, or a streaming venue, or a clean library — Celnet covers the whole span in one contract. A **closed terminal** gives a desk reach but not an embeddable, deterministic core it can extend; Celnet ships the Open Quant SDK so a desk runs its own private-IP models inside the engine. A **front-to-back platform** gives lifecycle coverage but treats FX-options analytics as a bolt-on; Celnet *is* the FX-options pricing system-of-record and slots into that lifecycle. A **data-venue** distributes prices but does not own the smile, the exotics, or the risk cube; Celnet does. A **modern library** prices cleanly but offers no streaming edge, no scenario and firm-wide risk, and no operational backbone; Celnet wraps the same numerical rigour in a production edge.

### 13.3 Out-intuits

Function is only half the contest; the other half is whether a desk can actually *use* it. Celnet's clients are two zooms of one model, not a scatter of disconnected screens. The GUI's Book and Risk views read the same position-fact cube, so a trader drills from an aggregated book row straight into a single position's scenario risk without changing tools or mental model. A scope breadcrumb, a pair navigator, a command palette, and consistent trend modes carry the same vocabulary across Ticket, Stream, Surface, Risk, and Book.

![Book aggregate drilling into position-level scenario risk](../assets/celnet-capabilities/shot-05-book-aggregate.png)
*Figure 13.2 — One model, two zooms: a net Book view whose rows drill straight through to position-level scenario risk.*

Critically, the front-end has no privileged path. The GUI uses the same APIs as any client, the Excel add-in runs no pricing in the cell, and a value is bit-identical across the GUI, the SDK, the CLI, and the spreadsheet. A desk can move from a streaming blotter to a structuring ticket to a marked surface to a risk grid — and reconcile every one of those numbers against the same server truth. That coherence is what "out-intuits" means in practice: fewer surfaces to learn, no contradictions to chase.

### 13.4 Out-performs

Celnet's hot core prices vanilla and Greeks at nanosecond scale, so on a streaming line **network framing, not computation, is the only meaningful latency floor**. A two-tier engine joins an async edge to pinned, zero-allocation hot cores over wait-free single-producer-single-consumer rings; state publication is lock-free and read-mostly; and blue-green handoff delivers hot upgrades with no downtime window. The pinned core stays log-, lock-, and allocation-free, with telemetry offloaded over a bounded, drop-on-full ring so observability never taxes the critical path.

Performance is held, not hoped: latency histograms capture the tail percentiles in a coordinated-omission-aware way, and benchmarks are regression-gated alongside the parity matrix, so a change that costs speed fails the gate.

![The performance ladder from in-core compute out to the wire](../assets/celnet-capabilities/fig-07-performance-ladder.png)
*Figure 13.3 — In-core pricing sits far below network framing on the latency ladder, so the wire is the floor and the maths is free.*

### 13.5 Supply-chain-clean and cross-platform deterministic

Celnet is built entirely on open-source, permissively-licensed components — no paid libraries, no proprietary solvers, no commercial runtime data dependencies — and that policy is enforced in the gates, not just intended. For a risk-managed trading desk this is a procurement and audit advantage: the dependency set is inspectable and license-clean by construction.

Determinism is the second pillar. A counter-based RNG is bit-identical between CPU and GPU, GPU results are reconciled against a high-precision CPU oracle, and a CPU SIMD fallback keeps the same answers where no accelerator is present. Plugin models replay bit-identically, and a marked surface is pinned by version so a stale or unknown version is rejected rather than silently re-deriving against live data. The same number reproduces across machines, accelerators, and client surfaces — the foundation of any defensible mark.

### 13.6 Adapts into the estate you already run

Positioning is not only about a head-to-head; it is about fit. Celnet is the FX-options pricing system-of-record inside the Celer trade lifecycle and runs across Standalone, Hybrid, and CelerIntegrated deployment modes via reversible adapter swaps on its seam traits — never a rewrite. The same market-data seam that powers those modes ingests external products and feeds as integration targets: venue and aggregator feeds such as **Fenics**, **Bloomberg**, **Refinitiv**, and **EBS** are adapter destinations, demonstrating that Celnet meets a desk where its data already lives.

![Celnet's adaptability across deployment modes and external feeds](../assets/celnet-capabilities/fig-10-deployment-modes.png)
*Figure 13.4 — Reversible adapter swaps move Celnet between Standalone, Hybrid, and CelerIntegrated modes; the same seam ingests external market-data feeds.*

### 13.7 The positioning, in one line

Celnet meets the closed terminal on reach, the front-to-back platform on lifecycle fit, the data-venue on distribution, and the modern library on numerical rigour — and it beats each on the dimension the others lack, in one unversioned contract, proven by a parity matrix you can run, on a supply-chain-clean, deterministic foundation.

---
<sub>[← Celer Trader & Estate Integration](12-celer-integration.md)  ·  **[Contents](../CELNET-CAPABILITIES.md)**  ·  [Engineering Rigor & Assurance →](14-engineering-rigor.md)</sub>
