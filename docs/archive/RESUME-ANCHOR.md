# Session-B resume anchor (one screen — context-clear-safe, replace in place)

**Goal:** Celnet 1.0-RC cut (`docs/RELEASE-1.0-RC.md`). origin/main = R1 landed (`c5a8f8b`, t2 12/12).

## In flight RIGHT NOW (both notify on completion; survive a context clear)
1. **audit-fix agent** — repairing the ONE failing T1 step: `celnet-server` test
   `entitlements_boundary::audit_records_emitted_for_allow_and_deny` (0 records vs 3 — the
   per-decision entitlement audit isn't captured; commercial-critical). Fixing the correct side
   (capture-vs-emit), no weakening. On its green → resume the batch-D gate.
2. **RC-readiness workflow** (`wss6evb7i`) — Round 3 (7 lenses) + security-deep + batch-D
   adversarial verify → release/no-release verdict. `releaseReady = zero P0/P1 ∧ batch-D LAND`.

## Batch-D landing (local main, NOT pushed; banked on origin/checkpoint/session-b-landing)
- Merged: pivot arm 32 (P==K≡TARF to_bits law) · cross-asset WS e2e · entitlements deny-by-default+audit · VV oracle. Coverage 24 arms.
- **GATE STATE: T1 paused at step 5/10** = `test-celnet-server` (the audit test above). fmt/clippy/proto all PASS in the ledger. Resume: `just t1 celnet-proto celnet-server celnet-golden celnet-parity celnet-client celnet-cli celnet-entitlements` (per-crate journaling → only the failed crate re-runs).
- After T1 green → `just t2` → merge to main → push → §6 gate-ledger entry.

## Then (RC cut)
- Read the RC verdict; fix any genuine P0/P1 in the same window (no cut with a known critical defect).
- R7 (W6 pricing-core mutation) + R8 (crypto-surface) = session-A's lanes (spend-paused); coordinate on the board, don't duplicate.
- Final t2 on the cut commit + ledger/CLAUDE-anchor/capabilities sync + tag.

## Mesh: session-A spend-paused, lanes banked on origin (lane/w6-analytics, lane/crypto-surface). §4.2/§4.3 gate protocol on the board. Graph healthy (~17k nodes; probe before trusting). Models: sonnet=mechanical, opus=verify/judgment (memory model-selection-policy).
