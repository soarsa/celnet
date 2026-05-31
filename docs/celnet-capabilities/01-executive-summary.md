<sub>**[Celnet Capabilities](../CELNET-CAPABILITIES.md)** › Executive Summary — Celnet at a Glance</sub>

# 1. Executive Summary — Celnet at a Glance

Celnet is a state-of-the-art FX-options pricing and risk platform — a Celer Technologies product — engineered to out-function, out-intuit, and out-perform the incumbent terminals, front-to-back platforms, and quant libraries a derivatives desk relies on today. It prices the full vanilla and first-generation exotics catalogue, marks and calibrates live smiles and surfaces, runs firm-wide scenario and limit risk, and streams two-way prices to high-performance counterparties — all behind a single clean contract and a modern trader experience. Where incumbents force a desk to stitch a closed terminal to a separate library to a separate booking system, Celnet is one coherent system: the maths, the marking workflow, the risk cube, and the lifecycle integration share one model and one source of truth.

The platform is built as a large multi-crate Rust workspace with an extensive automated test suite, and it is supply-chain-clean — open-source, permissively-licensed components only — and cross-platform deterministic. In-core pricing is so fast that network framing, not computation, is the only meaningful latency, so a desk feels a terminal that simply keeps up. Celnet ships as the FX-options pricing system-of-record inside the Celer trade lifecycle, and runs equally well standalone, so it slots into the estate a desk already has rather than demanding a rewrite.

**Headline capabilities at a glance:**

- **Real-time, microsecond-class pricing with the full FX desk Greek set in one pass** — Garman-Kohlhagen vanillas off the outright forward, every sensitivity (delta, gamma, vega, theta, both rhos, vanna, volga, charm, speed, zomma, color) computed together and finite-difference cross-validated, with all four delta conventions and a branch-aware strike↔delta solver.
- **A complete marking and surface workflow** — Vanna-Volga, SABR, SVI and SSVI smile models with broker-strangle calibration, a smile-model selector, arbitrage gates, and an arbitrage-free term structure interpolated in total variance — plus a first-generation exotics catalogue (digitals, touches, double-no-touch/double-touch, single and double barriers) cross-validated across analytic, PDE, and Monte-Carlo and validated to machine precision against an independent reference.
- **An Open Quant SDK for private models** — desks run their own proprietary pricing and calibration models inside the engine, sandboxed, deterministic, and hot-loadable, without forking Celnet.
- **One API-first contract behind every surface** — the GUI, the Rust client SDK, the admin CLI, and the Excel `CELNET.*` add-in all consume the same unversioned contract; the front-end uses the same APIs as any client, so a value is bit-identical everywhere it appears.
- **Firm-wide risk** — book-shaped scenario risk (shock grids, bucketed vega per tenor × delta, cross-gamma, theta-roll, versioned marked surfaces) and an OLAP position-fact cube with common-numeraire netting, additive roll-up, per-node VaR/ES re-derivation, a cascading limit tree, and entitlement-aware aggregation.
- **Native Celer-lifecycle integration** — Celnet injects FX-option price, Greeks, and surface into the Celer price path and option risk into the risk and position path, across standalone, hybrid, and fully-integrated deployment modes, with a real FIX engine, a conflating egress governor, and vendor-feed normalization that adapts to external products and feeds such as Fenics, Bloomberg, Refinitiv, and EBS.

This brochure walks each of these in turn, supported by a figure index of system, architecture, and workflow diagrams (`fig-01` … `fig-13`) and live screenshots of the running platform (`shot-01` … `shot-10`). The capability landscape below maps the whole surface area at a glance.

![Celnet capability landscape](../assets/celnet-capabilities/fig-12-capability-landscape.png)
*Figure 1 — The Celnet capability landscape: quant coverage, engine and performance, risk, the Open Quant SDK, the API-first edge, the client suite, and Celer-lifecycle integration, unified behind a single contract.*

---
<sub>[← Overview](../CELNET-CAPABILITIES.md)  ·  **[Contents](../CELNET-CAPABILITIES.md)**  ·  [Capability Map →](02-capability-map.md)</sub>
