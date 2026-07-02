---
name: no-mocks-policy
description: "Celnet allows no mocks/placeholders — only complete, state-of-the-art implementations."
metadata: 
  node_type: memory
  type: feedback
  originSessionId: b315eccc-f521-4987-b5b5-1a21d5710edb
---

Every artifact in Celnet must be **100% complete and state-of-the-art** — no mocks, stubs, `todo!()`, placeholders, or "left as an exercise". Where an implementation is large, split it across files/crates rather than abbreviating it.

**Why:** User directive (30 May 2026): the product is mission-critical FX-options pricing; correctness and completeness are non-negotiable.

**How to apply:** Prefer fewer, fully-finished features over many half-built ones. If scope can't be finished in a unit of work, narrow the scope — don't fake the depth. Numerical code must be validated against references (QuantLib/published prices), not asserted. See [[celnet-mission]].
