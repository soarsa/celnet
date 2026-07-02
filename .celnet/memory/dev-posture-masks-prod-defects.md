---
name: dev-posture-masks-prod-defects
description: "A friendly dev/test posture (Permissive entitlements, demo_edge) masks production-posture defects; gate against the production posture (Enforce), not the dev default."
metadata: 
  node_type: memory
  type: feedback
  originSessionId: ce643053-e1eb-472e-820d-f79a68761a05
---

When a server has a **production security posture** that differs from its friendly
dev default, every gate that runs the dev posture is BLIND to defects that only the
production posture surfaces.

**The case (2026-06-11, RC 1.0):** the entitlements boundary became deny-by-default
(`AccessMode::Enforce` — absent principal ⇒ `unauthenticated`), but ALL test/e2e
edges ran `demo_edge`/harnesses in **Permissive** (absent ⇒ grant-all). That masked
a real **P1**: every client (SDK/CLI/GUI/Excel) still *omitted* the principal by
default and documented "omit ⇒ grant-all", so the headline risk workflow was denied
against a production edge. 5 layers of green gates + the server-side fix itself
certified it; only the **adversarial convergence round** (reading the code, not the
gates) caught it. Same family as [[deferred-e2e-defect-reservoir]].

**Why:** a green gate proves "works under the posture the gate ran", not "works in
production". Permissive-everywhere is the entitlements analogue of mocking the thing
under test.

**How to apply:**
1. Gate against the PRODUCTION posture. The fix made the client + cli harnesses run
   `Enforce` explicitly and the gui/excel e2e `demo_edge` run
   `CELNET_ACCESS_MODE=enforce` — the risk tests now pass *because* the clients
   assert grant-all, not because the edge is permissive.
2. Set the production-default explicitly in the harness (not relied-on implicitly) so
   a future edit can't silently re-mask it.
3. When you flip a server default to a stricter posture, **propagate it to every
   client + every gate in the same change** — a server-only fix leaves the clients
   stale (here all 4 clients + their docs lied about "omit ⇒ grant-all").
4. Adversarial code-reading rounds catch what green gates certify — keep running them
   ([[mesh-coordinator-resume]] convergence loop).
