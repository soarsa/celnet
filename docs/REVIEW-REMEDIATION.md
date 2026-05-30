Wrote the master remediation plan to `/Users/adrian/code/celeroption/docs/REVIEW-REMEDIATION.md`.

Consolidation summary of what I did with the raw findings:

- **Verified the load-bearing claims against the tree** before writing: confirmed 19 crates (not the doc's 17/40), the `libm_exp`→`f64::exp` aliasing in `celnet-types/src/lib.rs:308-330`, the non-atomic seqlock copy in `celnet-engine/src/rt.rs:218/241`, the latency mismatch (doc 8.85 ns vs bench README 40.6 ns), the API-CLIENTS.md write-receipt, and the total absence of Sobol/Brownian-bridge.

- **Deduped overlapping findings across lanes.** The fabricated latency numbers appeared as a blocker in both the docs lane and the competitive lane → merged into one blocker (B5). The seqlock UB blocker and the unenforced single-writer minor were paired (B2 + m32). The "Philox" naming hit two crates → one blocker (B3). The versioned-protocol drift appeared in three docs → folded under M23 with the residual minor wording noted in W6.

- **Resolved the one contradiction:** the docs lane marks Asian/window-barrier as deferred while the exotics and competitive lanes show them implemented — I sided with the code (they exist: `mc.rs:277/363`, `lsv.rs`, `adi.rs:312`) and filed it as an under-claim to fix (m36), not a gap.

- **Re-ranked by true severity.** Promoted the smile-calibration "#1 production bug" and the flat-ATM-vol quoting to top of Majors (near-blocker correctness) rather than leaving them mid-list, since they cause silent book mismarking. Kept the four hard-rule/UB/determinism/credibility items as the only true GA blockers.

The doc contains: (1) executive verdict, (2) a 6-row BLOCKERS table with location+fix, (3) 25 MAJORS and 40 MINORS tables, (4) a 7-wave open-gap plan (W0 stop-the-bleeding → W6 doc truth-up) with dependencies and a concrete validation gate per wave, and (5) a guardrail-organized DoD/GA checklist with each item cross-referenced to its finding ID.