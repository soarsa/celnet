---
name: parallel-doc-session
description: "Coordination: a parallel Claude session owns the Celnet capabilities/architecture/CelNet-Trader-integration doc; this (code) session owns crates + the phase12 workflow."
metadata: 
  node_type: memory
  type: project
  originSessionId: 4e9a6b38-74d9-4a36-9109-4d49116555b3
---

Two Claude sessions run concurrently on /Users/adrian/code/celnet (see [[parallel-session-model]]):

- **DOC session** (the parallel one): owns a capabilities + architecture + CelNet-Trader-integration document — recommended new file `docs/CELNET-CAPABILITIES.md` (so it doesn't collide with the existing `docs/CELNET-INTEGRATION.md`/`docs/ARCHITECTURE.md` or the in-flight workflow's docs). READS code + the design corpus + memory + codebase-memory graph + the celnet wikis; does NOT edit crates, `gui/`, `excel/`, or other docs. If it finds an existing doc that needs a fix, it flags rather than edits (the code session/workflow may own it).
- **CODE session** (this one): owns the crates + `gui/`/`excel/` + the running **phase12 workflow** `wf_75862914-b58` (Phase 1+2: calendar ON/SN+IMM fix, proto/types SmileModel + market-series feed + attribution, server wiring, new risk crates `celnet-risk-normalize`→`-cube`→`-limits`∥`-entitlements`, client parity, e2e). See [[session-state-2026-05-31]].

**Live status the doc session must respect (honesty):** Phase 0 GUI is committed (HEAD `82b32ac`); Phase 1+2 is **in-flight** in the workflow and NOT yet committed — so the doc must distinguish **shipped** vs **landing/in-flight** vs **designed-only/deferred** (AAD-GPU + cross-fleet fan-out are deferred per RISK-HIERARCHY §3.3 / SCALE-OUT). The structural source of truth for "what code exists" is the codebase-memory graph + the CLAUDE.md ledger, not assumptions. Re-sync after the workflow lands + this session commits.

**API-first parity** ([[api-first-client-parity]]) is a key story for the doc: one `celnet-proto` contract; GUI/SDK(`celnet-client`)/Excel(`CELNET.*`)/docs in lockstep.
