---
name: no-commercial-products
description: Celnet uses only open-source / free software — no commercial products anywhere.
metadata: 
  node_type: memory
  type: feedback
  originSessionId: b315eccc-f521-4987-b5b5-1a21d5710edb
---

Celnet must use **no commercial products anywhere** — only open-source, permissively-licensed (MIT/Apache-2.0/BSD/ISC/etc.) software and free, academically-grounded methods.

**Why:** User directive (30 May 2026). Avoids licensing cost/lock-in and keeps the stack fully owned and auditable.

**How to apply:** No paid libraries, no proprietary commercial SDKs/solvers/data terminals as runtime deps (e.g. **no Intel MKL**, no Bloomberg/commercial-vendor runtime deps). QuantLib (open-source) as the golden oracle is fine. `cargo-deny`'s license allow-list enforces the OSS set. Any unavoidable proprietary-but-free toolkit (e.g. the CUDA toolkit used via CubeCL) must be an ADR with the fully-open fallback (wgpu/Vulkan) kept first-class. See [[scale-and-performance]], [[no-mocks-policy]].
