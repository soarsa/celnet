<sub>**[Celnet Capabilities](../CELNET-CAPABILITIES.md)** › Engineering Rigor & Assurance</sub>

# 14. Engineering Rigor & Assurance

A pricing platform is only as trustworthy as the evidence behind its numbers. Celnet is engineered so that every headline claim — correctness, determinism, latency, resilience — is *earned and continuously re-proven*, not asserted. The same automated gates that let the desk trade on a price let the architecture team trust the system under load and through upgrades. This closing section sets out how the platform holds its line.

![Celnet engineering assurance: golden-oracle validation, parity matrix, mutation and property testing, deterministic replay, and the resilient hot core](../assets/celnet-capabilities/fig-12-capability-landscape.png)
*Celnet's assurance landscape — every capability is backed by an executable proof that runs on every change.*

### 14.1 Correctness, validated to machine precision

Celnet's analytics are validated against an **independent reference library** (QuantLib) used as a golden oracle. Vanilla prices and the full FX desk Greek set, both digital styles, and all eight barrier variants are checked to machine precision against frozen reference tables — independent maths, independent implementation, agreeing to the last bits. Beyond the oracle, every Greek is **finite-difference cross-validated**: the analytic closed-form sensitivity and its numerical bump-and-revalue twin must agree, so each of price, spot- and forward-delta, gamma, vega, theta, both rhos, vanna, volga, charm, speed, zomma and color is proven, not presumed.

The exotics catalogue is cross-validated *internally* as well as against the oracle: for each instrument the analytic reflection method, the Crank–Nicolson PDE with a Rannacher start-up, and the Philox Monte-Carlo path engine are required to agree, so three independent numerical routes triangulate the same price. The smile and surface engine carries its own correctness gates — butterfly density non-negativity, calendar total-variance monotonicity, and vertical no-arbitrage — enforced as the surface is calibrated, so a marked surface that violates no-arbitrage is rejected at the source rather than priced from.

| Assurance technique | What it proves |
|---|---|
| Golden-oracle validation against an independent reference library | Prices and Greeks agree to machine precision with independent maths |
| Finite-difference Greek validation | Every analytic sensitivity matches its numerical twin |
| Cross-method exotics triangulation (analytic ~ PDE ~ Monte-Carlo) | Three independent routes converge on one price |
| Arbitrage gates on calibration | No surface that breaks butterfly / calendar / vertical no-arbitrage is admitted |

### 14.2 Property and mutation testing

Point checks confirm what an engineer thought to test; **property-based testing** confirms what they did not. Celnet asserts the invariants that must hold across whole input spaces — put–call parity, monotonicities, sign conventions, the branch behaviour of the strike↔delta solver across the premium-adjusted call-delta maximum — and the harness searches for counterexamples rather than waiting for one in production.

**Mutation testing** then audits the tests themselves: the suite deliberately perturbs the implementation and confirms the test suite catches the change. A test that survives a mutated price is a test that was not really guarding the price, and mutation analysis surfaces exactly those gaps. Together, property and mutation testing keep the **extensive automated test suite** honest as the platform evolves.

### 14.3 An executable parity matrix

Capability claims are not prose in a brochure alone — they are codified as an **executable parity matrix** that renders each "meets or beats" claim as a gated integration test. Every claim about coverage, conventions, surface behaviour, risk and streaming is exercised against the running system on every change, so the comparison against the vendor-neutral archetypes — a closed terminal, a front-to-back platform, a data-venue, a modern library — is *continuously proven rather than asserted*. If a capability regresses, the matrix fails the gate; the claim and the code never drift apart.

### 14.4 Deterministic, cross-platform behaviour

Determinism is a first-class guarantee. Celnet's counter-based random number generator is **bit-identical between CPU and GPU**, and GPU results are reconciled against a high-precision CPU oracle, so the same scenario produces the same number whether it runs on the accelerator or the CPU SIMD fallback. Plugin execution is equally deterministic: the WebAssembly sandbox supports **bit-identical replay**, so a model's output can be reproduced exactly from its inputs — the foundation for reproducible risk, auditable pricing, and confident debugging across heterogeneous hardware.

![Deterministic execution and the tiered plugin host — replayable, capability-scoped, bit-identical](../assets/celnet-capabilities/fig-05-plugin-tiers.png)
*Bit-identical RNG across CPU and GPU and replayable plugins make every Celnet number reproducible by construction.*

### 14.5 Zero-cost observability

The platform is fully instrumented without taxing the hot path. Latency is captured in **tail-percentile histograms** that are coordinated-omission aware, and telemetry is offloaded over a **bounded, drop-on-full ring** so the pinned hot core stays log-, lock- and allocation-free. Operators get the p50/p99/p99.9 picture they need for mission-critical running; the maths pays nothing for it. Performance itself is held by **regression-gated benchmarks**, so a change that would quietly slow pricing fails the gate before it reaches a desk.

### 14.6 Supply-chain cleanliness

Celnet is built exclusively on **open-source, permissively-licensed** software — no commercial libraries, solvers, or proprietary runtime data dependencies. The dependency set is policy-enforced in the gate: licences are checked against an approved set, advisories are scanned, and banned or duplicate dependencies are rejected automatically. The result is a platform a bank can vet, deploy, and own end-to-end without commercial entanglement.

### 14.7 A resilient core: zero-allocation, hot-upgradable, durable

The runtime is engineered to keep trading through faults and upgrades. A **zero-allocation hot core** — wait-free single-producer/single-consumer rings feeding pinned cores, lock-free read-mostly state publication via an atomically swapped market-state snapshot and a single-writer seqlock top-of-book — does no allocation, locking or logging on the pricing path. **Blue-green zero-downtime state handoff** lets a new build take over from a running one without dropping the book, so hot upgrades are a routine operation rather than an outage. Underneath, a **durable, checksummed, append-only journal** provides clean crash recovery: a torn final record heals to the last good entry on restart, and the book and its repricing come back bit-for-bit.

![The resilient engine: pinned zero-allocation hot core, lock-free publication, blue-green handoff, durable journal](../assets/celnet-capabilities/fig-13-engine-concurrency.png)
*A zero-allocation hot core, blue-green hot upgrade, and a durable journal keep Celnet pricing through faults and deployments.*

### 14.8 How it all earns the claim

These mechanisms are not independent niceties — they compose into a single discipline. The oracle and finite-difference gates earn *correctness*; property and mutation testing earn *robustness*; the parity matrix earns *competitiveness*; bit-identical RNG and replay earn *determinism*; zero-cost observability and regression-gated benchmarks earn *performance*; the zero-allocation core, blue-green handoff and durable journal earn *resilience*; supply-chain policy earns *ownership*. Built as a large multi-crate Rust workspace with an extensive automated test suite, Celnet is a platform whose every number a desk can trade on, a quant can reproduce, and an architect can deploy — because the proof runs every time the code does.

---
<sub>[← Competitive Positioning](13-competitive-positioning.md)  ·  **[Contents](../CELNET-CAPABILITIES.md)**  ·  [Back to Overview →](../CELNET-CAPABILITIES.md)</sub>
