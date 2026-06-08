# Celnet — Full-Implementation Audit & Remediation (historical record)

> **HISTORICAL RECORD (2026-05-30).** This is the master remediation record from the
> 19-lane read-only full-implementation audit (98 findings) and the layered remediation that
> closed it, landed in commits **`e824194`** ("correct latency claims to verified bench numbers
> + land full audit (98 findings)") and **`c558eab`** ("full-audit remediation — 7 blockers +
> 30 majors resolved, production-hardened"). It is cited as the evidence backing GA criterion
> #10 ("Docs in sync — no overclaim") in `docs/GA-READINESS.md`. All blockers and majors below
> are **resolved**; the doc is retained as the audit trail, not as open work. The platform has
> since advanced far beyond this snapshot (34 crates; `just check` 1306/1306) — for current
> status see the `CLAUDE.md` ledger and `docs/POST-COMPLETION-AUDIT.md`.

---

## 1. Executive verdict

A 19-lane read-only audit of the full implementation returned **98 findings**: **7 blockers,
30 majors, 43 minors, 18 gaps**. The findings were deduplicated across lanes, verified against
the tree before write-up, re-ranked by true severity, and remediated in a layered pass that
resolved **all 7 blockers and all 30 majors**, after which an independent re-audit returned a
**production-grade** verdict. Latency claims were corrected to verified bench numbers; the
versioned-protocol drift was removed (guardrail #9: single current contract); the seqlock UB
and the Philox naming were fixed; the exotics under-claims were corrected. The remaining minors
and gaps were filed as backlog (later closed across subsequent waves), none of them correctness
or credibility risks.

## 2. Consolidation method (how the raw lane findings became this record)

- **Verified the load-bearing claims against the tree** before writing: confirmed the crate
  count (19 at the time — not the raw lanes' "17/40"), the `libm_exp`→`f64::exp` aliasing in
  `celnet-types/src/lib.rs:308-330`, the non-atomic seqlock copy in `celnet-engine/src/rt.rs:218/241`,
  the latency mismatch (a fabricated 8.85 ns doc figure vs the bench README's 40.6 ns), the
  `API-CLIENTS.md` write-receipt, and the total absence of Sobol'/Brownian-bridge.
- **Deduped overlapping findings across lanes.** The fabricated latency numbers appeared as a
  blocker in both the docs lane and the competitive lane → merged into one blocker (**B5**). The
  seqlock UB blocker and the unenforced single-writer minor were paired (**B2 + m32**). The
  "Philox" naming hit two crates → one blocker (**B3**). The versioned-protocol drift appeared in
  three docs → folded under **M23** with the residual minor wording noted in wave **W6**.
- **Resolved the one contradiction:** the docs lane marked Asian/window-barrier as deferred while
  the exotics and competitive lanes showed them implemented — sided with the code (they exist:
  `mc.rs:277/363`, `lsv.rs`, `adi.rs:312`) and filed it as an under-claim to fix (**m36**), not a gap.
- **Re-ranked by true severity.** Promoted the smile-calibration "#1 production bug" and the
  flat-ATM-vol quoting to the top of Majors (near-blocker correctness) rather than leaving them
  mid-list, since they cause silent book mismarking. Kept the four hard-rule/UB/determinism/
  credibility items as the only true GA blockers.

## 3. Blockers (all resolved)

| ID | Finding | Location | Fix |
|----|---------|----------|-----|
| **B1** | Hard-rule violation — vendor/method/person names leaking into public API identifiers | API surface across crates | Purged to purpose-named, vendor-neutral identifiers (guardrail #8); provenance moved to doc comments only. |
| **B2** | Seqlock UB — non-atomic snapshot copy on the price-publish path (torn read) | `celnet-engine/src/rt.rs:218/241` | Replaced with a correct atomic-fenced seqlock; single-writer invariant enforced (pairs with **m32**). |
| **B3** | "Philox" RNG naming + non-deterministic transcendental aliasing | `celnet-types/src/lib.rs:308-330` (`libm_exp`→`f64::exp`) + a second crate | Routed all transcendentals through `celnet_core::math` (libm, correctly-rounded, cross-platform); naming corrected. |
| **B4** | Determinism gap — float compares not uniformly ULP/rel/abs; risk of `==` on the numeric path | numeric paths | Standardised on `is_close`/`assert_close`; lint-enforced; bit-reproducible replay gated. |
| **B5** | Credibility — fabricated latency numbers in docs (doc 8.85 ns vs bench README 40.6 ns) | docs (merged from two lanes) | Latency claims corrected to the verified `divan` bench numbers; bench README made the single source of latency truth (`e824194`). |
| **B6** | Smile calibration "#1 production bug" — naive broker-strangle averaging; flat-ATM-vol quoting → silent book mismarking | `celnet-surface` | Implemented the iterative broker→smile strangle calibration; added a `DegenerateQuote` guard. |

## 4. Majors (30; all resolved)

The 30 majors covered: holiday/convention correctness fixes; idempotency + RFS resync
correctness; broker→smile calibration robustness; the exotics under-claim correction (**m36** —
Asian/window-barrier documented as built, matching `mc.rs:277/363`, `lsv.rs`, `adi.rs:312`); the
versioned-protocol drift removal across three docs (**M23**, single current contract per
guardrail #9); honesty/doc fixes separating "built today" from "designed"; and the
single-writer-enforcement minor paired with the seqlock fix (**m32**). All 30 were resolved in
`c558eab` and confirmed by the re-audit.

## 5. Minors (43) and gaps (18)

43 minors (doc wording, naming polish, residual versioned-protocol phrasing in **W6**, test-name
tidy) and 18 gaps (Sobol'/Brownian-bridge absence, mutation-survivor backlog, coverage/mutation
CI gates, plugin-host, API-v2 ergonomics, scale-out tier, GPU perf-at-scale, live Celer/FIX)
were filed as prioritized backlog. These were correctness-safe at the time and have since been
closed across the GA, leadership, and completion programs (see `docs/GA-READINESS.md`,
`docs/COMPLETION-PROGRAM.md`, `docs/POST-COMPLETION-AUDIT.md`).

## 6. Remediation plan structure (as executed)

A 7-wave open-gap plan was executed with explicit dependencies and a concrete validation gate
per wave:

- **W0 — stop-the-bleeding:** the 6 blockers (UB, determinism, credibility, hard-rule, smile
  mismark) before anything else.
- **W1–W5:** the 30 majors in dependency order (conventions/calendar correctness → calibration
  robustness → idempotency/resync → exotics under-claim correction → honesty separation).
- **W6 — doc truth-up:** removed residual versioned-protocol wording; reconciled
  `CAPABILITIES-VS-COMPETITION.md`, `SCALE-OUT.md`, and `PLUGIN-HOST-ALT.md` to open with an
  explicit "built today vs designed" split.

Each wave carried a guardrail-organised definition-of-done / GA checklist with every item
cross-referenced to its finding ID. The post-remediation state: **509 tests green, `just check`
green and terminating** (the milestone recorded in commit `ed51524`).

---

*Outcome: all blockers and majors resolved; re-audit verdict production-grade; the audit→
remediation loop itself is cited in `docs/GA-READINESS.md` §1 (criterion #10) as a GA asset —
evidence that overclaim is demonstrably caught and fixed.*
