---
name: origin-main-team-churn
description: "A teammate's origin/main is fast + broadly-undergated; landing cross-cuts hits a re-merge/re-gate treadmill — build on branches, defer full-t2 landing."
metadata: 
  node_type: memory
  type: project
  originSessionId: 905eb293-4d97-4ffa-9bf5-b9e251021c51
---

During the architecture program (2026-06-29) a team member pushed **36+ commits to `origin/main` in one day** (13→6→17 cadence: action-capability kernel, per-user + **persisted** role-bundle capability overlay, FI/rates subsystem, FIX connections, Simulator, Excel sign-in). Their gate is **incomplete** — the combined full `just t2` repeatedly surfaced gaps their gate misses, each blocking a clean landing of my byte-identical item C:

- stream/accept auth under the new `Stream/Execute·FxOptions` capability gate not threaded through: celnet-bench wire-load, celnet-cli `conformance`+`risk_cli_parity`, 6 celnet-client SDK workflow tests, server `forwarding`.
- a **REAL server P1, still unfixed as of `fa9476a`**: the WS-mirror `StreamEdge` is built **without `.with_sessions(...)`** (`crates/celnet-server/src/ws/mod.rs`) → an empty `SessionRegistry` → under Enforce it rejects **every** login-tokened stream → reconnect storm that also breaks unary calls on that socket (the GUI/Excel are WS-primary). The gRPC edge is wired right (`lib.rs`); only the WS mirror isn't.
- GUI/Excel client never re-authenticated the open WS socket after login.
- a **parallel-test race** their NEW persisted stores introduced: `Edge::start` seeds+**saves** the CWD-relative `identity.json` (via `IdentityStore::config_path()`) + `fix-connections.json`; parallel `cargo test` edges corrupt it → `Edge::start` errs. Serial run = green, so it's test-isolation, not a defect.

Their gate evidently does NOT run: celnet-bench, celnet-cli/celnet-client conformance under **parallel** `cargo test`, the **live GUI-e2e under Enforce**, nor (sometimes) gui-typecheck. (Capability defaults are FINE — `Role::Trader` holds `Stream/Execute·FxOptions` by default via `sessions.rs` `TRADER_ACTIONS`; no deny-by-default trading regression.)

**Why:** the team out-paces a 3–4h M4 full-t2 — every time I re-merged + re-gated to land C, they'd pushed again, surfacing the next gap. That is the treadmill. Note: once you **defer to the team's approach in the auth/client files** (don't double-fix; e.g. they re-did CLI parity as "run the edge Permissive", `a2238de`), your remaining unique work is **disjoint** from their active lanes → future re-merges auto-clean, but a full re-gate per window is still forced.

**How to apply:** Operator decision (2026-06-29) = **pause C's landing, move to other items.** For C/E/G/D/H/L: build on a branch off the latest `origin/main`, **T1-gate scoped** (cargo + byte-identical parity corpus — this avoids the full-t2 client-e2e that surfaces the team's churn), and **DEFER the full-t2 landing** until `origin/main` stabilizes or the team merges. C is validated + preserved on `arch/C-product-engine-registry` (`b1303ec`). Re-attempt landings in a quiet window, or hand the branch to the team. Don't burn cycles stabilizing their churning branch to land one cross-cut. See also [[planned-builds-no-blocking]], [[dev-posture-masks-prod-defects]], [[token-and-context-discipline]].
