# Operations, Governance & Security Reference Library

This directory contains specifications and operational guidelines for governance, access control, notification workflows, and observability.

The primary operational gate documents are [VERIFICATION-CONTRACT.md](../VERIFICATION-CONTRACT.md) and [HARDENING.md](../HARDENING.md) at the documentation root.

---

## Operations & Governance Documents

| Document | Description | Status |
|---|---|---|
| [PERMISSIONS-GRANULAR-REVIEW.md](PERMISSIONS-GRANULAR-REVIEW.md) | Granular per-feature and per-action capability model across FX options and fixed income. | `REFERENCE` |
| [NOTIFICATIONS-REQUIREMENTS.md](NOTIFICATIONS-REQUIREMENTS.md) | Configurable trader notifications, breach alerts, and push messaging delivery. | `REFERENCE` |
| [OBSERVABILITY.md](OBSERVABILITY.md) | Zero-hot-path-cost telemetry, wait-free ring buffers, HdrHistogram metrics, and distributed tracing. | `REFERENCE` |

---

## Related Root Anchors

- [VERIFICATION-CONTRACT.md](../VERIFICATION-CONTRACT.md) — Anti-circular verification contract and oracle validation gates.
- [HARDENING.md](../HARDENING.md) — WS-T hardening gates, sanitizer runs, mutation floors, and release criteria.
- [PARALLEL-SESSIONS.md](../PARALLEL-SESSIONS.md) — Live parallel development mesh, lane claims, and tiered gate enforcement.
