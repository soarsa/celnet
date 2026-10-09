# CelNet — Institutional Quantitative Trading & Analytics Platform

[![Rust 1.96+](https://img.shields.io/badge/rust-1.96%2B-blue.svg)](https://www.rust-lang.org)
[![Memory Safety](https://img.shields.io/badge/memory--safety-%23!%5Bforbid(unsafe_code)%5D-brightgreen.svg)]()
[![License: MIT/Apache-2.0](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg)]()
[![Zero Mocks](https://img.shields.io/badge/tests-100%25%20Zero%20Mocks-orange.svg)]()

**CelNet** is an ultra-low-latency, deterministic, cross-asset options pricing, volatility surface, execution, and risk platform written in pure Rust. It delivers microsecond-class valuation on a core-pinned hot path, lock-free shared memory IPC, asynchronous cluster consensus with joint reconfiguration, and decentralized cryptographic capability licensing.

---

## Architecture & System Structure

CelNet is organized as a **64-crate virtual Cargo workspace** structured into cohesive domain layers:

```
┌────────────────────────────────────────────────────────────────────────────────────────────────────────┐
│                                       CELNET 64-CRATE WORKSPACE                                        │
├────────────────────────────────────────────────────────────────────────────────────────────────────────┤
│ Layer 0: Core Types & Interfaces                                                                       │
│   celnet-types • celnet-core • celnet-conventions • celnet-calendar • celnet-proto                     │
│                                                                                                        │
│ Layer 1: Vanilla & Linear Quantitative Engines                                                         │
│   celnet-vanilla • celnet-linear • celnet-equity-vanilla • celnet-commodity-vanilla • celnet-crypto-vanilla│
│                                                                                                        │
│ Layer 2: Fixed Income, Rates & Credit Leaf                                                             │
│   celnet-rates • celnet-bond • celnet-rates-risk • celnet-rates-exotics • celnet-refdata • celnet-refstore│
│                                                                                                        │
│ Layer 3: Volatility Surface & Smile Calibration                                                        │
│   celnet-surface • celnet-heston                                                                       │
│                                                                                                        │
│ Layer 4: Exotics, GPU Acceleration & Quasi-Monte Carlo                                                 │
│   celnet-exotics • celnet-qmc • celnet-gpu • celnet-risk-accel                                         │
│                                                                                                        │
│ Layer 5: Margin, XVA & Portfolio Risk Fleet                                                            │
│   celnet-margin • celnet-xva • celnet-risk-cube • celnet-risk-normalize • celnet-risk-fleet                │
│                                                                                                        │
│ Layer 6: Routing, Tiering, Hedging & Algorithmic Execution                                             │
│   celnet-router • celnet-tiering • celnet-hedge-routing • celnet-risk-routing • celnet-algo • celnet-rfq   │
│                                                                                                        │
│ Layer 7: Messaging, Shared Memory & Exchange Codecs                                                    │
│   celnet-fanout • celnet-shm • celnet-sbe • celnet-fix • celnet-exchange-codecs • celnet-journal          │
│                                                                                                        │
│ Layer 8: Consensus, Upgrades & Cryptographic Licensing                                                 │
│   celnet-replog • celnet-upgrade • celnet-license • celnet-plugin-api • celnet-plugin-host                │
│                                                                                                        │
│ Layer 9: Server Daemon, Simulators & Client SDKs                                                       │
│   celnet-server • celnet-client • celnet-cli • celnet-c-api • celnet-cme-sim • celnet-lp-sim • ...        │
└────────────────────────────────────────────────────────────────────────────────────────────────────────┘
```

---

## Key Differentiators & Invariants

1. **Ultra-Low-Latency Core**:
   - Zero dynamic heap allocations on pricing and sensitivity calculation hot paths.
   - Lock-free single-producer / multi-consumer (SPMC) broadcast ring buffers (`celnet-fanout`) with per-slot seqlocks and cache-line (64-byte) alignment.
   - Direct memory-mapped shared memory IPC (`celnet-shm`) and zero-copy Simple Binary Encoding (`celnet-sbe`).
2. **Deterministic Mathematics & Parity Verification**:
   - Bit-identical valuation across x86-64 and ARM64 architectures using correctly-rounded IEEE-754 primitives.
   - Comprehensive golden-vector test suites cross-validated against independent reference models with zero mocks.
3. **High-Availability Distributed Consensus**:
   - Multi-Raft and asymmetric quorum consensus (`celnet-replog`) enabling sub-millisecond local commits without WAN roundtrip latency.
   - Five-stage zero-loss rolling upgrade protocol (`celnet-upgrade`) with non-blocking traffic validation and twin comparator assertions.
4. **Decentralized Cryptographic Licensing**:
   - Ed25519-signed Capability Tokens with offline attenuation and first-order policy deduction (`celnet-license`).
   - TUF-aligned signed repository metadata, BLAKE3 content-addressed artifact caching, and JIT Wasm hot-swapping (`celnet-plugin-host`).

---

## Client Surfaces & SDKs

- **Web GUI Cockpit (`gui/`)**: High-density React + TypeScript trading studios (Pricing, Risk, Distribution, Blotter, Market, Policy).
- **Excel Add-in (`excel/`)**: High-performance custom worksheet functions (`CELNET.*`) supporting asynchronous array formulas and real-time streaming.
- **Python SDK (`python/celnet/`)**: Typed, high-performance client with asynchronous connection pooling and dynamic runtime management.
- **C-ABI FFI (`crates/celnet-c-api/`)**: Native C-compatible exports (`celnet.h`) for integration with C, C++, and C# environments.
- **Rust Client (`crates/celnet-client/`)**: Fully-featured async client with built-in reconnect backoff and protocol multiplexing.

---

## Quick Start & Verification

### Prerequisites
- **Rust**: 1.96.0+ (stable toolchain pinned in `rust-toolchain.toml`)
- **Node.js**: 20+ (for Web GUI and Excel Add-in)
- **Python**: 3.10+ (for Python SDK)

### Workspace Build & Test Commands

```bash
# Check all 64 workspace crates
cargo check --workspace

# Run all Rust unit and integration tests
cargo test --workspace

# Run Web GUI tests (Vitest)
cd gui && npm test -- --run

# Run Excel Add-in tests (Vitest)
cd excel && npm test -- --run

# Run Python SDK tests
cd python && python3 -m unittest discover -s tests -p "test_*.py"

# Run performance benchmarks and regression gates
cargo test -p celnet-bench
```

---

## Master Documentation Index

Detailed specifications, architectural blueprints, and quantitative guides are maintained in [`docs/`](docs/):

- **[Master Index](docs/README.md)**: System navigation and reference catalog.
- **[System Architecture](docs/ARCHITECTURE.md)**: As-built architecture, concurrency models, and NFRs.
- **[Architecture Blueprints](docs/architecture/README.md)**: Deep-dive specifications, scalability, SBE/SHM protocols, and dynamic capability hydration.
- **[Quantitative Models & Risk](docs/quant/README.md)**: Volatility surfaces, risk hierarchy, Greeks, and analytics.
- **[Fixed Income & Rates](docs/fixed-income/README.md)**: Multi-curve construction, cash bonds, credit pricing, and pricing groups.
- **[Hedging & Execution](docs/hedging/README.md)**: Auto-hedging, inventory skewing, and risk transfer.
- **[Client Surfaces & SDKs](docs/clients/README.md)**: Trading cockpits, Excel functions, FIX APIs, and SDK guides.
- **[Operating Guide](GUIDE.md)**: Core guardrails, testing laws, and development workflow.
