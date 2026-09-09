# Celnet Sovereign Demonstration Suite (September 2026)

This directory contains the cleanly separated demonstration runner, real-data streaming engine, scenario fixtures, and state-of-the-art interactive visual cockpit for Celnet.

---

## 1. Quick Start

### A. Run Full End-to-End Live Demonstration (Terminal)
```bash
./demo/run.sh
# or equivalently:
cargo run --release -p celnet-demo -- --act=all
```

### B. Launch Live Real-Time HTTP & WebSocket Demonstration Engine (Port 9876)
```bash
./demo/run.sh --serve
# with automatic browser launch:
./demo/run.sh --serve --web
```
Starts an async HTTP and WebSocket server streaming live real-time tick computations, Greeks from `celnet_price_vanilla`, CME MDP 3.0 book updates, and hardware nanosecond latency measurements at 20 Hz.

### C. Export Authentic Engine Dataset (Zero Mocks)
```bash
./demo/run.sh --export-data
```
Directly evaluates Celnet's core Rust engines and writes a 253 KB high-density JSON tape to `demo/web/celnet_real_data.json` and `docs/architecture/celnet_real_data.json` in ~20 ms.

### D. Open Standalone SOTA Visual Studio in Browser
```bash
./demo/run.sh --web
```
Or open [`demo/web/index.html`](web/index.html) or [`docs/architecture/CELNET-INTERACTIVE-SOTA-VISUAL-DEMO.html`](../docs/architecture/CELNET-INTERACTIVE-SOTA-VISUAL-DEMO.html) directly in any modern browser.

### E. Run Specific Demonstration Acts
```bash
./demo/run.sh --act=1     # Act 1: Quant Core & Roger Lee Asymptotic Bounds
./demo/run.sh --act=2     # Act 2: SBE Codecs & Bouchaud Propagator Execution
./demo/run.sh --act=3     # Act 3: CME SPAN 2 FHS VaR & 37.5% Cross-Margining Relief
./demo/run.sh --act=4     # Act 4: Lock-Free SHM IPC & Raft Cluster Chaos Failover
./demo/run.sh --act=5     # Act 5: Institutional Cockpit & 11.2M calls/sec C-API Streaming
```

### F. Read the Full Architectural Critique
Open [`docs/architecture/CELNET-DEMONSTRATION-REAL-DATA-CRITIQUE.html`](../docs/architecture/CELNET-DEMONSTRATION-REAL-DATA-CRITIQUE.html) or [`.md`](../docs/architecture/CELNET-DEMONSTRATION-REAL-DATA-CRITIQUE.md) for the forensic audit contrasting client-side synthetic approximations against authentic engine calculations.

---

## 2. Directory Structure

```text
demo/
├── README.md                  # This operating guide
├── run.sh                     # One-click execution script
├── web/
│   ├── index.html             # SOTA real-data visual studio (WebGPU/Canvas)
│   └── celnet_real_data.json  # 253 KB authentic Rust engine export (Zero Mocks)
└── scenarios/
    ├── act1_rates_shock.json
    ├── act2_propagator_burst.json
    └── act3_cross_margin_relief.json
```

---

## 3. Clean Separation & Zero-Mock Guarantee

* **Zero Core Modifications**: Core pricing and engine crates (`celnet-core`, `celnet-vanilla`, `celnet-rates-exotics`, etc.) are consumed exclusively via public read-only APIs.
* **Pure Safe Rust**: `#![forbid(unsafe_code)]` enforced throughout the demo harness.
* **Zero Mocks**: All calculations execute real numerical pricing models, true SBE encoders/decoders, real memory-mapped seqlock rings, 500-scenario CME SPAN 2 FHS VaR, and live loopback Raft clusters.
