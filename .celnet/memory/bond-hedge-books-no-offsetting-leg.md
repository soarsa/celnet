---
name: bond-hedge-books-no-offsetting-leg
description: "FIXED + LIVE ON UAT 2026-08-11 (f5c3b37e) — bond hedges now book a real offsetting leg (same security sold back, DV01 ratio 1), so bond risk actually reduces."
metadata:
  node_type: memory
  type: project
  originSessionId: 0cb2cf23-5e93-4047-91ee-786fff00f932
  modified: 2026-08-11T15:54:44.005Z
---

**Diagnosed + FIXED 2026-08-11.** Symptom the trader saw: hedge blotter shows fired,
filled, "worked" hedges, but no offsetting position appears in the books and rates
positions keep growing.

**1. BOND hedges booked no offsetting leg (the growing-position cause) — FIXED,
local `main` `b9119974`, NOT deployed.**
`offsetting_rates_leg` (`crates/celnet-server/src/services/rates_book.rs`) flipped
side + scaled notional for Ois/Irs/Fra, then `_ => return None` for the Bond arm. The
call site guarded booking behind `let Some(leg) = ...`, so a bond booked nothing —
while provenance below it was stamped unconditionally with real fill economics.

**The key insight the earlier diagnosis got WRONG.** It concluded the fix needed the
bond's DV01/duration at fire time to scale FACE by a DV01 ratio ("real quant work, not
a match-arm addition"). That is false, and it is what blocked the fix. The offsetting
leg is the **SAME security sold back** — identical coupon/maturity/day-count/
`instrument_id`, opposite side, `factor` of the `redemption` face. Leg and fill being
the identical bond makes their **DV01 ratio identically 1**, so scaling face by
`factor` sheds exactly `factor` of the risk under ANY duration measure. No curve, no
duration input. A DV01 ratio would only be needed to hedge a bond with a *different*
instrument. It was a match-arm addition after all. (Exact under the book's current
`redemption · 1bp` proxy AND under the ADR-0016 A3 curve DV01 that replaces it.)

Three regression tests, each verified RED on the prior code: the leg exactly negates
the shed; every rates arm negates exactly (swaps unregressed); and END-TO-END, an
over-cap bond fill leaves only the warehoused portion in the book — which on the old
code reports `net -5000, hedged 5000`, the reported symptom exactly.

**2. LP panel was asked for a product FAMILY, not a security — FIXED + DEPLOYED
earlier.** `internalise_instrument_label` returns "BOND"/"OIS"/…; that reached
`best_fill("BOND")` → never matched an agg-book instrument → 100% composite fallback →
`record_fill` is gated on `venue == LpPanel` so street-side LP analytics stayed
all-zero while `merge_tick_counts` still emitted ticks-only rows (the "19K ticks /
0 quotes / 0 wins" shape). Fix: additive `HedgeContext.execution_instrument_id` fed
from `hedge_execution_instrument()` (empty ⇒ None, never fabricated); used for the LP
lookup only, so family-scoped thresholds and provenance are untouched.
STILL UNVERIFIED: whether the LP sims actually quote the same curated `instrument_id`s
the wash-book's bonds hold — if not, `best_fill` still legitimately misses.

**3. Silent-failure seam — FIXED in the same commit.** `let _ = book_into_risk_book(...)`
discarded the Result, so a REJECTED leg still left provenance claiming a hedge. Now a
rejection is rung at `tracing::error!` (the street trade really happened, so the record
stays honestly non-advisory, but the risk/blotter divergence is visible).

**DEPLOYED 2026-08-11**: live on UAT in release `f5c3b37e-20260811T152629Z`
(`deploy.yml`, failed=0, HTTPS 200). Bond positions there should now stop growing —
worth confirming on the live book after some sim flow has run.

**Deploy/ops facts:** the `celnetctl restart` race is **FIXED + deployed** (`f5c3b37e`):
`restart` was `stop; start` back-to-back, but SIGKILL is ASYNCHRONOUS, so `start` bound a
port the corpse still held → `AddrInUse` → play fails with the server DOWN. Now three
bounded waits — `stop` waits for the reap, `restart` waits for the ports, `start` retries
an in-use bind (2s/4s/8s), retrying ONLY on address-in-use. A failed start also no longer
leaves a stale pidfile, and `status` now shows the real configured ports (it used to print
the template defaults). Knobs in `roles/celnet_provision/defaults/main.yml`.

**⚠ Hazard learned 2026-08-11 — orphaned on-box builds.** A deploy whose ansible
controller dies leaves its `cargo build` RUNNING on the box (ansible ControlPersist keeps
the sshd session alive). A later deploy then serializes behind it on cargo's target-dir
lock — not corruption, but ~20 min of invisible wait. Before deploying, check
`ps -eo lstart,etime,command | grep cargo` on the box; an orphan near its final link is
worth waiting out (its dependency artifacts are reused), not killing.
`deploy.yml` does NOT re-provision control scripts — a celnetctl change needs the
provision role (or `ansible uat -m template`) BEFORE a release deploy, else the deploy
restarts using the OLD script. `deploy.yml` prunes to the newest 2 releases automatically.
Still leaks a `start-fix-sim.sh` per deploy (2 observed after this one).
See also [[uat-deploy-recovery-and-hazards]] and [[hedge-wash-lp-analytics-and-ia-reorg]].
