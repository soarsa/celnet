---
name: api-first-client-parity
description: "Every capability lives in the one canonical API; the GUI, SDK(s), Excel plugin, and docs all consume it and stay consistent as the product evolves."
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 4e9a6b38-74d9-4a36-9109-4d49116555b3
---

**API-first, single-contract, all-clients-in-lockstep.** Every product capability must be exposed through the **one canonical API** (`celnet-proto` contract served by `celnet-server`). The **front-end (`gui/`) leverages the SAME APIs** as any external client — no privileged or divergent front-end-only path, and no capability computed client-side that the API doesn't also expose. When we add or evolve a capability we must keep **all** of these consistent, in the same change:

- the **API/contract** (`celnet-proto` + `celnet-server`),
- the **client SDK(s)** (`celnet-client`, and any future SDKs),
- the **Excel plugin** (`excel/`, the `CELNET.*` Office.js functions),
- the **docs** (`docs/INTERFACES.md` registry, design corpus, SDK/Excel guides).

**Why:** one clean current contract with no versioning (GUIDE.md guardrail #9) only stays clean if every surface evolves together; otherwise the GUI, SDK, and Excel drift and the "intuitive, consistent" promise breaks. This extends guardrails #9 (one contract) and #11 (trader-centric API, evolving SDKs).

**How to apply (every feature):**
1. Add the capability to the API contract first (the engine/server computes it — NOT the GUI). Example tension to fix: the GUI's Book view currently aggregates risk **client-side** by looping `transport.scenario`; the right design is a **server-side aggregate/portfolio-risk API** the GUI *and* SDK *and* Excel all call (see [[session-state-2026-05-31]] risk-hierarchy work).
2. Surface it in the SDK + Excel `CELNET.*` + the GUI via the same contract.
3. Update `docs/INTERFACES.md` and the relevant guides in the same change.
4. Keep it **intuitive** at every surface as it evolves — no half-wired or inconsistent capabilities.

Treat a feature as "done" only when API + SDK + Excel + GUI + docs are all consistent. Relevant context: [[celnet-brand-kit]], the experience-architecture work, and `docs/EXPERIENCE-ARCHITECTURE.md` should carry a "contract & client parity" lens for each proposed capability.
