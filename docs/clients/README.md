# Client Interfaces, GUI & Integration Reference Library

This directory contains specifications and design documents for Celnet's client surfaces: the React/WebGPU Trader GUI, the Excel Add-in, the typed client SDKs, and FIX integration.

All client surfaces consume **one clean, unversioned API contract** mirrored across gRPC and WebSocket.

---

## Client Surfaces & Experience

| Document | Description | Status |
|---|---|---|
| [GUI-EXPERIENCE-DESIGN.md](GUI-EXPERIENCE-DESIGN.md) | World-class sell-side trading cockpit: 5 workspaces, multi-asset views, and visual design language. | `REFERENCE` |
| [EXPERIENCE-ARCHITECTURE.md](EXPERIENCE-ARCHITECTURE.md) | Unified front-end information architecture, component hierarchy, and navigation flow. | `REFERENCE` |
| [CLIENT-PARITY-MATRIX.md](CLIENT-PARITY-MATRIX.md) | Executable parity matrix proving all 18 products reach all five client surfaces bit-identically. | `LIVE` |
| [API-CLIENTS.md](API-CLIENTS.md) | Trader-centric API philosophy, client SDK design, and typed client ergonomic targets. | `REFERENCE` |
| [EXCEL-INTEGRATION.md](EXCEL-INTEGRATION.md) | Excel add-in architecture, real-time streaming, and the `CELNET.*` worksheet function library. | `REFERENCE` |
| [EXCEL-ADDIN-LOCAL-BRINGUP.md](EXCEL-ADDIN-LOCAL-BRINGUP.md) | Developer runbook for local bring-up and testing of the Excel add-in and workbook. | `REFERENCE` |
| [CELNET-CONNECTIVITY-INTEGRATION.md](CELNET-CONNECTIVITY-INTEGRATION.md) | External gateway and connectivity extension framework architecture. | `REFERENCE` |
| [FIX-API.md](FIX-API.md) | Inbound FIX 4.4 acceptor client integration and tag mapping. | `REFERENCE` |
| [FIX-SIM-DESIGN.md](FIX-SIM-DESIGN.md) | FIX client-simulator bot design (`fix-sim`) for automated load and flow testing. | `REFERENCE` |

---

## Subdirectories

- [gui-redesign/](../gui-redesign/) — UX research, detailed mockups, critique rounds, and visual design tokens.

---

## Related Root Anchors

- [CELER-INTEGRATION.md](../CELER-INTEGRATION.md) — Integration map with Celer Trader and the trade-lifecycle estate.
- [INTERFACES.md](../INTERFACES.md) — The single unversioned wire contract and gRPC/WS service definitions.
